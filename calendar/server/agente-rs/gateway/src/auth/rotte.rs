//! /api/auth/*: le strade di Better Auth che la pagina usa, con le stesse risposte, gli stessi cookie e lo stesso ordine dei controlli
//! (limite di richieste, strada, corpo, origine, ganci prima, validazione, sessione, gestore, ganci dopo). Le altre strade di Better Auth
//! (registrazione, cambio di email o password, elenco delle sessioni...) la pagina non le usa e qui non ci sono: rispondono 404.

use super::cookie::{self, nome, SCADENZA_S};
use super::righe::{self, Sessione, Utente};
use crate::config::{self, Modo2fa, PASSWORD_MAX};
use crate::lockout;
use crate::py::{Errore, Esito};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;

/// Un giorno: dopo, una sessione si rinnova alla prima lettura; e una sessione più vecchia non è "fresca" per registrare una passkey.
pub const UN_GIORNO_S: i64 = 60 * 60 * 24;

/// Una richiesta come la vede Better Auth.
pub struct Richiesta<'a> {
    pub metodo: &'a str,
    pub url: &'a url::Url,
    /// Nomi in minuscolo, nell'ordine di arrivo.
    pub intestazioni: &'a [(String, String)],
    pub corpo: &'a [u8],
}

impl Richiesta<'_> {
    /// Headers.get di Fetch: i valori di due intestazioni uguali uniti con ", " (il cookie con "; ").
    pub fn intestazione(&self, n: &str) -> Option<String> {
        intestazione(self.intestazioni, n)
    }
}

pub fn intestazione(intestazioni: &[(String, String)], n: &str) -> Option<String> {
    let valori: Vec<&str> = intestazioni.iter().filter(|(k, _)| k == n).map(|(_, v)| v.as_str()).collect();
    if valori.is_empty() {
        return None;
    }
    Some(valori.join(if n == "cookie" { "; " } else { ", " }))
}

pub struct Risposta {
    pub stato: u16,
    pub intestazioni: Vec<(String, String)>,
    /// Il corpo JSON; None: nessun corpo.
    pub corpo: Option<String>,
}

/// Un APIError: lo stato e il corpo (None quando l'errore non ne ha).
#[derive(Debug)]
pub struct ErroreApi {
    pub stato: u16,
    pub corpo: Option<Json>,
}

pub fn errore(stato: u16, codice: &str, messaggio: &str) -> ErroreApi {
    ErroreApi { stato, corpo: Some(ogg! { "message": messaggio, "code": codice }) }
}

pub fn errore_messaggio(stato: u16, messaggio: &str) -> ErroreApi {
    ErroreApi { stato, corpo: Some(ogg! { "message": messaggio }) }
}

impl From<Errore> for ErroreApi {
    fn from(e: Errore) -> Self {
        eprintln!("auth: {}: {}", e.nome(), e.messaggio());
        ErroreApi { stato: 500, corpo: None }
    }
}

pub type Uscita<T> = Result<T, ErroreApi>;

pub fn non_autorizzato() -> ErroreApi {
    errore(401, "UNAUTHORIZED", "Unauthorized")
}

/// Il contesto di una richiesta: quello che arriva, le intestazioni che si accumulano per la risposta e la sessione letta.
pub struct Ctx<'a> {
    pub r: &'a Richiesta<'a>,
    pub modo: Modo2fa,
    pub segreto: String,
    pub corpo: Option<Json>,
    pub query: Mappa,
    pub uscita: Vec<(String, String)>,
    pub sessione: Option<Option<(Sessione, Utente)>>,
    pub nuova: Option<(Sessione, Utente)>,
}

impl<'a> Ctx<'a> {
    pub fn nuovo(r: &'a Richiesta<'a>, modo: Modo2fa) -> Self {
        let query = r.url.query_pairs().fold(Mappa::new(), |mut m, (k, v)| {
            m.insert(k.into_owned(), v.into_owned());
            m
        });
        Ctx { r, modo, segreto: config::auth_secret(), corpo: None, query, uscita: Vec::new(), sessione: None, nuova: None }
    }

