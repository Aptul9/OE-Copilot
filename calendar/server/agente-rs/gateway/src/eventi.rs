//! La raccolta degli eventi da tutte le sorgenti, in un posto solo (era lib/core/eventi.ts): le caselle lette dall'agente, i link ICS e il
//! calendario locale. Vanno lette insieme, così le due strade (la pagina e la disponibilità) non possono divergere.

use crate::agente::{calendari_di_inoltro, elenco_calendari, ordina_per_inizio, tutti_gli_eventi, Esito3, Meta};
use crate::caselle::{cede_il_posto, titolo_base, Ev, ANNULLATO_RE};
use crate::fuso::{iso, iso_secondi, mezzanotte, naive_locale, ora_hhmm, parse_bound, piu_giorni};
use crate::ics::{elenco_feed, eventi_feed};
use crate::impegni::{festivita, impegni};
use crate::locali::{elenco_locali, eventi_locali, gruppi_di};
use crate::orologio::adesso;
use crate::personalizzazione::{personalizzazione, FESTIVITA_NOME};
use crate::py::{js_trim, py_bool, py_str_v, string_js, Errore, Esito};
use crate::stato::{membri_gruppo, Config, Membri};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use std::collections::{HashMap, HashSet};

fn gravita(stato: &str) -> i32 {
    match stato {
        "avviso" => 1,
        "errore" => 2,
        _ => 0,
    }
}

/// I calendari delle caselle; restituisce Errore::Agente se l'agente non risponde.
pub async fn calendari_caselle(cfg: &mut Config, force: bool) -> Esito<Vec<Ev>> {
    elenco_calendari(cfg, force).await
}

/// Le voci della barra laterale che non sono calendari delle caselle: un elenco solo, usato da tutti, perché una sorgente dimenticata in
/// uno solo dei punti si accenderebbe ma non si spegnerebbe.
pub fn calendari_extra(cfg: &mut Config) -> Esito<Vec<Ev>> {
    let mut v = elenco_feed(cfg)?;
    v.extend(elenco_locali(cfg)?);
    v.extend(calendari_di_inoltro(cfg)?);
    Ok(v)
}

/// `String(e.title || "").trim()` senza prefissi, in minuscolo: il titolo con cui si riconoscono due copie.
fn titolo_confronto(e: &Ev) -> String {
    let t = if py_bool(e.get("title")) { string_js(e.get("title")) } else { String::new() };
    titolo_base(js_trim(&t)).to_lowercase()
}

/// Toglie lo stesso appuntamento arrivato da più fonti, anche con "FW: " o "RE: " davanti a uno dei titoli. Si tiene la copia della casella,
/// ma se la stessa riunione arriva ANCHE dal calendario Google di un'azienda si adotta la sua etichetta e il suo colore.
pub fn senza_doppioni(eventi: Vec<Ev>) -> Vec<Ev> {
    let mut indice: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Ev> = Vec::new();
    for e in eventi {
        let chiave = Json::Arr(vec![Json::Str(titolo_confronto(&e)), e.get("start").cloned().unwrap_or(Json::Null), e.get("end").cloned().unwrap_or(Json::Null)]).in_js();
        let Some(&posto) = indice.get(&chiave) else {
            indice.insert(chiave, out.len());
            out.push(e);
            continue;
        };
        let tenuto = &out[posto];
        if (py_bool(tenuto.get("inoltrata_da")) && !py_bool(e.get("inoltrata_da"))) || cede_il_posto(tenuto, &e) {
            // La copia di un flusso cede il posto all'invito vero; un invito inoltrato cede a qualunque altra copia
            out[posto] = e;
            continue;
        }
        if py_bool(e.get("is_google")) && !py_bool(tenuto.get("is_google")) {
            let tenuto = &mut out[posto];
            if let Some(a) = e.get("alias") {
                tenuto.insert("alias", a.clone());
            }
            if let Some(c) = e.get("color") {
                tenuto.insert("color", c.clone());
            }
        }
    }
    out
}

