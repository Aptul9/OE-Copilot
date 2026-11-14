//! Accounts on another computer (src/lib/relay.ts): the local relay there keeps data/N/messages.db of its slot up to
//! date through /api/relay/*, runs the commands the app queues in it and sends its notifications through the web app.

use crate::appdb;
use crate::slotdb::{self, ensure_slot_schema, insert_row};
use crate::state::{Viewing, key};
use crate::web::{Error, Req, Res, err, now_ms};
use crate::zod::{self, Z, arr, null, obj, opt, s};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const ON_ANOTHER_COMPUTER: &str = "This Teams account runs on another computer: start, stop and sign in there, in the window of its relay";
pub const COMMANDS_WAIT_MS: i64 = 25_000;

pub fn new_relay_token() -> String {
    crate::js::base64_url(&crate::auth::crypto::random(32))
}

pub fn relay_digest(token: &str) -> String {
    crate::auth::crypto::sha256_hex(token)
}

/// The account a request of a relay comes from.
#[derive(Clone, Debug)]
pub struct Caller {
    pub slot: i64,
    pub added: i64,
    pub digest: String,
}

fn refused() -> Error {
    Error::Http(401, "Missing or wrong token".into(), vec![("WWW-Authenticate".into(), "Bearer realm=\"teamsrelay\"".into())])
}

/// bearerToken: the token of an Authorization: Bearer header, "" without one.
pub fn bearer_token(header: Option<&str>) -> String {
    let re = regex::Regex::new(r"(?i)^Bearer\s+(\S+)\s*$").expect("fixed");
    header.and_then(|h| re.captures(h)).map(|c| c[1].to_string()).unwrap_or_default()
}

