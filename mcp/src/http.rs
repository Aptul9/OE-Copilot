//! Remote entry: one authenticated Streamable HTTP MCP endpoint. Backend credentials stay on the server; the client
//! holds only MCP_HTTP_TOKEN.
//!   POST/GET/DELETE /mcp   Authorization: Bearer <MCP_HTTP_TOKEN>, Host in MCP_HTTP_HOSTS, no Origin (no browser callers)
//!   GET /healthz           {"ok":true}, no auth, no backend call: for the container healthcheck
//!   /everflow/*            Authorization: Bearer <EVERFLOW_TOKEN>, read-only routes for Everflow (everflow.rs)
//!   GET /files/<ticket>    an attachment through a signed ten-minute link, no bearer (files.rs)

use crate::communication::Communication;
use crate::mcp::{self, Kind, error_body, kind};
use bytes::Bytes;
use futures_util::stream::{FuturesUnordered, StreamExt};
use http_body_util::{BodyExt, Empty, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use serde_json::Value;
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const MAX_BODY: usize = 4 * 1024 * 1024;
pub const MIN_TOKEN: usize = 32;
const KEEP_ALIVE: Duration = Duration::from_secs(15);

pub type Body = UnsyncBoxBody<Bytes, Infallible>;

pub struct App {
    pub comm: Communication,
    pub token: String,
    pub hosts: HashSet<String>,
    /// EVERFLOW_TOKEN: the bearer of the read-only /everflow/* routes (everflow.rs); None turns them off
    pub everflow: Option<String>,
}

/// MCP_HTTP_HOSTS: comma-separated Host header values.
pub fn hosts_from_env(raw: &str) -> HashSet<String> {
    raw.split(',').map(|h| h.trim().to_lowercase()).filter(|h| !h.is_empty()).collect()
}

pub(crate) fn empty(status: u16, headers: &[(&str, &str)]) -> Response<Body> {
    let mut r = Response::builder().status(status);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    r.body(Empty::new().boxed_unsync()).expect("response")
}

pub(crate) fn json(status: u16, body: &Value, headers: &[(&str, &str)]) -> Response<Body> {
    let mut r = Response::builder().status(status).header("content-type", "application/json");
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    r.body(Full::new(Bytes::from(crate::js::stringify(body))).boxed_unsync()).expect("response")
}

pub(crate) fn bearer_matches(header: Option<&str>, token: &str) -> bool {
    let got = header.unwrap_or("").as_bytes();
    let want = format!("Bearer {token}");
    let want = want.as_bytes();
    got.len() == want.len() && got.iter().zip(want).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

pub(crate) fn header<'a>(req: &'a Request<Incoming>, name: &str) -> Option<&'a str> {
    req.headers().get(name).and_then(|v| v.to_str().ok())
}

/// `application/json`, any parameters, any case.
fn json_content_type(v: Option<&str>) -> bool {
    v.and_then(|t| t.split(';').next()).is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
}

pub async fn handle(app: Arc<App>, req: Request<Incoming>) -> (Response<Body>, String) {
    let path: String = req.uri().path().chars().take(100).collect();
    let method = req.method().as_str().to_string();
    let r = route(app, req, &path).await;
    (r, format!("{method} {path}"))
}

async fn route(app: Arc<App>, req: Request<Incoming>, path: &str) -> Response<Body> {
    if path == "/healthz" && req.method() == hyper::Method::GET {
        return json(200, &serde_json::json!({"ok": true}), &[]);
    }
    if path.starts_with("/everflow/") {
        return crate::everflow::handle(&app, req, path).await;
    }
    if path.starts_with("/files/") {
        return crate::files::handle(&app, req).await;
    }
    if path != "/mcp" {
        return empty(404, &[]);
    }
    let host = header(&req, "host").unwrap_or("").to_lowercase();
    if !app.hosts.contains(&host) {
        return empty(421, &[]);
    }
    if req.headers().contains_key("origin") {
        return empty(403, &[]);
    }
    if !bearer_matches(header(&req, "authorization"), &app.token) {
        return empty(401, &[("WWW-Authenticate", "Bearer realm=\"communication-mcp\"")]);
    }
    match req.method().as_str() {
        "POST" => {}
        "GET" | "DELETE" => return json(405, &error_body(-32000, "Method not allowed."), &[("allow", "POST")]),
        _ => return empty(405, &[("Allow", "POST, GET, DELETE")]),
    }
    let declared = header(&req, "content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    if declared > MAX_BODY as u64 {
        return empty(413, &[("Connection", "close")]);
    }
    let accept = header(&req, "accept").unwrap_or("").to_string();
    let content_type = header(&req, "content-type").map(String::from);
    let version = header(&req, "mcp-protocol-version").map(String::from);
    let mut body = req.into_body();
    let mut bytes: Vec<u8> = vec![];
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else { return empty(400, &[("Connection", "close")]) };
        if let Some(d) = frame.data_ref() {
            bytes.extend_from_slice(d);
            if bytes.len() > MAX_BODY {
                return empty(413, &[("Connection", "close")]);
            }
        }
    }
    post(app, &accept, content_type.as_deref(), version.as_deref(), &bytes)
}

