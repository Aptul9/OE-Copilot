//! L'agente di Oe Copilot sulla VM: legge i calendari del telefono e li serve al gateway. Stessa API dell'agente Python, con le stesse
//! risposte, gli stessi codici e gli stessi testi di errore (tutte con "Authorization: Bearer <API_KEY>"):
//!   GET /api/eventi?inizio=ISO&fine=ISO  eventi del periodo + stato di ogni casella
//!   GET /api/calendari                   le caselle, con stato e numero di appuntamenti
//!   GET /api/salute                      stato delle caselle, senza rileggere
//!   POST /api/ics                        {"testo": ICS[, "inizio": ISO, "fine": ISO]}: il nome del calendario, o i suoi eventi nel periodo
//! e gli accessi alle caselle (/api/auth/*), il broker dei token della posta (GET /api/token-posta) e il gateway di sola lettura su posta e
//! calendari che non fa uscire i token.

use crate::comunicazione::GatewayComunicazione;
use crate::diretto::{CalendarioDiretto, Controllo, GestoreToken};
use crate::fonte::{Adesso, CalendarioAndroid, Contesto, DB, Fonte, Livello, Registro, adesso_vero, leggi_utf8, testo_js};
use crate::shared_provisioning::{attiva_condivisione, identita_da_token, richiesta_condivisione};
use crate::http::{self, Gestore, Richiesta, Risposta};
use indexmap::IndexMap;
use jiff::Timestamp;
use jiff::civil::DateTime;
use oe_nucleo::errori::{Errore, Esito};
use oe_nucleo::ics::{EventoIcs, espandi_ics, valida_ics};
use oe_nucleo::iso::{iso_naive, ora_locale_da};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use oe_nucleo::pyfmt::{py_bool, py_float, py_repr, py_repr_str, py_strip, py_tipo};
use oe_nucleo::query::{analizza_target, parse_qs};
use oe_nucleo::tempo::fuso_iana;
use oe_rete::errori::ErroreAccesso;
use oe_rete::file::con_suffisso;
use oe_rete::js::{js_trim, primi_utf16};
use oe_rete::sessione_web::OUTLOOK_WEB_CLIENT_ID;
use oe_rete::shared_commands::{SharedCommand, read_commands, save_commands};
use oe_rete::shared_labels::{carica_etichette, leggi_etichette, salva_etichette};
use oe_rete::shared_teams::{RapportoTeams, leggi_rapporto_teams, rapporto_valido};
use oe_rete::signin::{SignIns, campo, uuid};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const MAX_CORPO: u64 = 8_000_000;
/// Le risorse dei token che la chiave di TeamsRelay può chiedere: quelle della chat, mai outlook.
const RISORSE_CHAT: [&str; 4] = ["csa", "ic3", "spaces", "presence"];
const CACHE_S: f64 = 15.0;

/// La frase dello stato: quelle dell'agente Python, poi quelle di Node (`http.STATUS_CODES`).
fn frase_stato(codice: u16) -> &'static str {
    match codice {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Request Entity Too Large",
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        402 => "Payment Required",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a Teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Entity",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        509 => "Bandwidth Limit Exceeded",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => "",
    }
}

/// Il registro come lo scrive logging.basicConfig di Python: ora locale, livello, nome, messaggio, su stderr.
pub fn registro_su_stderr(zona: &str, adesso: Adesso) -> Registro {
    let tz = fuso_iana(zona).unwrap_or(jiff::tz::TimeZone::UTC);
    Rc::new(move |livello: Livello, testo: &str| {
        let ms = adesso();
        let t = Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH).to_zoned(tz.clone()).datetime();
        eprintln!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02},{:03} {} agente: {testo}",
            t.year(),
            t.month(),
            t.day(),
            t.hour(),
            t.minute(),
            t.second(),
            ms.rem_euclid(1000),
            livello.nome()
        );
    })
}

pub fn contesto_di(accessi: PathBuf, zona: &str, adesso: Option<Adesso>, registro: Option<Registro>) -> Contesto {
    let adesso = adesso.unwrap_or_else(|| Rc::new(adesso_vero));
    let registro = registro.unwrap_or_else(|| registro_su_stderr(zona, adesso.clone()));
    Contesto { accessi, zona: zona.to_string(), adesso, registro, tokens: None }
}

/// _carica_fonti: fonti.json elenca le caselle da leggere (tipo "android", "outlook" o "google"). Un file che manca o non ha la forma giusta
/// ferma l'avvio.
pub fn carica_fonti(percorso: &Path, cartella_db: &Path, ctx: &Contesto) -> Esito<Vec<Fonte>> {
    let testo = leggi_utf8(percorso).map_err(|e| Errore::Altro(crate::fonte::messaggio_sistema(&e, percorso, None)))?;
    let voci = Json::analizza(&testo).map_err(|e| Errore::Altro(format!("SyntaxError: {}", e.messaggio)))?;
    let elenco: Vec<Json> = match &voci {
        Json::Arr(v) => v.clone(),
        Json::Obj(m) => m.keys().map(|k| Json::from(k.as_str())).collect(),
        altro => return Err(Errore::Tipo(format!("'{}' object is not iterable", py_tipo(altro)))),
    };
    let usa_diretto_per_android = ctx.tokens.as_ref().is_some_and(|t| t.percorso.exists() || !cartella_db.join(DB).exists());
    let mut fonti = Vec::new();
    for v in &elenco {
        let Json::Obj(voce) = v else { return Err(Errore::Altro(format!("'{}' object has no attribute 'get'", py_tipo(v)))) };
        let tipo = voce.get("tipo").and_then(Json::as_str);
        if matches!(tipo, Some("android" | "outlook" | "google")) {
            for k in ["id", "nome", "account"] {
                if !voce.contains_key(k) {
                    return Err(Errore::Altro(format!("KeyError: {}", py_repr_str(k))));
                }
            }
            let calendari = match voce.get("calendari") {
                None | Some(Json::Null) => None,
                Some(Json::Arr(c)) => Some(c.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| Errore::Tipo("c.replace is not a function".to_string()))).collect::<Esito<Vec<_>>>()?),
                Some(_) => return Err(Errore::Tipo("(calendari ?? []).map is not a function".to_string())),
            };
            let account = voce.get("account").and_then(Json::as_str).ok_or_else(|| Errore::Tipo("s.replace is not a function".to_string()))?;
            let id = testo_js(voce.get("id").unwrap_or(&Json::Null));
            let nome = testo_js(voce.get("nome").unwrap_or(&Json::Null));
            if tipo != Some("android") || usa_diretto_per_android {
                let gestore = match &ctx.tokens {
                    Some(t) => t.clone(),
                    None => {
                        let trasporto: Rc<dyn oe_rete::trasporto::Trasporto> = Rc::new(oe_rete::trasporto::TrasportoReqwest::nuovo().map_err(|e| Errore::Altro(e.to_string()))?);
                        Rc::new(GestoreToken::nuovo(PathBuf::from("/stato/tokens.json"), crate::diretto::OpzioniToken { token_posta: None, adesso: Some(ctx.adesso.clone()), trasporto, strict: false }))
                    }
                };
                fonti.push(Fonte::Diretto(CalendarioDiretto::nuovo(id, nome, account, calendari, ctx.clone(), gestore)));
            } else {
                fonti.push(Fonte::Android(CalendarioAndroid::nuovo(id, nome, account, cartella_db.to_path_buf(), calendari, ctx.clone())));
            }
        } else {
            (ctx.registro)(Livello::Error, &format!("fonte sconosciuta, ignorata: {}", py_repr(v)));
        }
    }
    if let (Some(tokens), true) = (&ctx.tokens, usa_diretto_per_android) {
        for (email, voce) in tokens.elenco_account() {
            if !fonti.iter().any(|f| f.account().to_lowercase() == email.to_lowercase()) {
                let provider = voce.borrow().get("provider").map_or_else(|| "undefined".to_string(), testo_js);
                fonti.push(Fonte::Diretto(CalendarioDiretto::nuovo(format!("{provider}::{email}"), nome_da_email(&email), &email, None, ctx.clone(), tokens.clone())));
            }
        }
    }
    Ok(fonti)
}

