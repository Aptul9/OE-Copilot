//! Read-only routes for Everflow, with their own bearer (EVERFLOW_TOKEN): the requests Everflow already speaks,
//! forwarded on the container network so no backend and no credential leaves this host.
//!   POST /everflow/agent   {"requests": [{"path", "body"?}]}  -> {"responses": [{"status", "body"}]}  OE Copilot agent reads
//!   POST /everflow/slack   {"workspace", "requests": [{"method", "params"?}]} -> the same, Slack Web API reads
//!   GET  /everflow/teams?since=<ms>&max_rows=<n>               -> the TeamsRelay answer of /api/everflow/saved
//! Statuses and bodies of the agent and Slack pass through per request; anything else answers like /mcp: 421 for a
//! Host outside MCP_HTTP_HOSTS, 403 with Origin, 401 without the token, 404 when EVERFLOW_TOKEN or the backend is off.

use crate::http::{App, Body, MAX_BODY, bearer_matches, empty, header, json};
use crate::slack::Lookup;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

/// The agent endpoints Everflow reads: the accounts without a body, the reads with one.
const AGENT_GET: &str = "/api/communication/accounts";
const AGENT_POST: [&str; 3] = ["/api/communication/read", "/api/communication/gmail/read", "/api/communication/gchat/read"];
/// READ_METHODS of everflow src/watchers/slack.ts, nothing else.
const SLACK_METHODS: [&str; 7] =
    ["auth.test", "client.counts", "subscriptions.thread.getView", "conversations.history", "conversations.replies", "conversations.info", "users.info"];
const MAX_REQUESTS: usize = 100;
/// The whole answer of one call; past it the call stops with 507 rather than a cut answer.
const MAX_ANSWER: usize = 64 * 1024 * 1024;

/// 507: this answer is over MAX_ANSWER; Everflow reports it as <source>.result-limit, apart from a backend that is down (502).
fn too_large() -> Response<Body> {
    error(507, "answer too large")
}

fn error(status: u16, message: &str) -> Response<Body> {
    json(status, &json!({ "error": message }), &[])
}

pub async fn handle(app: &App, req: Request<Incoming>, path: &str) -> Response<Body> {
    let Some(token) = app.everflow.as_deref() else { return empty(404, &[]) };
    let host = header(&req, "host").unwrap_or("").to_lowercase();
    if !app.hosts.contains(&host) {
        return empty(421, &[]);
    }
    if req.headers().contains_key("origin") {
        return empty(403, &[]);
    }
    if !bearer_matches(header(&req, "authorization"), token) {
        return empty(401, &[("WWW-Authenticate", "Bearer realm=\"everflow\"")]);
    }
    let method = req.method().as_str().to_string();
    match (path, method.as_str()) {
        ("/everflow/agent", "POST") => match read(req).await {
            Ok(v) => agent(app, v).await,
            Err(r) => *r,
        },
        ("/everflow/slack", "POST") => match read(req).await {
            Ok(v) => slack(app, v).await,
            Err(r) => *r,
        },
        ("/everflow/teams", "GET") => teams(app, req.uri().query().unwrap_or("")).await,
        ("/everflow/agent" | "/everflow/slack", _) => empty(405, &[("Allow", "POST")]),
        ("/everflow/teams", _) => empty(405, &[("Allow", "GET")]),
        _ => empty(404, &[]),
    }
}

/// The JSON object of a request body, at most MAX_BODY. The refusal is boxed: a Response is too large to travel in an Err.
async fn read(req: Request<Incoming>) -> Result<Map<String, Value>, Box<Response<Body>>> {
    let declared = header(&req, "content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    if declared > MAX_BODY as u64 {
        return Err(Box::new(empty(413, &[("Connection", "close")])));
    }
    let mut body = req.into_body();
    let mut bytes: Vec<u8> = vec![];
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else { return Err(Box::new(empty(400, &[("Connection", "close")]))) };
        if let Some(d) = frame.data_ref() {
            bytes.extend_from_slice(d);
            if bytes.len() > MAX_BODY {
                return Err(Box::new(empty(413, &[("Connection", "close")])));
            }
        }
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(Box::new(error(400, "the body must be a JSON object"))),
    }
}

/// The `requests` list, at most MAX_REQUESTS, each an object.
fn requests(v: &Map<String, Value>) -> Result<Vec<Map<String, Value>>, Box<Response<Body>>> {
    let Some(list) = v.get("requests").and_then(Value::as_array) else { return Err(Box::new(error(400, "requests must be a list"))) };
    if list.len() > MAX_REQUESTS {
        return Err(Box::new(error(400, &format!("at most {MAX_REQUESTS} requests per call"))));
    }
    list.iter().map(|r| r.as_object().cloned().ok_or_else(|| Box::new(error(400, "each request must be an object")))).collect()
}

/// Answers in request order; the running total stays under MAX_ANSWER.
fn answer(responses: Vec<(u16, String)>) -> Response<Body> {
    let list: Vec<Value> = responses.into_iter().map(|(status, body)| json!({ "status": status, "body": body })).collect();
    json(200, &json!({ "responses": list }), &[])
}

