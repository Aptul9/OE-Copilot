//! Gli eventi aggiunti a mano dentro l'app, calendario "Altro" (era lib/core/locali.ts): l'unico calendario su cui l'app scrive. Stanno nel
//! database dello stato (archivio.rs), così non toccano nessuna casella di posta e non richiedono permessi.

use crate::archivio::{aggiungi_locale, leggi_righe_locali, togli_locali};
use crate::caselle::Ev;
use crate::fuso::{chiave, leggi_iso, Naive};
use crate::personalizzazione::ACCOUNT_LOCALE;
use crate::py::{mappa, py_bool, string_js, testo_pulito, tipo_py, Errore, Esito};
use crate::stato::{color_for_account, spento, Config};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;

pub const ID_CALENDARIO: &str = "local::altro";
const GRUPPI_MAX: usize = 20;

/// Gli eventi personali, nell'ordine in cui sono stati aggiunti.
pub fn leggi_locali() -> Esito<Vec<Json>> {
    leggi_righe_locali()
}

/// Il prossimo identificativo: il più alto fra quelli fatti di sole cifre, più uno (si parte da 1).
fn nuovo_id(esistenti: &[Json]) -> String {
    let numeri: Vec<f64> = esistenti
        .iter()
        .map(|e| match e.as_obj().and_then(|m| m.get("uid")) {
            None | Some(Json::Null) => String::new(),
            Some(u) => string_js(Some(u)),
        })
        .filter(|u| !u.is_empty() && u.bytes().all(|b| b.is_ascii_digit()))
        .map(|u| u.parse::<f64>().unwrap_or(f64::INFINITY))
        .collect();
    if numeri.is_empty() {
        return "1".to_string();
    }
    oe_nucleo::json::numero_js(numeri.iter().cloned().fold(f64::NEG_INFINITY, f64::max) + 1.0)
}

/// I gruppi di "A chi": nomi di gruppo (gruppi_calendari) senza doppioni, al più GRUPPI_MAX; vuoto = di nessuno in particolare.
fn leggi_gruppi_evento(v: Option<&Json>) -> Esito<Vec<String>> {
    let a = match v {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Arr(a)) => a,
        Some(_) => return Err(Errore::valore("A chi: serve un elenco di gruppi.")),
    };
    let mut nomi: Vec<String> = Vec::new();
    for g in a {
        let n = testo_pulito(Some(g))?;
        if !n.is_empty() && !nomi.contains(&n) {
            nomi.push(n);
        }
    }
    if nomi.len() > GRUPPI_MAX || nomi.iter().any(|g| crate::py::lunghezza_js(g) > 100) {
        return Err(Errore::valore("A chi: troppi gruppi o nomi troppo lunghi."));
    }
    Ok(nomi)
}

/// Aggiunge un evento. dati: title, start, end, allDay, location, notes, blocca, gruppi. Restituisce Errore::Valore con il testo che la
/// pagina mostra. `blocca` si salva solo se la pagina lo manda; `gruppi` ("A chi") sono i gruppi per cui è l'evento.
pub fn aggiungi_evento(dati: Option<&Json>) -> Esito<()> {
    let vuoto = Json::obj();
    let d = if py_bool(dati) { dati.unwrap() } else { &vuoto };
    let Json::Obj(d) = d else { return Err(Errore::tipo(format!("'{}' object has no attribute 'get'", tipo_py(Some(d))))) };
    let titolo = testo_pulito(d.get("title"))?;
    if titolo.is_empty() {
        return Err(Errore::valore("Il titolo è obbligatorio."));
    }
    let inizio = testo_pulito(d.get("start"))?;
    if inizio.is_empty() {
        return Err(Errore::valore("Manca la data/ora di inizio."));
    }
    let controllo = (|| {
        leggi_iso(&inizio)?;
        if py_bool(d.get("end")) {
            match d.get("end") {
                Some(Json::Str(f)) => {
                    leggi_iso(f)?;
                }
                _ => return Err(Errore::tipo("fromisoformat: argument must be str")),
            }
        }
        Ok(())
    })();
    match controllo {
        Err(Errore::Data(m)) => return Err(Errore::valore(format!("Data non valida: {m}"))),
        Err(e) => return Err(e),
        Ok(()) => {}
    }
    let gruppi = leggi_gruppi_evento(d.get("gruppi"))?;
    let fine = testo_pulito(d.get("end"))?;
    let luogo = testo_pulito(d.get("location"))?;
    let note = testo_pulito(d.get("notes"))?;
    let blocca = d.get("blocca").and_then(Json::as_bool);
    let tutto_giorno = py_bool(d.get("allDay"));
    aggiungi_locale(|esistenti| {
        let mut e = mappa(ogg! {
            "uid": nuovo_id(esistenti),
            "title": titolo,
            "start": inizio.clone(),
            "end": if fine.is_empty() { inizio.clone() } else { fine },
            "allDay": tutto_giorno,
            "location": luogo,
            "notes": note,
        });
        if let Some(b) = blocca {
            e.insert("blocca", b);
        }
        if !gruppi.is_empty() {
            e.insert("gruppi", Json::Arr(gruppi.into_iter().map(Json::Str).collect()));
        }
        Json::Obj(e)
    })
}