/// sameToken: compared by digests of one length.
pub fn same_token(given: &str, expected: &str) -> bool {
    let (a, b) = (crate::auth::crypto::sha256_hex(given), crate::auth::crypto::sha256_hex(expected));
    a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// The account of the relay calling: Authorization: Bearer <its token>, never from a web page.
pub fn require_relay(req: &Req) -> Res<Caller> {
    if req.headers.contains_key("origin") {
        return Err(err(403, "Requests from web pages are not accepted"));
    }
    let token = bearer_token(req.header("authorization").as_deref());
    let digest = if token.is_empty() { String::new() } else { relay_digest(&token) };
    let found = if digest.is_empty() { None } else { appdb::relay_account(&digest)? };
    let (slot, added, _) = found.ok_or_else(refused)?;
    Ok(Caller { slot, added, digest })
}

/// Right before a write: the token still belongs to the account the request started for.
fn still_relay(c: &Caller) -> Res<()> {
    match appdb::relay_account(&c.digest)? {
        Some((slot, added, _)) if slot == c.slot && added == c.added => Ok(()),
        _ => Err(refused()),
    }
}

/// relayJson: a JSON body of at most `max` bytes, checked against its schema.
pub fn relay_json(req: &Req, z: &Z, max: f64) -> Res<Value> {
    if req.declared_length().unwrap_or(0.0) > max || req.body.len() as f64 > max {
        return Err(err(413, "Body too large"));
    }
    let data: Value = serde_json::from_slice(&req.body).map_err(|_| err(400, "Invalid JSON body"))?;
    zod::parse(z, &data).map_err(|issues| err(400, format!("Invalid body: {}", zod::first_three(&issues))))
}

fn chat_row() -> Z {
    obj(vec![
        ("name", s(1000)),
        ("preview", null(s(20_000))),
        ("pos", null(Z::Int)),
        ("ts", null(Z::Int)),
        ("tm", null(s(100))),
        ("unread", null(Z::Int)),
        ("mention", null(Z::Int)),
        ("muted", null(Z::Int)),
        ("av", null(s(100))),
        ("presence", opt(null(s(20)))),
        ("kind", opt(null(s(50)))),
    ])
}

fn message_row() -> Z {
    obj(vec![
        ("idx", Z::Int),
        ("mid", null(s(200))),
        ("author", null(s(1000))),
        ("text", null(s(200_000))),
        ("mine", null(Z::Int)),
        ("reacts", null(s(20_000))),
        ("extra", null(s(2_000_000))),
    ])
}

fn activity_row() -> Z {
    obj(vec![
        ("id", s(200)),
        ("pos", null(Z::Int)),
        ("kind", null(s(50))),
        ("actor", null(s(1000))),
        ("title", null(s(20_000))),
        ("emoji", null(s(100))),
        ("preview", null(s(20_000))),
        ("tm", null(s(100))),
        ("chat", null(s(1000))),
        ("channel", null(Z::Int)),
        ("unread", null(Z::Int)),
        ("ts", null(Z::Int)),
        ("av", null(s(100))),
    ])
}

pub const COMMAND_STATUSES: &[&str] = &["pending", "done", "failed", "running", "unconfirmed"];
const URGENCY: &[&str] = &["very-low", "low", "normal", "high"];

pub fn sync_body() -> Z {
    obj(vec![
        ("host", s(100)),
        ("now", Z::PosInt),
        ("chats", opt(arr(chat_row(), 2000))),
        ("messages", opt(Z::Record(Some(1000), Box::new(arr(message_row(), 2000)), Some((2000, "too many chats"))))),
        ("state", opt(Z::Record(Some(1000), Box::new(null(s(2_000_000))), Some((5000, "too many keys"))))),
        ("activity", opt(arr(activity_row(), 2000))),
        ("calls", opt(arr(obj(vec![("since", Z::Int), ("caller", null(s(1000))), ("seconds", null(Z::Int))]), 500))),
        (
            "readby",
            opt(arr(obj(vec![("mid", s(200)), ("chat", null(s(1000))), ("label", null(s(1000))), ("names", null(s(100_000))), ("ts", null(Z::Int))]), 20_000)),
        ),
        ("commands", opt(arr(obj(vec![("id", Z::PosInt), ("status", Z::Enum(COMMAND_STATUSES))]), 5000))),
    ])
}

pub fn have_body() -> Z {
    obj(vec![("media", arr(s(100), 20_000)), ("files", arr(s(100), 20_000))])
}

pub fn push_body() -> Z {
    Z::Union(
        "op",
        vec![
            ("message", obj(vec![("op", Z::Enum(&["message"])), ("title", s(1000)), ("body", s(20_000)), ("chat", s(1000))])),
            ("alert", obj(vec![("op", Z::Enum(&["alert"])), ("title", s(1000)), ("body", s(20_000)), ("urgency", Z::Enum(URGENCY))])),
            ("notice", obj(vec![("op", Z::Enum(&["notice"])), ("title", s(1000)), ("body", s(20_000)), ("urgency", Z::Enum(URGENCY))])),
            (
                "call",
                obj(vec![
                    ("op", Z::Enum(&["call"])),
                    ("caller", s(1000)),
                    ("state", Z::Enum(&["ringing", "again", "ended"])),
                    ("since", Z::Int),
                    ("seconds", Z::Int),
                ]),
            ),
            ("missedCall", obj(vec![("op", Z::Enum(&["missedCall"])), ("caller", s(1000)), ("time", s(100))])),
        ],
    )
}

/// A new account on another computer starts from an empty database of the slot schema.
pub fn create_relay_slot(slot: i64) -> Res<()> {
    let dir = slotdb::slot_dir(slot);
    crate::slots::remove_dir(&dir)?;
    slotdb::open_store(&dir.join("messages.db"))?;
    Ok(())
}

fn slot_db_file(slot: i64) -> Res<PathBuf> {
    let f = slotdb::slot_db_path(slot);
    if !f.exists() {
        return Err(Error::NotReady);
    }
    Ok(f)
}

/// The database of the slot, made when the account was added and never here.
fn open_slot_db(slot: i64) -> Res<Connection> {
    let c = Connection::open_with_flags(slot_db_file(slot)?, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.busy_timeout(std::time::Duration::from_millis(8000))?;
    ensure_slot_schema(&c)?;
    Ok(c)
}

fn viewing_ts(v: Option<&str>) -> f64 {
    Viewing::parse(v).ts
}

/// The health, the ringing call and the call in progress go by the clock of the server.
fn on_server_clock(k: &str, v: &str, at: &dyn Fn(f64) -> f64) -> String {
    if ![key::HEALTH, key::CALL, key::IN_CALL].contains(&k) {
        return v.to_string();
    }
    let Ok(Value::Object(mut row)) = serde_json::from_str::<Value>(v) else { return v.to_string() };
    if k == key::HEALTH
        && let Some(ts) = row.get("ts").and_then(Value::as_f64)
    {
        row.insert("ts".into(), crate::web::num((at(ts * 1000.0) / 1000.0).floor()));
        return crate::js::stringify(&Value::Object(row));
    }
    let flag = if k == key::CALL { "ringing" } else { "active" };
    if k != key::HEALTH
        && row.get(flag) == Some(&Value::Bool(true))
        && let Some(seen) = row.get("seen").and_then(Value::as_f64)
    {
        row.insert("seen".into(), crate::web::num(at(seen)));
        return crate::js::stringify(&Value::Object(row));
    }
    v.to_string()
}

fn sql(v: Option<&Value>) -> rusqlite::types::Value {
    match v {
        None | Some(Value::Null) => rusqlite::types::Value::Null,
        Some(Value::Number(n)) => n.as_i64().map(rusqlite::types::Value::Integer).unwrap_or_else(|| rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0))),
        Some(Value::String(t)) => rusqlite::types::Value::Text(t.clone()),
        Some(Value::Bool(b)) => rusqlite::types::Value::Integer(i64::from(*b)),
        Some(other) => rusqlite::types::Value::Text(crate::js::stringify(other)),
    }
}