pub struct OpzioniAgente {
    pub chiave: String,
    /// La chiave di TeamsRelay (TEAMS_BROKER_KEY): apre solo l'elenco degli account condivisi e i token delle risorse della chat.
    pub chiave_teams: String,
    pub fonti: Vec<Fonte>,
    /// Il segno di vita scritto a ogni giro dal servizio che aggiorna il telefono (battito.json).
    pub battito: PathBuf,
    /// Il fuso locale: TZ, o Europe/Rome.
    pub zona: String,
    /// Token OAuth decifrati per l'MCP della posta (token-outlook.json).
    pub token_posta: PathBuf,
    /// Stato degli accessi scritto dal rilevatore (accessi.json).
    pub accessi: PathBuf,
    /// Il testo di un calendario ICS da espandere (POST /api/ics): oltre questi byte la richiesta è rifiutata.
    pub max_corpo: u64,
    /// Letture ravvicinate dello stesso periodo: una sola.
    pub cache_s: f64,
    pub adesso: Adesso,
    pub registro: Registro,
    /// Gestore dei token OAuth per la lettura diretta senza emulatore (tokens.json).
    pub tokens: Option<Rc<GestoreToken>>,
}

impl OpzioniAgente {
    pub fn nuove(chiave: &str, fonti: Vec<Fonte>, battito: PathBuf, zona: &str) -> Self {
        let adesso: Adesso = Rc::new(adesso_vero);
        OpzioniAgente {
            chiave: chiave.to_string(),
            chiave_teams: String::new(),
            fonti,
            battito,
            zona: zona.to_string(),
            token_posta: PathBuf::from("/stato/token-outlook.json"),
            accessi: PathBuf::from("/stato/accessi.json"),
            max_corpo: MAX_CORPO,
            cache_s: CACHE_S,
            registro: registro_su_stderr(zona, adesso.clone()),
            adesso,
            tokens: None,
        }
    }
}

struct VoceCache {
    t: f64,
    risposta: Rc<Json>,
}

pub struct Agente {
    o: OpzioniAgente,
    fonti: RefCell<Vec<Rc<Fonte>>>,
    cache: RefCell<IndexMap<String, VoceCache>>,
    /// L'ultimo stato degli account Teams mandato da TeamsRelay (shared_teams).
    rapporto_teams: RefCell<Option<RapportoTeams>>,
    /// Le caselle rimosse dalla pagina: ferme qui, così un nuovo accesso le riprende come fonti.json le descrive (id, calendari).
    ferme: RefCell<Vec<Rc<Fonte>>>,
    comunicazione: Option<GatewayComunicazione>,
}

fn accesso(e: ErroreAccesso) -> Errore {
    Errore::Altro(e.to_string())
}

pub fn nome_da_email(email: &str) -> String {
    let dominio = email.split('@').nth(1).unwrap_or_default().to_lowercase();
    if dominio == "gmail.com" || dominio == "googlemail.com" {
        return "Personale".to_string();
    }
    let radice = dominio.split('.').next().filter(|r| !r.is_empty()).or_else(|| email.split('@').next().filter(|r| !r.is_empty())).unwrap_or("Calendario");
    let mut c = radice.chars();
    match c.next() {
        Some(primo) => primo.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Cosa fa il gestore: una risposta, o chiudere la connessione senza rispondere (un'eccezione che Python non prende).
type Uscita = Result<http::Esito, Errore>;

fn ascii(s: &str) -> bool {
    s.is_ascii()
}

/// I parametri di una query come li legge URLSearchParams: "+" è uno spazio, le sequenze % si decodificano.
fn parametro_url(query: &str, nome: &str) -> Option<String> {
    let decodifica = |s: &str| -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < b.len() + 0 && b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit) => {
                    let h = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                    out.push(h(b[i + 1]) * 16 + h(b[i + 2]));
                    i += 2;
                }
                c => out.push(c),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    query.trim_start_matches('?').split('&').filter(|p| !p.is_empty()).find_map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decodifica(k) == nome).then(|| decodifica(v))
    })
}

/// json.loads accetta NaN, Infinity e -Infinity, JSON.parse no: fuori dalle stringhe diventano un oggetto (non è né nullo né una stringa,
/// come un float in Python).
fn con_estensioni_di_python(testo: &str) -> std::borrow::Cow<'_, str> {
    if !testo.contains("NaN") && !testo.contains("Infinity") {
        return std::borrow::Cow::Borrowed(testo);
    }
    let c: Vec<char> = testo.chars().collect();
    let mut out = String::with_capacity(testo.len());
    let mut dentro = false;
    let mut i = 0;
    while i < c.len() {
        let x = c[i];
        if dentro {
            out.push(x);
            if x == '\\' {
                if let Some(y) = c.get(i + 1) {
                    out.push(*y);
                }
                i += 1;
            } else if x == '"' {
                dentro = false;
            }
            i += 1;
            continue;
        }
        if x == '"' {
            dentro = true;
        }
        let resto: String = c[i..c.len().min(i + 10)].iter().collect();
        let parola = ["-Infinity", "Infinity", "-NaN", "NaN"].into_iter().find(|p| resto.starts_with(p));
        if let Some(p) = parola {
            let dopo = c.get(i + p.chars().count());
            if !dopo.is_some_and(|d| d.is_ascii_alphanumeric() || *d == '_') {
                out.push_str("{\"\\u0000nan\":1}");
                i += p.chars().count();
                continue;
            }
        }
        out.push(x);
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}

/// json.loads(bytes): la codifica si riconosce dai primi byte (UTF-8 con o senza BOM, UTF-16), un byte non valido è un errore.
pub fn leggi_json(b: &[u8]) -> Esito<Json> {
    let utf32 = || Errore::Valore("utf-32 non supportato".to_string());
    if b.len() >= 4 && (b[..4] == [0xff, 0xfe, 0, 0] || b[..4] == [0, 0, 0xfe, 0xff]) {
        return Err(utf32());
    }
    let mut codifica = "utf-8";
    if b.starts_with(&[0xff, 0xfe]) {
        codifica = "utf-16le";
    } else if b.starts_with(&[0xfe, 0xff]) {
        codifica = "utf-16be";
    } else if b.len() >= 4 {
        if b[0] == 0 {
            if b[1] == 0 {
                return Err(utf32());
            }
            codifica = "utf-16be";
        } else if b[1] == 0 {
            if b[2] == 0 && b[3] == 0 {
                return Err(utf32());
            }
            codifica = "utf-16le";
        }
    } else if b.len() == 2 {
        if b[0] == 0 {
            codifica = "utf-16be";
        } else if b[1] == 0 {
            codifica = "utf-16le";
        }
    }
    let non_valido = || Errore::Valore("The encoded data was not valid for encoding".to_string());
    let testo: String = match codifica {
        "utf-8" => {
            let t = std::str::from_utf8(b).map_err(|_| non_valido())?;
            t.strip_prefix('\u{feff}').unwrap_or(t).to_string()
        }
        _ => {
            if b.len() % 2 != 0 {
                return Err(non_valido());
            }
            let unita = b.chunks(2).map(|x| if codifica == "utf-16le" { u16::from_le_bytes([x[0], x[1]]) } else { u16::from_be_bytes([x[0], x[1]]) });
            let t: String = char::decode_utf16(unita).collect::<Result<String, _>>().map_err(|_| non_valido())?;
            t.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(t)
        }
    };
    Json::analizza(&con_estensioni_di_python(&testo)).map_err(|e| Errore::Valore(e.messaggio))
}

/// Le intestazioni di una risposta JSON, nell'ordine in cui le scrive Node.
fn intestazioni_json(prima: &[(&str, &str)], lunghezza: usize, extra: &[(String, String)], scarta: bool) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = prima.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect();
    v.push(("Content-Type".to_string(), "application/json; charset=utf-8".to_string()));
    v.push(("Content-Length".to_string(), lunghezza.to_string()));
    for (k, x) in extra {
        match v.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
            Some(voce) => voce.1 = x.clone(),
            None => v.push((k.clone(), x.clone())),
        }
    }
    v.push(("Connection".to_string(), if scarta { "keep-alive" } else { "close" }.to_string()));
    v
}

impl Agente {
    pub fn nuovo(mut o: OpzioniAgente) -> Rc<Self> {
        let mut fonti: Vec<Rc<Fonte>> = Vec::new();
        let mut ferme: Vec<Rc<Fonte>> = Vec::new();
        for f in std::mem::take(&mut o.fonti) {
            let rimossa = o.tokens.as_ref().is_some_and(|t| t.rimosso(f.account()));
            if rimossa { ferme.push(Rc::new(f)) } else { fonti.push(Rc::new(f)) }
        }
        let comunicazione = o.tokens.clone().map(GatewayComunicazione::nuovo);
        Rc::new(Agente { o, fonti: RefCell::new(fonti), cache: RefCell::new(IndexMap::new()), rapporto_teams: RefCell::new(None), ferme: RefCell::new(ferme), comunicazione })
    }

    fn adesso_s(&self) -> f64 {
        (self.o.adesso)() as f64 / 1000.0
    }

    fn fonti(&self) -> Vec<Rc<Fonte>> {
        self.fonti.borrow().clone()
    }

    fn ha_fonti_dirette(&self) -> bool {
        self.fonti.borrow().iter().any(|f| f.diretta())
    }

