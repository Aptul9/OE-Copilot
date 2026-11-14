//! The Microsoft sign-in of a slot's browser read over its DevTools port (src/supervisor/microsoft-session.ts, which used
//! Playwright's connectOverCDP): the login cookies and the user agent for a shared account, and the MCAS proxy of
//! Outlook, over the DevTools client of crate::cdp.

use super::broker::{Capture, Proxy};
use crate::agent::http::{Http, send};
use crate::agent::session::MicrosoftSession;
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use url::Url;

use crate::cdp::{Cdp, for_url, set_cookies};

/// A tab the supervisor opened on Outlook for a sign-in through the proxy, kept 30 minutes between asks.
struct Owned {
    cdp: Rc<Cdp>,
    target: String,
    timer: tokio::task::JoinHandle<()>,
}

/// The capture of the real browsers: account N's DevTools listen on base_port + N.
pub struct Browsers {
    base_port: u16,
    http: Http,
    owned: Rc<RefCell<HashMap<u16, Owned>>>,
}

impl Browsers {
    pub fn new(base_port: u16, http: Http) -> Rc<Browsers> {
        Rc::new(Browsers { base_port, http, owned: Rc::default() })
    }

    fn port(&self, slot: i64) -> Result<u16, String> {
        u16::try_from(i64::from(self.base_port) + slot).map_err(|_| "no DevTools port for this slot".to_string())
    }

    /// Microsoft SSO even when Teams is not licensed or has not finished loading.
    async fn session_of(&self, slot: i64, account: &str) -> Result<Option<MicrosoftSession>, String> {
        let cdp = Cdp::connect(self.port(slot)?).await?;
        let Some((page, _)) = cdp.pages().await?.into_iter().next() else { return Ok(None) };
        let login = Url::parse("https://login.microsoftonline.com").expect("fixed");
        let domain = regex::Regex::new(r"(?i)^\.?login\.microsoftonline\.com$").expect("fixed");
        let cookies: Vec<Map<String, Value>> = cdp
            .cookies()
            .await?
            .into_iter()
            .filter(|c| for_url(c, &login) && domain.is_match(c.get("domain").and_then(Value::as_str).unwrap_or("")))
            .filter_map(|c| c.as_object().cloned())
            .collect();
        let estsauth = regex::Regex::new("^ESTSAUTH(?:PERSISTENT|LIGHT)?$").expect("fixed");
        if !cookies.iter().any(|c| estsauth.is_match(c.get("name").and_then(Value::as_str).unwrap_or(""))) {
            return Ok(None);
        }
        let ua = cdp.evaluate(&page, "navigator.userAgent").await?;
        Ok(Some(MicrosoftSession { user_agent: crate::js::string(Some(&ua)), login_hint: account.into(), cookies }))
    }

    async fn release(owned: &RefCell<HashMap<u16, Owned>>, port: u16) {
        let Some(o) = owned.borrow_mut().remove(&port) else { return };
        o.timer.abort();
        let _ = o.cdp.send("Target.closeTarget", json!({ "targetId": o.target }), None).await;
    }

    /// The tab of `target` on `url`, waited for until its document is past loading (30 s, failures ignored).
    async fn navigate(cdp: &Cdp, target: &str, url: &str) {
        let wait = async {
            let attached = cdp.send("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await?;
            let session = attached.get("sessionId").and_then(Value::as_str).ok_or("no session")?.to_string();
            cdp.send("Page.navigate", json!({ "url": url }), Some(&session)).await?;
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let r =
                    cdp.send("Runtime.evaluate", json!({ "expression": "[location.href, document.readyState]", "returnByValue": true }), Some(&session)).await;
                let state = r.ok().and_then(|r| r.get("result").and_then(|x| x.get("value")).cloned()).unwrap_or(Value::Null);
                let href = state.get(0).and_then(Value::as_str).unwrap_or("");
                if href.starts_with("https://") && state.get(1).and_then(Value::as_str).is_some_and(|s| s != "loading") {
                    break;
                }
            }
            let _ = cdp.send("Target.detachFromTarget", json!({ "sessionId": session }), None).await;
            Ok::<(), String>(())
        };
        let _ = tokio::time::timeout(Duration::from_secs(30), wait).await;
    }