async fn agent(app: &App, v: Map<String, Value>) -> Response<Body> {
    let Some(gateway) = app.comm.google.as_ref() else { return empty(404, &[]) };
    let list = match requests(&v) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    // every request is checked before the first call
    let mut calls: Vec<(String, Option<Value>)> = vec![];
    for r in &list {
        let path = r.get("path").and_then(Value::as_str).unwrap_or("");
        let body = r.get("body").filter(|b| !b.is_null()).cloned();
        let fits = match &body {
            None => path == AGENT_GET,
            Some(b) => b.is_object() && AGENT_POST.contains(&path),
        };
        if !fits {
            return error(400, &format!("request not allowed: {}", path.chars().take(80).collect::<String>()));
        }
        calls.push((path.to_string(), body));
    }
    let mut out = vec![];
    let mut total = 0usize;
    for (path, body) in &calls {
        let (status, text) = match gateway.raw(path, body.as_ref(), MAX_ANSWER - total).await {
            Ok(r) => r,
            Err(crate::Error::Comm { code: "TOO_LARGE", .. }) => return too_large(),
            Err(crate::Error::Comm { code: "INTERRUPTED", .. }) => (502, json!({ "errore": "OE Copilot agent answer interrupted" }).to_string()),
            Err(_) => (502, json!({ "errore": "OE Copilot agent unreachable" }).to_string()),
        };
        total += text.len();
        out.push((status, text));
    }
    answer(out)
}

async fn slack(app: &App, v: Map<String, Value>) -> Response<Body> {
    static WORKSPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^T[A-Z0-9]+$").expect("fixed"));
    let Some(slack) = app.comm.slack.as_ref() else { return empty(404, &[]) };
    let workspace = v.get("workspace").and_then(Value::as_str).unwrap_or("");
    if !WORKSPACE.is_match(workspace) {
        return error(400, "workspace must be a Slack workspace id");
    }
    let list = match requests(&v) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut calls: Vec<(String, Vec<(String, String)>)> = vec![];
    for r in &list {
        let method = r.get("method").and_then(Value::as_str).unwrap_or("");
        if !SLACK_METHODS.contains(&method) {
            return error(400, &format!("method not allowed: {}", method.chars().take(80).collect::<String>()));
        }
        let mut params = vec![];
        match r.get("params") {
            None | Some(Value::Null) => {}
            Some(Value::Object(p)) => {
                for (k, v) in p {
                    // the session's token is the only one: a second one in the form would override it on Slack's side
                    if k.eq_ignore_ascii_case("token") {
                        return error(400, "params cannot carry a token");
                    }
                    let Some(v) = v.as_str() else { return error(400, "params must be strings") };
                    params.push((k.clone(), v.to_string()));
                }
            }
            Some(_) => return error(400, "params must be an object"),
        }
        calls.push((method.to_string(), params));
    }
    // the store is read once: every request of the call uses the same session
    let session = match slack.lookup(workspace) {
        Lookup::Missing => return error(404, "unknown Slack workspace"),
        Lookup::Unusable => return error(502, "slack session unusable"),
        Lookup::Ready(s) => s,
    };
    let mut out = vec![];
    let mut total = 0usize;
    for (method, params) in &calls {
        let (status, text) = match slack.raw(&session, method, params, MAX_ANSWER - total).await {
            Ok(r) => r,
            Err(crate::Error::Comm { code: "TOO_LARGE", .. }) => return too_large(),
            Err(crate::Error::Comm { code: "INTERRUPTED", .. }) => (502, json!({ "ok": false, "error": "slack_answer_interrupted" }).to_string()),
            Err(_) => (502, json!({ "ok": false, "error": "slack_unreachable" }).to_string()),
        };
        total += text.len();
        out.push((status, text));
    }
    answer(out)
}

async fn teams(app: &App, query: &str) -> Response<Body> {
    let Some(teams) = app.comm.teams.as_ref() else { return empty(404, &[]) };
    let param = |name: &str| form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    let digits = |v: Option<String>| v.filter(|s| !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_digit()));
    let since = digits(param("since")).and_then(|s| s.parse::<u64>().ok()).filter(|n| *n <= 9_007_199_254_740_991);
    let max_rows = digits(param("max_rows")).and_then(|s| s.parse::<u32>().ok()).filter(|n| (1..=100_000).contains(n));
    let (Some(since), Some(max_rows)) = (since, max_rows) else { return error(400, "since (ms) and max_rows (1 to 100000) are required") };
    match teams.saved(since, max_rows, MAX_ANSWER).await {
        Ok((200, body)) => Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(body)).boxed_unsync())
            .expect("response"),
        Ok((status, _)) => error(502, &format!("teams relay HTTP {status}")),
        Err(crate::Error::Comm { code: "TOO_LARGE", .. }) => too_large(),
        Err(crate::Error::Comm { code: "INTERRUPTED", .. }) => error(502, "teams relay answer interrupted"),
        Err(_) => error(502, "teams relay unreachable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists() {
        assert!(!AGENT_POST.contains(&"/api/communication/gmail/write"));
        assert!(!AGENT_POST.contains(&"/api/communication/gchat/write"));
        assert!(!SLACK_METHODS.iter().any(|m| m.starts_with("chat.") || m.contains("delete") || m.contains("open")));
    }
}
