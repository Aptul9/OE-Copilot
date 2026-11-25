//! The commands of the web app run by a light account (src/agent/light/commands.ts): through chatsvc, without a page.
//! Call controls (answer, hangup, mute) have their own lane, so a chat-list read or a message never holds one up.

use super::chatsvc::Part;
use super::media::image_key;
use super::run::{Light, in_app, mark_viewing};
use super::store::{PendingCommand, SavedMessage};
use super::{Fail, R, info, js_trim, now_ms, now_s, text_of, warn};
use crate::state::{cmd_result_key, key, members_key};
use serde_json::{Map, Value, json};
use std::rc::Rc;

/// Seconds a command may wait for the agent: older ones end as failed without touching Teams.
pub const COMMAND_MAX_AGE: i64 = 120;

/// The commands a light account cannot run: they need the Teams page.
const LIGHT_UNSUPPORTED: [&str; 5] = ["download", "call", "answer", "hangup", "mute"];
const CALL_LANE: [&str; 3] = ["answer", "hangup", "mute"];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lane {
    All,
    Calls,
    Other,
}

/// arg2 of a command as JSON: an object, else the empty arguments (parseArgs).
fn args(arg2: &str) -> Map<String, Value> {
    crate::state::object(Some(arg2)).unwrap_or_default()
}

/// A text field of the arguments: of the wrong type it reads as empty.
fn text(a: &Map<String, Value>, k: &str) -> String {
    a.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The parts of a message with people tagged; one part of another shape gives none.
fn parts(a: &Map<String, Value>) -> Vec<Part> {
    let Some(list) = a.get("parts").and_then(Value::as_array) else { return vec![] };
    let mut out = vec![];
    for p in list {
        match (p.get("text").and_then(Value::as_str), p.get("mention").and_then(Value::as_str)) {
            (Some(t), _) => out.push(Part::Text(t.into())),
            (None, Some(m)) => out.push(Part::Mention(m.into())),
            _ => return vec![],
        }
    }
    out
}

/// The messages of a chat through chatsvc, the images saved before kept for the same messages; an image not saved yet
/// is fetched with the chatsvc token.
pub async fn save_chat(l: &Rc<Light>, chat: &str) -> R<bool> {
    let Some(id) = l.list.borrow().ids.get(chat).cloned() else { return Ok(false) };
    let auth = l.auth().await?;
    let read_by = |mids: &[String]| l.store.read_by_cache(mids);
    let Some(messages) = l.chatsvc.fetch_chat_messages(&l.http, &auth, &id, &read_by).await else { return Ok(false) };
    let prev: Vec<Map<String, Value>> = l.store.messages(chat);
    let before_of = |mid: &str| prev.iter().find(|m| m.get("mid").and_then(Value::as_str) == Some(mid));
    let mut kept: Vec<SavedMessage> = vec![];
    for m in messages {
        let Some(extra) = m.extra.clone() else {
            kept.push(m);
            continue;
        };
        let before = before_of(&m.mid);
        let mut extra = extra;
        if let Some(Value::Array(images)) = extra.get("images").cloned()
            && !images.is_empty()
        {
            let mut resolved = vec![];
            for (i, im) in images.iter().enumerate() {
                let saved = before
                    .and_then(|b| b.get("images"))
                    .and_then(|x| x.get(i))
                    .and_then(|x| x.get("f"))
                    .and_then(Value::as_str)
                    .filter(|f| !f.is_empty())
                    .map(String::from);
                let url = im.get("url").and_then(Value::as_str).unwrap_or("");
                let f = match saved {
                    Some(f) => Some(f),
                    None if !url.is_empty() => l.media.image(&image_key(chat, &m.mid, i), url, &auth.token).await,
                    None => None,
                };
                resolved.push(match f {
                    Some(f) => json!({ "f": f, "w": im.get("w").cloned().unwrap_or(json!(0)), "h": im.get("h").cloned().unwrap_or(json!(0)) }),
                    None => im.clone(),
                });
            }
            extra.insert("images".into(), Value::Array(resolved));
        }
        if let Some(av) = before.and_then(|b| b.get("av")).filter(|v| crate::js::truthy(v)) {
            extra.insert("av".into(), av.clone());
        }
        let seen = before.and_then(|b| b.get("status")).and_then(Value::as_str).filter(|s| s.to_lowercase().contains("seen"));
        match seen {
            Some(s) => {
                extra.insert("status".into(), json!(s));
            }
            None => {
                if let Some(s) = m.extra.as_ref().and_then(|e| e.get("status")) {
                    extra.insert("status".into(), s.clone());
                }
            }
        }
        kept.push(SavedMessage { extra: Some(extra), ..m });
    }
    l.store.save_chat_messages(chat, &kept).map_err(|e| Fail::Other(e.to_string()))?;
    Ok(true)
}

/// The chat read: in the app at once, on Teams at its last message, with the items of the Activity feed it has.
pub async fn mark_read(l: &Rc<Light>, chat: &str) {
    let id = l.list.borrow().ids.get(chat).cloned();
    l.store.mark_chat_read(chat);
    let Some(id) = id else {
        warn("light", "mark read: no conversation id", &[("chat", chat.into())]);
        return;
    };
    let items = l.unread_feed.borrow_mut().remove(&id).unwrap_or_default();
    for (activity, _) in &items {
        l.store.mark_activity_read(activity);
    }
    match l.auth().await {
        Ok(auth) => {
            let last = l.store.messages(chat).last().and_then(|m| m.get("mid").and_then(Value::as_str).map(String::from));
            l.chatsvc.mark_read(&l.http, &auth, &id, last.as_deref()).await;
            for (_, mid) in &items {
                l.chatsvc.mark_message_read(&l.http, &auth, "48:notifications", mid).await;
            }
        }
        Err(e) => warn("light", &format!("mark read: {}", text_of(&e)), &[("chat", chat.into())]),
    }
}

/// After an action on a message: the chat stays the one in use, its messages saved again, the list read soon.
async fn after(l: &Rc<Light>, chat: &str, ok: bool, since: i64) -> &'static str {
    mark_viewing(&l.store, chat, since);
    l.store.set_state(key::ACTIVE_CHAT, chat);
    let _ = save_chat(l, chat).await;
    l.due_list.set(l.due_list.get().min(now_ms() + 1500));
    if ok { "done" } else { "failed" }
}

