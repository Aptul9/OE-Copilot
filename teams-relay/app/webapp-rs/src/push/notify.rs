//! The rules of the notifications (src/agent/logic/notify.ts) and the log lines of the agent (src/agent/log.ts).

use std::collections::HashMap;

/// How long the push service keeps a notification for a phone that is offline or asleep, in seconds.
pub const PUSH_TTL: i64 = 86_400;

/// How long the push service keeps a push of a call still ringing, in seconds: later it could not be answered.
pub const CALL_TTL: i64 = 60;

/// The same text within this many seconds is pushed once.
pub const PUSH_DEDUP_SECONDS: f64 = 150.0;

/// Seconds before the next try of a push the push service could not take: 5xx, no answer, 429 without Retry-After.
const PUSH_RETRY_DELAYS: [i64; 3] = [5, 30, 120];

/// A longer Retry-After of a 429 is cut to this, in seconds.
const PUSH_RETRY_AFTER_MAX: f64 = 900.0;

/// Wait before trying a push again after this failed attempt (0 = the first send), or None: other 4xx are refusals the
/// same request would get again, and the attempts are over after the last delay.
pub fn push_retry_delay(status: Option<u16>, retry_after: Option<&str>, attempt: usize, now_ms: f64) -> Option<i64> {
    let delay = *PUSH_RETRY_DELAYS.get(attempt)?;
    match status {
        Some(429) => Some(retry_after_seconds(retry_after, now_ms).unwrap_or(delay)),
        None => Some(delay),
        Some(s) if s >= 500 => Some(delay),
        _ => None,
    }
}

/// Retry-After is seconds or an HTTP date (RFC 9110); "1.5" or "-5" are neither.
pub fn retry_after_seconds(value: Option<&str>, now_ms: f64) -> Option<i64> {
    let v = value?.trim();
    if v.is_empty() {
        return None;
    }
    let s = if v.bytes().all(|b| b.is_ascii_digit()) {
        v.parse::<f64>().ok()?
    } else if regex::Regex::new(r"(?i)[a-z]{3}").expect("fixed").is_match(v) {
        let at = jiff::fmt::rfc2822::parse(v).ok()?.timestamp().as_millisecond() as f64;
        (at - now_ms) / 1000.0
    } else {
        return None;
    };
    if !s.is_finite() {
        return None;
    }
    Some(s.ceil().clamp(1.0, PUSH_RETRY_AFTER_MAX) as i64)
}

/// The notifications of one chat share this tag on the device.
pub fn chat_tag(slot: i64, chat: &str) -> String {
    format!("chat-{slot}-{chat}")
}

/// A call has one notification per account on the device: ringing, then ended.
pub fn call_tag(slot: i64) -> String {
    format!("call-{slot}")
}

/// The texts pushed lately, by their first 60 characters, lower case.
#[derive(Default)]
pub struct RecentPushes {
    seen: HashMap<String, f64>,
}

impl RecentPushes {
    pub fn new() -> Self {
        Self::default()
    }

    /// `now_ms`: the clock of the notifier.
    pub fn allow(&mut self, body: &str, now_ms: f64) -> bool {
        let key = utf16_slice(&body.trim().to_lowercase(), 0, 60);
        let now = now_ms / 1000.0;
        self.seen.retain(|_, at| now - *at <= PUSH_DEDUP_SECONDS);
        if key.is_empty() {
            return true;
        }
        if self.seen.contains_key(&key) {
            return false;
        }
        self.seen.insert(key, now);
        true
    }
}

/// When the owner has more Teams accounts, the notification says which one: organization, otherwise email.
pub fn account_label(owner_has_many: bool, me: &crate::state::Identity, slot: i64) -> String {
    if !owner_has_many {
        return String::new();
    }
    if !me.tenant.is_empty() {
        me.tenant.clone()
    } else if !me.email.is_empty() {
        me.email.clone()
    } else {
        format!("account {slot}")
    }
}

pub fn push_title(title: &str, label: &str) -> String {
    let t = if title.is_empty() { "TeamsRelay" } else { title };
    if label.is_empty() { t.to_string() } else { format!("{t} \u{b7} {label}") }
}

/// `s.slice(from, to)` of JavaScript, by UTF-16 units; a pair of surrogates cut in two loses its half.
pub fn utf16_slice(s: &str, from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut at = 0;
    for c in s.chars() {
        let n = c.len_utf16();
        if at >= from && at + n <= to {
            out.push(c);
        }
        at += n;
        if at >= to {
            break;
        }
    }
    out
}

/// A value of a log line.
#[derive(Clone, Debug)]
pub enum Field {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl From<&str> for Field {
    fn from(v: &str) -> Self {
        Field::Str(v.to_string())
    }
}

impl From<String> for Field {
    fn from(v: String) -> Self {
        Field::Str(v)
    }
}

impl From<usize> for Field {
    fn from(v: usize) -> Self {
        Field::Num(v as f64)
    }
}

impl From<i64> for Field {
    fn from(v: i64) -> Self {
        Field::Num(v as f64)
    }
}

impl From<u16> for Field {
    fn from(v: u16) -> Self {
        Field::Num(f64::from(v))
    }
}

impl From<bool> for Field {
    fn from(v: bool) -> Self {
        Field::Bool(v)
    }
}

const MAX: usize = 120;

fn show(v: &Field) -> String {
    match v {
        Field::Num(n) => crate::js::number_to_string(*n),
        Field::Bool(b) => b.to_string(),
        Field::Str(s) => {
            let s = if s.encode_utf16().count() > MAX { format!("{}...", utf16_slice(s, 0, MAX)) } else { s.clone() };
            let plain = !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c == '"' || c == '=');
            if plain { s } else { crate::js::stringify(&serde_json::Value::String(s)) }
        }
    }
}