    /// Il segno di vita del servizio: da battito.json se presente, altrimenti attivo per le fonti dirette via API.
    pub fn servizio(&self) -> Esito<Json> {
        let leggi = || -> Esito<Json> {
            let testo = leggi_utf8(&self.o.battito).map_err(|e| Errore::Altro(e.to_string()))?;
            let b = Json::analizza(&testo).map_err(|e| Errore::Altro(e.messaggio))?;
            let Json::Obj(dati) = &b else { return Ok(Json::Null) };
            let Some(quando) = dati.get("quando") else { return Ok(Json::Null) };
            let differenza = self.adesso_s() - py_float(quando)?;
            if differenza.is_nan() {
                return Ok(Json::Null);
            }
            if !differenza.is_finite() {
                return Err(Errore::SenzaRisposta("OverflowError: cannot convert float infinity to integer".to_string()));
            }
            let telefono = match dati.get("telefono") {
                Some(t) => py_bool(t),
                None => true,
            };
            Ok(ogg! { "eta_s": differenza.trunc().max(0.0), "pausa": py_bool(dati.get("pausa").unwrap_or(&Json::Null)), "telefono": telefono })
        };
        match leggi() {
            Ok(v) => Ok(v),
            Err(e @ Errore::SenzaRisposta(_)) => Err(e),
            Err(_) if self.ha_fonti_dirette() => Ok(ogg! { "eta_s": 0, "pausa": false, "telefono": true }),
            Err(_) => Ok(Json::Null),
        }
    }

    fn fonti_stato(&self) -> Json {
        let mut m = Mappa::new();
        for f in self.fonti.borrow().iter() {
            let s = f.stato();
            m.insert(f.nome(), ogg! { "stato": s.stato, "msg": s.msg });
        }
        Json::Obj(m)
    }

    pub async fn raccogli(&self, inizio: DateTime, fine: DateTime) -> Esito<Rc<Json>> {
        let chiave = format!("{}\u{0}{}", iso_naive(&inizio), iso_naive(&fine));
        if let Some(v) = self.cache.borrow().get(&chiave) {
            if self.adesso_s() - v.t < self.o.cache_s {
                return Ok(v.risposta.clone());
            }
        }
        let mut eventi: Vec<Json> = Vec::new();
        let mut errori: Vec<String> = Vec::new();
        for fonte in self.fonti() {
            if self.calendario_spento(&fonte) {
                continue;
            }
            let (trovati, err) = match fonte.eventi(inizio, fine).await {
                Ok(x) => x,
                Err(e @ Errore::SenzaRisposta(_)) => return Err(e),
                Err(e) => {
                    // Una fonte rotta non deve portarsi via le altre.
                    (self.o.registro)(Livello::Error, &format!("fonte {} fallita: {}", fonte.nome(), e.messaggio()));
                    (Vec::new(), vec![format!("{}: {}", fonte.nome(), e.messaggio())])
                }
            };
            eventi.extend(trovati);
            errori.extend(err);
        }
        let ora = Timestamp::from_millisecond((self.o.adesso)()).unwrap_or(Timestamp::UNIX_EPOCH).to_zoned(fuso_iana(&self.o.zona)?).datetime();
        let generato = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", ora.year(), ora.month(), ora.day(), ora.hour(), ora.minute(), ora.second());
        let risposta = Rc::new(ogg! {
            "eventi": eventi,
            "errori": errori,
            "fonti": self.fonti_stato(),
            "caselle": self.fonti().iter().map(|f| f.descrizione()).collect::<Vec<_>>(),
            "servizio": self.servizio()?,
            "generato": generato,
        });
        let mut cache = self.cache.borrow_mut();
        cache.insert(chiave, VoceCache { t: self.adesso_s(), risposta: risposta.clone() });
        // La cache non deve crescere all'infinito: si tengono le 50 più recenti.
        if cache.len() > 50 {
            let mut tempi: Vec<(String, f64)> = cache.iter().map(|(k, v)| (k.clone(), v.t)).collect();
            tempi.sort_by(|a, b| a.1.total_cmp(&b.1));
            let togli = tempi.len() - 50;
            for (k, _) in tempi.into_iter().take(togli) {
                cache.shift_remove(&k);
            }
        }
        Ok(risposta)
    }

    /// hmac.compare_digest sulle intestazioni: gli stessi byte, in tempo costante. Con caratteri non ASCII Python solleva TypeError e il
    /// gestore muore.
    fn confronta(&self, r: &Richiesta<'_>, chiave: &str) -> Esito<bool> {
        if chiave.is_empty() {
            return Ok(false);
        }
        let atteso = format!("Bearer {chiave}");
        let dato = r.intestazione("authorization").unwrap_or_default();
        if !ascii(dato) || !ascii(&atteso) {
            return Err(Errore::SenzaRisposta("TypeError: comparing strings with non-ASCII characters is not supported".to_string()));
        }
        let (a, b) = (dato.as_bytes(), atteso.as_bytes());
        Ok(a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0)
    }

    fn autorizzato(&self, r: &Richiesta<'_>) -> Esito<bool> {
        self.confronta(r, &self.o.chiave)
    }

    /// La chiave di TeamsRelay, solo dove serve a lui: l'elenco degli account condivisi e i token delle risorse della chat. Una chiave uguale
    /// a quella piena vale come piena.
    fn autorizzato_teams(&self, r: &Richiesta<'_>, metodo: &str, percorso: &str, query: &str) -> Esito<bool> {
        if self.o.chiave_teams == self.o.chiave || !self.confronta(r, &self.o.chiave_teams)? {
            return Ok(false);
        }
        if metodo == "POST" {
            return Ok(percorso == "/api/auth/shared" || percorso == "/api/auth/shared/teams");
        }
        if percorso == "/api/auth/shared" {
            return Ok(true);
        }
        Ok(percorso == "/api/token-resource" && RISORSE_CHAT.contains(&parametro_url(query, "resource").unwrap_or_default().as_str()))
    }

    fn registra_richiesta(&self, r: &Richiesta<'_>, codice: u16) {
        (self.o.registro)(Livello::Info, &format!("{} \"{} {} HTTP/{}\" {codice} -", r.remoto, r.metodo, r.url, r.versione));
    }

    fn invia(&self, r: &Richiesta<'_>, codice: u16, dati: &Json, scarta: bool) -> Uscita {
        self.invia_json(r, codice, dati.in_py().into_bytes(), scarta, &[], &[])
    }

    /// Un corpo JSON già pronto. Python risponde in HTTP/1.0: una richiesta, una connessione. Se il corpo non è stato letto si tiene aperta
    /// finché il cliente finisce di mandarlo (chiudere subito azzererebbe la connessione e la risposta potrebbe non arrivare).
    fn invia_json(&self, r: &Richiesta<'_>, codice: u16, corpo: Vec<u8>, scarta: bool, prima: &[(&str, &str)], extra: &[(String, String)]) -> Uscita {
        self.registra_richiesta(r, codice);
        Ok(http::Esito::Risposta(Risposta {
            codice,
            motivo: frase_stato(codice).to_string(),
            intestazioni: intestazioni_json(prima, corpo.len(), extra, scarta),
            corpo,
            scarta_corpo: scarta,
        }))
    }

    /// send_error di http.server: una pagina HTML con il codice (qui solo 501, per i metodi che l'agente non ha).
    fn pagina_errore(&self, r: &Richiesta<'_>, codice: u16, messaggio: &str, spiegazione: &str) -> Uscita {
        let corpo = format!(
            "<!DOCTYPE HTML>\n<html lang=\"en\">\n    <head>\n        <meta charset=\"utf-8\">\n        <title>Error response</title>\n    </head>\n    <body>\n        <h1>Error response</h1>\n        <p>Error code: {codice}</p>\n        <p>Message: {}.</p>\n        <p>Error code explanation: {codice} - {spiegazione}.</p>\n    </body>\n</html>\n",
            messaggio.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        );
        let motivo: String = messaggio.chars().flat_map(|c| if (' '..='~').contains(&c) { vec![c] } else { vec!['?'; c.len_utf16()] }).collect();
        self.registra_richiesta(r, codice);
        Ok(http::Esito::Risposta(Risposta {
            codice,
            motivo,
            intestazioni: vec![
                ("Connection".to_string(), "close".to_string()),
                ("Content-Type".to_string(), "text/html;charset=utf-8".to_string()),
                ("Content-Length".to_string(), corpo.len().to_string()),
            ],
            corpo: if r.metodo == "HEAD" { Vec::new() } else { corpo.into_bytes() },
            scarta_corpo: false,
        }))
    }

