//! Le caselle lette dall'agente sul server (era lib/core/agente.ts): una richiesta HTTP con la chiave dell'API. L'agente tiene i login,
//! legge i calendari e risponde con gli eventi e lo stato di ogni casella; qui si decide il pallino di ogni azienda.

use crate::caselle::{accoda, calendari_inoltrati, etichetta, leggi_cache_eventi, scrivi_cache_eventi, Ev};
use crate::dati::{nelle_dati, scrivi_json};
use crate::fuso::{da_secondi, iso, parse_bound};
use crate::personalizzazione::personalizzazione;
use crate::py::{numero, py_bool, py_str, string_js, Errore, Esito};
use crate::stato::{alias_per_account, color_for_account, confronta_utf16, spento, Config};
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

// Oltre questi secondi senza segni di vita dal servizio che aggiorna le caselle sul server, i dati non si possono più dare per freschi:
// prima giallo, poi rosso (rosso vuol dire "devi intervenire")
const SERVIZIO_LENTO_S: i64 = 180;
const SERVIZIO_FERMO_S: i64 = 900;
const ATTESA: Duration = Duration::from_secs(25);

fn gravita(stato: &str) -> i32 {
    match stato {
        "avviso" => 1,
        "errore" => 2,
        _ => 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatoCasella {
    pub stato: String,
    pub msg: String,
}

impl StatoCasella {
    pub fn json(&self) -> Json {
        ogg! { "stato": self.stato.clone(), "msg": self.msg.clone() }
    }
}

// Lo stato di ogni casella all'ultima lettura (serve al pallino delle aziende inoltrate, le cui copie stanno in una di queste caselle) e le
// caselle dell'ultima risposta, tenute anche su disco: con l'agente irraggiungibile la barra laterale mostra comunque le aziende, in rosso.
#[derive(Default)]
struct Memoria {
    ultimo_stato: IndexMap<String, StatoCasella>,
    note: Option<Vec<Json>>,
}

thread_local! {
    static MEMORIA: RefCell<Memoria> = RefCell::new(Memoria::default());
}

/// Lo stato delle caselle all'ultima lettura (per le prove).
pub fn ultimo_stato() -> IndexMap<String, StatoCasella> {
    MEMORIA.with(|m| m.borrow().ultimo_stato.clone())
}

/// Dimentica quello che si sa delle caselle.
pub fn azzera_memoria() {
    MEMORIA.with(|m| *m.borrow_mut() = Memoria::default());
}

pub fn id_calendario(indirizzo: &str) -> String {
    format!("remoto::{}", indirizzo.to_lowercase())
}

/// Il client HTTP del gateway verso l'agente e i servizi che non passano dalla guardia (i token della posta, le notifiche): senza i proxy
/// dell'ambiente, come fetch di Node. Uno per thread: le connessioni tenute aperte appartengono al runtime che le ha fatte.
pub fn client() -> reqwest::Client {
    thread_local! {
        static C: reqwest::Client = reqwest::Client::builder().no_proxy().build().expect("client HTTP");
    }
    C.with(Clone::clone)
}

/// GET all'agente. -> dati JSON, o Errore::Agente.
pub async fn chiedi(percorso: &str, parametri: &[(&str, &str)]) -> Esito<Json> {
    let mut url = format!("{}{}", crate::config::agente_url(), percorso);
    if !parametri.is_empty() {
        url.push('?');
        url.push_str(&form_urlencoded::Serializer::new(String::new()).extend_pairs(parametri).finish());
    }
    risposta(client().get(url).header("Authorization", format!("Bearer {}", crate::config::agente_chiave())).timeout(ATTESA)).await
}

/// POST all'agente con un corpo JSON.
pub async fn invia(percorso: &str, corpo: &Json) -> Esito<Json> {
    let url = format!("{}{}", crate::config::agente_url(), percorso);
    let r = client()
        .post(url)
        .header("Authorization", format!("Bearer {}", crate::config::agente_chiave()))
        .header("Content-Type", "application/json")
        .body(corpo.in_js())
        .timeout(ATTESA);
    risposta(r).await
}

async fn risposta(richiesta: reqwest::RequestBuilder) -> Esito<Json> {
    let r = match richiesta.send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agente remoto: richiesta fallita ({e})");
            return Err(Errore::Agente(format!("Il server dei calendari non risponde ({})", if e.is_timeout() { "ReadTimeout" } else { "ConnectionError" })));
        }
    };
    if r.status().as_u16() == 401 {
        return Err(Errore::Agente("Il server dei calendari rifiuta la chiave: va reimpostata".to_string()));
    }
    if r.status().as_u16() != 200 {
        return Err(Errore::Agente(format!("Il server dei calendari ha risposto {}", r.status().as_u16())));
    }
    let non_valida = || Errore::Agente("Risposta del server dei calendari non valida".to_string());
    let corpo = r.bytes().await.map_err(|_| non_valida())?;
    let testo = String::from_utf8_lossy(&corpo);
    Json::analizza(testo.strip_prefix('\u{feff}').unwrap_or(&testo)).map_err(|_| non_valida())
}

