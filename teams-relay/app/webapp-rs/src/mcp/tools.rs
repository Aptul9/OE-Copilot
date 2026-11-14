//! Tools of /mcp, as functions of the user the token acts as (src/lib/mcp/tools.ts): reads of what the agent saved, and
//! commands the agent runs on Teams (the same ones the app queues). A refusal is an error whose text the client sees.

use crate::accounts::{account_summary, up_since};
use crate::appdb::{self, Slot};
use crate::commands::{self, command_of};
use crate::js::{self, set, truthy_opt};
use crate::slotdb::{SlotReader, slot_dir, with_slot};
use crate::state::{Viewing, key};
use crate::web::{Error, Res, now_ms};
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const NO_CHAT: &str = "No chat with this name: use a name as list_chats gives it";

/// ToolError: a refusal the client reads as the text of the result.
pub fn refuse<T>(text: impl Into<String>) -> Res<T> {
    Err(Error::Http(400, text.into(), vec![]))
}

/// What the client reads of an error: the text of a refusal, HttpError or SlotNotReady; anything else is logged.
pub fn error_text(e: Error) -> String {
    match e {
        Error::Http(_, d, _) => d,
        Error::NotReady => "Account not ready yet".into(),
        Error::Internal(m) => {
            eprintln!("mcp: {m}");
            "Internal error".into()
        }
    }
}