    /// The MCAS proxy of Outlook, signed in through the browser: the bootstrap script of its mail page and every cookie.
    async fn proxy_of(&self, slot: i64) -> Result<Option<Proxy>, String> {
        let port = self.port(slot)?;
        let stale = self.owned.borrow().get(&port).map(|o| (o.cdp.clone(), o.target.clone()));
        let mut owned = None;
        if let Some((cdp, target)) = stale {
            if cdp.connected() && cdp.url_of(&target).await.is_ok() {
                owned = Some((cdp, target));
            } else {
                Self::release(&self.owned, port).await;
            }
        }
        let cdp = match &owned {
            Some((cdp, _)) => cdp.clone(),
            None => Cdp::connect(port).await?,
        };
        let outlook = regex::Regex::new(r"^https://outlook\.cloud\.microsoft(?:\.mcas\.ms)?/").expect("fixed");
        let existing = if owned.is_some() { None } else { cdp.pages().await?.into_iter().find(|(_, u)| outlook.is_match(u)).map(|(t, _)| t) };
        let page = match (&owned, &existing) {
            (Some((_, t)), _) => t.clone(),
            (None, Some(t)) => t.clone(),
            (None, None) => {
                let r = cdp.send("Target.createTarget", json!({ "url": "about:blank" }), None).await?;
                let target = r.get("targetId").and_then(Value::as_str).ok_or("no target")?.to_string();
                let o = self.owned.clone();
                let timer = tokio::task::spawn_local(async move {
                    tokio::time::sleep(Duration::from_secs(1800)).await;
                    Self::release(&o, port).await;
                });
                self.owned.borrow_mut().insert(port, Owned { cdp: cdp.clone(), target: target.clone(), timer });
                Self::navigate(&cdp, &target, "https://outlook.cloud.microsoft/mail/").await;
                target
            }
        };
        let here = Url::parse(&cdp.url_of(&page).await?).map_err(|e| e.to_string())?;
        let origin = here.origin().ascii_serialization();
        if origin != "https://outlook.cloud.microsoft.mcas.ms" {
            return Ok(None);
        }
        let mail = Url::parse(&format!("{origin}/mail/")).expect("fixed");
        let ua = crate::js::string(Some(&cdp.evaluate(&page, "navigator.userAgent").await?));
        let cookie = cdp
            .cookies()
            .await?
            .iter()
            .filter(|c| for_url(c, &mail))
            .map(|c| format!("{}={}", c["name"].as_str().unwrap_or(""), c["value"].as_str().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("; ");
        let mut headers = vec![("User-Agent", ua), ("Accept", "*/*".to_string())];
        if !cookie.is_empty() {
            headers.push(("Cookie", cookie));
        }
        let r = send(&self.http, "GET", mail.as_str(), headers, vec![], 15_000).await.map_err(|e| e.to_string())?;
        let set = set_cookies(&mail, &r);
        if !set.is_empty() {
            let _ = cdp.send("Storage.setCookies", json!({ "cookies": set }), None).await;
        }
        if !r.ok() || r.body.len() > 2 * 1024 * 1024 {
            return Ok(None);
        }
        let Some(url) = crate::agent::forms::bootstrap_url(&String::from_utf8_lossy(&r.body), &origin)? else { return Ok(None) };
        let captured = (url, cdp.cookies().await?);
        if self.owned.borrow().contains_key(&port) {
            Self::release(&self.owned, port).await;
        }
        Ok(Some(captured))
    }
}

impl Capture for Browsers {
    fn session(&self, slot: i64, account: &str) -> LocalBoxFuture<'_, Result<Option<MicrosoftSession>, String>> {
        let account = account.to_string();
        Box::pin(async move { self.session_of(slot, &account).await })
    }

    fn proxy(&self, slot: i64) -> LocalBoxFuture<'_, Result<Option<Proxy>, String>> {
        Box::pin(self.proxy_of(slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::playwright_cookie;

    #[test]
    fn cookies_in_the_shape_and_by_the_rules_of_playwright() {
        let raw = json!({"name": "ESTSAUTH", "value": "v", "domain": ".login.microsoftonline.com", "path": "/", "expires": -1, "size": 9, "httpOnly": true, "secure": true, "session": true, "priority": "Medium", "sameParty": false, "sourceScheme": "Secure", "sourcePort": 443});
        assert_eq!(
            playwright_cookie(&raw),
            json!({"name": "ESTSAUTH", "value": "v", "domain": ".login.microsoftonline.com", "path": "/", "expires": -1, "httpOnly": true, "secure": true, "sameSite": "Lax"})
        );
        let part = json!({"name": "a", "value": "b", "domain": "x.test", "path": "/", "expires": 1, "httpOnly": false, "secure": true, "sameSite": "None", "partitionKey": {"topLevelSite": "https://top.test", "hasCrossSiteAncestor": false}});
        assert_eq!(playwright_cookie(&part)["partitionKey"], "https://top.test");
        assert_eq!(playwright_cookie(&part)["_crHasCrossSiteAncestor"], false);
        let login = Url::parse("https://login.microsoftonline.com").expect("url");
        let c = |domain: &str, path: &str| json!({"domain": domain, "path": path, "secure": true});
        assert!(for_url(&c(".login.microsoftonline.com", "/"), &login));
        assert!(for_url(&c("login.microsoftonline.com", "/"), &login));
        assert!(for_url(&c(".microsoftonline.com", "/"), &login));
        assert!(!for_url(&c(".login.microsoftonline.com", "/common"), &login));
        assert!(!for_url(&c("other.test", "/"), &login));
        assert!(!for_url(&c("x.test", "/"), &Url::parse("http://x.test/").expect("url")));
        assert!(for_url(&c("localhost", "/"), &Url::parse("http://localhost/").expect("url")));
    }

    /// A real browser where CDP_CHROME names one: its login cookies and user agent read as the Node supervisor read
    /// them with Playwright, and the Outlook tab opened, navigated and kept for the next ask.
    #[test]
    fn captures_the_session_of_a_real_browser() {
        let Some(chrome) = std::env::var("CDP_CHROME").ok().filter(|c| !c.is_empty()) else {
            eprintln!("CDP_CHROME not set: the browser capture test is skipped");
            return;
        };
        crate::agent::testing::run(async {
            let profile = crate::agent::testing::temp_dir();
            let base = 9400 + (std::process::id() % 400) as u16;
            let mut browser = tokio::process::Command::new(&chrome)
                .args(["--headless=new", "--no-sandbox", "--no-first-run", "--user-agent=TeamsRelayTest/1.0"])
                .arg(format!("--remote-debugging-port={}", base + 1))
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg("about:blank")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .expect("browser");
            let mut cdp = None;
            for _ in 0..100 {
                if let Ok(c) = Cdp::connect(base + 1).await {
                    cdp = Some(c);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let cdp = cdp.expect("DevTools up");
            let b = Browsers::new(base, crate::agent::http::slot_http());
            assert_eq!(b.session(1, "me@example.com").await.expect("read"), None);
            // within the 400 days Chrome keeps a cookie
            let far = crate::web::now_s() + 30 * 86400;
            let cookies = json!([
                { "name": "ESTSAUTHPERSISTENT", "value": "p", "domain": ".login.microsoftonline.com", "path": "/", "secure": true, "httpOnly": true, "sameSite": "None", "expires": far },
                { "name": "buid", "value": "h", "url": "https://login.microsoftonline.com/", "secure": true },
                { "name": "deep", "value": "d", "domain": ".login.microsoftonline.com", "path": "/common", "secure": true },
                { "name": "wide", "value": "w", "domain": ".microsoftonline.com", "path": "/", "secure": true },
                { "name": "other", "value": "o", "domain": ".example.com", "path": "/" },
            ]);
            cdp.send("Storage.setCookies", json!({ "cookies": cookies }), None).await.expect("cookies");
            let s = b.session(1, "me@example.com").await.expect("read").expect("session");
            assert_eq!((s.user_agent.as_str(), s.login_hint.as_str()), ("TeamsRelayTest/1.0", "me@example.com"));
            let mut names: Vec<&str> = s.cookies.iter().map(|c| c["name"].as_str().unwrap_or("")).collect();
            names.sort();
            assert_eq!(names, ["ESTSAUTHPERSISTENT", "buid"]);
            let mut p = s.cookies.iter().find(|c| c["name"] == "ESTSAUTHPERSISTENT").cloned().expect("cookie");
            assert!((p["expires"].as_f64().expect("expires") - far as f64).abs() < 1.0);
            p.insert("expires".into(), json!(far));
            assert_eq!(
                Value::Object(p),
                json!({ "name": "ESTSAUTHPERSISTENT", "value": "p", "domain": ".login.microsoftonline.com", "path": "/", "expires": far, "httpOnly": true, "secure": true, "sameSite": "None" })
            );
            let h = s.cookies.iter().find(|c| c["name"] == "buid").expect("cookie");
            assert_eq!((h["domain"].clone(), h["expires"].clone(), h["sameSite"].clone()), (json!("login.microsoftonline.com"), json!(-1), json!("Lax")));
            // the session saved as the agent reads it back
            let file = profile.join("session.json");
            MicrosoftSession::save(&file, &s).expect("saved");
            assert_eq!(MicrosoftSession::load(&file).expect("loaded").cookies.len(), 2);

            // Outlook outside the proxy: a tab of the supervisor, kept for the next ask, gone after a release
            let before = cdp.pages().await.expect("pages").len();
            assert_eq!(b.proxy(1).await.expect("proxy"), None);
            let port = base + 1;
            let target = b.owned.borrow().get(&port).map(|o| o.target.clone()).expect("owned tab");
            assert_eq!(cdp.pages().await.expect("pages").len(), before + 1);
            assert!(cdp.url_of(&target).await.expect("url").starts_with("https://"), "navigated");
            assert_eq!(b.proxy(1).await.expect("proxy"), None);
            assert_eq!(cdp.pages().await.expect("pages").len(), before + 1);
            Browsers::release(&b.owned, port).await;
            assert_eq!(cdp.pages().await.expect("pages").len(), before);
            let _ = browser.kill().await;
        });
    }

    #[test]
    fn the_cookies_an_answer_sets_go_to_the_browser() {
        let url = Url::parse("https://outlook.cloud.microsoft.mcas.ms/mail/").expect("url");
        let r = crate::agent::testing::response(
            200,
            &[("Set-Cookie", "AdallomAuth00=x; Domain=.mcas.ms; Path=/; Secure; HttpOnly; SameSite=None"), ("Set-Cookie", "plain=y")],
            "",
        );
        let set = set_cookies(&url, &r);
        assert_eq!(
            set[0],
            json!({"name": "AdallomAuth00", "value": "x", "url": "https://outlook.cloud.microsoft.mcas.ms/mail/", "domain": ".mcas.ms", "path": "/", "secure": true, "httpOnly": true, "sameSite": "None"})
        );
        assert_eq!(set[1], json!({"name": "plain", "value": "y", "url": "https://outlook.cloud.microsoft.mcas.ms/mail/"}));
    }
}
