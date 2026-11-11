//! Lettura diretta dei calendari via API (Outlook REST v2.0 e Google Calendar v3), senza emulatore Android. Usa i token di aggiornamento in
//! tokens.json (OAuth2 con client_id di Outlook Mobile per Microsoft, master token aas_et/ o refresh_token per Google) e rinnova da sé gli
//! access token quando scadono, salvando su disco il nuovo refresh_token.

use crate::fonte::{Contesto, Livello, Stato, ora_minuti};
use indexmap::IndexMap;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use oe_nucleo::errori::{Errore, Esito};
use oe_nucleo::html::pulisci_link;
use oe_nucleo::ics::MEETING_URL;
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::pyfmt::py_strip;
use oe_nucleo::serie::contiene_evento;
use oe_nucleo::tempo::{fuso_iana, ora_t, testo_naive};
use oe_rete::errori::{ErroreAccesso, EsitoAccesso};
use oe_rete::js::{js_trim, numero_di, primi_utf16, stringa_di, vero};
use oe_rete::outlook_rest::{ErroreDyn, ErroreRest, FonteToken, FutureToken, MS_LOGIN, OUTLOOK_REST, OpzioniRest, claims_jwt, json_rest, leggi_testo, motivo_token_web_non_valido, upn_di};
use oe_rete::sessione_web::{
    ClienteWeb, OUTLOOK_WEB_CLIENT_ID, OUTLOOK_WEB_REDIRECT_URI, OUTLOOK_WEB_SCOPES, SessioneMicrosoft, autorizza_da_sessione, casuale_base64url, codifica_componente, json_o_vuoto, leggi_sessione, modulo,
    sha256_base64url,
};
use oe_rete::shared_account::{SharedAccount, allowed, audience, client_for, exp_di, has_shared_permissions, renew_shared};
use oe_rete::trasporto::{Intestazioni, Richiesta, Trasporto};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

// Client ID e redirect URI estratti da com.microsoft.office.outlook 5.2641.2 (TokenRestApi.AAD_CLIENT_ID e AuthUtil.getOneAuthRedirectUri)
pub const OUTLOOK_CLIENT_ID: &str = "27922004-5251-4030-b22d-91ecd9a37ea4";
pub const OUTLOOK_REDIRECT_URI: &str = "msauth://com.microsoft.office.outlook/fcg80qvoM1YMKJZibjBwQcDfOno%3D";
pub const OUTLOOK_SCOPES: &str = "https://outlook.office.com/.default offline_access openid profile";

// Parametri Google Calendar SyncAdapter (com.google.android.syncadapters.calendar)
pub const GOOGLE_CALENDAR_APP: &str = "com.google.android.syncadapters.calendar";
pub const GOOGLE_CLIENT_SIG: &str = "38918a453d07199354f8b19af05ec6562ced5788";
pub const GOOGLE_CALENDAR_SERVICE: &str = "oauth2:https://www.googleapis.com/auth/calendar";

// Parametri OAuth2 PKCE Desktop per accesso Google da browser senza emulatore
pub const GOOGLE_OAUTH_CLIENT_ID: &str = "406964657835-aq8lmia8j95dhl1a2bvharmfk3t1hgqj.apps.googleusercontent.com";
pub const GOOGLE_OAUTH_CLIENT_SECRET: &str = "kSmqreRr0qwBWJgbf5Y-PjSU";
pub const GOOGLE_OAUTH_REDIRECT_URI: &str = "http://localhost";
pub const GOOGLE_OAUTH_SCOPES: &str = "https://www.googleapis.com/auth/calendar openid email";
pub const GOOGLE_TOKEN_MAX_BYTES: usize = 64 * 1024;

const MARGINE_SCADENZA_S: i64 = 120;
const TTL_CALENDARI_S: i64 = 300;
const MAX_PKCE_IN_ATTESA: usize = 20;
const TTL_PKCE_S: i64 = 900;

/// Una casella di tokens.json: l'oggetto com'è nel file (con i campi che questa versione non conosce), condiviso come in JavaScript fra
/// chi lo sta rinnovando e il resto.
pub type Voce = Rc<RefCell<Mappa>>;

fn s(m: &Mappa, k: &str) -> Option<String> {
    m.get(k).and_then(Json::as_str).map(str::to_string)
}

fn ver(m: &Mappa, k: &str) -> bool {
    m.get(k).is_some_and(vero)
}

/// `Number(m[k] ?? 0)`.
fn num(m: &Mappa, k: &str) -> f64 {
    m.get(k).filter(|v| !v.is_null()).map_or(0.0, numero_di)
}

fn altro(m: impl Into<String>) -> ErroreAccesso {
    ErroreAccesso::Altro(m.into())
}

fn scaduto(m: impl Into<String>) -> ErroreAccesso {
    ErroreAccesso::Scaduto(m.into())
}

fn shared_di(m: &Mappa) -> Option<SharedAccount> {
    m.get("shared").filter(|v| vero(v)).and_then(SharedAccount::da_json)
}

/// Le capacità di posta e calendario (senza la chat) che un token deve avere.
fn richieste(shared: &Option<SharedAccount>) -> Vec<&'static str> {
    match shared {
        Some(sh) => sh.capabilities.iter().filter_map(|c| match c.as_str() {
            "mail" => Some("mail"),
            "calendar" => Some("calendar"),
            _ => None,
        })
        .collect(),
        None => vec!["calendar"],
    }
}

/// Una riga di token-outlook.json e di GET /api/token-posta (il broker dei token di prima, che resta).
fn info_posta(upn: &str, voce: &Mappa, token: &str, exp: f64, errore: bool, msg: &str) -> Json {
    let mut m = Mappa::new();
    m.insert("upn", upn);
    m.insert("token", token);
    m.insert("exp", exp.trunc());
    m.insert("endpoint_type", "outlook_rest");
    m.insert("base_url", OUTLOOK_REST);
    m.insert("scopes", voce.get("scopes").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| Json::from(OUTLOOK_SCOPES)));
    m.insert("phone_status", if errore { "errore" } else { "ok" });
    m.insert("phone_msg", msg);
    Json::Obj(m)
}

fn scrivi_json_atomico(percorso: &Path, dati: &Json) -> std::io::Result<()> {
    if let Some(dir) = percorso.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = percorso.as_os_str().to_owned();
    tmp.push(format!(".{}.{}.tmp", std::process::id(), crate::fonte::adesso_vero()));
    let tmp = PathBuf::from(tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    use std::io::Write;
    o.open(&tmp)?.write_all(format!("{}\n", dati.in_js_rientrato(2)).as_bytes())?;
    std::fs::rename(&tmp, percorso)
}

fn mtime_ms(p: &Path) -> std::io::Result<f64> {
    let m = std::fs::metadata(p)?.modified()?;
    Ok(m.duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0))
}

/// I parametri di una query come URLSearchParams.
fn parametri_query(q: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(q.strip_prefix('?').unwrap_or(q).as_bytes()).into_owned().collect()
}

fn prendi(p: &[(String, String)], k: &str) -> Option<String> {
    p.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
}

fn estrai_codice_da_input(codice_o_url: &str) -> EsitoAccesso<(String, Option<String>)> {
    let pulito = js_trim(codice_o_url);
    if pulito.is_empty() {
        return Err(altro("Manca l'indirizzo o il codice di autorizzazione"));
    }
    if pulito.contains("://") || pulito.starts_with('?') || pulito.contains("code=") || pulito.contains("error=") {
        let q = pulito.find('?');
        let h = pulito.find('#');
        let query = match (q, h) {
            (Some(q), Some(h)) if h > q => &pulito[q + 1..h],
            (Some(q), _) => &pulito[q + 1..],
            (None, Some(h)) => &pulito[h + 1..],
            (None, None) => pulito,
        };
        let p = parametri_query(query);
        let errore = prendi(&p, "error_description").filter(|e| !e.is_empty()).or_else(|| prendi(&p, "error").filter(|e| !e.is_empty()));
        if let Some(e) = errore {
            return Err(altro(format!("Accesso rifiutato: {e}")));
        }
        let Some(code) = prendi(&p, "code").filter(|c| !c.is_empty()) else { return Err(altro("L'indirizzo incollato non contiene il parametro code=...")) };
        return Ok((js_trim(&code).to_string(), prendi(&p, "state")));
    }
    Ok((pulito.to_string(), None))
}

/// Un rinnovo in corso: chi arriva dopo aspetta il suo esito invece di farne un altro.
struct Promessa {
    esito: RefCell<Option<EsitoAccesso<String>>>,
    fatto: tokio::sync::Notify,
}

impl Promessa {
    fn nuova() -> Rc<Self> {
        Rc::new(Promessa { esito: RefCell::new(None), fatto: tokio::sync::Notify::new() })
    }

    async fn attendi(&self) -> EsitoAccesso<String> {
        loop {
            let notifica = self.fatto.notified();
            tokio::pin!(notifica);
            notifica.as_mut().enable();
            if let Some(e) = self.esito.borrow().clone() {
                return e;
            }
            notifica.await;
        }
    }

    fn compi(&self, e: EsitoAccesso<String>) {
        *self.esito.borrow_mut() = Some(e);
        self.fatto.notify_waiters();
    }
}

struct Pkce {
    verifier: String,
    provider: &'static str,
    email: Option<String>,
    quando_s: i64,
}

#[derive(Default)]
struct Archivio {
    accounts: IndexMap<String, Voce>,
    rimossi: Option<Vec<String>>,
}

impl Archivio {
    fn in_json(&self) -> Json {
        let mut m = Mappa::new();
        let mut a = Mappa::new();
        for (k, v) in &self.accounts {
            a.insert(k.as_str(), Json::Obj(v.borrow().clone()));
        }
        m.insert("accounts", Json::Obj(a));
        if let Some(r) = &self.rimossi {
            m.insert("rimossi", r.clone());
        }
        Json::Obj(m)
    }
}

pub struct OpzioniToken {
    pub token_posta: Option<PathBuf>,
    pub adesso: Option<crate::fonte::Adesso>,
    pub trasporto: Rc<dyn Trasporto>,
    pub strict: bool,
}

pub struct GestoreToken {
    pub percorso: PathBuf,
    pub token_posta: Option<PathBuf>,
    adesso: crate::fonte::Adesso,
    strict: bool,
    /// Lo stesso trasporto per i rinnovi e per il gateway di posta e calendari (comunicazione).
    pub trasporto: Rc<dyn Trasporto>,
    mtime: Cell<f64>,
    dati: RefCell<Archivio>,
    in_corso: RefCell<HashMap<String, Rc<Promessa>>>,
    pkce: RefCell<IndexMap<String, Pkce>>,
    device: RefCell<IndexMap<String, (String, i64)>>,
}

pub struct EsitoDevice {
    pub url: String,
    pub user_code: String,
    pub state: String,
    pub intervallo_s: f64,
    pub url_pkce: String,
}

pub enum Controllo {
    InAttesa,
    Completato { account: String, tenant_id: String },
}

