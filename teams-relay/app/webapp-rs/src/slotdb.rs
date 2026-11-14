//! data/N/messages.db, created and written by the agent of slot N: the web app reads it and queues commands in it
//! (src/lib/slotdb.ts). Opening it never creates an empty file; a table the agent has not made yet reads as empty.

use crate::state::{self, CallState, Identity, InCall, RelayLink, key};
use crate::web::{Error, Res, now_ms, now_s, num};
use rusqlite::{Connection, OpenFlags, params, types::Value as Sql};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

pub fn slot_dir(slot: i64) -> PathBuf {
    crate::config::data_dir().join(slot.to_string())
}

pub fn slot_db_path(slot: i64) -> PathBuf {
    slot_dir(slot).join("messages.db")
}

/// A value of SQLite as better-sqlite3 hands it to JSON.stringify.
pub fn sql_json(v: Sql) -> Value {
    match v {
        Sql::Null => Value::Null,
        Sql::Integer(i) => json!(i),
        Sql::Real(f) => num(f),
        Sql::Text(t) => Value::String(t),
        Sql::Blob(b) => json!({ "type": "Buffer", "data": b }),
    }
}

pub struct SlotReader {
    c: Connection,
}

/// Rows of a query as JSON objects, columns in SELECT order.
pub fn rows(c: &Connection, sql: &str, p: impl rusqlite::Params) -> rusqlite::Result<Vec<Map<String, Value>>> {
    let mut st = c.prepare(sql)?;
    let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();

    st.query_map(p, |r| {
        let mut m = Map::new();
        for (i, n) in names.iter().enumerate() {
            m.insert(n.clone(), sql_json(r.get::<_, Sql>(i)?));
        }
        Ok(m)
    })?
    .collect()
}

const APP_STATUS: fn(&str) -> &str = |s| match s {
    "running" => "pending",
    "unconfirmed" => "failed",
    other => other,
};

/// Ids that Teams gave: an item saved without one has its place as id (x0, x1...).
pub const HAS_TEAMS_ID: &str = "id NOT GLOB 'x[0-9]*'";
pub const CALL_LOG_SIZE: i64 = 50;
pub const OPEN_REASONS: [&str; 4] = ["signed-out", "not-listed", "not-shown", "unreadable"];

impl SlotReader {
    pub fn open(file: &std::path::Path) -> Res<Self> {
        if !file.exists() {
            return Err(Error::NotReady);
        }
        let c = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        c.busy_timeout(std::time::Duration::from_millis(8000))?;
        Ok(SlotReader { c })
    }

    pub fn for_slot(slot: i64) -> Res<Self> {
        Self::open(&slot_db_path(slot))
    }