    pub fn ora(&self) -> i64 {
        crate::orologio::ora_vera()
    }

    pub fn cookie(&self) -> Option<String> {
        self.r.intestazione("cookie")
    }

    pub fn firmato(&self, quale: &str) -> Option<String> {
        cookie::firmato(self.cookie().as_deref(), &nome(quale), &self.segreto)
    }

    pub fn imposta(&mut self, riga: String) {
        self.uscita.push(("set-cookie".to_string(), riga));
    }

    pub fn intestazione_uscita(&mut self, k: &str, v: &str) {
        self.uscita.retain(|(x, _)| x != k);
        self.uscita.push((k.to_string(), v.to_string()));
    }

    /// removeSetCookieEntries: toglie dalla risposta i Set-Cookie già messi per quel nome (e i suoi pezzi).
    fn togli_in_uscita(&mut self, nome_completo: &str) {
        let esatto = format!("{nome_completo}=");
        let pezzo = format!("{nome_completo}.");
        self.uscita.retain(|(k, v)| k != "set-cookie" || !(v.starts_with(&esatto) || v.starts_with(&pezzo)));
    }

    /// expireCookie
    pub fn scadi(&mut self, quale: &str) {
        let n = nome(quale);
        self.togli_in_uscita(&n);
        self.imposta(cookie::scaduto(&n));
    }