impl GestoreToken {
    pub fn nuovo(percorso: PathBuf, o: OpzioniToken) -> Self {
        GestoreToken {
            percorso,
            token_posta: o.token_posta,
            adesso: o.adesso.unwrap_or_else(|| Rc::new(crate::fonte::adesso_vero)),
            strict: o.strict,
            trasporto: o.trasporto,
            mtime: Cell::new(-1.0),
            dati: RefCell::new(Archivio::default()),
            in_corso: RefCell::new(HashMap::new()),
            pkce: RefCell::new(IndexMap::new()),
            device: RefCell::new(IndexMap::new()),
        }
    }

    pub fn adesso_s(&self) -> i64 {
        (self.adesso)().div_euclid(1000)
    }

    fn carica_se_cambiato(&self) -> EsitoAccesso<()> {
        let esito = (|| -> Result<(), String> {
            if !self.percorso.exists() {
                if self.mtime.get() != 0.0 {
                    *self.dati.borrow_mut() = Archivio::default();
                    self.mtime.set(0.0);
                }
                return Ok(());
            }
            let m = mtime_ms(&self.percorso).map_err(|e| e.to_string())?;
            if m == self.mtime.get() {
                return Ok(());
            }
            let testo = oe_rete::file::leggi_utf8(&self.percorso).map_err(|e| e.to_string())?;
            let raw = Json::analizza(&testo).map_err(|e| e.messaggio)?;
            if let Json::Obj(obj) = &raw {
                let mut accounts = IndexMap::new();
                if let Some(Json::Obj(accs)) = obj.get("accounts") {
                    for (k, v) in accs.iter() {
                        let voce = match v {
                            Json::Obj(o) => o.clone(),
                            Json::Arr(a) => a.iter().enumerate().map(|(i, x)| (i.to_string(), x.clone())).collect(),
                            _ => continue,
                        };
                        accounts.insert(k.to_lowercase(), Rc::new(RefCell::new(voce)));
                    }
                }
                let rimossi: Vec<String> = obj.get("rimossi").and_then(Json::as_arr).map(|v| v.iter().filter_map(Json::as_str).map(str::to_lowercase).collect()).unwrap_or_default();
                *self.dati.borrow_mut() = Archivio { accounts, rimossi: if rimossi.is_empty() { None } else { Some(rimossi) } };
            }
            self.mtime.set(m);
            Ok(())
        })();
        match esito {
            // Mantiene i dati in memoria se il file è in corso di scrittura
            Err(e) if self.strict => Err(altro(e)),
            _ => Ok(()),
        }
    }

    fn salva(&self) -> EsitoAccesso<()> {
        let dati = self.dati.borrow().in_json();
        match scrivi_json_atomico(&self.percorso, &dati).and_then(|()| mtime_ms(&self.percorso)) {
            Ok(m) => self.mtime.set(m),
            // In ambiente a sola lettura i token rinnovati restano in memoria
            Err(e) if self.strict => return Err(altro(e.to_string())),
            Err(_) => {}
        }
        self.scrivi_token_posta();
        Ok(())
    }

    fn scrivi_token_posta(&self) {
        let Some(p) = &self.token_posta else { return };
        let adesso = self.adesso_s();
        let mut account = Vec::new();
        for (upn, v) in &self.dati.borrow().accounts {
            let v = v.borrow();
            let senza_posta = shared_di(&v).is_some_and(|sh| !sh.ha("mail"));
            if s(&v, "provider").as_deref() != Some("microsoft") || senza_posta || !ver(&v, "access_token") || num(&v, "expires_at") <= adesso as f64 {
                continue;
            }
            let errore = v.get("errore").filter(|e| !e.is_null()).map(stringa_di).unwrap_or_default();
            account.push(info_posta(upn, &v, &stringa_di(&v["access_token"]), num(&v, "expires_at"), ver(&v, "errore"), &errore));
        }
        let mut m = Mappa::new();
        m.insert("generato", adesso);
        m.insert("account", account);
        let _ = scrivi_json_atomico(p, &Json::Obj(m));
    }

    fn voce(&self, chiave: &str) -> Option<Voce> {
        self.dati.borrow().accounts.get(chiave).cloned()
    }

    /// Rilegge tokens.json se è cambiato; con `strict` un file illeggibile è un errore, come per la lettura che lo segue.
    pub fn carica(&self) -> EsitoAccesso<()> {
        self.carica_se_cambiato()
    }

    pub fn ha_account(&self, email: &str) -> bool {
        let _ = self.carica_se_cambiato();
        self.dati.borrow().accounts.contains_key(&email.to_lowercase())
    }

    pub fn voce_account(&self, email: &str) -> Option<Voce> {
        let _ = self.carica_se_cambiato();
        self.voce(&email.to_lowercase())
    }

