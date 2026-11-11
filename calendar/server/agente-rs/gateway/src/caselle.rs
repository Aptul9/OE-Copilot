//! Regole comuni alle caselle lette dall'agente (era lib/core/caselle.ts): di che azienda è una riunione, quale copia tenere fra due uguali,
//! che colore darle. Non dipendono da dove arrivano gli eventi.

use crate::dati::{nelle_dati, scrivi_json};
use crate::orologio::secondi;
use crate::personalizzazione::personalizzazione;
use crate::py::{lunghezza_js, py_bool, py_str_v, string_js, Esito};
use crate::stato::{alias_per_account, color_for_account, nome_predefinito, spento, Config};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Un evento come lo manda get_events: i campi li decide l'agente e questo codice, la pagina li legge per nome.
pub type Ev = Mappa;

const MAX_INTERVALLI_IN_CACHE: usize = 12;

/// `[\s]` di JavaScript.
pub const SPAZI: &str = r"[\t\n\x0B\x0C\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";

fn re_i(src: &str) -> Regex {
    RegexBuilder::new(&src.replace("\\s", SPAZI)).case_insensitive(true).build().expect("espressione fissa")
}

// ---- Copia su disco degli eventi ---------------------------------------------------------------------------------------
// Permette di mostrare il calendario subito anche con l'agente irraggiungibile, invece di una griglia vuota.
pub fn leggi_cache_eventi() -> Mappa {
    match crate::dati::leggi_utf8(&nelle_dati(&["events_cache.json"])).ok().and_then(|t| Json::analizza(&t).ok()) {
        Some(Json::Obj(m)) => m,
        _ => Mappa::new(),
    }
}

