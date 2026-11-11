//! Una casella di Outlook (o di Google) sul telefono Android virtuale, come la leggeva l'agente Python: una copia di calendar.db con i suoi file
//! di appoggio, la scelta dei calendari, gli invitati, le serie espanse, i doppioni tolti, lo stato di ogni lettura.

use indexmap::IndexMap;
use jiff::civil::DateTime;
use oe_nucleo::android::{OccorrenzaAndroid, RigaAndroid, annullata, espandi_righe_android};
use oe_nucleo::errori::{Errore, Esito};
use oe_nucleo::html::pulisci_link;
use oe_nucleo::ics::MEETING_URL;
use oe_nucleo::json::{Json, Mappa, numero_js};
use oe_nucleo::pyfmt::{py_bool, py_strip, py_tipo};
use rusqlite::Connection;
use rusqlite::types::ValueRef;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DB: &str = "calendar.db";
const TIPI_ACCOUNT: [&str; 2] = ["com.microsoft.office.outlook", "com.google"];
/// Attendees.RELATIONSHIP_ORGANIZER
const ORGANIZZATORE: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Livello {
    Info,
    Warning,
    Error,
}

impl Livello {
    pub fn nome(self) -> &'static str {
        match self {
            Livello::Info => "INFO",
            Livello::Warning => "WARNING",
            Livello::Error => "ERROR",
        }
    }
}

pub type Registro = Rc<dyn Fn(Livello, &str)>;
pub type Adesso = Rc<dyn Fn() -> i64>;

/// I millisecondi dall'epoca, come `Date.now()`.
pub fn adesso_vero() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Il contesto dell'agente che serve a una casella: dove sta lo stato degli accessi, che ore sono, in che fuso, dove scrivere i log.
#[derive(Clone)]
pub struct Contesto {
    pub accessi: PathBuf,
    pub zona: String,
    pub adesso: Adesso,
    pub registro: Registro,
    pub tokens: Option<Rc<crate::diretto::GestoreToken>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stato {
    pub stato: String,
    pub msg: Json,
}

impl Stato {
    pub fn non_letto() -> Self {
        Stato { stato: "avviso".to_string(), msg: Json::from("Non ancora letto") }
    }
}

/// Una riga di Events com'è nel database: colonna, valore.
pub type RigaDb = Mappa;

#[derive(Clone, Debug, Default)]
pub struct Persona {
    pub invitati: Vec<String>,
    pub organizzatore: Json,
}

#[derive(Debug, Default)]
pub struct Lettura {
    pub cal: IndexMap<i64, String>,
    pub eventi: Vec<RigaDb>,
    pub persone: HashMap<i64, Persona>,
    /// Le righe segnate "Occupato" (Mostra come di Outlook e Google); manca se calendar.db non ha la colonna.
    pub occupati: Option<HashSet<i64>>,
}

/// Un file di testo UTF-8 che si legge tutto o niente, senza il BOM iniziale (come `TextDecoder` con `fatal`).
pub fn leggi_utf8(percorso: &Path) -> io::Result<String> {
    let byte = fs::read(percorso)?;
    let testo = String::from_utf8(byte).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(match testo.strip_prefix('\u{feff}') {
        Some(resto) => resto.to_string(),
        None => testo,
    })
}

/// `String(valore)` di un valore che arriva da SQLite o da JSON.
pub fn testo_js(v: &Json) -> String {
    match v {
        Json::Null => "null".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => numero_js(*n),
        Json::Str(s) => s.clone(),
        Json::Arr(a) => a.iter().map(|x| if x.is_null() { String::new() } else { testo_js(x) }).collect::<Vec<_>>().join(","),
        Json::Obj(_) => "[object Object]".to_string(),
    }
}

/// La verità di JavaScript (`if (valore)`).
pub fn vero_js(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Num(n) => *n != 0.0 && !n.is_nan(),
        Json::Str(s) => !s.is_empty(),
        Json::Arr(_) | Json::Obj(_) => true,
    }
}

/// str.lower() di un valore che per Python deve essere una stringa.
fn minuscolo(v: &Json) -> Esito<String> {
    match v {
        Json::Str(s) => Ok(s.to_lowercase()),
        _ => Err(Errore::Altro(format!("'{}' object has no attribute 'lower'", py_tipo(v)))),
    }
}

