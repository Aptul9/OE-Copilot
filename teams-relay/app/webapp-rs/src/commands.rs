//! Commands for the agent of an account (src/lib/commands.ts, src/shared/command-input.ts, src/lib/mentions.ts):
//! rows in data/N/messages.db, refused while nothing would run them.

use crate::appdb;
use crate::slotdb::with_slot;
use crate::state::key;
use crate::web::{Res, err};
use serde_json::{Map, Value, json};

pub const REACTIONS: [&str; 6] = ["like", "heart", "laugh", "surprised", "cry", "angry"];
/// Commands a light account cannot run: they need the Teams page or its calls.
const LIGHT_UNSUPPORTED: [&str; 5] = ["download", "call", "answer", "hangup", "mute"];

/// Why the account takes no command, None when it takes them.
pub fn idle_reason(slot: i64) -> Res<Option<String>> {
    let s = appdb::slot_row(slot)?;
    if s.as_ref().is_some_and(|s| s.stopped != 0) {
        return Ok(Some("This Teams account is stopped: start it from its page or from Settings".into()));
    }
    if s.as_ref().is_some_and(|s| s.check_every != 0) {
        return Ok(Some("This Teams account runs only during its checks: set it to always on in Settings to act on Teams".into()));
    }
    if s.as_ref().is_some_and(|s| s.relay != 0) {
        let (agent, host) = with_slot(slot, |r| Ok((r.health(0).get("agent").cloned(), r.relay_link().host)))?;
        if agent != Some(json!("ok")) {
            let on = if host.is_empty() { String::new() } else { format!(" on {host}") };
            return Ok(Some(format!("The relay of this Teams account{on} is not connected: start it on that computer")));
        }
    }
    Ok(None)
}

fn takes_commands(slot: i64, kind: &str) -> Res<()> {
    if let Some(why) = idle_reason(slot)? {
        return Err(err(409, why));
    }
    let s = appdb::slot_row(slot)?;
    if let Some(s) = s
        && s.light != 0
        && s.refreshing == 0
        && LIGHT_UNSUPPORTED.contains(&kind)
        && (kind == "download" || !with_slot(slot, |r| Ok(r.state(key::CALL_AUDIO, json!(0)) == json!(1)))?)
    {
        return Err(err(409, "This Teams account runs without a browser: calls and downloads need Always on in Settings"));
    }
    Ok(())
}

pub fn queue(slot: i64, kind: &str, arg1: &str, arg2: &str) -> Res<i64> {
    takes_commands(slot, kind)?;
    with_slot(slot, |r| r.enqueue(kind, arg1, arg2))
}

pub fn queue_once(slot: i64, kind: &str, arg1: &str, arg2: &str) -> Res<i64> {
    takes_commands(slot, kind)?;
    with_slot(slot, |r| r.enqueue_once(kind, arg1, arg2))
}

fn utf16(s: &str) -> usize {
    s.encode_utf16().count()
}

pub fn chat_name(v: Option<&Value>) -> Res<String> {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() && utf16(s) <= 200 => Ok(s.clone()),
        _ => Err(err(400, "Invalid chat name")),
    }
}

fn message_id(v: Option<&Value>) -> Res<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() && utf16(s) <= 100 => Ok(s.clone()),
        _ => Err(err(400, "Invalid message id")),
    }
}

pub fn message_text(v: Option<&Value>) -> Res<String> {
    let Some(Value::String(s)) = v.filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())) else { return Err(err(400, "Empty text")) };
    if utf16(s) > 20000 {
        return Err(err(400, "Text too long"));
    }
    Ok(s.clone())
}

/// People tagged with @ in a message: up to 20 names as the members list shows them.
pub fn mention_names(v: Option<&Value>) -> Res<Vec<String>> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(a))
            if a.len() <= 20 && a.iter().all(|n| n.as_str().is_some_and(|n| !n.trim().is_empty() && utf16(n) <= 100 && !n.contains(['\n', '@']))) =>
        {
            Ok(a.iter().filter_map(|n| n.as_str().map(String::from)).collect())
        }
        _ => Err(err(400, "Invalid mentions")),
    }
}

fn message_args(mid: Option<&Value>) -> Res<String> {
    Ok(crate::js::stringify(&json!({ "mid": message_id(mid)? })))
}

fn text_args(mid: Option<&Value>, text: Option<&Value>) -> Res<String> {
    let mid = message_id(mid)?;
    Ok(crate::js::stringify(&json!({ "mid": mid, "text": message_text(text)? })))
}

fn react_args(mid: Option<&Value>, emoji: Option<&Value>, pill: Option<&Value>) -> Res<String> {
    let id = message_id(mid)?;
    if pill.is_some_and(crate::js::truthy) {
        match pill {
            Some(Value::String(p)) if utf16(p) <= 16 => return Ok(crate::js::stringify(&json!({ "mid": id, "pill": p }))),
            _ => return Err(err(400, "Invalid reaction")),
        }
    }
    match emoji.and_then(Value::as_str) {
        Some(e) if REACTIONS.contains(&e) => Ok(crate::js::stringify(&json!({ "mid": id, "emoji": e }))),
        _ => Err(err(400, "Unsupported reaction")),
    }
}

