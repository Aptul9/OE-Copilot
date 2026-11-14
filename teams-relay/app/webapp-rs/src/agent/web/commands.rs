//! The commands of the web app run by the browser agent (src/agent/commands): one handler per type, the outcome written
//! after the new state is saved; answer, hangup and mute are the call watch's.

use super::Agent;
use super::actions::{self, SendResult};
use super::jobs;
use crate::agent::chatsvc::Part;
use crate::agent::run::union;
use crate::agent::store::PendingCommand;
use crate::agent::{info, now_ms, now_s, warn};
use crate::state::{cmd_result_key, key, members_key};
use serde_json::{Map, Value, json};
use std::rc::Rc;

/// Seconds a command may wait for the agent: older ones end as failed without touching Teams.
pub const COMMAND_MAX_AGE: i64 = 120;
/// Milliseconds a refresh of the feed waits for the side bar
pub const RAIL_WAIT: u64 = 10_000;
/// Seconds after its tap a call is no longer placed, milliseconds Teams has to start recording once the keys went
pub const CALL_MAX_AGE: i64 = 25;
pub const CALL_START_WAIT: i64 = 10_000;
const CALL_COMMANDS: [&str; 3] = ["answer", "hangup", "mute"];

fn args(arg2: &str) -> Map<String, Value> {
    crate::state::object(Some(arg2)).unwrap_or_default()
}

fn text(a: &Map<String, Value>, k: &str) -> String {
    a.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn parts(a: &Map<String, Value>) -> Vec<Part> {
    let mut out = vec![];
    for p in a.get("parts").and_then(Value::as_array).into_iter().flatten() {
        match (p.get("text").and_then(Value::as_str), p.get("mention").and_then(Value::as_str)) {
            (Some(t), _) => out.push(Part::Text(t.into())),
            (None, Some(m)) => out.push(Part::Mention(m.into())),
            _ => return vec![],
        }
    }
    out
}

/// The chat saved as soon as the message went: the web app waits for it in the saved chat.
fn save_after<'a>(a: &'a Agent, chat: &'a str) -> actions::AfterPress<'a> {
    Some(Box::new(move || {
        Box::pin(async move {
            jobs::save_open_chat(a, chat).await;
        })
    }))
}

/// After an action on a message: the chat stays the one in use, its new state is saved, then the outcome; a message that
/// went out without Teams showing it sent is unconfirmed.
async fn after_message_action(a: &Agent, chat: &str, result: SendResult, since: i64) -> &'static str {
    jobs::mark_viewing_of(a, chat, since);
    a.store.set_state(key::ACTIVE_CHAT, chat);
    jobs::save_open_chat(a, chat).await;
    match result {
        SendResult::Sent => "done",
        SendResult::Unconfirmed => "unconfirmed",
        SendResult::Failed => "failed",
    }
}

fn ok(b: bool) -> SendResult {
    if b { SendResult::Sent } else { SendResult::Failed }
}

fn failed_with(a: &Agent, id: i64, reason: &str) -> &'static str {
    a.store.set_state(&cmd_result_key(id), &crate::js::stringify(&json!({ "reason": reason })));
    "failed"
}

/// arg1: chat. The app shows the chat: through chatsvc at once when it can, else Teams opens it and its messages are
/// saved; the reason of a failure in cmd_result:<id>.
async fn open(a: &Rc<Agent>, c: &PendingCommand) -> &'static str {
    let chat = c.arg1.as_str();
    jobs::mark_viewing_of(a, chat, c.ts);
    if a.teams() == "login" {
        return failed_with(a, c.id, "signed-out");
    }
    if a.saved_by_api.borrow().contains(chat) && !a.store.messages(chat).is_empty() {
        a.store.set_state(key::ACTIVE_CHAT, chat);
        let (a1, c1) = (a.clone(), chat.to_string());
        tokio::task::spawn_local(async move { jobs::mark_read_via_api(&a1, &c1).await });
        let (a2, c2) = (a.clone(), chat.to_string());
        tokio::task::spawn_local(async move {
            jobs::save_chat_via_api(&a2, &c2).await;
        });
        return "done";
    }
    if jobs::save_chat_via_api(a, chat).await {
        a.store.set_state(key::ACTIVE_CHAT, chat);
        let (a1, c1) = (a.clone(), chat.to_string());
        tokio::task::spawn_local(async move { jobs::mark_read_via_api(&a1, &c1).await });
        return "done";
    }
    let Some(tp) = a.tp() else { return "failed" };
    if let Some(problem) = tp.show_chat(chat).await {
        return failed_with(a, c.id, problem);
    }
    a.store.set_state(key::ACTIVE_CHAT, chat);
    if !jobs::save_open_chat(a, chat).await {
        return failed_with(a, c.id, "unreadable");
    }
    "done"
}