/// {email: {"stato", "msg"}} scritto dal rilevatore degli accessi, o {} se manca o non si legge.
fn leggi_accessi(percorso: &Path) -> Json {
    let Some(dati) = leggi_utf8(percorso).ok().and_then(|t| Json::analizza(&t).ok()) else { return Json::obj() };
    let Json::Obj(mut m) = dati else { return Json::obj() };
    match m.remove("caselle") {
        Some(caselle) if py_bool(&caselle) => caselle,
        _ => Json::obj(),
    }
}

/// dict.get() di un valore JSON: None se la chiave non c'è; per un valore che non è un dizionario Python solleva AttributeError.
fn prendi<'a>(valore: &'a Json, chiave: &str) -> Esito<&'a Json> {
    match valore {
        Json::Obj(m) => Ok(m.get(chiave).unwrap_or(&Json::Null)),
        _ => Err(Errore::Altro(format!("'{}' object has no attribute 'get'", py_tipo(valore)))),
    }
}

fn py_repr_percorso(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') { format!("\"{s}\"") } else { format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")) }
}

/// Il messaggio di un errore di sistema come lo scrive Python: "[Errno 13] Permission denied: '/percorso'".
pub fn messaggio_sistema(e: &io::Error, percorso: &Path, dest: Option<&Path>) -> String {
    let voce = match e.raw_os_error() {
        Some(2) => Some((2, "No such file or directory")),
        Some(5) => Some((5, "Input/output error")),
        Some(13) => Some((13, "Permission denied")),
        Some(17) => Some((17, "File exists")),
        Some(20) => Some((20, "Not a directory")),
        Some(21) => Some((21, "Is a directory")),
        Some(24) => Some((24, "Too many open files")),
        Some(28) => Some((28, "No space left on device")),
        Some(30) => Some((30, "Read-only file system")),
        Some(1) => Some((1, "Operation not permitted")),
        _ => None,
    };
    let Some((numero, testo)) = voce else { return e.to_string() };
    let file = if numero == 28 || numero == 30 { dest.unwrap_or(percorso) } else { percorso };
    format!("[Errno {numero}] {testo}: {}", py_repr_percorso(&file.to_string_lossy()))
}

pub enum ErroreLettura {
    /// Un errore di SQLite (sqlite3.DatabaseError in Python): l'unico che una lettura ritenta.
    Sqlite(String),
    Altro(Errore),
}

impl From<rusqlite::Error> for ErroreLettura {
    fn from(e: rusqlite::Error) -> Self {
        ErroreLettura::Sqlite(testo_sqlite(&e))
    }
}

impl From<Errore> for ErroreLettura {
    fn from(e: Errore) -> Self {
        ErroreLettura::Altro(e)
    }
}

fn testo_sqlite(e: &rusqlite::Error) -> String {
    // il testo di sqlite3_errmsg, come lo dà node:sqlite: senza la query e la posizione che rusqlite aggiunge
    match e {
        rusqlite::Error::SqliteFailure(_, Some(m)) | rusqlite::Error::SqlInputError { msg: m, .. } => m.clone(),
        altro => altro.to_string(),
    }
}

