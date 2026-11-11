//! Le caselle condivise con TeamsRelay: per quali risorse un account può dare token (Outlook per posta e calendario, i servizi di Teams per
//! la chat), con quale client si chiedono, e il rinnovo con il refresh token di famiglia.

use crate::errori::{ErroreAccesso, EsitoAccesso};
use crate::outlook_rest::claims_jwt;
use crate::sessione_web::{codifica_componente, json_o_vuoto, modulo};
use crate::trasporto::{Richiesta, Trasporto};
use oe_nucleo::json::{Json, Mappa, numero_js};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct SharedAccount {
    pub tenant_id: String,
    pub object_id: String,
    pub capabilities: Vec<String>,
    pub origin: String,
}

impl SharedAccount {
    /// Dal JSON di tokens.json; i campi che mancano restano vuoti (come una casella che `allowed` poi rifiuta).
    pub fn da_json(v: &Json) -> Option<SharedAccount> {
        let m = v.as_obj()?;
        let s = |k: &str| m.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
        Some(SharedAccount {
            tenant_id: s("tenantId"),
            object_id: s("objectId"),
            capabilities: m.get("capabilities").and_then(Json::as_arr).map(|v| v.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            origin: s("origin"),
        })
    }

    pub fn in_json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("tenantId", self.tenant_id.as_str());
        m.insert("objectId", self.object_id.as_str());
        m.insert("capabilities", self.capabilities.clone());
        m.insert("origin", self.origin.as_str());
        Json::Obj(m)
    }

    pub fn ha(&self, c: &str) -> bool {
        self.capabilities.iter().any(|x| x == c)
    }
}

fn risorse(r: &str) -> Option<&'static [&'static str]> {
    Some(match r {
        "outlook" => &["https://outlook.office.com", "https://outlook.office365.com", "00000002-0000-0ff1-ce00-000000000000"],
        "csa" => &["https://chatsvcagg.teams.microsoft.com"],
        "ic3" => &["https://ic3.teams.office.com"],
        "spaces" => &["https://api.spaces.skype.com"],
        "presence" => &["https://presence.teams.microsoft.com"],
        _ => return None,
    })
}

/// Le audience che una risorsa ammette per l'account, o l'errore se l'identità o la capacità non tornano.
pub fn allowed(s: &SharedAccount, r: &str) -> EsitoAccesso<&'static [&'static str]> {
    if s.tenant_id.is_empty() || s.object_id.is_empty() || !["mobile", "outlook-spa", "teams-spa"].contains(&s.origin.as_str()) {
        return Err(ErroreAccesso::altro("invalid shared identity"));
    }
    let aud = risorse(r);
    let capace = if r == "outlook" { s.ha("mail") || s.ha("calendar") } else { s.ha("chat") };
    match aud {
        Some(a) if capace => Ok(a),
        _ => Err(ErroreAccesso::altro("resource or capability denied")),
    }
}

fn scope_valido(scope: &str, famiglia: &str) -> bool {
    let s = scope.to_ascii_lowercase();
    let Some(resto) = s.strip_prefix(&famiglia.to_ascii_lowercase()).and_then(|r| r.strip_prefix('.')) else { return false };
    matches!(resto, "read" | "readwrite" | "read.shared" | "readwrite.shared")
}

pub fn has_shared_permissions(s: &SharedAccount, r: &str, token: &str) -> bool {
    if r != "outlook" {
        return true;
    }
    let scp = match claims_jwt(token).get("scp") {
        Some(v) if !matches!(v, Json::Null | Json::Bool(false)) && !(matches!(v, Json::Str(x) if x.is_empty())) && v.as_f64() != Some(0.0) => testo(v),
        _ => String::new(),
    };
    let scopes: Vec<&str> = scp.split(char::is_whitespace).collect();
    s.capabilities.iter().all(|cap| cap == "chat" || scopes.iter().any(|sc| if cap == "mail" { scope_valido(sc, "Mail") } else { scope_valido(sc, "Calendars") }))
}

fn testo(v: &Json) -> String {
    match v {
        Json::Str(s) => s.clone(),
        Json::Num(n) => numero_js(*n),
        Json::Bool(b) => b.to_string(),
        Json::Null => "null".to_string(),
        altro => altro.in_js(),
    }
}

/// Teams desktop and mobile: a member of the Microsoft family of clients (FOCI), like Outlook Mobile. Teams calling answers 410
/// ApiRestricted to a token of Outlook Mobile and 200 to one of this app, and the chat list of Teams is complete only for it.
pub const TEAMS_APP_CLIENT_ID: &str = "1fec8e78-bce4-4aaf-ab1b-5451cc387264";

