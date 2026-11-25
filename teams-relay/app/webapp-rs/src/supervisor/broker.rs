//! Shared Microsoft accounts of the OE calendar agent, which owns their sign-in (src/supervisor/broker.ts). The light agent
//! of a slot bound to one asks the agent for resource tokens of that account, never a browser or a credential of its
//! own. The web app reaches the agent's list of accounts through the supervisor (GET/POST /directory): the address and the
//! key of the agent stay in this container.

use super::accounts::AccountError;
use super::config::Env;
use crate::agent::http::{Http, send};
use crate::agent::session::MicrosoftSession;
use crate::agent::token::StoredToken;
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub struct SharedBroker {
    pub url: String,
    pub key: String,
}

/// The shared account of a light slot and its chat service region.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub account: String,
    pub region: String,
}

/// The chat service regions of Teams are short lowercase words (amer, emea, apac, in, au, ...).
static REGION: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new("^[a-z][a-z0-9]{1,15}$").expect("fixed"));
static EMAIL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[^\s@/?#]{1,128}@[^\s@/?#]{1,253}$").expect("fixed"));

/// The binding the web app sends with a light start: ?shared=<account>&region=<region>; none without shared.
pub fn binding_of(query: &str) -> Result<Option<Binding>, AccountError> {
    let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let account = super::super::agent::js_trim(&get("shared").unwrap_or_default()).to_lowercase();
    if account.is_empty() {
        return Ok(None);
    }
    let region = get("region").filter(|r| !r.is_empty()).unwrap_or_else(|| "emea".into());
    if !EMAIL.is_match(&account) {
        return Err(AccountError::new(400, "shared: not an account address"));
    }
    if !REGION.is_match(&region) {
        return Err(AccountError::new(400, "region: not a chat service region"));
    }
    Ok(Some(Binding { account, region }))
}

/// The broker variables of a light agent bound to a shared account: all four, none for a slot of its own.
pub fn broker_env(binding: Option<&Binding>, broker: Option<&SharedBroker>) -> Env {
    let (Some(b), Some(k)) = (binding, broker) else { return Env::new() };
    [
        ("SHARED_BROKER_URL", k.url.clone()),
        ("SHARED_BROKER_KEY", k.key.clone()),
        ("SHARED_BROKER_ACCOUNT", b.account.clone()),
        ("SHARED_BROKER_REGION", b.region.clone()),
    ]
    .into_iter()
    .map(|(n, v)| (n.to_string(), v))
    .collect()
}

/// The MCAS proxy of Outlook as the browser holds it: the bootstrap URL of its mail page and every cookie.
pub type Proxy = (String, Vec<Value>);

/// The sign-in of the browser of a slot, read over DevTools (cdp.rs; tests give a fake).
pub trait Capture {
    /// the Microsoft session of `account`: login hint, cookies, user agent; None without one
    fn session(&self, slot: i64, account: &str) -> LocalBoxFuture<'_, Result<Option<MicrosoftSession>, String>>;
    /// the MCAS proxy of Outlook: its bootstrap URL and the cookies; None while the browser is not through it
    fn proxy(&self, slot: i64) -> LocalBoxFuture<'_, Result<Option<Proxy>, String>>;
}

/// Copies the keys of `b` that are set, as an object spread where an undefined value leaves the key out.
fn pick(b: &Map<String, Value>, keys: &[&str]) -> Value {
    Value::Object(keys.iter().filter_map(|k| b.get(*k).map(|v| (k.to_string(), v.clone()))).collect())
}

/// Only the fields the agent's routes take go through: an account and its capabilities, or an operation of it.
fn set_body(body: &Value) -> Result<Value, AccountError> {
    let b = crate::js::obj(body);
    let Some(account) = b.get("account").and_then(Value::as_str) else { return Err(AccountError::new(400, "account: an address")) };
    match b.get("operation").and_then(Value::as_str) {
        Some("signin-cancel") => return Ok(pick(&b, &["operation", "account", "id"])),
        Some("connect") => return Ok(pick(&b, &["operation", "account", "capabilities"])),
        Some("signin-progress") => return Ok(pick(&b, &["operation", "account", "id", "slot", "phase", "url", "error"])),
        Some(op @ ("remove" | "chat")) if EMAIL.is_match(account) => {
            let mut out = Map::new();
            out.insert("account".into(), json!(account));
            out.insert("operation".into(), json!(op));
            if op == "chat"
                && let Some(on) = b.get("on")
            {
                out.insert("on".into(), on.clone());
            }
            return Ok(Value::Object(out));
        }
        _ => {}
    }
    if b.get("enabled") == Some(&Value::Bool(false)) {
        return Ok(json!({ "account": account, "enabled": false }));
    }
    match b.get("capabilities") {
        Some(c @ Value::Array(_)) => Ok(json!({ "account": account, "capabilities": c })),
        _ => Err(AccountError::new(400, "capabilities: a list of mail, calendar, chat")),
    }
}

fn report_body(body: &Value) -> Value {
    let b = crate::js::obj(body);
    let list = |k: &str| b.get(k).filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!([]));
    let mut out = Map::new();
    out.insert("accounts".into(), list("accounts"));
    out.insert("others".into(), list("others"));
    if let Some(r) = b.get("results").filter(|v| v.is_array()) {
        out.insert("results".into(), r.clone());
    }
    Value::Object(out)
}