    pub fn elenco_account(&self) -> Vec<(String, Voce)> {
        let _ = self.carica_se_cambiato();
        self.dati.borrow().accounts.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn rimosso(&self, email: &str) -> bool {
        let _ = self.carica_se_cambiato();
        self.dati.borrow().rimossi.as_ref().is_some_and(|r| r.contains(&email.to_lowercase()))
    }

    /// Rimuovi account: via l'accesso salvato, e la casella resta fuori anche da fonti.json.
    pub fn rimuovi(&self, email: &str) -> EsitoAccesso<()> {
        let chiave = email.to_lowercase();
        self.carica_se_cambiato()?;
        {
            let mut d = self.dati.borrow_mut();
            d.accounts.shift_remove(&chiave);
            let mut r = d.rimossi.take().unwrap_or_default();
            if !r.contains(&chiave) {
                r.push(chiave);
            }
            d.rimossi = Some(r);
        }
        self.salva()
    }

    /// Attiva o spegne la condivisione di una casella Microsoft; la cache per risorsa non sopravvive a nessun cambio.
    pub fn imposta_shared(&self, email: &str, shared: Option<Json>) -> EsitoAccesso<()> {
        self.carica_se_cambiato()?;
        let voce = self.voce(&email.to_lowercase()).filter(|v| s(&v.borrow(), "provider").as_deref() == Some("microsoft"));
        let Some(voce) = voce else { return Err(altro("Casella non trovata")) };
        {
            let mut v = voce.borrow_mut();
            match shared {
                Some(sh) => {
                    v.insert("shared", sh);
                }
                None => {
                    v.remove("shared");
                }
            }
            v.remove("resourceTokens");
        }
        self.salva()
    }

    /// Un nuovo accesso riporta dentro una casella rimossa.
    pub fn riammetti(&self, email: &str) -> EsitoAccesso<()> {
        let chiave = email.to_lowercase();
        self.carica_se_cambiato()?;
        {
            let mut d = self.dati.borrow_mut();
            let Some(r) = d.rimossi.as_ref().filter(|r| r.contains(&chiave)) else { return Ok(()) };
            let resto: Vec<String> = r.iter().filter(|x| **x != chiave).cloned().collect();
            d.rimossi = if resto.is_empty() { None } else { Some(resto) };
        }
        self.salva()
    }

    pub async fn ottieni_access_token(&self, email: &str, forza: bool) -> EsitoAccesso<String> {
        let chiave = email.to_lowercase();
        self.carica_se_cambiato()?;
        let Some(voce) = self.voce(&chiave) else {
            return Err(scaduto(format!("La casella {email} non ha un accesso configurato (Accedi di nuovo nella finestra Account)")));
        };
        if ver(&voce.borrow(), "shared") {
            return self.ottieni_risorsa(email, "outlook", forza).await;
        }
        let adesso = self.adesso_s();
        {
            let v = voce.borrow();
            if !forza && ver(&v, "access_token") && num(&v, "expires_at") > (adesso + MARGINE_SCADENZA_S) as f64 {
                return Ok(stringa_di(&v["access_token"]));
            }
        }
        let pendente = self.in_corso.borrow().get(&chiave).cloned();
        if let Some(p) = pendente {
            return p.attendi().await;
        }
        let promessa = Promessa::nuova();
        self.in_corso.borrow_mut().insert(chiave.clone(), promessa.clone());
        let google = s(&voce.borrow(), "provider").as_deref() == Some("google");
        let esito = if google { self.rinnova_google(&chiave, &voce).await } else { self.rinnova_microsoft(&chiave, &voce).await };
        self.in_corso.borrow_mut().remove(&chiave);
        promessa.compi(esito.clone());
        esito
    }

    pub async fn ottieni_risorsa(&self, email: &str, resource: &str, force: bool) -> EsitoAccesso<String> {
        let key = email.to_lowercase();
        loop {
            let p = self.in_corso.borrow().get(&key).cloned();
            match p {
                Some(p) => {
                    let _ = p.attendi().await;
                }
                None => break,
            }
        }
        self.carica_se_cambiato()?;
        let entry = self.voce(&key);
        let (entry, shared) = match entry {
            Some(e) => {
                let v = e.borrow();
                let sh = shared_di(&v).filter(|_| s(&v, "provider").as_deref() == Some("microsoft"));
                drop(v);
                match sh {
                    Some(sh) => (e, sh),
                    None => return Err(scaduto("shared account unavailable")),
                }
            }
            None => return Err(scaduto("shared account unavailable")),
        };
        allowed(&shared, resource)?;
        let (stored, refresh, identity, client_id_entry, sessione, hit) = {
            let v = entry.borrow();
            let stored = s(&v, "client_id").filter(|c| !c.is_empty()).unwrap_or_else(|| OUTLOOK_CLIENT_ID.to_string());
            let hit = v.get("resourceTokens").and_then(|r| r.get(resource)).cloned();
            (stored, s(&v, "refresh_token").filter(|r| !r.is_empty()), v.get("shared").map(Json::in_js), v.get("client_id").cloned(), v.get("sessione").cloned(), hit)
        };
        let client = client_for(&shared, resource, &stored).to_string();
        let adesso = self.adesso_s();
        if let Some(hit) = hit.filter(|h| vero(h)) {
            let token = hit.get("token").map(stringa_di).unwrap_or_default();
            if !force && hit.get("expiresAt").map_or(f64::NAN, numero_di) > (adesso + 120) as f64 {
                let claims = claims_jwt(&token);
                // a token cached before its resource was asked as Teams (another app) is asked again
                let same_app = client == stored || claims.get("appid").and_then(Json::as_str) == Some(client.as_str());
                let stesso = |k: &str, x: &str| claims.get(k).and_then(Json::as_str) == Some(x);
                if same_app
                    && has_shared_permissions(&shared, resource, &token)
                    && stesso("tid", &shared.tenant_id)
                    && stesso("oid", &shared.object_id)
                    && allowed(&shared, resource)?.contains(&audience(&claims).as_str())
                    && exp_di(&claims) > (adesso + 120) as f64
                {
                    return Ok(token);
                }
            }
        }
        let promessa = Promessa::nuova();
        self.in_corso.borrow_mut().insert(key.clone(), promessa.clone());
        let esito = self.rinnova_risorsa(&key, resource, &shared, &client, refresh.as_deref(), &identity, &client_id_entry, sessione.as_ref()).await;
        let esito = match esito {
            Ok(t) => Ok(t),
            Err(e) => {
                if matches!(e, ErroreAccesso::SharedRefreshExpired(_) | ErroreAccesso::SessioneScaduta(_) | ErroreAccesso::ProxyBrowser(_) | ErroreAccesso::Scaduto(_)) {
                    self.mtime.set(-1.0);
                    self.carica_se_cambiato()?;
                    if let Some(current) = self.voce(&key) {
                        let stesso = {
                            let c = current.borrow();
                            s(&c, "refresh_token").filter(|r| !r.is_empty()) == refresh && c.get("shared").map(Json::in_js) == identity
                        };
                        if stesso {
                            let testo = if matches!(e, ErroreAccesso::SessioneScaduta(_) | ErroreAccesso::ProxyBrowser(_)) {
                                "Sessione del browser scaduta: apri Connect services negli Account"
                            } else {
                                "Accesso scaduto: apri Connect services negli Account"
                            };
                            current.borrow_mut().insert("errore", testo);
                            self.salva()?;
                        }
                    }
                }
                Err(e)
            }
        };
        if self.in_corso.borrow().get(&key).is_some_and(|p| Rc::ptr_eq(p, &promessa)) {
            self.in_corso.borrow_mut().remove(&key);
        }
        promessa.compi(esito.clone());
        esito
    }

    #[allow(clippy::too_many_arguments)]
    async fn rinnova_risorsa(&self, key: &str, resource: &str, shared: &SharedAccount, client: &str, refresh: Option<&str>, identity: &Option<String>, client_id_entry: &Option<Json>, sessione: Option<&Json>) -> EsitoAccesso<String> {
        let mut recovered: Option<SessioneMicrosoft> = None;
        let primo = match refresh {
            None => Err(ErroreAccesso::SharedRefreshExpired("shared refresh missing".to_string())),
            // the family refresh token that comes back serves Outlook Mobile too: it is stored like any other
            Some(r) => renew_shared(shared, resource, r, client, &self.trasporto, self.adesso_s()).await,
        };
        let fresh = match primo {
            Ok(f) => f,
            Err(e) => {
                if !matches!(e, ErroreAccesso::SharedRefreshExpired(_)) || shared.origin != "outlook-spa" {
                    return Err(e);
                }
                let mut session = match sessione.and_then(leggi_sessione) {
                    Some(sx) if js_trim(&sx.login_hint).to_lowercase() == key => sx,
                    _ => return Err(scaduto("shared session unavailable")),
                };
                let json = autorizza_da_sessione(&mut session, &shared.tenant_id, &self.trasporto, self.adesso_s(), None).await?;
                let token = json.get("access_token").filter(|t| vero(t)).map(stringa_di).unwrap_or_default();
                if upn_di(&claims_jwt(&token)) != key {
                    return Err(scaduto("shared session account mismatch"));
                }
                self.preserve_shared(key, &token, &shared.origin)?;
                let motivo = motivo_token_web_non_valido(&Json::from(token.as_str()), key, self.adesso_s() as f64, &richieste(&Some(shared.clone())));
                let nuovo_refresh = json.get("refresh_token").and_then(Json::as_str).filter(|r| !r.is_empty()).map(str::to_string);
                let Some(nuovo_refresh) = nuovo_refresh.filter(|_| motivo.is_none()) else { return Err(scaduto(motivo.unwrap_or("shared session refresh missing"))) };
                recovered = Some(session);
                if resource == "outlook" {
                    oe_rete::shared_account::Rinnovo { expires_at: exp_di(&claims_jwt(&token)), token, refresh: nuovo_refresh }
                } else {
                    renew_shared(shared, resource, &nuovo_refresh, OUTLOOK_WEB_CLIENT_ID, &self.trasporto, self.adesso_s()).await?
                }
            }
        };
        let testo = std::fs::read_to_string(&self.percorso).map_err(|e| altro(format!("ENOENT: {e}")))?;
        Json::analizza(&testo).map_err(|e| altro(e.messaggio))?;
        self.mtime.set(-1.0);
        self.carica_se_cambiato()?;
        let current = self.voce(key);
        let cambiato = match &current {
            None => true,
            Some(c) => {
                let c = c.borrow();
                s(&c, "refresh_token").filter(|r| !r.is_empty()).as_deref() != refresh || &c.get("shared").map(Json::in_js) != identity || &c.get("client_id").cloned() != client_id_entry
            }
        };
        let Some(current) = current.filter(|_| !cambiato) else { return Err(scaduto("shared credential changed during renewal")) };
        let mut next = current.borrow().clone();
        if let Some(sx) = &recovered {
            next.insert("sessione", sx.in_json());
            next.insert("client_id", OUTLOOK_WEB_CLIENT_ID);
        }
        next.insert("refresh_token", fresh.refresh.as_str());
        let mut risorse = match (&recovered, next.get("resourceTokens")) {
            (None, Some(Json::Obj(r))) => r.clone(),
            _ => Mappa::new(),
        };
        let mut t = Mappa::new();
        t.insert("token", fresh.token.as_str());
        t.insert("expiresAt", fresh.expires_at);
        risorse.insert(resource, Json::Obj(t));
        next.insert("resourceTokens", Json::Obj(risorse));
        next.remove("errore");
        if resource == "outlook" {
            next.insert("access_token", fresh.token.as_str());
            next.insert("expires_at", fresh.expires_at);
        }
        let mut data = Archivio { accounts: self.dati.borrow().accounts.clone(), rimossi: self.dati.borrow().rimossi.clone() };
        data.accounts.insert(key.to_string(), Rc::new(RefCell::new(next)));
        scrivi_json_atomico(&self.percorso, &data.in_json()).map_err(|e| altro(e.to_string()))?;
        *self.dati.borrow_mut() = data;
        self.mtime.set(mtime_ms(&self.percorso).map_err(|e| altro(e.to_string()))?);
        self.scrivi_token_posta();
        Ok(fresh.token)
    }

    async fn rinnova_microsoft(&self, chiave: &str, voce: &Voce) -> EsitoAccesso<String> {
        let (refresh, client_id, tenant, ha_sessione, shared) = {
            let v = voce.borrow();
            (s(&v, "refresh_token").filter(|r| !r.is_empty()), s(&v, "client_id"), s(&v, "tenant_id").filter(|t| !t.is_empty()).unwrap_or_else(|| "common".to_string()), ver(&v, "sessione"), shared_di(&v))
        };
        let Some(refresh) = refresh else {
            if ha_sessione {
                return self.rinnova_da_sessione(chiave, voce).await;
            }
            return self.segna_scaduto(voce, "Accesso scaduto: Accedi di nuovo nella finestra Account");
        };
        let web = client_id.as_deref() == Some(OUTLOOK_WEB_CLIENT_ID);
        let url = format!("{MS_LOGIN}/{}/oauth2/v2.0/token", codifica_componente(&tenant));
        let cid = client_id.filter(|c| !c.is_empty()).unwrap_or_else(|| OUTLOOK_CLIENT_ID.to_string());
        let corpo = modulo(&[("client_id", &cid), ("grant_type", "refresh_token"), ("refresh_token", &refresh), ("scope", if web { OUTLOOK_WEB_SCOPES } else { OUTLOOK_SCOPES })]);
        // Il refresh_token del client web (SPA) si riscatta solo da una richiesta cross-origin: serve l'Origin di Outlook web.
        let mut r = Richiesta::post(url, corpo).intestazione("Content-Type", "application/x-www-form-urlencoded").intestazione("Accept", "application/json");
        if web {
            r = r.intestazione("Origin", url::Url::parse(OUTLOOK_WEB_REDIRECT_URI).map(|u| u.origin().ascii_serialization()).unwrap_or_default());
        }
        let resp = self.trasporto.invia(r.scadenza(Duration::from_millis(15_000))).await?;
        let testo = resp.testo()?;
        let json = match Json::analizza(&testo) {
            Ok(Json::Obj(m)) => m,
            _ => Mappa::new(),
        };
        if !resp.ok() || !matches!(json.get("access_token"), Some(Json::Str(_))) {
            let code = json.get("error").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
            if code == "temporarily_unavailable" || code == "server_error" {
                return Err(altro("Microsoft refresh temporarily unavailable"));
            }
            if resp.stato == 400 || resp.stato == 401 || code == "invalid_grant" || code == "interaction_required" {
                // Il refresh_token SPA dura 24 ore al massimo: la sessione del browser ne dà uno nuovo.
                if ha_sessione {
                    return self.rinnova_da_sessione(chiave, voce).await;
                }
                return self.segna_scaduto(voce, "Accesso scaduto: Accedi di nuovo nella finestra Account");
            }
            let dettaglio = json.get("error_description").filter(|v| vero(v)).or_else(|| json.get("error").filter(|v| vero(v))).map(stringa_di).unwrap_or(testo);
            return Err(altro(format!("Rinnovo token Microsoft fallito (HTTP {}: {})", resp.stato, primi_utf16(&dettaglio, 160))));
        }
        if web {
            // Il token del client web vale solo se è della stessa casella, per Outlook, non scaduto e con i calendari, come all'importazione.
            // Uno che non torna non si salva, nemmeno il suo refresh_token: si riprova dalla sessione del browser, se c'è.
            if let Some(motivo) = motivo_token_web_non_valido(&json["access_token"], chiave, self.adesso_s() as f64, &richieste(&shared)) {
                if ha_sessione {
                    return self.rinnova_da_sessione(chiave, voce).await;
                }
                return self.segna_scaduto(voce, &format!("Rinnovo di Outlook web rifiutato: {motivo}"));
            }
        }
        self.applica_token_microsoft(voce, &json)
    }

    fn segna_scaduto(&self, voce: &Voce, testo: &str) -> EsitoAccesso<String> {
        voce.borrow_mut().insert("errore", testo);
        self.salva()?;
        Err(scaduto(testo))
    }

    /// Riautorizza con i cookie della sessione del browser. I cookie ruotati si salvano anche quando Microsoft rifiuta o non risponde.
    async fn rinnova_da_sessione(&self, chiave: &str, voce: &Voce) -> EsitoAccesso<String> {
        // La sessione salvata ripassa dal filtro dell'importazione prima che un cookie esca: solo i cookie di login.microsoftonline.com, e
        // solo per la casella a cui appartiene.
        let (sessione, client_id, tenant, shared) = {
            let v = voce.borrow();
            (v.get("sessione").and_then(leggi_sessione), s(&v, "client_id"), s(&v, "tenant_id").filter(|t| !t.is_empty()).unwrap_or_else(|| "organizations".to_string()), shared_di(&v))
        };
        let Some(mut sessione) = sessione.filter(|sx| js_trim(&sx.login_hint).to_lowercase() == chiave) else {
            return self.segna_scaduto(voce, "Sessione del browser non valida: accedi di nuovo da Outlook web e importa la sessione");
        };
        let mut recovered_web = false;
        let esito = if client_id.as_deref() != Some(OUTLOOK_WEB_CLIENT_ID) {
            let redirect = OUTLOOK_REDIRECT_URI.replace("%3D", "=");
            match autorizza_da_sessione(&mut sessione, &tenant, &self.trasporto, self.adesso_s(), Some(ClienteWeb { id: OUTLOOK_CLIENT_ID, redirect: &redirect })).await {
                Err(ErroreAccesso::SessioneScaduta(_)) => {
                    recovered_web = true;
                    autorizza_da_sessione(&mut sessione, &tenant, &self.trasporto, self.adesso_s(), None).await
                }
                altro => altro,
            }
        } else {
            autorizza_da_sessione(&mut sessione, &tenant, &self.trasporto, self.adesso_s(), None).await
        };
        // autorizza_da_sessione ruota i cookie sulla sessione: si rimette nella voce, così ogni salvataggio da qui in poi li tiene
        voce.borrow_mut().insert("sessione", sessione.in_json());
        let json = match esito {
            Ok(j) => j,
            Err(ErroreAccesso::SessioneScaduta(_) | ErroreAccesso::ProxyBrowser(_)) => {
                return self.segna_scaduto(voce, "Sessione del browser scaduta: accedi di nuovo da Outlook web e importa la sessione");
            }
            Err(e) => {
                self.salva()?;
                return Err(e);
            }
        };
        if let Some(motivo) = motivo_token_web_non_valido(json.get("access_token").unwrap_or(&Json::Null), chiave, self.adesso_s() as f64, &richieste(&shared)) {
            return self.segna_scaduto(voce, &format!("Accesso dalla sessione del browser rifiutato: {motivo}"));
        }
        if recovered_web {
            let mut v = voce.borrow_mut();
            v.insert("client_id", OUTLOOK_WEB_CLIENT_ID);
            if let Some(Json::Obj(sh)) = v.get("shared").filter(|x| vero(x)).cloned() {
                let mut sh = sh;
                sh.insert("origin", "outlook-spa");
                v.insert("shared", Json::Obj(sh));
            }
            v.insert("resourceTokens", Json::obj());
        }
        self.applica_token_microsoft(voce, &json)
    }

    fn applica_token_microsoft(&self, voce: &Voce, json: &Mappa) -> EsitoAccesso<String> {
        let Some(Json::Str(token)) = json.get("access_token") else { return Err(altro("Risposta token Microsoft senza access_token")) };
        let adesso = self.adesso_s() as f64;
        let claims = claims_jwt(token);
        let exp_claim = num(&claims, "exp");
        let exp_in = json.get("expires_in").filter(|v| !v.is_null()).map_or(3600.0, numero_di);
        {
            let mut v = voce.borrow_mut();
            v.insert("access_token", token.as_str());
            v.insert("expires_at", if exp_claim > adesso { exp_claim } else { adesso + if exp_in.is_finite() { exp_in } else { 3600.0 } });
            if let Some(r) = json.get("refresh_token").and_then(Json::as_str).filter(|r| !r.is_empty()) {
                v.insert("refresh_token", r);
            }
            if let Some(t) = claims.get("tid").and_then(Json::as_str).filter(|t| !t.is_empty()) {
                v.insert("tenant_id", t);
            }
            if let Some(scp) = claims.get("scp").and_then(Json::as_str).filter(|t| !t.is_empty()) {
                v.insert("scopes", scp);
            }
            v.remove("errore");
        }
        self.salva()?;
        Ok(token.clone())
    }

    async fn rinnova_google(&self, chiave: &str, voce: &Voce) -> EsitoAccesso<String> {
        let (master, refresh, client_id, secret) = {
            let v = voce.borrow();
            (s(&v, "master_token").filter(|x| !x.is_empty()), s(&v, "refresh_token").filter(|x| !x.is_empty()), s(&v, "client_id").filter(|x| !x.is_empty()), s(&v, "client_secret").filter(|x| !x.is_empty()))
        };
        if let Some(master) = master {
            let corpo = modulo(&[
                ("accountType", "HOSTED_OR_GOOGLE"),
                ("Email", chiave),
                ("has_permission", "1"),
                ("EncryptedPasswd", &master),
                ("service", GOOGLE_CALENDAR_SERVICE),
                ("source", "android"),
                ("app", GOOGLE_CALENDAR_APP),
                ("client_sig", GOOGLE_CLIENT_SIG),
                ("device_country", "it"),
                ("operatorCountry", "it"),
                ("lang", "it"),
                ("sdk_version", "33"),
            ]);
            let r = Richiesta::post("https://android.clients.google.com/auth", corpo)
                .intestazione("Content-Type", "application/x-www-form-urlencoded")
                .intestazione("User-Agent", "GoogleAuth/1.4 (Linux; U; Android 13; it-it); gzip")
                .scadenza(Duration::from_millis(15_000));
            let resp = self.trasporto.invia(r).await?;
            let testo = resp.testo()?;
            let mut righe: Mappa = Mappa::new();
            for l in testo.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| l.contains('=')) {
                let i = l.find('=').unwrap_or_default();
                righe.insert(&l[..i], &l[i + 1..]);
            }
            let auth = righe.get("Auth").and_then(Json::as_str).filter(|a| !a.is_empty()).map(str::to_string);
            let Some(auth) = auth.filter(|_| resp.ok()) else {
                let err = righe.get("Error").and_then(Json::as_str).filter(|e| !e.is_empty()).map_or_else(|| format!("HTTP {}", resp.stato), str::to_string);
                if resp.stato == 401 || resp.stato == 403 || err == "BadAuthentication" || err == "NeedsBrowser" {
                    return self.segna_scaduto(voce, "Accesso Google scaduto: Accedi di nuovo nella finestra Account");
                }
                return Err(altro(format!("Rinnovo token Google fallito ({err})")));
            };
            let durata = righe.get("ExpiresInDurationSec").map_or(3600.0, numero_di);
            {
                let mut v = voce.borrow_mut();
                v.insert("access_token", auth.as_str());
                v.insert("expires_at", self.adesso_s() as f64 + if durata.is_finite() && durata > 0.0 { durata } else { 3600.0 });
                v.remove("errore");
            }
            self.salva()?;
            return Ok(auth);
        }
        if let (Some(refresh), Some(client_id)) = (refresh, client_id) {
            let mut campi = vec![("client_id", client_id.as_str()), ("grant_type", "refresh_token"), ("refresh_token", refresh.as_str())];
            if let Some(sx) = &secret {
                campi.push(("client_secret", sx));
            }
            let r = Richiesta::post("https://oauth2.googleapis.com/token", modulo(&campi))
                .intestazione("Content-Type", "application/x-www-form-urlencoded")
                .intestazione("Accept", "application/json")
                .senza_redirect()
                .scadenza(Duration::from_millis(15_000));
            let resp = self.trasporto.invia(r).await?;
            let json = if self.strict {
                match Json::analizza(&leggi_testo(&resp, Some(GOOGLE_TOKEN_MAX_BYTES))?) {
                    Ok(Json::Obj(m)) => m,
                    Ok(_) => Mappa::new(),
                    Err(e) => return Err(altro(e.messaggio)),
                }
            } else {
                json_o_vuoto(&resp)
            };
            if self.strict {
                self.carica_se_cambiato()?;
                if !self.voce(chiave).is_some_and(|v| Rc::ptr_eq(&v, voce)) {
                    return Err(scaduto("Google access changed during refresh; retry request"));
                }
            }
            let Some(Json::Str(token)) = json.get("access_token").filter(|_| resp.ok()) else {
                return self.segna_scaduto(voce, "Accesso Google scaduto: Accedi di nuovo nella finestra Account");
            };
            let durata = json.get("expires_in").filter(|v| !v.is_null()).map_or(3600.0, numero_di);
            {
                let mut v = voce.borrow_mut();
                v.insert("access_token", token.as_str());
                v.insert("expires_at", self.adesso_s() as f64 + if durata.is_finite() && durata > 0.0 { durata } else { 3600.0 });
                if let Some(r) = json.get("refresh_token").and_then(Json::as_str).filter(|r| !r.is_empty()) {
                    v.insert("refresh_token", r);
                }
                v.remove("errore");
            }
            self.salva()?;
            return Ok(token.clone());
        }
        self.segna_scaduto(voce, "Accesso Google mancante: Accedi di nuovo nella finestra Account")
    }