/// arg1: chat. A Teams audio call to the person of that 1:1 chat: Teams opens the chat and the agent presses Teams' own
/// shortcut; done once a Teams page records from the microphone.
async fn call(a: &Rc<Agent>, c: &PendingCommand) -> &'static str {
    let chat = c.arg1.as_str();
    let fail = |reason: &str| {
        info("call", "not placed", &[("callee", chat.into()), ("reason", reason.into())]);
        failed_with(a, c.id, reason)
    };
    if c.ts != 0 && now_s() - c.ts >= CALL_MAX_AGE {
        return fail("late");
    }
    if a.ringing.get() || a.in_call.get() {
        return fail("busy");
    }
    if a.teams() == "login" {
        return fail("signed-out");
    }
    if crate::agent::detector::is_self_chat(chat) || a.store.chats().into_iter().find(|x| x.name == chat).map(|x| x.kind).as_deref() != Some("one") {
        return fail("not-one");
    }
    jobs::mark_viewing_of(a, chat, c.ts);
    let Some(tp) = a.tp() else { return "failed" };
    if let Some(problem) = tp.show_chat(chat).await {
        return fail(problem);
    }
    a.store.set_state(key::ACTIVE_CHAT, chat);
    let page = tp.page.clone();
    let not_one = page.evaluate("calls.notOneOnOne", Some(&crate::agent::browser::scripts::st(json!({})))).await.unwrap_or(Value::Null);
    if crate::js::truthy(&not_one) {
        info("call", "header is not a 1:1 chat", &[("callee", chat.into()), ("why", crate::js::string(Some(&not_one)).into())]);
        return fail("not-one");
    }
    *a.outgoing.borrow_mut() = Some((chat.to_string(), now_ms()));
    let pressed = super::calls::start_audio_call(a, &page).await;
    if !pressed {
        *a.outgoing.borrow_mut() = None;
        return fail("busy");
    }
    info("call", "calling", &[("callee", chat.into())]);
    let until = now_ms() + CALL_START_WAIT;
    while now_ms() < until {
        if super::calls::recording_page(&page).await.is_some() {
            info("call", "placed", &[("callee", chat.into())]);
            return "done";
        }
        crate::agent::browser::sleep(250).await;
    }
    *a.outgoing.borrow_mut() = None;
    let overlays = page.evaluate("message-actions.openOverlayNames", Some(&crate::agent::browser::scripts::sel())).await.unwrap_or(json!([]));
    let names: Vec<String> = overlays.as_array().into_iter().flatten().map(|v| crate::js::string(Some(v))).collect();
    let mut fields = vec![("callee", chat.into())];
    if !names.is_empty() {
        fields.push(("overlays", names.join(", ").into()));
    }
    warn("call", "Teams did not start the call", &fields);
    fail("no-call")
}