fn row_values(o: &Value, cols: &[&str]) -> Vec<rusqlite::types::Value> {
    cols.iter().map(|c| sql(o.get(*c))).collect()
}

/// What changed in relay.db since the last sync, into data/N/messages.db in one transaction.
pub fn apply_sync(caller: &Caller, b: &Value, now: i64) -> Res<()> {
    still_relay(caller)?;
    let db = open_slot_db(caller.slot)?;
    let sent = b.get("now").and_then(Value::as_f64).unwrap_or(0.0);
    let now_f = now as f64;
    let at = move |ms: f64| now_f.min(ms + now_f - sent);
    {
        let tx = db.unchecked_transaction()?;
        if let Some(chats) = b.get("chats").and_then(Value::as_array) {
            tx.execute("DELETE FROM chats", [])?;
            let mut ins = tx.prepare(&insert_row("chats"))?;
            for c in chats {
                let mut v = row_values(c, &["name", "preview", "pos", "ts", "tm", "unread", "mention", "muted", "av"]);
                v.push(sql(c.get("presence")));
                v.push(match c.get("kind") {
                    None | Some(Value::Null) => rusqlite::types::Value::Text(String::new()),
                    k => sql(k),
                });
                ins.execute(rusqlite::params_from_iter(v))?;
            }
        }
        if let Some(Value::Object(m)) = b.get("messages") {
            let mut clear = tx.prepare("DELETE FROM chat_messages WHERE chat=?")?;
            let mut ins = tx.prepare(&insert_row("chat_messages"))?;
            for (chat, rows) in m {
                clear.execute([chat])?;
                for r in rows.as_array().cloned().unwrap_or_default() {
                    let mut v = vec![rusqlite::types::Value::Text(chat.clone())];
                    v.extend(row_values(&r, &["idx", "mid", "author", "text", "mine", "reacts", "extra"]));
                    ins.execute(rusqlite::params_from_iter(v))?;
                }
            }
        }
        if let Some(items) = b.get("activity").and_then(Value::as_array) {
            tx.execute("DELETE FROM activity", [])?;
            let mut ins = tx.prepare(&insert_row("activity"))?;
            for a in items {
                ins.execute(rusqlite::params_from_iter(row_values(
                    a,
                    &["id", "pos", "kind", "actor", "title", "emoji", "preview", "tm", "chat", "channel", "unread", "ts", "av"],
                )))?;
            }
        }
        if let Some(calls) = b.get("calls").and_then(Value::as_array) {
            tx.execute("DELETE FROM calls", [])?;
            let mut ins = tx.prepare(&insert_row("calls"))?;
            for c in calls {
                ins.execute(rusqlite::params_from_iter(row_values(c, &["since", "caller", "seconds"])))?;
            }
        }
        if let Some(rows) = b.get("readby").and_then(Value::as_array) {
            let mut ins = tx.prepare(&insert_row("readby"))?;
            for r in rows {
                ins.execute(rusqlite::params_from_iter(row_values(r, &["mid", "chat", "label", "names", "ts"])))?;
            }
        }
        if let Some(Value::Object(st)) = b.get("state") {
            for (k, v) in st {
                if k == key::RELAY {
                    continue;
                }
                if k == key::VIEWING {
                    let current: Option<String> = tx.query_row("SELECT v FROM state WHERE k=?", [k], |r| r.get(0)).optional()?.flatten();
                    if let Some(v) = v.as_str()
                        && viewing_ts(Some(v)) > viewing_ts(current.as_deref())
                    {
                        tx.execute("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)", params![k, v])?;
                    }
                } else if v.is_null() {
                    tx.execute("DELETE FROM state WHERE k=?", [k])?;
                } else {
                    let v = on_server_clock(k, v.as_str().unwrap_or(""), &at);
                    tx.execute("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)", params![k, v])?;
                }
            }
        }
        if let Some(cmds) = b.get("commands").and_then(Value::as_array) {
            for c in cmds {
                tx.execute("UPDATE commands SET status=? WHERE id=?", rusqlite::params_from_iter([sql(c.get("status")), sql(c.get("id"))]))?;
            }
        }
        let host = b.get("host").cloned().unwrap_or(json!(""));
        tx.execute(
            "INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)",
            params![key::RELAY, crate::js::stringify(&json!({ "host": host, "seen": now.div_euclid(1000) }))],
        )?;
        tx.commit()?;
    }
    let me_changed = b.get("state").and_then(Value::as_object).is_some_and(|s| s.contains_key(key::ME));
    if b.get("chats").is_some() || b.get("messages").is_some() || b.get("activity").is_some() || me_changed {
        prune_relay_media(caller, &media_files_of(&db));
    }
    Ok(())
}

