//! The requests of a light slot: its own connections, as each account had in a process of its own (src/agent/light/
//! connections.ts), behind the function of push/http.rs the tests replace; the slow ones logged
//! (src/agent/light/slow-requests.ts).

use super::{Fail, R, warn};
pub use crate::push::http::{Http, Request, Response};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Requests that took this long are logged, with when their answer started.
const SLOW_MS: u128 = 10_000;

/// A client of its own for one slot: no redirects, rustls with the webpki roots.
pub fn slot_http() -> Http {
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(10)).build().expect("HTTP client");
    Rc::new(move |r: Request| {
        let client = client.clone();
        Box::pin(async move {
            let started = Instant::now();
            let what = format!("{} {}", r.method, origin_and_path(&r.url));
            let method = reqwest::Method::from_bytes(r.method.as_bytes()).map_err(|e| e.to_string())?;
            let mut b = client.request(method, &r.url).timeout(r.timeout);
            for (k, v) in &r.headers {
                if !k.eq_ignore_ascii_case("content-length") {
                    b = b.header(k.as_str(), v.as_str());
                }
            }
            let failed = |e: reqwest::Error| if e.is_timeout() { "The operation was aborted due to timeout".to_string() } else { chain(&e) };
            let slow = |answer: Option<u128>, error: Option<&str>| {
                let took = started.elapsed().as_millis();
                if took >= SLOW_MS {
                    let answer = answer.map_or("none".to_string(), |a| format!("{a}ms"));
                    let error = error.map(|e| format!(" error={}", super::error_text(e))).unwrap_or_default();
                    warn("fetch", &format!("slow {what} after {took} ms answer={answer}{error}"), &[]);
                }
            };
            let res = match b.body(r.body).send().await {
                Ok(res) => res,
                Err(e) => {
                    let e = failed(e);
                    slow(None, Some(&e));
                    return Err(e);
                }
            };
            let answer = started.elapsed().as_millis();
            let status = res.status().as_u16();
            let headers = res.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
            let body = match res.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    let e = failed(e);
                    slow(Some(answer), Some(&e));
                    return Err(e);
                }
            };
            slow(Some(answer), None);
            Ok(Response { status, headers, body })
        })
    })
}

fn origin_and_path(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => format!("{}{}", u.origin().ascii_serialization(), u.path()),
        Err(_) => url.split('?').next().unwrap_or("").to_string(),
    }
}

/// The error and its causes on one line.
fn chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        text = format!("{text}: {s}");
        source = s.source();
    }
    text
}

/// One request; a request that got no answer is a failure with the text of why.
pub async fn send(http: &Http, method: &'static str, url: &str, headers: Vec<(&str, String)>, body: Vec<u8>, timeout_ms: u64) -> R<Response> {
    let headers = headers.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    http(Request { method, url: url.to_string(), headers, body, timeout: Duration::from_millis(timeout_ms) }).await.map_err(Fail::Other)
}

/// The JSON of an answer, as `await r.json()`: a body that is not JSON is a failure.
pub fn json(r: &Response) -> R<serde_json::Value> {
    serde_json::from_slice(&r.body).map_err(|e| Fail::Other(format!("Unexpected token in JSON: {e}")))
}

/// `await r.json().catch(() => ({}))`
pub fn json_or_empty(r: &Response) -> serde_json::Value {
    serde_json::from_slice(&r.body).unwrap_or_else(|_| serde_json::json!({}))
}

pub fn text(r: &Response) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

/// Every value of a header the answer carries more than once (Set-Cookie).
pub fn all<'a>(r: &'a Response, name: &str) -> Vec<&'a str> {
    r.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect()
}