/// Un valore di SQLite come lo dà node:sqlite: interi e reali sono numeri, i blob byte numerati.
fn valore(v: ValueRef<'_>) -> Json {
    match v {
        ValueRef::Null => Json::Null,
        ValueRef::Integer(i) => Json::Num(i as f64),
        ValueRef::Real(r) => Json::Num(r),
        ValueRef::Text(t) => Json::Str(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Json::Obj(b.iter().enumerate().map(|(i, x)| (i.to_string(), Json::from(*x as u32))).collect()),
    }
}

/// Le righe di una query, ognuna come oggetto con le colonne per nome.
fn righe(db: &Connection, sql: &str, ids: &[i64]) -> Result<Vec<Mappa>, rusqlite::Error> {
    let mut st = db.prepare(sql)?;
    let nomi: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
    let mut q = st.query(rusqlite::params_from_iter(ids.iter()))?;
    let mut out = Vec::new();
    while let Some(r) = q.next()? {
        let mut m = Mappa::new();
        for (i, nome) in nomi.iter().enumerate() {
            m.insert(nome.as_str(), valore(r.get_ref(i)?));
        }
        out.push(m);
    }
    Ok(out)
}

fn intero(v: Option<&Json>) -> Option<i64> {
    match v {
        Some(Json::Num(n)) if n.fract() == 0.0 => Some(*n as i64),
        _ => None,
    }
}

const SQL_CALENDARI: &str = "select _id, account_name, account_type, calendar_displayName, isPrimary, deleted, sync_events from Calendars";
const SQL_EVENTI: &str = "select _id, _sync_id, calendar_id, title, eventLocation, description, eventStatus, dtstart, dtend, eventTimezone, eventEndTimezone, duration, allDay, rrule, rdate, exrule, exdate, original_id, original_sync_id, originalInstanceTime, originalAllDay, organizer, isOrganizer, hasAttendeeData from Events where deleted = 0 and calendar_id in (%s)";
// "Mostra come" di Outlook e Google (CalendarContract.Events.AVAILABILITY): 0 occupato, 1 libero, 2 provvisorio. A parte da SQL_EVENTI: la
// colonna c'è in ogni calendar.db di Android, ma non nei database di prova scritti per i golden dell'agente Python, che non la leggeva.
const SQL_OCCUPATI: &str = "select _id from Events where deleted = 0 and availability = 0 and calendar_id in (%s)";
const SQL_INVITATI: &str = "select event_id, attendeeName, attendeeEmail, attendeeRelationship from Attendees where event_id in (select _id from Events where calendar_id in (%s))";

/// _estrai: i calendari della casella, i loro eventi e chi partecipa. None se la casella non è sul telefono. `tipo` riceve il tipo di ogni
/// calendario della casella che si incontra.
pub fn estrai(db: &Connection, account: &str, calendari: &[String], tipo: &mut Option<String>) -> Result<Option<Lettura>, ErroreLettura> {
    let mut cal: IndexMap<i64, String> = IndexMap::new();
    let mut sincronizzati: IndexMap<i64, String> = IndexMap::new();
    for r in righe(db, SQL_CALENDARI, &[])? {
        let id = intero(r.get("_id")).unwrap_or_default();
        let conto = r.get("account_name").unwrap_or(&Json::Null);
        let tp = r.get("account_type").unwrap_or(&Json::Null);
        let nome = r.get("calendar_displayName").unwrap_or(&Json::Null);
        let vuoto = Json::from("");
        if minuscolo(if py_bool(conto) { conto } else { &vuoto })? != account {
            continue;
        }
        let testo_tipo = if py_bool(tp) { testo_js(tp) } else { String::new() };
        if !TIPI_ACCOUNT.iter().any(|t| testo_tipo.starts_with(t)) {
            continue;
        }
        if py_bool(r.get("deleted").unwrap_or(&Json::Null)) {
            continue;
        }
        *tipo = Some(if testo_tipo.starts_with("com.google") { "caldav" } else { "outlook" }.to_string());
        let etichetta = if py_bool(nome) { testo_js(nome) } else { "Calendario".to_string() };
        if py_bool(r.get("sync_events").unwrap_or(&Json::Null)) {
            sincronizzati.insert(id, etichetta.clone());
        }
        let scelto = if calendari.is_empty() {
            py_bool(r.get("isPrimary").unwrap_or(&Json::Null))
        } else {
            calendari.contains(&minuscolo(if py_bool(nome) { nome } else { &vuoto })?)
        };
        if scelto {
            cal.insert(id, etichetta);
        }
    }
    // Senza un calendario principale e senza scelta esplicita si prendono quelli che si sincronizzano.
    let scelti = if cal.is_empty() && calendari.is_empty() { sincronizzati } else { cal };
    if scelti.is_empty() {
        return Ok(None);
    }
    let segnaposti = vec!["?"; scelti.len()].join(",");
    let ids: Vec<i64> = scelti.keys().copied().collect();
    let eventi = righe(db, &SQL_EVENTI.replace("%s", &segnaposti), &ids)?;
    let mut persone: HashMap<i64, Persona> = HashMap::new();
    for r in righe(db, &SQL_INVITATI.replace("%s", &segnaposti), &ids)? {
        let Some(evento) = intero(r.get("event_id")) else { continue };
        let p = persone.entry(evento).or_default();
        let email = r.get("attendeeEmail").unwrap_or(&Json::Null);
        if py_bool(email) {
            p.invitati.push(minuscolo(email)?);
        }
        if r.get("attendeeRelationship") == Some(&Json::Num(ORGANIZZATORE)) {
            let nome = r.get("attendeeName").unwrap_or(&Json::Null);
            p.organizzatore = if py_bool(nome) { nome.clone() } else { email.clone() };
        }
    }
    let occupati = match righe(db, &SQL_OCCUPATI.replace("%s", &segnaposti), &ids) {
        Ok(v) => Some(v.iter().filter_map(|r| intero(r.get("_id"))).collect()),
        Err(e) if testo_sqlite(&e).contains("no such column: availability") => None,
        Err(e) => return Err(e.into()),
    };
    Ok(Some(Lettura { cal: scelti, eventi, persone, occupati }))
}

fn intero_js(v: Option<&Json>) -> Option<i64> {
    match v? {
        Json::Num(n) if n.is_finite() => Some(n.trunc() as i64),
        Json::Bool(b) => Some(i64::from(*b)),
        Json::Str(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| n.trunc() as i64),
        _ => None,
    }
}

fn testo_opz(v: Option<&Json>) -> Option<String> {
    match v? {
        Json::Null => None,
        altro => Some(testo_js(altro)),
    }
}

/// Una riga di Events (dal database o da un caso di prova) come la legge l'espansione.
pub fn riga_android(r: &Mappa) -> RigaAndroid {
    let i = |k: &str| intero_js(r.get(k));
    let t = |k: &str| testo_opz(r.get(k));
    RigaAndroid {
        id: i("_id").unwrap_or_default(),
        sync_id: t("_sync_id"),
        title: t("title"),
        event_location: t("eventLocation"),
        description: t("description"),
        event_status: i("eventStatus"),
        dtstart: i("dtstart"),
        dtend: i("dtend"),
        event_timezone: t("eventTimezone"),
        event_end_timezone: t("eventEndTimezone"),
        duration: t("duration"),
        all_day: i("allDay"),
        rrule: t("rrule"),
        rdate: t("rdate"),
        exrule: t("exrule"),
        exdate: t("exdate"),
        original_id: i("original_id"),
        original_sync_id: t("original_sync_id"),
        original_instance_time: i("originalInstanceTime"),
        original_all_day: i("originalAllDay"),
        is_organizer: i("isOrganizer"),
        has_attendee_data: i("hasAttendeeData"),
    }
}

#[derive(Clone, Debug, Default)]
struct Dettagli {
    calendario: String,
    email: String,
    organizzatore: Json,
    invitati: Vec<String>,
    serie: bool,
    occupato: Option<bool>,
}

/// Per ogni UID di iCalendar i campi che iCalendar non ha: il calendario, l'organizzatore, gli invitati, se viene da una serie, se è "Occupato".
fn dettagli_di(lettura: &Lettura, righe: &[RigaAndroid]) -> HashMap<String, Dettagli> {
    let mut dettagli: HashMap<String, Dettagli> = HashMap::new();
    let mut per_sync: HashMap<&str, i64> = HashMap::new();
    for r in righe {
        if let Some(s) = r.sync_id.as_deref().filter(|s| !s.is_empty()) {
            per_sync.insert(s, r.id);
        }
    }
    for (r, grezza) in righe.iter().zip(&lettura.eventi) {
        let serie = r
            .original_id
            .filter(|x| *x != 0)
            .or_else(|| r.original_sync_id.as_deref().filter(|s| !s.is_empty()).and_then(|s| per_sync.get(s).copied()))
            .filter(|x| *x != 0);
        if serie.is_some() && annullata(r.event_status, r.is_organizer, r.has_attendee_data, true) && r.original_instance_time.is_some() {
            continue;
        }
        if annullata(r.event_status, r.is_organizer, r.has_attendee_data, serie.is_some()) || r.dtstart.is_none() {
            continue;
        }
        let uid = format!("a{}", serie.unwrap_or(r.id));
        let p = lettura.persone.get(&r.id);
        let organizer = grezza.get("organizer").filter(|v| !v.is_null());
        let d = dettagli.entry(uid).or_insert_with(|| {
            let mut invitati: Vec<String> = Vec::new();
            for x in p.map(|p| p.invitati.as_slice()).unwrap_or_default() {
                if !invitati.contains(x) {
                    invitati.push(x.clone());
                }
            }
            invitati.sort();
            let organizzatore = match p.map(|p| &p.organizzatore).filter(|o| vero_js(o)) {
                Some(o) => o.clone(),
                None => organizer.filter(|o| vero_js(o)).cloned().unwrap_or_else(|| Json::from("")),
            };
            Dettagli {
                calendario: intero_js(grezza.get("calendar_id")).and_then(|c| lettura.cal.get(&c)).cloned().unwrap_or_default(),
                email: organizer.map(|o| testo_js(o).to_lowercase()).unwrap_or_default(),
                organizzatore,
                invitati,
                serie: false,
                occupato: None,
            }
        });
        if serie.is_some() || non_vuota(&r.rrule) || non_vuota(&r.rdate) {
            d.serie = true;
        }
        // di una serie vale la riga della serie, non quella di un'occorrenza cambiata
        if let (Some(occupati), None) = (&lettura.occupati, serie) {
            d.occupato = Some(occupati.contains(&r.id));
        }
    }
    dettagli
}

fn non_vuota(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|x| !x.is_empty())
}