    /// None while the agent has not created the database yet.
    pub fn try_for_slot(slot: i64) -> Res<Option<Self>> {
        match Self::for_slot(slot) {
            Ok(r) => Ok(Some(r)),
            Err(Error::NotReady) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn conn(&self) -> &Connection {
        &self.c
    }

    /// A query that fails (a table the agent has not made yet) reads as no rows.
    pub fn all(&self, sql: &str, p: impl rusqlite::Params) -> Vec<Map<String, Value>> {
        rows(&self.c, sql, p).unwrap_or_default()
    }

    pub fn raw(&self, k: &str) -> Option<String> {
        self.c.query_row("SELECT v FROM state WHERE k=?", [k], |r| r.get::<_, Option<String>>(0)).ok().flatten()
    }

    pub fn put(&self, k: &str, v: &Value) -> Res<()> {
        self.c.execute("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)", params![k, crate::js::stringify(v)])?;
        Ok(())
    }

    /// The JSON of a state row; missing, empty or broken gives the fallback.
    pub fn state(&self, k: &str, fallback: Value) -> Value {
        match self.raw(k) {
            Some(v) if !v.is_empty() => serde_json::from_str(&v).unwrap_or(fallback),
            _ => fallback,
        }
    }

    pub fn active_chat(&self) -> String {
        self.raw(key::ACTIVE_CHAT).unwrap_or_default()
    }

    pub fn relay_link(&self) -> RelayLink {
        RelayLink::parse(self.raw(key::RELAY).as_deref())
    }

    pub fn chats(&self) -> Vec<Value> {
        self.all("SELECT name, preview, tm, unread, mention, muted, av, presence, kind FROM chats ORDER BY pos", [])
            .into_iter()
            .map(|mut c| {
                for (k, d) in [("muted", json!(0)), ("av", json!("")), ("presence", json!("")), ("kind", json!(""))] {
                    if c.get(k).is_none_or(Value::is_null) {
                        c.insert(k.into(), d);
                    }
                }
                Value::Object(c)
            })
            .collect()
    }

    pub fn messages(&self, chat: &str) -> Vec<Value> {
        self.all("SELECT mid, author, text, mine, reacts, extra FROM chat_messages WHERE chat=? ORDER BY idx", [chat])
            .into_iter()
            .map(|mut m| {
                let extra = m.shift_remove("extra");
                for (k, v) in parse_extra(extra.as_ref().and_then(Value::as_str)) {
                    m.insert(k, v);
                }
                Value::Object(m)
            })
            .collect()
    }

    fn activity_ts(&self) -> f64 {
        let v = crate::js::number(Some(&self.state(key::ACTIVITY_TS, json!(0))));
        if v.is_nan() { 0.0 } else { v }
    }

    pub fn activity(&self) -> Value {
        let items = self.all("SELECT id, kind, actor, title, emoji, preview, tm, chat, channel, unread, av FROM activity ORDER BY pos", []);
        json!({ "ts": num(self.activity_ts()), "items": items })
    }

    pub fn feed(&self) -> Vec<Map<String, Value>> {
        self.all("SELECT id, ts, title, body FROM messages ORDER BY id DESC LIMIT 150", [])
    }

    pub fn identity(&self) -> Identity {
        Identity::parse(self.raw(key::ME).as_deref())
    }

    /// People of a chat as the agent read them last; a stored value that is not an object fails like Members.parse.
    pub fn members(&self, chat: &str) -> Res<Value> {
        let v = self.state(&state::members_key(chat), json!({}));
        let v = if v.is_null() { json!({}) } else { v };
        state::members(&v).ok_or_else(|| Error::Internal("Members.parse: expected object".into()))
    }

    pub fn pending_command(&self, kind: &str, arg1: &str) -> i64 {
        self.all("SELECT id FROM commands WHERE type=? AND arg1=? AND status IN ('pending','running') ORDER BY id DESC LIMIT 1", params![kind, arg1])
            .first()
            .and_then(|r| r.get("id").and_then(Value::as_i64))
            .unwrap_or(0)
    }

    pub fn unread_count(&self) -> i64 {
        self.all("SELECT COUNT(*) AS c FROM chats WHERE unread=1 AND COALESCE(muted,0)=0 AND name NOT LIKE '%(You)%'", [])
            .first()
            .and_then(|r| r.get("c").and_then(Value::as_i64))
            .unwrap_or(0)
    }

    fn ids(&self, sql: &str) -> Option<Vec<Value>> {
        if self.activity_ts() == 0.0 {
            return None;
        }
        Some(self.all(sql, []).into_iter().map(|r| r.get("id").cloned().unwrap_or(Value::Null)).collect())
    }

    pub fn unread_activity(&self) -> Option<Vec<Value>> {
        self.ids(&format!("SELECT id FROM activity WHERE unread=1 AND {HAS_TEAMS_ID} ORDER BY pos"))
    }

    pub fn missed_calls(&self) -> Option<Vec<Value>> {
        self.ids(&format!("SELECT id FROM activity WHERE kind='call' AND {HAS_TEAMS_ID} ORDER BY pos"))
    }

    pub fn activity_ids(&self) -> Option<Vec<Value>> {
        self.ids("SELECT id FROM activity ORDER BY pos")
    }

    pub fn call(&self) -> Option<CallState> {
        CallState::parse(self.raw(key::CALL).as_deref())
    }

    pub fn in_call(&self) -> Option<InCall> {
        InCall::parse(self.raw(key::IN_CALL).as_deref())
    }

    pub fn call_log(&self) -> Vec<Map<String, Value>> {
        self.all(&format!("SELECT caller, since, seconds FROM calls ORDER BY since DESC, id DESC LIMIT {CALL_LOG_SIZE}"), [])
    }

    pub fn health(&self, added: i64) -> Map<String, Value> {
        state::health_of(&self.state(key::HEALTH, json!({})), added, now_ms() as f64 / 1000.0)
    }

    /// pending, done or failed, with the result row of the command; None for an unknown command.
    pub fn command_status(&self, id: i64) -> Option<Value> {
        let r = self.all("SELECT status FROM commands WHERE id=?", [id]);
        let status = r.first()?.get("status").map(|v| crate::js::string(Some(v)))?;
        Some(json!({ "status": APP_STATUS(&status), "result": self.state(&state::cmd_result_key(id), Value::Null) }))
    }

    /// The last open of the chat as the app follows it.
    pub fn open_of(&self, chat: &str) -> Option<Value> {
        let r = self.all("SELECT id, status FROM commands WHERE type='open' AND arg1=? ORDER BY id DESC LIMIT 1", [chat]);
        let row = r.first()?;
        let id = row.get("id").cloned().unwrap_or(Value::Null);
        let status = APP_STATUS(&crate::js::string(row.get("status"))).to_string();
        if status != "failed" {
            return Some(json!({ "id": id, "status": status }));
        }
        let why = self.state(&state::cmd_result_key(id.as_i64().unwrap_or(0)), Value::Null);
        match why.get("reason").and_then(Value::as_str).filter(|r| OPEN_REASONS.contains(r)) {
            Some(reason) if why.is_object() => Some(json!({ "id": id, "status": status, "reason": reason })),
            _ => Some(json!({ "id": id, "status": status })),
        }
    }

    pub fn enqueue(&self, kind: &str, arg1: &str, arg2: &str) -> Res<i64> {
        self.c.execute("INSERT INTO commands(ts, type, arg1, arg2) VALUES(?,?,?,?)", params![now_s(), kind, arg1, arg2])?;
        Ok(self.c.last_insert_rowid())
    }

    /// The same command still waiting for the agent, else a new one.
    pub fn enqueue_once(&self, kind: &str, arg1: &str, arg2: &str) -> Res<i64> {
        let tx = self.c.unchecked_transaction()?;
        let waiting =
            self.all("SELECT id FROM commands WHERE status='pending' AND type=? AND arg1=? AND arg2=? ORDER BY id LIMIT 1", params![kind, arg1, arg2]);
        let id = match waiting.first().and_then(|r| r.get("id").and_then(Value::as_i64)) {
            Some(id) => id,
            None => self.enqueue(kind, arg1, arg2)?,
        };
        tx.commit()?;
        Ok(id)
    }

    pub fn forget_last_check(&self) -> Res<()> {
        self.c.execute("DELETE FROM state WHERE k=?", [key::CHECK_SEEN])?;
        Ok(())
    }

    pub fn mark_viewing(&self, chat: &str) -> Res<()> {
        self.put(key::VIEWING, &json!({ "chat": chat, "ts": now_s() }))
    }

    pub fn leave_viewing(&self, chat: &str) -> Res<()> {
        let tx = self.c.unchecked_transaction()?;
        if state::Viewing::parse(self.raw(key::VIEWING).as_deref()).chat == chat {
            self.put(key::VIEWING, &json!({ "chat": "", "ts": now_s() }))?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mark_desktop(&self) -> Res<()> {
        self.put(key::DESKTOP, &json!({ "ts": now_s() }))
    }
}

/// The tables of data/N/messages.db as the agent makes them (src/shared/slot-db/schema.ts): missing tables and columns
/// added, nothing dropped.
pub fn ensure_slot_schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER, source TEXT, title TEXT, body TEXT);
         CREATE TABLE IF NOT EXISTS chats(name TEXT PRIMARY KEY, preview TEXT, pos INTEGER, ts INTEGER, tm TEXT, unread INTEGER, mention INTEGER);
         CREATE TABLE IF NOT EXISTS chat_messages(chat TEXT, idx INTEGER, mid TEXT, author TEXT, text TEXT, mine INTEGER, reacts TEXT, extra TEXT);
         CREATE TABLE IF NOT EXISTS commands(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER, type TEXT, arg1 TEXT, arg2 TEXT, status TEXT DEFAULT 'pending');
         CREATE TABLE IF NOT EXISTS state(k TEXT PRIMARY KEY, v TEXT);
         CREATE TABLE IF NOT EXISTS readby(mid TEXT PRIMARY KEY, chat TEXT, label TEXT, names TEXT, ts INTEGER);
         CREATE TABLE IF NOT EXISTS activity(id TEXT PRIMARY KEY, pos INTEGER, kind TEXT, actor TEXT, title TEXT, emoji TEXT, preview TEXT, tm TEXT, chat TEXT, unread INTEGER, ts INTEGER);
         CREATE TABLE IF NOT EXISTS calls(id INTEGER PRIMARY KEY AUTOINCREMENT, since INTEGER, caller TEXT, seconds INTEGER);",
    )?;
    for (table, column, decl) in [
        ("chats", "muted", "INTEGER DEFAULT 0"),
        ("chats", "av", "TEXT"),
        ("chat_messages", "extra", "TEXT"),
        ("activity", "channel", "INTEGER"),
        ("activity", "av", "TEXT"),
        ("commands", "key", "TEXT"),
        ("chats", "presence", "TEXT"),
        ("chats", "kind", "TEXT"),
    ] {
        let cols: Vec<String> = c.prepare(&format!("PRAGMA table_info({table})"))?.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<_, _>>()?;
        if !cols.iter().any(|x| x == column) {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
        }
    }
    c.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS commands_key ON commands(key) WHERE key IS NOT NULL")
}

/// SlotStore.open: the database of a slot, made when missing (folders too), WAL, its schema ensured.
pub fn open_store(file: &std::path::Path) -> Res<Connection> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let c = Connection::open(file)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.busy_timeout(std::time::Duration::from_millis(8000))?;
    ensure_slot_schema(&c)?;
    Ok(c)
}

