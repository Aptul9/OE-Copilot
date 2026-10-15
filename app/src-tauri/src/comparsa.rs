use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Theme, WebviewUrl, WebviewWindowBuilder};

use crate::{Stato, finestra};

// L'avviso a comparsa delle riunioni: una finestra piccola dell'app in basso a sinistra sopra la barra delle applicazioni dello schermo
// principale, lontano dagli avvisi di Windows e di Teams in basso a destra, sempre in primo piano e senza prendere il fuoco, con una
// riga per riunione sotto una testata con la X; con più righe cresce verso l'alto. È una finestra e non una notifica, quindi si vede anche con Non disturbare. La pagina (avviso.html) non ha comandi
// nativi: chiede con una navigazione, che qui si intercetta, e l'app la ridisegna con eval.

/// Misure in pixel logici, come in avviso.css: la testata e una riga per riunione; senza titoli (avviso anonimo) la finestra è più stretta
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
/// Se nessuno la chiude, una riga se ne va da sola 5 minuti dopo l'inizio della riunione
const DOPO_INIZIO_MS: f64 = 300_000.0;

/// La finestra dell'avviso (etichetta, una nuova a ogni apertura) e le righe che mostra, dalla riunione più vicina
struct Aperti {
    finestra: Option<String>,
    voci: Vec<Value>,
    numero: u32,
}

static APERTI: Mutex<Aperti> = Mutex::new(Aperti { finestra: None, voci: Vec::new(), numero: 0 });

/// Una riga come arriva alla pagina: testi accorciati, il logo solo come immagine data:, senza titolo se l'avviso è anonimo
fn contenuto(dati: &Value, anonimo: bool) -> Value {
    let testo = |campo: &str, massimo: usize| -> String { dati.get(campo).and_then(Value::as_str).unwrap_or_default().chars().take(massimo).collect() };
    let mut h = DefaultHasher::new();
    testo("chiave", 400).hash(&mut h);
    let logo = dati.get("logo").and_then(Value::as_str).filter(|l| l.starts_with("data:image/") && l.len() <= 400_000);
    json!({
        "id": format!("{:x}", h.finish()),
        "ora": testo("ora", 5),
        "titolo": if anonimo { String::new() } else { testo("titolo", 200) },
        "logo": logo,
        "sigla": testo("sigla", 4),
        "colore": testo("colore", 32),
        "scadenza": dati.get("inizio").and_then(Value::as_f64).map(|inizio| inizio + DOPO_INIZIO_MS),
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
    let quando = |v: &Value| v["scadenza"].as_f64().unwrap_or(f64::MAX);
    voci.sort_by(|a, b| quando(a).total_cmp(&quando(b)));
    aggiunte
}

pub fn mostra(app: &AppHandle, avvisi: &[Value]) {
    let stato = app.state::<Stato>();
    if !stato.comparsa.load(Ordering::Relaxed) {
        return;
    }
    let anonimo = stato.anonimo.load(Ordering::Relaxed);
    let mut aperti = APERTI.lock().unwrap_or_else(|e| e.into_inner());
    let aggiunte = unisci(&mut aperti.voci, avvisi.iter().map(|a| contenuto(a, anonimo)));
    if aggiunte.is_empty() {
        return;
    }
    for voce in &aggiunte {
        scadenza(app, voce);
    }
    disegna(app, &mut aperti);
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

/// La riga se ne va da sola alla sua scadenza, se nessuno l'ha già chiusa
fn scadenza(app: &AppHandle, voce: &Value) {
    let Some(alle) = voce["scadenza"].as_f64() else { return };
    let ora = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
    let (app, id) = (app.clone(), voce["id"].as_str().unwrap_or_default().to_string());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis((alle - ora).max(0.0) as u64));
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
    fn l_avviso_anonimo_non_porta_il_titolo() {
        let dati = json!({ "chiave": "e1|x", "ora": "10:00", "titolo": "Riunione segreta", "logo": "data:image/png;base64,AAAA", "sigla": "RO", "colore": "#2563eb", "inizio": 1000.0 });
        let pieno = contenuto(&dati, false);
        assert_eq!(pieno["titolo"], "Riunione segreta");
        assert_eq!(pieno["scadenza"], 301_000.0);
        let anonimo = contenuto(&dati, true);
        assert_eq!(anonimo["titolo"], "");
        assert!(!anonimo.to_string().contains("segreta"));
        assert_eq!((anonimo["ora"].as_str(), anonimo["logo"].as_str(), anonimo["sigla"].as_str()), (Some("10:00"), Some("data:image/png;base64,AAAA"), Some("RO")));
        assert_eq!(anonimo["id"], pieno["id"]);
        assert_ne!(contenuto(&json!({ "chiave": "e2|x" }), false)["id"], pieno["id"]);
    }

    #[test]
    fn solo_immagini_data_e_testi_accorciati() {
        for logo in ["https://example.org/x.png", "data:text/html;base64,PGI+", "javascript:alert(1)"] {
            assert!(contenuto(&json!({ "logo": logo }), false)["logo"].is_null(), "{logo}");
        }
        let lungo = contenuto(&json!({ "titolo": "x".repeat(500), "ora": "10:00:00", "sigla": "ABCDEF" }), false);
        assert_eq!(lungo["titolo"].as_str().map(str::len), Some(200));
        assert_eq!((lungo["ora"].as_str(), lungo["sigla"].as_str()), (Some("10:00"), Some("ABCD")));
        assert!(lungo["scadenza"].is_null());
    }

    #[test]
    fn una_riga_per_riunione_dalla_piu_vicina() {
        let riga = |chiave: &str, inizio: f64| contenuto(&json!({ "chiave": chiave, "inizio": inizio }), false);
        let mut voci = Vec::new();
        assert_eq!(unisci(&mut voci, [riga("b", 2.0), riga("a", 1.0)]).len(), 2);
        assert_eq!(voci.iter().map(|v| v["scadenza"].as_f64()).collect::<Vec<_>>(), [Some(300_001.0), Some(300_002.0)]);
        // la stessa riunione (secondo anticipo) non aggiunge una riga
        assert!(unisci(&mut voci, [riga("a", 1.0)]).is_empty());
        let aggiunte = unisci(&mut voci, (0..10).map(|n| riga(&format!("n{n}"), 10.0 + n as f64)));
        assert_eq!((aggiunte.len(), voci.len()), (RIGHE_MAX - 2, RIGHE_MAX));
    }
}