/// toolError of src/shared/relay-sync.ts.
pub fn tool_error(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

/// answer() of src/lib/mcp/server.ts: the data as JSON text and as structured content.
fn answer(r: Res<Value>) -> Value {
    match r {
        Ok(data) => json!({"content": [{"type": "text", "text": js::stringify(&data)}], "structuredContent": data}),
        Err(e) => tool_error(&error_text(e)),
    }
}

/// blocks(): an answer made of content blocks (images, files).
fn blocks(r: Res<Vec<Value>>) -> Value {
    match r {
        Ok(content) => json!({"content": content}),
        Err(e) => tool_error(&error_text(e)),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Wait {
    pub timeout_ms: u64,
    pub poll_ms: u64,
}

/// The agent drops a command older than 2 minutes (COMMAND_MAX_AGE): a tool waits less, then says it is still queued.
pub const COMMAND_WAIT: Wait = Wait { timeout_ms: 45_000, poll_ms: 300 };
const OPEN_WAIT: Wait = Wait { timeout_ms: 30_000, poll_ms: 500 };

/// The tool `name` with arguments already checked against its schema.
pub async fn call(user_id: &str, name: &str, a: &Map<String, Value>) -> Value {
    match name {
        "list_accounts" => answer(list_accounts(user_id)),
        "list_chats" => answer(list_chats(user_id, a)),
        "read_chat" => answer(read_chat(user_id, a)),
        "refresh_chat" => answer(refresh_chat(user_id, a, OPEN_WAIT).await),
        "list_activity" => answer(list_activity(user_id, a)),
        "send_message" => answer(send_message(user_id, a, COMMAND_WAIT).await),
        "react_message" => answer(react_message(user_id, a, COMMAND_WAIT).await),
        "edit_message" => answer(edit_message(user_id, a, COMMAND_WAIT).await),
        "delete_message" => answer(message_command(user_id, a, "delete", "the delete", COMMAND_WAIT).await),
        "undo_delete" => answer(message_command(user_id, a, "undodelete", "the message was brought back", COMMAND_WAIT).await),
        "get_attachment" => blocks(get_attachment(user_id, a, COMMAND_WAIT).await),
        _ => tool_error("Internal error"),
    }
}

fn arg_str<'a>(a: &'a Map<String, Value>, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}

fn arg_int(a: &Map<String, Value>, k: &str) -> Option<i64> {
    a.get(k).and_then(Value::as_f64).map(|f| f as i64)
}

/// String.prototype.trim: Unicode white space and the BOM.
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// An account of the user, ?a=N of the HTTP API: the given slot, or the first one.
fn account_of(user_id: &str, account: Option<i64>) -> Res<Slot> {
    let owned = appdb::slots_of(user_id)?;
    let ids: Vec<i64> = owned.iter().map(|s| s.slot).collect();
    let slot = crate::auth::pick_slot(&ids, account.map(|n| n.to_string()).as_deref())?;
    Ok(owned.into_iter().find(|s| s.slot == slot).expect("picked among the owned slots"))
}

/// PARK_AFTER of src/agent/logic/parking.ts.
const PARK_AFTER: f64 = 30.0;

/// The chat Teams shows, saved by the agent every second: the one last opened, while the app shows a chat (marked
/// within PARK_AFTER seconds, not left since; otherwise Teams parks on the self chat). Nothing while the agent or Teams
/// is not working.
fn live_chat(r: &SlotReader, s: &Slot) -> String {
    if s.stopped != 0 {
        return String::new();
    }
    let h = r.health(up_since(s));
    if h.get("agent") != Some(&json!("ok")) || h.get("teams") != Some(&json!("ok")) {
        return String::new();
    }
    let v = Viewing::parse(r.raw(key::VIEWING).as_deref());
    let shown = !v.chat.is_empty() && now_ms() as f64 / 1000.0 - v.ts < PARK_AFTER;
    if shown { r.active_chat() } else { String::new() }
}

/// Teams message ids are the milliseconds of the message.
pub fn message_time(mid: Option<&Value>) -> Option<String> {
    let s = match mid {
        None | Some(Value::Null) => String::new(),
        v => js::string(v),
    };
    let ok = s.len() == 13 && s.starts_with('1') && s.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| crate::auth::rows::iso_ms(s.parse().unwrap_or(0)))
}

fn non_empty_array(v: Option<&Value>) -> Option<&Vec<Value>> {
    v.and_then(Value::as_array).filter(|a| !a.is_empty())
}

/// A saved message as the tools give it: flags only when set.
pub fn tool_message(m: &Value) -> Value {
    let mut out = Map::new();
    set(&mut out, "id", m.get("mid").cloned());
    set(&mut out, "time", message_time(m.get("mid")).map(Value::String));
    set(&mut out, "author", m.get("author").cloned());
    out.insert("mine".into(), json!(truthy_opt(m.get("mine"))));
    set(&mut out, "text", m.get("text").cloned());
    if truthy_opt(m.get("quote")) {
        out.insert("quote".into(), m["quote"].clone());
    }
    if let Some(rs) = non_empty_array(m.get("reactions")) {
        let list: Vec<Value> = rs
            .iter()
            .map(|x| {
                let mut r = Map::new();
                set(&mut r, "emoji", x.get("e").cloned());
                set(&mut r, "count", x.get("n").cloned());
                set(&mut r, "mine", x.get("mine").cloned());
                Value::Object(r)
            })
            .collect();
        out.insert("reactions".into(), Value::Array(list));
    }
    if let Some(images) = non_empty_array(m.get("images")) {
        out.insert("images".into(), json!(images.len()));
    }
    if let Some(files) = non_empty_array(m.get("files")) {
        out.insert("files".into(), Value::Array(files.iter().map(|f| f.get("name").cloned().unwrap_or(Value::Null)).collect()));
    }
    for (from, to) in [("edited", "edited"), ("deleted", "deleted"), ("mentionsMe", "mentions_me")] {
        if truthy_opt(m.get(from)) {
            out.insert(to.into(), json!(true));
        }
    }
    Value::Object(out)
}

fn has_chat(r: &SlotReader, chat: &str) -> bool {
    r.chats().iter().any(|c| c.get("name").and_then(Value::as_str) == Some(chat))
}

fn chat_messages(r: &SlotReader, s: &Slot, chat: &str) -> Res<Value> {
    let messages: Vec<Value> = r.messages(chat).iter().map(tool_message).collect();
    if messages.is_empty() && !has_chat(r, chat) {
        return refuse(NO_CHAT);
    }
    let mut out = Map::new();
    out.insert("account".into(), json!(s.slot));
    out.insert("chat".into(), json!(chat));
    out.insert("live".into(), json!(live_chat(r, s) == chat));
    let none = messages.is_empty();
    out.insert("messages".into(), Value::Array(messages));
    if none {
        out.insert("note".into(), json!("No messages saved for this chat yet: refresh_chat opens it in Teams and reads them"));
    }
    Ok(Value::Object(out))
}

pub fn list_accounts(user_id: &str) -> Res<Value> {
    let mut accounts = vec![];
    for s in appdb::slots_of(user_id)? {
        let a = account_summary(&s)?;
        let mut o = Map::new();
        for k in ["slot", "name", "email", "tenant", "teams", "overall", "stopped", "unread"] {
            set(&mut o, k, a.get(k).cloned());
        }
        accounts.push(Value::Object(o));
    }
    Ok(json!({ "accounts": accounts }))
}

pub fn list_chats(user_id: &str, a: &Map<String, Value>) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let unread_only = truthy_opt(a.get("unread_only"));
    with_slot(s.slot, |r| {
        let live = live_chat(r, &s);
        let chats: Vec<Value> = r
            .chats()
            .into_iter()
            .filter(|c| !unread_only || truthy_opt(c.get("unread")))
            .map(|c| {
                let mut o = Map::new();
                set(&mut o, "name", c.get("name").cloned());
                set(&mut o, "preview", c.get("preview").cloned());
                set(&mut o, "last", c.get("tm").cloned());
                o.insert("unread".into(), json!(truthy_opt(c.get("unread"))));
                if truthy_opt(c.get("mention")) {
                    o.insert("mention".into(), json!(true));
                }
                if truthy_opt(c.get("muted")) {
                    o.insert("muted".into(), json!(true));
                }
                if truthy_opt(c.get("presence")) {
                    o.insert("presence".into(), c["presence"].clone());
                }
                if c.get("name").and_then(Value::as_str) == Some(live.as_str()) {
                    o.insert("open".into(), json!(true));
                }
                Value::Object(o)
            })
            .collect();
        Ok(json!({ "account": s.slot, "chats": chats }))
    })
}