/// Il confronto fra tuple di Python: la prima posizione che differisce decide.
fn peso_maggiore(a: &[bool; 4], b: &[bool; 4]) -> bool {
    a.iter().zip(b).find(|(x, y)| x != y).is_some_and(|(x, _)| *x)
}

pub struct InfoCasella<'a> {
    pub id: &'a str,
    pub nome: &'a str,
    pub account: &'a str,
    pub tipo: Option<&'a str>,
}

/// Gli eventi come li restituisce CalendarioAndroid.eventi: un'occorrenza per titolo, inizio e fine (la più completa), in ordine di inizio e
/// titolo. Outlook scrive una serie due volte, la serie con la sua regola e ogni occorrenza come appuntamento a sé: espandendo la serie le
/// occorrenze raddoppiano. Si tiene prima quella scritta da Outlook per l'occorrenza (ha il luogo e il link di quel giorno), poi quella con
/// link, luogo e organizzatore.
pub fn eventi_da_lettura(lettura: &Lettura, info: &InfoCasella<'_>, inizio: DateTime, fine: DateTime, zona: &str, grezzi: Option<Vec<OccorrenzaAndroid>>) -> Esito<Vec<Json>> {
    let righe: Vec<RigaAndroid> = lettura.eventi.iter().map(riga_android).collect();
    let occorrenze = match grezzi {
        Some(g) => g,
        None => espandi_righe_android(&righe, inizio, fine, Some(zona))?,
    };
    let dettagli = dettagli_di(lettura, &righe);
    let mut candidati: IndexMap<String, ([bool; 4], Mappa)> = IndexMap::new();
    for o in occorrenze {
        let d = dettagli.get(&o.uid);
        let link = MEETING_URL.find(&o.luogo).or_else(|| MEETING_URL.find(&o.descrizione));
        let online_url = match link {
            Some(l) => Some(pulisci_link(l.as_str())?),
            None => None,
        };
        let titolo = if o.titolo.is_empty() { "(senza titolo)".to_string() } else { o.titolo };
        let email = d.map(|d| d.email.clone()).unwrap_or_default();
        let peso = [!d.is_some_and(|d| d.serie), online_url.as_deref().is_some_and(|u| !u.is_empty()), !o.luogo.is_empty(), !email.is_empty()];
        let chiave = Json::Arr(vec![Json::from(titolo.as_str()), Json::from(o.inizio.as_str()), Json::from(o.fine.as_str())]).in_js();
        let mut e = Mappa::new();
        e.insert("id", format!("{}|{}|{}", info.id, o.uid, o.inizio));
        e.insert("account", info.account);
        e.insert("alias", info.nome);
        e.insert("calendar", format!("{} · {}", info.account, d.map(|d| d.calendario.as_str()).unwrap_or_default()));
        e.insert("title", titolo);
        e.insert("start", o.inizio);
        e.insert("end", o.fine);
        e.insert("allDay", o.tutto_giorno);
        e.insert("location", o.luogo);
        e.insert("organizer", d.map_or_else(|| Json::from(""), |d| d.organizzatore.clone()));
        e.insert("organizer_email", email);
        e.insert("attendees", d.map(|d| d.invitati.clone()).unwrap_or_default());
        e.insert("online_url", online_url);
        e.insert("web_link", Json::Null);
        e.insert("fonte", "android");
        e.insert("tipo", info.tipo);
        if let Some(occupato) = d.and_then(|d| d.occupato) {
            e.insert("occupato", occupato);
        }
        let sostituisci = match candidati.get(&chiave) {
            None => true,
            Some((attuale, _)) => peso_maggiore(&peso, attuale),
        };
        if sostituisci {
            candidati.insert(chiave, (peso, e));
        }
    }
    let mut eventi: Vec<Mappa> = candidati.into_values().map(|(_, e)| e).collect();
    let testo = |m: &Mappa, k: &str| m.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    eventi.sort_by(|a, b| testo(a, "start").cmp(&testo(b, "start")).then_with(|| testo(a, "title").cmp(&testo(b, "title"))));
    Ok(eventi.into_iter().map(Json::Obj).collect())
}

