//! Access tokens of a light account (src/agent/light/token.ts): redeemed from the MSAL refresh token the account's
//! browser run captured, as Teams web redeems it (single-page app, so with its Origin). Microsoft rolls the refresh
//! token at each renewal, the expiry stays: a refresh token of a single-page app holds 24 h from the sign-in.

use super::config::Broker;
use super::http::{Http, json_or_empty, send};
use super::{Fail, R, now_ms};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const IC3: &str = "https://ic3.teams.office.com/.default";
pub const CHATSVCAGG: &str = "https://chatsvcagg.teams.microsoft.com/.default";
pub const SPACES: &str = "https://api.spaces.skype.com/.default";
pub const PRESENCE: &str = "https://presence.teams.microsoft.com/.default";

/// A token renewed this long before it expires (ms).
const MARGIN_MS: i64 = 5 * 60_000;

/// region: of the chat service, as the cookies of the browser said it; expires_on: Unix seconds the refresh token ends.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredToken {
    pub refresh_token: String,
    pub client_id: String,
    pub tenant_id: String,
    pub region: String,
    pub expires_on: i64,
}

impl StoredToken {
    pub fn json(&self) -> Value {
        json!({ "refreshToken": self.refresh_token, "clientId": self.client_id, "tenantId": self.tenant_id, "region": self.region, "expiresOn": self.expires_on })
    }

    /// A credential: readable by its owner only.
    pub fn save(&self, file: &Path) -> R<()> {
        super::write_private(file, crate::js::stringify(&self.json()).as_bytes()).map_err(|e| Fail::Other(e.to_string()))?;
        super::chmod_private(file).map_err(|e| Fail::Other(e.to_string()))
    }

    pub fn load(file: &Path) -> Option<StoredToken> {
        let v: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let s = |k: &str| v.get(k).and_then(Value::as_str);
        let refresh = s("refreshToken").filter(|t| !t.is_empty())?;
        let region = s("region").filter(|r| !r.is_empty()).unwrap_or("emea");
        let expires = crate::js::number(v.get("expiresOn"));
        Some(StoredToken {
            refresh_token: refresh.to_string(),
            client_id: s("clientId")?.to_string(),
            tenant_id: s("tenantId")?.to_string(),
            region: region.to_string(),
            expires_on: if expires.is_finite() && expires != 0.0 { expires as i64 } else { 0 },
        })
    }
}

/// When an access token ends, from its exp claim (ms); unreadable, it is not kept.
fn expiry_of(token: &str, now: i64) -> i64 {
    let exp = crate::js::number(super::claims(token).get("exp"));
    if exp.is_finite() { (exp * 1000.0) as i64 } else { now }
}

pub struct Token {
    file: PathBuf,
    now: Rc<dyn Fn() -> i64>,
    broker: Option<Broker>,
    http: Http,
    cache: RefCell<HashMap<String, (String, i64)>>,
}

impl Token {
    pub fn new(file: PathBuf, broker: Option<Broker>, http: Http) -> Self {
        Token { file, now: Rc::new(now_ms), broker, http, cache: RefCell::new(HashMap::new()) }
    }

    #[cfg(test)]
    pub fn with_clock(file: PathBuf, broker: Option<Broker>, http: Http, now: Rc<dyn Fn() -> i64>) -> Self {
        Token { file, now, broker, http, cache: RefCell::new(HashMap::new()) }
    }