/// Toglie l'evento con quell'identificativo; se non c'è non succede niente.
pub fn elimina_evento(uid: Option<&Json>) -> Esito<()> {
    togli_locali(uid)
}

/// La voce singola dell'elenco calendari: il calendario locale è sempre presente.
pub fn elenco_locali(cfg: &mut Config) -> Esito<Vec<Ev>> {
    let conto = leggi_locali()?.len();
    let v = mappa(ogg! {
        "id": ID_CALENDARIO,
        "account": ACCOUNT_LOCALE,
        "alias": ACCOUNT_LOCALE,
        "label": ACCOUNT_LOCALE,
        "url": Json::Null,
        "count": conto,
        "is_ics": false,
        "is_local": true,
        "color": color_for_account(cfg, ACCOUNT_LOCALE, None)?,
        "enabled": !spento(cfg, ID_CALENDARIO),
        // Sta su questo disco: non c'è un collegamento che possa cadere
        "stato": "ok",
        "stato_msg": "Locale",
    });
    Ok(vec![v])
}

/// I gruppi di "A chi" di un evento salvato: solo testi, vuoto se non ne ha.
pub fn gruppi_di(e: &Mappa) -> Vec<String> {
    crate::py::lista(e.get("gruppi")).iter().filter_map(|g| g.as_str().map(str::to_string)).collect()
}

fn o(e: &Mappa, k: &str, altrimenti: Json) -> Json {
    if py_bool(e.get(k)) {
        e.get(k).cloned().unwrap_or_default()
    } else {
        altrimenti
    }
}

/// Gli eventi locali che si sovrappongono all'intervallo (ora locale senza fuso). Con il calendario spento restano quelli fatti per un gruppo.
pub fn eventi_locali(cfg: &mut Config, inizio: &Naive, fine: &Naive) -> Esito<Vec<Ev>> {
    let spento_cal = spento(cfg, ID_CALENDARIO);
    let colore = color_for_account(cfg, ACCOUNT_LOCALE, None)?;
    let mut out = Vec::new();
    for x in leggi_locali()? {
        // un evento salvato che non è un oggetto non ha campi: come in JavaScript, i suoi campi valgono undefined
        let e = x.as_obj().cloned().unwrap_or_default();
        let gruppi = gruppi_di(&e);
        if spento_cal && gruppi.is_empty() {
            continue;
        }
        let s = leggi_iso(&string_js(e.get("start")));
        let f = leggi_iso(&string_js(if py_bool(e.get("end")) { e.get("end") } else { e.get("start") }));
        let (s, f) = match (s, f) {
            (Ok(s), Ok(f)) => (s, f),
            (Err(Errore::Data(_)), _) | (_, Err(Errore::Data(_))) => continue,
            (Err(e), _) | (_, Err(e)) => return Err(e),
        };
        // Python non confronta un'ora locale con una che ha lo scarto
        if s.1.is_some() || f.1.is_some() {
            return Err(Errore::altro("can't compare offset-naive and offset-aware datetimes"));
        }
        if !(chiave(&f.0) > chiave(inizio) && chiave(&s.0) < chiave(fine)) {
            continue;
        }
        let uid = e.get("uid").cloned().unwrap_or(Json::Null);
        let start = e.get("start").cloned().unwrap_or(Json::Null);
        let mut v = mappa(ogg! {
            "id": format!("local::{}", string_js(e.get("uid"))),
            "uid": uid,
            "account": ACCOUNT_LOCALE,
            "alias": ACCOUNT_LOCALE,
            "calendar": ACCOUNT_LOCALE,
            "color": colore.clone(),
            "title": o(&e, "title", Json::Str("(senza titolo)".to_string())),
            "start": start.clone(),
            "end": o(&e, "end", start),
            "allDay": py_bool(e.get("allDay")),
            "location": o(&e, "location", Json::Str(String::new())),
            "organizer": "",
            "online_url": Json::Null,
            "web_link": Json::Null,
            "is_local": true,
        });
        // uid: e.uid, che JSON.stringify non scrive se manca
        if !e.contains_key("uid") {
            v.remove("uid");
        }
        if let Some(Json::Bool(b)) = e.get("blocca") {
            v.insert("blocca", *b);
        }
        if !gruppi.is_empty() {
            v.insert("gruppi", Json::Arr(gruppi.into_iter().map(Json::Str).collect()));
        }
        out.push(v);
    }
    Ok(out)
}