    async fn post(&self, r: &mut Richiesta<'_>) -> Uscita {
        let (percorso, query) = analizza_target(&r.url)?;
        if !self.autorizzato(r)? && !self.autorizzato_teams(r, "POST", &percorso, &query)? {
            return self.invia(r, 401, &ogg! { "errore": "non autorizzato" }, false);
        }
        const NOTI: [&str; 13] = [
            "/api/ics",
            "/api/auth/inizio",
            "/api/auth/completa",
            "/api/auth/controlla",
            "/api/auth/rimuovi",
            "/api/auth/shared",
            "/api/auth/shared/teams",
            "/api/auth/shared/labels",
            "/api/communication/read",
            "/api/communication/gmail/read",
            "/api/communication/gmail/write",
            "/api/communication/gchat/read",
            "/api/communication/gchat/write",
        ];
        if !NOTI.contains(&percorso.as_str()) {
            return self.invia(r, 404, &ogg! { "errore": "sconosciuto" }, false);
        }
        let dichiarata = r.intestazione("content-length").filter(|v| !v.is_empty()).unwrap_or("0");
        let t = dichiarata.trim_matches(|c: char| c.is_whitespace());
        let cifre = t.strip_prefix(['+', '-']).unwrap_or(t);
        let lunghezza: f64 = if !cifre.is_empty() && cifre.bytes().all(|b| b.is_ascii_digit()) { t.parse().unwrap_or(f64::INFINITY) } else { -1.0 };
        if !(lunghezza > 0.0 && lunghezza <= self.o.max_corpo as f64) {
            let codice = if lunghezza > self.o.max_corpo as f64 { 413 } else { 400 };
            return self.invia(r, codice, &ogg! { "errore": "lunghezza della richiesta non valida" }, true);
        }
        let Some(corpo) = r.leggi_corpo().await else { return Ok(http::Esito::Chiudi) };
        if percorso.starts_with("/api/communication/gmail/") || percorso.starts_with("/api/communication/gchat/") {
            return self.google_request(r, &corpo);
        }
        if ["/api/auth/inizio", "/api/auth/completa", "/api/auth/rimuovi"].contains(&percorso.as_str()) {
            let Ok(dati) = leggi_json(&corpo) else { return self.invia(r, 400, &ogg! { "errore": "invalid JSON" }, false) };
            if let Json::Obj(m) = &dati {
                let provider = m.get("provider").map_or_else(|| "undefined".to_string(), testo_js);
                if ["gmail", "gchat", "google-web"].contains(&provider.as_str()) {
                    return self.google_request(r, &corpo);
                }
            }
        }
        if percorso == "/api/communication/read" {
            return self.leggi_comunicazione(r, &corpo).await;
        }
        if percorso == "/api/auth/shared" {
            return self.auth_shared(r, &corpo).await;
        }
        if percorso == "/api/auth/shared/labels" {
            let Some(tokens) = self.o.tokens.clone() else { return self.invia(r, 400, &ogg! { "ok": false, "errore": "Token store unavailable" }, false) };
            let etichette = match leggi_json(&corpo) {
                Err(_) => return self.invia(r, 400, &ogg! { "ok": false, "errore": "invalid JSON" }, false),
                Ok(d) => match leggi_etichette(&d) {
                    Ok(e) => e,
                    Err(e) => return self.invia(r, 400, &ogg! { "ok": false, "errore": e.0 }, false),
                },
            };
            salva_etichette(&con_suffisso(&tokens.percorso, ".labels.json"), &etichette)?;
            return self.invia(r, 200, &ogg! { "ok": true }, false);
        }
        if percorso == "/api/auth/shared/teams" {
            let Ok(dati) = leggi_json(&corpo) else { return self.invia(r, 400, &ogg! { "errore": "invalid JSON" }, false) };
            let campi_ammessi = match &dati {
                Json::Obj(m) => m.keys().all(|k| ["accounts", "others", "results"].contains(&k.as_str())),
                Json::Arr(a) => a.is_empty(),
                _ => false,
            };
            if !campi_ammessi {
                return self.invia(r, 400, &ogg! { "errore": "Invalid report fields" }, false);
            }
            let mut rapporto = Mappa::new();
            rapporto.insert("accounts", dati.get("accounts").cloned().unwrap_or_default());
            rapporto.insert("others", dati.get("others").cloned().unwrap_or_default());
            match leggi_rapporto_teams(&Json::Obj(rapporto), self.adesso_s().floor() as i64) {
                Ok(x) => *self.rapporto_teams.borrow_mut() = Some(x),
                Err(e) => return self.invia(r, 400, &ogg! { "errore": e.0 }, false),
            }
            if let (Some(tokens), Some(Json::Arr(risultati))) = (&self.o.tokens, dati.get("results")) {
                let file = con_suffisso(&tokens.percorso, ".commands.json");
                let mut restano = Vec::new();
                for c in read_commands(&file)? {
                    match risultati.iter().find(|x| x.get("id").and_then(Json::as_str) == Some(c.id.as_str())) {
                        None => restano.push(c),
                        Some(x) => {
                            if let Some(e) = x.get("error").filter(|e| oe_rete::js::vero(e)) {
                                restano.push(SharedCommand { error: Some(primi_utf16(&testo_js(e), 300)), ..c });
                            }
                        }
                    }
                }
                save_commands(&file, &restano)?;
            }
            return self.invia(r, 200, &ogg! { "ok": true }, false);
        }
        if ["/api/auth/inizio", "/api/auth/rimuovi", "/api/auth/controlla", "/api/auth/completa"].contains(&percorso.as_str()) {
            let Some(tokens) = self.o.tokens.clone() else { return self.invia(r, 400, &ogg! { "ok": false, "errore": "gestore token non configurato" }, false) };
            let esito = match percorso.as_str() {
                "/api/auth/inizio" => self.auth_inizio(&tokens, &corpo).await,
                "/api/auth/rimuovi" => self.auth_rimuovi(&tokens, &corpo),
                "/api/auth/controlla" => self.auth_controlla(&tokens, &corpo).await,
                _ => self.auth_completa(&tokens, &corpo).await,
            };
            return match esito {
                Ok(v) => self.invia(r, 200, &v, false),
                Err(e @ Errore::SenzaRisposta(_)) => Err(e),
                Err(e) => self.invia(r, 200, &ogg! { "ok": false, "errore": e.messaggio() }, false),
            };
        }
        self.ics(r, &corpo)
    }

    fn sposta_in_ferme(&self, email: &str) {
        let mut fonti = self.fonti.borrow_mut();
        let (via, restano): (Vec<Rc<Fonte>>, Vec<Rc<Fonte>>) = std::mem::take(&mut *fonti).into_iter().partition(|f| f.account().to_lowercase() == email);
        *fonti = restano;
        self.ferme.borrow_mut().extend(via);
    }

    fn assicura_fonte_diretta(&self, account: &str, provider: &str) -> Esito<()> {
        let email = account.to_lowercase();
        if let Some(t) = &self.o.tokens {
            t.riammetti(&email).map_err(accesso)?;
        }
        {
            let mut ferme = self.ferme.borrow_mut();
            let (tornano, restano): (Vec<Rc<Fonte>>, Vec<Rc<Fonte>>) = std::mem::take(&mut *ferme).into_iter().partition(|f| f.account().to_lowercase() == email);
            *ferme = restano;
            self.fonti.borrow_mut().extend(tornano);
        }
        let mut trovata = false;
        for f in self.fonti.borrow().iter().filter(|f| f.account().to_lowercase() == email) {
            f.svuota_cache();
            trovata = true;
        }
        if let (false, Some(tokens)) = (trovata, &self.o.tokens) {
            let mut ctx = contesto_di(self.o.accessi.clone(), &self.o.zona, Some(self.o.adesso.clone()), Some(self.o.registro.clone()));
            ctx.tokens = Some(tokens.clone());
            let f = CalendarioDiretto::nuovo(format!("{provider}::{email}"), nome_da_email(&email), &email, None, ctx, tokens.clone());
            self.fonti.borrow_mut().push(Rc::new(Fonte::Diretto(f)));
        }
        Ok(())
    }

    fn adesso_s_intero(&self) -> i64 {
        (self.o.adesso)().div_euclid(1000)
    }

    fn signins(&self, tokens: &GestoreToken) -> SignIns {
        let adesso = self.o.adesso.clone();
        SignIns::nuovo(&con_suffisso(&tokens.percorso, ".signins.json"), move || adesso().div_euclid(1000))
    }

    fn cancel_sign_in(&self, account: &str) -> Esito<()> {
        let Some(tokens) = &self.o.tokens else { return Ok(()) };
        let jobs = self.signins(tokens);
        for job in jobs.list().map_err(accesso)?.into_iter().filter(|j| campo(j, "account") == account && matches!(campo(j, "phase"), "queued" | "browser")) {
            jobs.update(
                campo(&job, "id"),
                &[("phase", Some(Json::from("blocked"))), ("error", Some(Json::from("Connection cancelled because service choices changed. Start a new connection if needed.")))],
            )
            .map_err(accesso)?;
        }
        Ok(())
    }

