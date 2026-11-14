//! The Microsoft session of a light account renewed over HTTP (src/agent/light/session-auth.ts): the login cookies the
//! browser run captured ask the authorize endpoint for a code with prompt=none, and the code is redeemed with PKCE for
//! a fresh refresh token of Teams web. A session that wants the owner gives NeedsSignIn: the browser renewal follows.

use super::http::{Http, Response, all, json_or_empty, send};
use super::token::StoredToken;
use super::{Fail, R, now_ms, random_bytes};
use serde_json::{Map, Value, json};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const TEAMS_WEB_CLIENT: &str = "5e3ce6c0-2b1f-4285-8d4b-75ee78787346";
const LOGIN: &str = "https://login.microsoftonline.com";
const REDIRECT: &str = "https://teams.cloud.microsoft/v2/authv2";
const SCOPE: &str = "https://ic3.teams.office.com/.default openid profile offline_access";

pub fn session_file(token_file: &Path) -> PathBuf {
    token_file.parent().map(PathBuf::from).unwrap_or_default().join("session.json")
}

pub fn needs(m: &str) -> Fail {
    Fail::NeedsSignIn(m.into())
}

/// A cookie as the browser gave it (other fields kept as they are): name, value, domain, path, expires (-1: session).
pub type Cookie = Map<String, Value>;

pub fn field<'a>(c: &'a Cookie, k: &str) -> &'a str {
    c.get(k).and_then(Value::as_str).unwrap_or("")
}

pub fn expires(c: &Cookie) -> f64 {
    c.get("expires").and_then(Value::as_f64).unwrap_or(f64::NAN)
}

