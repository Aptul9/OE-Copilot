//! MCP over Streamable HTTP, stateless, as createMcpHandler of @modelcontextprotocol/server 2.4 serves 2025-era
//! clients: every POST is a fresh exchange, requests are answered on an SSE stream, notifications and responses get
//! 202, GET and DELETE 405.

use super::schema::{TOOLS, tool, validate};
use crate::web::{Body, Resp, empty, json_with};
use bytes::Bytes;
use futures_util::stream::{FuturesUnordered, StreamExt};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::mpsc;

pub const LATEST: &str = "2025-11-25";
pub const SUPPORTED: [&str; 5] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07"];
pub const MAX_BATCH: usize = 100;
const KEEP_ALIVE: Duration = Duration::from_secs(15);

pub enum Kind {
    Request,
    Notification,
    Response,
}

/// The JSON-RPC message kind, or None for something that is not one (the SDK answers 400 for the whole POST).
pub fn kind(m: &Value) -> Option<Kind> {
    let o = m.as_object()?;
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return None;
    }
    let id_ok = |id: &Value| id.is_string() || id.as_i64().is_some() || id.as_u64().is_some() || id.as_f64().is_some_and(|f| f.fract() == 0.0);
    let params_ok = o.get("params").is_none_or(Value::is_object);
    match (o.get("method"), o.get("id")) {
        (Some(Value::String(_)), Some(id)) if id_ok(id) && params_ok => Some(Kind::Request),
        (Some(Value::String(_)), None) if params_ok => Some(Kind::Notification),
        (None, Some(id))
            if id_ok(id)
                && (o.get("result").is_some_and(Value::is_object)
                    || o.get("error").is_some_and(|e| e.get("code").is_some_and(Value::is_number) && e.get("message").is_some_and(Value::is_string))) =>
        {
            Some(Kind::Response)
        }
        _ => None,
    }
}

pub fn error_body(code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "error": {"code": code, "message": message}, "id": null})
}

fn ok(id: &Value, result: Value) -> Value {
    json!({"result": result, "jsonrpc": "2.0", "id": id})
}

fn err(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One zod invalid_type issue, as the SDK prints them in its error messages.
fn type_issue(expected: &str, path: &[&str], got: Option<&Value>) -> Value {
    let received = super::schema::received(got);
    json!({"expected": expected, "code": "invalid_type", "path": path, "message": format!("Invalid input: expected {expected}, received {received}")})
}

/// zod's issues for an initialize request whose params break the schema, empty when they fit.
fn initialize_issues(req: &Value) -> Vec<Value> {
    let issue = type_issue;
    let Some(params) = req.get("params").filter(|p| p.is_object()) else { return vec![issue("object", &["params"], req.get("params"))] };
    let mut out = vec![];
    if !params.get("protocolVersion").is_some_and(Value::is_string) {
        out.push(issue("string", &["params", "protocolVersion"], params.get("protocolVersion")));
    }
    if !params.get("capabilities").is_some_and(Value::is_object) {
        out.push(issue("object", &["params", "capabilities"], params.get("capabilities")));
    }
    match params.get("clientInfo") {
        Some(ci @ Value::Object(_)) => {
            for k in ["name", "version"] {
                if !ci.get(k).is_some_and(Value::is_string) {
                    out.push(issue("string", &["params", "clientInfo", k], ci.get(k)));
                }
            }
        }
        other => out.push(issue("object", &["params", "clientInfo"], other)),
    }
    out
}

/// A well-formed initialize request: the one kind that may not share a POST.
pub fn is_initialize(req: &Value) -> bool {
    req.get("method").and_then(Value::as_str) == Some("initialize") && initialize_issues(req).is_empty()
}

/// The answer to one request, for the user the token acts as.
pub async fn answer(user_id: &str, req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let params = req.get("params").cloned().unwrap_or(json!({}));
    match req.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" if !is_initialize(req) => err(&id, -32603, &serde_json::to_string_pretty(&initialize_issues(req)).unwrap_or_default()),
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = if SUPPORTED.contains(&asked) { asked } else { LATEST };
            ok(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {"name": "teamsrelay", "version": "1.0.0"},
                }),
            )
        }
        "ping" => ok(&id, json!({})),
        "tools/list" => ok(&id, json!({"tools": *TOOLS})),
        "tools/call" => {
            let mut issues = vec![];
            if !params.get("name").is_some_and(Value::is_string) {
                issues.push(type_issue("string", &["params", "name"], params.get("name")));
            }
            if params.get("arguments").is_some_and(|a| !a.is_object()) {
                issues.push(type_issue("record", &["params", "arguments"], params.get("arguments")));
            }
            if !issues.is_empty() {
                return err(&id, -32602, &format!("Invalid tools/call request: {}", serde_json::to_string_pretty(&issues).unwrap_or_default()));
            }
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(t) = tool(name) else { return err(&id, -32602, &format!("Tool {name} not found")) };
            match validate(t, params.get("arguments")) {
                Ok(args) => ok(&id, super::tools::call(user_id, name, &args).await),
                Err(issues) => ok(
                    &id,
                    json!({"content": [{"type": "text", "text": format!("Input validation error: Invalid arguments for tool {name}: {issues}")}], "isError": true}),
                ),
            }
        }
        _ => err(&id, -32601, "Method not found"),
    }
}

