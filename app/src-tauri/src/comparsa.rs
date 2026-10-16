use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Theme, WebviewUrl, WebviewWindowBuilder};

use crate::finestra;

// L'avviso a comparsa delle riunioni: una finestra piccola dell'app in basso a sinistra sopra la barra delle applicazioni dello schermo
// principale, lontano dagli avvisi di Windows e di Teams in basso a destra, sempre in primo piano e senza prendere il fuoco, con una
// riga per riunione sotto una testata con la X; con più righe cresce verso l'alto. È una finestra e non una notifica, quindi si vede anche con Non disturbare. La pagina (avviso.html) non ha comandi
// nativi: chiede con una navigazione, che qui si intercetta, e l'app la ridisegna con eval.

/// Misure in pixel logici, come in avviso.css: la testata e una riga per riunione; senza titoli ("Solo ora e azienda") la finestra è più stretta
const LARGHEZZA: f64 = 340.0;
const LARGHEZZA_ANONIMO: f64 = 148.0;
const TESTATA: f64 = 30.0;
const RIGA: f64 = 46.0;
const MARGINE: f64 = 12.0;
/// Gli sfondi di avviso.css, scuro e chiaro: la finestra parte già del colore della pagina invece di lampeggiare bianca
const SFONDO_SCURO: Color = Color(0x1c, 0x1c, 0x22, 0xff);
const SFONDO_CHIARO: Color = Color(0xf6, 0xf6, 0xf9, 0xff);
/// La cornice invisibile che Windows mette ai lati di una finestra senza bordi con l'ombra: senza toglierla il margine a sinistra raddoppia
const OMBRA: f64 = 8.0;
/// ponytail: al massimo 6 righe, la settima riunione non compare (la notifica di Windows c'è lo stesso)
const RIGHE_MAX: usize = 6;
/// Se nessuno la chiude, una riga se ne va da sola 5 secondi dopo essere comparsa
const DURATA: Duration = Duration::from_secs(5);

/// La finestra dell'avviso (etichetta, una nuova a ogni apertura) e le righe che mostra, dalla riunione più vicina
struct Aperti {
    finestra: Option<String>,
    voci: Vec<Value>,
    numero: u32,
}

static APERTI: Mutex<Aperti> = Mutex::new(Aperti { finestra: None, voci: Vec::new(), numero: 0 });

/// Una riga come arriva alla pagina: testi accorciati, il logo solo come immagine data:. Il titolo manca se la pagina non lo manda ("Solo ora e azienda")
fn contenuto(dati: &Value) -> Value {
    let testo = |campo: &str, massimo: usize| -> String { dati.get(campo).and_then(Value::as_str).unwrap_or_default().chars().take(massimo).collect() };
    let mut h = DefaultHasher::new();
    testo("chiave", 400).hash(&mut h);
    let logo = dati.get("logo").and_then(Value::as_str).filter(|l| l.starts_with("data:image/") && l.len() <= 400_000);
    json!({
        "id": format!("{:x}", h.finish()),
        "ora": testo("ora", 5),
        "titolo": testo("titolo", 200),
        "logo": logo,
        "sigla": testo("sigla", 4),
        "colore": testo("colore", 32),
        "inizio": dati.get("inizio").and_then(Value::as_f64),
    })
}

/// Aggiunge le riunioni che non ci sono già (al massimo RIGHE_MAX righe) e rimette in ordine dalla più vicina; restituisce le aggiunte
fn unisci(voci: &mut Vec<Value>, nuove: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut aggiunte = Vec::new();
    for voce in nuove {
        if voci.len() < RIGHE_MAX && !voci.iter().any(|v| v["id"] == voce["id"]) {
            aggiunte.push(voce.clone());
            voci.push(voce);
        }
    }
    let quando = |v: &Value| v["inizio"].as_f64().unwrap_or(f64::MAX);
    voci.sort_by(|a, b| quando(a).total_cmp(&quando(b)));
    aggiunte
}

/// Mostra le riunioni nuove e suona una volta: il suono dei promemoria è dell'avviso (la notifica, con l'avviso acceso, è muta)
pub fn mostra(app: &AppHandle, avvisi: &[Value], suono: &str) {
    let mut aperti = APERTI.lock().unwrap_or_else(|e| e.into_inner());
    let aggiunte = unisci(&mut aperti.voci, avvisi.iter().map(contenuto));
    if aggiunte.is_empty() {
        return;
    }
    // il suono parte per primo (PlaySound ci mette fino a un secondo); DURATA conta da quando la finestra c'è, anche se crearla
    // ha richiesto tempo (PC carico, per esempio durante una riunione)
    #[cfg(windows)]
    suona(suono);
    disegna(app, &mut aperti);
    for voce in &aggiunte {
        scadenza(app, voce);
    }
}