/// The check of an account the web app starts only to check it: the whole list and the feed, then a push for each new
/// missed call and one more when a chat or another notification is unread that was not.
async fn check(a: &Rc<Agent>) -> &'static str {
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let chats = jobs::scan_chats_full(a).await;
    let feed = jobs::read_activity(a, RAIL_WAIT).await;
    let unread: Vec<_> = a.store.chats().into_iter().filter(|c| c.unread && !c.muted && !crate::agent::detector::is_self_chat(&c.name)).collect();
    let before = a.store.get_state(key::CHECK_SEEN);
    let was = (!before.is_empty()).then(|| crate::state::object(Some(&before)).unwrap_or_default());
    let ids = |o: Option<&Value>| -> Vec<String> {
        o.and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
    };
    let read = feed.is_some();
    let calls = if read { a.store.missed_calls() } else { vec![] };
    let unread_now = if read { a.store.unread_activity() } else { vec![] };
    let in_read: std::collections::HashSet<String> = if read { a.store.activity_ids().into_iter().collect() } else { Default::default() };
    let was_activity = ids(was.as_ref().and_then(|w| w.get("activity")));
    let was_calls = was.as_ref().and_then(|w| w.get("calls")).map(|c| ids(Some(c)));
    let was_read = was.as_ref().is_some_and(|w| w.get("read") == Some(&Value::Bool(true)));
    let now_chats: Vec<String> = unread.iter().map(|c| format!("{}\n{}", c.name, c.preview)).collect();
    let activity =
        if read { union(unread_now.clone(), was_activity.iter().filter(|id| !in_read.contains(*id)).cloned().collect()) } else { was_activity.clone() };
    let now_calls = if read { Some(union(calls.iter().map(|c| c.0.clone()).collect(), was_calls.clone().unwrap_or_default())) } else { was_calls.clone() };
    let mut now = Map::new();
    now.insert("chats".into(), json!(now_chats));
    now.insert("activity".into(), json!(activity));
    if let Some(c) = &now_calls {
        now.insert("calls".into(), json!(c));
    }
    now.insert("read".into(), json!(read || was_read));
    // recorded before the pushes: a check cut short pushes none of it again
    a.store.set_state(key::CHECK_SEEN, &crate::js::stringify(&Value::Object(now)));
    if let Some(w) = &was {
        let compare = read && was_read;
        let had: std::collections::HashSet<String> = was_calls.clone().unwrap_or_default().into_iter().collect();
        let missed: Vec<_> = if compare && was_calls.is_some() { calls.iter().filter(|c| !had.contains(&c.0)).collect() } else { vec![] };
        let call_ids: std::collections::HashSet<&String> = calls.iter().map(|c| &c.0).collect();
        let was_chats = ids(w.get("chats"));
        let new_chats = now_chats.iter().filter(|c| !was_chats.contains(c)).count();
        let new_items = if compare { unread_now.iter().filter(|id| !was_activity.contains(id) && !call_ids.contains(id)).count() } else { 0 };
        let found: Vec<String> = [
            (!unread.is_empty()).then(|| plural(unread.len(), "unread chat", "unread chats")),
            (new_items > 0).then(|| plural(new_items, "new notification", "new notifications")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let title = found.join(", ");
        let mut pushes: Vec<futures_util::future::LocalBoxFuture<'_, usize>> = missed.iter().map(|c| a.notifier.missed_call(&c.1, &c.2)).collect();
        if new_chats > 0 || new_items > 0 {
            pushes.push(a.notifier.notice(&title, "Found by the check: open TeamsRelay to read them."));
        }
        futures_util::future::join_all(pushes).await;
    }
    if !read {
        warn("check", "Activity feed not read: notifications as last read", &[]);
    }
    if chats.is_some() { "done" } else { "failed" }
}

/// One command; an unknown type ends as done, a handler that fails as failed.
async fn run_command(a: &Rc<Agent>, c: &PendingCommand) -> &'static str {
    let chat = c.arg1.as_str();
    let ar = args(&c.arg2);
    match c.kind.as_str() {
        "open" => open(a, c).await,
        "send" => {
            let r = actions::send_text(a, chat, &c.arg2, save_after(a, chat)).await;
            after_message_action(a, chat, r, c.ts).await
        }
        "reply" => {
            let r = actions::reply_with_quote(a, chat, &text(&ar, "mid"), &text(&ar, "text"), save_after(a, chat)).await;
            after_message_action(a, chat, r, c.ts).await
        }
        "react" => {
            let (mid, pill) = (text(&ar, "mid"), text(&ar, "pill"));
            let done =
                if !pill.is_empty() { actions::toggle_pill(a, chat, &mid, &pill).await } else { actions::react(a, chat, &mid, &text(&ar, "emoji")).await };
            after_message_action(a, chat, ok(done), c.ts).await
        }
        "edit" => {
            let done = actions::edit_message(a, chat, &text(&ar, "mid"), &text(&ar, "text")).await;
            after_message_action(a, chat, ok(done), c.ts).await
        }
        "delete" => {
            let done = actions::delete_message(a, chat, &text(&ar, "mid")).await;
            after_message_action(a, chat, ok(done), c.ts).await
        }
        "undodelete" => {
            let done = actions::undo_delete(a, chat, &text(&ar, "mid")).await;
            after_message_action(a, chat, ok(done), c.ts).await
        }
        "download" => {
            let Some(page) = a.page() else { return "failed" };
            match a.media.download(&page, chat, &text(&ar, "name")).await {
                Some(f) => {
                    a.store.set_state(&cmd_result_key(c.id), &crate::js::stringify(&json!({ "f": f })));
                    "done"
                }
                None => "failed",
            }
        }
        "activity" => {
            if jobs::read_activity(a, RAIL_WAIT).await.is_none() {
                return "failed";
            }
            if !(a.checked_only)() {
                crate::agent::run::push_missed_calls(&a.store, &a.notifier).await;
            }
            "done"
        }
        "resync" => {
            if let Ok(rows) = jobs::read_list(a).await {
                let _ = a.store.save_chats(&rows.into_iter().map(|r| r.entry).collect::<Vec<_>>(), false);
            }
            let active = a.store.get_state(key::ACTIVE_CHAT);
            if !active.is_empty() {
                jobs::save_open_chat(a, &active).await;
            }
            "done"
        }
        "recheck" => {
            let (good, why) = jobs::self_check(a).await;
            a.notifier.alert("Teams", &if good { "Everything works".to_string() } else { format!("Problem: {why}") }).await;
            "done"
        }
        "sendimage" => {
            let (file, caption) = (text(&ar, "file"), text(&ar, "text"));
            if !regex::Regex::new(r"^[0-9a-f]{16}\.(png|jpg|gif|webp)$").expect("fixed").is_match(&file) {
                warn("image", "not an upload", &[("file", crate::agent::cut(&file, 60).into())]);
                return "failed";
            }
            let upload = a.config.uploads_dir.join(&file);
            let Ok(data) = std::fs::read(&upload) else {
                warn("image", "upload missing", &[("file", file.as_str().into())]);
                return "failed";
            };
            let ext = file.rsplit('.').next().unwrap_or("png");
            let mime = match ext {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                "gif" => "image/gif",
                _ => "image/webp",
            };
            let r = actions::send_image(a, chat, &format!("image.{ext}"), mime, &data, &caption, save_after(a, chat)).await;
            let out = after_message_action(a, chat, r, c.ts).await;
            // the upload goes whatever happened: the app sends it again on a retry
            let _ = std::fs::remove_file(&upload);
            out
        }
        "members" => {
            jobs::mark_viewing_of(a, chat, c.ts);
            let names = actions::read_members(a, chat).await;
            a.store.set_state(key::ACTIVE_CHAT, chat);
            match names {
                Some(n) => {
                    a.store.set_state(&members_key(chat), &crate::js::stringify(&json!({ "ts": now_s(), "names": n })));
                    "done"
                }
                None => "failed",
            }
        }
        "sendmentions" => {
            let p = parts(&ar);
            if p.is_empty() {
                return "failed";
            }
            let r = actions::send_with_mentions(a, chat, &p, save_after(a, chat)).await;
            after_message_action(a, chat, r, c.ts).await
        }
        "check" => check(a).await,
        "call" => call(a, c).await,
        k if CALL_COMMANDS.contains(&k) => "failed",
        _ => "done",
    }
}