pub fn sse_event(message: &Value) -> String {
    format!("event: message\ndata: {}\n\n", crate::js::stringify(message))
}

fn json(status: u16, body: &Value) -> Resp {
    json_with(status, body)
}

/// `application/json`, any parameters, any case.
fn json_content_type(v: Option<&str>) -> bool {
    v.and_then(|t| t.split(';').next()).is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
}

/// GET and DELETE: a stateless server has no stream to open and no session to end.
pub fn not_allowed() -> Resp {
    let mut r = json(405, &error_body(-32000, "Method not allowed."));
    r.headers_mut().insert("allow", hyper::header::HeaderValue::from_static("POST"));
    r
}

/// One POST of the transport: its checks in the SDK's order, then the answers.
pub fn post(user_id: String, accept: &str, content_type: Option<&str>, version: Option<&str>, bytes: &[u8]) -> Resp {
    if !json_content_type(content_type) {
        return json(415, &error_body(-32000, "Unsupported Media Type: Content-Type must be application/json"));
    }
    let acceptable = accept.contains("application/json") && accept.contains("text/event-stream");
    let not_acceptable = || json(406, &error_body(-32000, "Not Acceptable: Client must accept both application/json and text/event-stream"));
    let text = String::from_utf8_lossy(bytes);
    let Ok(raw) = serde_json::from_str::<Value>(text.strip_prefix('\u{feff}').unwrap_or(&text)) else {
        if !acceptable {
            return not_acceptable();
        }
        return json(400, &error_body(-32700, "Parse error: Invalid JSON"));
    };
    // the entry's shape check comes before the transport's Accept check
    let messages: Vec<Value> = match raw {
        Value::Array(a) => {
            if a.is_empty() {
                return json(400, &error_body(-32600, "Bad Request: empty JSON-RPC batch"));
            }
            if a.iter().any(|m| kind(m).is_none()) {
                return json(400, &error_body(-32600, "Bad Request: JSON-RPC batch contains an invalid message"));
            }
            a
        }
        v => {
            if kind(&v).is_none() {
                let mut body = error_body(-32600, "Bad Request: the request body is not a valid JSON-RPC message");
                let id = v.get("id").filter(|i| v.get("method").is_some_and(Value::is_string) && (i.is_string() || i.is_number())).cloned();
                body["id"] = id.unwrap_or(Value::Null);
                return json(400, &body);
            }
            vec![v]
        }
    };
    if !acceptable {
        return not_acceptable();
    }
    if messages.len() > MAX_BATCH {
        return json(400, &error_body(-32600, &format!("Invalid Request: Batch must not exceed {MAX_BATCH} messages")));
    }
    let requests: Vec<Value> = messages.iter().filter(|m| matches!(kind(m), Some(Kind::Request))).cloned().collect();
    let init = requests.iter().any(is_initialize);
    if init && messages.len() > 1 {
        return json(400, &error_body(-32600, "Invalid Request: Only one initialization request is allowed"));
    }
    if !init && let Some(v) = version.filter(|v| !SUPPORTED.contains(v)) {
        let message = format!("Bad Request: Unsupported protocol version: {v} (supported versions: {})", SUPPORTED.join(", "));
        return json(400, &error_body(-32000, &message));
    }
    if requests.is_empty() {
        return empty(202);
    }
    sse(user_id, requests)
}

/// One SSE event per answer as each finishes, a keep-alive comment every 15 s, closed after the last answer. A client
/// that goes away drops the body: the running calls are cancelled with it.
fn sse(user_id: String, requests: Vec<Value>) -> Resp {
    let (tx, mut rx) = mpsc::channel::<Bytes>(16);
    tokio::task::spawn_local(async move {
        let mut pending: FuturesUnordered<_> = requests.iter().map(|r| answer(&user_id, r)).collect();
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
        loop {
            tokio::select! {
                next = pending.next() => match next {
                    Some(answer) => {
                        if tx.send(Bytes::from(sse_event(&answer))).await.is_err() {
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
    let body: Body = StreamBody::new(stream).boxed_unsync();
    hyper::Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache, no-transform")
        .header("connection", "keep-alive")
        .header("x-accel-buffering", "no")
        .body(body)
        .expect("response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert!(matches!(kind(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})), Some(Kind::Request)));
        assert!(matches!(kind(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})), Some(Kind::Notification)));
        assert!(matches!(kind(&json!({"jsonrpc": "2.0", "id": 1, "result": {}})), Some(Kind::Response)));
        assert!(kind(&json!({"id": 1, "method": "ping"})).is_none());
        assert!(kind(&json!({"jsonrpc": "2.0", "id": 1.5, "method": "ping"})).is_none());
        assert!(kind(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": [1]})).is_none());
    }

    #[test]
    fn wire_shapes() {
        assert_eq!(sse_event(&ok(&json!(3), json!({}))), "event: message\ndata: {\"result\":{},\"jsonrpc\":\"2.0\",\"id\":3}\n\n");
        assert_eq!(
            crate::js::stringify(&err(&json!(4), -32601, "Method not found")),
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"Method not found"}}"#
        );
        assert!(json_content_type(Some("Application/JSON; charset=utf-8")));
        assert!(!json_content_type(Some("text/plain")));
        assert!(!json_content_type(None));
    }
}
