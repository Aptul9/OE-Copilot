//! La sessione web di Google (i cookie di un Chrome già entrato) per le parti di Google che non hanno un'API con OAuth usabile: Google Chat.
//! Si importa dal PC (scripts/import-google.ts) con POST /api/auth/completa {"provider": "gchat", "sessione": {...}}, sta in
//! /stato/google-web.json (0600) e si tiene viva da sola: i cookie che Google ruota nelle risposte si salvano, e quando Google rifiuta la
//! sessione si prova una volta accounts.google.com/RotateCookies prima di dire che va importata di nuovo.
//! I cookie escono solo verso google.com e i suoi sottodomini, scelti per host e percorso come li sceglie il browser; i salti verso
//! googleusercontent.com restano senza cookie.

use crate::js::js_trim;
use crate::sessione_web::data_cookie;
use crate::trasporto::{Richiesta, Trasporto};
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct CookieGoogle {
    pub name: String,
    pub value: String,
    /// ".google.com" vale per il dominio e i sottodomini; senza punto iniziale solo per quell'host.
    pub domain: String,
    pub path: String,
    /// Secondi Unix; 0 per un cookie di sessione.
    pub expires: f64,
}

impl CookieGoogle {
    pub fn in_json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("name", self.name.as_str());
        m.insert("value", self.value.as_str());
        m.insert("domain", self.domain.as_str());
        m.insert("path", self.path.as_str());
        m.insert("expires", self.expires);
        Json::Obj(m)
    }

    /// Un cookie salvato: le tre stringhe ci devono essere, il resto prende il valore vuoto.
    fn da_json(v: &Json) -> Option<CookieGoogle> {
        let s = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
        Some(CookieGoogle {
            name: s("name")?,
            value: s("value")?,
            domain: s("domain")?,
            path: s("path").unwrap_or_default(),
            expires: v.get("expires").and_then(Json::as_f64).unwrap_or(0.0),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessioneGoogleWeb {
    pub account: String,
    pub user_agent: String,
    pub cookies: Vec<CookieGoogle>,
    /// L'id utente di Chat (gaia) di chi ha la sessione.
    pub self_id: Option<String>,
    pub salvata: f64,
    pub errore: Option<String>,
}

impl SessioneGoogleWeb {
    /// Nell'ordine in cui l'importazione costruisce la sessione: i dati letti, poi la casella e l'ora, poi l'id di Chat e l'errore.
    pub fn in_json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("user_agent", self.user_agent.as_str());
        m.insert("cookies", Json::Arr(self.cookies.iter().map(CookieGoogle::in_json).collect()));
        m.insert("account", self.account.as_str());
        m.insert("salvata", self.salvata);
        if let Some(id) = &self.self_id {
            m.insert("self_id", id.as_str());
        }
        if let Some(e) = &self.errore {
            m.insert("errore", e.as_str());
        }
        Json::Obj(m)
    }

    fn da_json(v: &Json) -> SessioneGoogleWeb {
        let s = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
        SessioneGoogleWeb {
            account: s("account").unwrap_or_default(),
            user_agent: s("user_agent").unwrap_or_default(),
            cookies: v.get("cookies").and_then(Json::as_arr).map(|c| c.iter().filter_map(CookieGoogle::da_json).collect()).unwrap_or_default(),
            self_id: s("self_id"),
            salvata: v.get("salvata").and_then(Json::as_f64).unwrap_or(0.0),
            errore: s("errore"),
        }
    }
}

/// Una sessione condivisa fra l'archivio e i client di Chat: chi la tiene sa se è ancora quella dell'archivio (`Rc::ptr_eq`).
pub type SessioneCondivisa = Rc<RefCell<SessioneGoogleWeb>>;

pub const UA_PREDEFINITO: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";
// i cookie che RotateCookies rinnova (gweb, store/renew.go)
const RUOTATI: [&str; 5] = ["SIDCC", "__Secure-1PSIDCC", "__Secure-3PSIDCC", "__Secure-1PSIDTS", "__Secure-3PSIDTS"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErroreSessioneGoogle(pub String);

impl fmt::Display for ErroreSessioneGoogle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ErroreSessioneGoogle {}

/// Gli errori di Gmail e di Chat come li tratta il server: `GmailError` e `GoogleChatError` danno il loro codice e il messaggio,
/// `ErroreSessioneGoogle` un 400, tutto il resto un 502 senza dettagli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroreGoogle {
    Http { messaggio: String, status: u16 },
    Sessione(String),
    Altro(String),
}

impl ErroreGoogle {
    pub fn http(messaggio: impl Into<String>, status: u16) -> Self {
        ErroreGoogle::Http { messaggio: messaggio.into(), status }
    }

    pub fn status(&self) -> u16 {
        match self {
            ErroreGoogle::Http { status, .. } => *status,
            ErroreGoogle::Sessione(_) => 400,
            ErroreGoogle::Altro(_) => 502,
        }
    }
}

impl fmt::Display for ErroreGoogle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroreGoogle::Http { messaggio, .. } | ErroreGoogle::Sessione(messaggio) | ErroreGoogle::Altro(messaggio) => f.write_str(messaggio),
        }
    }
}