    /// I pezzi del cookie della copia della sessione che la richiesta porta (session_data e session_data.N): si fanno scadere.
    fn pulisci_dati_sessione(&mut self) {
        let n = nome(cookie::DATI_SESSIONE);
        let presenti: Vec<String> = cookie::leggi_tutti(self.cookie().as_deref().unwrap_or(""))
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| *k == n || k.strip_prefix(&format!("{n}.")).is_some_and(|i| i.parse::<u64>().is_ok_and(|x| x.to_string() == i)))
            .collect();
        for k in presenti {
            self.imposta(cookie::scaduto(&k));
        }
    }

    /// deleteSessionCookie
    pub fn elimina_cookie_sessione(&mut self, salta_non_ricordare: bool) {
        self.scadi(cookie::SESSIONE);
        self.scadi(cookie::DATI_SESSIONE);
        self.pulisci_dati_sessione();
        if !salta_non_ricordare {
            self.scadi(cookie::NON_RICORDARE);
        }
    }

    /// setSessionCookie: il token firmato (30 giorni, o fino alla chiusura del browser se non va ricordata) e la nuova sessione nel contesto.
    pub fn imposta_cookie_sessione(&mut self, s: &Sessione, u: &Utente, non_ricordare: Option<bool>) {
        let non_ricordare = non_ricordare.unwrap_or_else(|| self.firmato(cookie::NON_RICORDARE).is_some());
        let riga = cookie::scrivi_firmato(&nome(cookie::SESSIONE), &s.token, &self.segreto, (!non_ricordare).then_some(SCADENZA_S));
        self.imposta(riga);
        if non_ricordare {
            let riga = cookie::scrivi_firmato(&nome(cookie::NON_RICORDARE), "true", &self.segreto, None);
            self.imposta(riga);
        }
        self.nuova = Some((s.clone(), u.clone()));
    }

    /// getSession: la sessione del cookie, rinnovata se ha più di un giorno; scaduta o sconosciuta, i cookie si cancellano.
    pub fn leggi_sessione(&mut self, senza_rinnovo: bool) -> Uscita<Option<(Sessione, Utente)>> {
        let ora = self.ora();
        let token = self.firmato(cookie::SESSIONE);
        if cookie::leggi_tutti(self.cookie().as_deref().unwrap_or("")).iter().any(|(k, _)| *k == nome(cookie::DATI_SESSIONE) || k.starts_with(&format!("{}.", nome(cookie::DATI_SESSIONE)))) {
            self.pulisci_dati_sessione();
        }
        let Some(token) = token else { return Ok(None) };
        let non_ricordare = self.firmato(cookie::NON_RICORDARE).is_some();
        let trovata = righe::trova_sessione(&token).map_err(|_| errore(500, "FAILED_TO_GET_SESSION", "Failed to get session"))?;
        let Some((s, u)) = trovata.filter(|(s, _)| s.expires_at >= ora) else {
            self.elimina_cookie_sessione(false);
            if let Ok(Some(_)) = righe::trova_sessione(&token) {
                let _ = righe::elimina_sessione(&token);
            }
            return Ok(None);
        };
        if non_ricordare || senza_rinnovo {
            return Ok(Some((s, u)));
        }
        if s.expires_at - SCADENZA_S * 1000 + UN_GIORNO_S * 1000 <= ora {
            let Some(rinnovata) = righe::rinnova_sessione(&s.token, ora + SCADENZA_S * 1000, ora).map_err(|_| errore(500, "FAILED_TO_GET_SESSION", "Failed to get session"))? else {
                self.elimina_cookie_sessione(false);
                return Err(errore(401, "FAILED_TO_GET_SESSION", "Failed to get session"));
            };
            let riga = cookie::scrivi_firmato(&nome(cookie::SESSIONE), &rinnovata.token, &self.segreto, Some(SCADENZA_S));
            self.imposta(riga);
            self.nuova = Some((rinnovata.clone(), u.clone()));
            return Ok(Some((rinnovata, u)));
        }
        Ok(Some((s, u)))
    }

    /// getSessionFromCtx: letta una volta per richiesta.
    pub fn sessione(&mut self) -> Uscita<Option<(Sessione, Utente)>> {
        if let Some(s) = &self.sessione {
            return Ok(s.clone());
        }
        let s = self.leggi_sessione(false).unwrap_or(None);
        self.sessione = Some(s.clone());
        Ok(s)
    }

    /// sessionMiddleware
    pub fn richiedi_sessione(&mut self) -> Uscita<(Sessione, Utente)> {
        self.sessione()?.ok_or_else(non_autorizzato)
    }

    /// freshSessionMiddleware: una sessione di più di un giorno non basta per registrare una passkey.
    pub fn richiedi_sessione_fresca(&mut self) -> Uscita<(Sessione, Utente)> {
        let (s, u) = self.richiedi_sessione()?;
        if s.created_at.is_none_or(|c| self.ora() - c >= UN_GIORNO_S * 1000) {
            return Err(errore(403, "SESSION_NOT_FRESH", "Session is not fresh"));
        }
        Ok((s, u))
    }

    /// L'indirizzo per la sessione e per il limite (getIP), "" se non si sa.
    pub fn ip(&self) -> String {
        super::limiti::ip(self.r.intestazione("x-forwarded-for").as_deref()).unwrap_or_default()
    }

    pub fn browser(&self) -> String {
        self.r.intestazione("user-agent").unwrap_or_default()
    }
}

