//! Calendari sottoscritti con un link ICS (era lib/core/ics.ts): Google, Apple, le festività italiane. Il link è di fatto una chiave: chi ce
//! l'ha legge il calendario, quindi sta solo nella configurazione sul server. Il testo si scarica da qui, con la sua copia su disco e il suo
//! ritardo; le ricorrenze le espande l'agente. Il link lo sceglie chi usa l'app: lo scarico passa dalla guardia di rete_sicura.rs.

use crate::agente::invia;
use crate::caselle::Ev;
use crate::dati::nelle_dati;
use crate::fuso::{iso, Naive};
use crate::orologio::adesso;
use crate::personalizzazione::{COLORE_FESTIVITA, FESTIVITA_ID, FESTIVITA_NOME, FESTIVITA_URL};
use crate::py::{py_bool, string_js, testo_pulito, Errore, Esito, Motivo};
use crate::rete_sicura::{scarica_sicuro, Opzioni};
use crate::stato::{color_for_account, imposta_acceso, modifica_config, spento, Config};
use crate::uscita::{posti, rete_di_prova, TROPPE_RICHIESTE};
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use regex::{Regex, RegexBuilder};
use sha1::{Digest, Sha1};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::LazyLock;

/// Google rigenera l'ICS ogni qualche ora: scaricarlo a ogni cambio di vista sarebbe inutile e lento.
pub const TTL_S: f64 = 1800.0;
/// Quanto può essere grande un calendario: si legge e si accetta fino a qui.
pub const LIMITE_ICS: usize = 8 * 1024 * 1024;
const ATTESA_MS: u64 = 15_000;
/// Il Content-Type non è richiesto; se c'è, è uno di questi. Quello che decide sono i primi byte.
const TIPI_ICS: [&str; 3] = ["text/calendar", "text/plain", "application/octet-stream"];
const NON_ICS: &str = "il link risponde ma non contiene un calendario iCal";
const ILLEGGIBILE: &str = "Il link risponde ma non contiene un calendario iCal valido. Assicurati di aver copiato l'indirizzo segreto in formato iCal.";
const MAX_ESPANSIONI: usize = 64;

/// Come si scarica un calendario: i limiti del calendario, non quelli dei loghi; le regole sugli indirizzi sono quelle di sempre.
pub fn opzioni_ics() -> Opzioni {
    Opzioni {
        limite_lettura: LIMITE_ICS,
        limite_accettato: LIMITE_ICS,
        attesa_ms: ATTESA_MS,
        accept: "text/calendar, text/plain;q=0.9, */*;q=0.1".to_string(),
        user_agent: "CalendarHub/1.0".to_string(),
        testo_troppo_grande: format!("Il calendario è troppo grande (massimo {} MB)", LIMITE_ICS / 1024 / 1024),
    }
}

thread_local! {
    // Esito dell'ultima lettura per feed, per il pallino di stato: un feed irraggiungibile deve vedersi, non sparire.
    static STATO_FEED: RefCell<IndexMap<String, (String, String)>> = RefCell::new(IndexMap::new());
    // L'ultima espansione riuscita di ogni calendario e intervallo: se l'agente non risponde, le festività e gli altri link restano quelli di
    // un minuto fa invece di sparire.
    static ESPANSIONI: RefCell<IndexMap<String, Vec<Json>>> = RefCell::new(IndexMap::new());
}