fn open_failed(l: &Light, id: i64, reason: &str) -> &'static str {
    l.store.set_state(&cmd_result_key(id), &crate::js::stringify(&json!({ "reason": reason })));
    "failed"
}

/// The conversation of the chat and the token for chatsvc, or None.
async fn target(l: &Rc<Light>, chat: &str) -> R<Option<(String, super::api::TeamsAuth)>> {
    let Some(id) = l.list.borrow().ids.get(chat).cloned() else { return Ok(None) };
    Ok(Some((id, l.auth().await?)))
}

/// The other people of the chat, named: name and mri.
async fn people_of(l: &Rc<Light>, chat: &str) -> R<Option<Vec<(String, String)>>> {
    let Some(id) = l.list.borrow().ids.get(chat).cloned() else { return Ok(None) };
    let mris = l.list.borrow().members.get(&id).cloned().unwrap_or_default();
    let missing: Vec<String> = mris.iter().filter(|m| !l.people.borrow().contains_key(*m)).cloned().collect();
    if !missing.is_empty() {
        let found = l.api.people(&missing).await?;
        l.people.borrow_mut().extend(found);
    }
    let people = l.people.borrow();
    Ok(Some(mris.iter().filter_map(|m| people.get(m).map(|p| (p.display_name.clone(), m.clone()))).collect()))
}

/// What the loop does, asked now: read the chat list, read the feed.
pub struct Reads {
    pub list: Box<dyn Fn() -> futures_util::future::LocalBoxFuture<'static, bool>>,
    pub activity: Box<dyn Fn() -> futures_util::future::LocalBoxFuture<'static, bool>>,
}