/// La chiave di un evento eliminato dalla pagina (config eventi_eliminati): titolo senza FW/RE, inizio e fine in ora di Roma, come in
/// senza_doppioni. None se gli orari non si leggono.
pub fn chiave_eliminato(e: &Mappa) -> Esito<Option<String>> {
    let ora = |t: Option<&Json>| -> Esito<String> { Ok(iso_secondi(&naive_locale(t.and_then(Json::as_str).unwrap_or(""))?)) };
    let fine = if py_bool(e.get("end")) { e.get("end") } else { e.get("start") };
    match (ora(e.get("start")), ora(fine)) {
        (Ok(s), Ok(f)) => Ok(Some(Json::Arr(vec![Json::Str(titolo_confronto(e)), Json::Str(s), Json::Str(f)]).in_js())),
        (Err(Errore::Data(_)), _) | (_, Err(Errore::Data(_))) => Ok(None),
        (Err(x), _) | (_, Err(x)) => Err(x),
    }
}

/// Toglie gli eventi eliminati dalla pagina: le caselle si leggono e basta, quindi l'evento si nasconde qui.
fn senza_eliminati(cfg: &Config, eventi: Vec<Ev>) -> Esito<Vec<Ev>> {
    // v?.chiave: una voce senza chiave vale undefined, e non è mai la chiave di un evento
    let chiavi: Vec<Option<Json>> = crate::py::lista(cfg.get("eventi_eliminati")).iter().map(|v| v.get("chiave").cloned()).collect();
    if chiavi.is_empty() {
        return Ok(eventi);
    }
    let mut out = Vec::new();
    for e in eventi {
        let k = match chiave_eliminato(&e)? {
            Some(s) => Json::Str(s),
            None => Json::Null,
        };
        if !chiavi.iter().any(|c| c.as_ref() == Some(&k)) {
            out.push(e);
        }
    }
    Ok(out)
}

fn testo_o(v: Option<&Json>, altrimenti: &str) -> String {
    if py_bool(v) {
        string_js(v)
    } else {
        altrimenti.to_string()
    }
}

/// Come è andata QUESTA lettura, azienda per azienda: il pallino accanto a ogni calendario. Se un'azienda ha più sorgenti vince la peggiore.
pub fn stato_fonti(cfg: &mut Config, meta: &Meta, errori: &[String]) -> Mappa {
    let ora = ora_hhmm(adesso());
    // L'agente dice da sé com'è andata, azienda per azienda
    let vuota = Mappa::new();
    let propri = meta.get("stato_aziende").and_then(Json::as_obj).unwrap_or(&vuota);
    let mut fonti = Mappa::new();
    if let Some(Json::Obj(aziende)) = meta.get("aziende") {
        for azienda in aziende.keys() {
            let v = match propri.get(azienda) {
                Some(p) if py_bool(Some(p)) => p.clone(),
                _ => ogg! { "stato": "ok", "msg": format!("Letto alle {ora}") },
            };
            fonti.insert(azienda.clone(), v);
        }
    }
    let altri = match calendari_extra(cfg) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("stato fonti: elenco calendari non leggibile {}: {}", e.nome(), e.messaggio());
            return fonti;
        }
    };
    for c in &altri {
        let nome = if py_bool(c.get("alias")) { string_js(c.get("alias")) } else { string_js(c.get("account")) };
        let stato = testo_o(c.get("stato"), "ok");
        let msg = testo_o(c.get("stato_msg"), "");
        let Some(prec) = fonti.get(&nome).cloned() else {
            fonti.insert(nome, ogg! { "stato": stato, "msg": msg });
            continue;
        };
        // Più sorgenti per la stessa azienda: si tengono tutti i messaggi e vince lo stato peggiore
        let prec_stato = string_js(prec.get("stato"));
        let prec_msg = prec.get("msg").cloned().unwrap_or(Json::Null);
        let nuovo_stato = if gravita(&stato) > gravita(&prec_stato) { Json::Str(stato) } else { prec.get("stato").cloned().unwrap_or(Json::Null) };
        let mut msgs: Vec<String> = Vec::new();
        if py_bool(Some(&prec_msg)) {
            msgs.push(string_js(Some(&prec_msg)));
        }
        if !msg.is_empty() {
            msgs.push(msg);
        }
        fonti.insert(nome, ogg! { "stato": nuovo_stato, "msg": msgs.join(" · ") });
    }
    // Se un errore nomina un'azienda, il suo pallino DEVE essere rosso
    for testo in errori {
        let (nome, dettaglio) = match testo.find(':') {
            None => (js_trim(testo).to_string(), String::new()),
            Some(i) => (js_trim(&testo[..i]).to_string(), testo[i + 1..].to_string()),
        };
        if fonti.contains_key(&nome) {
            let d = js_trim(&dettaglio);
            fonti.insert(nome.clone(), ogg! { "stato": "errore", "msg": if d.is_empty() { nome.clone() } else { d.to_string() } });
        }
    }
    fonti
}

