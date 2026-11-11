//! Accesso Microsoft dalla sessione del browser, senza browser: stessi cookie ESTS che il browser ha già, autorizzazione silenziosa
//! (prompt=none, PKCE) del client web di Outlook. Serve per le caselle dove il client di Outlook Mobile è respinto dall'accesso condizionale
//! (AADSTS53003) ma Outlook web no. Stesso schema di teamsrelay app/src/agent/light/session-auth.ts, con client, redirect e risorsa di
//! Outlook web.

use crate::errori::{ErroreAccesso, EsitoAccesso};
use crate::mcas_outlook::{ProxySession, authorize_proxy, proxy_cookie};
use crate::outlook_rest::{MS_LOGIN, leggi_testo};
use crate::trasporto::{Richiesta, Risposta, Trasporto};
use oe_nucleo::json::{Json, Mappa};
use regex::Regex;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;
use url::Url;

pub const OUTLOOK_WEB_CLIENT_ID: &str = "9199bf20-a13f-4107-85dc-02114787ef48";
pub const OUTLOOK_WEB_REDIRECT_URI: &str = "https://outlook.cloud.microsoft/mail/";
pub const OUTLOOK_WEB_SCOPES: &str = "https://outlook.office.com/.default offline_access openid profile";

/// Un cookie come lo scrive teamsrelay (name, value, domain, path, expires e quello che il browser aggiunge): l'oggetto resta com'è, così
/// tokens.json torna uguale.
pub type CookieSessione = Mappa;

pub fn campo<'a>(c: &'a CookieSessione, k: &str) -> &'a str {
    c.get(k).and_then(Json::as_str).unwrap_or_default()
}

pub fn scadenza(c: &CookieSessione) -> f64 {
    c.get("expires").and_then(Json::as_f64).unwrap_or(f64::NAN)
}

pub fn nuovo_cookie(name: &str, value: &str, domain: &str, path: &str, expires: f64) -> CookieSessione {
    let mut m = Mappa::new();
    m.insert("name", name);
    m.insert("value", value);
    m.insert("domain", domain);
    m.insert("path", path);
    m.insert("expires", expires);
    m
}

/// Il formato di teamsrelay session.json: si importa così com'è.
#[derive(Clone, Debug, PartialEq)]
pub struct SessioneMicrosoft {
    pub user_agent: String,
    pub login_hint: String,
    pub cookies: Vec<CookieSessione>,
    pub proxy: Option<ProxySession>,
}

impl SessioneMicrosoft {
    pub fn in_json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("userAgent", self.user_agent.as_str());
        m.insert("loginHint", self.login_hint.as_str());
        m.insert("cookies", Json::Arr(self.cookies.iter().cloned().map(Json::Obj).collect()));
        if let Some(p) = &self.proxy {
            m.insert("proxy", p.in_json());
        }
        Json::Obj(m)
    }
}