/// The list of accounts of the OE calendar agent, with the key of this container: the web app never holds it.
pub struct Directory {
    broker: SharedBroker,
    http: Http,
    data_dir: Option<PathBuf>,
    capture: Option<Rc<dyn Capture>>,
}

impl Directory {
    pub fn new(broker: SharedBroker, http: Http, data_dir: Option<PathBuf>, capture: Option<Rc<dyn Capture>>) -> Directory {
        Directory { broker, http, data_dir, capture }
    }

    async fn call(&self, path: &str, body: Option<Value>) -> Result<Value, AccountError> {
        let import = body.as_ref().and_then(|b| b.get("operation")).and_then(Value::as_str) == Some("signin-import");
        let url = url::Url::parse(&self.broker.url).and_then(|u| u.join(path)).map_err(|_| AccountError::new(502, "OE calendar agent not reachable"))?;
        let headers = vec![("Authorization", format!("Bearer {}", self.broker.key)), ("Content-Type", "application/json".into())];
        let (method, data) = match &body {
            Some(b) => ("POST", crate::js::stringify(b).into_bytes()),
            None => ("GET", vec![]),
        };
        let r = send(&self.http, method, url.as_str(), headers, data, if import { 150_000 } else { 45_000 }).await;
        // redirect: "error": a redirect fails like no answer
        let r = match r {
            Ok(r) if !(300..400).contains(&r.status) => r,
            _ => return Err(AccountError::new(502, "OE calendar agent not reachable")),
        };
        let j: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        if r.status == 401 {
            return Err(AccountError::new(502, "OE calendar agent refused the key of the browsers container (TEAMS_BROKER_KEY)"));
        }
        if !r.ok() && r.status == 400 && import {
            let flag = |k: &str| j.get(k) == Some(&Value::Bool(true));
            let errore = j.get("errore").and_then(Value::as_str).filter(|e| !e.is_empty()).unwrap_or("Microsoft sign-in refused");
            return Ok(json!({ "ok": false, "needsProxy": flag("needsProxy"), "needsBrowser": flag("needsBrowser"), "errore": errore }));
        }
        if !r.ok() || !(j.is_object() || j.is_array()) {
            return Err(AccountError::new(502, &format!("OE calendar agent answered {}", r.status)));
        }
        Ok(j)
    }

    pub async fn list(&self) -> Result<Value, AccountError> {
        self.call("/api/auth/shared", None).await
    }

    pub async fn set(&self, body: &Value) -> Result<Value, AccountError> {
        self.call("/api/auth/shared", Some(set_body(body)?)).await
    }

    pub async fn report(&self, body: &Value) -> Result<Value, AccountError> {
        self.call("/api/auth/shared/teams", Some(report_body(body))).await
    }