    async fn auth_shared(&self, r: &Richiesta<'_>, corpo: &[u8]) -> Uscita {
        let Ok(dati) = leggi_json(corpo) else { return self.invia(r, 400, &ogg! { "errore": "invalid JSON" }, false) };
        let operazione = dati.get("operation");
        let stretta = operazione.and_then(Json::as_str).unwrap_or_default().to_string();
        let come_testo = operazione.map_or_else(|| "undefined".to_string(), testo_js);
        let attivo = |o: &Json| !matches!(o, Json::Null | Json::Bool(false)) && !(o.as_f64() == Some(0.0)) && o.as_str() != Some("");
        if stretta == "signin-cancel" {
            let Some(tokens) = self.o.tokens.clone() else { return self.invia(r, 400, &ogg! { "ok": false, "errore": "Token store unavailable" }, false) };
            let account = dati.get("account").and_then(Json::as_str).map(|a| js_trim(a).to_lowercase()).unwrap_or_default();
            return match self.signins(&tokens).cancel(&dati["id"], &Json::from(account.as_str())) {
                Ok(signin) => {
                    (self.o.registro)(Livello::Info, &format!("accesso guidato annullato: {account}"));
                    self.invia(r, 200, &ogg! { "ok": true, "signin": Json::Obj(signin) }, false)
                }
                Err(e) => self.invia(r, 400, &ogg! { "ok": false, "errore": e.to_string() }, false),
            };
        }
        if attivo(&dati) && ["connect", "signin-progress", "signin-import"].contains(&come_testo.as_str()) {
            let Some(tokens) = self.o.tokens.clone() else { return self.invia(r, 400, &ogg! { "ok": false, "errore": "Token store unavailable" }, false) };
            return match self.accesso_guidato(&tokens, &dati, &stretta).await {
                Ok(v) => self.invia(r, 200, &v, false),
                Err(e) => {
                    let testo = e.to_string();
                    let mut m = Mappa::new();
                    m.insert("ok", false);
                    if matches!(e, ErroreAccesso::ProxyBrowser(_)) {
                        m.insert("needsProxy", true);
                    }
                    if matches!(e, ErroreAccesso::SessioneScaduta(_)) && !testo.contains("AADSTS53003") {
                        m.insert("needsBrowser", true);
                    }
                    m.insert("errore", testo);
                    self.invia(r, 400, &Json::Obj(m), false)
                }
            };
        }
        if attivo(&dati) && ["remove", "chat", "calls"].contains(&stretta.as_str()) {
            return self.comando_account(r, &dati, &stretta);
        }
        if let (Some(tokens), Some(Json::Str(account)), Some(Json::Arr(caps))) = (&self.o.tokens, dati.get("account"), dati.get("capabilities")) {
            let account = js_trim(account).to_lowercase();
            let prima: Vec<Json> = tokens
                .voce_account(&account)
                .and_then(|v| v.borrow().get("shared").filter(|s| oe_rete::js::vero(s)).and_then(|s| s.get("capabilities")).and_then(Json::as_arr).cloned())
                .unwrap_or_else(|| vec![Json::from("mail"), Json::from("calendar")]);
            let solo_togliere = caps.iter().all(|c| prima.contains(c));
            if solo_togliere {
                self.cancel_sign_in(&account)?;
            } else {
                let aperti = SignIns::nuovo(&con_suffisso(&tokens.percorso, ".signins.json"), || adesso_vero().div_euclid(1000))
                    .list()
                    .map_err(accesso)?
                    .iter()
                    .any(|j| campo(j, "account") == account && matches!(campo(j, "phase"), "queued" | "browser"));
                if aperti {
                    return self.invia(r, 409, &ogg! { "ok": false, "errore": "Finish the connection before enabling more services" }, false);
                }
            }
        }
        let esito = richiesta_condivisione(self.o.tokens.as_deref(), Some(&dati), None).await;
        if esito.get("ok") == Some(&Json::Bool(true)) {
            self.cache.borrow_mut().clear();
            let account = esito.get("account").filter(|a| !a.is_null()).map(testo_js).unwrap_or_default();
            (self.o.registro)(Livello::Info, &format!("condivisione casella aggiornata: {account}"));
        }
        self.invia(r, 200, &esito, false)
    }