pub fn read_chat(user_id: &str, a: &Map<String, Value>) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    with_slot(s.slot, |r| chat_messages(r, &s, arg_str(a, "chat")))
}

/// The account takes commands, Teams works on it and the chat is in its list: nothing is queued otherwise.
fn can_act(s: &Slot, chat: &str) -> Res<()> {
    if let Some(idle) = commands::idle_reason(s.slot)? {
        return refuse(idle);
    }
    with_slot(s.slot, |r| {
        if !has_chat(r, chat) {
            return refuse(NO_CHAT);
        }
        let h = r.health(up_since(s));
        if h.get("agent") != Some(&json!("ok")) {
            return refuse("Teams is not working on this account (agent not running): see list_accounts");
        }
        if h.get("teams") != Some(&json!("ok")) {
            return refuse(format!("Teams is not working on this account ({}): see list_accounts", js::string(h.get("teams"))));
        }
        Ok(())
    })
}

fn status_of(slot: i64, id: i64) -> Res<Option<(String, Value)>> {
    with_slot(slot, |r| Ok(r.command_status(id).map(|st| (js::string(st.get("status")), st.get("result").cloned().unwrap_or(Value::Null)))))
}

/// Opens the chat in Teams with the open command of the app and waits for the agent: Teams marks the chat as read.
pub async fn refresh_chat(user_id: &str, a: &Map<String, Value>, wait: Wait) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let chat = arg_str(a, "chat");
    can_act(&s, chat)?;
    let id = commands::queue(s.slot, "open", chat, "")?;
    let deadline = Instant::now() + Duration::from_millis(wait.timeout_ms);
    loop {
        let status = status_of(s.slot, id)?.map(|(st, _)| st);
        if status.as_deref() == Some("done") {
            break;
        }
        if status.as_deref() == Some("failed") {
            return refuse("Teams could not open this chat");
        }
        if Instant::now() > deadline {
            return refuse(format!("Teams did not open this chat within {} s: see list_accounts", js::number_to_string(wait.timeout_ms as f64 / 1000.0)));
        }
        tokio::time::sleep(Duration::from_millis(wait.poll_ms)).await;
    }
    with_slot(s.slot, |r| {
        // an agent of an earlier release ends open as done even when Teams did not open the chat: only then it becomes
        // the active chat
        if r.active_chat() != chat {
            return refuse("Teams did not open this chat");
        }
        chat_messages(r, &s, chat)
    })
}