/// Every pending command, oldest first, marked running while it runs; the chat list read after each one.
pub async fn run_pending_commands(a: &Rc<Agent>) {
    let interrupted = a.store.interrupted_commands();
    if interrupted > 0 {
        warn("cmd", "stopped while running, not run again: unconfirmed", &[("commands", interrupted.into())]);
    }
    for c in a.store.pending_commands() {
        let expired = a.store.expire_pending_commands(COMMAND_MAX_AGE);
        if expired > 0 {
            warn("cmd", "waited too long, not run", &[("commands", expired.into())]);
        }
        if a.store.command_status(c.id).as_deref() != Some("pending") {
            continue;
        }
        // the call watch takes them at its next look
        if CALL_COMMANDS.contains(&c.kind.as_str()) {
            continue;
        }
        // a refresh of the feed, or a check, waits until the side bar is clickable
        if (c.kind == "activity" || c.kind == "check") && !a.rail_ready.get() {
            continue;
        }
        // an open or a call waits while Teams shows no chat list yet; signed out it fails at once with its reason
        let teams = a.teams();
        if (c.kind == "open" || c.kind == "call") && teams != "ok" && teams != "login" {
            continue;
        }
        let mut fields = vec![("id", c.id.into())];
        if c.kind != "download" {
            fields.push(("arg", c.arg1.as_str().into()));
        }
        info("CMD", &c.kind, &fields);
        a.store.start_command(c.id);
        let outcome = match a.page() {
            Some(page) => page.as_agent(run_command(a, &c)).await,
            None => "failed",
        };
        a.store.finish_command(c.id, outcome);
        jobs::scan_chats(a).await;
    }
}