impl std::error::Error for ErroreGoogle {}

impl From<ErroreSessioneGoogle> for ErroreGoogle {
    fn from(e: ErroreSessioneGoogle) -> Self {
        ErroreGoogle::Sessione(e.0)
    }
}

/// `/^[!#$%&'*+\-.^_`|~0-9A-Za-z]{1,128}$/`
fn nome_cookie(n: &str) -> bool {
    (1..=128).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// `/^[\x21\x23-\x2b\x2d-\x3a\x3c-\x5b\x5d-\x7e]{0,4096}$/`
fn valore_cookie(v: &str) -> bool {
    v.len() <= 4096 && v.bytes().all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
}

pub fn host_google(host: &str) -> bool {
    host == "google.com" || host.ends_with(".google.com")
}

fn dominio_google(dominio: &str) -> bool {
    host_google(dominio.strip_prefix('.').unwrap_or(dominio))
}

fn corrisponde(c: &CookieGoogle, host: &str, percorso: &str, adesso_s: f64) -> bool {
    if c.expires > 0.0 && c.expires <= adesso_s {
        return false;
    }
    match c.domain.strip_prefix('.') {
        Some(d) => {
            if host != d && !host.ends_with(&format!(".{d}")) {
                return false;
            }
        }
        None => {
            if host != c.domain {
                return false;
            }
        }
    }
    let p = if c.path.is_empty() { "/" } else { c.path.as_str() };
    percorso == p || (percorso.starts_with(p) && (p.ends_with('/') || percorso.as_bytes().get(p.len()) == Some(&b'/')))
}

/// Per ogni nome il cookie che il browser manderebbe a host+percorso: quello dell'host vince su quello del dominio.
pub fn cookie_per<'a>(cookies: &'a [CookieGoogle], host: &str, percorso: &str, adesso_s: f64) -> IndexMap<String, &'a CookieGoogle> {
    let mut out: IndexMap<String, &CookieGoogle> = IndexMap::new();
    if !host_google(host) {
        return out;
    }
    for c in cookies {
        if !corrisponde(c, host, percorso, adesso_s) {
            continue;
        }
        if out.get(&c.name).is_some_and(|prima| !prima.domain.starts_with('.') && c.domain.starts_with('.')) {
            continue;
        }
        out.insert(c.name.clone(), c);
    }
    out
}

/// L'intestazione Cookie per host+percorso, "" fuori da google.com.
pub fn intestazione_cookie(cookies: &[CookieGoogle], host: &str, percorso: &str, adesso_s: f64) -> String {
    let mut coppie: Vec<String> = cookie_per(cookies, host, percorso, adesso_s).values().map(|c| format!("{}={}", c.name, c.value)).collect();
    coppie.sort();
    coppie.join("; ")
}

/// Un Set-Cookie letto: il cookie nella forma salvata e se è già scaduto (`via`).
pub struct SetCookie {
    pub cookie: CookieGoogle,
    pub via: bool,
}