/// Toglie gli eventi delle aziende spente in "I miei calendari". Il filtro guarda l'AZIENDA mostrata, non la cartella che ospita l'evento.
fn solo_aziende_accese(cfg: &mut Config, eventi: Vec<Ev>, aziende_caselle: &Mappa) -> Esito<Vec<Ev>> {
    let mut acceso: Mappa = aziende_caselle.clone();
    for c in calendari_extra(cfg)? {
        let nome = if py_bool(c.get("alias")) { string_js(c.get("alias")) } else { string_js(c.get("account")) };
        let prima = py_bool(acceso.get(&nome));
        acceso.insert(nome, prima || py_bool(c.get("enabled")));
    }
    let spente: HashSet<String> = acceso.iter().filter(|(_, on)| !py_bool(Some(on))).map(|(n, _)| n.clone()).collect();
    if spente.is_empty() {
        return Ok(eventi);
    }
    // un evento personale fatto per dei gruppi ("A chi") si vede con gli eventi personali accesi, o se è acceso qualcosa di uno dei suoi gruppi
    let gruppi = cfg.get("gruppi_calendari").and_then(Json::as_obj).cloned().unwrap_or_default();
    let gruppo_acceso = |g: &str| matches!(gruppi.get(g), Some(Json::Arr(a)) if a.iter().any(|x| py_bool(acceso.get(&py_str_v(x)))));
    Ok(eventi
        .into_iter()
        .filter(|e| {
            let nome = if py_bool(e.get("alias")) { string_js(e.get("alias")) } else { string_js(e.get("account")) };
            let suoi = gruppi_di(e);
            if py_bool(e.get("is_local")) && !suoi.is_empty() {
                return !spente.contains(&nome) || suoi.iter().any(|g| gruppo_acceso(g));
            }
            !spente.contains(&nome)
        })
        .collect())
}

/// La configurazione con tutti i calendari accesi, per chi non guarda la vista (disponibilità, promemoria): una copia che non va su disco.
pub fn tutti_accesi(cfg: &Config) -> Config {
    let mut c = cfg.clone();
    c.insert("disabled_calendars", Json::Arr(vec![]));
    c.insert("_sola_lettura", true);
    c
}