    /// Restituisce i token Microsoft attivi (rinnovando quelli in scadenza) per GET /api/token-posta.
    pub async fn token_posta_attivi(&self) -> Vec<Json> {
        let _ = self.carica_se_cambiato();
        let mut out = Vec::new();
        for (upn, voce) in self.elenco_account() {
            let (microsoft, senza_posta) = {
                let v = voce.borrow();
                (s(&v, "provider").as_deref() == Some("microsoft"), shared_di(&v).is_some_and(|sh| !sh.ha("mail")))
            };
            if !microsoft || senza_posta {
                continue;
            }
            match self.ottieni_access_token(&upn, false).await {
                Ok(tok) => {
                    let v = voce.borrow();
                    let exp = v.get("expires_at").filter(|e| !e.is_null()).map_or((self.adesso_s() + 3600) as f64, numero_di);
                    out.push(info_posta(&upn, &v, &tok, exp, false, ""));
                }
                Err(e) => {
                    let v = voce.borrow();
                    if !ver(&v, "shared") && ver(&v, "access_token") && num(&v, "expires_at") > self.adesso_s() as f64 {
                        out.push(info_posta(&upn, &v, &stringa_di(&v["access_token"]), num(&v, "expires_at"), true, &e.to_string()));
                    }
                }
            }
        }
        out
    }

    fn prepara_pkce(&self, provider: &'static str, email: Option<&str>) -> (String, String) {
        let adesso = self.adesso_s();
        let mut pkce = self.pkce.borrow_mut();
        pkce.retain(|_, v| adesso - v.quando_s <= TTL_PKCE_S);
        if pkce.len() >= MAX_PKCE_IN_ATTESA {
            pkce.shift_remove_index(0);
        }
        let verifier = casuale_base64url(32);
        let challenge = sha256_base64url(&verifier);
        let state = casuale_base64url(16);
        pkce.insert(state.clone(), Pkce { verifier, provider, email: email.map(|e| js_trim(e).to_string()), quando_s: adesso });
        (challenge, state)
    }