/// Un Set-Cookie ricevuto da `host`, nella forma salvata; None se non va tenuto (fuori da google.com, nome o valore strani).
pub fn leggi_set_cookie(riga: &str, host: &str, adesso_s: f64) -> Option<SetCookie> {
    let mut pezzi = riga.split(';');
    let coppia = pezzi.next().unwrap_or_default();
    let uguale = coppia.find('=').filter(|i| *i >= 1)?;
    let name = js_trim(&coppia[..uguale]).to_string();
    let value = js_trim(&coppia[uguale + 1..]).to_string();
    if !nome_cookie(&name) || !valore_cookie(&value) || !host_google(host) {
        return None;
    }
    let mut domain = host.to_string();
    let mut percorso = "/".to_string();
    let mut expires = 0.0;
    let mut max_age: Option<f64> = None;
    for a in pezzi {
        let (k, v) = match a.find('=') {
            Some(i) => (js_trim(&a[..i]).to_lowercase(), js_trim(&a[i + 1..])),
            None => (js_trim(a).to_lowercase(), ""),
        };
        if k == "domain" && !v.is_empty() {
            let d = v.strip_prefix('.').unwrap_or(v).to_lowercase();
            // un dominio che non contiene l'host, o fuori da google.com, rende il cookie non valido
            if (host != d && !host.ends_with(&format!(".{d}"))) || !host_google(&d) {
                return None;
            }
            domain = format!(".{d}");
        } else if k == "path" && v.starts_with('/') {
            percorso = v.to_string();
        } else if k == "max-age" && !v.is_empty() && v.strip_prefix('-').unwrap_or(v).bytes().all(|b| b.is_ascii_digit()) && v != "-" {
            max_age = v.parse().ok();
        } else if k == "expires" {
            if let Some(t) = data_cookie(v) {
                expires = (t / 1000.0).floor();
            }
        }
    }
    if let Some(m) = max_age {
        expires = if m <= 0.0 { -1.0 } else { adesso_s + m };
    }
    Some(SetCookie { cookie: CookieGoogle { name, value, domain, path: percorso, expires }, via: expires != 0.0 && expires <= adesso_s })
}

/// Assorbe i Set-Cookie di una risposta di `host`: sostituisce lo stesso nome, dominio e percorso, toglie quelli scaduti. true se è cambiato
/// qualcosa.
pub fn assorbi_cookie<S: AsRef<str>>(cookies: &mut Vec<CookieGoogle>, host: &str, righe: &[S], adesso_s: f64, solo_nomi: Option<&[&str]>) -> bool {
    let mut cambiato = false;
    for riga in righe {
        let Some(SetCookie { cookie: c, via }) = leggi_set_cookie(riga.as_ref(), host, adesso_s) else { continue };
        if solo_nomi.is_some_and(|n| !n.contains(&c.name.as_str())) {
            continue;
        }
        let i = cookies.iter().position(|x| x.name == c.name && x.domain == c.domain && x.path == c.path);
        if via {
            if let Some(i) = i {
                cookies.remove(i);
                cambiato = true;
            }
            continue;
        }
        match i {
            None => cookies.push(c),
            Some(i) if cookies[i].value != c.value || cookies[i].expires != c.expires => cookies[i] = c,
            Some(_) => continue,
        }
        cambiato = true;
    }
    cambiato
}

/// La sessione importata: user_agent e cookie nel formato di gweb e di Network.getCookies (name, value, domain, path, expires). Restano solo
/// i cookie di google.com; senza SID non è una sessione Google e si rifiuta.
pub fn leggi_sessione_importata(dati: &Json, adesso_s: f64) -> Result<(String, Vec<CookieGoogle>), ErroreSessioneGoogle> {
    let rifiuta = |m: &str| Err(ErroreSessioneGoogle(m.to_string()));
    let Json::Obj(r) = dati else { return rifiuta("serve {\"user_agent\": ..., \"cookies\": [...]}") };
    let Some(Json::Arr(elenco)) = r.get("cookies") else { return rifiuta("nella sessione mancano i cookies") };
    if elenco.len() > 1000 {
        return rifiuta("too many cookies");
    }
    let mut cookies = Vec::new();
    for c in elenco {
        let (Some(Json::Str(name)), Some(Json::Str(value)), Some(Json::Str(domain))) = (c.get("name"), c.get("value"), c.get("domain")) else { continue };
        let dominio = domain.to_lowercase();
        if !nome_cookie(name) || !valore_cookie(value) || !dominio_google(&dominio) {
            continue;
        }
        let expires = match c.get("expires") {
            Some(Json::Num(n)) if n.is_finite() && *n > 0.0 => n.floor(),
            _ => 0.0,
        };
        if expires != 0.0 && expires <= adesso_s {
            continue;
        }
        let path = match c.get("path") {
            Some(Json::Str(p)) if p.starts_with('/') => p.clone(),
            _ => "/".to_string(),
        };
        cookies.push(CookieGoogle { name: name.clone(), value: value.clone(), domain: dominio, path, expires });
    }
    if !cookies.iter().any(|c| c.name == "SID" && c.domain == ".google.com") {
        return rifiuta("nella sessione manca il cookie SID di google.com: il profilo non è entrato in Google");
    }
    let ua = match r.get("user_agent") {
        Some(Json::Str(u)) if (1..=512).contains(&u.len()) && u.bytes().all(|b| (0x20..=0x7e).contains(&b)) => u.trim_matches(' ').replacen("HeadlessChrome", "Chrome", 1),
        _ => UA_PREDEFINITO.to_string(),
    };
    Ok((ua, cookies))
}