/// Tutti gli eventi dell'intervallo, da ogni sorgente. -> (eventi, errori, meta). `allow_cache` false vieta di rispondere con l'ultima
/// lettura salvata; `con_feed` false lascia fuori i calendari da link ICS (anche le festività).
pub async fn raccogli(cfg: &mut Config, start_iso: &str, end_iso: &str, allow_cache: bool, con_feed: bool) -> Esito<Esito3> {
    let (mut eventi, mut errori, mut meta) = tutti_gli_eventi(cfg, start_iso, end_iso, allow_cache).await?;
    // i giorni interi delle caselle restano fuori, ma non uno segnato "Occupato": è un giorno bloccato, e si vede anche nella pagina
    if !personalizzazione().mostra_tutto_il_giorno {
        eventi.retain(|e| !py_bool(e.get("allDay")) || e.get("occupato") == Some(&Json::Bool(true)));
    }
    let (inizio, fine) = match (parse_bound(start_iso), parse_bound(end_iso)) {
        (Ok(i), Ok(f)) => (i, f),
        (Err(Errore::Data(m)), _) | (_, Err(Errore::Data(m))) => {
            errori.push(format!("Intervallo non valido: {m}"));
            return Ok((eventi, errori, meta));
        }
        (Err(e), _) | (_, Err(e)) => return Err(e),
    };
    if con_feed {
        match eventi_feed(cfg, &inizio, &fine).await {
            Ok((trovati, errori_fonte)) => {
                eventi.extend(trovati);
                errori.extend(errori_fonte);
            }
            Err(e) => {
                // Una sorgente rotta non deve portarsi via le altre
                eprintln!("raccogli: calendari da link falliti {}: {}", e.nome(), e.messaggio());
                errori.push(format!("Calendari da link: {}", e.messaggio()));
            }
        }
    }
    match eventi_locali(cfg, &inizio, &fine) {
        Ok(v) => eventi.extend(v),
        Err(e) => {
            eprintln!("raccogli: eventi locali falliti {}: {}", e.nome(), e.messaggio());
            errori.push(format!("Eventi personali: {}", e.messaggio()));
        }
    }
    // Una riunione annullata resta nel calendario con "Canceled:" nel titolo: qui, che è la strada di pagina, disponibilità, condivisione e
    // promemoria, non esce
    eventi.retain(|e| !ANNULLATO_RE.is_match(&if py_bool(e.get("title")) { string_js(e.get("title")) } else { String::new() }));
    let eventi = senza_eliminati(cfg, senza_doppioni(eventi))?;
    let aziende = meta.get("aziende").and_then(Json::as_obj).cloned().unwrap_or_default();
    let mut eventi = solo_aziende_accese(cfg, eventi, &aziende)?;
    ordina_per_inizio(&mut eventi);
    let fonti = stato_fonti(cfg, &meta, &errori);
    meta.insert("fonti", Json::Obj(fonti));
    Ok((eventi, errori, meta))
}

/// Cosa può sapere la pagina pubblica dei `giorni` giorni da `primo_giorno`: (impegni, festività, completo), solo orari, mai cosa c'è.
/// Contano tutti i calendari, accesi o spenti nella barra laterale. Con `gruppo` contano solo le aziende di quel gruppo (errore "Gruppo non
/// trovato" se non esiste). `completo` è falso se una sorgente non ha risposto bene.
pub async fn occupato(cfg: &Config, primo_giorno: (i64, i64, i64), giorni: i64, gruppo: &str) -> Esito<(Vec<[String; 2]>, Vec<[String; 3]>, bool)> {
    let mut membri: Option<Membri> = None;
    if !gruppo.is_empty() {
        membri = membri_gruppo(cfg, gruppo);
        if membri.is_none() {
            return Err(Errore::altro("Gruppo non trovato"));
        }
    }
    let inizio = mezzanotte(primo_giorno.0, primo_giorno.1, primo_giorno.2);
    let mut copia = tutti_accesi(cfg);
    let (eventi, errori, meta) = raccogli(&mut copia, &iso(&inizio), &iso(&piu_giorni(&inizio, giorni)), false, true).await?;
    let completo = errori.is_empty() && !py_bool(meta.get("outlook_starting")) && !py_bool(meta.get("stale"));
    // il calendario che un altro OE Copilot ha condiviso con noi non è un impegno nostro: non rende occupata la pagina pubblica
    let nostri: Vec<Ev> = eventi.iter().filter(|e| !py_bool(e.get("da_condivisione"))).cloned().collect();
    Ok((impegni(&nostri, primo_giorno, giorni, membri.as_ref()), festivita(&eventi, FESTIVITA_NOME), completo))
}