pub fn list_activity(user_id: &str, a: &Map<String, Value>) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let unread_only = truthy_opt(a.get("unread_only"));
    with_slot(s.slot, |r| {
        let act = r.activity();
        let ts = act.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let read_at = if ts != 0.0 { json!(crate::auth::rows::iso_ms((ts * 1000.0).trunc() as i64)) } else { Value::Null };
        let items: Vec<Value> = act
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|i| !unread_only || truthy_opt(i.get("unread")))
            .map(|i| {
                let mut o = Map::new();
                for (from, to) in [("kind", "kind"), ("actor", "actor"), ("title", "title")] {
                    set(&mut o, to, i.get(from).cloned());
                }
                if truthy_opt(i.get("emoji")) {
                    o.insert("emoji".into(), i["emoji"].clone());
                }
                set(&mut o, "preview", i.get("preview").cloned());
                set(&mut o, "time", i.get("tm").cloned());
                set(&mut o, "chat", i.get("chat").cloned());
                if truthy_opt(i.get("channel")) {
                    o.insert("channel".into(), json!(true));
                }
                o.insert("unread".into(), json!(truthy_opt(i.get("unread"))));
                Value::Object(o)
            })
            .collect();
        Ok(json!({ "account": s.slot, "read_at": read_at, "items": items }))
    })
}

// ---- commands: what the app does on a message, run by the agent of the account on Teams

struct Ran {
    id: i64,
    /// done, failed or pending
    status: &'static str,
    result: Value,
}

async fn run(slot: i64, kind: &str, arg1: &str, arg2: &str, wait: Wait) -> Res<Ran> {
    let id = commands::queue(slot, kind, arg1, arg2)?;
    let deadline = Instant::now() + Duration::from_millis(wait.timeout_ms);
    loop {
        if let Some((st, result)) = status_of(slot, id)?
            && st != "pending"
        {
            return Ok(Ran { id, status: if st == "done" { "done" } else { "failed" }, result });
        }
        if Instant::now() > deadline {
            return Ok(Ran { id, status: "pending", result: Value::Null });
        }
        tokio::time::sleep(Duration::from_millis(wait.poll_ms)).await;
    }
}

/// What a tool answers once its command ran: the message as the agent saved it right after (null when not among the
/// saved ones). A failed command may still have reached Teams (the agent stopped while it ran, Teams did not show it).
fn outcome(s: &Slot, chat: &str, ran: &Ran, what: &str, mid: Option<&str>) -> Res<Value> {
    if ran.status == "failed" {
        return refuse(format!("Teams did not confirm {what}: read_chat (or refresh_chat) shows whether it happened, check before trying again"));
    }
    let mut out = Map::new();
    out.insert("account".into(), json!(s.slot));
    out.insert("chat".into(), json!(chat));
    out.insert("command".into(), json!(ran.id));
    out.insert("status".into(), json!(ran.status));
    if ran.status == "pending" {
        out.insert("note".into(), json!("Queued, not run yet: the agent runs it within 2 minutes or drops it. read_chat shows the outcome later"));
        return Ok(Value::Object(out));
    }
    let message = with_slot(s.slot, |r| {
        let all = r.messages(chat);
        let m = match mid {
            None => all.iter().rfind(|x| truthy_opt(x.get("mine")) && !truthy_opt(x.get("deleted"))),
            Some(mid) => all.iter().find(|x| x.get("mid").and_then(Value::as_str) == Some(mid)),
        };
        Ok(m.map(tool_message).unwrap_or(Value::Null))
    })?;
    out.insert("message".into(), message);
    Ok(Value::Object(out))
}

/// base64 of an image, a data: URL included.
fn image_bytes(v: &str) -> Res<Vec<u8>> {
    static DATA_URL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^data:[^,]*;base64,").expect("fixed"));
    let bare: String = DATA_URL.replace(v, "").chars().filter(|c| !c.is_whitespace() && *c != '\u{feff}').collect();
    let data = js::base64_decode_lenient(&bare);
    if data.is_empty() {
        return refuse("image is not base64");
    }
    Ok(data)
}