/// Rinnova i cookie di verifica (SIDCC, __Secure-*PSIDTS...) con accounts.google.com/RotateCookies, come fa il browser. true se Google ne ha
/// dati di nuovi: chi chiama deve comunque riprovare la lettura vera, una rotazione riuscita non prova che la sessione valga ancora.
pub async fn ruota_cookie(s: &SessioneCondivisa, trasporto: &Rc<dyn Trasporto>, adesso_s: f64) -> bool {
    let (ua, cookie) = {
        let s = s.borrow();
        (s.user_agent.clone(), intestazione_cookie(&s.cookies, "accounts.google.com", "/RotateCookies", adesso_s))
    };
    let r = Richiesta::post("https://accounts.google.com/RotateCookies", "[000,\"-0000000000000000000\"]")
        .intestazione("Content-Type", "application/json")
        .intestazione("Origin", "https://accounts.google.com")
        .intestazione("User-Agent", ua)
        .intestazione("Cookie", cookie)
        .senza_redirect()
        .scadenza(Duration::from_millis(25_000));
    let Ok(risposta) = trasporto.invia(r).await else { return false };
    if risposta.stato != 200 {
        return false;
    }
    let validi: Vec<&str> = risposta
        .intestazioni
        .tutti("set-cookie")
        .filter(|riga| {
            leggi_set_cookie(riga, "accounts.google.com", adesso_s)
                .is_some_and(|c| c.cookie.domain == ".google.com" && c.cookie.path == "/" && c.cookie.expires > adesso_s && !c.via && !c.cookie.value.is_empty())
        })
        .collect();
    assorbi_cookie(&mut s.borrow_mut().cookies, "accounts.google.com", &validi, adesso_s, Some(&RUOTATI))
}

#[derive(Default)]
struct DatiArchivio {
    sessioni: IndexMap<String, SessioneCondivisa>,
    /// id utente di Chat -> nome, per non richiederli a ogni lettura.
    nomi: Option<Mappa>,
}

/// /stato/google-web.json: le sessioni per casella e la cache dei nomi. Si rilegge quando cambia sul disco, si scrive atomico in 0600.
pub struct ArchivioGoogleWeb {
    pub percorso: PathBuf,
    mtime: Cell<f64>,
    dati: RefCell<DatiArchivio>,
}

fn mtime_ms(p: &Path) -> std::io::Result<f64> {
    let m = std::fs::metadata(p)?.modified()?;
    Ok(m.duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0))
}

impl ArchivioGoogleWeb {
    pub fn nuovo(percorso: PathBuf) -> Self {
        ArchivioGoogleWeb { percorso, mtime: Cell::new(-1.0), dati: RefCell::new(DatiArchivio::default()) }
    }

    fn carica(&self) -> Result<(), String> {
        if !self.percorso.exists() {
            if self.mtime.get() != 0.0 {
                *self.dati.borrow_mut() = DatiArchivio::default();
            }
            self.mtime.set(0.0);
            return Ok(());
        }
        let m = mtime_ms(&self.percorso).map_err(|e| e.to_string())?;
        if m == self.mtime.get() {
            return Ok(());
        }
        let testo = crate::file::leggi_utf8(&self.percorso).map_err(|e| e.to_string())?;
        let raw = Json::analizza(&testo).map_err(|e| e.messaggio)?;
        let Some(Json::Obj(grezze)) = raw.get("sessioni") else { return Err("Invalid Google session store".to_string()) };
        let mut sessioni = IndexMap::new();
        for (k, v) in grezze.iter() {
            let account = v.get("account").map(|a| a.as_str().map(str::to_lowercase));
            if !matches!(v, Json::Obj(_)) || !matches!(v.get("cookies"), Some(Json::Arr(_))) || account != Some(Some(k.to_lowercase())) {
                return Err("Invalid saved Google session".to_string());
            }
            sessioni.insert(k.to_lowercase(), Rc::new(RefCell::new(SessioneGoogleWeb::da_json(v))));
        }
        let nomi = match raw.get("nomi") {
            Some(Json::Obj(n)) => n.clone(),
            _ => Mappa::new(),
        };
        *self.dati.borrow_mut() = DatiArchivio { sessioni, nomi: Some(nomi) };
        self.mtime.set(m);
        Ok(())
    }