static NOME_COOKIE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[!#$%&'*+.^_`|~A-Za-z0-9_-]+$").expect("valida"));

pub fn nome_cookie_valido(n: &str) -> bool {
    NOME_COOKIE.is_match(n)
}

pub fn valore_cookie_valido(v: &str) -> bool {
    !v.chars().any(|c| (c as u32) <= 0x20 || c == '\x7f' || c == ';')
}

fn cookie_di_login(c: &CookieSessione) -> bool {
    let dominio = campo(c, "domain");
    dominio.strip_prefix('.').unwrap_or(dominio).eq_ignore_ascii_case("login.microsoftonline.com")
        && nome_cookie_valido(campo(c, "name"))
        && valore_cookie_valido(campo(c, "value"))
        && campo(c, "path").starts_with('/')
}

fn tenant_valido(t: &str) -> bool {
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// Un cookie con le quattro stringhe e la scadenza numerica.
pub fn forma_cookie(c: &Json) -> Option<&CookieSessione> {
    let m = c.as_obj()?;
    let stringhe = ["name", "value", "domain", "path"].iter().all(|k| matches!(m.get(k), Some(Json::Str(_))));
    (stringhe && scadenza(m).is_finite()).then_some(m)
}

/// Tiene solo i cookie di login.microsoftonline.com: gli altri non servono e non escono. None se la forma non torna.
pub fn leggi_sessione(dati: &Json) -> Option<SessioneMicrosoft> {
    let s = dati.as_obj()?;
    let user_agent = s.get("userAgent")?.as_str().filter(|u| !u.is_empty() && !u.contains(['\r', '\n']))?;
    let login_hint = s.get("loginHint")?.as_str()?;
    let cookies = s.get("cookies")?.as_arr()?.iter().filter_map(forma_cookie).filter(|c| cookie_di_login(c)).cloned().collect();
    let mut proxy = None;
    if let Some(p) = s.get("proxy").filter(|p| matches!(p, Json::Obj(_) | Json::Arr(_))) {
        let bootstrap = p.get("bootstrapUrl").and_then(Json::as_str);
        let lista = p.get("cookies").and_then(Json::as_arr);
        let (Some(b), Some(l)) = (bootstrap, lista) else { return None };
        if l.len() > 512 {
            return None;
        }
        proxy = Some(ProxySession { bootstrap_url: b.to_string(), cookies: l.iter().filter_map(|c| c.as_obj()).filter(|c| proxy_cookie(c)).cloned().collect() });
    }
    Some(SessioneMicrosoft { user_agent: user_agent.to_string(), login_hint: login_hint.to_string(), cookies, proxy })
}

const MESI: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// `Date.parse` di una data di Set-Cookie ("Wed, 21 Oct 2026 07:28:00 GMT", anche con i trattini), in millisecondi.
pub fn data_cookie(v: &str) -> Option<f64> {
    let parti: Vec<&str> = v.split(|c: char| c == ' ' || c == '-' || c == ',').filter(|p| !p.is_empty()).collect();
    let i = parti.iter().position(|p| p.bytes().all(|b| b.is_ascii_digit()))?;
    let giorno: i64 = parti.get(i)?.parse().ok()?;
    let mese = MESI.iter().position(|m| parti.get(i + 1).is_some_and(|p| p.to_ascii_lowercase().starts_with(m)))? as i64 + 1;
    let mut anno: i64 = parti.get(i + 2)?.parse().ok()?;
    if anno < 50 {
        anno += 2000;
    } else if anno < 100 {
        anno += 1900;
    }
    let ore: Vec<i64> = parti.get(i + 3)?.split(':').map(|x| x.parse().ok()).collect::<Option<Vec<_>>>()?;
    let (h, mi, s) = (*ore.first()?, *ore.get(1).unwrap_or(&0), *ore.get(2).unwrap_or(&0));
    if !(1..=31).contains(&giorno) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let giorni = oe_nucleo::tempo::giorni_da_civile(anno, mese, giorno);
    Some(((giorni * 86400 + h * 3600 + mi * 60 + s) * 1000) as f64)
}

fn dominio_nudo(d: &str) -> String {
    d.strip_prefix('.').unwrap_or(d).to_lowercase()
}

/// Aggiorna i cookie con i Set-Cookie di Microsoft (ESTS ruota), come fa il browser.
fn ruota(s: &mut SessioneMicrosoft, r: &Risposta, adesso_s: i64) {
    for raw in r.intestazioni.tutti("set-cookie") {
        let mut pezzi = raw.split(';');
        let coppia = pezzi.next().unwrap_or_default();
        let Some(uguale) = coppia.find('=').filter(|i| *i >= 1) else { continue };
        let (mut domain, mut path, mut expires) = ("login.microsoftonline.com".to_string(), "/".to_string(), -1.0);
        let mut max_age: Option<i64> = None;
        for a in pezzi {
            let (k, v) = match a.find('=') {
                Some(i) => (a[..i].trim().to_lowercase(), a[i + 1..].trim()),
                None => (a.trim().to_lowercase(), ""),
            };
            match k.as_str() {
                "domain" => domain = v.to_string(),
                "path" => path = v.to_string(),
                "expires" => {
                    if let Some(ms) = data_cookie(v) {
                        expires = ms / 1000.0;
                    }
                }
                "max-age" if !v.is_empty() && v.strip_prefix('-').unwrap_or(v).bytes().all(|b| b.is_ascii_digit()) && v != "-" => max_age = v.parse().ok(),
                _ => {}
            }
        }
        if let Some(m) = max_age {
            expires = (adesso_s + m) as f64;
        }
        let c = nuovo_cookie(coppia[..uguale].trim(), coppia[uguale + 1..].trim(), &domain, &path, expires);
        if !cookie_di_login(&c) {
            continue;
        }
        let dominio = dominio_nudo(&domain);
        s.cookies.retain(|v| !(campo(v, "name") == campo(&c, "name") && dominio_nudo(campo(v, "domain")) == dominio && campo(v, "path") == path));
        if expires == -1.0 || expires > adesso_s as f64 {
            s.cookies.push(c);
        }
    }
}

pub fn casuale_base64url(n: usize) -> String {
    use base64::Engine;
    let mut b = vec![0u8; n];
    let _ = getrandom::fill(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

pub fn sha256_base64url(testo: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(testo.as_bytes()))
}

pub fn modulo(campi: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new()).extend_pairs(campi).finish()
}

/// `encodeURIComponent`.
pub fn codifica_componente(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Il primo valore di un parametro di una query, come `URLSearchParams.get`.
pub fn parametro(u: &Url, nome: &str) -> Option<String> {
    u.query_pairs().find(|(k, _)| k == nome).map(|(_, v)| v.into_owned())
}

pub struct ClienteWeb<'a> {
    pub id: &'a str,
    pub redirect: &'a str,
}

/// La risposta JSON di un token endpoint come la legge `resp.json().catch(() => ({}))`.
pub fn json_o_vuoto(r: &Risposta) -> Mappa {
    match r.testo().ok().and_then(|t| Json::analizza(&t).ok()) {
        Some(Json::Obj(m)) => m,
        _ => Mappa::new(),
    }
}

/// Autorizzazione silenziosa con la sessione del browser, poi scambio del codice. Restituisce la risposta del token endpoint (access_token,
/// refresh_token, ...). La sessione viene aggiornata sul posto con i cookie ruotati: chi chiama la salva.
pub async fn autorizza_da_sessione(s: &mut SessioneMicrosoft, tenant: &str, trasporto: &Rc<dyn Trasporto>, adesso_s: i64, cliente: Option<ClienteWeb<'_>>) -> EsitoAccesso<Mappa> {
    let cliente = cliente.unwrap_or(ClienteWeb { id: OUTLOOK_WEB_CLIENT_ID, redirect: OUTLOOK_WEB_REDIRECT_URI });
    if let (Some(proxy), true) = (&mut s.proxy, cliente.id == OUTLOOK_WEB_CLIENT_ID) {
        return authorize_proxy(proxy, &s.login_hint.trim().to_lowercase(), &s.user_agent, trasporto).await;
    }
    if !tenant_valido(tenant) {
        return Err(ErroreAccesso::altro("Tenant Microsoft non valido"));
    }
    let endpoint = format!("{MS_LOGIN}/{}/oauth2/v2.0", codifica_componente(tenant));
    let mut url = Url::parse(&format!("{endpoint}/authorize")).map_err(|_| ErroreAccesso::altro("Invalid URL"))?;
    let percorso = url.path().to_string();
    let mut validi: Vec<&CookieSessione> = s
        .cookies
        .iter()
        .filter(|c| {
            let e = scadenza(c);
            let p = campo(c, "path");
            (e == -1.0 || e > adesso_s as f64) && (p == "/" || percorso == p || percorso.starts_with(&if p.ends_with('/') { p.to_string() } else { format!("{p}/") }))
        })
        .collect();
    let ests = |n: &str| {
        let n = n.to_ascii_uppercase();
        n == "ESTSAUTH" || n == "ESTSAUTHPERSISTENT" || n == "ESTSAUTHLIGHT"
    };
    if !validi.iter().any(|c| ests(campo(c, "name"))) {
        return Err(ErroreAccesso::SessioneScaduta("Sessione Microsoft finita: accedi di nuovo da Outlook web".to_string()));
    }
    let verifier = casuale_base64url(32);
    let state = casuale_base64url(24);
    let sfida = sha256_base64url(&verifier);
    url.set_query(Some(&modulo(&[
        ("client_id", cliente.id),
        ("redirect_uri", cliente.redirect),
        ("response_type", "code"),
        ("response_mode", "query"),
        ("scope", OUTLOOK_WEB_SCOPES),
        ("prompt", "none"),
        ("login_hint", &s.login_hint),
        ("state", &state),
        ("code_challenge", &sfida),
        ("code_challenge_method", "S256"),
    ])));
    validi.sort_by_key(|c| std::cmp::Reverse(campo(c, "path").encode_utf16().count()));
    let cookie = validi.iter().map(|c| format!("{}={}", campo(c, "name"), campo(c, "value"))).collect::<Vec<_>>().join("; ");
    let r = trasporto
        .invia(Richiesta::get(url.as_str()).intestazione("Cookie", cookie).intestazione("User-Agent", s.user_agent.clone()).senza_redirect().scadenza(Duration::from_millis(20_000)))
        .await?;
    ruota(s, &r, adesso_s);
    if r.stato == 429 || r.stato >= 500 {
        return Err(ErroreAccesso::altro(format!("Autorizzazione Microsoft non disponibile (HTTP {})", r.stato)));
    }
    if r.stato == 200 && cliente.id == OUTLOOK_WEB_CLIENT_ID {
        static MCAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)<form(?-u:\b)[^>]*(?-u:\b)action=["']https://[a-z0-9-]+\.access\.mcas\.ms/aad_login["']"#).expect("valida"));
        let html = leggi_testo(&r, Some(2 * 1024 * 1024))?;
        if MCAS.is_match(&html) {
            return Err(ErroreAccesso::ProxyBrowser("Microsoft requires its security-proxy browser session".to_string()));
        }
    }
    let chiedi_di_nuovo = || ErroreAccesso::SessioneScaduta("Microsoft chiede di accedere di nuovo da Outlook web".to_string());
    let location = r.intestazioni.ottieni("location");
    let Some(location) = location.filter(|l| !l.is_empty() && (300..400).contains(&r.stato)) else { return Err(chiedi_di_nuovo()) };
    let base = Url::parse(MS_LOGIN).map_err(|_| ErroreAccesso::altro("Invalid URL"))?;
    let ritorno = base.join(&location).map_err(|_| ErroreAccesso::altro("Invalid URL"))?;
    let atteso = Url::parse(cliente.redirect).map_err(|_| ErroreAccesso::altro("Invalid URL"))?;
    let host = |u: &Url| format!("{}{}", u.host_str().unwrap_or_default(), u.port().map(|p| format!(":{p}")).unwrap_or_default());
    if ritorno.scheme() != atteso.scheme() || host(&ritorno) != host(&atteso) || ritorno.path() != atteso.path() || !ritorno.username().is_empty() || ritorno.password().is_some() {
        return Err(chiedi_di_nuovo());
    }
    if parametro(&ritorno, "state").as_deref() != Some(state.as_str()) {
        return Err(ErroreAccesso::altro("Autorizzazione Microsoft: state diverso"));
    }
    let errore = parametro(&ritorno, "error");
    if matches!(errore.as_deref(), Some("server_error" | "temporarily_unavailable")) {
        return Err(ErroreAccesso::altro("Autorizzazione Microsoft momentaneamente non disponibile"));
    }
    let descrizione = parametro(&ritorno, "error_description").unwrap_or_default();
    if errore.as_deref().is_some_and(|e| !e.is_empty()) && aadsts53003(&descrizione) {
        return Err(ErroreAccesso::SessioneScaduta("Microsoft tenant policy refused this client (AADSTS53003). Repeating sign-in will not change the policy.".to_string()));
    }
    let code = parametro(&ritorno, "code").filter(|c| !c.is_empty());
    let (None | Some(""), Some(code)) = (errore.as_deref(), code) else { return Err(chiedi_di_nuovo()) };
    let mut richiesta = Richiesta::post(
        format!("{endpoint}/token"),
        modulo(&[("client_id", cliente.id), ("redirect_uri", cliente.redirect), ("grant_type", "authorization_code"), ("code", &code), ("code_verifier", &verifier), ("scope", OUTLOOK_WEB_SCOPES)]),
    )
    .intestazione("Content-Type", "application/x-www-form-urlencoded")
    .intestazione("Accept", "application/json");
    if atteso.scheme() == "https" {
        richiesta = richiesta.intestazione("Origin", atteso.origin().ascii_serialization());
    }
    let t = trasporto.invia(richiesta.intestazione("User-Agent", s.user_agent.clone()).senza_redirect().scadenza(Duration::from_millis(20_000))).await?;
    ruota(s, &t, adesso_s);
    let json = json_o_vuoto(&t);
    if t.stato == 429 || t.stato >= 500 {
        return Err(ErroreAccesso::altro(format!("Scambio codice Microsoft non disponibile (HTTP {})", t.stato)));
    }
    if matches!(json.get("error").and_then(Json::as_str), Some("server_error" | "temporarily_unavailable")) {
        return Err(ErroreAccesso::altro("Microsoft authorization temporarily unavailable"));
    }
    if !t.ok() || !matches!(json.get("access_token"), Some(Json::Str(_))) {
        return Err(chiedi_di_nuovo());
    }
    Ok(json)
}

/// `/AADSTS53003\b/`
fn aadsts53003(testo: &str) -> bool {
    testo.match_indices("AADSTS53003").any(|(i, m)| !testo[i + m.len()..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_'))
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn date_dei_cookie_e_sessione() {
        assert_eq!(data_cookie("Wed, 21 Oct 2026 07:28:00 GMT"), Some(1792567680000.0));
        assert_eq!(data_cookie("Wed, 21-Oct-2026 07:28:00 GMT"), Some(1792567680000.0));
        assert_eq!(data_cookie("domani"), None);
        let s = leggi_sessione(
            &Json::analizza(r#"{"userAgent":"UA","loginHint":"a@b.it","cookies":[{"name":"ESTSAUTH","value":"x","domain":".login.microsoftonline.com","path":"/","expires":-1,"httpOnly":true},{"name":"y","value":"1","domain":"example.com","path":"/","expires":-1}],"altro":1}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(s.in_json().in_js(), r#"{"userAgent":"UA","loginHint":"a@b.it","cookies":[{"name":"ESTSAUTH","value":"x","domain":".login.microsoftonline.com","path":"/","expires":-1,"httpOnly":true}]}"#);
        assert!(leggi_sessione(&Json::analizza(r#"{"userAgent":"a\nb","loginHint":"","cookies":[]}"#).unwrap()).is_none());
        assert!(aadsts53003("AADSTS53003: blocked") && !aadsts53003("AADSTS530031"));
    }
}