fn post(app: Arc<App>, accept: &str, content_type: Option<&str>, version: Option<&str>, bytes: &[u8]) -> Response<Body> {
    if !json_content_type(content_type) {
        return json(415, &error_body(-32000, "Unsupported Media Type: Content-Type must be application/json"), &[]);
    }
    let acceptable = accept.contains("application/json") && accept.contains("text/event-stream");
    let not_acceptable = || json(406, &error_body(-32000, "Not Acceptable: Client must accept both application/json and text/event-stream"), &[]);
    let text = String::from_utf8_lossy(bytes);
    let Ok(raw) = serde_json::from_str::<Value>(text.strip_prefix('\u{feff}').unwrap_or(&text)) else {
        if !acceptable {
            return not_acceptable();
        }
        return json(400, &error_body(-32700, "Parse error: Invalid JSON"), &[]);
    };
    // the entry's shape check comes before the transport's Accept check
    let messages: Vec<Value> = match raw {
        Value::Array(a) => {
            if a.is_empty() {
                return json(400, &error_body(-32600, "Bad Request: empty JSON-RPC batch"), &[]);
            }
            if a.iter().any(|m| kind(m).is_none()) {
                return json(400, &error_body(-32600, "Bad Request: JSON-RPC batch contains an invalid message"), &[]);
            }
            a
        }
        v => {
            if kind(&v).is_none() {
                let mut body = error_body(-32600, "Bad Request: the request body is not a valid JSON-RPC message");
                let id = v.get("id").filter(|i| v.get("method").is_some_and(Value::is_string) && (i.is_string() || i.is_number())).cloned();
                body["id"] = id.unwrap_or(Value::Null);
                return json(400, &body, &[]);
            }
            vec![v]
        }
    };
    if !acceptable {
        return not_acceptable();
    }
    if messages.len() > mcp::MAX_BATCH {
        return json(400, &error_body(-32600, &format!("Invalid Request: Batch must not exceed {} messages", mcp::MAX_BATCH)), &[]);
    }
    let requests: Vec<Value> = messages.iter().filter(|m| matches!(kind(m), Some(Kind::Request))).cloned().collect();
    let init = requests.iter().any(mcp::is_initialize);
    if init && messages.len() > 1 {
        return json(400, &error_body(-32600, "Invalid Request: Only one initialization request is allowed"), &[]);
    }
    if !init && let Some(v) = version.filter(|v| !mcp::SUPPORTED.contains(v)) {
        let message = format!("Bad Request: Unsupported protocol version: {v} (supported versions: {})", mcp::SUPPORTED.join(", "));
        return json(400, &error_body(-32000, &message), &[]);
    }
    if requests.is_empty() {
        return empty(202, &[]);
    }
    sse(app, requests)
}

/// One SSE event per answer as each finishes, a keep-alive comment every 15 s, closed after the last answer. A client
/// that goes away drops the body: the running calls are cancelled with it.
fn sse(app: Arc<App>, requests: Vec<Value>) -> Response<Body> {
    let (tx, mut rx) = mpsc::channel::<Bytes>(16);
    tokio::task::spawn_local(async move {
        let mut pending: FuturesUnordered<_> = requests.iter().map(|r| mcp::answer(&app.comm, r)).collect();
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
        loop {
            tokio::select! {
                next = pending.next() => match next {
                    Some(answer) => {
                        if tx.send(Bytes::from(mcp::sse_event(&answer))).await.is_err() {
                            return;
                        }
                    }
                    None => return,
                },
                _ = tick.tick() => {
                    if tx.send(Bytes::from_static(b": keepalive\n\n")).await.is_err() {
                        return;
                    }
                }
                _ = tx.closed() => return,
            }
        }
    });
    let stream = futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx).map(|o| o.map(|b| Ok::<_, Infallible>(Frame::data(b)))));
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache, no-transform")
        .header("connection", "keep-alive")
        .header("x-accel-buffering", "no")
        .body(StreamBody::new(stream).boxed_unsync())
        .expect("response")
}

/// Serves until a stop signal, then lets the open exchanges finish for up to 10 s. Runs inside a LocalSet: one thread,
/// every task local to it.
pub async fn serve(app: Arc<App>, listener: tokio::net::TcpListener, stop: impl std::future::Future<Output = ()>) {
    const SLOTS: u32 = 1 << 20;
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let open = Arc::new(tokio::sync::Semaphore::new(SLOTS as usize));
    tokio::pin!(stop);
    loop {
        let (stream, _) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(a) => a,
                Err(_) => continue,
            },
            _ = &mut stop => break,
        };
        let app = app.clone();
        let mut down = shutdown.subscribe();
        let guard = open.clone().try_acquire_owned().ok();
        tokio::task::spawn_local(async move {
            let _guard = guard;
            let service = hyper::service::service_fn(move |req| {
                let app = app.clone();
                async move {
                    let started = Instant::now();
                    let (resp, line) = handle(app, req).await;
                    if !line.ends_with(" /healthz") {
                        eprintln!("{line} {} {}ms", resp.status().as_u16(), started.elapsed().as_millis());
                    }
                    Ok::<_, Infallible>(resp)
                }
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(Duration::from_secs(15))
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service);
            tokio::pin!(conn);
            tokio::select! {
                _ = conn.as_mut() => {}
                _ = down.changed() => {
                    conn.as_mut().graceful_shutdown();
                    let _ = conn.await;
                }
            }
        });
    }
    let _ = shutdown.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(10), open.acquire_many(SLOTS)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert!(bearer_matches(Some("Bearer abc"), "abc"));
        assert!(!bearer_matches(Some("bearer abc"), "abc"));
        assert!(!bearer_matches(None, "abc"));
        assert!(json_content_type(Some("Application/JSON; charset=utf-8")));
        assert!(!json_content_type(Some("text/plain")));
        assert!(!json_content_type(None));
        assert_eq!(hosts_from_env(" A.example ,127.0.0.1:8190,,"), ["a.example".to_string(), "127.0.0.1:8190".to_string()].into_iter().collect());
    }
}