/// The commands queued after `after` and the chat the app shows when newer than `vts`: at once when there are any,
/// else as soon as some come, at the latest after `wait_ms`.
pub async fn wait_for_relay_commands(caller: &Caller, after: i64, vts: f64, wait_ms: i64) -> Res<(Vec<Value>, Value)> {
    let db = open_slot_db(caller.slot)?;
    let end = now_ms() + wait_ms;
    loop {
        still_relay(caller)?;
        let commands: Vec<Value> = slotdb::rows(
            &db,
            "SELECT id, COALESCE(ts, 0) AS ts, type, COALESCE(arg1, '') AS arg1, COALESCE(arg2, '') AS arg2 FROM commands WHERE id > ? AND status='pending' ORDER BY id LIMIT 100",
            [after],
        )?
        .into_iter()
        .map(Value::Object)
        .collect();
        let raw: Option<String> = db.query_row("SELECT v FROM state WHERE k=?", [key::VIEWING], |r| r.get(0)).optional()?.flatten();
        let v = Viewing::parse(raw.as_deref());
        let shown = if v.ts > vts { json!({ "chat": v.chat, "ts": crate::web::num(v.ts) }) } else { Value::Null };
        if !commands.is_empty() || !shown.is_null() || now_ms() >= end {
            return Ok((commands, shown));
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

struct Pusher {
    added: i64,
    notifier: Rc<crate::push::Notifier>,
}

struct Room {
    added: i64,
    used: f64,
}

thread_local! {
    static PUSHERS: RefCell<HashMap<i64, Pusher>> = RefCell::new(HashMap::new());
    static ROOMS: RefCell<HashMap<i64, Room>> = RefCell::new(HashMap::new());
}

fn holds(slot: i64, added: i64) -> bool {
    appdb::slot_row(slot).ok().flatten().is_some_and(|s| s.relay != 0 && s.added == added)
}

fn relay_notifier(caller: &Caller) -> Res<Rc<crate::push::Notifier>> {
    let known = PUSHERS.with(|p| p.borrow().get(&caller.slot).map(|x| (x.added, x.notifier.clone())));
    if let Some((added, n)) = &known
        && *added == caller.added
    {
        return Ok(n.clone());
    }
    if known.is_some() {
        forget_relay(caller.slot);
    }
    let push = crate::config::push().map_err(Error::Internal)?;
    let vapid = match crate::push::load_vapid_keys(&push.vapid_private_key_file, &push.vapid_app_key_file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("relay {}: push keys: {e}", caller.slot);
            None
        }
    };
    let fcm = match crate::push::load_service_account(&push.fcm_credentials) {
        Ok(Some(sa)) => Some(crate::push::FcmSender::new(sa)),
        Ok(None) => None,
        Err(e) => {
            eprintln!("relay {}: FCM key: {e}", caller.slot);
            None
        }
    };
    let store_file = slot_db_file(caller.slot)?;
    let (slot, added) = (caller.slot, caller.added);
    let notifier = Rc::new(crate::push::Notifier::new(crate::push::NotifierOptions {
        store_file,
        devices: Box::new(crate::push::Devices { store: crate::push::AppStore::new(slot), holds: Box::new(move || holds(slot, added)) }),
        vapid,
        subject: push.vapid_subject.clone(),
        ntfy: push.ntfy.clone(),
        fcm,
        answerable: true,
    }));
    PUSHERS.with(|p| p.borrow_mut().insert(slot, Pusher { added, notifier: notifier.clone() }));
    Ok(notifier)
}

/// The account is removed: a later account on the slot gets a notifier and a room of its own.
pub fn forget_relay(slot: i64) {
    PUSHERS.with(|p| p.borrow_mut().remove(&slot));
    ROOMS.with(|r| r.borrow_mut().remove(&slot));
}

/// A notification the relay passes on to the devices of the owner: the devices the push service took.
pub async fn relay_push(caller: &Caller, b: &Value) -> Res<usize> {
    still_relay(caller)?;
    let n = relay_notifier(caller)?;
    let s = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let i = |k: &str| b.get(k).and_then(Value::as_i64).unwrap_or(0);
    Ok(match s("op").as_str() {
        "message" => {
            n.message(&s("title"), &s("body"), &s("chat")).await;
            n.device_count()
        }
        "alert" => n.alert(&s("title"), &s("body"), &s("urgency")).await,
        "notice" => n.notice(&s("title"), &s("body"), &s("urgency")).await,
        "call" => n.call(&s("caller"), &s("state"), i("since"), i("seconds"), false).await,
        _ => n.missed_call(&s("caller"), &s("time")).await,
    })
}

pub fn media_name() -> regex::Regex {
    regex::Regex::new(r"^[0-9a-f]{16}\.(png|jpg|gif|webp)$").expect("fixed")
}

pub fn file_name() -> regex::Regex {
    regex::Regex::new(r"^[0-9a-f]{16}(\.[a-z0-9]{1,8})?$").expect("fixed")
}

pub const MAX_DOWNLOAD: f64 = 100e6;

fn folder_of(slot: i64, kind: &str) -> PathBuf {
    slotdb::slot_dir(slot).join(if kind == "media" { "media" } else { "files" })
}

/// Of the files the relay has, the ones the slot lacks.
pub fn missing_relay_files(slot: i64, have: &Value) -> Value {
    let missing = |kind: &str, re: regex::Regex| -> Vec<Value> {
        let mut seen = HashSet::new();
        have.get(kind)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(String::from))
            .filter(|n| seen.insert(n.clone()))
            .filter(|n| re.is_match(n) && !folder_of(slot, kind).join(n).exists())
            .map(Value::String)
            .collect()
    };
    json!({ "media": missing("media", media_name()), "files": missing("files", file_name()) })
}

/// mediaFilesOf: the files of the media folder the rows of a slot database name.
pub fn media_files_of(db: &Connection) -> HashSet<String> {
    let mut files = HashSet::new();
    let mut add = |v: Option<&Value>| {
        if let Some(Value::String(s)) = v
            && !s.is_empty()
        {
            files.insert(s.clone());
        }
    };
    for r in slotdb::rows(db, "SELECT av FROM chats UNION SELECT av FROM activity", []).unwrap_or_default() {
        add(r.get("av"));
    }
    for r in slotdb::rows(db, "SELECT extra FROM chat_messages", []).unwrap_or_default() {
        let extra = slotdb::parse_extra(r.get("extra").and_then(Value::as_str));
        add(extra.get("av"));
        if let Some(Value::Array(images)) = extra.get("images") {
            for im in images {
                add(im.get("f"));
            }
        }
    }
    let me: Option<String> = db.query_row("SELECT v FROM state WHERE k=?", [key::ME], |r| r.get(0)).optional().ok().flatten().flatten();
    if let Some(m) = crate::state::object(me.as_deref()) {
        add(m.get("av").filter(|v| v.is_string()));
        add(m.get("logo").filter(|v| v.is_string()));
    }
    files
}

/// pruneMedia: the files of a media folder no row names any more; `removed` hears the size of each.
pub fn prune_media(dir: &Path, named: &HashSet<String>, removed: &mut dyn FnMut(u64)) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let re = media_name();
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !re.is_match(&name) || named.contains(&name) {
            continue;
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        if let Err(x) = std::fs::remove_file(e.path()) {
            eprintln!("media: {x} {name}");
            continue;
        }
        removed(size);
        n += 1;
    }
    n
}