// ---- Validazione del corpo (zod) ----------------------------------------------------------------------------------------
fn tipo_zod(v: Option<&Json>) -> &'static str {
    match v {
        None => "undefined",
        Some(Json::Null) => "null",
        Some(Json::Bool(_)) => "boolean",
        Some(Json::Num(n)) if n.is_nan() => "NaN",
        Some(Json::Num(_)) => "number",
        Some(Json::Str(_)) => "string",
        Some(Json::Arr(_)) => "array",
        Some(Json::Obj(_)) => "object",
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tipo {
    Testo,
    Booleano,
    Qualunque,
    Record,
}

/// Un campo dello schema: nome, tipo, se è obbligatorio.
pub struct Campo(pub &'static str, pub Tipo, pub bool);

/// Il corpo secondo lo schema: Err con il testo di better-call ("[body.email] Invalid input: expected string, received undefined").
pub fn valida(corpo: Option<&Json>, campi: &[Campo], corpo_facoltativo: bool) -> Uscita<Mappa> {
    let oggetto = match corpo {
        Some(Json::Obj(m)) => m.clone(),
        None if corpo_facoltativo => return Ok(Mappa::new()),
        altro => {
            return Err(errore(400, "VALIDATION_ERROR", &format!("[body] Invalid input: expected object, received {}", tipo_zod(altro))));
        }
    };
    let mut problemi = Vec::new();
    for Campo(n, tipo, obbligatorio) in campi {
        let v = oggetto.get(n);
        let atteso = match tipo {
            Tipo::Testo => "string",
            Tipo::Booleano => "boolean",
            Tipo::Record => "record",
            Tipo::Qualunque => continue,
        };
        let buono = match (v, tipo) {
            (None, _) => !obbligatorio,
            (Some(Json::Str(_)), Tipo::Testo) | (Some(Json::Bool(_)), Tipo::Booleano) | (Some(Json::Obj(_)), Tipo::Record) => true,
            _ => false,
        };
        if !buono {
            problemi.push(format!("[body.{n}] Invalid input: expected {atteso}, received {}", tipo_zod(v)));
        }
    }
    if !problemi.is_empty() {
        return Err(errore(400, "VALIDATION_ERROR", &problemi.join("; ")));
    }
    Ok(oggetto)
}

// ---- Origine e CSRF -----------------------------------------------------------------------------------------------------
fn origine_di(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let o = u.origin();
    o.is_tuple().then(|| o.ascii_serialization())
}

fn origini_fidate() -> Vec<String> {
    let mut v = Vec::new();
    if let Some(o) = origine_di(&config::app_url()) {
        v.push(o);
    }
    v
}

/// isSafeRelativeURL di Better Auth.
fn relativo_sicuro(v: &str) -> bool {
    if !v.starts_with('/') || v.starts_with("//") || v.contains('\\') || v.chars().any(|c| matches!(c as u32, 0..=0x1f | 0x7f..=0x9f)) {
        return false;
    }
    let percorso = &v[..v.find(['?', '#']).unwrap_or(v.len())].to_ascii_lowercase();
    if percorso.contains("%2f") || percorso.contains("%5c") {
        return false;
    }
    url::Url::parse("https://better-auth.invalid").and_then(|b| b.join(v)).is_ok_and(|u| u.origin().ascii_serialization() == "https://better-auth.invalid")
}

/// matchesOriginPattern per gli indirizzi http(s) e i percorsi relativi.
fn fidato(url: &str, relativi: bool) -> bool {
    if url.starts_with('/') {
        return relativi && relativo_sicuro(url);
    }
    origine_di(url).is_some_and(|o| origini_fidate().contains(&o))
}

/// validateOrigin: con i cookie (o se `forza`) l'origine (o il referer) deve essere quella del sito.
fn controlla_origine(ctx: &Ctx, forza: bool) -> Uscita<()> {
    let origin = ctx.r.intestazione("origin");
    let testo = origin.clone().filter(|o| !o.is_empty()).or_else(|| ctx.r.intestazione("referer")).unwrap_or_default();
    if !(forza || ctx.r.intestazione("cookie").is_some()) {
        return Ok(());
    }
    let dedotta = (origin.as_deref() == Some("null") && ctx.r.intestazione("sec-fetch-site").as_deref() == Some("same-origin")).then(|| ctx.r.url.origin().ascii_serialization());
    let da_controllare = dedotta.unwrap_or(testo);
    if da_controllare.is_empty() || da_controllare == "null" {
        return Err(errore(403, "MISSING_OR_NULL_ORIGIN", "Missing or null Origin"));
    }
    if !fidato(&da_controllare, false) {
        eprintln!("auth: Invalid origin: {da_controllare}");
        return Err(errore(403, "INVALID_ORIGIN", "Invalid origin"));
    }
    Ok(())
}

/// originCheckMiddleware: le richieste che cambiano qualcosa, e gli indirizzi di ritorno nel corpo.
fn controllo_origine(ctx: &Ctx) -> Uscita<()> {
    if matches!(ctx.r.metodo, "GET" | "OPTIONS" | "HEAD") {
        return Ok(());
    }
    controlla_origine(ctx, false)?;
    let corpo = ctx.corpo.as_ref();
    let voci = [("callbackURL", "INVALID_CALLBACK_URL", "Invalid callbackURL"), ("redirectTo", "INVALID_REDIRECT_URL", "Invalid redirectURL"), ("errorCallbackURL", "INVALID_ERROR_CALLBACK_URL", "Invalid errorCallbackURL"), ("newUserCallbackURL", "INVALID_NEW_USER_CALLBACK_URL", "Invalid newUserCallbackURL")];
    for (i, (campo, codice, messaggio)) in voci.iter().enumerate() {
        let v = corpo.and_then(|c| c.get(campo)).or_else(|| if i == 0 { ctx.query.get("callbackURL") } else { None });
        let Some(v) = v.filter(|v| crate::py::py_bool(Some(v))) else { continue };
        let Some(t) = v.as_str() else { return Err(errore_messaggio(400, &format!("Invalid {}: expected a string", ["callbackURL", "redirectURL", "errorCallbackURL", "newUserCallbackURL"][i]))) };
        if !fidato(t, true) {
            return Err(errore(403, codice, messaggio));
        }
    }
    Ok(())
}

/// formCsrfMiddleware dell'accesso con la password: con i cookie l'origine; senza, una navigazione da un altro sito è rifiutata e un
/// fetch deve dire da dove viene.
fn controllo_form(ctx: &Ctx) -> Uscita<()> {
    if ctx.r.intestazione("cookie").is_some() {
        return controlla_origine(ctx, false);
    }
    let (site, mode, dest) = (ctx.r.intestazione("sec-fetch-site"), ctx.r.intestazione("sec-fetch-mode"), ctx.r.intestazione("sec-fetch-dest"));
    let pieno = |v: &Option<String>| v.as_deref().is_some_and(|s| !crate::py::js_trim(s).is_empty());
    if pieno(&site) || pieno(&mode) || pieno(&dest) {
        if site.as_deref() == Some("cross-site") && mode.as_deref() == Some("navigate") {
            eprintln!("auth: Blocked cross-site navigation login attempt (CSRF protection)");
            return Err(errore(403, "CROSS_SITE_NAVIGATION_LOGIN_BLOCKED", "Cross-site navigation login blocked. This request appears to be a CSRF attack."));
        }
        return controlla_origine(ctx, true);
    }
    if ctx.r.intestazione("origin").is_some() || ctx.r.intestazione("referer").is_some() {
        return controlla_origine(ctx, true);
    }
    Ok(())
}

// ---- Il corpo ---------------------------------------------------------------------------------------------------------------
/// getBody: JSON (o il modulo, dove è ammesso), con il 415 di better-call per un Content-Type diverso.
fn leggi_corpo(r: &Richiesta, anche_modulo: bool) -> Uscita<Option<Json>> {
    let tipo = r.intestazione("content-type").unwrap_or_default();
    let minuscolo = tipo.to_lowercase();
    if r.corpo.is_empty() && tipo.is_empty() {
        return Ok(None);
    }
    let ammessi: &[&str] = if anche_modulo { &["application/x-www-form-urlencoded", "application/json"] } else { &["application/json"] };
    let base = minuscolo.split(';').next().unwrap_or("").trim().to_string();
    if !ammessi.iter().any(|a| base == *a || base.contains(a)) {
        let elenco = ammessi.join(", ");
        if minuscolo.is_empty() {
            return Err(errore(415, "UNSUPPORTED_MEDIA_TYPE", &format!("Content-Type is required. Allowed types: {elenco}")));
        }
        return Err(errore(415, "UNSUPPORTED_MEDIA_TYPE", &format!("Content-Type \"{tipo}\" is not allowed. Allowed types: {elenco}")));
    }
    let json = regex::Regex::new(r"(?i)^application/([a-z0-9.+-]*\+)?json").expect("fissa");
    if json.is_match(&minuscolo) {
        let testo = String::from_utf8_lossy(r.corpo);
        let testo = testo.strip_prefix('\u{feff}').unwrap_or(&testo);
        return Json::analizza(testo).map(Some).map_err(|_| errore(400, "BAD_REQUEST", "Invalid JSON in request body"));
    }
    if minuscolo.contains("application/x-www-form-urlencoded") {
        let mut m = Mappa::new();
        for (k, v) in form_urlencoded::parse(r.corpo) {
            m.insert(k.into_owned(), v.into_owned());
        }
        return Ok(Some(Json::Obj(m)));
    }
    Ok(Some(Json::Str(String::from_utf8_lossy(r.corpo).into_owned())))
}

// ---- Le strade ----------------------------------------------------------------------------------------------------------
fn risposta_json(stato: u16, intestazioni: Vec<(String, String)>, corpo: Option<Json>) -> Risposta {
    let mut h = intestazioni;
    let corpo = corpo.map(|c| c.in_js());
    if corpo.is_some() || stato != 404 {
        h.push(("content-type".to_string(), "application/json".to_string()));
    }
    Risposta { stato, intestazioni: h, corpo }
}

/// Le strade che ci sono in questa modalità del secondo fattore.
fn esiste(metodo: &str, p: &str, modo: Modo2fa) -> bool {
    match (metodo, p) {
        ("GET", "/get-session" | "/ok") | ("POST", "/get-session" | "/sign-in/email" | "/sign-out") => true,
        ("POST", "/two-factor/enable" | "/two-factor/disable" | "/two-factor/generate-backup-codes" | "/two-factor/verify-totp" | "/two-factor/verify-backup-code") => modo != Modo2fa::Off,
        ("GET", "/passkey/generate-register-options" | "/passkey/generate-authenticate-options" | "/passkey/list-user-passkeys")
        | ("POST", "/passkey/verify-registration" | "/passkey/verify-authentication" | "/passkey/delete-passkey" | "/passkey/update-passkey") => modo == Modo2fa::Passkey,
        _ => false,
    }
}

/// Una richiesta a /api/auth/*.
pub async fn gestisci(r: &Richiesta<'_>, modo: Modo2fa) -> Risposta {
    let ora = crate::orologio::ora_vera();
    if let Some(attesa) = super::limiti::consuma(r.url.path(), r.intestazione("x-forwarded-for").as_deref(), modo, ora) {
        return Risposta {
            stato: 429,
            intestazioni: vec![("content-type".to_string(), "application/json".to_string()), ("x-retry-after".to_string(), attesa.to_string())],
            corpo: Some(r#"{"message":"Too many requests. Please try again later."}"#.to_string()),
        };
    }
    let pathname = r.url.path();
    let Some(p) = pathname.strip_prefix("/api/auth").filter(|p| p.starts_with('/') && p.len() > 1 && !p.contains("//")) else {
        return Risposta { stato: 404, intestazioni: vec![], corpo: None };
    };
    if !esiste(r.metodo, p, modo) {
        return Risposta { stato: 404, intestazioni: vec![], corpo: None };
    }
    let mut ctx = Ctx::nuovo(r, modo);
    let esito = esegui(&mut ctx, p).await;
    let mut uscita = std::mem::take(&mut ctx.uscita);
    match esito {
        Ok((stato, corpo)) => risposta_json(stato, uscita, Some(corpo)),
        Err(e) => {
            if e.stato == 500 && e.corpo.is_none() {
                // un errore che non è un APIError: Better Auth risponde 500 senza corpo
                return Risposta { stato: 500, intestazioni: vec![], corpo: None };
            }
            let corpo = e.corpo;
            if corpo.is_none() {
                uscita.push(("content-type".to_string(), "application/json".to_string()));
                return Risposta { stato: e.stato, intestazioni: uscita, corpo: None };
            }
            risposta_json(e.stato, uscita, corpo)
        }
    }
}

async fn esegui(ctx: &mut Ctx<'_>, p: &str) -> Uscita<(u16, Json)> {
    ctx.corpo = if ctx.r.metodo == "GET" { None } else { leggi_corpo(ctx.r, p == "/sign-in/email")? };
    controllo_origine(ctx)?;
    match p {
        "/get-session" => get_session(ctx).map(|c| (200, c)),
        "/ok" => Ok((200, ogg! { "ok": true })),
        "/sign-in/email" => accesso(ctx).await,
        "/sign-out" => uscita(ctx).map(|c| (200, c)),
        x if x.starts_with("/two-factor/") => super::due_fattori::gestisci(ctx, x).await,
        x => super::passkey::gestisci(ctx, x),
    }
}

fn get_session(ctx: &mut Ctx) -> Uscita<Json> {
    ctx.intestazione_uscita("cache-control", "no-store");
    ctx.intestazione_uscita("pragma", "no-cache");
    if ctx.r.metodo == "POST" {
        return Err(errore(405, "METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED", "POST method requires deferSessionRefresh to be enabled in session config"));
    }
    // z.coerce.boolean(): qualunque testo non vuoto vale vero
    let senza_rinnovo = ctx.query.get("disableRefresh").and_then(Json::as_str).is_some_and(|s| !s.is_empty());
    Ok(match ctx.leggi_sessione(senza_rinnovo)? {
        Some((s, u)) => ogg! { "session": s.json(), "user": u.json() },
        None => Json::Null,
    })
}

/// Il testo del blocco per account, come il gancio di auth.ts.
fn testo_blocco(secondi: i64) -> String {
    format!("Troppi tentativi falliti. Riprova tra {} minuti.", ((secondi as f64 / 60.0).ceil() as i64).max(1))
}

/// La forma di un indirizzo di posta per zod (z.email()).
fn email_valida(e: &str) -> bool {
    let Some(chiocciola) = e.rfind('@') else { return false };
    let (locale, dominio) = (&e[..chiocciola], &e[chiocciola + 1..]);
    if locale.is_empty() || locale.starts_with('.') || e.contains("..") {
        return false;
    }
    let ultimo = locale.chars().last().unwrap_or('.');
    if !locale.chars().all(|c| c.is_ascii_alphanumeric() || "_'+-.".contains(c)) || !(ultimo.is_ascii_alphanumeric() || "_+-".contains(ultimo)) {
        return false;
    }
    let etichette: Vec<&str> = dominio.split('.').collect();
    if etichette.len() < 2 {
        return false;
    }
    let (ultima, prime) = etichette.split_last().unwrap();
    ultima.len() >= 2 && ultima.chars().all(|c| c.is_ascii_alphabetic()) && prime.iter().all(|l| l.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Calcola l'hash di una password fuori dal thread del server (32 MB e qualche decina di millisecondi).
pub async fn hash_password(password: String) -> String {
    tokio::task::spawn_blocking(move || super::cripto::hash_password(&password)).await.unwrap_or_default()
}

pub async fn verifica_password(hash: String, password: String) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || super::cripto::verifica_password(&hash, &password)).await.unwrap_or_else(|e| Err(e.to_string()))
}

/// POST /sign-in/email, con il gancio del blocco per account prima e dopo, e quello del secondo fattore.
async fn accesso(ctx: &mut Ctx<'_>) -> Uscita<(u16, Json)> {
    let db = righe::db()?;
    let email_grezza = lockout::normalize_email(ctx.corpo.as_ref().and_then(|c| c.get("email")));
    let adesso_s = ctx.ora() as f64 / 1000.0;
    // prima: l'account bloccato si rifiuta prima di guardare la password
    if !email_grezza.is_empty() {
        let restano = lockout::locked_for(&db, &email_grezza, adesso_s)?;
        if restano > 0 {
            return Err(errore_messaggio(429, &testo_blocco(restano)));
        }
    }
    let esito = accesso_con_password(ctx).await;
    // dopo: il conto si azzera con la password giusta, se no si conta l'errore
    if !email_grezza.is_empty() {
        if ctx.nuova.is_some() {
            lockout::clear_failures(&db, &email_grezza)?;
        } else if lockout::locked_for(&db, &email_grezza, ctx.ora() as f64 / 1000.0)? == 0 {
            lockout::register_failure(&db, &email_grezza, ctx.ora() as f64 / 1000.0)?;
        }
    }
    let corpo = esito?;
    if ctx.modo != Modo2fa::Off {
        if let Some(r) = super::due_fattori::dopo_accesso(ctx)? {
            return Ok((200, r));
        }
    }
    Ok((200, corpo))
}

async fn accesso_con_password(ctx: &mut Ctx<'_>) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("email", Tipo::Testo, true), Campo("password", Tipo::Testo, true), Campo("callbackURL", Tipo::Testo, false), Campo("rememberMe", Tipo::Booleano, false)], false)?;
    controllo_form(ctx)?;
    let email = corpo.get("email").and_then(Json::as_str).unwrap_or("").to_string();
    let password = corpo.get("password").and_then(Json::as_str).unwrap_or("").to_string();
    if !email_valida(&email) {
        return Err(errore(400, "INVALID_EMAIL", "Invalid email"));
    }
    if crate::py::lunghezza_js(&password) > PASSWORD_MAX {
        return Err(errore(400, "PASSWORD_TOO_LONG", "Password too long"));
    }
    let utente = righe::utente_per_email(&email)?;
    let hash = match &utente {
        Some(u) => righe::password_di(&u.id)?,
        None => None,
    };
    let non_valido = || errore(401, "INVALID_EMAIL_OR_PASSWORD", "Invalid email or password");
    let (Some(u), Some(Some(hash))) = (utente, hash) else {
        // lo stesso tempo di una password sbagliata: chi prova non sa se l'utente esiste
        hash_password(password).await;
        return Err(non_valido());
    };
    if !verifica_password(hash, password).await.map_err(|e| Errore::altro(e))? {
        return Err(non_valido());
    }
    let non_ricordare = corpo.get("rememberMe") == Some(&Json::Bool(false));
    let s = righe::crea_sessione(&u.id, non_ricordare, &ctx.ip(), &ctx.browser(), ctx.ora())?;
    ctx.imposta_cookie_sessione(&s, &u, Some(non_ricordare));
    let ritorno = corpo.get("callbackURL").and_then(Json::as_str).map(str::to_string);
    let mut m = Mappa::new();
    m.insert("redirect", ritorno.is_some());
    m.insert("token", s.token.clone());
    if let Some(u) = &ritorno {
        ctx.intestazione_uscita("location", u);
        m.insert("url", u.clone());
    }
    m.insert("user", u.json());
    Ok(Json::Obj(m))
}

/// POST /sign-out: la sessione si toglie dal database e i cookie scadono.
fn uscita(ctx: &mut Ctx) -> Uscita<Json> {
    valida(ctx.corpo.as_ref(), &[Campo("callbackURL", Tipo::Testo, false), Campo("disableRedirect", Tipo::Booleano, false), Campo("state", Tipo::Testo, false)], true)?;
    if let Some(token) = ctx.firmato(cookie::SESSIONE) {
        if let Err(e) = righe::elimina_sessione(&token) {
            eprintln!("auth: Failed to delete session from database {e}");
        }
    }
    ctx.elimina_cookie_sessione(false);
    Ok(ogg! { "success": true })
}

/// L'utente della sessione di una richiesta, verificata sul database (non basta la presenza del cookie): come auth.api.getSession, che
/// rinnova la sessione nel database ma non manda cookie.
pub fn sessione_corrente(intestazioni: &[(String, String)], url: &url::Url) -> Esito<Option<(Sessione, Utente)>> {
    let r = Richiesta { metodo: "GET", url, intestazioni, corpo: &[] };
    let mut ctx = Ctx::nuovo(&r, config::auth2fa());
    match ctx.leggi_sessione(false) {
        Ok(s) => Ok(s),
        Err(_) => Ok(None),
    }
}