/// A name and a value that cannot carry another header.
pub fn token_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+.^_`|~-".contains(c))
}

pub fn clean_value(v: &str) -> bool {
    !v.chars().any(|c| c <= '\u{20}' || c == '\u{7f}' || c == ';')
}

fn login_cookie(c: &Cookie) -> bool {
    let domain = field(c, "domain").to_lowercase();
    (domain == "login.microsoftonline.com" || domain == ".login.microsoftonline.com")
        && token_name(field(c, "name"))
        && clean_value(field(c, "value"))
        && field(c, "path").starts_with('/')
}

/// What a cookie of the session file must be to be used.
fn usable(c: &Value) -> Option<Cookie> {
    let Value::Object(c) = c else { return None };
    let strings = ["name", "value", "domain", "path"].iter().all(|k| c.get(*k).is_some_and(Value::is_string));
    (strings && expires(c).is_finite() && login_cookie(c)).then(|| c.clone())
}

/// The session the browser run captured: user agent, login hint, the cookies of login.microsoftonline.com.
#[derive(Clone, Debug, PartialEq)]
pub struct MicrosoftSession {
    pub user_agent: String,
    pub login_hint: String,
    pub cookies: Vec<Cookie>,
}

/// fs.writeFileSync(file.new, mode 0600), chmod, rename: never a half-written credential.
pub fn save_credential(file: &Path, value: &Value) -> R<()> {
    let tmp = PathBuf::from(format!("{}.new", file.display()));
    let io = |e: std::io::Error| Fail::Other(e.to_string());
    super::write_private(&tmp, crate::js::stringify(value).as_bytes()).map_err(io)?;
    super::chmod_private(&tmp).map_err(io)?;
    std::fs::rename(&tmp, file).map_err(io)
}

impl MicrosoftSession {
    pub fn save(file: &Path, s: &MicrosoftSession) -> R<()> {
        let cookies: Vec<Value> = s.cookies.iter().filter(|c| login_cookie(c)).map(|c| Value::Object(c.clone())).collect();
        save_credential(file, &json!({ "userAgent": s.user_agent, "loginHint": s.login_hint, "cookies": cookies }))
    }

    pub fn load(file: &Path) -> Option<MicrosoftSession> {
        let v: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let ua = v.get("userAgent").and_then(Value::as_str).filter(|u| !u.is_empty() && !u.contains(['\r', '\n']))?;
        let hint = v.get("loginHint").and_then(Value::as_str)?;
        let cookies = v.get("cookies").and_then(Value::as_array)?;
        Some(MicrosoftSession { user_agent: ua.into(), login_hint: hint.into(), cookies: cookies.iter().filter_map(usable).collect() })
    }
}

/// Date.parse of the Expires of a cookie: "Wdy, DD Mon YYYY HH:MM:SS GMT" or with dashes ("DD-Mon-YYYY"), in ms.
pub fn cookie_date(v: &str) -> Option<f64> {
    if let Some(ms) = super::parse_date(v) {
        return Some(ms);
    }
    let parts: Vec<&str> = v.split([' ', ',', '-']).filter(|p| !p.is_empty()).collect();
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let (mut day, mut month, mut year, mut time) = (None, None, None, None);
    for p in parts {
        let low = p.to_ascii_lowercase();
        if let Some(m) = months.iter().position(|m| low.starts_with(m)) {
            month = Some(m as i8 + 1);
        } else if p.contains(':') {
            time = Some(p);
        } else if let Ok(n) = p.parse::<i32>() {
            if n > 31 { year = Some(n) } else { day = Some(n) }
        }
    }
    let (day, month, mut year, time) = (day?, month?, year?, time?);
    if year < 100 {
        year += if year < 50 { 2000 } else { 1900 };
    }
    let hms: Vec<i8> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    let dt = jiff::civil::DateTime::new(year as i16, month, day as i8, *hms.first()?, *hms.get(1).unwrap_or(&0), *hms.get(2).unwrap_or(&0), 0).ok()?;
    dt.to_zoned(jiff::tz::TimeZone::UTC).ok().map(|z| z.timestamp().as_millisecond() as f64)
}

/// The cookies an answer sets on the session: replaced by name, domain and path, kept while they last.
pub fn rotate(cookies: &mut Vec<Cookie>, r: &Response, now: i64, accept: &dyn Fn(&Cookie) -> bool) {
    for raw in all(r, "set-cookie") {
        let mut attrs = raw.split(';');
        let pair = attrs.next().unwrap_or("");
        let Some(eq) = pair.find('=').filter(|e| *e >= 1) else { continue };
        let mut c = Cookie::new();
        c.insert("name".into(), json!(pair[..eq].trim()));
        c.insert("value".into(), json!(pair[eq + 1..].trim()));
        c.insert("domain".into(), json!("login.microsoftonline.com"));
        c.insert("path".into(), json!("/"));
        c.insert("expires".into(), json!(-1));
        let mut max_age = None;
        for attr in attrs {
            let (k, v) = match attr.find('=') {
                Some(at) => (attr[..at].trim().to_lowercase(), attr[at + 1..].trim()),
                None => (attr.trim().to_lowercase(), ""),
            };
            match k.as_str() {
                "domain" => {
                    c.insert("domain".into(), json!(v));
                }
                "path" => {
                    c.insert("path".into(), json!(v));
                }
                "expires" => {
                    if let Some(ms) = cookie_date(v) {
                        c.insert("expires".into(), crate::web::num(ms / 1000.0));
                    }
                }
                "max-age"
                    if !v.trim_start_matches('-').is_empty()
                        && v.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit())
                        && v.matches('-').count() <= 1 =>
                {
                    max_age = v.parse::<f64>().ok()
                }
                _ => {}
            }
        }
        if let Some(age) = max_age {
            c.insert("expires".into(), crate::web::num(now as f64 / 1000.0 + age));
        }
        if !accept(&c) {
            continue;
        }
        let bare = |d: &str| d.trim_start_matches('.').to_lowercase();
        cookies.retain(|old| {
            !(field(old, "name") == field(&c, "name") && bare(field(old, "domain")) == bare(field(&c, "domain")) && field(old, "path") == field(&c, "path"))
        });
        let e = expires(&c);
        if e == -1.0 || e > now as f64 / 1000.0 {
            cookies.push(c);
        }
    }
}

pub fn pkce() -> (String, String) {
    let verifier = crate::js::base64_url(&random_bytes(32));
    let challenge = crate::js::base64_url(&sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub struct SessionAuth {
    file: PathBuf,
    http: Http,
    now: Rc<dyn Fn() -> i64>,
}

impl SessionAuth {
    pub fn new(file: PathBuf, http: Http) -> Self {
        SessionAuth { file, http, now: Rc::new(now_ms) }
    }

    #[cfg(test)]
    pub fn with_clock(file: PathBuf, http: Http, now: Rc<dyn Fn() -> i64>) -> Self {
        SessionAuth { file, http, now }
    }

    async fn request(&self, method: &'static str, url: &str, headers: Vec<(&str, String)>, body: Vec<u8>) -> R<Response> {
        send(&self.http, method, url, headers, body, 20_000).await.map_err(|_| Fail::Other("Microsoft session request failed".into()))
    }

    fn rotate(&self, s: &mut MicrosoftSession, r: &Response) -> R<()> {
        rotate(&mut s.cookies, r, (self.now)(), &login_cookie);
        MicrosoftSession::save(&self.file, s)
    }

    pub async fn renew(&self, stored: &StoredToken) -> R<StoredToken> {
        let Some(mut s) = MicrosoftSession::load(&self.file) else { return Err(needs("Microsoft session missing or unreadable")) };
        let endpoint = format!("{LOGIN}/{}/oauth2/v2.0", crate::js::encode_uri_component(&stored.tenant_id));
        let mut url = url::Url::parse(&format!("{endpoint}/authorize")).map_err(|e| Fail::Other(e.to_string()))?;
        let now_s = (self.now)() as f64 / 1000.0;
        let pathname = url.path().to_string();
        let mut cookies: Vec<&Cookie> = s
            .cookies
            .iter()
            .filter(|c| {
                let (e, p) = (expires(c), field(c, "path"));
                (e == -1.0 || e > now_s) && (p == "/" || pathname == p || pathname.starts_with(&if p.ends_with('/') { p.to_string() } else { format!("{p}/") }))
            })
            .collect();
        if !cookies.iter().any(|c| regex::Regex::new("(?i)^ESTSAUTH(?:PERSISTENT|LIGHT)?$").expect("fixed").is_match(field(c, "name"))) {
            return Err(needs("Microsoft session ended"));
        }
        let (verifier, challenge) = pkce();
        let state = crate::js::base64_url(&random_bytes(24));
        url.set_query(Some(&crate::js::search_params(&[
            ("client_id", &stored.client_id),
            ("redirect_uri", REDIRECT),
            ("response_type", "code"),
            ("response_mode", "query"),
            ("scope", SCOPE),
            ("prompt", "none"),
            ("login_hint", &s.login_hint),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ])));
        cookies.sort_by_key(|c| std::cmp::Reverse(field(c, "path").chars().count()));
        let cookie = cookies.iter().map(|c| format!("{}={}", field(c, "name"), field(c, "value"))).collect::<Vec<_>>().join("; ");
        let r = self.request("GET", url.as_str(), vec![("Cookie", cookie), ("User-Agent", s.user_agent.clone())], vec![]).await?;
        self.rotate(&mut s, &r)?;
        if r.status == 429 || r.status >= 500 {
            return Err(Fail::Other(format!("Microsoft authorize HTTP {}", r.status)));
        }
        let location = r.header("location").unwrap_or("");
        if r.status < 300 || r.status >= 400 || location.is_empty() {
            return Err(needs("Microsoft session requires sign-in"));
        }
        let base = url::Url::parse(LOGIN).expect("fixed");
        let back = base.join(location).map_err(|_| needs("Microsoft session requires interactive sign-in"))?;
        if format!("{}{}", back.origin().ascii_serialization(), back.path()) != REDIRECT || !back.username().is_empty() || back.password().is_some() {
            return Err(needs("Microsoft session requires interactive sign-in"));
        }
        let q = |k: &str| back.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string());
        if q("state").as_deref() != Some(state.as_str()) {
            return Err(Fail::Other("Microsoft authorize state mismatch".into()));
        }
        match q("error").as_deref() {
            Some("server_error" | "temporarily_unavailable") => return Err(Fail::Other("Microsoft authorize temporarily unavailable".into())),
            Some(_) => return Err(needs("Microsoft session requires sign-in")),
            None => {}
        }
        let Some(code) = q("code").filter(|c| !c.is_empty()) else { return Err(needs("Microsoft authorize returned no code")) };
        let body = crate::js::search_params(&[
            ("client_id", &stored.client_id),
            ("redirect_uri", REDIRECT),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &verifier),
            ("scope", SCOPE),
        ]);
        let headers = vec![
            ("Content-Type", "application/x-www-form-urlencoded".to_string()),
            ("Origin", "https://teams.cloud.microsoft".to_string()),
            ("User-Agent", s.user_agent.clone()),
        ];
        let token = self.request("POST", &format!("{endpoint}/token"), headers, body.into_bytes()).await?;
        self.rotate(&mut s, &token)?;
        let j = json_or_empty(&token);
        if token.status == 429 || token.status >= 500 {
            return Err(Fail::Other(format!("Microsoft code redeem HTTP {}", token.status)));
        }
        let Some(refresh) = j.get("refresh_token").and_then(Value::as_str).filter(|t| token.ok() && !t.is_empty()) else {
            return Err(needs("Microsoft code redeem requires sign-in"));
        };
        Ok(StoredToken { refresh_token: refresh.into(), expires_on: (self.now)().div_euclid(1000) + 86400, ..stored.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{Net, response, run, temp_dir};

    fn stored() -> StoredToken {
        StoredToken { refresh_token: "old".into(), client_id: "client".into(), tenant_id: "tenant".into(), region: "emea".into(), expires_on: 1001 }
    }

    fn cookie() -> Cookie {
        json!({ "name": "ESTSAUTHPERSISTENT", "value": "cookie-secret", "domain": ".login.microsoftonline.com", "path": "/", "expires": 90000 })
            .as_object()
            .cloned()
            .expect("object")
    }

    fn setup(net: &Net) -> (PathBuf, SessionAuth) {
        let file = temp_dir().join("session.json");
        MicrosoftSession::save(&file, &MicrosoftSession { user_agent: "browser-ua".into(), login_hint: "me@example.com".into(), cookies: vec![cookie()] })
            .expect("saved");
        (file.clone(), SessionAuth::with_clock(file, net.http(), Rc::new(|| 1_000_000)))
    }

    fn query(url: &str) -> std::collections::HashMap<String, String> {
        url::Url::parse(url).expect("url").query_pairs().into_owned().collect()
    }

    #[test]
    fn uses_pkce_state_and_the_exact_redirect_without_sending_cookies_to_token_redemption() {
        let net = Net::new();
        let (_, session) = setup(&net);
        net.respond(|s| {
            if s.url.contains("/authorize") {
                let p = query(&s.url);
                assert_eq!(
                    (p["prompt"].as_str(), p["login_hint"].as_str(), p["code_challenge_method"].as_str(), p["response_mode"].as_str()),
                    ("none", "me@example.com", "S256", "query")
                );
                let location = format!("{}?code=code-secret&state={}", p["redirect_uri"], p["state"]);
                return response(302, &[("Location", &location)], "");
            }
            response(200, &[], r#"{"refresh_token":"new"}"#)
        });
        assert_eq!(run(session.renew(&stored())).expect("renewed"), StoredToken { refresh_token: "new".into(), expires_on: 87400, ..stored() });
        let seen = net.requests();
        let challenge = query(&seen[0].url)["code_challenge"].clone();
        assert!(seen[1].header("Cookie").is_none());
        assert_eq!(seen[1].header("Origin"), Some("https://teams.cloud.microsoft"));
        let body: std::collections::HashMap<String, String> = form_urlencoded::parse(&seen[1].body).into_owned().collect();
        assert_eq!(crate::js::base64_url(&sha2::Sha256::digest(body["code_verifier"].as_bytes())), challenge);
        assert_eq!(body["code"], "code-secret");
    }

    #[test]
    fn persists_rotated_cookies_on_a_failed_authorize_respecting_max_age_and_domain() {
        let net = Net::new();
        let (file, session) = setup(&net);
        net.respond(|_| response(503, &[("Set-Cookie", "ESTSAUTHPERSISTENT=rolled; Domain=.login.microsoftonline.com; Path=/; Max-Age=7200")], ""));
        assert!(matches!(run(session.renew(&stored())), Err(Fail::Other(_))));
        let mut rolled = cookie();
        rolled.insert("value".into(), json!("rolled"));
        rolled.insert("expires".into(), json!(8200));
        assert_eq!(MicrosoftSession::load(&file).expect("session").cookies, [rolled]);
    }

    #[test]
    fn an_owner_sign_in_is_needed_without_disclosing_the_error_description() {
        for error in ["login_required", "interaction_required", "consent_required"] {
            let net = Net::new();
            let (_, session) = setup(&net);
            net.respond(move |s| {
                let location =
                    format!("https://teams.cloud.microsoft/v2/authv2?state={}&error={error}&error_description=cookie-secret", query(&s.url)["state"]);
                response(302, &[("Location", &location)], "")
            });
            let e = run(session.renew(&stored())).unwrap_err();
            assert!(matches!(&e, Fail::NeedsSignIn(m) if !m.contains("cookie-secret")), "{e:?}");
        }
    }

    #[test]
    fn rejects_a_state_mismatch_and_never_follows_an_unexpected_redirect() {
        let net = Net::new();
        let (_, session) = setup(&net);
        net.respond(|_| response(302, &[("Location", "https://teams.cloud.microsoft/v2/authv2?state=wrong&code=code-secret")], ""));
        assert!(run(session.renew(&stored())).unwrap_err().to_string().contains("state mismatch"));
        assert_eq!(net.requests().len(), 1);
        let net = Net::new();
        let (_, session) = setup(&net);
        net.respond(|_| response(302, &[("Location", "https://other.test/?code=code-secret")], ""));
        assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
        assert_eq!(net.requests().len(), 1);
    }

    #[test]
    fn missing_malformed_or_expired_cookies_need_sign_in() {
        let net = Net::new();
        let (file, session) = setup(&net);
        let mut old = cookie();
        old.insert("expires".into(), json!(999));
        for content in ["{}".to_string(), "broken".to_string(), json!({ "userAgent": "ua", "loginHint": "", "cookies": [old] }).to_string()] {
            std::fs::write(&file, content).expect("write");
            assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
        }
    }

    #[test]
    fn filters_other_hosts_header_injection_and_cookie_paths_before_sending() {
        let net = Net::new();
        let (file, session) = setup(&net);
        let with = |k: &str, v: &str| {
            let mut c = cookie();
            c.insert(k.into(), json!(v));
            c
        };
        let cookies = vec![cookie(), with("domain", "evil.login.microsoftonline.com"), with("name", "bad\r\n"), with("path", "/other")];
        MicrosoftSession::save(&file, &MicrosoftSession { user_agent: "ua".into(), login_hint: "".into(), cookies }).expect("saved");
        net.respond(|_| response(503, &[], ""));
        assert!(run(session.renew(&stored())).unwrap_err().to_string().contains("HTTP 503"));
        assert_eq!(net.requests()[0].header("Cookie"), Some("ESTSAUTHPERSISTENT=cookie-secret"));
    }

    #[test]
    fn dates_of_cookies() {
        assert_eq!(cookie_date("Thu, 01 Jan 2026 00:00:00 GMT"), Some(1_767_225_600_000.0));
        assert_eq!(cookie_date("Thu, 01-Jan-2026 00:00:00 GMT"), Some(1_767_225_600_000.0));
        assert_eq!(cookie_date("nonsense"), None);
    }
}