fn command(kind: &str, fields: Value) -> Res<(String, String, String)> {
    command_of(kind, fields.as_object().expect("an object"))
}

/// A message, a reply with quote, a message tagging people (@name in the text, name in mentions) or an image with an
/// optional caption.
pub async fn send_message(user_id: &str, a: &Map<String, Value>, wait: Wait) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let chat = arg_str(a, "chat");
    let text = arg_str(a, "text");
    let image = truthy_opt(a.get("image"));
    let reply_to = truthy_opt(a.get("reply_to"));
    let mentions = a.get("mentions").and_then(Value::as_array).is_some_and(|m| !m.is_empty());
    if image && (reply_to || mentions) {
        return refuse("An image goes alone or with a caption in text: no reply_to or mentions with it");
    }
    if !image && js_trim(text).is_empty() {
        return refuse("text is required (or image)");
    }
    can_act(&s, chat)?;
    if image {
        let file = crate::uploads::save_image(s.slot, &image_bytes(arg_str(a, "image"))?)?;
        let ran = match run(s.slot, "sendimage", chat, &js::stringify(&json!({ "file": file, "text": text })), wait).await {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_file(crate::uploads::uploads_dir(s.slot).join(&file));
                return Err(e);
            }
        };
        return outcome(&s, chat, &ran, "the image was sent", None);
    }
    if reply_to {
        if mentions {
            return refuse("A reply cannot tag people: send a message with mentions instead");
        }
        let (t, a1, a2) = command("reply", json!({ "type": "reply", "chat": chat, "mid": arg_str(a, "reply_to"), "text": text }))?;
        return outcome(&s, chat, &run(s.slot, &t, &a1, &a2, wait).await?, "the reply was sent", None);
    }
    let checked = commands::message_text(Some(&json!(text)))?;
    let names = commands::mention_names(a.get("mentions"))?;
    let parts = commands::mention_parts(&checked, &names);
    let ran = if parts.iter().any(|p| p.get("mention").is_some()) {
        run(s.slot, "sendmentions", chat, &js::stringify(&json!({ "parts": parts })), wait).await?
    } else {
        run(s.slot, "send", chat, text, wait).await?
    };
    outcome(&s, chat, &ran, "the message was sent", None)
}

/// One of the six quick reactions by name, or the emoji of a reaction already under the message (as read_chat gives
/// it). Like in Teams, a reaction that is already yours is taken away.
pub async fn react_message(user_id: &str, a: &Map<String, Value>, wait: Wait) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let chat = arg_str(a, "chat");
    can_act(&s, chat)?;
    let reaction = arg_str(a, "reaction");
    let name = js_trim(reaction).to_lowercase();
    let mid = arg_str(a, "message_id");
    let fields = if commands::REACTIONS.contains(&name.as_str()) {
        json!({ "type": "react", "chat": chat, "mid": mid, "emoji": name })
    } else {
        json!({ "type": "react", "chat": chat, "mid": mid, "pill": reaction })
    };
    let (t, a1, a2) = command("react", fields)?;
    outcome(&s, chat, &run(s.slot, &t, &a1, &a2, wait).await?, "the reaction", Some(mid))
}

pub async fn edit_message(user_id: &str, a: &Map<String, Value>, wait: Wait) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let chat = arg_str(a, "chat");
    can_act(&s, chat)?;
    let mid = arg_str(a, "message_id");
    let (t, a1, a2) = command("edit", json!({ "type": "edit", "chat": chat, "mid": mid, "text": arg_str(a, "text") }))?;
    outcome(&s, chat, &run(s.slot, &t, &a1, &a2, wait).await?, "the edit", Some(mid))
}

/// delete_message and undo_delete: a command on one message of the account.
pub async fn message_command(user_id: &str, a: &Map<String, Value>, kind: &str, what: &str, wait: Wait) -> Res<Value> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let chat = arg_str(a, "chat");
    can_act(&s, chat)?;
    let mid = arg_str(a, "message_id");
    let (t, a1, a2) = command(kind, json!({ "type": kind, "chat": chat, "mid": mid }))?;
    outcome(&s, chat, &run(s.slot, &t, &a1, &a2, wait).await?, what, Some(mid))
}