    async fn accesso_guidato(&self, tokens: &Rc<GestoreToken>, data: &Json, operazione: &str) -> Result<Json, ErroreAccesso> {
        let jobs = self.signins(tokens);
        let account = data.get("account").and_then(Json::as_str).map(|a| js_trim(a).to_lowercase()).unwrap_or_default();
        if operazione == "connect" {
            let rapporto = self.rapporto_teams.borrow().clone();
            let Some(report) = rapporto_valido(rapporto.as_ref(), self.adesso_s_intero()).cloned() else {
                return Err(ErroreAccesso::altro("Microsoft sign-in service unavailable. TeamsRelay must be connected."));
            };
            let previous = tokens.voce_account(&account).map(|v| v.borrow().clone());
            let caps = match data.get("capabilities") {
                Some(Json::Arr(v)) if !v.is_empty() && v.iter().all(|c| matches!(c.as_str(), Some("mail" | "calendar" | "chat"))) => v.clone(),
                _ => return Err(ErroreAccesso::altro("Choose mail, calendar or Teams")),
            };
            let provider = previous.as_ref().and_then(|p| p.get("provider").and_then(Json::as_str).map(str::to_string));
            if previous.is_some() && provider.as_deref() != Some("microsoft") && caps.iter().any(|c| c.as_str() != Some("chat")) {
                return Err(ErroreAccesso::altro("This address already has a different mail provider. Its credentials will not be replaced."));
            }
            let existing: Vec<Json> = match (&previous, provider.as_deref()) {
                (Some(p), Some("microsoft")) => p.get("shared").filter(|s| oe_rete::js::vero(s)).and_then(|s| s.get("capabilities")).and_then(Json::as_arr).cloned().unwrap_or_else(|| vec![Json::from("mail"), Json::from("calendar")]),
                _ => Vec::new(),
            };
            let browser_on = report.others.iter().any(|(e, t)| *e == account && t.state != "stopped");
            let mut tutte: Vec<Json> = Vec::new();
            for c in caps.iter().chain(existing.iter()).cloned().chain(browser_on.then(|| Json::from("chat"))) {
                if !tutte.contains(&c) {
                    tutte.push(c);
                }
            }
            let signin = jobs.start(&account, &Json::Arr(tutte))?;
            let id = campo(&signin, "id").to_string();
            let mobile_rinnovabile = previous.as_ref().is_some_and(|p| {
                provider.as_deref() == Some("microsoft")
                    && p.get("refresh_token").is_some_and(oe_rete::js::vero)
                    && p.get("client_id").and_then(Json::as_str) != Some(OUTLOOK_WEB_CLIENT_ID)
                    && !p.get("errore").is_some_and(oe_rete::js::vero)
            });
            if campo(&signin, "phase") == "queued" && mobile_rinnovabile && !report.others.iter().any(|(e, _)| *e == account) {
                match attiva_condivisione(tokens, &account, signin.get("capabilities")).await {
                    Ok(_) => {
                        jobs.update(&id, &[("phase", Some(Json::from("complete"))), ("route", Some(Json::from("outlook-mobile")))])?;
                        self.cache.borrow_mut().clear();
                    }
                    Err(e) => {
                        // Only expired/refused credentials enter the browser flow; an outage must not request a fresh login.
                        let scaduta = tokens.voce_account(&account).is_some_and(|v| v.borrow().get("errore").is_some_and(oe_rete::js::vero));
                        if !scaduta && !matches!(e.causa, Some(ErroreAccesso::SharedRifiutato(_))) {
                            jobs.update(&id, &[("phase", Some(Json::from("blocked"))), ("error", Some(Json::from(e.messaggio)))])?;
                        }
                    }
                }
            }
            let corrente = jobs.list()?.into_iter().find(|j| campo(j, "id") == id).map_or(Json::Null, Json::Obj);
            return Ok(ogg! { "ok": true, "signin": corrente });
        }
        let job = jobs.find(&data["id"], &Json::from(account.as_str()))?;
        let slot = data.get("slot");
        let fase = data.get("phase").and_then(Json::as_str).unwrap_or_default();
        let fase_job = campo(&job, "phase").to_string();
        let first_binding = operazione == "signin-progress" && fase == "browser" && fase_job == "queued";
        let slot_intero = slot.and_then(Json::as_f64).filter(|n| n.fract() == 0.0);
        let slot_job = job.get("slot").filter(|s| oe_rete::js::vero(s));
        let esente = operazione == "signin-progress" && fase == "blocked" && slot.is_none();
        if !esente && (slot_intero.is_none_or(|n| !(1.0..=9999.0).contains(&n)) || (slot_job.is_some_and(|s| Some(s) != slot) && !first_binding)) {
            return Err(ErroreAccesso::altro("Sign-in slot mismatch"));
        }
        if operazione == "signin-progress" {
            if !["browser", "blocked", "complete"].contains(&fase) {
                return Err(ErroreAccesso::altro("Invalid sign-in phase"));
            }
            let url = match data.get("url").filter(|u| oe_rete::js::vero(u)) {
                Some(u) => Some(url::Url::parse(&testo_js(u)).map_err(|_| ErroreAccesso::altro("Invalid URL"))?),
                None => None,
            };
            if let Some(u) = &url {
                let atteso = format!("/api/desktop/{}", slot.map(testo_js).unwrap_or_else(|| "undefined".to_string()));
                if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some() || u.path() != atteso || u.query().is_some_and(|q| !q.is_empty()) || u.fragment().is_some_and(|f| !f.is_empty()) {
                    return Err(ErroreAccesso::altro("Invalid sign-in URL"));
                }
            }
            let caps_job: Vec<String> = job.get("capabilities").and_then(Json::as_arr).map(|v| v.iter().map(testo_js).collect()).unwrap_or_default();
            if fase == "complete" && caps_job.iter().any(|c| c != "chat") && !tokens.ha_account(&account) {
                return Err(ErroreAccesso::altro("Mailbox not imported"));
            }
            let mut patch: Vec<(&str, Option<Json>)> = vec![("phase", Some(Json::from(fase)))];
            if let Some(s) = slot.filter(|s| oe_rete::js::vero(s)) {
                patch.push(("slot", Some(Json::from(oe_rete::js::numero_di(s)))));
            }
            if let Some(u) = &url {
                patch.push(("url", Some(Json::from(u.to_string()))));
            }
            patch.push(("error", data.get("error").and_then(Json::as_str).map(|e| Json::from(primi_utf16(e, 300)))));
            let signin = jobs.update(campo(&job, "id"), &patch)?;
            return Ok(ogg! { "ok": true, "signin": Json::Obj(signin) });
        }
        let sessione = data.get("sessione").cloned().unwrap_or_default();
        let hint = sessione.get("loginHint").and_then(Json::as_str).map(|h| js_trim(h).to_lowercase());
        if !slot_job.is_some() || fase_job != "browser" || hint.as_deref() != Some(account.as_str()) {
            return Err(ErroreAccesso::altro("Microsoft session identity mismatch"));
        }
        let caps: Vec<&str> = job.get("capabilities").and_then(Json::as_arr).map(|v| v.iter().filter_map(Json::as_str).filter(|c| *c != "chat").collect()).unwrap_or_default();
        let required: Vec<&str> = caps.iter().copied().filter(|c| *c == "mail" || *c == "calendar").collect();
        let id_job = campo(&job, "id").to_string();
        let valida = |token: &str| -> Result<(), ErroreAccesso> {
            jobs.find(&Json::from(id_job.as_str()), &Json::from(account.as_str()))?;
            if tokens.voce_account(&account).is_some_and(|v| v.borrow().get("provider").and_then(Json::as_str) == Some("google")) {
                return Err(ErroreAccesso::altro("The account provider changed during sign-in"));
            }
            identita_da_token(token, &account).map_err(|e| ErroreAccesso::altro(e.messaggio))?;
            Ok(())
        };
        let tenant = data.get("tenant").and_then(Json::as_str).unwrap_or("organizations");
        let (_, route) = tokens.importa_guidata(&sessione, tenant, Some(&valida), &required).await?;
        let token = tokens.voce_account(&account).and_then(|v| v.borrow().get("access_token").filter(|t| oe_rete::js::vero(t)).map(testo_js));
        let Some(token) = token else { return Err(ErroreAccesso::altro("Microsoft did not return a mailbox token")) };
        let (tid, oid) = identita_da_token(&token, &account).map_err(|e| ErroreAccesso::altro(e.messaggio))?;
        let mut sh = Mappa::new();
        sh.insert("tenantId", tid);
        sh.insert("objectId", oid);
        sh.insert("capabilities", caps.iter().map(|c| c.to_string()).collect::<Vec<_>>());
        sh.insert("origin", if route == "outlook-web" { "outlook-spa" } else { "mobile" });
        tokens.imposta_shared(&account, Some(Json::Obj(sh)))?;
        self.assicura_fonte_diretta(&account, "microsoft").map_err(|e| ErroreAccesso::altro(e.messaggio()))?;
        self.cache.borrow_mut().clear();
        jobs.update(&id_job, &[("route", Some(Json::from(route.as_str())))])?;
        Ok(ogg! { "ok": true, "route": route })
    }

    fn comando_account(&self, r: &Richiesta<'_>, data: &Json, operazione: &str) -> Uscita {
        let (Some(tokens), Some(Json::Str(account))) = (self.o.tokens.clone(), data.get("account")) else {
            return self.invia(r, 400, &ogg! { "ok": false, "errore": "account required" }, false);
        };
        let account = js_trim(account).to_lowercase();
        let rapporto = self.rapporto_teams.borrow().clone();
        let Some(teams) = rapporto_valido(rapporto.as_ref(), self.adesso_s_intero()) else {
            return self.invia(r, 409, &ogg! { "ok": false, "errore": "Teams report unavailable" }, false);
        };
        let mailbox = tokens.voce_account(&account);
        if mailbox.as_ref().is_some_and(|m| m.borrow().get("provider").and_then(Json::as_str) != Some("microsoft")) {
            return self.invia(r, 400, &ogg! { "ok": false, "errore": "Microsoft account required" }, false);
        }
        let browser = teams.others.iter().any(|(e, _)| *e == account);
        if mailbox.is_none() && !browser && !teams.accounts.contains_key(&account) {
            return self.invia(r, 404, &ogg! { "ok": false, "errore": "Account not found" }, false);
        }
        let on = data.get("on").and_then(Json::as_bool);
        if operazione == "chat" && (on.is_none() || !browser) {
            return self.invia(r, 400, &ogg! { "ok": false, "errore": "Browser Teams account required" }, false);
        }
        let calls = teams.accounts.get(&account).or_else(|| teams.others.iter().find(|(e, _)| *e == account).map(|(_, s)| s)).and_then(|s| s.calls);
        if operazione == "calls" && (on.is_none() || calls.is_none()) {
            return self.invia(r, 400, &ogg! { "ok": false, "errore": "Calls are switched only for Teams accounts without a browser" }, false);
        }
        let file = con_suffisso(&tokens.percorso, ".commands.json");
        let mut commands = read_commands(&file)?;
        if commands.iter().any(|c| c.account == account) {
            return self.invia(r, 409, &ogg! { "ok": false, "errore": "Account operation already pending" }, false);
        }
        commands.push(SharedCommand { id: uuid(), account: account.clone(), operation: operazione.to_string(), on: if operazione != "remove" { on } else { None }, error: None });
        save_commands(&file, &commands)?;
        if operazione == "remove" || (operazione == "chat" && on == Some(false)) {
            self.cancel_sign_in(&account)?;
        }
        if operazione == "remove" && mailbox.is_some() {
            self.sposta_in_ferme(&account);
            tokens.rimuovi(&account).map_err(accesso)?;
            self.cache.borrow_mut().clear();
        }
        self.invia(r, 200, &ogg! { "ok": true, "account": account, "pending": true }, false)
    }

    fn oggetto_o_vuoto(corpo: &[u8]) -> Esito<Json> {
        let dati = leggi_json(corpo)?;
        Ok(if matches!(dati, Json::Obj(_)) { dati } else { Json::obj() })
    }

    async fn auth_inizio(&self, tokens: &GestoreToken, corpo: &[u8]) -> Esito<Json> {
        let r = Self::oggetto_o_vuoto(corpo)?;
        let email = r.get("email").and_then(Json::as_str).map(|e| js_trim(e).to_string());
        if r.get("provider").and_then(Json::as_str) == Some("google") {
            let (url, state) = tokens.inizia_google_pkce(email.as_deref());
            return Ok(ogg! { "ok": true, "provider": "google", "modo": "pkce", "url": url, "state": state });
        }
        if r.get("modo").and_then(Json::as_str) == Some("device") {
            // Ripiega sul link PKCE diretto se il codice dispositivo non è raggiungibile
            if let Ok(d) = tokens.inizia_device_microsoft(email.as_deref()).await {
                return Ok(ogg! { "ok": true, "provider": "microsoft", "modo": "device", "url": d.url, "user_code": d.user_code, "state": d.state, "intervallo_s": d.intervallo_s, "url_pkce": d.url_pkce });
            }
        }
        let (url, state) = tokens.inizia_pkce(email.as_deref());
        Ok(ogg! { "ok": true, "provider": "microsoft", "modo": "pkce", "url": url, "state": state })
    }