    /// The guided sign-in of a shared account: the Microsoft session of the slot's browser (else the one saved) handed
    /// to the agent; the MCAS proxy of Outlook too when the agent asks for it.
    pub async fn connect(&self, body: &Value) -> Result<Value, AccountError> {
        let b = crate::js::obj(body);
        let slot = b.get("slot").and_then(Value::as_f64).filter(|s| s.fract() == 0.0 && (1.0..=9999.0).contains(s));
        let account = b.get("account").and_then(Value::as_str).filter(|a| EMAIL.is_match(a));
        let (Some(dir), Some(slot), Some(account)) = (&self.data_dir, slot, account) else { return Err(AccountError::new(400, "Invalid sign-in slot")) };
        let slot = slot as i64;
        let folder = dir.join(slot.to_string());
        let captured = match &self.capture {
            Some(c) => c.session(slot, account).await.ok().flatten(),
            None => None,
        };
        // a saved session of another address (an earlier sign-in of the slot) is no session: the browser asks again
        let saved = || MicrosoftSession::load(&folder.join("session.json")).filter(|s| crate::agent::js_trim(&s.login_hint).to_lowercase() == account);
        let Some(session) = captured.or_else(saved) else {
            return Ok(json!({ "ok": false, "interaction": true }));
        };
        if crate::agent::js_trim(&session.login_hint).to_lowercase() != account {
            return Err(AccountError::new(409, "Microsoft session identity mismatch"));
        }
        let tenant =
            StoredToken::load(&folder.join("light-token.json")).map(|t| t.tenant_id).filter(|t| !t.is_empty()).unwrap_or_else(|| "organizations".into());
        let saved_proxy = crate::agent::mcas::load(&folder.join("mcas-session.json")).map(|(s, cookies)| {
            (s.get("bootstrapUrl").and_then(Value::as_str).unwrap_or("").to_string(), cookies.into_iter().map(Value::Object).collect::<Vec<_>>())
        });
        let imported = |proxy: Option<Proxy>| {
            let mut sessione = Map::new();
            sessione.insert("loginHint".into(), json!(session.login_hint));
            sessione.insert("cookies".into(), Value::Array(session.cookies.iter().cloned().map(Value::Object).collect()));
            sessione.insert("userAgent".into(), json!(session.user_agent));
            if let Some((url, cookies)) = proxy {
                sessione.insert("proxy".into(), json!({ "bootstrapUrl": url, "cookies": cookies }));
            }
            let mut req = Map::new();
            req.insert("operation".into(), json!("signin-import"));
            for (k, v) in &b {
                req.insert(k.clone(), v.clone());
            }
            req.insert("sessione".into(), Value::Object(sessione));
            req.insert("tenant".into(), json!(tenant));
            self.call("/api/auth/shared", Some(Value::Object(req)))
        };
        let flag = |r: &Value, k: &str| r.get(k).is_some_and(crate::js::truthy);
        let mut result = imported(saved_proxy).await?;
        if flag(&result, "needsProxy") || flag(&result, "needsBrowser") {
            let captured = match &self.capture {
                Some(c) => c.proxy(slot).await.ok().flatten(),
                None => None,
            };
            let Some(captured) = captured else { return Ok(json!({ "ok": false, "interaction": true })) };
            result = imported(Some(captured)).await?;
        }
        if flag(&result, "needsBrowser") {
            return Ok(json!({ "ok": false, "interaction": true }));
        }
        if flag(&result, "ok") {
            use std::os::unix::fs::DirBuilderExt;
            let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&folder);
            MicrosoftSession::save(&folder.join("session.json"), &session).map_err(|e| AccountError::new(500, &e.to_string()))?;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{Net, run, temp_dir};

    const MARIO: (&str, &str) = ("mario@example.com", "amer");

    fn mario() -> Binding {
        Binding { account: MARIO.0.into(), region: MARIO.1.into() }
    }

    #[test]
    fn the_binding_of_a_light_start() {
        assert_eq!(binding_of("shared=Mario@Example.com&region=amer").expect("ok"), Some(mario()));
        assert_eq!(binding_of("shared=mario@example.com").expect("ok"), Some(Binding { account: "mario@example.com".into(), region: "emea".into() }));
        assert_eq!(binding_of("").expect("ok"), None);
        for q in ["shared=me&region=emea", "shared=a@example.com/../x&region=emea", "shared=a@example.com&region=emea/../x", "shared=a@example.com&region=EMEA"]
        {
            assert_eq!(binding_of(q).unwrap_err().status, 400, "{q}");
        }
        let k = SharedBroker { url: "http://oe-copilot-agent:8180".into(), key: "secret-key".into() };
        let env = broker_env(Some(&mario()), Some(&k));
        assert_eq!(
            (env["SHARED_BROKER_URL"].as_str(), env["SHARED_BROKER_KEY"].as_str(), env["SHARED_BROKER_ACCOUNT"].as_str(), env["SHARED_BROKER_REGION"].as_str()),
            ("http://oe-copilot-agent:8180", "secret-key", "mario@example.com", "amer")
        );
        assert!(broker_env(Some(&mario()), None).is_empty() && broker_env(None, Some(&k)).is_empty());
    }

    fn directory(net: &Net, data: Option<PathBuf>, capture: Option<Rc<dyn Capture>>) -> Directory {
        Directory::new(SharedBroker { url: "http://oe-copilot-agent:8180".into(), key: "secret-key".into() }, net.http(), data, capture)
    }

    #[test]
    fn passes_list_change_and_report_with_the_key_of_this_container_only_the_fields_the_routes_take() {
        run(async {
            let net = Net::new();
            net.answer(|_| (200, r#"{"ok":true,"accounts":[]}"#.into()));
            let d = directory(&net, None, None);
            assert_eq!(d.list().await.expect("list"), json!({"ok": true, "accounts": []}));
            d.set(&json!({"account": "mario@example.com", "capabilities": ["chat"], "tenantId": "evil"})).await.expect("set");
            d.set(&json!({"account": "mario@example.com", "enabled": false})).await.expect("set");
            d.set(&json!({"operation": "signin-cancel", "account": "failed@example.com", "id": "failed-signin"})).await.expect("cancel");
            d.report(&json!({"accounts": [{"account": "mario@example.com", "slot": 2, "state": "ok"}], "others": [], "key": "x"})).await.expect("report");
            let seen: Vec<(String, String, Option<Value>)> = net
                .requests()
                .iter()
                .map(|s| (s.url.clone(), s.header("Authorization").unwrap_or("").to_string(), (!s.body.is_empty()).then(|| s.json())))
                .collect();
            let shared = "http://oe-copilot-agent:8180/api/auth/shared".to_string();
            let auth = "Bearer secret-key".to_string();
            assert_eq!(
                seen,
                [
                    (shared.clone(), auth.clone(), None),
                    (shared.clone(), auth.clone(), Some(json!({"account": "mario@example.com", "capabilities": ["chat"]}))),
                    (shared.clone(), auth.clone(), Some(json!({"account": "mario@example.com", "enabled": false}))),
                    (shared, auth.clone(), Some(json!({"operation": "signin-cancel", "account": "failed@example.com", "id": "failed-signin"}))),
                    (
                        "http://oe-copilot-agent:8180/api/auth/shared/teams".into(),
                        auth,
                        Some(json!({"accounts": [{"account": "mario@example.com", "slot": 2, "state": "ok"}], "others": []}))
                    ),
                ]
            );
            assert_eq!(d.set(&json!({"account": "x@example.com"})).await.unwrap_err().status, 400);
        });
    }

    #[test]
    fn an_agent_that_refuses_the_key_or_does_not_answer_is_a_502_without_the_key() {
        run(async {
            let net = Net::new();
            net.answer(|_| (401, r#"{"errore":"non autorizzato"}"#.into()));
            let d = directory(&net, None, None);
            let e = d.list().await.unwrap_err();
            assert!(e.status == 502 && e.message.contains("TEAMS_BROKER_KEY"));
            net.fail("fetch failed");
            let e = d.list().await.unwrap_err();
            assert!(e.message.contains("not reachable") && !e.message.contains("secret-key"));
            net.answer(|_| (302, String::new()));
            assert!(d.list().await.unwrap_err().message.contains("not reachable"));
        });
    }

    struct FakeCapture {
        session: Option<MicrosoftSession>,
        proxy: Option<Proxy>,
    }

    impl Capture for FakeCapture {
        fn session(&self, _: i64, _: &str) -> LocalBoxFuture<'_, Result<Option<MicrosoftSession>, String>> {
            Box::pin(async move { Ok(self.session.clone()) })
        }
        fn proxy(&self, _: i64) -> LocalBoxFuture<'_, Result<Option<Proxy>, String>> {
            Box::pin(async move { Ok(self.proxy.clone()) })
        }
    }

    fn session(hint: &str) -> MicrosoftSession {
        let cookie = json!({"name": "ESTSAUTHPERSISTENT", "value": "c", "domain": ".login.microsoftonline.com", "path": "/", "expires": 9_999_999_999i64, "httpOnly": true, "secure": true, "sameSite": "None"});
        MicrosoftSession { user_agent: "ua".into(), login_hint: hint.into(), cookies: vec![cookie.as_object().cloned().expect("o")] }
    }

    #[test]
    fn the_guided_sign_in_hands_the_browser_session_then_the_proxy_and_saves_the_session_once_accepted() {
        run(async {
            let data = temp_dir();
            let net = Net::new();
            let calls = Rc::new(std::cell::Cell::new(0));
            let c = calls.clone();
            net.answer(move |_| {
                c.set(c.get() + 1);
                (
                    if c.get() == 1 { 400 } else { 200 },
                    if c.get() == 1 { r#"{"needsProxy":true,"errore":"proxy"}"#.into() } else { r#"{"ok":true,"route":"/x"}"#.into() },
                )
            });
            let capture: Rc<dyn Capture> = Rc::new(FakeCapture {
                session: Some(session("Mario@example.com ")),
                proxy: Some(("https://mcas-proxyweb.mcas.ms/js-bootstrap.js".into(), vec![json!({"name": "AdallomAuth00"})])),
            });
            let d = directory(&net, Some(data.clone()), Some(capture));
            let r = d.connect(&json!({"id": "s1", "account": "mario@example.com", "slot": 4})).await.expect("connect");
            assert_eq!(r, json!({"ok": true, "route": "/x"}));
            let sent: Vec<Value> = net.requests().iter().map(|s| s.json()).collect();
            assert_eq!(
                (sent[0]["operation"].clone(), sent[0]["id"].clone(), sent[0]["slot"].clone(), sent[0]["tenant"].clone()),
                (json!("signin-import"), json!("s1"), json!(4), json!("organizations"))
            );
            assert!(sent[0]["sessione"].get("proxy").is_none());
            assert_eq!(sent[1]["sessione"]["proxy"]["bootstrapUrl"], "https://mcas-proxyweb.mcas.ms/js-bootstrap.js");
            assert_eq!(MicrosoftSession::load(&data.join("4").join("session.json")).expect("saved").login_hint, "Mario@example.com ");

            let d = directory(&net, Some(data.clone()), Some(Rc::new(FakeCapture { session: Some(session("anna@example.com")), proxy: None })));
            assert_eq!(d.connect(&json!({"account": "mario@example.com", "slot": 4})).await.unwrap_err().status, 409);
            let d = directory(&net, Some(temp_dir()), Some(Rc::new(FakeCapture { session: None, proxy: None })));
            assert_eq!(d.connect(&json!({"account": "mario@example.com", "slot": 4})).await.expect("none"), json!({"ok": false, "interaction": true}));
            // a session saved by an earlier sign-in of another address: the browser sign-in, not a dead end
            let stale = temp_dir();
            std::fs::create_dir_all(stale.join("4")).expect("dir");
            MicrosoftSession::save(&stale.join("4").join("session.json"), &session("anna@example.com")).expect("saved");
            let before = net.requests().len();
            let d = directory(&net, Some(stale), Some(Rc::new(FakeCapture { session: None, proxy: None })));
            assert_eq!(d.connect(&json!({"account": "mario@example.com", "slot": 4})).await.expect("stale"), json!({"ok": false, "interaction": true}));
            assert_eq!(net.requests().len(), before);
            for bad in
                [json!({"account": "mario@example.com", "slot": 0}), json!({"account": "mario@example.com", "slot": 1.5}), json!({"account": "me", "slot": 2})]
            {
                assert_eq!(d.connect(&bad).await.unwrap_err().status, 400);
            }
        });
    }
}