/// The client a resource of a shared account is asked as: the Teams resources of an Outlook Mobile sign-in (a family refresh token) as
/// Teams itself; anything else as the client of the sign-in.
pub fn client_for<'a>(s: &SharedAccount, r: &str, stored: &'a str) -> &'a str {
    if s.origin == "mobile" && r != "outlook" { TEAMS_APP_CLIENT_ID } else { stored }
}

pub struct Rinnovo {
    pub token: String,
    pub expires_at: f64,
    pub refresh: String,
}

/// `String(c.aud).replace(/\/$/, "")`
pub fn audience(claims: &Mappa) -> String {
    let a = testo(claims.get("aud").unwrap_or(&Json::Null));
    a.strip_suffix('/').map(str::to_string).unwrap_or(a)
}

fn numero(v: Option<&Json>) -> f64 {
    match v {
        Some(Json::Num(n)) => *n,
        Some(Json::Str(s)) => {
            let t = s.trim();
            if t.is_empty() { 0.0 } else { t.parse().unwrap_or(f64::NAN) }
        }
        Some(Json::Bool(b)) => f64::from(u8::from(*b)),
        Some(Json::Null) => 0.0,
        _ => f64::NAN,
    }
}

pub fn exp_di(claims: &Mappa) -> f64 {
    numero(claims.get("exp"))
}

pub async fn renew_shared(s: &SharedAccount, r: &str, refresh: &str, client: &str, trasporto: &Rc<dyn Trasporto>, now: i64) -> EsitoAccesso<Rinnovo> {
    let aud = allowed(s, r)?;
    let mut richiesta = Richiesta::post(
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", codifica_componente(&s.tenant_id)),
        modulo(&[("client_id", client), ("grant_type", "refresh_token"), ("refresh_token", refresh), ("scope", &format!("{}/.default offline_access openid profile", aud[0]))]),
    )
    .intestazione("Content-Type", "application/x-www-form-urlencoded");
    if s.origin != "mobile" {
        richiesta = richiesta.intestazione("Origin", if s.origin == "teams-spa" { "https://teams.microsoft.com" } else { "https://outlook.cloud.microsoft" });
    }
    let risposta = trasporto.invia(richiesta.senza_redirect().scadenza(Duration::from_millis(15_000))).await?;
    // redirect: "error" di fetch: un rinvio è un errore di rete
    if (300..400).contains(&risposta.stato) && risposta.intestazioni.contiene("location") {
        return Err(ErroreAccesso::altro("fetch failed"));
    }
    let testo_corpo = risposta.testo()?;
    let j = match Json::analizza(&testo_corpo) {
        Ok(Json::Obj(m)) => m,
        Ok(_) => json_o_vuoto(&risposta),
        Err(e) => return Err(ErroreAccesso::altro(e.messaggio)),
    };
    let errore = j.get("error").and_then(Json::as_str).unwrap_or_default();
    if !risposta.ok() && (errore == "invalid_grant" || errore == "interaction_required") {
        return Err(ErroreAccesso::SharedRefreshExpired("shared refresh expired".to_string()));
    }
    if !risposta.ok() && (400..500).contains(&risposta.stato) && risposta.stato != 429 && errore != "temporarily_unavailable" && errore != "server_error" {
        return Err(ErroreAccesso::SharedRifiutato("Microsoft refused this client for the requested service".to_string()));
    }
    let Some(Json::Str(token)) = j.get("access_token").filter(|_| risposta.ok()) else { return Err(ErroreAccesso::altro("shared renewal refused")) };
    let c = claims_jwt(token);
    let exp = exp_di(&c);
    let stesso = |k: &str, v: &str| c.get(k).and_then(Json::as_str) == Some(v);
    if !has_shared_permissions(s, r, token) || !stesso("tid", &s.tenant_id) || !stesso("oid", &s.object_id) || !aud.contains(&audience(&c).as_str()) || !exp.is_finite() || exp <= (now + 120) as f64 {
        return Err(ErroreAccesso::altro("shared identity, audience or expiry mismatch"));
    }
    let nuovo = j.get("refresh_token").and_then(Json::as_str).filter(|r| !r.is_empty()).unwrap_or(refresh);
    Ok(Rinnovo { token: token.clone(), expires_at: exp, refresh: nuovo.to_string() })
}
