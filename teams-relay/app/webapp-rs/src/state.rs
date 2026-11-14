//! The state rows of data/N/messages.db and their schemas (src/shared/slot-db/state.ts): every field falls back on its
//! own (zod .catch), a value that is not an object gives the fallback, keys come out in schema order.

use serde_json::{Map, Value, json};

pub mod key {
    pub const HEALTH: &str = "health";
    pub const PRESENCE: &str = "presence";
    pub const LOGIN_WATCH: &str = "login_watch";
    pub const ACTIVE_CHAT: &str = "active_chat";
    pub const VIEWING: &str = "viewing";
    pub const ME: &str = "me";
    pub const ACTIVITY_TS: &str = "activity_ts";
    pub const CHECK_SEEN: &str = "check_seen";
    pub const CALL: &str = "call";
    pub const IN_CALL: &str = "in_call";
    pub const RELAY: &str = "relay";
    pub const DESKTOP: &str = "desktop";
    pub const CALL_AUDIO: &str = "call_audio";
    pub const LIGHT_TOKEN: &str = "light_token";
    pub const STATUS_NAMES: &str = "status_names";
}

pub fn cmd_result_key(id: i64) -> String {
    format!("cmd_result:{id}")
}

pub fn members_key(chat: &str) -> String {
    format!("members:{chat}")
}

pub const CALL_FRESH_FOR: f64 = 10.0;

/// z.string().catch(d)
pub fn s(o: &Map<String, Value>, k: &str, d: &str) -> String {
    o.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
}

/// z.number().catch(d)
pub fn n(o: &Map<String, Value>, k: &str, d: f64) -> f64 {
    o.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// z.boolean().catch(d)
pub fn b(o: &Map<String, Value>, k: &str, d: bool) -> bool {
    o.get(k).and_then(Value::as_bool).unwrap_or(d)
}

/// z.number() kept as JSON (an integer stays one)
fn nv(o: &Map<String, Value>, k: &str) -> Value {
    match o.get(k) {
        Some(v @ Value::Number(_)) => v.clone(),
        _ => json!(0),
    }
}

/// parseState: JSON of a row; missing, broken or not an object gives None (the caller's fallback).
pub fn object(raw: Option<&str>) -> Option<Map<String, Value>> {
    let raw = raw.filter(|r| !r.is_empty())?;
    match serde_json::from_str::<Value>(raw).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub name: String,
    pub email: String,
    pub tenant: String,
    pub av: String,
    pub logo: String,
}

impl Identity {
    pub fn parse(raw: Option<&str>) -> Self {
        let o = object(raw).unwrap_or_default();
        Identity { name: s(&o, "name", ""), email: s(&o, "email", ""), tenant: s(&o, "tenant", ""), av: s(&o, "av", ""), logo: s(&o, "logo", "") }
    }
    pub fn json(&self) -> Value {
        json!({ "name": self.name, "email": self.email, "tenant": self.tenant, "av": self.av, "logo": self.logo })
    }
}

/// The computer of the relay of an account on another computer and its last sync.
pub struct RelayLink {
    pub host: String,
    pub seen: Value,
}

impl RelayLink {
    pub fn parse(raw: Option<&str>) -> Self {
        let o = object(raw).unwrap_or_default();
        RelayLink { host: s(&o, "host", ""), seen: nv(&o, "seen") }
    }
}

pub struct Viewing {
    pub chat: String,
    pub ts: f64,
}

impl Viewing {
    pub fn parse(raw: Option<&str>) -> Self {
        let o = object(raw).unwrap_or_default();
        Viewing { chat: s(&o, "chat", ""), ts: n(&o, "ts", 0.0) }
    }
}

/// The incoming call (since, seen: ms on the wall clock).
#[derive(Clone, Debug)]
pub struct CallState {
    pub caller: String,
    pub since: Value,
    pub seen: f64,
    pub ringing: bool,
}

impl CallState {
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        let o = object(raw)?;
        Some(CallState { caller: s(&o, "caller", ""), since: nv(&o, "since"), seen: n(&o, "seen", 0.0), ringing: b(&o, "ringing", false) })
    }
}

/// The call in progress; muted absent while the agent cannot read it.
#[derive(Clone, Debug)]
pub struct InCall {
    pub caller: String,
    pub since: Value,
    pub seen: f64,
    pub active: bool,
    pub muted: Option<bool>,
}

impl InCall {
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        let o = object(raw)?;
        Some(InCall {
            caller: s(&o, "caller", ""),
            since: nv(&o, "since"),
            seen: n(&o, "seen", 0.0),
            active: b(&o, "active", false),
            muted: o.get("muted").and_then(Value::as_bool),
        })
    }
}