/// commandOf: the command of a body as the agent reads it (type, arg1, arg2).
pub fn command_of(kind: &str, b: &Map<String, Value>) -> Res<(String, String, String)> {
    let chat = || chat_name(b.get("chat"));
    Ok(match kind {
        "open" => (kind.into(), chat()?, String::new()),
        "send" => (kind.into(), chat()?, message_text(b.get("text"))?),
        "reply" | "edit" => (kind.into(), chat()?, text_args(b.get("mid"), b.get("text"))?),
        "delete" | "undodelete" => (kind.into(), chat()?, message_args(b.get("mid"))?),
        "react" => (kind.into(), chat()?, react_args(b.get("mid"), b.get("emoji"), b.get("pill"))?),
        "resync" | "recheck" => (kind.into(), String::new(), String::new()),
        _ => return Err(err(400, "Unknown command")),
    })
}

/// commandRoute: a command made of what the app sends alone, the chat named `name`.
pub async fn command_route(kind: &'static str, req: crate::web::Req) -> crate::web::Res {
    let (_, slot) = crate::auth::require_slot(&req)?;
    let mut b = req.json()?;
    let name = b.get("name").cloned();
    b.insert("type".into(), json!(kind));
    match name {
        Some(n) => b.insert("chat".into(), n),
        None => b.shift_remove("chat"),
    };
    let (t, a1, a2) = command_of(kind, &b)?;
    crate::web::json(json!({ "ok": true, "id": queue(slot, &t, &a1, &a2)? }))
}

/// mentionParts: the message in the order Teams gets it, text and the people tagged with @.
pub fn mention_parts(text: &str, names: &[String]) -> Vec<Value> {
    let mut unique: Vec<&String> = vec![];
    for n in names.iter().filter(|n| !n.is_empty()) {
        if !unique.contains(&n) {
            unique.push(n);
        }
    }
    // longest first, as the stable sort of the TypeScript keeps equal lengths in order
    unique.sort_by_key(|n| std::cmp::Reverse(n.encode_utf16().count()));
    if unique.is_empty() {
        return vec![json!({ "text": text })];
    }
    // (?<![\p{L}\p{N}@.])@(names)(?![\p{L}\p{N}]), the names tried in that order at each @, as the regex backtracks
    let word = |c: char| c.is_alphanumeric();
    let mut parts = vec![];
    let mut at = 0;
    let mut i = 0;
    while let Some(off) = text[i..].find('@') {
        let start = i + off;
        let before = text[..start].chars().last();
        let mut matched = None;
        if !before.is_some_and(|c| word(c) || c == '@' || c == '.') {
            for n in &unique {
                if text[start + 1..].starts_with(n.as_str()) && !text[start + 1 + n.len()..].chars().next().is_some_and(word) {
                    matched = Some(*n);
                    break;
                }
            }
        }
        match matched {
            Some(n) => {
                if start > at {
                    parts.push(json!({ "text": &text[at..start] }));
                }
                parts.push(json!({ "mention": n }));
                at = start + 1 + n.len();
                i = at;
            }
            None => i = start + 1,
        }
    }
    if at < text.len() {
        parts.push(json!({ "text": &text[at..] }));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions() {
        let names = vec!["Anna".to_string(), "Anna Rossi".to_string()];
        assert_eq!(
            mention_parts("hi @Anna Rossi and @Anna!", &names),
            vec![json!({"text": "hi "}), json!({"mention": "Anna Rossi"}), json!({"text": " and "}), json!({"mention": "Anna"}), json!({"text": "!"})]
        );
        assert_eq!(mention_parts("mail anna@Anna.it", &names), vec![json!({"text": "mail anna@Anna.it"})]);
        assert_eq!(mention_parts("@Annabel", &names), vec![json!({"text": "@Annabel"})]);
        assert_eq!(mention_parts("x", &[]), vec![json!({"text": "x"})]);
    }

    #[test]
    fn inputs() {
        assert!(chat_name(Some(&json!("  "))).is_err());
        assert_eq!(
            command_of("react", json!({"chat": "c", "mid": "1", "emoji": "like"}).as_object().unwrap()).ok().unwrap().2,
            r#"{"mid":"1","emoji":"like"}"#
        );
        assert!(command_of("react", json!({"chat": "c", "mid": "1", "emoji": "x"}).as_object().unwrap()).is_err());
        assert_eq!(command_of("reply", json!({"chat": "c", "mid": "1", "text": "t"}).as_object().unwrap()).ok().unwrap().2, r#"{"mid":"1","text":"t"}"#);
    }
}
