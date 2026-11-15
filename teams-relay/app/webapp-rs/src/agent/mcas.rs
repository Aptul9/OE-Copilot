//! The Microsoft session of a light account behind Defender for Cloud Apps renewed over HTTP (src/agent/light/
//! mcas-session.ts): the sign-in through the proxy of the tenant, step by step, each step checked against the shape
//! observed; anything else hands the account to its browser renewal. The TypeScript used a Playwright request context
//! for its cookie jar; this keeps a jar of its own with the same matching rules.

use super::forms;
use super::http::{Http, Response, all, send};
use super::session::{Cookie, TEAMS_WEB_CLIENT, clean_value, cookie_date, expires, field, needs, pkce, save_credential, token_name};
use super::token::StoredToken;
use super::{Fail, R, now_ms, random_bytes, random_uuid};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use url::Url;

const LOGIN: &str = "https://login.microsoftonline.com";
const PROXY: &str = "https://login.microsoftonline.com.mcas.ms";
const APPS: [&str; 2] = ["https://teams.microsoft.com", "https://teams.cloud.microsoft"];
const SCOPE: &str = "openid profile offline_access";

pub fn mcas_session_file(token_file: &Path) -> PathBuf {
    token_file.parent().map(PathBuf::from).unwrap_or_default().join("mcas-session.json")
}

fn valid_cookie(c: &Value) -> Option<Cookie> {
    let Value::Object(c) = c else { return None };
    let strings = ["name", "value", "domain", "path"].iter().all(|k| c.get(*k).is_some_and(Value::is_string));
    if !strings || !expires(c).is_finite() || !token_name(field(c, "name")) || !clean_value(field(c, "value")) || !field(c, "path").starts_with('/') {
        return None;
    }
    let domain = field(c, "domain").trim_start_matches('.').to_lowercase();
    let name = field(c, "name");
    let ok = domain == "login.microsoftonline.com"
        || (domain == "mcas.ms" && regex::Regex::new(r"^AdallomAuth\d*$").expect("fixed").is_match(name))
        || (regex::Regex::new(r"^[a-z0-9-]+\.access\.mcas\.ms$").expect("fixed").is_match(&domain) && name == "CAACSkipDisclaimer");
    ok.then(|| c.clone())
}

fn bootstrap_address(value: &str) -> R<Url> {
    let u = Url::parse(value).map_err(|_| needs("MCAS bootstrap destination unsupported"))?;
    let plain = u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.port().is_none();
    if !plain || u.host_str() != Some("mcas-proxyweb.mcas.ms") || u.path() != "/js-bootstrap.js" {
        return Err(needs("MCAS bootstrap destination unsupported"));
    }
    Ok(u)
}

/// A parser that refuses: the browser renewal takes over.
fn normal<T>(r: forms::Res<T>) -> R<T> {
    r.map_err(|_| needs("MCAS continuation needs browser renewal"))
}

struct Settings {
    auth_user_data: String,
    top_frame_saas_id: String,
    tsid: bool,
    extended_tsid: bool,
    by_url: bool,
}

fn settings(source: &str) -> R<Settings> {
    let c = normal(forms::configuration(source, "__JSWRAPPER.initWrapperFixer"))?;
    // only the observed non-integrity context is implemented; a policy change keeps the browser fallback
    if c.get("urlIntegrityEnforcementEnabled") != Some(&Value::Bool(false))
        || c.get("suffixDomainName") != Some(&json!("mcas.ms"))
        || c.get("niptuckSubDomainName") != Some(&json!("mcas-proxyweb"))
    {
        return Err(needs("MCAS policy needs browser renewal"));
    }
    let auth = c.get("authUserData").and_then(Value::as_str).unwrap_or("");
    let tsid = c.get("topFrameSaasId").filter(|v| v.is_string() || v.is_number());
    let (Some(tsid), false) = (tsid, auth.is_empty() || auth.encode_utf16().count() > 30000) else { return Err(needs("MCAS bootstrap incomplete")) };
    let t = |k: &str| c.get(k).is_some_and(crate::js::truthy);
    Ok(Settings {
        auth_user_data: auth.into(),
        top_frame_saas_id: crate::js::string(Some(tsid)),
        tsid: t("topFrameSaasIdParamEnabled"),
        extended_tsid: t("extendedTopFrameSaasIdParamEnabled"),
        by_url: t("resolveNavigationRequestByUrlEnabled"),
    })
}