// ---- attachments: images the agent saved, SharePoint files it downloads with the Teams session

/// Largest file a tool answer carries.
pub const MAX_INLINE: usize = 8_000_000;
/// Longest text of a text file a tool answer carries, in UTF-16 units.
const MAX_TEXT: usize = 200_000;

static MEDIA_NAME: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[0-9a-f]{16}\.(png|jpg|gif|webp)$").expect("fixed"));
static FILE_NAME: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[0-9a-f]{16}(\.[a-z0-9]{1,8})?$").expect("fixed"));
static TEXT_EXT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^(txt|md|csv|tsv|json|xml|html?|log|ya?ml|ini|conf|cfg|toml|sql|sh|ps1|bat|py|js|ts|java|cs|go|rb|php|css|tf|hcl)$").expect("fixed")
});

fn file_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// path.extname (posix): the last dot of the last segment and what follows, "" for none or a leading dot.
fn extname(name: &str) -> &str {
    let base = &name[name.rfind('/').map_or(0, |i| i + 1)..];
    match base.rfind('.') {
        Some(i) if i > 0 && base != ".." => &base[i..],
        _ => "",
    }
}

/// isSharePointUrl of src/agent/logic/files.ts: SharePoint and OneDrive for work, over HTTPS.
fn is_share_point_url(url: &str) -> bool {
    url.starts_with("https://") && url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.ends_with(".sharepoint.com"))).unwrap_or(false)
}

/// An image of a message the page could not fetch, by its public link (Giphy...): HTTPS to a named host only, never
/// an address of the server's own network. The bytes and the type, or why not.
async fn public_image(url: &str) -> Result<(Vec<u8>, String), String> {
    static LOCAL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)(^|\.)(localhost|local|internal|lan)$").expect("fixed"));
    static TEAMS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)(^|\.)(asm\.skype\.com|teams\.microsoft\.com|teams\.cloud\.microsoft)$").expect("fixed"));
    let Ok(u) = url::Url::parse(url) else { return Err("not a link".into()) };
    let host = u.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']').to_string();
    if u.scheme() != "https" || host.parse::<std::net::IpAddr>().is_ok() || !host.contains('.') || LOCAL.is_match(&host) {
        return Err("not a public link".into());
    }
    if TEAMS.is_match(&host) {
        return Err("Teams keeps this image behind the sign-in of the account and it is not saved yet: refresh_chat saves it".into());
    }
    let fetch = async {
        let r = crate::http_client().get(u.as_str()).send().await.map_err(|_| "fetch failed".to_string())?;
        // redirect: "error" of fetch
        if [301, 302, 303, 307, 308].contains(&r.status().as_u16()) {
            return Err("fetch failed".to_string());
        }
        let header = |k: &str| r.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let kind = header("content-type").split(';').next().unwrap_or("").trim().to_string();
        if !r.status().is_success() || !kind.starts_with("image/") {
            return Ok(Err(format!("the link answered {} {}", r.status().as_u16(), if kind.is_empty() { "without a type" } else { &kind })));
        }
        let length = r.headers().get("content-length").and_then(|v| v.to_str().ok()).map(|v| js::number(Some(&json!(v)))).unwrap_or(0.0);
        if length > MAX_INLINE as f64 {
            return Ok(Err("larger than 8 MB".into()));
        }
        let data = r.bytes().await.map_err(|_| "fetch failed".to_string())?;
        if data.len() > MAX_INLINE {
            return Ok(Err("larger than 8 MB".into()));
        }
        Ok(Ok((data.to_vec(), kind)))
    };
    match tokio::time::timeout(Duration::from_secs(15), fetch).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(format!("not fetched: {e}")),
        Err(_) => Err("not fetched: The operation was aborted due to timeout".into()),
    }
}

/// The file of a finished download: a relay uploads it to the server right after, so it may come a little later.
async fn downloaded(slot: i64, f: &str, wait: Wait) -> Option<Vec<u8>> {
    let file = slot_dir(slot).join("files").join(f);
    let deadline = Instant::now() + Duration::from_millis(wait.timeout_ms.min(20_000));
    loop {
        if let Ok(data) = std::fs::read(&file) {
            return Some(data);
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(wait.poll_ms)).await;
    }
}

