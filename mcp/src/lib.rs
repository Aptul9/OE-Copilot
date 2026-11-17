//! Communication MCP: one remote MCP endpoint for mail, calendars and chats (Outlook, Gmail, Google Chat, Teams, Slack).
//! Rust rewrite of the TypeScript server in ts/, same tools, schemas, handles and answers.

pub mod calendar;
pub mod communication;
pub mod everflow;
pub mod gateway;
pub mod gmail;
pub mod html;
pub mod http;
pub mod identity;
pub mod js;
pub mod mail;
pub mod mcp;
pub mod outlook;
pub mod schema;
pub mod send;
pub mod slack;
pub mod teams;
pub mod tokens;
pub mod url;

use std::fmt;

/// A failure a tool reports. `Comm` carries a code and message the client sees; `Provider` is any other failure, shown
/// only as the generic PROVIDER_ERROR (its text may quote mail content and stays out of the answer).
#[derive(Debug)]
pub enum Error {
    Comm { code: &'static str, message: String },
    Provider(String),
}

impl Error {
    pub fn comm(code: &'static str, message: impl Into<String>) -> Self {
        Error::Comm { code, message: message.into() }
    }
    pub fn provider(message: impl Into<String>) -> Self {
        Error::Provider(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Comm { code, message } => write!(f, "{code}: {message}"),
            Error::Provider(m) => write!(f, "{m}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn unsupported<T>(provider: &str, operation: &str) -> Result<T> {
    Err(Error::comm("UNSUPPORTED_CAPABILITY", format!("{provider} does not support {operation}")))
}

/// Shared HTTP client: no redirects (a credential would follow them), rustls with the webpki roots.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(std::time::Duration::from_secs(15)).build().expect("HTTP client")
    })
}

/// A response body as text, read in chunks; past `max_bytes` it is an error, never a cut body.
pub async fn text_body(mut r: reqwest::Response, max_bytes: usize) -> Result<String> {
    let mut out: Vec<u8> = vec![];
    while let Some(chunk) = r.chunk().await.map_err(|e| Error::provider(format!("response body: {e}")))? {
        if out.len() + chunk.len() > max_bytes {
            return Err(Error::comm("TOO_LARGE", "answer too large"));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The JSON of a response body, or None when it is not JSON.
pub async fn json_body(r: reqwest::Response) -> Option<serde_json::Value> {
    let bytes = r.bytes().await.ok()?;
    serde_json::from_slice(&bytes).ok()
}
