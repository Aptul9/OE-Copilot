//! Control API for the web app (src/supervisor/server.ts), HTTP on a unix socket that only root opens:
//!   POST /accounts/N/start | signin | light[?shared=<account>&region=<region>] | stop | wipe | show     GET /accounts
//!   GET /directory   POST /directory   POST /directory/teams   POST /directory/connect

use super::accounts::{AccountError, Accounts};
use super::broker::{Directory, binding_of};
use super::process::Lines;
use bytes::Bytes;
use futures_util::future::LocalBoxFuture;
use http_body_util::{BodyExt, Full, Limited};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::future::Future;
use std::rc::Rc;
use std::sync::LazyLock;

/// The body of a directory request: an account and its capabilities, or the report of the slots.
const MAX_BODY: usize = 64 * 1024;

static ACTION: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new("^/accounts/([^/]+)/(start|signin|light|stop|wipe|show)$").expect("fixed"));

pub struct Control {
    pub accounts: Rc<Accounts>,
    /// brings the desktop session up and resolves when it is: a browser started before it would find no display
    pub desktop: Rc<dyn Fn() -> LocalBoxFuture<'static, Result<(), String>>>,
    pub log: Lines,
    /// the list of shared Microsoft accounts of the OE calendar agent; None without a broker
    pub directory: Option<Rc<Directory>>,
}

type Reply = (u16, Option<Value>);

fn not_allowed() -> Result<Reply, AccountError> {
    Ok((405, Some(json!({ "detail": "Method not allowed" }))))
}

async fn read_json(body: hyper::body::Incoming) -> Result<Value, AccountError> {
    let bytes = match Limited::new(body, MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) if e.is::<http_body_util::LengthLimitError>() => return Err(AccountError::new(413, "Request body too large")),
        Err(e) => return Err(AccountError::new(400, &e.to_string())),
    };
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|_| AccountError::new(400, "Request body is not JSON"))
}

impl Control {
    async fn desktop_up(&self) -> Result<(), AccountError> {
        (self.desktop)().await.map_err(|e| AccountError::new(500, &e))
    }

    async fn handle(&self, method: &hyper::Method, url: &str, body: hyper::body::Incoming) -> Result<Reply, AccountError> {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let post = method == hyper::Method::POST;
        if path == "/accounts" {
            return if method == hyper::Method::GET { Ok((200, Some(self.accounts.status()))) } else { not_allowed() };
        }
        if path == "/directory" || path == "/directory/teams" || path == "/directory/connect" {
            let Some(d) = &self.directory else {
                return Ok((404, Some(json!({ "detail": "No shared Microsoft accounts: SHARED_BROKER_URL and SHARED_BROKER_KEY are not set" }))));
            };
            if path == "/directory" && method == hyper::Method::GET {
                return Ok((200, Some(d.list().await?)));
            }
            if !post {
                return not_allowed();
            }
            let b = read_json(body).await?;
            let answer = match path {
                "/directory/connect" => d.connect(&b).await?,
                "/directory" => d.set(&b).await?,
                _ => d.report(&b).await?,
            };
            return Ok((200, Some(answer)));
        }
        let Some(m) = ACTION.captures(path) else { return Ok((404, Some(json!({ "detail": "Not found" })))) };
        if !post {
            return not_allowed();
        }
        let raw = &m[1];
        let n = if raw.bytes().all(|b| b.is_ascii_digit()) { raw.parse::<i64>().ok() } else { None };
        let a = &self.accounts;
        match &m[2] {
            "signin" => {
                self.desktop_up().await?;
                a.signin(n).await?;
                Ok((204, None))
            }
            "start" => {
                self.desktop_up().await?;
                a.start(n).await?;
                Ok((204, None))
            }
            "light" => {
                a.light(n, binding_of(query)?).await?;
                Ok((204, None))
            }
            "stop" => {
                a.stop(n).await?;
                Ok((204, None))
            }
            "wipe" => {
                a.wipe(n).await?;
                Ok((204, None))
            }
            _ => Ok((200, Some(json!({ "shown": a.show(n).await? })))),
        }
    }

    async fn respond(self: Rc<Self>, req: hyper::Request<hyper::body::Incoming>) -> hyper::Response<Full<Bytes>> {
        let (parts, body) = req.into_parts();
        let url = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_string();
        let (status, body) = match self.handle(&parts.method, &url, body).await {
            Ok(r) => r,
            Err(e) => {
                if e.status >= 500 {
                    (self.log)(&format!("control: {} {url} failed: {}", parts.method, e.message));
                }
                (e.status, Some(json!({ "detail": e.message })))
            }
        };
        let b = hyper::Response::builder().status(status);
        match body {
            Some(v) => b.header("content-type", "application/json").body(Full::new(Bytes::from(crate::js::stringify(&v)))),
            None => b.body(Full::new(Bytes::new())),
        }
        .expect("response")
    }
}

/// The socket, in place of one left by an earlier run, open to its user only.
pub fn listen(socket: &str) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(socket);
    let l = tokio::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

