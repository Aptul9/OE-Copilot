//! The HTTP requests of the notifications (Web Push, FCM, ntfy), behind a function the tests replace.

use futures_util::future::LocalBoxFuture;
use std::rc::Rc;
use std::time::Duration;

#[derive(Debug)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

impl Request {
    pub fn post(url: &str, headers: Vec<(String, String)>, body: Vec<u8>, timeout_ms: u64) -> Self {
        Request { method: "POST", url: url.to_string(), headers, body, timeout: Duration::from_millis(timeout_ms) }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One request: the answer, whatever its status, or the text of the failure when none came ("Socket timeout" after
/// the timeout, as web-push says).
pub type Http = Rc<dyn Fn(Request) -> LocalBoxFuture<'static, Result<Response, String>>>;

/// Over the shared client of the crate.
pub fn reqwest_http() -> Http {
    Rc::new(|r: Request| {
        Box::pin(async move {
            let method = reqwest::Method::from_bytes(r.method.as_bytes()).map_err(|e| e.to_string())?;
            let mut b = crate::http_client().request(method, &r.url).timeout(r.timeout);
            for (k, v) in &r.headers {
                // set by the client from the body
                if !k.eq_ignore_ascii_case("content-length") {
                    b = b.header(k.as_str(), v.as_str());
                }
            }
            let failed = |e: reqwest::Error| if e.is_timeout() { "Socket timeout".to_string() } else { error_chain(&e) };
            let res = b.body(r.body).send().await.map_err(failed)?;
            let status = res.status().as_u16();
            let headers = res.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
            let body = res.bytes().await.map_err(failed)?.to_vec();
            Ok(Response { status, headers, body })
        })
    })
}

/// The error and its causes on one line: reqwest says "error sending request" and the reason below it.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        text = format!("{text}: {s}");
        source = s.source();
    }
    text
}