/// One line per event: "<prefix>: <message> key=value ...".
pub fn format(prefix: &str, message: &str, fields: &[(&str, Field)]) -> String {
    let mut parts = vec![format!("{prefix}:")];
    if !message.is_empty() {
        parts.push(message.to_string());
    }
    parts.extend(fields.iter().map(|(k, v)| format!("{k}={}", show(v))));
    parts.join(" ")
}

/// First line of an error, short.
pub fn error_text(e: &str) -> String {
    utf16_slice(e.split('\n').next().unwrap_or(""), 0, MAX)
}

pub fn info(prefix: &str, message: &str, fields: &[(&str, Field)]) {
    println!("{}", format(prefix, message, fields));
}

pub fn warn(prefix: &str, message: &str, fields: &[(&str, Field)]) {
    eprintln!("{}", format(prefix, message, fields));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Identity;

    #[test]
    fn dedup() {
        let mut r = RecentPushes::new();
        let mut now = 1_000_000.0;
        assert!(r.allow("Are you there?", now));
        now += 10_000.0;
        assert!(!r.allow("  are you THERE?  ", now));
        now += PUSH_DEDUP_SECONDS * 1000.0;
        assert!(r.allow("Are you there?", now));
        let mut r = RecentPushes::new();
        assert!(r.allow(&format!("{} first", "x".repeat(60)), 0.0));
        assert!(!r.allow(&format!("{} second", "x".repeat(60)), 0.0));
        assert!(r.allow("", 0.0));
        assert!(r.allow("   ", 0.0));
        assert!(r.allow("   ", 0.0));
    }

    #[test]
    fn title_and_label() {
        let me = Identity { name: "Anna Rossi".into(), email: "anna.rossi@contoso.example".into(), tenant: "Contoso".into(), ..Default::default() };
        assert_eq!(account_label(false, &me, 2), "");
        assert_eq!(account_label(true, &me, 2), "Contoso");
        let no_tenant = Identity { tenant: String::new(), ..me.clone() };
        assert_eq!(account_label(true, &no_tenant, 2), "anna.rossi@contoso.example");
        let nothing = Identity { email: String::new(), ..no_tenant };
        assert_eq!(account_label(true, &nothing, 2), "account 2");
        assert_eq!(push_title("Anna Rossi", "Contoso"), "Anna Rossi \u{b7} Contoso");
        assert_eq!(push_title("Anna Rossi", "Contoso").as_bytes()[10..14], [b' ', 0xc2, 0xb7, b' ']);
        assert_eq!(push_title("", ""), "TeamsRelay");
        assert_eq!(chat_tag(2, "Anna Rossi"), "chat-2-Anna Rossi");
        assert_eq!(call_tag(3), "call-3");
    }

    #[test]
    fn retry_delays() {
        let now = jiff::Timestamp::from_second(1_790_452_800).expect("ts").as_millisecond() as f64;
        assert_eq!(jiff::Timestamp::from_second(1_790_452_800).expect("ts").to_string(), "2026-09-26T20:00:00Z");
        let d: Vec<_> = (0..4).map(|a| push_retry_delay(Some(503), None, a, now)).collect();
        assert_eq!(d, [Some(5), Some(30), Some(120), None]);
        assert_eq!(push_retry_delay(Some(500), None, 0, now), Some(5));
        assert_eq!(push_retry_delay(None, None, 1, now), Some(30));
        assert_eq!(push_retry_delay(Some(429), Some("42"), 0, now), Some(42));
        assert_eq!(push_retry_delay(Some(429), Some("0"), 0, now), Some(1));
        assert_eq!(push_retry_delay(Some(429), Some("86400"), 0, now), Some(900));
        assert_eq!(push_retry_delay(Some(429), Some("Sat, 26 Sep 2026 20:01:00 GMT"), 0, now), Some(60));
        assert_eq!(push_retry_delay(Some(429), Some("soon"), 1, now), Some(30));
        assert_eq!(push_retry_delay(Some(429), Some("1.5"), 0, now), Some(5));
        assert_eq!(push_retry_delay(Some(429), Some("-5"), 0, now), Some(5));
        assert_eq!(push_retry_delay(Some(429), None, 2, now), Some(120));
        assert_eq!(push_retry_delay(Some(429), Some("42"), 3, now), None);
        for s in [400, 401, 403, 404, 410, 413] {
            assert_eq!(push_retry_delay(Some(s), None, 0, now), None);
        }
    }

    #[test]
    fn log_lines() {
        assert_eq!(format("push", "sent on retry", &[("attempt", 1usize.into()), ("fcm", true.into())]), "push: sent on retry attempt=1 fcm=true");
        assert_eq!(format("push", "x", &[("retry", "none".into()), ("e", "a b".into()), ("z", "".into())]), r#"push: x retry=none e="a b" z="""#);
        assert_eq!(format("ntfy", "", &[]), "ntfy:");
        assert_eq!(error_text("first\nsecond"), "first");
    }
}