    pub fn salva(&self) -> Result<(), String> {
        let dati = {
            let d = self.dati.borrow();
            let mut sessioni = Mappa::new();
            for (k, s) in &d.sessioni {
                sessioni.insert(k.as_str(), s.borrow().in_json());
            }
            let mut m = Mappa::new();
            m.insert("sessioni", Json::Obj(sessioni));
            if let Some(n) = &d.nomi {
                m.insert("nomi", Json::Obj(n.clone()));
            }
            Json::Obj(m)
        };
        if let Some(cartella) = self.percorso.parent() {
            let mut b = std::fs::DirBuilder::new();
            b.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            b.create(cartella).map_err(|e| e.to_string())?;
        }
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let tmp = crate::file::con_suffisso(&self.percorso, &format!(".{}.{ms}.tmp", std::process::id()));
        let scrivi = || -> std::io::Result<f64> {
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
            use std::io::Write;
            o.open(&tmp)?.write_all(format!("{}\n", dati.in_js_rientrato(2)).as_bytes())?;
            std::fs::rename(&tmp, &self.percorso)?;
            mtime_ms(&self.percorso)
        };
        match scrivi() {
            Ok(m) => {
                self.mtime.set(m);
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e.to_string())
            }
        }
    }

    pub fn elenco(&self) -> Result<Vec<SessioneCondivisa>, String> {
        self.carica()?;
        Ok(self.dati.borrow().sessioni.values().cloned().collect())
    }

    pub fn sessione(&self, account: &str) -> Result<Option<SessioneCondivisa>, String> {
        self.carica()?;
        Ok(self.dati.borrow().sessioni.get(&account.to_lowercase()).cloned())
    }

    pub fn metti(&self, s: SessioneCondivisa) -> Result<(), String> {
        self.carica()?;
        let chiave = s.borrow().account.to_lowercase();
        let prima = self.dati.borrow_mut().sessioni.insert(chiave.clone(), s);
        self.salva().inspect_err(|_| {
            let mut d = self.dati.borrow_mut();
            match prima {
                Some(p) => {
                    d.sessioni.insert(chiave, p);
                }
                None => {
                    d.sessioni.shift_remove(&chiave);
                }
            }
        })
    }

    pub fn rimuovi(&self, account: &str) -> Result<bool, String> {
        self.carica()?;
        let c = account.to_lowercase();
        let Some((i, _, prima)) = self.dati.borrow_mut().sessioni.shift_remove_full(&c) else { return Ok(false) };
        if let Err(e) = self.salva() {
            self.dati.borrow_mut().sessioni.shift_insert(i, c, prima);
            return Err(e);
        }
        Ok(true)
    }

    pub fn nome(&self, id: &str) -> Result<Option<String>, String> {
        self.carica()?;
        Ok(self.dati.borrow().nomi.as_ref().and_then(|n| n.get(id)).and_then(Json::as_str).map(str::to_string))
    }

    /// Aggiunge nomi alla cache; si salva solo se qualcosa è cambiato.
    pub fn aggiungi_nomi(&self, nuovi: &[(String, String)]) -> Result<(), String> {
        self.carica()?;
        let cambiato = {
            let mut d = self.dati.borrow_mut();
            let nomi = d.nomi.get_or_insert_with(Mappa::new);
            let mut cambiato = false;
            for (id, n) in nuovi {
                if nomi.get(id).and_then(Json::as_str) != Some(n.as_str()) {
                    nomi.insert(id.as_str(), n.as_str());
                    cambiato = true;
                }
            }
            cambiato
        };
        if cambiato { self.salva() } else { Ok(()) }
    }
}