    /// Avvia il flusso OAuth2 PKCE di Outlook Mobile e restituisce l'indirizzo di login Microsoft.
    pub fn inizia_pkce(&self, email: Option<&str>) -> (String, String) {
        let (challenge, state) = self.prepara_pkce("microsoft", email);
        let mut campi = vec![
            ("client_id", OUTLOOK_CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", OUTLOOK_REDIRECT_URI),
            ("response_mode", "query"),
            ("scope", OUTLOOK_SCOPES),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("prompt", "select_account"),
            ("nux", "1"),
            ("msafed", "0"),
        ];
        let hint = email.map(js_trim).filter(|e| !e.is_empty());
        if let Some(h) = hint {
            campi.push(("login_hint", h));
        }
        (format!("{MS_LOGIN}/common/oauth2/v2.0/authorize?{}", modulo(&campi)), state)
    }

    /// Avvia il flusso OAuth2 Device Code di Outlook Mobile (codice + polling automatico).
    pub async fn inizia_device_microsoft(&self, email: Option<&str>) -> EsitoAccesso<EsitoDevice> {
        let (url_pkce, state) = self.inizia_pkce(email);
        let adesso = self.adesso_s();
        self.device.borrow_mut().retain(|_, v| adesso - v.1 <= TTL_PKCE_S);
        let r = Richiesta::post(format!("{MS_LOGIN}/common/oauth2/v2.0/devicecode"), modulo(&[("client_id", OUTLOOK_CLIENT_ID), ("scope", OUTLOOK_SCOPES)]))
            .intestazione("Content-Type", "application/x-www-form-urlencoded")
            .intestazione("Accept", "application/json")
            .scadenza(Duration::from_millis(15_000));
        let resp = self.trasporto.invia(r).await?;
        let json = json_o_vuoto(&resp);
        let (Some(Json::Str(device_code)), Some(Json::Str(user_code))) = (json.get("device_code"), json.get("user_code")) else {
            return Err(altro(format!("Avvio codice dispositivo Microsoft fallito: {}", errore_oauth(&json, resp.stato))));
        };
        if !resp.ok() {
            return Err(altro(format!("Avvio codice dispositivo Microsoft fallito: {}", errore_oauth(&json, resp.stato))));
        }
        self.device.borrow_mut().insert(state.clone(), (device_code.clone(), adesso));
        let url = json.get("verification_uri").and_then(Json::as_str).filter(|u| !u.is_empty()).unwrap_or("https://microsoft.com/devicelogin").to_string();
        let intervallo = json.get("interval").filter(|v| !v.is_null()).map_or(5.0, numero_di);
        Ok(EsitoDevice { url, user_code: user_code.clone(), state, intervallo_s: if intervallo.is_finite() && intervallo > 0.0 { intervallo } else { 5.0 }, url_pkce })
    }

    /// Controlla se l'utente ha completato il login Microsoft Device Code nella scheda del browser.
    pub async fn controlla_device_microsoft(&self, state: &str) -> EsitoAccesso<Controllo> {
        let Some((device_code, _)) = self.device.borrow().get(state).cloned() else {
            return Err(altro("Sessione di accesso scaduta o non trovata: clicca di nuovo su 'Accedi'"));
        };
        let r = Richiesta::post(
            format!("{MS_LOGIN}/common/oauth2/v2.0/token"),
            modulo(&[("client_id", OUTLOOK_CLIENT_ID), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", &device_code)]),
        )
        .intestazione("Content-Type", "application/x-www-form-urlencoded")
        .intestazione("Accept", "application/json")
        .scadenza(Duration::from_millis(15_000));
        let resp = self.trasporto.invia(r).await?;
        let json = json_o_vuoto(&resp);
        let err = json.get("error").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
        if !resp.ok() && (err == "authorization_pending" || err == "slow_down") {
            return Ok(Controllo::InAttesa);
        }
        self.device.borrow_mut().shift_remove(state);
        self.pkce.borrow_mut().shift_remove(state);
        if !resp.ok() || !matches!(json.get("access_token"), Some(Json::Str(_))) {
            return Err(altro(format!("Accesso Microsoft non riuscito: {}", errore_oauth(&json, resp.stato))));
        }
        let (account, tenant_id) = self.salva_risposta_token_microsoft(&json)?;
        Ok(Controllo::Completato { account, tenant_id })
    }

    fn preserve_shared(&self, upn: &str, token: &str, origin: &str) -> EsitoAccesso<Option<Json>> {
        let Some(Json::Obj(grezzo)) = self.voce(upn).and_then(|v| v.borrow().get("shared").filter(|x| vero(x)).cloned()) else { return Ok(None) };
        let Some(shared) = SharedAccount::da_json(&Json::Obj(grezzo.clone())) else { return Ok(None) };
        let claims = claims_jwt(token);
        let exp = exp_di(&claims);
        let stesso = |k: &str, x: &str| claims.get(k).and_then(Json::as_str) == Some(x);
        let solo_posta = SharedAccount { capabilities: vec!["mail".to_string()], ..shared.clone() };
        if !has_shared_permissions(&shared, "outlook", token)
            || !exp.is_finite()
            || !stesso("tid", &shared.tenant_id)
            || !stesso("oid", &shared.object_id)
            || !allowed(&solo_posta, "outlook")?.contains(&audience(&claims).as_str())
            || exp <= (self.adesso_s() + 120) as f64
        {
            return Err(altro("shared identity mismatch"));
        }
        let mut sh = grezzo;
        sh.insert("origin", origin);
        Ok(Some(Json::Obj(sh)))
    }

    fn salva_risposta_token_microsoft(&self, json: &Mappa) -> EsitoAccesso<(String, String)> {
        let mut claims = claims_jwt(json.get("id_token").and_then(Json::as_str).unwrap_or_default());
        let access = json.get("access_token").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_else(|| "undefined".to_string());
        for (k, v) in claims_jwt(&access).iter() {
            claims.insert(k.as_str(), v.clone());
        }
        let upn = ["upn", "preferred_username", "unique_name", "email"].iter().find_map(|k| claims.get(k).filter(|v| vero(v))).map(stringa_di).unwrap_or_default();
        let upn = js_trim(&upn).to_lowercase();
        if upn.is_empty() {
            return Err(altro("Impossibile determinare l'indirizzo email dal token Microsoft"));
        }
        let tid = claims.get("tid").filter(|v| vero(v)).map_or_else(|| "common".to_string(), stringa_di);
        let adesso = self.adesso_s() as f64;
        let exp_claim = num(&claims, "exp");
        let exp_in = json.get("expires_in").filter(|v| !v.is_null()).map_or(3600.0, numero_di);
        self.carica_se_cambiato()?;
        let shared = self.preserve_shared(&upn, &access, "mobile")?;
        let precedente = self.voce(&upn).and_then(|v| v.borrow().get("refresh_token").cloned());
        let mut v = Mappa::new();
        if let Some(sh) = shared {
            v.insert("shared", sh);
        }
        v.insert("provider", "microsoft");
        v.insert("client_id", OUTLOOK_CLIENT_ID);
        v.insert("tenant_id", tid.as_str());
        match json.get("refresh_token") {
            Some(Json::Str(r)) => {
                v.insert("refresh_token", r.as_str());
            }
            _ => {
                if let Some(p) = precedente {
                    v.insert("refresh_token", p);
                }
            }
        }
        v.insert("access_token", access.as_str());
        v.insert("expires_at", if exp_claim > adesso { exp_claim } else { adesso + if exp_in.is_finite() { exp_in } else { 3600.0 } });
        v.insert("scopes", claims.get("scp").and_then(Json::as_str).unwrap_or(OUTLOOK_SCOPES));
        self.dati.borrow_mut().accounts.insert(upn.clone(), Rc::new(RefCell::new(v)));
        self.salva()?;
        Ok((upn, tid))
    }

    /// Importa la sessione del browser (formato teamsrelay session.json) per le caselle dove Outlook Mobile è respinto dall'accesso
    /// condizionale. Prima di salvare prova l'autorizzazione: casella, risorsa Outlook e permesso di posta devono tornare.
    pub async fn importa_sessione(&self, dati: &Json, tenant: &str, valida: Option<&dyn Fn(&str) -> EsitoAccesso<()>>, required: &[&str]) -> EsitoAccesso<(String, String)> {
        let Some(mut sessione) = leggi_sessione(dati) else { return Err(altro("Sessione del browser non valida (servono userAgent, loginHint e cookies)")) };
        let atteso = js_trim(&sessione.login_hint).to_lowercase();
        if !atteso.contains('@') {
            return Err(altro("Nella sessione manca loginHint con l'indirizzo della casella"));
        }
        let t = js_trim(tenant);
        let json = match autorizza_da_sessione(&mut sessione, if t.is_empty() { "organizations" } else { t }, &self.trasporto, self.adesso_s(), None).await {
            Ok(j) => j,
            Err(ErroreAccesso::SessioneScaduta(m)) => return Err(ErroreAccesso::SessioneScaduta(format!("Accesso Microsoft non riuscito: {m}"))),
            Err(e) => return Err(e),
        };
        // stesso controllo di ogni rinnovo del client web: casella, risorsa Outlook, scadenza, calendari
        if let Some(motivo) = motivo_token_web_non_valido(json.get("access_token").unwrap_or(&Json::Null), &atteso, self.adesso_s() as f64, required) {
            return Err(altro(motivo));
        }
        if !json.get("refresh_token").and_then(Json::as_str).is_some_and(|r| !r.is_empty()) {
            return Err(altro("Microsoft did not issue a renewable mailbox grant"));
        }
        let access = stringa_di(&json["access_token"]);
        if let Some(v) = valida {
            v(&access)?;
        }
        let claims = claims_jwt(&access);
        let upn = upn_di(&claims);
        let tid = claims.get("tid").filter(|v| vero(v)).map_or_else(|| tenant.to_string(), stringa_di);
        self.carica_se_cambiato()?;
        let shared = self.preserve_shared(&upn, &access, "outlook-spa")?;
        let mut v = Mappa::new();
        if let Some(sh) = shared {
            v.insert("shared", sh);
        }
        v.insert("provider", "microsoft");
        v.insert("client_id", OUTLOOK_WEB_CLIENT_ID);
        v.insert("tenant_id", tid.as_str());
        v.insert("sessione", sessione.in_json());
        let voce = Rc::new(RefCell::new(v));
        self.dati.borrow_mut().accounts.insert(upn.clone(), voce.clone());
        self.applica_token_microsoft(&voce, &json)?;
        Ok((upn, tid))
    }

    pub async fn importa_guidata(&self, dati: &Json, tenant: &str, valida: Option<&dyn Fn(&str) -> EsitoAccesso<()>>, required: &[&str]) -> EsitoAccesso<(String, String)> {
        let sessione = leggi_sessione(dati);
        let forma = |h: &str| h.split_once('@').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('@') && !h.chars().any(oe_rete::js::spazio_js));
        let Some(mut sessione) = sessione.filter(|sx| forma(&sx.login_hint)) else { return Err(altro("Valid Microsoft session required")) };
        let account = js_trim(&sessione.login_hint).to_lowercase();
        let precedente = self.voce_account(&account).and_then(|v| s(&v.borrow(), "client_id"));
        if precedente.as_deref() != Some(OUTLOOK_WEB_CLIENT_ID) {
            let redirect = OUTLOOK_REDIRECT_URI.replace("%3D", "=");
            let tentativo = async {
                let json = autorizza_da_sessione(&mut sessione, if tenant.is_empty() { "organizations" } else { tenant }, &self.trasporto, self.adesso_s(), Some(ClienteWeb { id: OUTLOOK_CLIENT_ID, redirect: &redirect })).await?;
                if let Some(motivo) = motivo_token_web_non_valido(json.get("access_token").unwrap_or(&Json::Null), &account, self.adesso_s() as f64, required) {
                    return Err(altro(motivo));
                }
                if !json.get("refresh_token").and_then(Json::as_str).is_some_and(|r| !r.is_empty()) {
                    return Err(ErroreAccesso::SessioneScaduta("Microsoft did not issue a renewable mobile grant".to_string()));
                }
                if let Some(v) = valida {
                    v(&stringa_di(&json["access_token"]))?;
                }
                let (acc, _) = self.salva_risposta_token_microsoft(&json)?;
                Ok(acc)
            };
            match tentativo.await {
                Ok(acc) => {
                    if let Some(v) = self.voce(&acc) {
                        v.borrow_mut().insert("sessione", sessione.in_json());
                    }
                    self.salva()?;
                    return Ok((acc, "outlook-mobile".to_string()));
                }
                Err(ErroreAccesso::SessioneScaduta(_)) => {}
                Err(e) => return Err(e),
            }
        }
        self.importa_sessione(&sessione.in_json(), tenant, valida, required).await?;
        Ok((account, "outlook-web".to_string()))
    }

    /// Completa il login OAuth2 PKCE di Outlook Mobile con l'indirizzo msauth://... o il codice ricevuto.
    pub async fn completa_pkce(&self, codice_o_url: &str, state_dato: Option<&str>) -> EsitoAccesso<(String, String)> {
        let (code, state) = estrai_codice_da_input(codice_o_url)?;
        let chiave = self.scegli_state("microsoft", state, state_dato);
        let Some(attesa) = self.pkce.borrow_mut().shift_remove(&chiave) else {
            return Err(altro("Sessione di accesso scaduta o non trovata: clicca di nuovo su 'Accedi con Microsoft'"));
        };
        self.device.borrow_mut().shift_remove(&chiave);
        let r = Richiesta::post(
            format!("{MS_LOGIN}/common/oauth2/v2.0/token"),
            modulo(&[("client_id", OUTLOOK_CLIENT_ID), ("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", OUTLOOK_REDIRECT_URI), ("code_verifier", &attesa.verifier), ("scope", OUTLOOK_SCOPES)]),
        )
        .intestazione("Content-Type", "application/x-www-form-urlencoded")
        .intestazione("Accept", "application/json")
        .scadenza(Duration::from_millis(15_000));
        let resp = self.trasporto.invia(r).await?;
        let json = json_o_vuoto(&resp);
        if !resp.ok() || !matches!(json.get("access_token"), Some(Json::Str(_))) {
            return Err(altro(format!("Scambio codice Microsoft fallito: {}", errore_oauth(&json, resp.stato))));
        }
        self.salva_risposta_token_microsoft(&json)
    }

    fn scegli_state(&self, provider: &str, state: Option<String>, state_dato: Option<&str>) -> String {
        let candidati: Vec<String> = self.pkce.borrow().iter().filter(|(_, v)| v.provider == provider).map(|(k, _)| k.clone()).collect();
        state.filter(|s| !s.is_empty()).or_else(|| state_dato.filter(|s| !s.is_empty()).map(str::to_string)).or_else(|| candidati.last().cloned()).unwrap_or_default()
    }

    /// Avvia il flusso OAuth2 PKCE per Google Calendar e restituisce l'indirizzo di login Google.
    pub fn inizia_google_pkce(&self, email: Option<&str>) -> (String, String) {
        let (challenge, state) = self.prepara_pkce("google", email);
        let mut campi = vec![
            ("client_id", GOOGLE_OAUTH_CLIENT_ID),
            ("redirect_uri", GOOGLE_OAUTH_REDIRECT_URI),
            ("response_type", "code"),
            ("scope", GOOGLE_OAUTH_SCOPES),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ];
        if let Some(h) = email.map(js_trim).filter(|e| !e.is_empty()) {
            campi.push(("login_hint", h));
        }
        (format!("https://accounts.google.com/o/oauth2/v2/auth?{}", modulo(&campi)), state)
    }

    /// Completa il login OAuth2 PKCE di Google Calendar con l'indirizzo http://localhost/?code=... o il codice ricevuto.
    pub async fn completa_google_pkce(&self, codice_o_url: &str, state_dato: Option<&str>, email_dato: Option<&str>) -> EsitoAccesso<String> {
        let (code, state) = estrai_codice_da_input(codice_o_url)?;
        let chiave = self.scegli_state("google", state, state_dato);
        let Some(attesa) = self.pkce.borrow_mut().shift_remove(&chiave) else {
            return Err(altro("Sessione di accesso Google scaduta o non trovata: clicca di nuovo su 'Accedi con Google'"));
        };
        let r = Richiesta::post(
            "https://oauth2.googleapis.com/token",
            modulo(&[
                ("client_id", GOOGLE_OAUTH_CLIENT_ID),
                ("client_secret", GOOGLE_OAUTH_CLIENT_SECRET),
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", GOOGLE_OAUTH_REDIRECT_URI),
                ("code_verifier", &attesa.verifier),
            ]),
        )
        .intestazione("Content-Type", "application/x-www-form-urlencoded")
        .intestazione("Accept", "application/json")
        .scadenza(Duration::from_millis(15_000));
        let resp = self.trasporto.invia(r).await?;
        let json = json_o_vuoto(&resp);
        let Some(Json::Str(access)) = json.get("access_token").filter(|_| resp.ok()) else {
            return Err(altro(format!("Scambio codice Google fallito: {}", errore_oauth(&json, resp.stato))));
        };
        let claims = claims_jwt(json.get("id_token").and_then(Json::as_str).unwrap_or_default());
        let mut email = claims
            .get("email")
            .filter(|v| vero(v))
            .map(stringa_di)
            .or_else(|| email_dato.filter(|e| !e.is_empty()).map(str::to_string))
            .or_else(|| attesa.email.clone().filter(|e| !e.is_empty()))
            .unwrap_or_default();
        email = js_trim(&email).to_lowercase();
        if email.is_empty() {
            let r = Richiesta::get("https://openidconnect.googleapis.com/v1/userinfo").intestazione("Authorization", format!("Bearer {access}")).intestazione("Accept", "application/json").scadenza(Duration::from_millis(10_000));
            let u = self.trasporto.invia(r).await?;
            email = js_trim(&json_o_vuoto(&u).get("email").filter(|v| vero(v)).map(stringa_di).unwrap_or_default()).to_lowercase();
        }
        if email.is_empty() || !email.contains('@') {
            return Err(altro("Impossibile determinare l'indirizzo email dal login Google"));
        }
        let durata = json.get("expires_in").filter(|v| !v.is_null()).map_or(3600.0, numero_di);
        self.carica_se_cambiato()?;
        let precedente = self.voce(&email).and_then(|v| v.borrow().get("refresh_token").cloned());
        let mut v = Mappa::new();
        v.insert("provider", "google");
        v.insert("client_id", GOOGLE_OAUTH_CLIENT_ID);
        v.insert("client_secret", GOOGLE_OAUTH_CLIENT_SECRET);
        match json.get("refresh_token").and_then(Json::as_str).filter(|r| !r.is_empty()) {
            Some(r) => {
                v.insert("refresh_token", r);
            }
            None => {
                if let Some(p) = precedente {
                    v.insert("refresh_token", p);
                }
            }
        }
        v.insert("access_token", access.as_str());
        v.insert("expires_at", self.adesso_s() as f64 + if durata.is_finite() && durata > 0.0 { durata } else { 3600.0 });
        v.insert("scopes", GOOGLE_OAUTH_SCOPES);
        self.dati.borrow_mut().accounts.insert(email.clone(), Rc::new(RefCell::new(v)));
        self.salva()?;
        Ok(email)
    }

    /// Salva e verifica un master token (aas_et/...) o access token per una casella Google.
    pub async fn salva_google(&self, email: &str, token: &str) -> EsitoAccesso<String> {
        let pulita = js_trim(email).to_lowercase();
        let tok = js_trim(token).to_string();
        if pulita.is_empty() || !pulita.contains('@') {
            return Err(altro("Indirizzo email Google non valido"));
        }
        if tok.is_empty() {
            return Err(altro("Token Google mancante"));
        }
        self.carica_se_cambiato()?;
        let precedente = self.voce(&pulita);
        let mut nuova = Mappa::new();
        nuova.insert("provider", "google");
        let access = tok.starts_with("ya29.");
        if access {
            if let Some(m) = precedente.as_ref().and_then(|p| p.borrow().get("master_token").cloned()) {
                nuova.insert("master_token", m);
            }
            nuova.insert("access_token", tok.as_str());
            nuova.insert("expires_at", self.adesso_s() + 3500);
        } else {
            nuova.insert("master_token", tok.as_str());
        }
        self.dati.borrow_mut().accounts.insert(pulita.clone(), Rc::new(RefCell::new(nuova)));
        if let Err(e) = self.ottieni_access_token(&pulita, !access).await {
            let mut d = self.dati.borrow_mut();
            match precedente {
                Some(p) => {
                    d.accounts.insert(pulita, p);
                }
                None => {
                    d.accounts.shift_remove(&pulita);
                }
            }
            return Err(e);
        }
        self.salva()?;
        Ok(pulita)
    }
}

fn errore_oauth(json: &Mappa, stato: u16) -> String {
    json.get("error_description").filter(|v| vero(v)).or_else(|| json.get("error").filter(|v| vero(v))).map_or_else(|| format!("HTTP {stato}"), stringa_di)
}

impl FonteToken for GestoreToken {
    fn ottieni_access_token<'a>(&'a self, email: &'a str, forza: bool) -> FutureToken<'a> {
        Box::pin(async move { GestoreToken::ottieni_access_token(self, email, forza).await.map_err(|e| Box::new(e) as ErroreDyn) })
    }
}

#[derive(Clone)]
struct CalInfo {
    id: String,
    nome: String,
}

pub struct CalendarioDiretto {
    pub id: String,
    pub nome: String,
    pub account: String,
    pub calendari: Vec<String>,
    ctx: Contesto,
    tokens: Rc<GestoreToken>,
    stato: RefCell<Stato>,
    tipo: RefCell<Option<String>>,
    conteggio: Cell<Option<f64>>,
    cache_calendari: RefCell<Option<(i64, Vec<CalInfo>)>>,
}

/// Un errore della lettura diretta: quelli del nucleo (orari impossibili) risalgono, gli altri diventano lo stato della casella.
enum ErroreLettura {
    Nucleo(Errore),
    Casella(String),
}

impl From<ErroreRest> for ErroreLettura {
    fn from(e: ErroreRest) -> Self {
        ErroreLettura::Casella(e.to_string())
    }
}

impl From<ErroreAccesso> for ErroreLettura {
    fn from(e: ErroreAccesso) -> Self {
        ErroreLettura::Casella(e.to_string())
    }
}

fn data_iso(s: &str) -> Result<Date, ErroreLettura> {
    let t = s.get(..10).unwrap_or(s);
    t.parse::<Date>().map_err(|e| ErroreLettura::Casella(format!("RangeError: {e}")))
}

/// `Temporal.Instant.from(testo).toZonedDateTimeISO(zona).toPlainDateTime()`
fn istante_locale(testo: &str, tz: &TimeZone) -> Result<DateTime, ErroreLettura> {
    let t: Timestamp = testo.parse().map_err(|e| ErroreLettura::Casella(format!("RangeError: {e}")))?;
    Ok(t.to_zoned(tz.clone()).datetime())
}

/// `instant.toString()`: AAAA-MM-GGTHH:MM:SS[.frazione]Z
fn testo_istante(t: Timestamp) -> String {
    let w = t.to_zoned(TimeZone::UTC).datetime();
    let mut s = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", w.year(), w.month(), w.day(), w.hour(), w.minute(), w.second());
    let ns = w.subsec_nanosecond();
    if ns != 0 {
        s.push_str(format!(".{ns:09}").trim_end_matches('0'));
    }
    s.push('Z');
    s
}

impl CalendarioDiretto {
    pub fn nuovo(id: String, nome: String, account: &str, calendari: Option<Vec<String>>, ctx: Contesto, tokens: Rc<GestoreToken>) -> Self {
        let account = py_strip(account).to_lowercase();
        let tipo = tokens.voce_account(&account).map(|v| if s(&v.borrow(), "provider").as_deref() == Some("google") { "caldav" } else { "outlook" }.to_string());
        CalendarioDiretto {
            id,
            nome,
            account,
            calendari: calendari.unwrap_or_default().iter().map(|c| py_strip(c).to_lowercase()).collect(),
            ctx,
            tokens,
            stato: RefCell::new(Stato::non_letto()),
            tipo: RefCell::new(tipo),
            conteggio: Cell::new(None),
            cache_calendari: RefCell::new(None),
        }
    }

    pub fn stato(&self) -> Stato {
        self.stato.borrow().clone()
    }

    pub fn tipo(&self) -> Option<String> {
        self.tipo.borrow().clone()
    }

    pub fn svuota_cache(&self) {
        *self.cache_calendari.borrow_mut() = None;
    }

    /// Lo stesso trasporto del gateway di posta e calendari: un 401 rinnova il token una volta e riprova.
    async fn richiesta_json(&self, url: &str, intestazioni: &[(&str, String)]) -> Result<Json, ErroreLettura> {
        let senza_calendario = self.tokens.voce_account(&self.account).and_then(|v| shared_di(&v.borrow())).is_some_and(|sh| !sh.ha("calendar"));
        if senza_calendario {
            return Err(ErroreLettura::Casella("calendar capability disabled".to_string()));
        }
        let mut h = Intestazioni::new();
        for (k, v) in intestazioni {
            h.imposta(k, v.clone());
        }
        Ok(json_rest(self.tokens.as_ref(), &self.tokens.trasporto, &self.account, url, &OpzioniRest { headers: h, ..Default::default() }).await?)
    }

    async fn calendari_scelti(&self, forza: bool) -> Result<Vec<CalInfo>, ErroreLettura> {
        let adesso = (self.ctx.adesso)().div_euclid(1000);
        if let Some((quando, scelti)) = self.cache_calendari.borrow().as_ref() {
            if !forza && adesso - quando < TTL_CALENDARI_S {
                return Ok(scelti.clone());
            }
        }
        let Some(voce) = self.tokens.voce_account(&self.account) else {
            return Err(ErroreAccesso::Scaduto(format!("La casella {} non ha credenziali salvate (Accedi di nuovo nella finestra Account)", self.account)).into());
        };
        let google = s(&voce.borrow(), "provider").as_deref() == Some("google");
        *self.tipo.borrow_mut() = Some(if google { "caldav" } else { "outlook" }.to_string());
        let mut scelti: Vec<CalInfo> = Vec::new();
        let mut tutti: Vec<CalInfo> = Vec::new();
        if google {
            let json = self.richiesta_json("https://www.googleapis.com/calendar/v3/users/me/calendarList?maxResults=250", &[]).await?;
            for it in json.get("items").and_then(Json::as_arr).map(Vec::as_slice).unwrap_or_default() {
                let id = it.get("id").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
                let summary = it.get("summary").filter(|v| !v.is_null()).map_or_else(|| id.clone(), stringa_di);
                if id.is_empty() {
                    continue;
                }
                tutti.push(CalInfo { id: id.clone(), nome: summary.clone() });
                let scelto = if self.calendari.is_empty() { it.get("primary").is_some_and(vero) } else { self.calendari.contains(&summary.to_lowercase()) || self.calendari.contains(&id.to_lowercase()) };
                if scelto {
                    scelti.push(CalInfo { id, nome: summary });
                }
            }
        } else {
            let json = self.richiesta_json(&format!("{OUTLOOK_REST}/me/calendars?$top=50&$select=Id,Name,IsDefaultCalendar"), &[]).await?;
            for it in json.get("value").and_then(Json::as_arr).map(Vec::as_slice).unwrap_or_default() {
                let id = it.get("Id").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
                let nome = it.get("Name").filter(|v| !v.is_null()).map_or_else(|| "Calendario".to_string(), stringa_di);
                if id.is_empty() {
                    continue;
                }
                tutti.push(CalInfo { id: id.clone(), nome: nome.clone() });
                let scelto = if self.calendari.is_empty() { it.get("IsDefaultCalendar").is_some_and(vero) } else { self.calendari.contains(&nome.to_lowercase()) };
                if scelto {
                    scelti.push(CalInfo { id, nome });
                }
            }
        }
        if scelti.is_empty() && self.calendari.is_empty() {
            if let Some(primo) = tutti.first() {
                scelti.push(primo.clone());
            }
        }
        if scelti.is_empty() {
            return Err(ErroreLettura::Casella(format!("Nessun calendario trovato per {}", self.account)));
        }
        *self.cache_calendari.borrow_mut() = Some((adesso, scelti.clone()));
        Ok(scelti)
    }

    fn segna_ok(&self) {
        let ora = ora_minuti((self.ctx.adesso)(), &self.ctx.zona);
        *self.stato.borrow_mut() = Stato { stato: "ok".to_string(), msg: Json::from(format!("Letto alle {ora}")) };
    }

    fn segna_errore(&self, msg: &str) -> Vec<String> {
        (self.ctx.registro)(Livello::Warning, &format!("{}: lettura non riuscita ({msg})", self.nome));
        *self.stato.borrow_mut() = Stato { stato: "errore".to_string(), msg: Json::from(msg) };
        vec![format!("{}: {msg}", self.nome)]
    }

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

    pub async fn riepilogo(&self) -> Esito<Json> {
        match self.calendari_scelti(true).await {
            Ok(_) => {
                if self.conteggio.get().is_none() {
                    self.conteggio.set(Some(0.0));
                }
                self.segna_ok();
            }
            Err(ErroreLettura::Casella(m)) => {
                self.segna_errore(&m);
            }
            Err(ErroreLettura::Nucleo(e)) => {
                self.segna_errore(e.messaggio());
            }
        }
        Ok(self.descrizione())
    }

    pub async fn eventi(&self, inizio: DateTime, fine: DateTime) -> Esito<(Vec<Json>, Vec<String>)> {
        let zona: Rc<str> = Rc::from(self.ctx.zona.as_str());
        if inizio > fine {
            let (da, a) = (ora_t(inizio, Some(zona.clone()), 0), ora_t(fine, Some(zona.clone()), 0));
            contiene_evento(&da, &a, &da, &da)?;
        }
        let cals = match self.calendari_scelti(false).await {
            Ok(c) => c,
            Err(ErroreLettura::Casella(m)) => return Ok((Vec::new(), self.segna_errore(&m))),
            Err(ErroreLettura::Nucleo(e)) => return Ok((Vec::new(), self.segna_errore(e.messaggio()))),
        };
        match self.leggi_eventi(inizio, fine, &cals, &zona).await {
            Ok((risultati, totale)) => {
                let n = self.conteggio.get().unwrap_or(0.0).max(totale as f64).max(risultati.len() as f64);
                self.conteggio.set(Some(n));
                self.segna_ok();
                Ok((risultati, Vec::new()))
            }
            Err(ErroreLettura::Nucleo(e @ (Errore::Altro(_) | Errore::Tipo(_)))) => Err(e),
            Err(ErroreLettura::Nucleo(e)) => Ok((Vec::new(), self.segna_errore(e.messaggio()))),
            Err(ErroreLettura::Casella(m)) => Ok((Vec::new(), self.segna_errore(&m))),
        }
    }

    async fn leggi_eventi(&self, inizio: DateTime, fine: DateTime, cals: &[CalInfo], zona: &Rc<str>) -> Result<(Vec<Json>, usize), ErroreLettura> {
        let Some(voce) = self.tokens.voce_account(&self.account) else { return Err(ErroreLettura::Casella("Cannot read properties of undefined (reading 'provider')".to_string())) };
        let google = s(&voce.borrow(), "provider").as_deref() == Some("google");
        let tz = fuso_iana(zona).map_err(ErroreLettura::Nucleo)?;
        // Allarga di 1 giorno in UTC la query HTTP per coprire i giorni interi a cavallo della mezzanotte
        let istante = |w: DateTime, ore: i64| -> Result<String, ErroreLettura> {
            let z = tz.to_ambiguous_zoned(w).compatible().map_err(|e| ErroreLettura::Casella(format!("RangeError: {e}")))?;
            let t = z.timestamp().checked_add(SignedDuration::from_hours(ore)).map_err(|e| ErroreLettura::Casella(format!("RangeError: {e}")))?;
            Ok(testo_istante(t))
        };
        let utc_inizio = istante(inizio, -24)?;
        let utc_fine = istante(fine, 24)?;
        let da_finestra = ora_t(inizio, Some(zona.clone()), 0);
        let a_finestra = ora_t(fine, Some(zona.clone()), 0);
        let dentro = |i: DateTime, f: DateTime| -> Result<bool, ErroreLettura> {
            contiene_evento(&da_finestra, &a_finestra, &ora_t(i, Some(zona.clone()), 0), &ora_t(f, Some(zona.clone()), 0)).map_err(ErroreLettura::Nucleo)
        };
        let mut candidati: IndexMap<String, ([bool; 4], Mappa)> = IndexMap::new();
        let mut totale = 0usize;
        for cal in cals {
            let (items, prefisso, tipo) = if google {
                let url = format!(
                    "https://www.googleapis.com/calendar/v3/calendars/{}/events?singleEvents=true&timeMin={}&timeMax={}&maxResults=2500",
                    codifica_componente(&cal.id),
                    codifica_componente(&utc_inizio),
                    codifica_componente(&utc_fine)
                );
                (self.richiesta_json(&url, &[]).await?.get("items").and_then(Json::as_arr).cloned().unwrap_or_default(), "g", "caldav")
            } else {
                let url = format!(
                    "{OUTLOOK_REST}/me/calendars/{}/calendarview?startDateTime={}&endDateTime={}&$top=1000&$select=Id,SeriesMasterId,Type,Subject,BodyPreview,Body,Start,End,IsAllDay,IsCancelled,ShowAs,Location,Organizer,Attendees,OnlineMeetingUrl,OnlineMeeting",
                    codifica_componente(&cal.id),
                    codifica_componente(&utc_inizio),
                    codifica_componente(&utc_fine)
                );
                let h = [("Prefer", format!("outlook.timezone=\"{}\"", self.ctx.zona))];
                (self.richiesta_json(&url, &h).await?.get("value").and_then(Json::as_arr).cloned().unwrap_or_default(), "m", "outlook")
            };
            totale += items.len();
            for it in &items {
                let letto = if google { self.evento_google(it, &tz)? } else { self.evento_outlook(it)? };
                let Some(ev) = letto else { continue };
                if !dentro(ev.inizio, ev.fine)? {
                    continue;
                }
                let link = MEETING_URL.find(&ev.luogo).or_else(|| MEETING_URL.find(&ev.descrizione)).map(|m| m.as_str().to_string()).or(ev.link_extra.clone());
                let online_url = match link.filter(|l| !l.is_empty()) {
                    Some(l) => Some(pulisci_link(&l).map_err(ErroreLettura::Nucleo)?),
                    None => None,
                };
                let inizio_str = testo_naive(&ev.inizio);
                let fine_str = testo_naive(&ev.fine);
                let titolo = if ev.titolo.is_empty() { "(senza titolo)".to_string() } else { ev.titolo.clone() };
                let mut e = Mappa::new();
                e.insert("id", format!("{}|{prefisso}{}|{inizio_str}", self.id, ev.uid));
                e.insert("account", self.account.as_str());
                e.insert("alias", self.nome.as_str());
                e.insert("calendar", format!("{} · {}", self.account, cal.nome));
                e.insert("title", titolo.as_str());
                e.insert("start", inizio_str.as_str());
                e.insert("end", fine_str.as_str());
                e.insert("allDay", ev.tutto_giorno);
                e.insert("location", ev.luogo.as_str());
                e.insert("organizer", ev.org_nome.as_str());
                e.insert("organizer_email", ev.org_email.as_str());
                e.insert("attendees", ev.invitati.clone());
                e.insert("online_url", online_url.clone());
                e.insert("web_link", Json::Null);
                e.insert("fonte", "android");
                e.insert("tipo", tipo);
                e.insert("occupato", ev.occupato);
                let peso = [!ev.serie, online_url.as_deref().is_some_and(|u| !u.is_empty()), !ev.luogo.is_empty(), !ev.org_email.is_empty()];
                let chiave = Json::Arr(vec![Json::from(titolo.as_str()), Json::from(inizio_str.as_str()), Json::from(fine_str.as_str())]).in_js();
                let sostituisci = candidati.get(&chiave).is_none_or(|(attuale, _)| peso.iter().zip(attuale).find(|(x, y)| x != y).is_some_and(|(x, _)| *x));
                if sostituisci {
                    candidati.insert(chiave, (peso, e));
                }
            }
        }
        let mut risultati: Vec<Mappa> = candidati.into_values().map(|(_, e)| e).collect();
        let t = |m: &Mappa, k: &str| m.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
        risultati.sort_by(|a, b| t(a, "start").cmp(&t(b, "start")).then_with(|| t(a, "title").cmp(&t(b, "title"))));
        Ok((risultati.into_iter().map(Json::Obj).collect(), totale))
    }

    fn evento_google(&self, it: &Json, tz: &TimeZone) -> Result<Option<EventoLetto>, ErroreLettura> {
        if it.get("status").and_then(Json::as_str) == Some("cancelled") {
            return Ok(None);
        }
        let vuoto = Json::obj();
        let start = it.get("start").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let end = it.get("end").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let tutto_giorno = matches!(start.get("date"), Some(Json::Str(_))) && !start.get("dateTime").is_some_and(vero);
        let (inizio, fine) = if tutto_giorno {
            let i = data_iso(&stringa_di(&start["date"]))?.at(0, 0, 0, 0);
            let f = match end.get("date").filter(|v| vero(v)) {
                Some(d) => data_iso(&stringa_di(d))?.at(0, 0, 0, 0),
                None => i.checked_add(jiff::Span::new().days(1)).map_err(|e| ErroreLettura::Casella(format!("RangeError: {e}")))?,
            };
            (i, f)
        } else if let Some(Json::Str(dt)) = start.get("dateTime") {
            let i = istante_locale(dt, tz)?;
            let f = match end.get("dateTime") {
                Some(Json::Str(d)) => istante_locale(d, tz)?,
                _ => i,
            };
            (i, f)
        } else {
            return Ok(None);
        };
        let luogo = it.get("location").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
        let descrizione = it.get("description").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
        let mut extra = None;
        if MEETING_URL.find(&luogo).is_none() && MEETING_URL.find(&descrizione).is_none() {
            if let Some(Json::Str(h)) = it.get("hangoutLink").filter(|h| vero(h)) {
                extra = Some(h.clone());
            } else if let Some(eps) = it.get("conferenceData").filter(|c| matches!(c, Json::Obj(_) | Json::Arr(_))).and_then(|c| c.get("entryPoints")).and_then(Json::as_arr) {
                extra = eps.iter().filter_map(|ep| ep.get("uri").and_then(Json::as_str)).find(|u| MEETING_URL.is_match(u)).map(str::to_string);
            }
        }
        let vuoto = Json::obj();
        let org = it.get("organizer").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let org_email = org.get("email").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default().to_lowercase();
        let org_nome = org.get("displayName").filter(|v| vero(v)).or_else(|| org.get("email").filter(|v| vero(v))).map(stringa_di).unwrap_or_default();
        let invitati = invitati(it.get("attendees"), |a| a.get("email").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default());
        let uid = it.get("recurringEventId").filter(|v| vero(v)).or_else(|| it.get("id").filter(|v| vero(v))).map(stringa_di).unwrap_or_default();
        Ok(Some(EventoLetto {
            inizio,
            fine,
            tutto_giorno,
            luogo,
            descrizione,
            link_extra: extra,
            org_email,
            org_nome,
            invitati,
            uid,
            serie: it.get("recurringEventId").is_some_and(vero),
            occupato: it.get("transparency").and_then(Json::as_str) != Some("transparent"),
            titolo: it.get("summary").filter(|v| vero(v)).map(stringa_di).unwrap_or_default(),
        }))
    }

    fn evento_outlook(&self, it: &Json) -> Result<Option<EventoLetto>, ErroreLettura> {
        if it.get("IsCancelled") == Some(&Json::Bool(true)) {
            return Ok(None);
        }
        let vuoto = Json::obj();
        let start = it.get("Start").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let end = it.get("End").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let grezzo = |o: &Json| primi_utf16(&o.get("DateTime").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default(), 19);
        let (start_raw, end_raw) = (grezzo(start), grezzo(end));
        if start_raw.encode_utf16().count() < 19 {
            return Ok(None);
        }
        let tutto_giorno = it.get("IsAllDay").is_some_and(vero);
        let leggi = |r: &str| -> Result<DateTime, ErroreLettura> {
            if tutto_giorno {
                Ok(data_iso(r)?.at(0, 0, 0, 0))
            } else {
                r.parse::<DateTime>().map_err(|e| ErroreLettura::Casella(format!("RangeError: {e}")))
            }
        };
        let inizio = leggi(&start_raw)?;
        let fine = if end_raw.encode_utf16().count() >= 19 { leggi(&end_raw)? } else { inizio };
        let vuoto = Json::obj();
        let luogo = it.get("Location").filter(|v| !v.is_null()).unwrap_or(&vuoto).get("DisplayName").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
        let corpo = it.get("Body").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let descrizione = corpo.get("Content").filter(|v| !v.is_null()).or_else(|| it.get("BodyPreview").filter(|v| !v.is_null())).map(stringa_di).unwrap_or_default();
        let online = it.get("OnlineMeeting").filter(|v| !v.is_null()).unwrap_or(&vuoto);
        let join = online.get("JoinUrl").filter(|v| vero(v)).or_else(|| it.get("OnlineMeetingUrl").filter(|v| vero(v))).map(stringa_di).unwrap_or_default();
        let extra = if MEETING_URL.find(&luogo).is_none() && MEETING_URL.find(&descrizione).is_none() {
            if join.is_empty() { None } else { Some(MEETING_URL.find(&join).map_or_else(|| join.clone(), |m| m.as_str().to_string())) }
        } else {
            None
        };
        let org = it.get("Organizer").filter(|v| !v.is_null()).unwrap_or(&vuoto).get("EmailAddress").cloned().unwrap_or_default();
        let org_email = org.get("Address").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default().to_lowercase();
        let org_nome = org.get("Name").filter(|v| vero(v)).or_else(|| org.get("Address").filter(|v| vero(v))).map(stringa_di).unwrap_or_default();
        let invitati = invitati(it.get("Attendees"), |a| a.get("EmailAddress").filter(|v| !v.is_null()).and_then(|e| e.get("Address")).filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default());
        let tipo = it.get("Type").and_then(Json::as_str);
        let serie = tipo == Some("Occurrence") || (it.get("SeriesMasterId").is_some_and(vero) && tipo != Some("Exception"));
        let uid = it.get("SeriesMasterId").filter(|v| vero(v)).or_else(|| it.get("Id").filter(|v| vero(v))).map(stringa_di).unwrap_or_default();
        let show_as = it.get("ShowAs").filter(|v| !v.is_null()).map(stringa_di).unwrap_or_default();
        Ok(Some(EventoLetto {
            inizio,
            fine,
            tutto_giorno,
            luogo,
            descrizione,
            link_extra: extra,
            org_email,
            org_nome,
            invitati,
            uid,
            serie,
            occupato: show_as == "Busy" || show_as == "Oof",
            titolo: it.get("Subject").filter(|v| vero(v)).map(stringa_di).unwrap_or_default(),
        }))
    }
}

struct EventoLetto {
    inizio: DateTime,
    fine: DateTime,
    tutto_giorno: bool,
    luogo: String,
    descrizione: String,
    /// Il link quando né il luogo né la descrizione ne hanno uno: hangoutLink, conferenceData, OnlineMeeting.
    link_extra: Option<String>,
    org_email: String,
    org_nome: String,
    invitati: Vec<String>,
    uid: String,
    serie: bool,
    occupato: bool,
    titolo: String,
}

fn invitati(lista: Option<&Json>, indirizzo: impl Fn(&Json) -> String) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in lista.and_then(Json::as_arr).map(Vec::as_slice).unwrap_or_default() {
        let e = if a.is_null() { String::new() } else { indirizzo(a).to_lowercase() };
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out.sort();
    out
}