fn prune_relay_media(caller: &Caller, named: &HashSet<String>) {
    let slot = caller.slot;
    prune_media(&folder_of(slot, "media"), named, &mut |size| {
        ROOMS.with(|r| {
            if let Some(room) = r.borrow_mut().get_mut(&slot) {
                room.used -= size as f64;
            }
        })
    });
}

fn folder_bytes(dir: &Path) -> f64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0.0 };
    entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.metadata().map(|m| m.len() as f64).unwrap_or(0.0)).sum()
}

fn room_used(caller: &Caller) -> f64 {
    ROOMS.with(|r| {
        let mut r = r.borrow_mut();
        let fresh = r.get(&caller.slot).is_none_or(|x| x.added != caller.added);
        if fresh {
            let used = folder_bytes(&folder_of(caller.slot, "media")) + folder_bytes(&folder_of(caller.slot, "files"));
            r.insert(caller.slot, Room { added: caller.added, used });
        }
        r.get(&caller.slot).map_or(0.0, |x| x.used)
    })
}

fn room_add(slot: i64, d: f64) {
    ROOMS.with(|r| {
        if let Some(room) = r.borrow_mut().get_mut(&slot) {
            room.used += d;
        }
    })
}

/// A file of the relay, written whole or not at all, within the room of the account (RELAY_QUOTA_MB).
pub async fn save_relay_file(caller: &Caller, kind: &str, name: &str, body: Option<hyper::body::Incoming>, length: Option<f64>) -> Res<()> {
    use http_body_util::BodyExt;
    use tokio::io::AsyncWriteExt;
    let (re, max) = if kind == "media" { (media_name(), 10e6) } else { (file_name(), MAX_DOWNLOAD) };
    if !re.is_match(name) {
        return Err(err(400, "Invalid file name"));
    }
    let Some(mut body) = body else { return Err(err(400, "Missing file")) };
    slot_db_file(caller.slot)?;
    let used = room_used(caller);
    let quota = crate::config::relay_quota_bytes();
    let too_large = || err(413, "File too large");
    let no_room = || err(413, "No room left for the files of this account on the server");
    if let Some(l) = length.filter(|l| l.is_finite()) {
        if l > max {
            return Err(too_large());
        }
        if used + l > quota {
            return Err(no_room());
        }
    }
    let dir = folder_of(caller.slot, kind);
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(name);
    let part = dir.join(format!("{name}.{}.part", crate::auth::crypto::hex(&crate::auth::crypto::random(4))));
    let mut size = 0f64;
    let mut held = 0f64;
    let result: Res<()> = async {
        let mut f = tokio::fs::File::create(&part).await?;
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| Error::Internal(format!("upload: {e}")))?;
            let Some(chunk) = frame.data_ref() else { continue };
            size += chunk.len() as f64;
            if size > max {
                return Err(too_large());
            }
            let used_now = ROOMS.with(|r| r.borrow().get(&caller.slot).map_or(0.0, |x| x.used));
            if used_now + chunk.len() as f64 > quota {
                return Err(no_room());
            }
            room_add(caller.slot, chunk.len() as f64);
            held += chunk.len() as f64;
            f.write_all(chunk).await?;
        }
        f.flush().await?;
        drop(f);
        if size == 0.0 {
            return Err(err(400, "Missing file"));
        }
        if kind == "media" {
            let head: Vec<u8> = std::fs::read(&part)?.into_iter().take(16).collect();
            let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("");
            if crate::uploads::image_ext(&head) != Some(ext) {
                return Err(err(415, "Not the image its name says"));
            }
        }
        still_relay(caller)?;
        let replaced = std::fs::metadata(&target).map(|m| m.len() as f64).unwrap_or(0.0);
        std::fs::rename(&part, &target)?;
        room_add(caller.slot, -replaced);
        Ok(())
    }
    .await;
    if result.is_err() {
        room_add(caller.slot, -held);
    }
    let _ = std::fs::remove_file(&part);
    result
}

/// The answer of GET /api/relay/commands.
pub fn commands_answer(added: i64, commands: Vec<Value>, viewing: Value, devices: i64) -> Value {
    let mut m = Map::new();
    m.insert("added".into(), json!(added));
    m.insert("commands".into(), Value::Array(commands));
    m.insert("viewing".into(), viewing);
    m.insert("devices".into(), json!(devices));
    Value::Object(m)
}