pub fn scrivi_cache_eventi(chiave: &str, eventi: &[Json]) {
    let mut dati = leggi_cache_eventi();
    dati.insert(chiave, ogg! { "stamp": secondi(), "eventi": Json::Arr(eventi.to_vec()) });
    if dati.len() > MAX_INTERVALLI_IN_CACHE {
        // Tiene i più recenti: navigando tra le settimane le chiavi crescono
        let stamp = |v: &Json| v.get("stamp").and_then(Json::as_f64).unwrap_or(0.0);
        let mut recenti: Vec<(String, Json)> = dati.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        recenti.sort_by(|a, b| (stamp(&b.1) - stamp(&a.1)).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
        dati = recenti.into_iter().take(MAX_INTERVALLI_IN_CACHE).collect();
    }
    // la copia è un'ottimizzazione
    let _ = scrivi_json(&nelle_dati(&["events_cache.json"]), &Json::Obj(dati));
}

// ---- Di che azienda è un appuntamento ------------------------------------------------------------------------------------
/// Prefisso che il programma di posta mette davanti al titolo di un invito inoltrato (FW/Fwd, I, WG, TR, RV), anche ripetuto.
pub static INOLTRO_RE: LazyLock<Regex> = LazyLock::new(|| re_i(r"^\s*(?:(?:fwd?|i|wg|tr|rv)\s*:\s*)+"));

/// Gli stessi più quelli di risposta (RE, R, AW): una riunione creata rispondendo a una mail porta "RE: " davanti ed è la stessa riunione.
static PREFISSO_RE: LazyLock<Regex> = LazyLock::new(|| re_i(r"^\s*(?:(?:fwd?|i|wg|tr|rv|re|r|aw)\s*:\s*)+"));

/// Titolo che Outlook dà a una riunione annullata dall'organizzatore, che resta nel calendario.
pub static ANNULLATO_RE: LazyLock<Regex> = LazyLock::new(|| re_i(r"^\s*(?:canceled|cancelled|annullat[oaie]|cancellat[oaie])\s*:"));

/// Il titolo senza prefissi di inoltro o risposta: quello che hanno in comune le copie di uno stesso invito.
pub fn titolo_base(titolo: &str) -> String {
    let senza = PREFISSO_RE.replace(titolo, "");
    if senza.is_empty() {
        titolo.to_string()
    } else {
        senza.into_owned()
    }
}

/// Il titolo di un evento come testo: quello che RegExp.test vedrebbe.
pub fn titolo_di(e: &Ev) -> String {
    string_js(e.get("title"))
}

/// Una copia inoltrata ("FW: ...") cede il posto all'altra, che non lo è.
pub fn cede_il_posto(copia: &Ev, altra: &Ev) -> bool {
    INOLTRO_RE.is_match(&titolo_di(copia)) && !INOLTRO_RE.is_match(&titolo_di(altra))
}

/// Il resto di un testo dopo `n` unità UTF-16 (`s.slice(n)`).
pub fn dopo_js(s: &str, n: usize) -> &str {
    let mut unita = 0;
    for (i, c) in s.char_indices() {
        if unita >= n {
            return &s[i..];
        }
        unita += c.len_utf16();
    }
    ""
}

/// (azienda, titolo senza prefisso) per le copie dei flussi, se no (None, titolo).
pub fn azienda_da_titolo(titolo: &str) -> (Option<String>, String) {
    for (azienda, d) in personalizzazione().aziende_inoltrate.iter() {
        if let Some(m) = d.prefisso.find(titolo) {
            let resto = crate::py::js_trim(dopo_js(titolo, lunghezza_js(m.as_str()))).to_string();
            return (Some(azienda.clone()), if resto.is_empty() { "(senza titolo)".to_string() } else { resto });
        }
    }
    (None, titolo.to_string())
}

/// Azienda dedotta dal nome della cartella calendario, o None.
pub fn azienda_da_cartella(label: Option<&Json>) -> Option<String> {
    if !py_bool(label) {
        return None;
    }
    let testo = string_js(label).to_lowercase();
    personalizzazione().azienda_per_cartella.iter().find(|(nome, _)| testo.contains(&nome.to_lowercase())).map(|(_, a)| py_str_v(a))
}

/// Nome azienda dedotto dall'indirizzo dell'organizzatore: solo per i domini aziendali, mai per i provider generici.
pub fn azienda_da_email(email: Option<&Json>) -> Option<String> {
    if !py_bool(email) || !string_js(email).contains('@') {
        return None;
    }
    let p = personalizzazione();
    let ind = crate::py::js_trim(&string_js(email).to_lowercase()).to_string();
    let dominio = ind.rsplit('@').next().unwrap_or("").to_string();
    if !p.domini_aziendali.contains(&dominio) {
        return None;
    }
    if let Some(n) = p.nomi_per_indirizzo.get(&ind).filter(|_| !p.caselle_di_colleghi.contains(&ind)) {
        return Some(py_str_v(n));
    }
    if let Some(n) = p.nomi_per_dominio.get(&dominio) {
        return Some(py_str_v(n));
    }
    // Dominio aziendale senza nome esplicito: il nome del dominio, mai quello di un'altra azienda per ripiego
    Some(nome_predefinito(&ind))
}

/// La chiave di un doppione: JSON.stringify([account, titolo base, start, end]).
fn chiave_doppione(ev: &Ev) -> String {
    Json::Arr(vec![ev.get("account").cloned().unwrap_or(Json::Null), Json::Str(titolo_base(&titolo_di(ev))), ev.get("start").cloned().unwrap_or(Json::Null), ev.get("end").cloned().unwrap_or(Json::Null)]).in_js()
}

fn o_altro(a: Option<&Json>, b: Option<&Json>) -> Json {
    if py_bool(a) {
        a.cloned().unwrap_or_default()
    } else {
        b.cloned().unwrap_or(Json::Null)
    }
}

/// Aggiunge `ev` a `events`, riconoscendo le copie e scartando i doppioni. `seen` è l'indice (chiave -> posizione) che chi legge tiene per
/// tutta la lettura; `invitato` dice quale casella dell'utente era fra gli invitati, e si chiama solo se serve.
pub fn accoda(events: &mut Vec<Ev>, seen: &mut HashMap<String, usize>, mut ev: Ev, etichetta_calendario: &str, invitato: impl FnOnce() -> String) {
    let (inoltrata, titolo) = azienda_da_titolo(&titolo_di(&ev));
    if let Some(az) = &inoltrata {
        ev.insert("title", titolo);
        ev.insert("inoltrata_da", az.clone());
    }
    // Se né la cartella né l'organizzatore dicono di che azienda è, si guarda a quale casella era indirizzato l'invito
    let vero = |x: Option<String>| x.is_some_and(|s| !s.is_empty());
    if inoltrata.is_none() && !vero(azienda_da_cartella(Some(&Json::Str(etichetta_calendario.to_string())))) && !vero(azienda_da_email(ev.get("organizer_email"))) {
        ev.insert("invitato_come", invitato());
    }
    // Un invito inoltrato ("FW: ...") o creato rispondendo ("RE: ...") e la stessa riunione letta dal calendario di chi l'ha organizzata sono una riunione sola
    let key = chiave_doppione(&ev);
    let posto = seen.get(&key).copied();
    if let Some(p) = posto {
        if inoltrata.is_none() && !py_bool(events[p].get("inoltrata_da")) {
            if cede_il_posto(&events[p], &ev) {
                let link = o_altro(ev.get("online_url"), events[p].get("online_url"));
                ev.insert("online_url", link);
                events[p] = ev;
            } else {
                let link = o_altro(events[p].get("online_url"), ev.get("online_url"));
                events[p].insert("online_url", link);
            }
            return;
        }
        // La stessa riunione come invito vero e come copia del flusso: vince l'invito, che ha il link e l'organizzatore reale
        if py_bool(events[p].get("inoltrata_da")) && inoltrata.is_none() {
            events[p] = ev;
        }
        return;
    }
    seen.insert(key, events.len());
    events.push(ev);
}

/// Dà a ogni evento l'azienda (alias) e il colore con cui va mostrato.
pub fn etichetta(cfg: &mut Config, events: &mut [Ev]) -> Esito<()> {
    let p = personalizzazione();
    let mut account: Vec<String> = Vec::new();
    for e in events.iter() {
        let a = string_js(e.get("account"));
        if !account.contains(&a) {
            account.push(a);
        }
    }
    let alias = alias_per_account(cfg, account.iter().map(String::as_str));
    // Colore per azienda ricavato dagli account già colorati: un evento riclassificato su un'azienda riusa il colore della sua casella
    let colori_noti: Mappa = cfg.get("account_colors").and_then(Json::as_obj).cloned().unwrap_or_default();
    let alias_noti = alias_per_account(cfg, colori_noti.keys().map(String::as_str));
    let mut colore_azienda: indexmap::IndexMap<String, Json> = indexmap::IndexMap::new();
    for (acc, col) in colori_noti.iter() {
        let nome = alias_noti.get(acc).cloned().unwrap_or_else(|| acc.clone());
        colore_azienda.entry(nome).or_insert_with(|| col.clone());
    }
    for e in events.iter() {
        let a = string_js(e.get("account"));
        let nome = alias.get(&a).cloned().unwrap_or(a);
        colore_azienda.entry(nome).or_insert_with(|| e.get("color").cloned().unwrap_or(Json::Null));
    }
    // Le aziende inoltrate non hanno una casella: il colore va assegnato qui
    for (azienda, d) in p.aziende_inoltrate.iter() {
        let c = color_for_account(cfg, azienda, Some(&d.colore))?;
        colore_azienda.insert(azienda.clone(), c);
    }
    for e in events.iter_mut() {
        let a = string_js(e.get("account"));
        e.insert("alias", alias.get(&a).cloned().unwrap_or_else(|| a.clone()));
        if p.caselle_di_colleghi.contains(&a.to_lowercase()) {
            continue; // calendario di un collega: i suoi eventi restano suoi
        }
        // Priorità: la copia di un flusso, poi una cartella dedicata a un'azienda, poi chi organizza, poi a chi era indirizzato l'invito
        let az = if py_bool(e.get("inoltrata_da")) {
            Some(string_js(e.get("inoltrata_da")))
        } else {
            let pieno = |x: Option<String>| x.filter(|s| !s.is_empty());
            pieno(azienda_da_cartella(e.get("calendar"))).or_else(|| pieno(azienda_da_email(e.get("organizer_email")))).or_else(|| pieno(azienda_da_email(e.get("invitato_come"))))
        };
        if let Some(az) = az.filter(|x| !x.is_empty()) {
            if Some(az.as_str()) != e.get("alias").and_then(Json::as_str) {
                e.insert("alias", az.clone());
                if let Some(c) = colore_azienda.get(&az) {
                    e.insert("color", c.clone());
                }
            }
        }
    }
    Ok(())
}

// ---- Aziende copiate da un flusso ----------------------------------------------------------------------------------------
/// Voci per la barra laterale, una per azienda inoltrata: interruttore, colore e pallino propri. `stato_di` dà il pallino.
pub fn calendari_inoltrati(cfg: &mut Config, stato_di: impl Fn(&str) -> Ev) -> Esito<Vec<Ev>> {
    let mut out = Vec::new();
    for (azienda, d) in personalizzazione().aziende_inoltrate.iter() {
        let cal_id = format!("inoltro::{}", azienda.to_lowercase());
        let mut v = match ogg! {
            "id": cal_id.clone(),
            "account": azienda.clone(),
            "alias": azienda.clone(),
            "label": format!("Riunioni {azienda}"),
            "count": Json::Null,
            "nota": format!("copiate da Power Automate in {}", d.via),
            "is_inoltro": true,
            "color": color_for_account(cfg, azienda, Some(&d.colore))?,
            "enabled": !spento(cfg, &cal_id),
        } {
            Json::Obj(m) => m,
            _ => unreachable!(),
        };
        for (k, x) in stato_di(azienda).iter() {
            v.insert(k.clone(), x.clone());
        }
        out.push(v);
    }
    Ok(out)
}
