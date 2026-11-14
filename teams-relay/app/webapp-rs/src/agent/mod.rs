//! The light agent of TeamsRelay in Rust (src/agent/light): a Teams account with no browser of its own. It reads the
//! chats and the Activity feed through the Teams services, sends the push notifications and runs the commands of the
//! web app, on the sign-in the account's browser run captured. Every light slot of the browsers container runs in one
//! process, the light host (host.rs), driven by the supervisor over the IPC channel of fork.

pub mod activity;
pub mod api;
pub mod browser;
pub mod calls;
pub mod chatsvc;
pub mod commands;
pub mod config;
pub mod detector;
pub mod devices;
pub mod forms;
pub mod host;
pub mod http;
pub mod ipc;
pub mod list;
pub mod mcas;
pub mod media;
pub mod run;
pub mod session;
pub mod slot;
pub mod status;
pub mod store;
pub mod teams;
pub mod token;
pub mod web;

pub use crate::push::notify::{Field, error_text, info, warn};

/// Why a step of the agent failed, in the kinds the agent tells apart.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    /// no refresh token, or Microsoft refused it: the account needs its browser run (SignedOut)
    SignedOut(String),
    /// the Microsoft session needs the browser renewal (NeedsSignIn)
    NeedsSignIn(String),
    /// a Teams service answered outside 2xx (HttpStatusError)
    Status {
        status: u16,
        what: String,
    },
    Other(String),
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::SignedOut(m) | Fail::NeedsSignIn(m) | Fail::Other(m) => f.write_str(m),
            Fail::Status { status, what } => write!(f, "{what}: HTTP {status}"),
        }
    }
}

pub type R<T> = Result<T, Fail>;

pub fn other(m: impl Into<String>) -> Fail {
    Fail::Other(m.into())
}

/// errorText of src/agent/log.ts: the first line, short.
pub fn text_of(e: &Fail) -> String {
    error_text(&e.to_string())
}

pub fn now_ms() -> i64 {
    crate::web::now_ms()
}

pub fn now_s() -> i64 {
    crate::web::now_s()
}

/// The claims of a JWT, an empty object when it is not one.
pub fn claims(token: &str) -> serde_json::Map<String, serde_json::Value> {
    let part = token.split('.').nth(1).unwrap_or("");
    match serde_json::from_slice::<serde_json::Value>(&crate::js::base64_decode_lenient(part)) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    }
}

/// `s.slice(0, n)` of JavaScript: at most n UTF-16 units.
pub fn cut(s: &str, n: usize) -> String {
    crate::push::notify::utf16_slice(s, 0, n)
}

/// `s.trim()` of JavaScript: white space and the BOM.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// Date.parse of the times Teams and Microsoft write (ISO 8601 with a zone, else the local time), in ms.
pub fn parse_date(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(t) = s.parse::<jiff::Timestamp>() {
        return Some(t.as_millisecond() as f64);
    }
    if let Ok(dt) = s.parse::<jiff::civil::DateTime>() {
        return dt.to_zoned(local_zone()).ok().map(|z| z.timestamp().as_millisecond() as f64);
    }
    if let Ok(d) = s.parse::<jiff::civil::Date>() {
        // a date alone is UTC in Date.parse
        return d.to_zoned(jiff::tz::TimeZone::UTC).ok().map(|z| z.timestamp().as_millisecond() as f64);
    }
    None
}

/// The time zone of the process (TZ), as Node takes it; UTC when unknown.
pub fn local_zone() -> jiff::tz::TimeZone {
    match std::env::var("TZ") {
        Ok(tz) if !tz.is_empty() => jiff::tz::TimeZone::get(tz.trim_start_matches(':')).unwrap_or(jiff::tz::TimeZone::UTC),
        _ => jiff::tz::TimeZone::try_system().unwrap_or(jiff::tz::TimeZone::UTC),
    }
}

/// crypto.randomUUID(): a version 4 UUID.
pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("random");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("random");
    b
}

/// A credential written for its owner only: mode 0600 on a new file, as fs.writeFileSync(file, data, { mode }).
pub fn write_private(file: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(file)?;
    f.write_all(data)
}

/// fs.chmodSync(file, 0o600)
pub fn chmod_private(file: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
pub(crate) mod testing;