/// The columns of the rows the agent saves and a relay sends, in this order.
pub fn insert_row(table: &str) -> String {
    let cols: &[&str] = match table {
        "chats" => &["name", "preview", "pos", "ts", "tm", "unread", "mention", "muted", "av", "presence", "kind"],
        "chat_messages" => &["chat", "idx", "mid", "author", "text", "mine", "reacts", "extra"],
        "activity" => &["id", "pos", "kind", "actor", "title", "emoji", "preview", "tm", "chat", "channel", "unread", "ts", "av"],
        "calls" => &["since", "caller", "seconds"],
        _ => &["mid", "chat", "label", "names", "ts"],
    };
    format!("INSERT OR REPLACE INTO {table}({}) VALUES({})", cols.join(", "), cols.iter().map(|_| "?").collect::<Vec<_>>().join(","))
}

/// The JSON extra of a row of chat_messages, as its object spread: nothing when it is not an object.
pub fn parse_extra(v: Option<&str>) -> Map<String, Value> {
    let Some(v) = v.filter(|v| !v.is_empty()) else { return Map::new() };
    match serde_json::from_str::<Value>(v) {
        Ok(Value::Object(m)) => m,
        Ok(Value::Array(a)) => a.into_iter().enumerate().map(|(i, x)| (i.to_string(), x)).collect(),
        _ => Map::new(),
    }
}

/// withSlot: 503 while the agent has not created the database.
pub fn with_slot<T>(slot: i64, f: impl FnOnce(&SlotReader) -> Res<T>) -> Res<T> {
    f(&SlotReader::for_slot(slot)?)
}

/// withSlotOr: the fallback while the agent has not created the database.
pub fn with_slot_or<T>(slot: i64, f: impl FnOnce(&SlotReader) -> Res<T>, fallback: T) -> Res<T> {
    match SlotReader::try_for_slot(slot)? {
        Some(r) => f(&r),
        None => Ok(fallback),
    }
}