/// An attachment of a message: an image (its saved file, its link) or a file (its name, its link).
struct Item {
    image: bool,
    f: Option<Value>,
    name: Option<Value>,
    url: Option<Value>,
}

/// The images (first, in order) and files of a saved message, as content of the answer: images to look at, text files
/// as text, other files as embedded resources. index: 1-based, the images first, then the files; all when missing.
pub async fn get_attachment(user_id: &str, a: &Map<String, Value>, wait: Wait) -> Res<Vec<Value>> {
    let s = account_of(user_id, arg_int(a, "account"))?;
    let chat = arg_str(a, "chat");
    let mid = arg_str(a, "message_id");
    let m = with_slot(s.slot, |r| Ok(r.messages(chat).into_iter().find(|x| x.get("mid").and_then(Value::as_str) == Some(mid))))?;
    let Some(m) = m else { return refuse("No saved message with this id in this chat: read_chat gives the ids (refresh_chat for current ones)") };
    let list_of = |k: &str| m.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut all: Vec<Item> =
        list_of("images").iter().map(|im| Item { image: true, f: im.get("f").cloned(), name: None, url: im.get("url").cloned() }).collect();
    all.extend(list_of("files").iter().map(|f| Item { image: false, f: None, name: f.get("name").cloned(), url: f.get("url").cloned() }));
    if all.is_empty() {
        return refuse("This message has no image or file");
    }
    let index = arg_int(a, "index");
    if let Some(i) = index
        && (i < 1 || i as usize > all.len())
    {
        return refuse(format!("index out of range: the message has {} attachment(s)", all.len()));
    }
    let picked: Vec<(usize, &Item)> = match index {
        None => all.iter().enumerate().map(|(i, x)| (i + 1, x)).collect(),
        Some(i) => vec![(i as usize, &all[i as usize - 1])],
    };

    let mut blocks: Vec<Value> = vec![];
    let mut list: Vec<Value> = vec![];
    for (index, x) in picked {
        let url = x.url.as_ref().and_then(Value::as_str).unwrap_or("");
        let mut at = Map::new();
        at.insert("index".into(), json!(index));
        if x.image {
            at.insert("kind".into(), json!("image"));
            at.insert("shown".into(), json!(false));
            let saved = x.f.as_ref().and_then(Value::as_str).filter(|f| MEDIA_NAME.is_match(f)).and_then(|f| {
                let data = std::fs::read(slot_dir(s.slot).join("media").join(f)).ok()?;
                Some((data, crate::uploads::image_type(&extname(f)[1..])))
            });
            if let Some((data, kind)) = saved {
                blocks.push(json!({ "type": "image", "data": js::base64_std(&data), "mimeType": kind }));
                at.insert("shown".into(), json!(true));
            } else if !truthy_opt(x.url.as_ref()) {
                at.insert("note".into(), json!("the image is not saved on the server (yet): refresh_chat saves the images of the chat"));
            } else {
                match public_image(url).await {
                    Err(why) => {
                        at.insert("note".into(), json!(why));
                        at.insert("link".into(), json!(url));
                    }
                    Ok((data, kind)) => {
                        blocks.push(json!({ "type": "image", "data": js::base64_std(&data), "mimeType": kind }));
                        at.insert("shown".into(), json!(true));
                    }
                }
            }
            list.push(Value::Object(at));
            continue;
        }
        at.insert("kind".into(), json!("file"));
        set(&mut at, "name", x.name.clone());
        at.insert("shown".into(), json!(false));
        let name = x.name.as_ref().map_or_else(|| "undefined".to_string(), |n| js::string(Some(n)));
        if let Some(block) = file_block(&s, url, x.url.as_ref(), x.name.as_ref(), &name, &mut at, wait).await {
            blocks.push(block);
            at.insert("shown".into(), json!(true));
        }
        list.push(Value::Object(at));
    }
    let summary = json!({ "account": s.slot, "chat": chat, "message": tool_message(&m), "attachments": list });
    let mut out = vec![json!({ "type": "text", "text": js::stringify(&summary) })];
    out.extend(blocks);
    Ok(out)
}