// ---- Stato: il pallino di ogni casella -----------------------------------------------------------------------------------
fn nascosto(c: &Json) -> bool {
    personalizzazione().account_nascosti.contains(&string_js(c.get("account")).to_lowercase())
}

fn note() -> Vec<Json> {
    if let Some(n) = MEMORIA.with(|m| m.borrow().note.clone()) {
        return n;
    }
    let lette = crate::dati::leggi_utf8(&nelle_dati(&["agente_caselle.json"]))
        .ok()
        .and_then(|t| Json::analizza(&t).ok())
        .and_then(|d| match d {
            Json::Arr(v) => Some(v.into_iter().filter(|c| py_bool(c.get("account")) && !nascosto(c)).collect()),
            _ => None,
        })
        .unwrap_or_default();
    MEMORIA.with(|m| m.borrow_mut().note = Some(lette));
    MEMORIA.with(|m| m.borrow().note.clone().unwrap_or_default())
}

fn ricorda(visibili: &[Json]) {
    let nuove: Vec<Json> = visibili.iter().map(|c| ogg! { "account": string_js(c.get("account")).to_lowercase(), "tipo": c.get("tipo").cloned().unwrap_or(Json::Null) }).collect();
    if Json::Arr(nuove.clone()).in_js() == Json::Arr(note()).in_js() {
        return;
    }
    MEMORIA.with(|m| m.borrow_mut().note = Some(nuove.clone()));
    if let Err(e) = scrivi_json(&nelle_dati(&["agente_caselle.json"]), &Json::Arr(nuove)) {
        eprintln!("agente remoto: elenco delle caselle non salvato {e}");
    }
}

/// L'agente non risponde: ogni casella nota è in errore. -> i loro stati
fn tutte_ferme(errore: &Errore) -> IndexMap<String, StatoCasella> {
    let mut stati = IndexMap::new();
    for c in note() {
        stati.insert(string_js(c.get("account")), StatoCasella { stato: "errore".to_string(), msg: errore.messaggio().to_string() });
    }
    MEMORIA.with(|m| m.borrow_mut().ultimo_stato = stati.clone());
    stati
}

/// (stato, messaggio) se il meccanismo che aggiorna le caselle non gira bene: la lettura riesce anche con i dati fermi.
fn guasto_servizio(servizio: Option<&Json>) -> Option<(&'static str, String)> {
    let vuoto = match servizio {
        Some(Json::Obj(m)) => m.is_empty(),
        s => !py_bool(s),
    };
    if vuoto {
        return Some(("avviso", "Dal server non arriva il segno di vita dell'aggiornamento".to_string()));
    }
    let s = servizio.unwrap();
    let telefono = match s.get("telefono") {
        Some(t) => py_bool(Some(t)),
        None => true,
    };
    if !telefono {
        return Some(("errore", "Sul server il lettore delle caselle è spento".to_string()));
    }
    let n = numero(s.get("eta_s"));
    let eta = if n.is_nan() || n == 0.0 { 0 } else { n.trunc() as i64 };
    if eta > SERVIZIO_FERMO_S {
        return Some(("errore", format!("Sul server l'aggiornamento è fermo da {} minuti", eta.div_euclid(60))));
    }
    if eta > SERVIZIO_LENTO_S {
        return Some(("avviso", format!("Sul server l'aggiornamento è in ritardo di {} minuti", eta.div_euclid(60))));
    }
    if py_bool(s.get("pausa")) {
        return Some(("avviso", "Sul server l'aggiornamento è in pausa (accesso in corso?)".to_string()));
    }
    None
}