/// One command; None for a type the light agent has no handler for.
async fn handle(l: &Rc<Light>, reads: &Reads, cmd: &PendingCommand) -> R<Option<&'static str>> {
    let (chat, ts) = (cmd.arg1.as_str(), cmd.ts);
    let done = |ok: bool| if ok { "done" } else { "failed" };
    let calls = l.calls.borrow().clone();
    let a = args(&cmd.arg2);
    Ok(Some(match cmd.kind.as_str() {
        "call" => {
            let thread = l.list.borrow().ids.get(chat).cloned();
            let mine = format!("8:orgid:{}", l.me.borrow().0);
            let peers: Option<Vec<String>> =
                thread.as_ref().and_then(|t| l.list.borrow().members.get(t).map(|m| m.iter().filter(|x| **x != mine).cloned().collect()));
            let peers = if thread.is_some() { peers } else { Some(vec![]) };
            match (&calls, &thread, peers.as_ref().filter(|p| p.len() == 1)) {
                (Some(c), Some(t), Some(p)) => {
                    let mut args = Map::new();
                    args.insert("thread".into(), json!(t));
                    args.insert("peer".into(), json!(p[0]));
                    args.insert("name".into(), json!(chat));
                    done(c.command("call", args).await)
                }
                _ => {
                    let n = peers.map_or(0, |p| p.len());
                    warn(
                        "calling",
                        "outgoing peer unavailable",
                        &[("worker", calls.is_some().into()), ("thread", thread.is_some().into()), ("peers", n.into())],
                    );
                    "failed"
                }
            }
        }
        "answer" | "mute" => match calls {
            Some(c) => {
                let parsed: Value = serde_json::from_str(if cmd.arg2.is_empty() { "{}" } else { &cmd.arg2 })
                    .map_err(|e| Fail::Other(format!("Unexpected token in JSON: {e}")))?;
                done(c.command(&cmd.kind, crate::js::obj(&parsed)).await)
            }
            None => "failed",
        },
        "hangup" => match calls {
            Some(c) => done(c.command("hangup", Map::new()).await),
            None => "failed",
        },
        "open" => {
            mark_viewing(&l.store, chat, ts);
            if l.signed_out.get() {
                return Ok(Some(open_failed(l, cmd.id, "signed-out")));
            }
            if !l.list.borrow().ids.contains_key(chat) {
                return Ok(Some(open_failed(l, cmd.id, "not-listed")));
            }
            if !save_chat(l, chat).await? {
                return Ok(Some(open_failed(l, cmd.id, "unreadable")));
            }
            l.store.set_state(key::ACTIVE_CHAT, chat);
            let (l2, c2) = (l.clone(), chat.to_string());
            tokio::task::spawn_local(crate::push::notify::keep_slot(async move { mark_read(&l2, &c2).await }));
            "done"
        }
        "send" => {
            let t = target(l, chat).await?;
            let body = js_trim(&cmd.arg2).to_string();
            let ok = match (&t, body.is_empty()) {
                (Some((id, auth)), false) => l.chatsvc.send_text(&l.http, auth, id, &body).await?.is_some(),
                _ => false,
            };
            after(l, chat, ok, ts).await
        }
        "reply" | "edit" => {
            let (mid, body) = (text(&a, "mid"), js_trim(&text(&a, "text")).to_string());
            let t = target(l, chat).await?;
            let ok = match (&t, mid.is_empty() || body.is_empty()) {
                (Some((id, auth)), false) if cmd.kind == "reply" => l.chatsvc.reply_text(&l.http, auth, id, &mid, &body).await?.is_some(),
                (Some((id, auth)), false) => l.chatsvc.edit_text(&l.http, auth, id, &mid, &body).await?,
                _ => false,
            };
            after(l, chat, ok, ts).await
        }
        "delete" | "undodelete" => {
            let mid = text(&a, "mid");
            let t = target(l, chat).await?;
            let ok = match (&t, mid.is_empty()) {
                (Some((id, auth)), false) if cmd.kind == "delete" => l.chatsvc.delete_message(&l.http, auth, id, &mid).await?,
                (Some((id, auth)), false) => l.chatsvc.undelete_message(&l.http, auth, id, &mid).await?,
                _ => false,
            };
            after(l, chat, ok, ts).await
        }
        "react" => {
            let mid = text(&a, "mid");
            let pick = [text(&a, "pill"), text(&a, "emoji")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
            let t = target(l, chat).await?;
            let ok = match (&t, mid.is_empty()) {
                (Some((id, auth)), false) => l.chatsvc.toggle_reaction(&l.http, auth, id, &mid, &pick).await,
                _ => false,
            };
            after(l, chat, ok, ts).await
        }
        // the upload is deleted whatever happened: the app sends it again on a retry
        "sendimage" => {
            let (file, caption) = (text(&a, "file"), text(&a, "text"));
            if !regex::Regex::new(r"^[0-9a-f]{16}\.(png|jpg|gif|webp)$").expect("fixed").is_match(&file) {
                return Ok(Some("failed"));
            }
            let upload = l.config.uploads_dir.join(&file);
            let run = async {
                let data = std::fs::read(&upload).map_err(|e| Fail::Other(e.to_string()))?;
                let mime = match file.rsplit('.').next().unwrap_or("") {
                    "png" => "image/png",
                    "jpg" => "image/jpeg",
                    "gif" => "image/gif",
                    _ => "image/webp",
                };
                let t = target(l, chat).await?;
                let ok = match &t {
                    Some((id, auth)) => l.chatsvc.send_image(&l.http, auth, id, &data, mime, &caption).await?,
                    None => false,
                };
                Ok::<&str, Fail>(after(l, chat, ok, ts).await)
            };
            let outcome = run.await;
            let _ = std::fs::remove_file(&upload);
            match outcome {
                Ok(o) => o,
                Err(e) => {
                    warn("image", &text_of(&e), &[("file", file.as_str().into())]);
                    "failed"
                }
            }
        }
        "members" => {
            mark_viewing(&l.store, chat, ts);
            l.store.set_state(key::ACTIVE_CHAT, chat);
            match people_of(l, chat).await? {
                None => "failed",
                Some(people) => {
                    let names: Vec<String> = people.into_iter().map(|p| p.0).collect();
                    l.store.set_state(&members_key(chat), &crate::js::stringify(&json!({ "ts": now_s(), "names": names })));
                    "done"
                }
            }
        }
        "sendmentions" => {
            let parts = parts(&a);
            let t = target(l, chat).await?;
            let people = people_of(l, chat).await?;
            let ok = match (&t, &people) {
                (Some((id, auth)), Some(people)) => {
                    let mri_of = |name: &str| people.iter().find(|p| p.0 == name).map(|p| p.1.clone());
                    l.chatsvc.send_mentions(&l.http, auth, id, &parts, &mri_of).await?.is_some()
                }
                _ => false,
            };
            after(l, chat, ok, ts).await
        }
        "activity" => done((reads.activity)().await),
        "resync" => {
            (reads.list)().await;
            "done"
        }
        // the outcome goes as a push: it also shows that notifications reach the phone
        "recheck" => {
            let ok = (reads.list)().await && (reads.activity)().await;
            let why = if l.signed_out.get() {
                "signed out, the browser signs in again soon".to_string()
            } else if l.error.borrow().is_empty() {
                "Teams not reachable".to_string()
            } else {
                l.error.borrow().clone()
            };
            l.notifier.alert("Teams", &if ok { "Everything works".to_string() } else { format!("Problem: {why}") }).await;
            "done"
        }
        "check" => "done",
        _ => return Ok(None),
    }))
}