    fn auth_rimuovi(&self, tokens: &GestoreToken, corpo: &[u8]) -> Esito<Json> {
        let r = Self::oggetto_o_vuoto(corpo)?;
        let email = r.get("account").and_then(Json::as_str).map(|a| js_trim(a).to_lowercase()).unwrap_or_default();
        let sue = self.fonti.borrow().iter().any(|f| f.account().to_lowercase() == email);
        if email.is_empty() || (!sue && !tokens.ha_account(&email)) {
            return Ok(ogg! { "ok": false, "errore": "Casella non trovata" });
        }
        self.sposta_in_ferme(&email);
        tokens.rimuovi(&email).map_err(accesso)?;
        self.cache.borrow_mut().clear();
        (self.o.registro)(Livello::Info, &format!("casella rimossa: {email}"));
        Ok(ogg! { "ok": true, "account": email })
    }

    async fn auth_controlla(&self, tokens: &GestoreToken, corpo: &[u8]) -> Esito<Json> {
        let r = Self::oggetto_o_vuoto(corpo)?;
        let state = r.get("state").and_then(Json::as_str).map(|s| js_trim(s).to_string()).unwrap_or_default();
        match tokens.controlla_device_microsoft(&state).await.map_err(accesso)? {
            Controllo::Completato { account, .. } => {
                self.cache.borrow_mut().clear();
                self.assicura_fonte_diretta(&account, "microsoft")?;
                Ok(ogg! { "ok": true, "stato": "completato", "account": account, "provider": "microsoft" })
            }
            Controllo::InAttesa => Ok(ogg! { "ok": true, "stato": "in_attesa" }),
        }
    }

    async fn auth_completa(&self, tokens: &GestoreToken, corpo: &[u8]) -> Esito<Json> {
        let dati = leggi_json(corpo)?;
        let Json::Obj(r) = &dati else { return Err(Errore::Valore("not a dict".to_string())) };
        let provider = if r.get("provider").and_then(Json::as_str) == Some("google") { "google" } else { "microsoft" };
        let state = r.get("state").and_then(Json::as_str);
        let testo = |k: &str| r.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
        let account = if provider == "google" {
            let codice = testo("codice");
            if !js_trim(&codice).is_empty() {
                tokens.completa_google_pkce(&codice, state, Some(&testo("email"))).await.map_err(accesso)?
            } else {
                tokens.salva_google(&testo("email"), &testo("token")).await.map_err(accesso)?
            }
        } else if let Some(sessione) = r.get("sessione") {
            // Sessione del browser (formato teamsrelay session.json) per le caselle dove Outlook Mobile è respinto
            tokens.importa_sessione(sessione, &testo("tenant"), None, &["calendar"]).await.map_err(accesso)?.0
        } else {
            tokens.completa_pkce(&testo("codice"), state).await.map_err(accesso)?.0
        };
        self.cache.borrow_mut().clear();
        self.assicura_fonte_diretta(&account, provider)?;
        Ok(ogg! { "ok": true, "account": account, "provider": provider })
    }

    /// POST /api/communication/read: una lettura di posta o calendario con le credenziali dell'agente; il token non esce.
    async fn leggi_comunicazione(&self, r: &Richiesta<'_>, corpo: &[u8]) -> Uscita {
        let Some(comunicazione) = &self.comunicazione else { return self.invia(r, 404, &ogg! { "errore": "casella non trovata" }, false) };
        let Ok(dati) = leggi_json(corpo) else { return self.invia(r, 400, &ogg! { "errore": "il corpo non è JSON" }, false) };
        match comunicazione.leggi(&dati).await {
            Ok(testo) => self.invia_json(r, 200, testo.into_bytes(), false, &[], &[]),
            Err(e) => {
                let corpo = ogg! { "errore": e.messaggio.as_str() }.in_py().into_bytes();
                self.invia_json(r, e.codice, corpo, false, &[], &e.intestazioni)
            }
        }
    }

    fn ics(&self, r: &Richiesta<'_>, corpo: &[u8]) -> Uscita {
        let leggi = || -> Esito<(String, Option<(DateTime, DateTime)>)> {
            let richiesta = leggi_json(corpo)?;
            let Json::Obj(m) = &richiesta else { return Err(Errore::Tipo("not a dict".to_string())) };
            let Some(t) = m.get("testo") else { return Err(Errore::Altro("KeyError: 'testo'".to_string())) };
            let da = m.get("inizio").cloned().unwrap_or(Json::Null);
            let a = m.get("fine").cloned().unwrap_or(Json::Null);
            let Json::Str(testo) = t else { return Err(Errore::Valore("periodo".to_string())) };
            if (da.is_null() || a.is_null()) && !(da.is_null() && a.is_null()) {
                return Err(Errore::Valore("periodo".to_string()));
            }
            let locale = |x: &Json| -> Esito<DateTime> {
                let Json::Str(s) = x else { return Err(Errore::Altro(format!("'{}' object has no attribute 'replace'", py_tipo(x)))) };
                ora_locale_da(s, &self.o.zona)
            };
            let intervallo = if da.is_null() { None } else { Some((locale(&da)?, locale(&a)?)) };
            Ok((testo.clone(), intervallo))
        };
        let (testo, intervallo) = match leggi() {
            Ok(x) => x,
            Err(e @ Errore::SenzaRisposta(_)) => return Err(e),
            Err(_) => return self.invia(r, 400, &ogg! { "errore": "serve {\"testo\": ..., \"inizio\": ..., \"fine\": ...}" }, false),
        };
        let esito = match intervallo {
            None => valida_ics(&testo).map(|nome| ogg! { "ok": true, "nome": nome }),
            Some((da, a)) => espandi_ics(&testo, &iso_naive(&da), &iso_naive(&a), Some(&self.o.zona)).map(|e| ogg! { "ok": true, "eventi": e.iter().map(EventoIcs::in_json).collect::<Vec<_>>() }),
        };
        match esito {
            Ok(v) => self.invia(r, 200, &v, false),
            // un testo che non è un calendario non deve fermare l'agente: lo si dice a chi l'ha mandato
            Err(e @ Errore::SenzaRisposta(_)) => Err(e),
            Err(e) => self.invia(r, 200, &ogg! { "ok": false, "errore": e.messaggio(), "valore": matches!(e, Errore::Valore(_) | Errore::Tipo(_)) }, false),
        }
    }

    fn google_request(&self, r: &Richiesta<'_>, _corpo: &[u8]) -> Uscita {
        self.invia(r, 404, &ogg! { "errore": "Google service not configured" }, false)
    }

    /// Una casella condivisa senza la capacità calendario: il calendario non la legge e non la mostra, senza errori.
    fn calendario_spento(&self, f: &Fonte) -> bool {
        let Some(tokens) = &self.o.tokens else { return false };
        tokens.voce_account(f.account()).and_then(|v| v.borrow().get("shared").filter(|s| oe_rete::js::vero(s)).cloned()).is_some_and(|s| {
            !s.get("capabilities").and_then(Json::as_arr).is_some_and(|c| c.iter().any(|x| x.as_str() == Some("calendar")))
        })
    }