fn testo_vero(v: Option<&Json>, altrimenti: &str) -> String {
    if py_bool(v) {
        string_js(v)
    } else {
        altrimenti.to_string()
    }
}

fn stato_casella(casella: &Json, servizio: Option<&Json>) -> StatoCasella {
    let mut stato = testo_vero(casella.get("stato"), "ok");
    let mut msg = testo_vero(casella.get("msg"), "");
    if stato == "ok" {
        if let Some((s, m)) = guasto_servizio(servizio) {
            stato = s.to_string();
            msg = [m, msg].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        }
    }
    StatoCasella { stato, msg }
}

/// Di due stati della stessa azienda tiene il più grave, con tutti i messaggi.
fn peggiore(a: Option<&StatoCasella>, b: StatoCasella) -> StatoCasella {
    let Some(a) = a else { return b };
    let grave = if gravita(&b.stato) > gravita(&a.stato) { &b } else { a };
    let mut msg: Vec<&str> = Vec::new();
    for m in [a.msg.as_str(), b.msg.as_str()] {
        if !m.is_empty() && !msg.contains(&m) {
            msg.push(m);
        }
    }
    StatoCasella { stato: grave.stato.clone(), msg: msg.join(" · ") }
}

fn caselle_visibili(dati: &Json) -> Vec<Json> {
    crate::py::lista(dati.get("caselle")).iter().filter(|c| py_bool(c.get("account")) && !nascosto(c)).cloned().collect()
}

fn via_di(azienda: &str) -> String {
    personalizzazione().aziende_inoltrate.get(azienda).map(|d| d.via.clone()).unwrap_or_default().to_lowercase()
}

/// Il pallino di un'azienda inoltrata: le copie stanno nel calendario di un'altra casella, se quella si legge le copie arrivano.
pub fn stato_inoltro(azienda: &str) -> Ev {
    let via = via_di(azienda);
    let ospite = MEMORIA.with(|m| m.borrow().ultimo_stato.get(&via).cloned());
    let v = match ospite {
        None => ogg! { "stato": "avviso", "stato_msg": "Non ancora letto" },
        Some(o) if o.stato != "ok" => ogg! { "stato": o.stato, "stato_msg": o.msg },
        Some(_) => ogg! { "stato": "ok", "stato_msg": format!("Copie lette dal calendario di {via}; flusso non verificabile da remoto") },
    };
    match v {
        Json::Obj(m) => m,
        _ => unreachable!(),
    }
}

/// Le aziende inoltrate che hanno ora una casella propria, letta senza errori all'ultima lettura: le loro copie non servono, la barra non le
/// mostra come calendario a parte e la casella ospite non le elenca. Se quella casella va in errore le copie tornano da sole.
pub fn inoltri_sostituiti(cfg: &Config) -> HashSet<String> {
    let stati = ultimo_stato();
    let alias = alias_per_account(cfg, stati.keys().map(String::as_str));
    let inoltrate = &personalizzazione().aziende_inoltrate;
    stati.iter().filter(|(a, s)| s.stato != "errore" && alias.get(*a).is_some_and(|n| inoltrate.contains_key(n))).filter_map(|(a, _)| alias.get(a).cloned()).collect()
}