/// URLSearchParams.set: the first one of the name takes the value, the others go; appended when there is none.
fn set_param(url: &mut Url, name: &str, value: &str) {
    let mut pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    match pairs.iter().position(|(k, _)| k == name) {
        Some(i) => {
            pairs[i].1 = value.into();
            let mut seen = false;
            pairs.retain(|(k, _)| {
                let keep = k != name || !seen;
                seen |= k == name;
                keep
            });
        }
        None => pairs.push((name.into(), value.into())),
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    url.set_query(Some(&crate::js::search_params(&refs)));
}

fn proxy_url(mut url: Url, xhr: bool, c: &Settings) -> Url {
    if c.extended_tsid || (!xhr && c.tsid) {
        set_param(&mut url, "McasTsid", &c.top_frame_saas_id);
    }
    if xhr || c.by_url {
        set_param(&mut url, "McasCtx", if xhr { "2" } else { "4" });
    }
    if xhr {
        set_param(&mut url, "McasUserAuth", &c.auth_user_data);
    }
    url
}

fn permitted(value: &str, gateway: &str) -> R<Url> {
    let u = Url::parse(value).map_err(|_| Fail::Other("Invalid URL".into()))?;
    let allowed = [
        "login.microsoftonline.com",
        "login.microsoftonline.com.mcas.ms",
        "teams.microsoft.com",
        "teams.microsoft.com.mcas.ms",
        "teams.cloud.microsoft",
        "teams.cloud.microsoft.mcas.ms",
        "inline.cdn.mcas.ms",
        gateway,
    ];
    let host = u.host_str().unwrap_or("");
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.port().is_some() || host.is_empty() || !allowed.contains(&host) {
        return Err(needs("MCAS auth destination unsupported"));
    }
    Ok(u)
}

fn origin(u: &Url) -> String {
    u.origin().ascii_serialization()
}

/// The cookies of the renewal, matched as Playwright's request context matches them.
struct Jar {
    cookies: Vec<Cookie>,
    now: Rc<dyn Fn() -> i64>,
}

impl Jar {
    fn live(&self, c: &Cookie) -> bool {
        let e = expires(c);
        e == -1.0 || e * 1000.0 >= (self.now)() as f64
    }

    fn header(&self, url: &Url) -> String {
        let host = url.host_str().unwrap_or("").to_lowercase();
        let path = url.path();
        let domain_ok = |d: &str| {
            let d = d.to_lowercase();
            host == d || (d.starts_with('.') && format!(".{host}").ends_with(&d))
        };
        let path_ok = |p: &str| {
            if path == p {
                return true;
            }
            let v = if path.ends_with('/') { path.to_string() } else { format!("{path}/") };
            let p = if p.ends_with('/') { p.to_string() } else { format!("{p}/") };
            v.starts_with(&p)
        };
        self.cookies
            .iter()
            .filter(|c| self.live(c) && domain_ok(field(c, "domain")) && path_ok(field(c, "path")))
            .filter(|c| !c.get("secure").and_then(Value::as_bool).unwrap_or(false) || url.scheme() == "https")
            .map(|c| format!("{}={}", field(c, "name"), field(c, "value")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The cookies an answer sets, for the host it came from: a Domain must cover that host.
    fn update(&mut self, url: &Url, r: &Response) {
        let host = url.host_str().unwrap_or("").to_lowercase();
        for raw in all(r, "set-cookie") {
            let mut attrs = raw.split(';');
            let pair = attrs.next().unwrap_or("");
            let Some(eq) = pair.find('=') else { continue };
            let mut c = Map::new();
            c.insert("name".into(), json!(pair[..eq].trim()));
            c.insert("value".into(), json!(pair[eq + 1..].trim()));
            c.insert("domain".into(), json!(host));
            c.insert("path".into(), json!(default_path(url.path())));
            c.insert("expires".into(), json!(-1));
            c.insert("httpOnly".into(), json!(false));
            c.insert("secure".into(), json!(false));
            c.insert("sameSite".into(), json!("Lax"));
            let mut max_age = None;
            let mut bad = false;
            for attr in attrs {
                let (k, v) = match attr.find('=') {
                    Some(at) => (attr[..at].trim().to_lowercase(), attr[at + 1..].trim()),
                    None => (attr.trim().to_lowercase(), ""),
                };
                match k.as_str() {
                    "domain" if !v.is_empty() => {
                        let d = v.to_lowercase();
                        let d = if d.starts_with('.') { d } else { format!(".{d}") };
                        bad |= !format!(".{host}").ends_with(&d);
                        c.insert("domain".into(), json!(d));
                    }
                    "path" if v.starts_with('/') => {
                        c.insert("path".into(), json!(v));
                    }
                    "expires" => {
                        if let Some(ms) = cookie_date(v) {
                            c.insert("expires".into(), crate::web::num(ms / 1000.0));
                        }
                    }
                    "max-age" => max_age = v.parse::<f64>().ok(),
                    "secure" => {
                        c.insert("secure".into(), json!(true));
                    }
                    "httponly" => {
                        c.insert("httpOnly".into(), json!(true));
                    }
                    "samesite" => {
                        let s = match v.to_lowercase().as_str() {
                            "strict" => "Strict",
                            "none" => "None",
                            _ => "Lax",
                        };
                        c.insert("sameSite".into(), json!(s));
                    }
                    _ => {}
                }
            }
            if bad {
                continue;
            }
            // a Max-Age of 0 or less ends the cookie now
            if let Some(age) = max_age {
                let at = if age <= 0.0 { 0.0 } else { (self.now)() as f64 / 1000.0 + age };
                c.insert("expires".into(), crate::web::num(at));
            }
            let same = |o: &Cookie| field(o, "name") == field(&c, "name") && field(o, "domain") == field(&c, "domain") && field(o, "path") == field(&c, "path");
            let live = self.live(&c);
            match self.cookies.iter_mut().find(|o| same(o)) {
                Some(o) if live => *o = c,
                Some(_) => self.cookies.retain(|o| !same(o)),
                None if live => self.cookies.push(c),
                None => {}
            }
        }
    }
}

/// The default path of a cookie set without one: the directory of the request path.
fn default_path(path: &str) -> String {
    if !path.starts_with('/') || path.matches('/').count() == 1 {
        return "/".into();
    }
    path[..path.rfind('/').unwrap_or(0)].to_string()
}

/// The body of an answer the flow reads: an error of the proxy is tried again, any other refusal or a body too big
/// goes to the browser.
fn text(r: &Response, limit: usize) -> R<String> {
    if r.status == 429 || r.status >= 500 {
        return Err(Fail::Other(format!("MCAS HTTP {}", r.status)));
    }
    if !r.ok() {
        return Err(needs("MCAS session requires browser renewal"));
    }
    if r.body.len() > limit {
        return Err(needs("MCAS continuation too large"));
    }
    Ok(String::from_utf8_lossy(&r.body).to_string())
}

pub struct McasSession {
    file: PathBuf,
    http: Http,
    now: Rc<dyn Fn() -> i64>,
}

/// The session file: user agent, login hint, tenant, the origin of Teams and the bootstrap of the proxy, the cookies.
pub fn load(file: &Path) -> Option<(Map<String, Value>, Vec<Cookie>)> {
    let Value::Object(s) = serde_json::from_slice::<Value>(&std::fs::read(file).ok()?).ok()? else { return None };
    let str_of = |k: &str| s.get(k).and_then(Value::as_str);
    let ua = str_of("userAgent").filter(|u| !u.is_empty() && !u.contains(['\r', '\n']))?;
    let _ = ua;
    str_of("loginHint").filter(|h| regex::Regex::new(r"^[^\s@]+@[^\s@]+$").expect("fixed").is_match(h))?;
    str_of("tenantId").filter(|t| regex::Regex::new(r"(?i)^[a-f0-9-]{36}$").expect("fixed").is_match(t))?;
    str_of("appOrigin").filter(|o| APPS.contains(o))?;
    let cookies = s.get("cookies").and_then(Value::as_array).filter(|c| c.len() <= 512)?;
    bootstrap_address(str_of("bootstrapUrl")?).ok()?;
    let cookies = cookies.iter().filter_map(valid_cookie).collect();
    Some((s, cookies))
}

/// A GET as the browser's request context makes it (Playwright's context.request): its cookies, the cookies of the
/// answer stored in the browser, no redirect followed.
async fn browser_get(http: &Http, browser: &crate::agent::browser::Browser, url: &Url, headers: Vec<(&str, String)>, timeout_ms: u64) -> R<Response> {
    let cookies = browser.cookies().await.map_err(Fail::Other)?;
    let mut headers = headers;
    let cookie = crate::cdp::cookie_header(&cookies, url);
    if !cookie.is_empty() {
        headers.push(("Cookie", cookie));
    }
    let r = send(http, "GET", url.as_str(), headers, vec![], timeout_ms).await?;
    let _ = browser.set_cookies(crate::cdp::set_cookies(url, &r)).await;
    Ok(r)
}

/// The session of a Teams page the browser holds behind the proxy (McasSession.capture): the bootstrap script its root
/// loads, checked as the renewal checks it, and the cookies of the sign-in and of the proxy. False when the page is not
/// behind the proxy, or anything is not of the shape observed.
pub async fn capture(file: &Path, http: &Http, page: &crate::agent::browser::Page, tenant_id: &str, login_hint: &str, user_agent: &str) -> bool {
    matches!(capture_session(file, http, page, tenant_id, login_hint, user_agent).await, Ok(true))
}

async fn capture_session(file: &Path, http: &Http, page: &crate::agent::browser::Page, tenant_id: &str, login_hint: &str, user_agent: &str) -> R<bool> {
    let here = Url::parse(&page.url()).map_err(|e| Fail::Other(e.to_string()))?;
    let own = here.origin().ascii_serialization();
    let app_origin = own.strip_suffix(".mcas.ms").unwrap_or(&own).to_string();
    if here.scheme() != "https" || !here.host_str().unwrap_or("").ends_with(".mcas.ms") || !APPS.contains(&app_origin.as_str()) {
        return Ok(false);
    }
    let browser = page.browser().ok_or_else(|| Fail::Other("no browser".into()))?;
    let ua = || vec![("User-Agent", user_agent.to_string())];
    let start = Url::parse(&format!("{own}/")).map_err(|e| Fail::Other(e.to_string()))?;
    let mut at = start.clone();
    let mut root = browser_get(http, &browser, &at, ua(), 15_000).await?;
    for _ in 0..2 {
        if !(300..400).contains(&root.status) {
            break;
        }
        let Some(next) = root.header("location").and_then(|l| at.join(l).ok()) else { return Ok(false) };
        if next.origin().ascii_serialization() != own || !["/v2", "/v2/"].contains(&next.path()) {
            return Ok(false);
        }
        at = next;
        root = browser_get(http, &browser, &at, ua(), 15_000).await?;
    }
    let Some(url) = forms::bootstrap_url(&text(&root, 2 * 1024 * 1024)?, &own).map_err(|e| Fail::Other(e.to_string()))? else { return Ok(false) };
    let address = bootstrap_address(&url)?;
    let mut headers = ua();
    headers.push(("Referer", start.to_string()));
    settings(&text(&browser_get(http, &browser, &address, headers, 10_000).await?, 128 * 1024)?)?;
    let cookies: Vec<Value> = browser.cookies().await.map_err(Fail::Other)?.iter().filter_map(valid_cookie).map(Value::Object).collect();
    let estsauth = regex::Regex::new("^ESTSAUTH(?:PERSISTENT|LIGHT)?$").expect("fixed");
    if !cookies.iter().any(|c| estsauth.is_match(c["name"].as_str().unwrap_or(""))) || login_hint.is_empty() {
        return Ok(false);
    }
    let captured =
        json!({ "tenantId": tenant_id, "loginHint": login_hint, "userAgent": user_agent, "appOrigin": app_origin, "bootstrapUrl": url, "cookies": cookies });
    save_credential(file, &captured)?;
    Ok(true)
}

impl McasSession {
    pub fn new(file: PathBuf, http: Http) -> Self {
        McasSession { file, http, now: Rc::new(now_ms) }
    }

    #[cfg(test)]
    pub fn with_clock(file: PathBuf, http: Http, now: Rc<dyn Fn() -> i64>) -> Self {
        McasSession { file, http, now }
    }

    pub async fn renew(&self, stored: &StoredToken) -> R<StoredToken> {
        let Some((session, cookies)) = load(&self.file)
            .filter(|(s, _)| s.get("tenantId").and_then(Value::as_str) == Some(stored.tenant_id.as_str()) && stored.client_id == TEAMS_WEB_CLIENT)
        else {
            return Err(needs("MCAS session missing or unsupported"));
        };
        let now_s = (self.now)() as f64 / 1000.0;
        let estsauth = regex::Regex::new("^ESTSAUTH(?:PERSISTENT|LIGHT)?$").expect("fixed");
        if !cookies.iter().any(|c| estsauth.is_match(field(c, "name")) && (expires(c) == -1.0 || expires(c) > now_s)) {
            return Err(needs("Microsoft session ended"));
        }
        let cookies = cookies
            .into_iter()
            .map(|mut c| {
                for (k, d) in [("httpOnly", json!(true)), ("secure", json!(true)), ("sameSite", json!("Lax"))] {
                    if c.get(k).is_none_or(Value::is_null) {
                        c.insert(k.into(), d);
                    }
                }
                c
            })
            .collect();
        let jar = RefCell::new(Jar { cookies, now: self.now.clone() });
        let result = self.flow(&session, stored, &jar).await;
        let kept: Vec<Value> = jar.borrow().cookies.iter().filter_map(|c| valid_cookie(&Value::Object(c.clone()))).map(Value::Object).collect();
        let mut saved = session.clone();
        saved.insert("cookies".into(), Value::Array(kept));
        let written = save_credential(&self.file, &Value::Object(saved));
        let token = result?;
        written?;
        Ok(token)
    }

    async fn flow(&self, session: &Map<String, Value>, stored: &StoredToken, jar: &RefCell<Jar>) -> R<StoredToken> {
        let s = |k: &str| session.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (ua, hint, app_origin) = (s("userAgent"), s("loginHint"), s("appOrigin"));
        let deadline = (self.now)() + 90_000;
        // one request: GET, or a form POSTed, with the cookies of the jar and the agent of the browser
        let go = |url: Url, form: Option<Vec<(String, String)>>, headers: Vec<(&'static str, String)>| {
            let ua = ua.clone();
            async move {
                let timeout = 15_000.min(deadline - (self.now)());
                if timeout <= 0 {
                    return Err(Fail::Other("MCAS renewal timed out".into()));
                }
                let cookie = jar.borrow().header(&url);
                let mut h: Vec<(&str, String)> = vec![("User-Agent", ua), ("Accept", "*/*".into())];
                if !cookie.is_empty() {
                    h.push(("Cookie", cookie));
                }
                h.extend(headers);
                let (method, body) = match form {
                    Some(f) => {
                        h.push(("Content-Type", "application/x-www-form-urlencoded".into()));
                        let refs: Vec<(&str, &str)> = f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                        ("POST", crate::js::search_params(&refs).into_bytes())
                    }
                    None => ("GET", vec![]),
                };
                let r = send(&self.http, method, url.as_str(), h, body, timeout as u64).await.map_err(|_| Fail::Other("MCAS request failed".into()))?;
                jar.borrow_mut().update(&url, &r);
                Ok::<(Url, Response), Fail>((url, r))
            }
        };
        let bootstrap = bootstrap_address(&s("bootstrapUrl"))?;
        let c = settings(&text(&go(bootstrap, None, vec![]).await?.1, 128 * 1024)?)?;
        let (verifier, challenge) = pkce();
        let mut back = Url::parse(&format!("{app_origin}/")).map_err(|e| Fail::Other(e.to_string()))?;
        back.set_query(Some(&crate::js::search_params(&[("enablemcasfort21", "true"), ("loginHint", &hint), ("tenantId", &stored.tenant_id)])));
        let meta = json!({ "id": random_uuid(), "meta": { "interactionType": "redirect" } });
        let state = format!("{}|{}", crate::js::base64_std(crate::js::stringify(&meta).as_bytes()), back);
        let redirect = format!("{app_origin}/v2/authv2");
        let mut url = Url::parse(&format!("{PROXY}/common/oauth2/v2.0/authorize")).expect("fixed");
        url.set_query(Some(&crate::js::search_params(&[
            ("client_id", &stored.client_id),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("response_mode", "fragment"),
            ("scope", SCOPE),
            ("client_info", "1"),
            ("login_hint", &hint),
            ("state", &state),
            ("nonce", &crate::js::base64_url(&random_bytes(24))),
            ("client-request-id", &random_uuid()),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ])));
        let (mut at, mut r) = go(proxy_url(url, false, &c), None, vec![]).await?;
        let (mut gateway, mut code, mut resumed) = (String::new(), String::new(), false);
        let moved_page = regex::Regex::new(r"(?i)<title>\s*Object Moved\s*</title>").expect("fixed");
        let form_page = regex::Regex::new(r"(?i)<form\b").expect("fixed");
        let reprocess = regex::Regex::new(r"^/(?:common|[a-f0-9-]{36})/reprocess$").expect("fixed");
        for _ in 0..14 {
            let from = permitted(at.as_str(), &gateway)?;
            let next = match r.header("location") {
                Some(l) => Some(permitted(from.join(l).map_err(|_| Fail::Other("Invalid URL".into()))?.as_str(), &gateway)?),
                None => None,
            };
            if let Some(next) = next {
                let p: Vec<(String, String)> = if next.query_pairs().any(|(k, _)| k == "state") {
                    next.query_pairs().into_owned().collect()
                } else {
                    form_urlencoded::parse(next.fragment().unwrap_or("").as_bytes()).into_owned().collect()
                };
                let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
                if get("code").is_some() || get("error").is_some() {
                    if origin(&next).trim_end_matches(".mcas.ms") != app_origin || next.path() != "/v2/authv2" {
                        return Err(needs("MCAS callback destination unsupported"));
                    }
                    if get("state").as_deref() != Some(state.as_str()) {
                        return Err(Fail::Other("MCAS callback state mismatch".into()));
                    }
                    match get("error").as_deref() {
                        Some("server_error" | "temporarily_unavailable") => return Err(Fail::Other("Microsoft authorize temporarily unavailable".into())),
                        Some(_) => return Err(needs("Microsoft session requires browser renewal")),
                        None => {}
                    }
                    code = normal(forms::authorization_code(next.as_str(), &state))?;
                    break;
                }
                (at, r) = if next.host_str() == Some("inline.cdn.mcas.ms") {
                    let (action, fields) = normal(forms::mcas_restore(next.as_str(), &gateway))?;
                    let o = origin(&next);
                    go(permitted(&action, &gateway)?, Some(fields), vec![("Origin", o.clone()), ("Referer", format!("{o}/"))]).await?
                } else {
                    go(next, None, vec![]).await?
                };
                continue;
            }
            let html = text(&r, 2 * 1024 * 1024)?;
            let o = origin(&from);
            if from.host_str() == Some(gateway.as_str()) && from.path() == "/finalize_login" && moved_page.is_match(&html) {
                let moved = normal(forms::mcas_moved(&html, ""))?;
                (at, r) = go(permitted(&moved, &gateway)?, None, vec![]).await?;
                continue;
            }
            if html.contains("PROXYWEB_MONITORED_ACCESS_MESSAGE") {
                let (action, fields) = normal(forms::mcas_continuation(&html))?;
                if gateway.is_empty() || Url::parse(&action).ok().and_then(|u| u.host_str().map(String::from)).as_deref() != Some(gateway.as_str()) {
                    return Err(needs("MCAS gateway changed"));
                }
                (at, r) = go(permitted(&action, &gateway)?, Some(fields), vec![("Origin", o.clone()), ("Referer", format!("{o}/"))]).await?;
                continue;
            }
            if form_page.is_match(&html) {
                if o != LOGIN {
                    return Err(needs("MCAS form issuer unsupported"));
                }
                let (action, id_token) = normal(forms::mcas_form(&html))?;
                let target = Url::parse(&action).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
                if !gateway.is_empty() && gateway != target {
                    return Err(needs("MCAS gateway changed"));
                }
                gateway = target;
                (at, r) =
                    go(permitted(&action, &gateway)?, Some(vec![("id_token".into(), id_token)]), vec![("Origin", o.clone()), ("Referer", format!("{o}/"))])
                        .await?;
                continue;
            }
            let Ok(config) = forms::configuration(&html, "$Config") else { return Err(needs("Microsoft session requires browser renewal")) };
            let sessions: Vec<&Value> = config
                .get("arrSessions")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|x| {
                            x.get("isSignedIn") == Some(&Value::Bool(true))
                                && x.get("matchesLoginHint") == Some(&Value::Bool(true))
                                && x.get("name").and_then(Value::as_str).is_some_and(|n| n.to_lowercase() == hint.to_lowercase())
                                && x.get("id").and_then(Value::as_str).is_some_and(|i| !i.is_empty())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let refresh = config.get("urlRefresh").and_then(Value::as_str);
            if let (false, "50058", Some(true), 1, Some(refresh)) =
                (resumed, crate::js::string(config.get("sErrorCode")).as_str(), config.get("fSsoFeasible").and_then(Value::as_bool), sessions.len(), refresh)
            {
                let mut resume = permitted(from.join(refresh).map_err(|_| Fail::Other("Invalid URL".into()))?.as_str(), &gateway)?;
                if origin(&resume) != LOGIN || !reprocess.is_match(resume.path()) {
                    return Err(needs("Microsoft session selection unsupported"));
                }
                set_param(&mut resume, "sessionid", sessions[0].get("id").and_then(Value::as_str).unwrap_or(""));
                resumed = true;
                (at, r) = go(resume, None, vec![]).await?;
                continue;
            }
            return Err(needs("Microsoft session requires browser renewal"));
        }
        if code.is_empty() {
            return Err(needs("MCAS continuation limit reached"));
        }
        let mut token_url = Url::parse(&format!("{PROXY}/common/oauth2/v2.0/token")).expect("fixed");
        set_param(&mut token_url, "client-request-id", &random_uuid());
        let mut target = proxy_url(token_url, true, &c);
        let form = vec![
            ("client_id".to_string(), stored.client_id.clone()),
            ("redirect_uri".to_string(), redirect.clone()),
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code),
            ("code_verifier".to_string(), verifier),
            ("scope".to_string(), SCOPE.to_string()),
        ];
        let mut origin_header = format!("{app_origin}.mcas.ms");
        for _ in 0..3 {
            r = go(target.clone(), Some(form.clone()), vec![("Origin", origin_header.clone())]).await?.1;
            if r.status != 307 {
                break;
            }
            target = permitted(target.join(r.header("location").unwrap_or("")).map_err(|_| Fail::Other("Invalid URL".into()))?.as_str(), &gateway)?;
            if origin(&target) != LOGIN || target.path() != "/common/oauth2/v2.0/token" {
                return Err(needs("MCAS token redirect unsupported"));
            }
            origin_header = "null".into();
        }
        let body = text(&r, 2 * 1024 * 1024)?;
        let result: Value = normal(serde_json::from_str(&body).map_err(|e| e.to_string()))?;
        let Some(refresh) = result.get("refresh_token").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
            return Err(needs("Microsoft code redemption needs browser renewal"));
        };
        for t in [result.get("access_token"), result.get("id_token")] {
            let Some(t) = t.and_then(Value::as_str).filter(|t| t.split('.').count() == 3) else { continue };
            let claims: Value =
                normal(serde_json::from_slice(&crate::js::base64_decode_lenient(t.split('.').nth(1).unwrap_or(""))).map_err(|e| e.to_string()))?;
            if let Some(tid) = claims.get("tid")
                && tid != &json!(stored.tenant_id)
            {
                return Err(needs("Microsoft returned a different tenant"));
            }
        }
        Ok(StoredToken { refresh_token: refresh.into(), expires_on: (self.now)().div_euclid(1000) + 86400, ..stored.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{Net, Seen, response, run, temp_dir};
    use sha2::Digest;

    const TENANT: &str = "11111111-2222-3333-4444-555555555555";
    const GATEWAY: &str = "tenant.access.mcas.ms";
    const BOOTSTRAP: &str = "https://mcas-proxyweb.mcas.ms/js-bootstrap.js?McasTsid=42&type=window";

    fn stored() -> StoredToken {
        StoredToken { refresh_token: "old-rt".into(), client_id: TEAMS_WEB_CLIENT.into(), tenant_id: TENANT.into(), region: "emea".into(), expires_on: 2000 }
    }

    #[derive(Default, Clone)]
    struct Options {
        wrong_state: bool,
        integrity: bool,
        sessions: Option<Value>,
        transient: bool,
        redirect: Option<&'static str>,
        malformed_bootstrap: bool,
        credential_form: bool,
        wrong_tenant: bool,
        plain_callback: bool,
        callback_error: Option<&'static str>,
    }

    fn query(u: &str, k: &str) -> Option<String> {
        Url::parse(u).ok()?.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string())
    }

    fn setup(o: Options) -> (Net, PathBuf, McasSession) {
        let file = temp_dir().join("mcas-session.json");
        let cookie = json!({ "name": "ESTSAUTHPERSISTENT", "value": "private-cookie", "domain": ".login.microsoftonline.com", "path": "/", "expires": 90000 });
        let proxy = json!({ "name": "AdallomAuth00", "value": "private-cookie", "domain": ".mcas.ms", "path": "/", "expires": 90000 });
        let data = json!({ "userAgent": "browser-ua", "loginHint": "me@example.com", "tenantId": TENANT, "appOrigin": "https://teams.microsoft.com", "bootstrapUrl": BOOTSTRAP, "cookies": [cookie, proxy] });
        std::fs::write(&file, data.to_string()).expect("session");
        let settings = json!({
            "suffixDomainName": "mcas.ms", "niptuckSubDomainName": "mcas-proxyweb", "tenantId": "a-different-mcas-internal-tenant",
            "authUserData": "private-proxy-auth", "topFrameSaasId": 42, "topFrameSaasIdParamEnabled": true,
            "extendedTopFrameSaasIdParamEnabled": true, "resolveNavigationRequestByUrlEnabled": true, "urlIntegrityEnforcementEnabled": o.integrity,
        });
        let net = Net::new();
        let state = Rc::new(RefCell::new((String::new(), String::new())));
        net.respond(move |s: &Seen| {
            let u = Url::parse(&s.url).expect("url");
            let host = u.host_str().unwrap_or("");
            if s.method == "GET" {
                if s.url == BOOTSTRAP {
                    let body = if o.malformed_bootstrap { "unrecognized-bootstrap".into() } else { format!("__JSWRAPPER.initWrapperFixer({settings});") };
                    // the proxy rolls its own cookie on its way; a login cookie it sets is refused (another host)
                    let set = [("Set-Cookie", "AdallomAuth00=rolled-proxy; Domain=.mcas.ms; Path=/"), ("Set-Cookie", "ESTSAUTHPERSISTENT=foreign; Domain=.login.microsoftonline.com; Path=/")];
                    return response(if o.transient { 503 } else { 200 }, &set, &body);
                }
                if host == "login.microsoftonline.com.mcas.ms" {
                    *state.borrow_mut() = (query(&s.url, "state").expect("state"), query(&s.url, "code_challenge").expect("challenge"));
                    assert_eq!((query(&s.url, "McasCtx"), query(&s.url, "McasTsid")), (Some("4".into()), Some("42".into())));
                    assert_eq!(query(&s.url, "redirect_uri").as_deref(), Some("https://teams.microsoft.com/v2/authv2"));
                    return response(302, &[("location", o.redirect.unwrap_or("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?cas=private"))], "");
                }
                if u.path().ends_with("/authorize") {
                    let sessions = o.sessions.clone().unwrap_or(json!([{ "id": "owned-session", "name": "me@example.com", "matchesLoginHint": true, "isSignedIn": true }]));
                    let config = json!({ "sErrorCode": "50058", "fSsoFeasible": true, "arrSessions": sessions, "urlRefresh": "https://login.microsoftonline.com/common/reprocess?ctx=private" });
                    // Microsoft rolls the login cookie
                    let set = [("Set-Cookie", "ESTSAUTHPERSISTENT=rolled-cookie; Domain=.login.microsoftonline.com; Path=/; Secure; HttpOnly")];
                    return response(200, &set, &format!("<script>$Config={config};</script>"));
                }
                if u.path() == "/common/reprocess" {
                    assert_eq!(query(&s.url, "sessionid").as_deref(), Some("owned-session"));
                    assert!(s.header("Cookie").is_some_and(|c| c.contains("ESTSAUTHPERSISTENT=")));
                    if o.credential_form {
                        return response(200, &[], r#"<form method="post" action="https://login.microsoftonline.com/common/login"><input type="password" name="passwd"></form>"#);
                    }
                    return response(200, &[], &format!(r#"<form method="post" action="https://{GATEWAY}/aad_login"><input type="hidden" name="id_token" value="private-token"></form>"#));
                }
                panic!("unexpected test GET {}", s.url);
            }
            let form: std::collections::HashMap<String, String> = form_urlencoded::parse(&s.body).into_owned().collect();
            if host == GATEWAY {
                let st = if o.wrong_state { "wrong".to_string() } else { state.borrow().0.clone() };
                let what = o.callback_error.map_or("code=private-code".to_string(), |e| format!("error={e}"));
                let suffix = if o.plain_callback { "" } else { ".mcas.ms" };
                let location = format!("https://teams.microsoft.com{suffix}/v2/authv2#{what}&state={}", crate::js::encode_uri_component(&st));
                return response(302, &[("location", &location)], "");
            }
            assert_eq!((form["client_id"].as_str(), form["grant_type"].as_str(), form["code"].as_str()), (TEAMS_WEB_CLIENT, "authorization_code", "private-code"));
            assert_eq!(crate::js::base64_url(&sha2::Sha256::digest(form["code_verifier"].as_bytes())), state.borrow().1);
            if host == "login.microsoftonline.com.mcas.ms" {
                assert_eq!((query(&s.url, "McasCtx"), query(&s.url, "McasUserAuth")), (Some("2".into()), Some("private-proxy-auth".into())));
                assert_eq!(s.header("Origin"), Some("https://teams.microsoft.com.mcas.ms"));
                return response(307, &[("location", "https://login.microsoftonline.com/common/oauth2/v2.0/token")], "");
            }
            assert_eq!((host, s.header("Origin")), ("login.microsoftonline.com", Some("null")));
            let access = if o.wrong_tenant { format!("header.{}.signature", crate::js::base64_url(br#"{"tid":"other-tenant"}"#)) } else { "new-access".into() };
            response(200, &[], &json!({ "refresh_token": "new-rt", "access_token": access }).to_string())
        });
        let session = McasSession::with_clock(file.clone(), net.http(), Rc::new(|| 1_000_000));
        (net, file, session)
    }

    fn posted(net: &Net) -> Vec<Seen> {
        net.requests().into_iter().filter(|s| s.method == "POST").collect()
    }

    fn cookie(file: &Path, name: &str) -> String {
        load(file).expect("session").1.iter().find(|c| field(c, "name") == name).map(|c| field(c, "value").to_string()).unwrap_or_default()
    }

    #[test]
    fn renews_through_the_sso_account_selection_and_the_proxy_token_redirect() {
        let (_, file, session) = setup(Options::default());
        assert_eq!(run(session.renew(&stored())).expect("renewed"), StoredToken { refresh_token: "new-rt".into(), expires_on: 87400, ..stored() });
        assert_eq!((cookie(&file, "ESTSAUTHPERSISTENT"), cookie(&file, "AdallomAuth00")), ("rolled-cookie".into(), "rolled-proxy".into()));
    }

    #[test]
    fn does_not_select_an_absent_signed_out_or_ambiguous_account() {
        for sessions in [
            json!([{ "id": "other", "name": "other@example.com", "matchesLoginHint": true, "isSignedIn": true }]),
            json!([{ "id": "owned-session", "name": "me@example.com", "matchesLoginHint": true, "isSignedIn": false }]),
            json!([{ "id": "one", "name": "me@example.com", "matchesLoginHint": true, "isSignedIn": true }, { "id": "two", "name": "me@example.com", "matchesLoginHint": true, "isSignedIn": true }]),
        ] {
            let (net, _, session) = setup(Options { sessions: Some(sessions), ..Default::default() });
            assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
            assert!(posted(&net).is_empty());
        }
    }

    #[test]
    fn rejects_a_wrong_callback_state_and_an_unknown_callback_before_redeeming() {
        let (net, _, session) = setup(Options { wrong_state: true, ..Default::default() });
        assert!(run(session.renew(&stored())).unwrap_err().to_string().to_lowercase().contains("state"));
        assert!(posted(&net).iter().all(|s| !s.url.contains("/token")));
        let (net, _, session) = setup(Options { plain_callback: true, ..Default::default() });
        assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
        assert!(posted(&net).iter().all(|s| !s.url.contains("/token")));
    }

    #[test]
    fn retries_a_temporary_oauth_error_without_switching_to_the_browser() {
        for error in ["server_error", "temporarily_unavailable"] {
            let (net, _, session) = setup(Options { callback_error: Some(error), ..Default::default() });
            assert!(matches!(run(session.renew(&stored())), Err(Fail::Other(_))));
            assert!(posted(&net).iter().all(|s| !s.url.contains("/token")));
        }
    }

    #[test]
    fn falls_back_on_an_integrity_policy_an_unrelated_redirect_or_an_unsupported_answer() {
        let (net, _, session) = setup(Options { integrity: true, ..Default::default() });
        assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
        assert_eq!(net.requests().len(), 1);
        let (net, _, session) = setup(Options { redirect: Some("https://unrelated.test/path?code=private"), ..Default::default() });
        assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
        assert!(net.requests().iter().all(|s| !s.url.contains("unrelated.test")) && posted(&net).is_empty());
        for o in [
            Options { malformed_bootstrap: true, ..Default::default() },
            Options { credential_form: true, ..Default::default() },
            Options { wrong_tenant: true, ..Default::default() },
        ] {
            let (_, _, session) = setup(o);
            assert!(matches!(run(session.renew(&stored())), Err(Fail::NeedsSignIn(_))));
        }
    }

    #[test]
    fn keeps_the_rotated_cookies_on_a_transient_error_without_asking_for_the_owner() {
        let (_, file, session) = setup(Options { transient: true, ..Default::default() });
        assert!(matches!(run(session.renew(&stored())), Err(Fail::Other(_))));
        assert_eq!((cookie(&file, "AdallomAuth00"), cookie(&file, "ESTSAUTHPERSISTENT")), ("rolled-proxy".into(), "private-cookie".into()));
    }

    #[test]
    fn cookie_paths_and_domains() {
        assert_eq!(default_path("/a/b/c"), "/a/b");
        assert_eq!(default_path("/a"), "/");
        let mut jar = Jar { cookies: vec![], now: Rc::new(|| 1_000_000) };
        let url = Url::parse("https://login.microsoftonline.com/common/oauth2/v2.0/authorize").expect("url");
        jar.update(&url, &response(200, &[("Set-Cookie", "a=1; Domain=evil.test"), ("Set-Cookie", "b=2; Path=/common"), ("Set-Cookie", "c=3; Max-Age=0")], ""));
        assert_eq!(jar.cookies.iter().map(|c| field(c, "name")).collect::<Vec<_>>(), ["b"]);
        assert_eq!(jar.header(&url), "b=2");
        assert_eq!(jar.header(&Url::parse("https://login.microsoftonline.com/other").expect("url")), "");
    }
}
