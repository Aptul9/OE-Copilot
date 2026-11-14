//! The supervisor of the browsers container, reached only through its unix socket in the volume both containers share
//! (src/lib/control.ts): start, stop, wipe and show the accounts, and the directory of the shared Microsoft accounts.

use crate::web::{Error, Res, err};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serde_json::Value;
use std::time::Duration;

/// A request on the socket: status and body, or the error text of the connection.
async fn call(socket: &str, method: &str, path: &str, body: Option<String>, wait: Duration) -> Result<(u16, String), String> {
    let run = async {
        let stream = tokio::net::UnixStream::connect(socket).await.map_err(|e| connect_text(socket, &e))?;
        let (mut send, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.map_err(|e| e.to_string())?;
        tokio::task::spawn_local(async move {
            let _ = conn.await;
        });
        let mut b = hyper::Request::builder().method(method).uri(path).header("host", "localhost");
        let data = body.unwrap_or_default();
        if !data.is_empty() {
            b = b.header("content-type", "application/json").header("content-length", data.len().to_string());
        }
        let resp = send.send_request(b.body(Full::new(Bytes::from(data))).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let bytes = resp.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    };
    match tokio::time::timeout(wait, run).await {
        Ok(r) => r,
        Err(_) => Err(format!("no answer within {} s", wait.as_secs())),
    }
}

/// The text Node gives a failed connect, e.g. "connect ENOENT /run/teamsrelay/control.sock".
fn connect_text(socket: &str, e: &std::io::Error) -> String {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        _ => return format!("connect {e} {socket}"),
    };
    format!("connect {code} {socket}")
}

fn detail(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body).ok()?.get("detail")?.as_str().map(String::from)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Start,
    Signin,
    Light,
    Stop,
    Wipe,
    Show,
}

impl Action {
    fn path(self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Signin => "signin",
            Action::Light => "light",
            Action::Stop => "stop",
            Action::Wipe => "wipe",
            Action::Show => "show",
        }
    }
    fn verb(self) -> &'static str {
        match self {
            Action::Start => "Start",
            Action::Signin => "Open sign-in browser",
            Action::Light => "Start without browser",
            Action::Stop => "Stop",
            Action::Wipe => "Wipe",
            Action::Show => "Show",
        }
    }
}

/// One action on an account; the light start carries the shared account of the slot and its region.
pub async fn post(action: Action, n: i64) -> Res<String> {
    let mut query = String::new();
    if action == Action::Light
        && let Some((account, region)) = crate::appdb::shared_binding(n)?
    {
        query = format!("?{}", crate::js::search_params(&[("shared", &account), ("region", &region)]));
    }
    let socket = crate::config::control_socket();
    match call(&socket, "POST", &format!("/accounts/{n}/{}{query}", action.path()), None, Duration::from_secs(40)).await {
        Ok((status, body)) if (200..300).contains(&status) => Ok(body),
        Ok((status, body)) => {
            Err(err(502, format!("{} of account {n} failed: {}", action.verb(), detail(&body).unwrap_or_else(|| format!("status {status}")))))
        }
        Err(e) => Err(err(503, format!("Browsers container not reachable: {e}"))),
    }
}

pub async fn start(n: i64) -> Res<()> {
    post(Action::Start, n).await.map(drop)
}
pub async fn signin(n: i64) -> Res<()> {
    post(Action::Signin, n).await.map(drop)
}
pub async fn light(n: i64) -> Res<()> {
    post(Action::Light, n).await.map(drop)
}
pub async fn stop(n: i64) -> Res<()> {
    post(Action::Stop, n).await.map(drop)
}
pub async fn wipe(n: i64) -> Res<()> {
    post(Action::Wipe, n).await.map(drop)
}

/// The window of the account to the front of the remote desktop.
pub async fn show(n: i64) -> Res<bool> {
    let body = post(Action::Show, n).await?;
    let text = if body.is_empty() { "{}" } else { &body };
    let v: Value = serde_json::from_str(text).map_err(|e| Error::Internal(format!("show: {e}")))?;
    Ok(v.get("shown") == Some(&Value::Bool(true)))
}

/// The directory of the shared Microsoft accounts of the OE calendar agent, through the supervisor: None for a 404 (no
/// broker, or a supervisor of an earlier release).
pub async fn directory(method: &str, path: &str, body: Option<&Value>) -> Res<Option<Value>> {
    let wait = if path == "/directory/connect" { 180 } else { 50 };
    let socket = crate::config::control_socket();
    match call(&socket, method, path, body.map(crate::js::stringify), Duration::from_secs(wait)).await {
        Ok((404, _)) => Ok(None),
        Ok((200, text)) => serde_json::from_str(&text).map(Some).map_err(|_| err(502, "Shared accounts not reached: not JSON")),
        Ok((status, text)) => Err(err(502, format!("Shared accounts not reached: {}", detail(&text).unwrap_or_else(|| format!("status {status}"))))),
        Err(e) => Err(err(503, format!("Browsers container not reachable: {}", e.replace("no answer within 180 s", "no answer within 50 s")))),
    }
}

pub async fn directory_list() -> Res<Option<Value>> {
    directory("GET", "/directory", None).await
}
pub async fn directory_set(body: &Value) -> Res<Option<Value>> {
    directory("POST", "/directory", Some(body)).await
}
pub async fn directory_report(body: &Value) -> Res<Option<Value>> {
    directory("POST", "/directory/teams", Some(body)).await
}
pub async fn directory_connect(body: &Value) -> Res<Option<Value>> {
    directory("POST", "/directory/connect", Some(body)).await
}