/// The call the account rings with now: caller and since.
pub fn ringing_call(c: Option<&CallState>, now_ms: f64) -> Option<(String, Value)> {
    let c = c?;
    if !c.ringing || now_ms - c.seen > CALL_FRESH_FOR * 1000.0 {
        return None;
    }
    Some((c.caller.clone(), c.since.clone()))
}

/// The call in progress, if the agent saw it a moment ago: caller, since, muted.
pub fn in_call_of(c: Option<&InCall>, now_ms: f64) -> Option<(String, Value, Option<bool>)> {
    let c = c?;
    if !c.active || now_ms - c.seen > CALL_FRESH_FOR * 1000.0 {
        return None;
    }
    Some((c.caller.clone(), c.since.clone(), c.muted))
}

/// The sign-in of a light account (src/shared/slot-db/state.ts LightSignIn).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightSignIn {
    pub at: f64,
    pub expires_on: f64,
    pub refused: bool,
    pub session: Option<bool>,
    pub browser: Option<bool>,
    pub mcas: Option<bool>,
    pub broker: Option<bool>,
}

impl LightSignIn {
    /// An object that is not one, or an optional field of the wrong type, fails the whole parse: the fallback.
    pub fn parse_value(v: &Value) -> Self {
        let Value::Object(o) = v else { return Self::default() };
        let opt = |k: &str| -> Result<Option<bool>, ()> {
            match o.get(k) {
                None => Ok(None),
                Some(Value::Bool(b)) => Ok(Some(*b)),
                Some(_) => Err(()),
            }
        };
        let (Ok(session), Ok(browser), Ok(mcas), Ok(broker)) = (opt("session"), opt("browser"), opt("mcas"), opt("broker")) else { return Self::default() };
        LightSignIn { at: n(o, "at", 0.0), expires_on: n(o, "expiresOn", 0.0), refused: b(o, "refused", false), session, browser, mcas, broker }
    }
}

/// Members of a chat; Members.parse throws on a value that is not an object.
pub fn members(v: &Value) -> Option<Value> {
    let Value::Object(o) = v else { return None };
    let names: Vec<Value> = match o.get("names") {
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => a.clone(),
        _ => vec![],
    };
    let ts = match o.get("ts") {
        Some(v @ Value::Number(_)) => v.clone(),
        _ => json!(0),
    };
    Some(json!({ "ts": ts, "names": names }))
}

/// healthOf: the agent's row, plus whether the agent still writes it. Older than a minute, it no longer describes
/// reality; `added` is when the slot was switched on.
pub fn health_of(saved: &Value, added: i64, now_s: f64) -> Map<String, Value> {
    let mut h = match saved {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let ts = crate::js::number(h.get("ts"));
    let ts = if ts.is_nan() { 0.0 } else { ts };
    let agent_ok = now_s - ts < 60.0;
    crate::js::set(&mut h, "agent", Some(json!(if agent_ok { "ok" } else { "stale" })));
    if !agent_ok {
        let starting = now_s - (added as f64) < 180.0;
        crate::js::set(&mut h, "teams", Some(json!(if starting { "starting" } else { "unknown" })));
        crate::js::set(&mut h, "watcher", Some(json!("stale")));
        crate::js::set(&mut h, "overall", Some(json!(if starting { "yellow" } else { "red" })));
    }
    if h.get("overall").is_none_or(Value::is_null) {
        crate::js::set(&mut h, "overall", Some(json!("yellow")));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health() {
        let h = health_of(&json!({"cdp": "ok", "ts": 100, "teams": "ok", "overall": "green"}), 0, 130.0);
        assert_eq!(Value::Object(h).to_string(), r#"{"cdp":"ok","ts":100,"teams":"ok","overall":"green","agent":"ok"}"#);
        let h = health_of(&json!({"ts": 100, "teams": "ok", "overall": "green"}), 50, 200.0);
        assert_eq!(Value::Object(h).to_string(), r#"{"ts":100,"teams":"starting","overall":"yellow","agent":"stale","watcher":"stale"}"#);
        let h = health_of(&json!({}), 0, 1000.0);
        assert_eq!(Value::Object(h).to_string(), r#"{"agent":"stale","teams":"unknown","watcher":"stale","overall":"red"}"#);
    }

    #[test]
    fn schemas() {
        assert_eq!(Identity::parse(Some(r#"{"name":5,"email":"a","x":1}"#)).json().to_string(), r#"{"name":"","email":"a","tenant":"","av":"","logo":""}"#);
        assert_eq!(Identity::parse(Some("[1]")), Identity::default());
        assert!(CallState::parse(Some("nope")).is_none());
        assert_eq!(LightSignIn::parse_value(&json!({"at": 5, "session": "x"})), LightSignIn::default());
        assert_eq!(LightSignIn::parse_value(&json!({"at": 5, "broker": true})).broker, Some(true));
        assert_eq!(members(&json!({"ts": 3, "names": ["a", 1]})), Some(json!({"ts": 3, "names": []})));
    }
}