/// Serves until `stop`; requests in progress are dropped with the process.
pub async fn serve(listener: tokio::net::UnixListener, control: Rc<Control>, stop: impl Future<Output = ()>) {
    tokio::pin!(stop);
    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((s, _)) => s,
                Err(_) => continue,
            },
            _ = &mut stop => break,
        };
        let c = control.clone();
        tokio::task::spawn_local(async move {
            let service = hyper::service::service_fn(move |req| {
                let c = c.clone();
                async move { Ok::<_, Infallible>(c.respond(req).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), service).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{Net, run, temp_dir};
    use crate::supervisor::accounts::tests::{accounts_with, cfg};
    use std::cell::RefCell;
    use std::time::Duration;

    /// the socket, the calls of the processes, the switch of the desktop, how many times the desktop was asked for
    type Served = (String, Rc<RefCell<Vec<String>>>, tokio::sync::watch::Sender<bool>, Rc<std::cell::Cell<u32>>);

    /// A server on a temporary socket whose desktop comes up when the test says.
    fn serve_test(directory: Option<Rc<Directory>>, desktop_fails: bool) -> Served {
        let sock = temp_dir().join("control.sock").display().to_string();
        let (accounts, w) = accounts_with(cfg(), 0, None);
        let (up, rx) = tokio::sync::watch::channel(false);
        let asked = Rc::new(std::cell::Cell::new(0));
        let a = asked.clone();
        let control = Rc::new(Control {
            accounts,
            desktop: Rc::new(move || {
                a.set(a.get() + 1);
                let mut rx = rx.clone();
                Box::pin(async move {
                    if desktop_fails {
                        return Err("spawn failed".to_string());
                    }
                    let _ = rx.wait_for(|v| *v).await;
                    Ok(())
                })
            }),
            log: Rc::new(|_: &str| {}),
            directory,
        });
        let l = listen(&sock).expect("listen");
        tokio::task::spawn_local(serve(l, control, futures_util::future::pending()));
        (sock, w.calls, up, asked)
    }

    async fn request(sock: &str, method: &str, path: &str) -> (u16, Value) {
        let (s, body) = crate::control::call(sock, method, path, None, Duration::from_secs(10)).await.expect("answer");
        (s, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    #[test]
    fn starts_once_the_desktop_is_up_and_a_light_start_does_not_wait_for_it() {
        run(async {
            let (sock, calls, up, asked) = serve_test(None, false);
            let s = sock.clone();
            let reply = tokio::task::spawn_local(async move { request(&s, "POST", "/accounts/1/start").await });
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(calls.borrow().is_empty());
            assert_eq!(request(&sock, "POST", "/accounts/4/light").await.0, 204);
            assert_eq!(asked.get(), 1);
            up.send(true).expect("up");
            assert_eq!(reply.await.expect("task").0, 204);
            assert_eq!(*calls.borrow(), ["start light-4", "start browser-1", "start agent-1"]);
            assert_eq!(request(&sock, "POST", "/accounts/6/signin").await.0, 204);
            assert_eq!(asked.get(), 2);
        });
    }

    #[test]
    fn shows_stops_wipes_lists_and_answers_404_405_and_500() {
        run(async {
            let (sock, calls, up, _) = serve_test(None, false);
            up.send(true).expect("up");
            request(&sock, "POST", "/accounts/2/start").await;
            assert_eq!(request(&sock, "POST", "/accounts/2/show").await, (200, json!({"shown": true})));
            assert_eq!(request(&sock, "POST", "/accounts/2/wipe").await, (409, json!({"detail": "Account 2 is running: stop it first"})));
            assert_eq!(request(&sock, "POST", "/accounts/2/stop").await.0, 204);
            assert_eq!(request(&sock, "POST", "/accounts/2/wipe").await.0, 204);
            let (status, list) = request(&sock, "GET", "/accounts").await;
            assert_eq!((status, list[0]["account"].clone(), list[0]["browser"]["running"].clone()), (200, json!(2), json!(false)));
            calls.borrow_mut().clear();
            assert_eq!(request(&sock, "POST", "/accounts/0/start").await, (404, json!({"detail": "No account 0: accounts are numbered from 1"})));
            assert_eq!(request(&sock, "POST", "/accounts/x/start").await.0, 404);
            assert_eq!(request(&sock, "POST", "/elsewhere").await.0, 404);
            assert_eq!(request(&sock, "GET", "/accounts/1/start").await.0, 405);
            assert_eq!(request(&sock, "DELETE", "/accounts").await.0, 405);
            assert_eq!(request(&sock, "POST", "/accounts/2/light?shared=me&region=emea").await.0, 400);
            assert_eq!(request(&sock, "GET", "/directory").await.0, 404);
            assert!(calls.borrow().is_empty());
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&sock).expect("socket").permissions().mode() & 0o777, 0o600);

            let (sock, _, _, _) = serve_test(None, true);
            assert_eq!(request(&sock, "POST", "/accounts/1/start").await, (500, json!({"detail": "spawn failed"})));
        });
    }

    #[test]
    fn the_directory_goes_to_the_agent_and_a_body_too_large_is_refused() {
        run(async {
            let net = Net::new();
            net.answer(|_| (200, r#"{"ok":true,"accounts":[]}"#.into()));
            let d = Directory::new(crate::supervisor::broker::SharedBroker { url: "http://agent:8180".into(), key: "k".into() }, net.http(), None, None);
            let (sock, _, _, _) = serve_test(Some(Rc::new(d)), false);
            assert_eq!(request(&sock, "GET", "/directory").await, (200, json!({"ok": true, "accounts": []})));
            let body = json!({"account": "mario@example.com", "capabilities": ["chat"]}).to_string();
            let (s, _) = crate::control::call(&sock, "POST", "/directory", Some(body), Duration::from_secs(10)).await.expect("answer");
            assert_eq!(s, 200);
            let (s, b) = crate::control::call(&sock, "POST", "/directory/teams", Some("x".repeat(70 * 1024)), Duration::from_secs(10)).await.expect("answer");
            assert_eq!((s, b.as_str()), (413, r#"{"detail":"Request body too large"}"#));
            let (s, _) = crate::control::call(&sock, "POST", "/directory/teams", Some("{nope".into()), Duration::from_secs(10)).await.expect("answer");
            assert_eq!(s, 400);
            assert_eq!(net.requests().len(), 2);
        });
    }
}
