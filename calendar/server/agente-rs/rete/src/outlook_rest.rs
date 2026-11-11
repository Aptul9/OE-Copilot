//! Il nucleo Microsoft che l'agente usa in più punti: gli indirizzi di Microsoft, i claims dei token, il controllo dei token del client web
//! di Outlook e il trasporto REST (GET con il bearer della casella; un 401 rinnova il token una volta e riprova). Il trasporto è lo stesso
//! per i calendari (CalendarioDiretto, anche Google) e per il gateway di posta e calendari (comunicazione.ts).

use crate::js::{buffer_base64, js_trim, numero_di, primi_utf16, stringa_di, vero};
use crate::trasporto::{ErroreRete, Intestazioni, Richiesta, Risposta, Trasporto};
use oe_nucleo::json::{Json, Mappa};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

pub const MS_LOGIN: &str = "https://login.microsoftonline.com";
pub const OUTLOOK_REST: &str = "https://outlook.office.com/api/v2.0";
const AUD_OUTLOOK: [&str; 2] = ["https://outlook.office.com", "https://outlook.office365.com"];

/// I claims di un JWT, senza verificarne la firma: servono a leggere casella, risorsa, scadenza e permessi. Vuoti se non è un JWT.
pub fn claims_jwt(jwt: &str) -> Mappa {
    let mut parti = jwt.split('.');
    let Some(p) = parti.nth(1).filter(|p| !p.is_empty()) else { return Mappa::new() };
    let testo = String::from_utf8_lossy(&buffer_base64(p)).into_owned();
    match Json::analizza(&testo) {
        Ok(Json::Obj(m)) => m,
        _ => Mappa::new(),
    }
}

/// La casella di un token Microsoft, in minuscolo; "" se il token non la dice.
pub fn upn_di(claims: &Mappa) -> String {
    for k in ["upn", "preferred_username", "unique_name"] {
        if let Some(v) = claims.get(k).filter(|v| vero(v)) {
            return stringa_di(v).to_lowercase();
        }
    }
    String::new()
}

/// `^(Calendars|Mail)\.(Read|ReadWrite)(\.Shared)?$`
pub fn scope_di(scope: &str, famiglia: &str) -> bool {
    let Some(resto) = scope.strip_prefix(famiglia).and_then(|r| r.strip_prefix('.')) else { return false };
    matches!(resto, "Read" | "ReadWrite" | "Read.Shared" | "ReadWrite.Shared")
}

/// Un token del client web di Outlook si accetta solo se è della stessa casella, per la risorsa Outlook, non scaduto e con il permesso sui
/// calendari: vale per l'importazione della sessione, per ogni rinnovo col refresh_token del client web e per ogni nuovo accesso dalla
/// sessione del browser. Restituisce il motivo del rifiuto, o `None` se il token va bene. `required` contiene "mail" e/o "calendar".
pub fn motivo_token_web_non_valido(access_token: &Json, casella: &str, adesso_s: f64, required: &[&str]) -> Option<&'static str> {
    let Some(token) = access_token.as_str().filter(|t| !t.is_empty()) else { return Some("Risposta token Microsoft senza access_token") };
    let claims = claims_jwt(token);
    if upn_di(&claims) != js_trim(casella).to_lowercase() {
        return Some("Microsoft ha restituito un'altra casella");
    }
    if !claims.get("aud").and_then(Json::as_str).is_some_and(|a| AUD_OUTLOOK.contains(&a)) {
        return Some("Microsoft ha restituito un token per un'altra risorsa");
    }
    let exp = claims.get("exp").map_or(f64::NAN, numero_di);
    if !exp.is_finite() || exp <= adesso_s {
        return Some("Microsoft ha restituito un token già scaduto");
    }
    let scp: Vec<&str> = claims.get("scp").and_then(Json::as_str).map(|s| s.split(' ').collect()).unwrap_or_default();
    if required.contains(&"calendar") && !scp.iter().any(|s| scope_di(s, "Calendars")) {
        return Some("Il token non ha il permesso sui calendari");
    }
    if required.contains(&"mail") && !scp.iter().any(|s| scope_di(s, "Mail")) {
        return Some("Il token non ha il permesso sulla posta");
    }
    None
}