    async fn get(&self, r: &mut Richiesta<'_>) -> Uscita {
        let (percorso, query) = analizza_target(&r.url)?;
        let piena = self.autorizzato(r)?;
        if !piena && !self.autorizzato_teams(r, "GET", &percorso, &query)? {
            return self.invia(r, 401, &ogg! { "errore": "non autorizzato" }, false);
        }
        let q = parse_qs(&query);
        match percorso.as_str() {
            "/api/salute" => self.invia(r, 200, &ogg! { "fonti": self.fonti_stato(), "servizio": self.servizio()? }, false),
            "/api/communication/accounts" => {
                let caselle = self.comunicazione.as_ref().map(|c| c.caselle()).unwrap_or_default();
                self.invia(r, 200, &ogg! { "accounts": caselle }, false)
            }
            "/api/communication/google/accounts" => self.invia(r, 200, &ogg! { "accounts": Json::Arr(Vec::new()) }, false),
            "/api/auth/shared" => {
                let rapporto = self.rapporto_teams.borrow().clone();
                let teams = rapporto_valido(rapporto.as_ref(), self.adesso_s_intero());
                let mut m = match richiesta_condivisione(self.o.tokens.as_deref(), None, teams).await {
                    Json::Obj(m) => m,
                    _ => Mappa::new(),
                };
                match &self.o.tokens {
                    Some(t) => {
                        let signins = self.signins(t).list().map_err(accesso)?;
                        m.insert("signins", Json::Arr(signins.into_iter().map(Json::Obj).collect()));
                        m.insert("commands", Json::Arr(read_commands(&con_suffisso(&t.percorso, ".commands.json"))?.iter().map(SharedCommand::a_json).collect()));
                        m.insert("labels", carica_etichette(&con_suffisso(&t.percorso, ".labels.json")));
                    }
                    None => {
                        m.insert("signins", Json::Arr(Vec::new()));
                        m.insert("commands", Json::Arr(Vec::new()));
                        m.insert("labels", Json::Null);
                    }
                }
                self.invia(r, 200, &Json::Obj(m), false)
            }
            "/api/auth/stato" => {
                let caselle: Vec<Json> = self
                    .fonti()
                    .iter()
                    .map(|f| {
                        let s = f.stato();
                        let voce = self.o.tokens.as_ref().and_then(|t| t.voce_account(f.account())).map(|v| v.borrow().clone());
                        let errore = voce.as_ref().and_then(|v| v.get("errore").filter(|e| oe_rete::js::vero(e)).cloned());
                        let provider = voce.as_ref().and_then(|v| v.get("provider").filter(|p| !p.is_null()).cloned()).unwrap_or_else(|| Json::from(if f.tipo().as_deref() == Some("caldav") { "google" } else { "microsoft" }));
                        let msg = errore.clone().unwrap_or_else(|| Json::from(if s.msg.is_null() { String::new() } else { testo_js(&s.msg) }));
                        ogg! {
                            "id": f.id(),
                            "nome": f.nome(),
                            "account": f.account(),
                            "provider": provider,
                            "configurato": voce.is_some(),
                            "scaduto": voce.is_none() || errore.is_some(),
                            "stato": if errore.is_some() { "errore".to_string() } else { s.stato },
                            "msg": msg,
                        }
                    })
                    .collect();
                self.invia(r, 200, &ogg! { "caselle": caselle }, false)
            }
            // Trusted service endpoint. Capabilities do not downscope raw tokens; gateways filter reads.
            "/api/token-resource" => {
                let target_query = r.url.split_once('?').map(|(_, q)| q.split('#').next().unwrap_or_default()).unwrap_or_default().to_string();
                let risorsa = parametro_url(&target_query, "resource").unwrap_or_default();
                let (codice, corpo) = if !piena && !RISORSE_CHAT.contains(&risorsa.as_str()) {
                    (401, ogg! { "errore": "non autorizzato" })
                } else {
                    let account = parametro_url(&target_query, "account").unwrap_or_default();
                    let esito = match &self.o.tokens {
                        Some(t) => t.ottieni_risorsa(&account, &risorsa, false).await.ok(),
                        None => None,
                    };
                    match esito {
                        Some(token) => (200, ogg! { "token": token }),
                        None => (403, ogg! { "errore": "shared token unavailable" }),
                    }
                };
                self.invia_json(r, codice, corpo.in_py().into_bytes(), false, &[("Cache-Control", "no-store")], &[])
            }
            "/api/token-posta" => {
                if let Some(t) = self.o.tokens.as_ref().filter(|t| !t.elenco_account().is_empty()) {
                    let diretti = t.token_posta_attivi().await;
                    if !diretti.is_empty() {
                        return self.invia(r, 200, &ogg! { "generato": self.adesso_s_intero(), "account": diretti, "servizio": self.servizio()? }, false);
                    }
                }
                let mut generato = Json::from(0);
                let mut account: Vec<Json> = Vec::new();
                if let Some(Json::Obj(raw)) = leggi_utf8(&self.o.token_posta).ok().and_then(|t| Json::analizza(&t).ok()) {
                    if let Some(Json::Num(n)) = raw.get("generato") {
                        generato = Json::Num(*n);
                    }
                    if let Some(Json::Arr(a)) = raw.get("account") {
                        account = a.clone();
                    }
                }
                let caselle_stato = match leggi_utf8(&self.o.accessi).ok().and_then(|t| Json::analizza(&t).ok()) {
                    Some(Json::Obj(acc)) => acc.get("caselle").filter(|c| matches!(c, Json::Obj(_) | Json::Arr(_))).cloned().unwrap_or_default(),
                    _ => Json::Null,
                };
                let mut arricchiti: Vec<Json> = Vec::new();
                for a in &account {
                    let upn_grezzo = testo_js(a.get("upn").filter(|u| !u.is_null()).unwrap_or(&Json::from("")));
                    let condivisa = self.o.tokens.as_ref().and_then(|t| t.voce_account(&upn_grezzo)).is_some_and(|v| v.borrow().get("shared").is_some_and(oe_rete::js::vero));
                    if condivisa {
                        continue;
                    }
                    let upn = upn_grezzo.to_lowercase();
                    let st = caselle_stato.get(&upn).cloned();
                    let mut m = match a {
                        Json::Obj(m) => m.clone(),
                        _ => Mappa::new(),
                    };
                    let campo = |k: &str, d: &str| st.as_ref().and_then(|s| s.get(k)).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| Json::from(d));
                    m.insert("phone_status", campo("stato", "ok"));
                    m.insert("phone_msg", campo("msg", ""));
                    arricchiti.push(Json::Obj(m));
                }
                self.invia(r, 200, &ogg! { "generato": generato, "account": arricchiti, "servizio": self.servizio()? }, false)
            }
            "/api/calendari" => {
                let mut caselle: Vec<Json> = Vec::new();
                for f in self.fonti() {
                    if self.calendario_spento(&f) {
                        continue;
                    }
                    match f.riepilogo().await {
                        Ok(c) => caselle.push(c),
                        Err(e @ Errore::SenzaRisposta(_)) => return Err(e),
                        Err(e) => {
                            (self.o.registro)(Livello::Error, &format!("fonte {}: riepilogo fallito: {}", f.nome(), e.messaggio()));
                            caselle.push(ogg! { "id": f.id(), "account": f.account(), "nome": f.nome(), "tipo": f.tipo(), "eventi": Json::Null, "stato": "errore", "msg": e.messaggio() });
                        }
                    }
                }
                self.invia(r, 200, &ogg! { "caselle": caselle, "servizio": self.servizio()? }, false)
            }
            "/api/eventi" => {
                let leggi = || -> Esito<(DateTime, DateTime)> {
                    let primo = q.get("inizio").and_then(|v| v.first()).ok_or_else(|| Errore::Altro("'inizio'".to_string()))?;
                    let inizio = ora_locale_da(primo, &self.o.zona)?;
                    let secondo = q.get("fine").and_then(|v| v.first()).ok_or_else(|| Errore::Altro("'fine'".to_string()))?;
                    Ok((inizio, ora_locale_da(secondo, &self.o.zona)?))
                };
                let (inizio, fine) = match leggi() {
                    Ok(x) => x,
                    Err(e @ (Errore::Altro(_) | Errore::Tipo(_) | Errore::Valore(_))) => return self.invia(r, 400, &ogg! { "errore": format!("periodo non valido ({})", e.messaggio()) }, false),
                    Err(e) => return Err(e),
                };
                let risposta = self.raccogli(inizio, fine).await?;
                self.invia(r, 200, &risposta, false)
            }
            _ => self.invia(r, 404, &ogg! { "errore": "sconosciuto" }, false),
        }
    }
}

impl Gestore for Rc<Agente> {
    async fn gestisci(&self, r: &mut Richiesta<'_>) -> http::Esito {
        let esito = match r.metodo.as_str() {
            "GET" => self.get(r).await,
            "POST" => self.post(r).await,
            altro => {
                let messaggio = format!("Unsupported method ({})", py_repr_str(altro));
                self.pagina_errore(r, 501, &messaggio, "Server does not support this operation")
            }
        };
        match esito {
            Ok(x) => x,
            Err(e) => {
                // un'eccezione che Python non prende chiude la connessione senza risposta, e qui lo stesso
                let testo = match &e {
                    Errore::SenzaRisposta(m) => format!("gestore fermato: {m}"),
                    altro => format!("gestore fermato da un errore inatteso: {}", altro.messaggio()),
                };
                (self.o.registro)(Livello::Error, &testo);
                http::Esito::Chiudi
            }
        }
    }
}

/// Le caselle con i nomi, per il messaggio di avvio: `pyRepr(fonti.map(f => f.nome))`.
pub fn nomi_fonti(fonti: &[Fonte]) -> String {
    py_repr(&Json::Arr(fonti.iter().map(|f| Json::from(f.nome())).collect()))
}

/// `pyStrip` di un testo d'ambiente, per i numeri di porta.
pub fn strip(s: &str) -> &str {
    py_strip(s)
}