pub fn calendari_di_inoltro(cfg: &mut Config) -> Esito<Vec<Ev>> {
    let sostituiti = inoltri_sostituiti(cfg);
    Ok(calendari_inoltrati(cfg, stato_inoltro)?.into_iter().filter(|c| !sostituiti.contains(&string_js(c.get("account")))).collect())
}

// ---- Calendari ed eventi -------------------------------------------------------------------------------------------------
/// Una voce per casella letta dall'agente, con colore, stato e interruttore. Se l'agente non risponde si dà l'ultimo elenco riuscito, con
/// ogni casella in errore; con force, o se un elenco non c'è ancora, si restituisce l'errore dell'agente.
pub async fn elenco_calendari(cfg: &mut Config, force: bool) -> Esito<Vec<Ev>> {
    let (lette, stati) = match chiedi("/api/calendari", &[]).await {
        Ok(dati) => {
            let lette = caselle_visibili(&dati);
            ricorda(&lette);
            let servizio = dati.get("servizio");
            let mut stati = IndexMap::new();
            for c in &lette {
                stati.insert(string_js(c.get("account")).to_lowercase(), stato_casella(c, servizio));
            }
            MEMORIA.with(|m| m.borrow_mut().ultimo_stato = stati.clone());
            (lette, stati)
        }
        Err(e) => {
            if !matches!(e, Errore::Agente(_)) || force || note().is_empty() {
                return Err(e);
            }
            let lette = note();
            let stati = tutte_ferme(&e);
            (lette, stati)
        }
    };
    let mut out: Vec<Ev> = Vec::new();
    for c in &lette {
        let account = string_js(c.get("account")).to_lowercase();
        let cal_id = id_calendario(&account);
        let Some(s) = stati.get(&account) else { return Err(Errore::tipo("Cannot read properties of undefined (reading 'stato')")) };
        let conto = if py_bool(c.get("eventi")) { c.get("eventi").cloned().unwrap_or_default() } else { Json::Num(0.0) };
        let v = ogg! {
            "id": cal_id.clone(),
            "account": account.clone(),
            "label": format!("{account} · Calendario"),
            "count": conto,
            "color": color_for_account(cfg, &account, None)?,
            "enabled": !spento(cfg, &cal_id),
            "stato": s.stato.clone(),
            "stato_msg": s.msg.clone(),
        };
        if let Json::Obj(m) = v {
            out.push(m);
        }
    }
    let alias = alias_per_account(cfg, out.iter().map(|c| c.get("account").and_then(Json::as_str).unwrap_or("")).collect::<Vec<_>>());
    for c in &mut out {
        let a = c.get("account").and_then(Json::as_str).unwrap_or("").to_string();
        c.insert("alias", alias.get(&a).cloned().unwrap_or(a));
    }
    Ok(out)
}

pub type Meta = Mappa;
pub type Esito3 = (Vec<Ev>, Vec<String>, Meta);

pub fn meta_nuova() -> Meta {
    match ogg! { "stale": false, "stale_at": Json::Null, "outlook_starting": false } {
        Json::Obj(m) => m,
        _ => unreachable!(),
    }
}

/// L'agente non si raggiunge: dati salvati e pallini rossi.
fn senza_agente(cfg: &Config, errore: &Errore, chiave_cache: &str, allow_cache: bool, mut meta: Meta) -> Esito3 {
    meta.insert("outlook_starting", true); // l'interfaccia mostra i dati salvati e riprova
    meta.insert("avviso_sorgente", "Il server dei calendari non risponde");
    let stati = tutte_ferme(errore);
    let alias = alias_per_account(cfg, stati.keys().map(String::as_str));
    let mut aziende = Mappa::new();
    let mut stato_aziende = Mappa::new();
    for (account, stato) in &stati {
        let azienda = alias.get(account).cloned().unwrap_or_default();
        let prima = py_bool(aziende.get(&azienda));
        aziende.insert(azienda.clone(), prima || !spento(cfg, &id_calendario(account)));
        stato_aziende.insert(azienda, stato.json());
    }
    meta.insert("aziende", Json::Obj(aziende));
    meta.insert("stato_aziende", Json::Obj(stato_aziende));
    if allow_cache {
        let salvato = leggi_cache_eventi().get(chiave_cache).cloned();
        // salvato && Object.keys(salvato).length
        let pieno = |s: &Json| match s {
            Json::Obj(m) => !m.is_empty(),
            Json::Arr(a) => !a.is_empty(),
            Json::Str(t) => !t.is_empty(),
            _ => false,
        };
        if let Some(s) = salvato.filter(pieno) {
            meta.insert("stale", true);
            let stamp = match s.get("stamp") {
                Some(Json::Null) | None => 0.0,
                Some(x) => numero(Some(x)),
            };
            meta.insert("stale_at", da_secondi(stamp));
            let eventi = match s.get("eventi") {
                Some(Json::Arr(v)) => v.iter().filter_map(|e| e.as_obj().cloned()).collect(),
                _ => Vec::new(),
            };
            return (eventi, vec![errore.messaggio().to_string()], meta);
        }
    }
    (Vec::new(), vec![errore.messaggio().to_string()], meta)
}