    pub fn broker_backed(&self) -> bool {
        self.broker.is_some()
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Region of the chat service: of the shared account's mapping, else the one the browser run captured.
    pub fn region(&self) -> String {
        if let Some(b) = &self.broker {
            return b.region.clone();
        }
        self.stored().map(|s| s.region).filter(|r| !r.is_empty()).unwrap_or_else(|| "emea".into())
    }

    pub fn stored(&self) -> Option<StoredToken> {
        if self.broker.is_some() {
            return None;
        }
        StoredToken::load(&self.file)
    }

    /// The next access tokens are redeemed again.
    pub fn renew_all(&self) {
        self.cache.borrow_mut().clear();
    }

    fn cached(&self, scope: &str) -> Option<(String, i64)> {
        self.cache.borrow().get(scope).cloned()
    }

    pub async fn access_token(&self, scope: &str) -> R<String> {
        let hit = self.cached(scope);
        let now = (self.now)();
        if let Some(b) = &self.broker {
            if let Some((t, at)) = &hit
                && now < at - MARGIN_MS
            {
                return Ok(t.clone());
            }
            let resource = match scope {
                CHATSVCAGG => "csa",
                IC3 => "ic3",
                SPACES => "spaces",
                PRESENCE => "presence",
                _ => return Err(Fail::SignedOut("unsupported broker resource".into())),
            };
            let mut url = url::Url::parse(&b.url).and_then(|u| u.join("/api/token-resource")).map_err(|e| Fail::Other(e.to_string()))?;
            url.query_pairs_mut().append_pair("account", &b.account).append_pair("resource", resource);
            let answer = send(&self.http, "GET", url.as_str(), vec![("Authorization", format!("Bearer {}", b.key))], vec![], 20_000).await;
            // redirect: "error": a redirect fails like no answer
            let r = match answer {
                Ok(r) if !(300..400).contains(&r.status) => r,
                _ => {
                    // the calendar agent restarting: a token still cached keeps working until it ends
                    if let Some((t, at)) = hit
                        && (self.now)() < at
                    {
                        return Ok(t);
                    }
                    return Err(Fail::Other("shared broker not reachable".into()));
                }
            };
            let token = json_or_empty(&r).get("token").and_then(Value::as_str).unwrap_or("").to_string();
            if !r.ok() || token.is_empty() {
                return Err(Fail::SignedOut("shared broker unavailable".into()));
            }
            // reused until shortly before it ends: the agent caches it too, this saves a call per request
            self.cache.borrow_mut().insert(scope.to_string(), (token.clone(), expiry_of(&token, (self.now)())));
            return Ok(token);
        }
        if let Some((t, at)) = &hit
            && now < at - MARGIN_MS
        {
            return Ok(t.clone());
        }
        let Some(stored) = self.stored() else { return Err(Fail::SignedOut("no refresh token stored".into())) };
        let body = crate::js::search_params(&[
            ("client_id", &stored.client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", &stored.refresh_token),
            ("scope", &format!("{scope} openid profile offline_access")),
            ("client_info", "1"),
        ]);
        let url = format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", stored.tenant_id);
        let headers = vec![("Content-Type", "application/x-www-form-urlencoded".to_string()), ("Origin", "https://teams.microsoft.com".to_string())];
        let r = send(&self.http, "POST", &url, headers, body.into_bytes(), 15_000).await?;
        let j = json_or_empty(&r);
        let access = j.get("access_token").and_then(Value::as_str).unwrap_or("");
        if !r.ok() || access.is_empty() {
            // a token still cached keeps working until it ends
            if let Some((t, at)) = hit
                && (self.now)() < at
            {
                return Ok(t);
            }
            let error = j.get("error").and_then(Value::as_str).unwrap_or("");
            if error == "invalid_grant" {
                return Err(Fail::SignedOut("refresh token refused".into()));
            }
            return Err(Fail::Other(format!("token endpoint {} {error}", r.status).trim().to_string()));
        }
        if let Some(rolled) = j.get("refresh_token").and_then(Value::as_str) {
            StoredToken { refresh_token: rolled.to_string(), ..stored }.save(&self.file)?;
        }
        let expires_in = j.get("expires_in").and_then(Value::as_f64).unwrap_or(3600.0);
        self.cache.borrow_mut().insert(scope.to_string(), (access.to_string(), (self.now)() + (expires_in * 1000.0) as i64));
        Ok(access.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{Net, run, temp_dir};
    use std::cell::Cell;

    fn stored() -> StoredToken {
        StoredToken { refresh_token: "rt-1".into(), client_id: "client-1".into(), tenant_id: "tenant-1".into(), region: "emea".into(), expires_on: 2_000 }
    }

    fn granted(net: &Net, access: &'static str, rolled: &'static str) {
        net.answer(move |_| (200, json!({ "access_token": access, "refresh_token": rolled, "expires_in": 4771 }).to_string()));
    }

    fn setup(net: &Net, now: Rc<Cell<i64>>) -> (PathBuf, Token) {
        let file = temp_dir().join("light-token.json");
        stored().save(&file).expect("saved");
        let clock = now.clone();
        (file.clone(), Token::with_clock(file, None, net.http(), Rc::new(move || clock.get())))
    }

    #[test]
    fn redeems_the_refresh_token_at_the_tenant_token_endpoint_as_teams_web_does() {
        let net = Net::new();
        granted(&net, "at-1", "rt-2");
        let (_, token) = setup(&net, Rc::new(Cell::new(1_000_000)));
        assert_eq!(run(token.access_token(IC3)).expect("token"), "at-1");
        let r = &net.requests()[0];
        assert_eq!(r.url, "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token");
        assert_eq!(r.header("Origin"), Some("https://teams.microsoft.com"));
        let body: HashMap<String, String> = form_urlencoded::parse(&r.body).into_owned().collect();
        assert_eq!((body["grant_type"].as_str(), body["refresh_token"].as_str(), body["client_id"].as_str()), ("refresh_token", "rt-1", "client-1"));
        assert!(body["scope"].contains(IC3) && body["scope"].contains("offline_access"));
    }

    #[test]
    fn serves_the_cached_token_until_5_minutes_before_it_expires_then_renews() {
        let net = Net::new();
        granted(&net, "at-1", "rt-2");
        let now = Rc::new(Cell::new(1_000_000));
        let (_, token) = setup(&net, now.clone());
        run(token.access_token(IC3)).expect("token");
        now.set(now.get() + 4_771_000 - 6 * 60_000);
        run(token.access_token(IC3)).expect("token");
        assert_eq!(net.requests().len(), 1);
        now.set(now.get() + 2 * 60_000);
        run(token.access_token(IC3)).expect("token");
        assert_eq!(net.requests().len(), 2);
        // again after renew_all, though the cached one still holds
        token.renew_all();
        run(token.access_token(IC3)).expect("token");
        assert_eq!(net.requests().len(), 3);
    }

    #[test]
    fn keeps_the_rolled_refresh_token_with_the_expiry_of_the_first_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let net = Net::new();
        granted(&net, "at-1", "rt-rolled");
        let (file, token) = setup(&net, Rc::new(Cell::new(1_000_000)));
        run(token.access_token(IC3)).expect("token");
        let saved = StoredToken::load(&file).expect("saved");
        assert_eq!((saved.refresh_token.as_str(), saved.expires_on), ("rt-rolled", 2_000));
        assert_eq!(std::fs::metadata(&file).expect("file").permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn serves_a_token_still_valid_when_microsoft_refuses_then_is_signed_out() {
        let net = Net::new();
        granted(&net, "at-1", "rt-2");
        let now = Rc::new(Cell::new(1_000_000));
        let (_, token) = setup(&net, now.clone());
        run(token.access_token(IC3)).expect("token");
        now.set(now.get() + 4_771_000 - 60_000);
        net.answer(|_| (400, r#"{"error":"invalid_grant"}"#.into()));
        assert_eq!(run(token.access_token(IC3)).expect("cached"), "at-1");
        now.set(now.get() + 2 * 60_000);
        assert!(matches!(run(token.access_token(IC3)), Err(Fail::SignedOut(_))));
        let none = Token::new(temp_dir().join("light-token.json"), None, net.http());
        assert!(matches!(run(none.access_token(IC3)), Err(Fail::SignedOut(_))));
    }

    #[test]
    fn never_puts_a_token_in_an_error() {
        let net = Net::new();
        net.answer(|_| (500, r#"{"error":"server_error"}"#.into()));
        let (_, token) = setup(&net, Rc::new(Cell::new(1_000_000)));
        let e = run(token.access_token(IC3)).unwrap_err();
        assert_eq!(e.to_string(), "token endpoint 500 server_error");
    }

    fn broker() -> Broker {
        Broker { url: "http://agent:8765".into(), key: "key".into(), account: "user".into(), region: "emea".into() }
    }

    #[test]
    fn a_broker_maps_scopes_never_reads_a_local_credential_and_fails_closed() {
        let net = Net::new();
        net.answer(|_| (200, r#"{"token":"resource-token"}"#.into()));
        let token = Token::new("missing-local-file".into(), Some(broker()), net.http());
        assert!(token.stored().is_none() && token.broker_backed());
        assert_eq!(run(token.access_token(CHATSVCAGG)).expect("token"), "resource-token");
        run(token.access_token(IC3)).expect("token");
        let urls: Vec<url::Url> = net.requests().iter().map(|r| url::Url::parse(&r.url).expect("url")).collect();
        let resource = |u: &url::Url| u.query_pairs().find(|(k, _)| k == "resource").map(|(_, v)| v.to_string());
        assert_eq!(urls.iter().map(resource).collect::<Vec<_>>(), [Some("csa".into()), Some("ic3".into())]);
        assert_eq!((urls[0].path(), net.requests()[0].header("Authorization")), ("/api/token-resource", Some("Bearer key")));

        let net = Net::new();
        net.answer(|_| (403, "{}".into()));
        let token = Token::new("missing".into(), Some(broker()), net.http());
        assert_eq!(run(token.access_token(CHATSVCAGG)).unwrap_err().to_string(), "shared broker unavailable");
        assert!(run(token.access_token("https://evil.invalid/.default")).unwrap_err().to_string().contains("unsupported"));
        assert_eq!(net.requests().len(), 1);
        assert_eq!(Token::new("missing".into(), Some(Broker { region: "amer".into(), ..broker() }), net.http()).region(), "amer");
        assert_eq!(Token::new("missing".into(), None, net.http()).region(), "emea");
    }

    #[test]
    fn reuses_a_broker_token_until_shortly_before_it_ends_and_a_cached_one_while_the_agent_restarts() {
        let exp = 2_000_000_000i64;
        let jwt = format!("x.{}.x", crate::js::base64_url(json!({ "exp": exp }).to_string().as_bytes()));
        let net = Net::new();
        let answer = jwt.clone();
        net.answer(move |_| (200, json!({ "token": answer }).to_string()));
        let now = Rc::new(Cell::new(exp * 1000 - 3_600_000));
        let clock = now.clone();
        let token = Token::with_clock("missing".into(), Some(broker()), net.http(), Rc::new(move || clock.get()));
        assert_eq!(run(token.access_token(CHATSVCAGG)).expect("token"), jwt);
        assert_eq!(run(token.access_token(CHATSVCAGG)).expect("token"), jwt);
        assert_eq!(net.requests().len(), 1);
        now.set(exp * 1000 - 60_000);
        net.fail("fetch failed");
        assert_eq!(run(token.access_token(CHATSVCAGG)).expect("cached"), jwt);
        now.set(exp * 1000 + 1);
        assert!(run(token.access_token(CHATSVCAGG)).unwrap_err().to_string().contains("not reachable"));
    }
}
