//! A minimal client of the Chrome DevTools Protocol, in place of Playwright's connectOverCDP: requests by id on the
//! browser's websocket, flat sessions for pages and frames, events to whoever subscribed. Cookies come out in the shape
//! Playwright 1.64 gave them (doGetCookies, filterCookies), as the files and the OE calendar agent keep them.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

type Pending = Rc<RefCell<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
type Subscribers = Rc<RefCell<Vec<mpsc::UnboundedSender<Value>>>>;

/// One connection to the browser.
pub struct Cdp {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    subscribers: Subscribers,
    next: Cell<u64>,
    closed: Rc<Cell<bool>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Cdp {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl Cdp {
    /// The browser of DevTools port `port`, in 5 s.
    pub async fn connect(port: u16) -> Result<Rc<Cdp>, String> {
        Self::connect_url(&format!("http://127.0.0.1:{port}")).await
    }

    /// The browser whose DevTools answer at `base` (http://127.0.0.1:9222), in 5 s.
    pub async fn connect_url(base: &str) -> Result<Rc<Cdp>, String> {
        tokio::time::timeout(Duration::from_secs(5), Self::open(base)).await.map_err(|_| format!("DevTools of {base} did not answer in 5 s"))?
    }

    async fn open(base: &str) -> Result<Rc<Cdp>, String> {
        let base = Url::parse(base).map_err(|e| e.to_string())?;
        let version_url = base.join("/json/version").map_err(|e| e.to_string())?;
        let body = reqwest::Client::new().get(version_url).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
        let version: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        let ws = version.get("webSocketDebuggerUrl").and_then(Value::as_str).ok_or("no webSocketDebuggerUrl")?;
        let host = base.host_str().unwrap_or("127.0.0.1").to_string();
        let port = base.port_or_known_default().unwrap_or(9222);
        let stream = tokio::net::TcpStream::connect((host.as_str(), port)).await.map_err(|e| e.to_string())?;
        let (socket, _) = tokio_tungstenite::client_async_with_config(ws, stream, Some(socket_config())).await.map_err(|e| e.to_string())?;
        let (mut write, mut read) = socket.split();
        let (out, mut queue) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Rc::default();
        let subscribers: Subscribers = Rc::default();
        let closed = Rc::new(Cell::new(false));
        let writer = tokio::task::spawn_local(async move {
            while let Some(text) = queue.recv().await {
                if write.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            let _ = write.close().await;
        });
        let (p, s, c) = (pending.clone(), subscribers.clone(), closed.clone());
        let reader = tokio::task::spawn_local(async move {
            while let Some(Ok(m)) = read.next().await {
                let Message::Text(t) = m else { continue };
                let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                let Some(id) = v.get("id").and_then(Value::as_u64) else {
                    s.borrow_mut().retain(|tx| tx.send(v.clone()).is_ok());
                    continue;
                };
                let Some(tx) = p.borrow_mut().remove(&id) else { continue };
                let answer = match v.get("error") {
                    Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("DevTools error").to_string()),
                    None => Ok(v.get("result").cloned().unwrap_or(json!({}))),
                };
                let _ = tx.send(answer);
            }
            c.set(true);
            s.borrow_mut().clear();
            for (_, tx) in p.borrow_mut().drain() {
                let _ = tx.send(Err("DevTools connection closed".into()));
            }
        });
        Ok(Rc::new(Cdp { out, pending, subscribers, next: Cell::new(1), closed, tasks: vec![writer, reader] }))
    }

    pub fn connected(&self) -> bool {
        !self.closed.get()
    }

    /// Every event from now on (messages without an id: method, params, sessionId), until the receiver is dropped.
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        if !self.closed.get() {
            self.subscribers.borrow_mut().push(tx);
        }
        rx
    }

    /// One command, of the browser or of the target of `session`; 30 s at most.
    pub async fn send(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        self.send_within(method, params, session, Duration::from_secs(30)).await
    }

    pub async fn send_within(&self, method: &str, params: Value, session: Option<&str>, wait: Duration) -> Result<Value, String> {
        let id = self.next.get();
        self.next.set(id + 1);
        let mut m = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            m["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.borrow_mut().insert(id, tx);
        if self.closed.get() || self.out.send(m.to_string()).is_err() {
            self.pending.borrow_mut().remove(&id);
            return Err("DevTools connection closed".into());
        }
        match tokio::time::timeout(wait, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("DevTools connection closed".into()),
            Err(_) => {
                self.pending.borrow_mut().remove(&id);
                Err(format!("{method}: no answer in {} s", wait.as_secs()))
            }
        }
    }

    /// The tabs of the browser: id and URL, in the order Chromium lists them.
    pub async fn pages(&self) -> Result<Vec<(String, String)>, String> {
        let r = self.send("Target.getTargets", json!({}), None).await?;
        let s = |t: &Value, k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Ok(r.get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| s(t, "type") == "page")
            .map(|t| (s(t, "targetId"), s(t, "url")))
            .collect())
    }

    /// The URL of a tab now; Err when it is gone.
    pub async fn url_of(&self, target: &str) -> Result<String, String> {
        let r = self.send("Target.getTargetInfo", json!({ "targetId": target }), None).await?;
        Ok(r.get("targetInfo").and_then(|t| t.get("url")).and_then(Value::as_str).unwrap_or("").to_string())
    }

    /// An expression evaluated in a tab, its value; the page may be between two documents, so it is tried a few times.
    pub async fn evaluate(&self, target: &str, expression: &str) -> Result<Value, String> {
        let attached = self.send("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await?;
        let session = attached.get("sessionId").and_then(Value::as_str).ok_or("no session")?.to_string();
        let mut last = String::new();
        let mut value = None;
        for _ in 0..5 {
            match self.send("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true }), Some(&session)).await {
                Ok(r) if r.get("exceptionDetails").is_none() => {
                    value = Some(r.get("result").and_then(|x| x.get("value")).cloned().unwrap_or(Value::Null));
                    break;
                }
                Ok(_) => last = "exception".into(),
                Err(e) => last = e,
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let _ = self.send("Target.detachFromTarget", json!({ "sessionId": session }), None).await;
        value.ok_or(last)
    }

    /// The cookies of the browser's default context as Playwright gave them.
    pub async fn cookies(&self) -> Result<Vec<Value>, String> {
        let r = self.send("Storage.getCookies", json!({}), None).await?;
        Ok(r.get("cookies").and_then(Value::as_array).into_iter().flatten().map(playwright_cookie).collect())
    }
}

/// The websocket frames of a large page (a Teams IndexedDB read) fit.
fn socket_config() -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(256 << 20)).max_frame_size(Some(256 << 20))
}

/// A cookie of Storage.getCookies as Playwright's doGetCookies made it.
pub fn playwright_cookie(c: &Value) -> Value {
    let mut out = Map::new();
    for k in ["name", "value", "domain", "path", "expires", "httpOnly", "secure"] {
        if let Some(v) = c.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    out.insert("sameSite".into(), c.get("sameSite").filter(|v| !v.is_null()).cloned().unwrap_or(json!("Lax")));
    if let Some(p) = c.get("partitionKey").filter(|p| crate::js::truthy(p)) {
        if let Some(h) = p.get("hasCrossSiteAncestor") {
            out.insert("_crHasCrossSiteAncestor".into(), h.clone());
        }
        if let Some(t) = p.get("topLevelSite") {
            out.insert("partitionKey".into(), t.clone());
        }
    }
    Value::Object(out)
}

fn local(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host == "[::1]" || regex::Regex::new(r"^127\.\d+\.\d+\.\d+$").expect("fixed").is_match(host)
}

/// filterCookies of Playwright: the cookies a request to `url` carries.
pub fn for_url(c: &Value, url: &Url) -> bool {
    let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("");
    let domain = if s("domain").starts_with('.') { s("domain").to_string() } else { format!(".{}", s("domain")) };
    let host = url.host_str().unwrap_or("");
    format!(".{host}").ends_with(&domain)
        && url.path().starts_with(s("path"))
        && (url.scheme() == "https" || local(host) || c.get("secure") != Some(&Value::Bool(true)))
}

/// The Cookie header of a request to `url` from the cookies of the browser.
pub fn cookie_header(cookies: &[Value], url: &Url) -> String {
    cookies
        .iter()
        .filter(|c| for_url(c, url))
        .map(|c| format!("{}={}", c["name"].as_str().unwrap_or(""), c["value"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The cookies an answer sets, as Playwright's request context stores them in the browser (Storage.setCookies).
pub fn set_cookies(url: &Url, r: &crate::agent::http::Response) -> Vec<Value> {
    let mut out = vec![];
    for raw in crate::agent::http::all(r, "set-cookie") {
        let mut attrs = raw.split(';');
        let pair = attrs.next().unwrap_or("");
        let Some(eq) = pair.find('=') else { continue };
        let mut c =
            json!({ "name": pair[..eq].trim(), "value": pair[eq + 1..].trim(), "url": format!("{}{}", url.origin().ascii_serialization(), url.path()) });
        for attr in attrs {
            let (k, v) = match attr.find('=') {
                Some(at) => (attr[..at].trim().to_lowercase(), attr[at + 1..].trim()),
                None => (attr.trim().to_lowercase(), ""),
            };
            match k.as_str() {
                "domain" if !v.is_empty() => c["domain"] = json!(v),
                "path" if v.starts_with('/') => c["path"] = json!(v),
                "secure" => c["secure"] = json!(true),
                "httponly" => c["httpOnly"] = json!(true),
                "samesite" => {
                    c["sameSite"] = json!(match v.to_lowercase().as_str() {
                        "strict" => "Strict",
                        "none" => "None",
                        _ => "Lax",
                    })
                }
                "max-age" => {
                    if let Ok(age) = v.parse::<f64>() {
                        c["expires"] = crate::web::num(crate::web::now_ms() as f64 / 1000.0 + age);
                    }
                }
                "expires" => {
                    if c.get("expires").is_none()
                        && let Some(ms) = crate::agent::session::cookie_date(v)
                    {
                        c["expires"] = crate::web::num(ms / 1000.0);
                    }
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}