/// Gli eventi di tutte le caselle nell'intervallo: (eventi, errori, meta). In più meta.stato_aziende dà il pallino di ogni azienda per
/// QUESTA lettura.
pub async fn tutti_gli_eventi(cfg: &mut Config, start_iso: &str, end_iso: &str, allow_cache: bool) -> Esito<Esito3> {
    let mut meta = meta_nuova();
    let (start, end) = match (parse_bound(start_iso), parse_bound(end_iso)) {
        (Ok(s), Ok(e)) => (s, e),
        (Err(Errore::Data(m)), _) | (_, Err(Errore::Data(m))) => return Ok((Vec::new(), vec![format!("Intervallo non valido: {m}")], meta)),
        (Err(e), _) | (_, Err(e)) => return Err(e),
    };
    let chiave_cache = format!("{}|{}", iso(&start), iso(&end));
    let dati = match chiedi("/api/eventi", &[("inizio", &iso(&start)), ("fine", &iso(&end))]).await {
        Ok(d) => d,
        Err(e @ Errore::Agente(_)) => return Ok(senza_agente(cfg, &e, &chiave_cache, allow_cache, meta)),
        Err(e) => return Err(e),
    };

    let servizio = dati.get("servizio");
    let visibili = caselle_visibili(&dati);
    ricorda(&visibili);
    let mut mie: Vec<String> = Vec::new();
    for c in &visibili {
        let a = string_js(c.get("account")).to_lowercase();
        if !mie.contains(&a) {
            mie.push(a);
        }
    }
    let mut ordinate = mie.clone();
    ordinate.sort_by(|a, b| confronta_utf16(a, b));
    let alias_casella = alias_per_account(cfg, ordinate.iter().map(String::as_str));
    // Per ogni azienda: se ha almeno una casella accesa, e com'è andata la lettura. Anche le caselle spente si leggono: possono contenere
    // riunioni di un'ALTRA azienda, che devono restare visibili se quella è accesa.
    let mut aziende = Mappa::new();
    let mut stato_aziende: IndexMap<String, StatoCasella> = IndexMap::new();
    let mut errors = Vec::new();
    let mut stati = IndexMap::new();
    for c in &visibili {
        let account = string_js(c.get("account")).to_lowercase();
        let azienda = alias_casella.get(&account).cloned().unwrap_or_default();
        let stato = stato_casella(c, servizio);
        stati.insert(account.clone(), stato.clone());
        let prima = py_bool(aziende.get(&azienda));
        aziende.insert(azienda.clone(), prima || !spento(cfg, &id_calendario(&account)));
        let p = peggiore(stato_aziende.get(&azienda), stato.clone());
        stato_aziende.insert(azienda.clone(), p);
        if stato.stato == "errore" {
            errors.push(format!("{azienda}: {}", stato.msg));
        }
    }
    // Sostituzione in un colpo solo: chi legge da un'altra richiesta non deve mai trovare l'elenco a metà
    MEMORIA.with(|m| m.borrow_mut().ultimo_stato = stati);

    let mie_set: HashSet<&str> = mie.iter().map(String::as_str).collect();
    let mut events: Vec<Ev> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for e in crate::py::lista(dati.get("eventi")) {
        let account = testo_vero(e.get("account"), "").to_lowercase();
        if !mie_set.contains(account.as_str()) || !py_bool(e.get("start")) {
            continue;
        }
        // Il nome del calendario lo dice l'agente: una casella ne può portare più d'uno (i calendari condivisi da altre aziende)
        let label = if py_bool(e.get("calendar")) { e.get("calendar").cloned().unwrap_or_default() } else { Json::Str(format!("{account} · Calendario")) };
        let start = e.get("start").cloned().unwrap_or_default();
        let id = if py_bool(e.get("id")) { e.get("id").cloned().unwrap_or_default() } else { Json::Str(format!("{account}|{}", string_js(Some(&start)))) };
        let o = |k: &str, altrimenti: Json| if py_bool(e.get(k)) { e.get(k).cloned().unwrap_or_default() } else { altrimenti };
        let ev = ogg! {
            "id": id,
            "account": account.clone(),
            "calendar": label.clone(),
            "color": color_for_account(cfg, &account, None)?,
            "title": o("title", Json::Str("(senza titolo)".to_string())),
            "start": start.clone(),
            "end": o("end", start.clone()),
            "allDay": py_bool(e.get("allDay")),
            "location": o("location", Json::Str(String::new())),
            "organizer": o("organizer", Json::Str(String::new())),
            "organizer_email": o("organizer_email", Json::Str(String::new())),
            "online_url": match e.get("online_url") { None | Some(Json::Null) => Json::Null, Some(x) => x.clone() },
            "web_link": Json::Null,
            // Non c'è un EntryID di Outlook: l'interfaccia lo deve sapere
            "remoto": true,
        };
        let Json::Obj(mut ev) = ev else { unreachable!() };
        // Calendario Google: è il calendario dell'azienda e la sua etichetta vince su quella della casella in cui Outlook ha archiviato lo stesso invito
        if e.get("tipo").and_then(Json::as_str) == Some("caldav") {
            ev.insert("is_google", true);
        }
        // "Mostra come: Occupato" in Outlook o Google: un giorno intero o un impegno lungo così conta nella disponibilità (impegni.rs)
        if let Some(Json::Bool(b)) = e.get("occupato") {
            ev.insert("occupato", *b);
        }
        let invitati: Vec<String> = crate::py::lista(e.get("attendees")).iter().map(|x| py_str(Some(x)).to_lowercase()).collect();
        let etichetta_cal = string_js(Some(&label));
        accoda(&mut events, &mut seen, ev, &etichetta_cal, || invitati.iter().find(|i| mie_set.contains(i.as_str())).cloned().unwrap_or_default());
    }
    // le copie di un'azienda che si legge ora dalla sua casella: quelle della casella bastano
    let sostituiti = inoltri_sostituiti(cfg);
    events.retain(|e| !(py_bool(e.get("inoltrata_da")) && sostituiti.contains(&string_js(e.get("inoltrata_da")))));
    ordina_per_inizio(&mut events);
    etichetta(cfg, &mut events)?;
    meta.insert("aziende", Json::Obj(aziende));
    meta.insert("stato_aziende", Json::Obj(stato_aziende.into_iter().map(|(k, v)| (k, v.json())).collect()));
    if allow_cache {
        scrivi_cache_eventi(&chiave_cache, &events.iter().cloned().map(Json::Obj).collect::<Vec<_>>());
    }
    Ok((events, errors, meta))
}

/// `(a.start || "") < (b.start || "")`, stabile: l'ordine degli inizi come testi.
pub fn ordina_per_inizio(events: &mut [Ev]) {
    events.sort_by(|a, b| confronta_utf16(&inizio_testo(a), &inizio_testo(b)));
}

fn inizio_testo(e: &Ev) -> String {
    testo_vero(e.get("start"), "")
}