/// L'evento di Windows di un suono dei promemoria (SUONI in server/web-ts/src/lib/core/promemoria.ts), lo stesso che il toast suona con
/// quel nome: sveglie e squilli stanno fra i Looping; un nome sconosciuto ha quello predefinito
pub(crate) fn evento_suono(nome: &str) -> String {
    let serie = |prefisso: &str| nome.strip_prefix(prefisso).is_some_and(|n| matches!(n, "" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10"));
    match nome {
        "IM" | "Mail" | "Reminder" | "SMS" => format!("Notification.{nome}"),
        _ if serie("Alarm") || serie("Call") => format!("Notification.Looping.{nome}"),
        _ => "Notification.Default".into(),
    }
}

/// Suona l'evento con PlaySound e ritorna subito: il file scelto in Pannello di controllo, Suoni, al volume dei suoni di sistema
#[cfg(windows)]
fn suona(nome: &str) {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(suono: *const u16, modulo: *mut std::ffi::c_void, opzioni: u32) -> i32;
    }
    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_ALIAS: u32 = 0x0001_0000;
    const SND_SYSTEM: u32 = 0x0020_0000;
    let evento: Vec<u16> = evento_suono(nome).encode_utf16().chain([0]).collect();
    // il nome, terminato da zero, resta vivo per tutta la chiamata
    unsafe { PlaySoundW(evento.as_ptr(), std::ptr::null_mut(), SND_ALIAS | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM) };
}

/// Toglie la riga di una riunione, o tutte con None (la X, o un clic sull'avviso)
fn rimuovi(app: &AppHandle, id: Option<&str>) {
    let mut aperti = APERTI.lock().unwrap_or_else(|e| e.into_inner());
    let prima = aperti.voci.len();
    aperti.voci.retain(|v| id.is_some_and(|id| v["id"] != id));
    if aperti.voci.len() != prima {
        disegna(app, &mut aperti);
    }
}

/// La riga se ne va da sola dopo DURATA, se nessuno l'ha già chiusa
fn scadenza(app: &AppHandle, voce: &Value) {
    let (app, id) = (app.clone(), voce["id"].as_str().unwrap_or_default().to_string());
    std::thread::spawn(move || {
        std::thread::sleep(DURATA);
        rimuovi(&app, Some(&id));
    });
}

/// Porta la finestra alle righe: la chiude se non ce ne sono più, se no la crea, oppure la ridimensiona, la rimette al suo posto e la ridisegna
fn disegna(app: &AppHandle, aperti: &mut Aperti) {
    let aperta = aperti.finestra.as_ref().and_then(|e| app.get_webview_window(e));
    if aperti.voci.is_empty() {
        aperti.finestra = None;
        if let Some(f) = aperta {
            let _ = f.destroy();
        }
        return;
    }
    let Ok(Some(schermo)) = app.primary_monitor() else { return };
    let (scala, area) = (schermo.scale_factor(), schermo.work_area());
    let larghezza = if aperti.voci.iter().any(|v| v["titolo"] != "") { LARGHEZZA } else { LARGHEZZA_ANONIMO };
    let altezza = TESTATA + RIGA * aperti.voci.len() as f64;
    let x = area.position.x as f64 / scala + MARGINE - OMBRA;
    let y = (area.position.y as f64 + area.size.height as f64) / scala - altezza - MARGINE;
    let righe = Value::Array(aperti.voci.clone());
    if let Some(f) = aperta {
        let _ = f.set_size(LogicalSize::new(larghezza, altezza));
        let _ = f.set_position(LogicalPosition::new(x, y));
        let _ = f.eval(format!("window.__oeMostra({righe})"));
        return;
    }

    aperti.numero += 1;
    let etichetta = format!("avviso-{}", aperti.numero);
    // chiaro o scuro come Windows, che la finestra principale segue
    let tema = app.get_webview_window(finestra::PRINCIPALE).and_then(|f| f.theme().ok()).unwrap_or(Theme::Light);
    let sfondo = if matches!(tema, Theme::Dark) { SFONDO_SCURO } else { SFONDO_CHIARO };
    let app_nav = app.clone();
    let costruttore = WebviewWindowBuilder::new(app, &etichetta, WebviewUrl::App("avviso.html".into()))
        .title("OE Copilot")
        .inner_size(larghezza, altezza)
        .position(x, y)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .theme(Some(tema))
        .background_color(sfondo)
        .initialization_script(format!("window.__OE_AVVISI__ = {righe};"))
        // /apri è un clic sull'avviso, /chiudi la X: in tutti e due i casi l'avviso si chiude
        .on_navigation(move |url| {
            let apri = match (url.host_str(), url.path()) {
                (Some("tauri.localhost"), "/avviso.html") => return true,
                (Some("tauri.localhost"), "/apri") => true,
                (Some("tauri.localhost"), "/chiudi") => false,
                _ => return false,
            };
            let app = app_nav.clone();
            // dal gestore della navigazione la finestra non si tocca: lo fa un altro thread
            std::thread::spawn(move || {
                if apri {
                    finestra::mostra(&app);
                }
                rimuovi(&app, None);
            });
            false
        })
        .on_new_window(|_, _| NewWindowResponse::Deny);
    match costruttore.build() {
        Ok(_) => aperti.finestra = Some(etichetta),
        Err(errore) => crate::registra_errore(&app.config().identifier, &format!("avviso a comparsa non mostrato: {errore}")),
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn senza_titolo_la_riga_ha_ora_logo_e_sigla() {
        let dati = json!({ "chiave": "e1|x", "ora": "10:00", "titolo": "Riunione", "logo": "data:image/png;base64,AAAA", "sigla": "RO", "colore": "#2563eb", "inizio": 1000.0 });
        let pieno = contenuto(&dati);
        assert_eq!(pieno["titolo"], "Riunione");
        assert_eq!(pieno["inizio"], 1000.0);
        let minimo = contenuto(&json!({ "chiave": "e1|x", "ora": "10:00", "titolo": "", "logo": "data:image/png;base64,AAAA", "sigla": "RO", "inizio": 1000.0 }));
        assert_eq!(minimo["titolo"], "");
        assert_eq!((minimo["ora"].as_str(), minimo["logo"].as_str(), minimo["sigla"].as_str()), (Some("10:00"), Some("data:image/png;base64,AAAA"), Some("RO")));
        assert_eq!(minimo["id"], pieno["id"]);
        assert_ne!(contenuto(&json!({ "chiave": "e2|x" }))["id"], pieno["id"]);
    }

    #[test]
    fn solo_immagini_data_e_testi_accorciati() {
        for logo in ["https://example.org/x.png", "data:text/html;base64,PGI+", "javascript:alert(1)"] {
            assert!(contenuto(&json!({ "logo": logo }))["logo"].is_null(), "{logo}");
        }
        let lungo = contenuto(&json!({ "titolo": "x".repeat(500), "ora": "10:00:00", "sigla": "ABCDEF" }));
        assert_eq!(lungo["titolo"].as_str().map(str::len), Some(200));
        assert_eq!((lungo["ora"].as_str(), lungo["sigla"].as_str()), (Some("10:00"), Some("ABCD")));
        assert!(lungo["inizio"].is_null());
    }

    #[test]
    fn il_suono_e_l_evento_di_windows_della_notifica() {
        assert_eq!(evento_suono("Default"), "Notification.Default");
        assert_eq!(evento_suono("Reminder"), "Notification.Reminder");
        assert_eq!(evento_suono("Alarm"), "Notification.Looping.Alarm");
        assert_eq!(evento_suono("Call10"), "Notification.Looping.Call10");
        for strano in ["", "Alarm1", "Alarm11", "Call02", "../x", "Notification.Mail"] {
            assert_eq!(evento_suono(strano), "Notification.Default", "{strano}");
        }
    }

    #[test]
    fn una_riga_per_riunione_dalla_piu_vicina() {
        let riga = |chiave: &str, inizio: f64| contenuto(&json!({ "chiave": chiave, "inizio": inizio }));
        let mut voci = Vec::new();
        assert_eq!(unisci(&mut voci, [riga("b", 2.0), riga("a", 1.0)]).len(), 2);
        assert_eq!(voci.iter().map(|v| v["inizio"].as_f64()).collect::<Vec<_>>(), [Some(1.0), Some(2.0)]);
        // la stessa riunione (secondo anticipo) non aggiunge una riga
        assert!(unisci(&mut voci, [riga("a", 1.0)]).is_empty());
        let aggiunte = unisci(&mut voci, (0..10).map(|n| riga(&format!("n{n}"), 10.0 + n as f64)));
        assert_eq!((aggiunte.len(), voci.len()), (RIGHE_MAX - 2, RIGHE_MAX));
    }
}