/// Dimentica l'esito delle letture (per le prove).
pub fn azzera_feed() {
    STATO_FEED.with(|s| s.borrow_mut().clear());
    ESPANSIONI.with(|s| s.borrow_mut().clear());
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn feed_id(url: &str) -> String {
    format!("ics_{}", &hex(&Sha1::digest(url.as_bytes()))[..12])
}

/// Accetta anche i link webcal:// che usano diversi calendari.
pub fn normalizza_url(url: Option<&Json>) -> Esito<String> {
    let u = testo_pulito(url)?;
    Ok(if u.to_lowercase().starts_with("webcal://") { format!("https://{}", &u["webcal://".len()..]) } else { u })
}

/// Il calendario delle festività italiane: sempre presente e non modificabile.
pub fn feed_festivita() -> Json {
    ogg! { "id": FESTIVITA_ID, "name": FESTIVITA_NOME, "url": FESTIVITA_URL, "builtin": true }
}

pub fn feed_di(cfg: &Config) -> Vec<Json> {
    crate::py::lista(cfg.get("ics_calendars")).to_vec()
}

/// `f.name || f.url` (None: undefined)
fn nome_feed(f: &Json) -> Option<Json> {
    if py_bool(f.get("name")) {
        f.get("name").cloned()
    } else {
        f.get("url").cloned()
    }
}

/// Mette la voce solo se il valore c'è: JSON.stringify non scrive una proprietà undefined.
fn metti(m: &mut Mappa, k: &str, v: Option<&Json>) {
    if let Some(v) = v {
        m.insert(k, v.clone());
    }
}

/// Le voci dell'elenco calendari dei feed: le festività compaiono come gli altri, chi non le vuole le spegne.
pub fn elenco_feed(cfg: &mut Config) -> Esito<Vec<Ev>> {
    let mut out = Vec::new();
    for f in std::iter::once(feed_festivita()).chain(feed_di(cfg)) {
        let nome = nome_feed(&f);
        let id = string_js(f.get("id"));
        let stato = STATO_FEED.with(|s| s.borrow().get(&id).cloned());
        let builtin = py_bool(f.get("builtin"));
        let colore = if builtin { Json::Str(COLORE_FESTIVITA.to_string()) } else { color_for_account(cfg, &string_js(nome.as_ref()), None)? };
        let mut v = Mappa::new();
        metti(&mut v, "id", f.get("id"));
        metti(&mut v, "account", nome.as_ref());
        metti(&mut v, "label", nome.as_ref());
        metti(&mut v, "url", f.get("url"));
        v.insert("count", Json::Null); // un ICS non ha un conteggio a colpo d'occhio
        v.insert("is_ics", true);
        v.insert("color", colore);
        v.insert("enabled", !spento(cfg, &id));
        v.insert("builtin", builtin);
        // Verde solo se una lettura è andata a buon fine davvero
        v.insert("stato", stato.as_ref().map_or("avviso".to_string(), |s| s.0.clone()));
        v.insert("stato_msg", stato.map_or("Non ancora letto".to_string(), |s| s.1));
        out.push(v);
    }
    Ok(out)
}

// ---- Scarico e copia su disco ----------------------------------------------------------------------------------------------
/// I due punti non sono ammessi nei nomi file su Windows e l'id delle festività li contiene: si sostituiscono.
pub fn percorso_cache(id: &str) -> PathBuf {
    let pulito: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    nelle_dati(&["ics_cache", &format!("{pulito}.ics")])
}

pub fn salva_cache(id: &str, testo: &str) {
    let scritto = std::fs::create_dir_all(nelle_dati(&["ics_cache"])).and_then(|()| std::fs::write(percorso_cache(id), testo));
    if let Err(e) = scritto {
        // La copia locale è un'ottimizzazione, ma se non si scrive ogni lettura riparte dalla rete: meglio saperlo
        eprintln!("ics: copia locale di {id} non scritta {e}");
    }
}

pub fn togli_cache(id: &str) {
    let _ = std::fs::remove_file(percorso_cache(id));
}

fn decodifica(corpo: &[u8], tipo: Option<&str>) -> String {
    // Come requests: il charset dell'intestazione; per un text/* senza charset ISO-8859-1; altrimenti UTF-8
    static CHARSET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i-u)charset=["']?([A-Za-z0-9_-]+)"#).unwrap());
    let t = tipo.unwrap_or("");
    let etichetta = match CHARSET.captures(t) {
        Some(c) => c[1].to_string(),
        None if t.to_ascii_lowercase().contains("text") => "iso-8859-1".to_string(),
        None => "utf-8".to_string(),
    };
    let codifica = encoding_rs::Encoding::for_label(etichetta.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    codifica.decode_with_bom_removal(corpo).0.into_owned()
}

fn inizio_calendario(d: &[u8], bom: usize) -> bool {
    let mut i = bom;
    while i < d.len() && matches!(d[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    d.len() - i >= 15 && d[i..i + 15].eq_ignore_ascii_case(b"BEGIN:VCALENDAR")
}

/// Il testo di un calendario da un link: guardia di rete_sicura.rs e uno dei posti dei collegamenti verso l'esterno. Una risposta che non
/// comincia come un calendario non è un calendario. Uno stato HTTP da 400 in su dà Errore::Http, con il testo di requests.
pub async fn scarica(url: &str) -> Esito<String> {
    let r = {
        let _posto = posti().prendi().await?;
        scarica_sicuro(url, rete_di_prova().as_ref(), &opzioni_ics()).await
    };
    let r = match r {
        Ok(r) => r,
        Err(Errore::Rete { http: Some(d), .. }) => {
            return Err(Errore::Http { stato: d.stato, messaggio: format!("{} {} Error: {} for url: {}", d.stato, if d.stato >= 500 { "Server" } else { "Client" }, d.testo, d.url) })
        }
        Err(e) => return Err(e),
    };
    let tipo = r.tipo_dichiarato.as_deref().unwrap_or("").split(';').next().unwrap_or("").trim().to_lowercase();
    let bom = if r.dati.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };
    if (!tipo.is_empty() && !TIPI_ICS.contains(&tipo.as_str())) || !inizio_calendario(&r.dati, bom) {
        return Err(Errore::valore(NON_ICS));
    }
    Ok(decodifica(&r.dati[bom..], r.tipo_dichiarato.as_deref()))
}

/// Il testo ICS, dalla copia se fresca. -> (testo, errore morbido); testo None se non c'è né rete né copia.
pub async fn testo_calendario(id: &str, url: &str) -> (Option<String>, Option<String>) {
    let percorso = percorso_cache(id);
    let fresca = std::fs::metadata(&percorso)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|d| adesso() as f64 / 1000.0 - d.as_secs_f64() < TTL_S);
    if fresca {
        if let Ok(t) = crate::dati::leggi_utf8(&percorso) {
            return (Some(t), None);
        }
    }
    match scarica(url).await {
        Ok(testo) => {
            salva_cache(id, &testo);
            (Some(testo), None)
        }
        Err(e) => {
            // Un link che non c'è più (404, 410) è stato revocato o cambiato da chi lo aveva dato: la copia che si ha non è più un dato che si
            // può mostrare, e si toglie. Per le festività, che sono nostre, no: lì un 404 è un guasto del servizio
            if let Errore::Http { stato: stato @ (404 | 410), .. } = e {
                if id != FESTIVITA_ID {
                    togli_cache(id);
                    return (None, Some(format!("il link non risponde più ({stato}): è stato revocato o cambiato da chi lo aveva dato")));
                }
            }
            // Rete assente, o il servizio che non risponde: meglio dati vecchi che nessun dato
            match crate::dati::leggi_utf8(&percorso) {
                Ok(t) => (Some(t), Some(format!("aggiornamento non riuscito ({})", e.messaggio()))),
                Err(_) => (None, Some(e.messaggio().to_string())),
            }
        }
    }
}

// ---- Aggiunta e rimozione di un calendario da link ---------------------------------------------------------------------------
/// Aggiunge un calendario da link dopo aver controllato che il link risponda e che sia un calendario: il controllo lo fa l'agente, con la
/// stessa libreria che poi espande le ricorrenze. Restituisce Errore::Valore con il testo che la pagina mostra.
pub async fn aggiungi_feed(nome: Option<&Json>, url: Option<&Json>) -> Esito<Json> {
    let indirizzo = normalizza_url(url)?;
    let l = indirizzo.to_ascii_lowercase();
    if !(l.starts_with("http://") || l.starts_with("https://")) {
        return Err(Errore::valore("Il link deve iniziare con https:// (o webcal://)."));
    }
    let testo = match scarica(&indirizzo).await {
        Ok(t) => t,
        Err(Errore::Valore(m)) if m == NON_ICS => return Err(Errore::valore(ILLEGGIBILE)),
        // la coda piena non dice niente del link
        Err(e) if e.messaggio() == TROPPE_RICHIESTE => return Err(Errore::valore(TROPPE_RICHIESTE)),
        Err(e) => return Err(Errore::valore(format!("Il link non risponde: {}", e.messaggio()))),
    };
    let verifica = invia("/api/ics", &ogg! { "testo": testo.clone() }).await?;
    if verifica.get("ok") != Some(&Json::Bool(true)) {
        if verifica.get("valore") == Some(&Json::Bool(true)) {
            return Err(Errore::valore(ILLEGGIBILE));
        }
        return Err(Errore::altro(match verifica.get("errore") {
            None | Some(Json::Null) => "risposta dell'agente non valida".to_string(),
            x => string_js(x),
        }));
    }
    let id = feed_id(&indirizzo);
    let nome_scelto = testo_pulito(nome)?;
    // Il confronto con i calendari che ci sono e l'aggiunta sono un blocco solo, sulla configurazione di adesso: il download può essere lento
    let feed = modifica_config(|cfg| {
        if !matches!(cfg.get("ics_calendars"), Some(Json::Arr(_))) {
            cfg.insert("ics_calendars", Json::Arr(vec![]));
        }
        let Some(Json::Arr(feeds)) = cfg.get_mut("ics_calendars") else { unreachable!() };
        if feeds.iter().any(|f| f.get("id").and_then(Json::as_str) == Some(&id)) {
            return Err(Errore::valore("Questo calendario è già presente."));
        }
        let nome = if nome_scelto.is_empty() {
            if py_bool(verifica.get("nome")) {
                string_js(verifica.get("nome"))
            } else {
                "Calendario ICS".to_string()
            }
        } else {
            nome_scelto.clone()
        };
        let nuovo = ogg! { "id": id.clone(), "name": nome, "url": indirizzo.clone() };
        feeds.push(nuovo.clone());
        Ok(nuovo)
    })?;
    salva_cache(&id, &testo);
    Ok(feed)
}

/// Toglie un calendario da link: la voce, l'interruttore e la copia del testo. Un id che non c'è non cambia niente.
pub fn rimuovi_feed(id: Option<&Json>) -> Esito<()> {
    let testo_id = string_js(id);
    modifica_config(|cfg| {
        let restano: Vec<Json> = feed_di(cfg).into_iter().filter(|f| match (f.get("id"), id) {
            (Some(a), Some(b)) => a != b,
            (None, None) => false,
            _ => true,
        }).collect();
        cfg.insert("ics_calendars", Json::Arr(restano));
        imposta_acceso(cfg, &testo_id, true);
        Ok(())
    })?;
    togli_cache(&testo_id);
    Ok(())
}

// ---- Lettura eventi ------------------------------------------------------------------------------------------------------
/// I calendari che un altro OE Copilot pubblica con un link di condivisione portano questa riga e questo PRODID (ics_uscita.rs).
static DA_CONDIVISIONE: LazyLock<Regex> = LazyLock::new(|| RegexBuilder::new(r"^X-OE-COPILOT-SHARE:1[ \t]*\r?$").multi_line(true).crlf(true).build().unwrap());
static PRODID_CONDIVISIONE: LazyLock<Regex> = LazyLock::new(|| RegexBuilder::new(r"^PRODID:-//OE Copilot//Condivisione//IT[ \t]*\r?$").multi_line(true).crlf(true).build().unwrap());

/// Le ricorrenze del testo nell'intervallo, espanse dall'agente.
pub async fn espandi(testo: &str, inizio: &Naive, fine: &Naive) -> Esito<Vec<Json>> {
    let chiave = format!("{}|{}|{}", hex(&Sha1::digest(testo.as_bytes())), iso(inizio), iso(fine));
    let r = match invia("/api/ics", &ogg! { "testo": testo, "inizio": iso(inizio), "fine": iso(fine) }).await {
        Ok(r) => r,
        Err(e) => {
            let ultima = if matches!(e, Errore::Agente(_)) { ESPANSIONI.with(|c| c.borrow().get(&chiave).cloned()) } else { None };
            let Some(ultima) = ultima else { return Err(e) };
            eprintln!("ics: l'agente non risponde ({}), si usa l'ultima lettura", e.messaggio());
            return Ok(ultima);
        }
    };
    if r.get("ok") != Some(&Json::Bool(true)) {
        return Err(Errore::altro(match r.get("errore") {
            None | Some(Json::Null) => "risposta dell'agente non valida".to_string(),
            x => string_js(x),
        }));
    }
    let eventi = crate::py::lista(r.get("eventi")).to_vec();
    ESPANSIONI.with(|c| {
        let mut c = c.borrow_mut();
        c.shift_remove(&chiave);
        c.insert(chiave, eventi.clone());
        if c.len() > MAX_ESPANSIONI {
            c.shift_remove_index(0);
        }
    });
    Ok(eventi)
}

/// Gli eventi dei calendari ICS accesi nell'intervallo (ora locale senza fuso). -> (eventi, errori)
pub async fn eventi_feed(cfg: &mut Config, inizio: &Naive, fine: &Naive) -> Esito<(Vec<Ev>, Vec<String>)> {
    let mut eventi = Vec::new();
    let mut errori = Vec::new();
    for feed in std::iter::once(feed_festivita()).chain(feed_di(cfg)) {
        let builtin = py_bool(feed.get("builtin"));
        let id = string_js(feed.get("id"));
        let spento_feed = spento(cfg, &id);
        if spento_feed && !builtin {
            continue;
        }
        // Le festività si leggono anche da spente (costa nulla: c'è la copia locale) ma partono marcate "nascosto"
        let nome = nome_feed(&feed);
        let nome_testo = string_js(nome.as_ref());
        let (testo, errore) = testo_calendario(&id, &string_js(feed.get("url"))).await;
        let stato = match &errore {
            Some(m) => (if testo.as_deref().is_some_and(|t| !t.is_empty()) { "avviso" } else { "errore" }.to_string(), m.clone()),
            None => ("ok".to_string(), "Aggiornato".to_string()),
        };
        STATO_FEED.with(|s| s.borrow_mut().insert(id.clone(), stato));
        if let Some(m) = &errore {
            errori.push(format!("{nome_testo}: {m}"));
        }
        let Some(testo) = testo.filter(|t| !t.is_empty()) else { continue };
        let trovati = match espandi(&testo, inizio, fine).await {
            Ok(t) => t,
            Err(e) => {
                // Un calendario malformato non deve far cadere il resto
                errori.push(format!("{nome_testo}: calendario illeggibile ({})", e.messaggio()));
                continue;
            }
        };
        let colore = if builtin { Json::Str(COLORE_FESTIVITA.to_string()) } else { color_for_account(cfg, &nome_testo, None)? };
        // gli eventi di un calendario condiviso da un altro OE Copilot restano segnati: la disponibilità pubblica non li conta come impegni
        let condiviso = DA_CONDIVISIONE.is_match(&testo) && PRODID_CONDIVISIONE.is_match(&testo);
        for ev in trovati {
            let mut v = Mappa::new();
            if condiviso {
                v.insert("da_condivisione", true);
            }
            v.insert("id", format!("{id}|{}|{}", string_js(ev.get("uid")), string_js(ev.get("inizio"))));
            metti(&mut v, "account", nome.as_ref());
            metti(&mut v, "alias", nome.as_ref()); // per gli ICS il nome scelto è già l'etichetta
            metti(&mut v, "calendar", nome.as_ref());
            v.insert("color", colore.clone());
            v.insert("title", if py_bool(ev.get("titolo")) { ev.get("titolo").cloned().unwrap_or_default() } else { Json::Str("(senza titolo)".to_string()) });
            metti(&mut v, "start", ev.get("inizio"));
            metti(&mut v, "end", ev.get("fine"));
            v.insert("allDay", py_bool(ev.get("tutto_giorno")));
            metti(&mut v, "location", ev.get("luogo"));
            metti(&mut v, "organizer", ev.get("organizzatore"));
            metti(&mut v, "online_url", ev.get("link"));
            v.insert("web_link", Json::Null);
            v.insert("nascosto", spento_feed);
            eventi.push(v);
        }
    }
    Ok((eventi, errori))
}

/// Il motivo dell'errore per un Motivo (per le prove).
pub fn e_non_risponde(e: &Errore) -> bool {
    matches!(e, Errore::Rete { motivo: Motivo::NonRisponde, .. })
}
