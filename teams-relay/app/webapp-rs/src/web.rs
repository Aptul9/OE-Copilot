//! Requests, answers and errors of the routes. An error keeps the shape of the TypeScript web app: {"detail": "..."}
//! (src/lib/http.ts), a slot whose database the agent has not made yet is a 503.

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full, combinators::UnsyncBoxBody};
use hyper::header::HeaderMap;
use serde_json::{Map, Value, json};
use std::convert::Infallible;

pub type Body = UnsyncBoxBody<Bytes, Infallible>;
pub type Resp = hyper::Response<Body>;

#[derive(Debug)]
pub enum Error {
    /// HttpError: a status, its reason and extra headers
    Http(u16, String, Vec<(String, String)>),
    /// SlotNotReady
    NotReady,
    /// anything else: logged, answered 500 "Internal error"
    Internal(String),
}

pub type Res<T = Resp> = Result<T, Error>;

pub fn http<T>(status: u16, detail: impl Into<String>) -> Res<T> {
    Err(Error::Http(status, detail.into(), vec![]))
}

pub fn err(status: u16, detail: impl Into<String>) -> Error {
    Error::Http(status, detail.into(), vec![])
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Internal(format!("sqlite: {e}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Internal(format!("io: {e}"))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(s, d, _) => write!(f, "{s} {d}"),
            Error::NotReady => write!(f, "Account not ready yet"),
            Error::Internal(m) => write!(f, "{m}"),
        }
    }
}

/// The answer of an error, as route() of src/lib/http.ts gives it.
pub fn error_response(e: Error) -> Resp {
    match e {
        Error::Http(status, detail, headers) => {
            let mut r = json_with(status, &json!({ "detail": detail }));
            for (k, v) in headers {
                if let (Ok(k), Ok(v)) = (hyper::header::HeaderName::from_bytes(k.as_bytes()), hyper::header::HeaderValue::from_str(&v)) {
                    r.headers_mut().insert(k, v);
                }
            }
            r
        }
        Error::NotReady => json_with(503, &json!({ "detail": "Account not ready yet" })),
        Error::Internal(m) => {
            eprintln!("{m}");
            json_with(500, &json!({ "detail": "Internal error" }))
        }
    }
}

/// One request: what a route reads of it.
pub struct Req {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    /// path parameters of the route ([slot], [id], [file], [slug])
    pub params: Vec<(String, String)>,
    /// the body left unread for a route that streams it (server::take_stream)
    pub stream_id: Option<u64>,
}

impl Req {
    /// Headers.get of Fetch: values of the same header joined with ", " (the cookie with "; ").
    pub fn header(&self, name: &str) -> Option<String> {
        let values: Vec<&str> = self.headers.get_all(name).iter().filter_map(|v| v.to_str().ok()).collect();
        if values.is_empty() {
            return None;
        }
        Some(values.join(if name.eq_ignore_ascii_case("cookie") { "; " } else { ", " }))
    }

    /// URLSearchParams.get: the first value, decoded.
    pub fn query_param(&self, name: &str) -> Option<String> {
        form_urlencoded::parse(self.query.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
    }

    pub fn query_pairs(&self) -> Vec<(String, String)> {
        form_urlencoded::parse(self.query.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }

    pub fn param(&self, name: &str) -> String {
        self.params.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    /// The URL of the request as Next.js sees it behind the proxy: the path and query, on the address of the app.
    pub fn url(&self) -> String {
        format!("{}{}{}", self.path, if self.query.is_empty() { "" } else { "?" }, self.query)
    }

    /// body() of src/lib/http.ts: a JSON object, or 400 "Invalid JSON body".
    pub fn json(&self) -> Res<Map<String, Value>> {
        let text = String::from_utf8_lossy(&self.body);
        match serde_json::from_str::<Value>(text.strip_prefix('\u{feff}').unwrap_or(&text)) {
            Ok(Value::Object(m)) => Ok(m),
            _ => http(400, "Invalid JSON body"),
        }
    }

    /// Content-Length of the request, None without one (NaN when unreadable, as Number() of a bad header).
    pub fn declared_length(&self) -> Option<f64> {
        self.header("content-length").map(|v| crate::js::number(Some(&Value::String(v))))
    }
}

/// text() of src/lib/http.ts: a string of at most `max` UTF-16 units, or 400.
pub fn text(v: Option<&Value>, field: &str, max: usize) -> Res<String> {
    let Some(Value::String(s)) = v else { return http(400, format!("Missing {field}")) };
    if s.encode_utf16().count() > max {
        return http(400, format!("{field} too long"));
    }
    Ok(s.clone())
}

pub fn json_with(status: u16, v: &Value) -> Resp {
    hyper::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(crate::js::stringify(v))).boxed_unsync())
        .expect("response")
}

/// Response.json(v): 200, application/json.
pub fn json(v: Value) -> Res {
    Ok(json_with(200, &v))
}

pub fn ok() -> Res {
    json(json!({ "ok": true }))
}

pub fn empty(status: u16) -> Resp {
    hyper::Response::builder().status(status).body(Empty::new().boxed_unsync()).expect("response")
}

pub fn redirect(location: &str) -> Res {
    Ok(hyper::Response::builder().status(302).header("location", location).body(Empty::new().boxed_unsync()).expect("response"))
}

pub fn bytes(status: u16, headers: &[(&str, String)], data: Vec<u8>) -> Resp {
    let mut b = hyper::Response::builder().status(status);
    for (k, v) in headers {
        b = b.header(*k, v.as_str());
    }
    b.body(Full::new(Bytes::from(data)).boxed_unsync()).expect("response")
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A number as JSON.stringify writes it: integral values without a fraction.
pub fn num(f: f64) -> Value {
    if f.is_finite() && f == f.trunc() && f.abs() < 9.007_199_254_740_992e15 {
        json!(f as i64)
    } else if f.is_finite() {
        json!(f)
    } else {
        Value::Null
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// nowSeconds() of the agent: Unix seconds, rounded down.
pub fn now_s() -> i64 {
    now_ms().div_euclid(1000)
}