/// A file of a message downloaded from SharePoint with the Teams session, as a content block; None with the note and
/// link of why not set on `at`.
async fn file_block(
    s: &Slot,
    url: &str,
    raw_url: Option<&Value>,
    raw_name: Option<&Value>,
    name: &str,
    at: &mut Map<String, Value>,
    wait: Wait,
) -> Option<Value> {
    if !is_share_point_url(url) {
        at.insert("note".into(), json!("not on SharePoint or OneDrive: not downloaded"));
        set(at, "link", raw_url.cloned());
        return None;
    }
    let mut args = Map::new();
    set(&mut args, "name", raw_name.cloned());
    let ran = match run(s.slot, "download", url, &js::stringify(&Value::Object(args)), wait).await {
        Ok(r) => r,
        Err(e) => {
            at.insert("note".into(), json!(message_of(e)));
            at.insert("link".into(), json!(url));
            return None;
        }
    };
    let f = ran.result.get("f").and_then(Value::as_str).filter(|_| ran.result.is_object()).map(String::from);
    let Some(f) = f.filter(|f| ran.status == "done" && FILE_NAME.is_match(f)) else {
        let note = if ran.status == "pending" { "download still queued: ask again in a minute" } else { "Teams did not give the file" };
        at.insert("note".into(), json!(note));
        at.insert("link".into(), json!(url));
        return None;
    };
    let data = downloaded(s.slot, &f, wait).await;
    let link = format!("{}/files/{f}?a={}&name={}", crate::config::app_url(), s.slot, js::encode_uri_component(name));
    at.insert("link".into(), json!(link));
    let Some(data) = data else {
        at.insert("note".into(), json!("downloaded, not on the server yet: ask again in a minute"));
        return None;
    };
    if data.len() > MAX_INLINE {
        let mb = (data.len() as f64 / 1e6 + 0.5).floor();
        at.insert("note".into(), json!(format!("{} MB, too large to carry here: open the link signed in to the app", js::number_to_string(mb))));
        return None;
    }
    let ext = extname(name).get(1..).unwrap_or("").to_lowercase();
    let kind = file_type(&ext).unwrap_or(if TEXT_EXT.is_match(&ext) { "text/plain" } else { "application/octet-stream" });
    Some(if kind.starts_with("image/") {
        json!({ "type": "image", "data": js::base64_std(&data), "mimeType": kind })
    } else if kind == "text/plain" {
        let text = String::from_utf8_lossy(&data);
        let units: Vec<u16> = text.encode_utf16().collect();
        let body = if units.len() > MAX_TEXT { format!("{}\n[cut]", String::from_utf16_lossy(&units[..MAX_TEXT])) } else { text.into_owned() };
        json!({ "type": "text", "text": format!("--- {name} ---\n{body}") })
    } else {
        json!({ "type": "resource", "resource": { "uri": link, "mimeType": kind, "blob": js::base64_std(&data) } })
    })
}

/// (e as Error).message: the detail of an HttpError, the text of anything else.
fn message_of(e: Error) -> String {
    match e {
        Error::Http(_, d, _) => d,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_rules() {
        assert_eq!(message_time(Some(&json!("1790431664072"))).as_deref(), Some("2026-09-26T14:07:44.072Z"));
        assert_eq!(message_time(Some(&json!("temp-1"))), None);
        assert_eq!(message_time(Some(&json!("123"))), None);
        assert_eq!(extname("a/b.c.CSV"), ".CSV");
        assert_eq!(extname(".bashrc"), "");
        assert_eq!(extname("noext"), "");
        assert!(is_share_point_url("https://contoso.sharepoint.com/x"));
        assert!(!is_share_point_url("http://contoso.sharepoint.com/x"));
        assert!(!is_share_point_url("https://example.com/a.pdf"));
        assert_eq!(image_bytes("data:image/png;base64,aGk=").ok(), Some(b"hi".to_vec()));
        assert!(image_bytes("***").is_err());
        assert_eq!(js_trim("\u{feff} x \n"), "x");
    }
}