/// Every pending command of the lane, oldest first, as the browser agent runs them.
pub async fn run_light_commands(l: &Rc<Light>, reads: &Reads, lane: Lane) {
    let interrupted = if lane == Lane::All { l.store.interrupted_commands() } else { 0 };
    if interrupted > 0 {
        warn("cmd", "stopped while running, not run again: unconfirmed", &[("commands", interrupted.into())]);
    }
    for cmd in l.store.pending_commands() {
        let call = CALL_LANE.contains(&cmd.kind.as_str());
        if (lane == Lane::Calls && !call) || (lane == Lane::Other && call) {
            continue;
        }
        let expired = l.store.expire_pending_commands(COMMAND_MAX_AGE);
        if expired > 0 {
            warn("cmd", "waited too long, not run", &[("commands", expired.into())]);
        }
        if l.store.command_status(cmd.id).as_deref() != Some("pending") {
            continue;
        }
        let mut fields = vec![("id", cmd.id.into())];
        if cmd.kind != "download" {
            fields.push(("arg", cmd.arg1.as_str().into()));
        }
        info("CMD", &cmd.kind, &fields);
        l.store.start_command(cmd.id);
        let outcome = match handle(l, reads, &cmd).await {
            Ok(Some(o)) => o,
            Ok(None) => {
                if LIGHT_UNSUPPORTED.contains(&cmd.kind.as_str()) {
                    "failed"
                } else {
                    "done"
                }
            }
            Err(e) => {
                if matches!(e, Fail::SignedOut(_)) {
                    l.signed_out.set(true);
                }
                warn("cmd", &text_of(&e), &[("id", cmd.id.into()), ("type", cmd.kind.as_str().into())]);
                "failed"
            }
        };
        l.store.finish_command(cmd.id, outcome);
    }
}

/// Whether the app shows this chat now (inApp), for the commands of other modules.
pub fn shown_in_app(l: &Light, chat: &str) -> bool {
    in_app(&l.store, chat)
}
