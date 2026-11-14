//! /api/call/*: answer, hang up, mute and place Teams calls from the app, and who may open the socket of their sound.

use crate::appdb;
use crate::auth::require_slot;
use crate::commands::{chat_name, queue_once};
use crate::route;
use crate::server::Route;
use crate::slotdb::with_slot;
use crate::state::{in_call_of, key, ringing_call};
use crate::web::{Req, Res, err, json, now_ms};
use serde_json::{Value, json};

pub fn routes() -> Vec<Route> {
    vec![
        route!("POST", "/api/call/answer", answer),
        route!("GET", "/api/call/audio", audio),
        route!("POST", "/api/call/hangup", hangup),
        route!("POST", "/api/call/mute", mute),
        route!("POST", "/api/call/start", start),
    ]
}

fn call_audio(slot: i64) -> Res<bool> {
    with_slot(slot, |r| Ok(r.state(key::CALL_AUDIO, json!(0)) == json!(1)))
}

/// The sound goes to the app: an account on another computer, or a light account whose relay of the sound runs.
fn audio_to_app(slot: i64) -> Res<bool> {
    Ok(appdb::slot_row(slot)?.is_some_and(|s| s.relay != 0) || call_audio(slot)?)
}

/// {since}: the call ringing now, by when it started ringing.
async fn answer(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let b = req.json()?;
    let since = b
        .get("since")
        .and_then(Value::as_f64)
        .filter(|s| s.fract() == 0.0 && *s > 0.0 && *s <= 9_007_199_254_740_991.0)
        .ok_or_else(|| err(400, "Missing call"))?;
    let ringing = with_slot(slot, |r| Ok(ringing_call(r.call().as_ref(), now_ms() as f64)))?;
    let Some((caller, ring_since)) = ringing.filter(|(_, s)| s.as_f64() == Some(since)) else { return Err(err(409, "Call no longer ringing")) };
    let relay = audio_to_app(slot)?;
    let args = if relay && b.get("audio") == Some(&Value::Bool(true)) { json!({ "since": ring_since, "audio": true }) } else { json!({ "since": ring_since }) };
    let id = queue_once(slot, "answer", &caller, &crate::js::stringify(&args))?;
    json(json!({ "ok": true, "id": id, "desktop": if relay { Value::Null } else { json!(crate::config::desktop_url_of(slot)) } }))
}

/// Who may open the socket of the sound of a call: the relay by its token, or the page of the owner by its session from
/// the site of the app only. (slot, side)
pub fn audio_side(req: &Req) -> Res<(i64, &'static str)> {
    if req.headers.contains_key("authorization") {
        let a = req.query_param("a").unwrap_or_default();
        let slot = crate::js::number(Some(&Value::String(a.clone())));
        let slot = if req.query_param("a").is_none() { 0.0 } else { slot };
        if slot.fract() == 0.0 && slot > 0.0 && slot <= 9_007_199_254_740_991.0 && !req.headers.contains_key("origin") {
            let slot = slot as i64;
            if let Some(row) = appdb::slot_row(slot)?
                && row.light != 0
                && row.stopped == 0
                && call_audio(slot)?
            {
                let file = crate::config::data_dir().join(slot.to_string()).join("calling-audio-token");
                if let Ok(expected) = std::fs::read_to_string(file) {
                    let expected = expected.trim();
                    let auth = req.header("authorization").unwrap_or_default();
                    let got = auth.strip_prefix("Bearer ").unwrap_or(&auth);
                    if expected.len() >= 32 && got.len() == expected.len() && got.bytes().zip(expected.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0 {
                        return Ok((slot, "relay"));
                    }
                }
            }
        }
        return Ok((crate::relay::require_relay(req)?.slot, "relay"));
    }
    let app = url::Url::parse(&crate::config::app_url()).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
    if req.header("origin").as_deref() != Some(app.as_str()) {
        return Err(err(403, "Not from the app"));
    }
    let (_, slot) = require_slot(req)?;
    if !audio_to_app(slot)? {
        return Err(err(409, "The sound of this account goes through its remote desktop"));
    }
    Ok((slot, "app"))
}

async fn audio(req: Req) -> Res {
    let (slot, side) = audio_side(&req)?;
    json(json!({ "slot": slot, "side": side }))
}

async fn hangup(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let call = with_slot(slot, |r| Ok(in_call_of(r.in_call().as_ref(), now_ms() as f64)))?;
    let Some((caller, _, _)) = call else { return Err(err(409, "No call in progress")) };
    json(json!({ "ok": true, "id": queue_once(slot, "hangup", &caller, "")? }))
}

/// {on}: Teams' own mute of the call in progress.
async fn mute(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let b = req.json()?;
    let Some(on) = b.get("on").and_then(Value::as_bool) else { return Err(err(400, "Missing mute state")) };
    let call = with_slot(slot, |r| Ok(in_call_of(r.in_call().as_ref(), now_ms() as f64)))?;
    let Some((caller, _, _)) = call else { return Err(err(409, "No call in progress")) };
    json(json!({ "ok": true, "id": queue_once(slot, "mute", &caller, &crate::js::stringify(&json!({ "on": on })))? }))
}

/// {name}: a Teams audio call to the person of that 1:1 chat, while no call rings or runs on the account.
async fn start(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let b = req.json()?;
    let name = chat_name(b.get("name"))?;
    let now = now_ms() as f64;
    let (busy, kind) = with_slot(slot, |r| {
        let busy = ringing_call(r.call().as_ref(), now).is_some() || in_call_of(r.in_call().as_ref(), now).is_some();
        let kind = r
            .chats()
            .into_iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .and_then(|c| c.get("kind").and_then(Value::as_str).map(String::from))
            .unwrap_or_default();
        Ok((busy, kind))
    })?;
    if busy {
        return Err(err(409, "A call is on: end it first"));
    }
    if kind != "one" || name.to_lowercase().contains("(you)") {
        return Err(err(409, "Only a 1:1 chat can be called"));
    }
    let relay = audio_to_app(slot)?;
    let arg2 = if relay && b.get("audio") == Some(&Value::Bool(true)) { crate::js::stringify(&json!({ "audio": true })) } else { String::new() };
    let id = queue_once(slot, "call", &name, &arg2)?;
    json(json!({ "ok": true, "id": id, "desktop": if relay { Value::Null } else { json!(crate::config::desktop_url_of(slot)) } }))
}