thread_local! {
    static CONTATORE: Cell<u64> = const { Cell::new(0) };
}

/// mkdtemp: una cartella nuova, solo nostra, nella cartella temporanea.
fn cartella_temporanea(prefisso: &str) -> io::Result<PathBuf> {
    const LETTERE: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let base = std::env::temp_dir();
    loop {
        let n = CONTATORE.with(|c| {
            c.set(c.get() + 1);
            c.get()
        });
        let mut x = adesso_vero() as u64 ^ (u64::from(std::process::id()) << 32) ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let coda: String = (0..6)
            .map(|_| {
                let c = LETTERE[(x % LETTERE.len() as u64) as usize] as char;
                x /= LETTERE.len() as u64;
                c
            })
            .collect();
        let dir = base.join(format!("{prefisso}{coda}"));
        let mut b = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        match b.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// copyFileSync: aprire una cartella riesce, leggerla no (EISDIR), come per Node.
fn copia(sorgente: &Path, dest: &Path) -> Result<(), String> {
    let errore = |e: io::Error| messaggio_sistema(&e, sorgente, Some(dest));
    let mut r = File::open(sorgente).map_err(errore)?;
    let mut w = File::create(dest).map_err(errore)?;
    io::copy(&mut r, &mut w).map_err(errore)?;
    Ok(())
}

/// Copia il database con i suoi file di appoggio in una cartella nuova (Android lo tiene aperto) e lo apre.
fn copia_e_apri(cartella: &Path) -> Result<(PathBuf, Connection), String> {
    let tmp = cartella_temporanea("android_").map_err(|e| messaggio_sistema(&e, &std::env::temp_dir().join("android_XXXXXX"), None))?;
    let esito = (|| {
        for suffisso in ["", "-journal", "-wal", "-shm"] {
            let nome = format!("{DB}{suffisso}");
            let sorgente = cartella.join(&nome);
            if sorgente.exists() {
                copia(&sorgente, &tmp.join(&nome))?;
            }
        }
        Connection::open(tmp.join(DB)).map_err(|e| testo_sqlite(&e))
    })();
    match esito {
        Ok(db) => Ok((tmp, db)),
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

/// "HH:MM" di un istante nel fuso, come `toLocaleTimeString("en-GB", {hour: "2-digit", minute: "2-digit", hourCycle: "h23"})`.
pub fn ora_minuti(ms: i64, zona: &str) -> String {
    let tz = jiff::tz::TimeZone::get(zona).unwrap_or(jiff::tz::TimeZone::UTC);
    let t = jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_zoned(tz);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

const fn gravita(stato: &str) -> u8 {
    match stato.as_bytes() {
        b"avviso" => 1,
        b"errore" => 2,
        _ => 0,
    }
}

pub struct CalendarioAndroid {
    pub id: String,
    pub nome: String,
    pub account: String,
    pub cartella: PathBuf,
    pub calendari: Vec<String>,
    ctx: Contesto,
    stato: RefCell<Stato>,
    /// Chi tiene aggiornata la casella sul telefono: "outlook" o "caldav" (servizi Google: il nome resta "caldav" perché l'app sul PC lo conosce).
    tipo: RefCell<Option<String>>,
    /// Appuntamenti presenti sul telefono.
    conteggio: Cell<Option<usize>>,
}

impl CalendarioAndroid {
    pub fn nuovo(id: String, nome: String, account: &str, cartella: PathBuf, calendari: Option<Vec<String>>, ctx: Contesto) -> Self {
        CalendarioAndroid {
            id,
            nome,
            account: py_strip(account).to_lowercase(),
            cartella,
            calendari: calendari.unwrap_or_default().iter().map(|c| py_strip(c).to_lowercase()).collect(),
            ctx,
            stato: RefCell::new(Stato::non_letto()),
            tipo: RefCell::new(None),
            conteggio: Cell::new(None),
        }
    }

    pub fn stato(&self) -> Stato {
        self.stato.borrow().clone()
    }

    pub fn tipo(&self) -> Option<String> {
        self.tipo.borrow().clone()
    }

    fn segna(&self, stato: &str, msg: Json) {
        *self.stato.borrow_mut() = Stato { stato: stato.to_string(), msg };
    }

    /// Legge il telefono: una copia di calendar.db presa a metà scrittura si ritenta una volta, dopo un secondo.
    async fn leggi(&self) -> Result<Option<Lettura>, String> {
        if !self.cartella.join(DB).exists() {
            return Err("calendario del telefono non trovato".to_string());
        }
        let mut ultimo = String::new();
        for tentativo in 0..2 {
            let (tmp, db) = copia_e_apri(&self.cartella)?;
            // la copia è nostra: un journal "caldo" (copia presa a metà scrittura) si riavvolge, come con sqlite3.connect di Python; query_only
            // impedisce che una nostra istruzione scriva
            let mut tipo = self.tipo();
            let esito = db.execute_batch("PRAGMA query_only = ON").map_err(ErroreLettura::from).and_then(|()| estrai(&db, &self.account, &self.calendari, &mut tipo));
            *self.tipo.borrow_mut() = tipo;
            drop(db);
            let _ = fs::remove_dir_all(&tmp);
            match esito {
                Ok(l) => return Ok(l),
                Err(ErroreLettura::Altro(e)) => return Err(e.messaggio().to_string()),
                Err(ErroreLettura::Sqlite(m)) => ultimo = m,
            }
            if tentativo == 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(ultimo)
    }

    /// Legge il telefono e aggiorna lo stato. -> (dati grezzi o None, errori)
    pub async fn leggi_e_segna(&self) -> Esito<(Option<Lettura>, Vec<String>)> {
        let grezzi = match self.leggi().await {
            Ok(g) => g,
            Err(testo) => {
                (self.ctx.registro)(Livello::Warning, &format!("{}: lettura non riuscita ({testo})", self.nome));
                self.segna("errore", Json::from(format!("Calendario del telefono non leggibile ({testo})")));
                return Ok((None, vec![format!("{}: {testo}", self.nome)]));
            }
        };
        let Some(grezzi) = grezzi else {
            let msg = format!("La casella {} non e' sul telefono (login da rifare?)", self.account);
            let errore = format!("{}: {msg}", self.nome);
            self.segna("errore", Json::from(msg));
            return Ok((None, vec![errore]));
        };
        self.conteggio.set(Some(grezzi.eventi.len()));
        // Si dice solo cosa si è verificato davvero: la lettura è riuscita. L'ora di scrittura di calendar.db non dice niente di Outlook (la
        // muove ogni 2 minuti la sincronizzazione Google): la freschezza di ogni casella la misura il rilevatore del supervisore.
        let ora = ora_minuti((self.ctx.adesso)(), &self.ctx.zona);
        self.segna("ok", Json::from(format!("Letto dal telefono alle {ora}")));
        self.applica_stato_accesso()?;
        Ok((Some(grezzi), Vec::new()))
    }

    /// Se l'accesso alla casella sul telefono non è a posto, lo dice il pallino. La lettura riesce anche con il login scaduto (sono dati già
    /// sul telefono): il verdetto arriva dal rilevatore. Si applica solo se peggiora lo stato della lettura.
    fn applica_stato_accesso(&self) -> Esito<()> {
        let accessi = leggi_accessi(&self.ctx.accessi);
        let v = prendi(&accessi, &self.account)?;
        if !py_bool(v) {
            return Ok(());
        }
        let s = prendi(v, "stato")?;
        let stato = if py_bool(s) { s.clone() } else { Json::from("ok") };
        if matches!(stato, Json::Arr(_) | Json::Obj(_)) {
            return Err(Errore::Altro(format!("unhashable type: '{}'", py_tipo(&stato))));
        }
        let Json::Str(stato) = stato else { return Ok(()) };
        let attuale = self.stato();
        if gravita(&stato) > gravita(&attuale.stato) {
            let msg = prendi(v, "msg")?;
            let msg = if py_bool(msg) { msg.clone() } else { attuale.msg };
            self.segna(&stato, msg);
        }
        Ok(())
    }

    /// La casella come la vede l'app: chi è, com'è andata l'ultima lettura.
    pub fn descrizione(&self) -> Json {
        let stato = self.stato();
        let mut m = Mappa::new();
        m.insert("id", self.id.as_str());
        m.insert("account", self.account.as_str());
        m.insert("nome", self.nome.as_str());
        m.insert("tipo", self.tipo());
        m.insert("eventi", self.conteggio.get());
        m.insert("stato", stato.stato);
        m.insert("msg", stato.msg);
        Json::Obj(m)
    }

    /// Come descrizione(), ma dopo una lettura fresca (per l'elenco dei calendari).
    pub async fn riepilogo(&self) -> Esito<Json> {
        self.leggi_e_segna().await?;
        Ok(self.descrizione())
    }

    /// Eventi fra due date locali senza fuso. -> (eventi, errori)
    pub async fn eventi(&self, inizio: DateTime, fine: DateTime) -> Esito<(Vec<Json>, Vec<String>)> {
        let (grezzi, errori) = self.leggi_e_segna().await?;
        let Some(grezzi) = grezzi else { return Ok((Vec::new(), errori)) };
        let tipo = self.tipo();
        let info = InfoCasella { id: &self.id, nome: &self.nome, account: &self.account, tipo: tipo.as_deref() };
        Ok((eventi_da_lettura(&grezzi, &info, inizio, fine, &self.ctx.zona, None)?, Vec::new()))
    }
}

/// Una casella da leggere: dal telefono, o (dopo un accesso) direttamente dal fornitore.
pub enum Fonte {
    Android(CalendarioAndroid),
    Diretto(crate::diretto::CalendarioDiretto),
}

impl Fonte {
    pub fn id(&self) -> &str {
        match self {
            Fonte::Android(f) => &f.id,
            Fonte::Diretto(f) => &f.id,
        }
    }

    pub fn nome(&self) -> &str {
        match self {
            Fonte::Android(f) => &f.nome,
            Fonte::Diretto(f) => &f.nome,
        }
    }

    pub fn account(&self) -> &str {
        match self {
            Fonte::Android(f) => &f.account,
            Fonte::Diretto(f) => &f.account,
        }
    }

    pub fn tipo(&self) -> Option<String> {
        match self {
            Fonte::Android(f) => f.tipo(),
            Fonte::Diretto(f) => f.tipo(),
        }
    }

    pub fn stato(&self) -> Stato {
        match self {
            Fonte::Android(f) => f.stato(),
            Fonte::Diretto(f) => f.stato(),
        }
    }

    pub fn descrizione(&self) -> Json {
        match self {
            Fonte::Android(f) => f.descrizione(),
            Fonte::Diretto(f) => f.descrizione(),
        }
    }

    pub fn diretta(&self) -> bool {
        matches!(self, Fonte::Diretto(_))
    }

    pub async fn riepilogo(&self) -> Esito<Json> {
        match self {
            Fonte::Android(f) => f.riepilogo().await,
            Fonte::Diretto(f) => f.riepilogo().await,
        }
    }

    pub async fn eventi(&self, inizio: DateTime, fine: DateTime) -> Esito<(Vec<Json>, Vec<String>)> {
        match self {
            Fonte::Android(f) => f.eventi(inizio, fine).await,
            Fonte::Diretto(f) => f.eventi(inizio, fine).await,
        }
    }

    pub fn svuota_cache(&self) {
        if let Fonte::Diretto(f) = self {
            f.svuota_cache();
        }
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn errori_di_sistema_come_python() {
        let e = io::Error::from_raw_os_error(21);
        assert_eq!(messaggio_sistema(&e, Path::new("/x/calendar.db"), None), "[Errno 21] Is a directory: '/x/calendar.db'");
        let e = io::Error::from_raw_os_error(30);
        assert_eq!(messaggio_sistema(&e, Path::new("/a"), Some(Path::new("/b/it's"))), "[Errno 30] Read-only file system: \"/b/it's\"");
    }

    #[test]
    fn copiare_una_cartella_e_eisdir() {
        let dir = cartella_temporanea("prova_").unwrap();
        let sotto = dir.join("calendar.db");
        fs::create_dir(&sotto).unwrap();
        let errore = copia(&sotto, &dir.join("copia")).unwrap_err();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(errore, format!("[Errno 21] Is a directory: '{}'", sotto.display()));
    }

    #[test]
    fn ora_e_valori() {
        assert_eq!(ora_minuti(1_794_370_257_000, "Europe/Rome"), "05:10");
        assert_eq!(testo_js(&Json::Num(1.0)), "1");
        assert!(!vero_js(&Json::Num(f64::NAN)));
    }
}