pub type ErroreDyn = Box<dyn std::error::Error>;
pub type FutureToken<'a> = Pin<Box<dyn Future<Output = Result<String, ErroreDyn>> + 'a>>;

/// Chi dà i token di una casella e li rinnova (GestoreToken, nell'agente).
pub trait FonteToken {
    fn ottieni_access_token<'a>(&'a self, email: &'a str, forza: bool) -> FutureToken<'a>;
}

/// Un errore di `getRest`: quello della fonte dei token, quello della rete o un `Error` con il suo testo.
#[derive(Debug)]
pub enum ErroreRest {
    Token(ErroreDyn),
    Rete(ErroreRete),
    Altro(String),
}

impl fmt::Display for ErroreRest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroreRest::Token(e) => e.fmt(f),
            ErroreRest::Rete(e) => e.fmt(f),
            ErroreRest::Altro(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ErroreRest {}

impl From<ErroreRete> for ErroreRest {
    fn from(e: ErroreRete) -> Self {
        ErroreRest::Rete(e)
    }
}

#[derive(Clone, Debug)]
pub struct OpzioniRest {
    pub headers: Intestazioni,
    pub timeout: Duration,
    /// Oltre questi byte il corpo non si legge: `ErroreRete::RispostaGrande`. Senza, si legge tutto.
    pub max_byte: Option<usize>,
    /// Senza, i redirect si seguono (come fetch).
    pub segui_redirect: bool,
}

impl Default for OpzioniRest {
    fn default() -> Self {
        OpzioniRest { headers: Intestazioni::new(), timeout: Duration::from_millis(20_000), max_byte: None, segui_redirect: true }
    }
}

#[derive(Clone, Debug)]
pub struct RispostaRest {
    pub status: u16,
    pub status_text: String,
    pub headers: Intestazioni,
    pub testo: String,
}

/// `leggiTesto`: il corpo come testo UTF-8, con `ErroreRispostaGrande` oltre `max_byte` (anche solo dichiarati da content-length).
pub fn leggi_testo(resp: &Risposta, max_byte: Option<usize>) -> Result<String, ErroreRete> {
    if let Some(max) = max_byte {
        let dichiarata = resp.intestazioni.primo("content-length").map_or(f64::NAN, |v| numero_di(&Json::Str(v.to_string())));
        if dichiarata > max as f64 || resp.corpo.len() > max {
            return Err(ErroreRete::RispostaGrande(max));
        }
    }
    resp.testo()
}

/// GET con il token della casella. Un 401 forza il rinnovo del token e riprova una volta; il secondo esito, qualunque sia, torna a chi
/// chiama. Il corpo di una risposta d'errore che non si riesce a leggere diventa "".
pub async fn get_rest(tokens: &dyn FonteToken, trasporto: &Rc<dyn Trasporto>, account: &str, url: &str, o: &OpzioniRest) -> Result<RispostaRest, ErroreRest> {
    for forza in [false, true] {
        let token = tokens.ottieni_access_token(account, forza).await.map_err(ErroreRest::Token)?;
        let mut r = Richiesta::get(url).scadenza(o.timeout).max_byte(o.max_byte);
        r.intestazioni = o.headers.clone();
        r.intestazioni.imposta("Authorization", format!("Bearer {token}"));
        r.intestazioni.imposta("Accept", "application/json");
        r.segui_redirect = o.segui_redirect;
        let resp = trasporto.invia(r).await?;
        if resp.stato == 401 && !forza {
            continue;
        }
        let testo = if resp.ok() { leggi_testo(&resp, o.max_byte)? } else { leggi_testo(&resp, o.max_byte).unwrap_or_default() };
        return Ok(RispostaRest { status: resp.stato, status_text: resp.testo_stato().to_string(), headers: resp.intestazioni, testo });
    }
    Err(ErroreRest::Altro("irraggiungibile".into()))
}

/// GET che vuole un JSON: una risposta non 2xx è un errore "HTTP <codice>: <inizio del corpo>".
pub async fn json_rest(tokens: &dyn FonteToken, trasporto: &Rc<dyn Trasporto>, account: &str, url: &str, o: &OpzioniRest) -> Result<Json, ErroreRest> {
    let r = get_rest(tokens, trasporto, account, url, o).await?;
    if !(200..=299).contains(&r.status) {
        let inizio = primi_utf16(&r.testo, 160);
        return Err(ErroreRest::Altro(format!("HTTP {}: {}", r.status, if inizio.is_empty() { r.status_text } else { inizio })));
    }
    Json::analizza(&r.testo).map_err(|e| ErroreRest::Altro(e.messaggio))
}

#[cfg(test)]
mod prove {
    use super::*;
    use crate::trasporto::prova::{TrasportoFinto, json};
    use std::cell::RefCell;

    fn jwt(c: &str) -> String {
        use base64::Engine;
        format!("x.{}.x", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(c))
    }

    struct Fonte(RefCell<Vec<bool>>);
    impl FonteToken for Fonte {
        fn ottieni_access_token<'a>(&'a self, _e: &'a str, forza: bool) -> FutureToken<'a> {
            self.0.borrow_mut().push(forza);
            Box::pin(async move { Ok(if forza { "nuovo" } else { "vecchio" }.to_string()) })
        }
    }

    #[test]
    fn claims_e_motivi() {
        let t = jwt(r#"{"upn":"Mario@X.it","aud":"https://outlook.office.com","exp":2000,"scp":"Mail.Read Calendars.ReadWrite.Shared"}"#);
        assert_eq!(upn_di(&claims_jwt(&t)), "mario@x.it");
        assert!(claims_jwt("a").is_empty() && claims_jwt("a.[1].c").is_empty());
        let ok = Json::Str(t.clone());
        assert_eq!(motivo_token_web_non_valido(&ok, " mario@x.IT ", 1000.0, &["calendar", "mail"]), None);
        assert_eq!(motivo_token_web_non_valido(&ok, "altro@x.it", 1000.0, &["mail"]), Some("Microsoft ha restituito un'altra casella"));
        assert_eq!(motivo_token_web_non_valido(&ok, "mario@x.it", 2000.0, &["mail"]), Some("Microsoft ha restituito un token già scaduto"));
        assert_eq!(motivo_token_web_non_valido(&Json::Null, "a", 0.0, &[]), Some("Risposta token Microsoft senza access_token"));
        let solo_mail = Json::Str(jwt(r#"{"upn":"a","aud":"https://outlook.office365.com","exp":9,"scp":"Mail.ReadWrite"}"#));
        assert_eq!(motivo_token_web_non_valido(&solo_mail, "a", 0.0, &["calendar"]), Some("Il token non ha il permesso sui calendari"));
        assert_eq!(motivo_token_web_non_valido(&solo_mail, "a", 0.0, &["mail"]), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn un_401_rinnova_e_riprova_una_volta() {
        let finto = TrasportoFinto::con_copione([Ok(json(401, "{}")), Ok(json(200, r#"{"a":1}"#))]);
        let t: Rc<dyn Trasporto> = finto.clone();
        let fonte = Fonte(RefCell::new(vec![]));
        let v = json_rest(&fonte, &t, "a@b", "https://outlook.office.com/api/v2.0/me", &OpzioniRest::default()).await.unwrap();
        assert_eq!(v.in_js(), r#"{"a":1}"#);
        assert_eq!(*fonte.0.borrow(), vec![false, true]);
        let r = finto.richieste();
        assert_eq!(r[1].intestazioni.primo("authorization"), Some("Bearer nuovo"));
        assert_eq!(r[1].intestazioni.primo("accept"), Some("application/json"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn errori_http_e_corpo_grande() {
        let t: Rc<dyn Trasporto> = TrasportoFinto::con_copione([Ok(json(500, "")), Ok(json(200, "0123456789"))]);
        let fonte = Fonte(RefCell::new(vec![]));
        let e = json_rest(&fonte, &t, "a", "https://x/", &OpzioniRest::default()).await.unwrap_err();
        assert_eq!(e.to_string(), "HTTP 500: Internal Server Error");
        let o = OpzioniRest { max_byte: Some(5), ..Default::default() };
        let e = json_rest(&fonte, &t, "a", "https://x/", &o).await.unwrap_err();
        assert_eq!(e.to_string(), "risposta oltre 5 byte");
    }
}
