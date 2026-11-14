//! data/app.db: the Better Auth tables, slot ownership and push subscriptions (src/lib/appdb.ts). The agents read
//! teams_accounts and push_subscriptions: the column names never change. One connection for the process, as
//! better-sqlite3 had.

use crate::web::{Error, Res, err, now_s};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 2;

static DB: Mutex<Option<(String, Connection)>> = Mutex::new(None);

pub fn open(file: &str) -> rusqlite::Result<Connection> {
    if let Some(dir) = std::path::Path::new(file).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let c = Connection::open(file)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.busy_timeout(std::time::Duration::from_millis(8000))?;
    Ok(c)
}

/// The app database of APP_DB, opened once (again when APP_DB changes, as in the tests).
pub fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Res<T> {
    let file = crate::config::app_db();
    let mut g = DB.lock().unwrap_or_else(|p| p.into_inner());
    if g.as_ref().is_none_or(|(f, _)| *f != file) {
        *g = Some((file.clone(), open(&file)?));
    }
    let (_, c) = g.as_ref().expect("opened");
    Ok(f(c)?)
}

/// The same, for code that returns route errors of its own.
pub fn with_res<T>(f: impl FnOnce(&Connection) -> Res<T>) -> Res<T> {
    let file = crate::config::app_db();
    let mut g = DB.lock().unwrap_or_else(|p| p.into_inner());
    if g.as_ref().is_none_or(|(f, _)| *f != file) {
        *g = Some((file.clone(), open(&file)?));
    }
    let (_, c) = g.as_ref().expect("opened");
    f(c)
}

/// A row of teams_accounts as the web app reads it: relay says whether a relay token is set, never the token.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub slot: i64,
    pub owner_id: String,
    pub added: i64,
    pub stopped: i64,
    pub started: i64,
    pub check_every: i64,
    pub check_due: i64,
    pub checked: i64,
    pub check_result: String,
    pub checking: i64,
    pub relay: i64,
    pub browser_off: i64,
    pub light: i64,
    pub refreshing: i64,
    pub refreshed: i64,
    pub refresh_result: String,
    pub calls: i64,
    pub notify_mobile: i64,
    pub notify_desktop: i64,
    pub shared_account: String,
    pub shared_region: String,
}

const SLOT_COLUMNS: &str = "slot, owner_id, added, stopped, started, check_every, check_due, checked, check_result, checking, (relay_token <> '') AS relay, browser_off, light, refreshing, refreshed, refresh_result, calls, notify_mobile, notify_desktop, shared_account, shared_region";

impl Slot {
    fn from(r: &Row) -> rusqlite::Result<Self> {
        Ok(Slot {
            slot: r.get(0)?,
            owner_id: r.get(1)?,
            added: r.get(2)?,
            stopped: r.get(3)?,
            started: r.get(4)?,
            check_every: r.get(5)?,
            check_due: r.get(6)?,
            checked: r.get(7)?,
            check_result: r.get(8)?,
            checking: r.get(9)?,
            relay: r.get(10)?,
            browser_off: r.get(11)?,
            light: r.get(12)?,
            refreshing: r.get(13)?,
            refreshed: r.get(14)?,
            refresh_result: r.get(15)?,
            calls: r.get(16)?,
            notify_mobile: r.get(17)?,
            notify_desktop: r.get(18)?,
            shared_account: r.get(19)?,
            shared_region: r.get(20)?,
        })
    }

    /// The row as better-sqlite3 returned it, in column order.
    pub fn json(&self) -> Value {
        json!({
            "slot": self.slot, "owner_id": self.owner_id, "added": self.added, "stopped": self.stopped, "started": self.started,
            "check_every": self.check_every, "check_due": self.check_due, "checked": self.checked, "check_result": self.check_result,
            "checking": self.checking, "relay": self.relay, "browser_off": self.browser_off, "light": self.light,
            "refreshing": self.refreshing, "refreshed": self.refreshed, "refresh_result": self.refresh_result, "calls": self.calls,
            "notify_mobile": self.notify_mobile, "notify_desktop": self.notify_desktop, "shared_account": self.shared_account,
            "shared_region": self.shared_region,
        })
    }
}

pub fn migrate(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS teams_accounts(slot INTEGER PRIMARY KEY, owner_id TEXT NOT NULL, added INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS teams_accounts_owner ON teams_accounts(owner_id);
         CREATE TABLE IF NOT EXISTS push_subscriptions(endpoint TEXT PRIMARY KEY, user_id TEXT NOT NULL, sub TEXT NOT NULL, created INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS push_subscriptions_user ON push_subscriptions(user_id);",
    )?;
    let columns: Vec<String> = c.prepare("SELECT name FROM pragma_table_info('teams_accounts')")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for (column, decl) in [
        ("stopped", "INTEGER NOT NULL DEFAULT 0"),
        ("started", "INTEGER NOT NULL DEFAULT 0"),
        ("check_every", "INTEGER NOT NULL DEFAULT 0"),
        ("check_due", "INTEGER NOT NULL DEFAULT 0"),
        ("checked", "INTEGER NOT NULL DEFAULT 0"),
        ("check_result", "TEXT NOT NULL DEFAULT ''"),
        ("checking", "INTEGER NOT NULL DEFAULT 0"),
        ("relay_token", "TEXT NOT NULL DEFAULT ''"),
        ("browser_off", "INTEGER NOT NULL DEFAULT 0"),
        ("light", "INTEGER NOT NULL DEFAULT 0"),
        ("refreshing", "INTEGER NOT NULL DEFAULT 0"),
        ("refreshed", "INTEGER NOT NULL DEFAULT 0"),
        ("refresh_result", "TEXT NOT NULL DEFAULT ''"),
        ("calls", "INTEGER NOT NULL DEFAULT 1"),
        ("notify_mobile", "INTEGER NOT NULL DEFAULT 1"),
        ("notify_desktop", "INTEGER NOT NULL DEFAULT 1"),
        ("shared_account", "TEXT NOT NULL DEFAULT ''"),
        ("shared_region", "TEXT NOT NULL DEFAULT ''"),
    ] {
        if !columns.iter().any(|x| x == column) {
            c.execute_batch(&format!("ALTER TABLE teams_accounts ADD COLUMN {column} {decl}"))?;
        }
    }
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS browser_actions(id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, user_id TEXT NOT NULL, client_id TEXT NOT NULL,
           slot INTEGER NOT NULL, tool TEXT NOT NULL, host TEXT NOT NULL DEFAULT '', outcome TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS browser_actions_slot ON browser_actions(slot, id);
         CREATE TABLE IF NOT EXISTS status_pages(slug TEXT PRIMARY KEY, slot INTEGER NOT NULL, name TEXT NOT NULL, added INTEGER NOT NULL,
           ends INTEGER NOT NULL DEFAULT 0);",
    )?;
    Ok(())
}

fn has_table(c: &Connection, name: &str) -> rusqlite::Result<bool> {
    Ok(c.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", [name], |_| Ok(())).optional()?.is_some())
}

/// The previous single-owner release: its slots and devices go to the first administrator, once.
pub fn adopt_legacy_data(c: &Connection, user_id: &str) -> rusqlite::Result<(usize, usize)> {
    let tx = c.unchecked_transaction()?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok((0, 0));
    }
    let now = now_s();
    let mut slots = 0;
    let mut devices = 0;
    if has_table(&tx, "accounts")? {
        slots =
            tx.execute("INSERT OR IGNORE INTO teams_accounts(slot, owner_id, added) SELECT slot, ?, COALESCE(added, ?) FROM accounts", params![user_id, now])?;
    }
    if has_table(&tx, "push_subs")? {
        devices = tx.execute(
            "INSERT OR IGNORE INTO push_subscriptions(endpoint, user_id, sub, created) SELECT endpoint, ?, sub, ? FROM push_subs WHERE endpoint <> ''",
            params![user_id, now],
        )?;
    }
    tx.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    tx.commit()?;
    Ok((slots, devices))
}

pub fn list_slots() -> Res<Vec<Slot>> {
    with(|c| c.prepare(&format!("SELECT {SLOT_COLUMNS} FROM teams_accounts ORDER BY slot"))?.query_map([], Slot::from)?.collect())
}

pub fn slots_of(user_id: &str) -> Res<Vec<Slot>> {
    with(|c| c.prepare(&format!("SELECT {SLOT_COLUMNS} FROM teams_accounts WHERE owner_id=? ORDER BY slot"))?.query_map([user_id], Slot::from)?.collect())
}

pub fn slot_row(slot: i64) -> Res<Option<Slot>> {
    with(|c| c.query_row(&format!("SELECT {SLOT_COLUMNS} FROM teams_accounts WHERE slot=?"), [slot], Slot::from).optional())
}

fn run(sql: &str, p: impl rusqlite::Params) -> Res<usize> {
    with(|c| c.execute(sql, p))
}

pub fn set_check_every(slot: i64, every: i64, now: i64) -> Res<()> {
    run("UPDATE teams_accounts SET check_every=?, check_due=? WHERE slot=?", params![every, if every != 0 { now + every } else { 0 }, slot]).map(drop)
}

pub fn ask_check(slot: i64) -> Res<()> {
    run("UPDATE teams_accounts SET check_due=0 WHERE slot=?", [slot]).map(drop)
}

pub fn begin_check(slot: i64, now: i64) -> Res<()> {
    run("UPDATE teams_accounts SET checking=? WHERE slot=?", [now, slot]).map(drop)
}

/// A check that ended; None: cut short, nothing recorded.
pub fn end_check(slot: i64, now: i64, result: Option<&str>, asked: bool) -> Res<()> {
    match result {
        None => run("UPDATE teams_accounts SET checking=0 WHERE slot=?", [slot]),
        Some(result) => run(
            "UPDATE teams_accounts SET checking=0, checked=?, check_result=?, check_due=CASE WHEN check_due = 0 AND NOT ? THEN 0 WHEN check_every > 0 THEN ? + check_every ELSE 0 END WHERE slot=?",
            params![now, result, i64::from(asked), now, slot],
        ),
    }
    .map(drop)
}

pub fn next_check(now: i64) -> Res<Option<Slot>> {
    with(|c| {
        c.query_row(
            &format!("SELECT {SLOT_COLUMNS} FROM teams_accounts WHERE check_every > 0 AND stopped = 0 AND check_due <= ? ORDER BY check_due, slot LIMIT 1"),
            [now],
            Slot::from,
        )
        .optional()
    })
}

pub fn set_light(slot: i64, light: bool) -> Res<()> {
    if light {
        run("UPDATE teams_accounts SET light=1, check_every=0, check_due=0 WHERE slot=?", [slot]).map(drop)
    } else {
        run("UPDATE teams_accounts SET light=0, refreshing=0 WHERE slot=?", [slot]).map(drop)
    }
}

pub fn set_calls(slot: i64, on: bool) -> Res<()> {
    run("UPDATE teams_accounts SET calls=? WHERE slot=?", params![i64::from(on), slot]).map(drop)
}

pub fn set_notify(slot: i64, mobile: Option<bool>, desktop: Option<bool>) -> Res<()> {
    if let Some(m) = mobile {
        run("UPDATE teams_accounts SET notify_mobile=? WHERE slot=?", params![i64::from(m), slot])?;
    }
    if let Some(d) = desktop {
        run("UPDATE teams_accounts SET notify_desktop=? WHERE slot=?", params![i64::from(d), slot])?;
    }
    Ok(())
}

pub fn begin_refresh(slot: i64, now: i64) -> Res<()> {
    run("UPDATE teams_accounts SET refreshing=? WHERE slot=?", [now, slot]).map(drop)
}

pub const REFRESH_ASKED: &str = "asked";

pub fn ask_refresh(slot: i64) -> Res<()> {
    run("UPDATE teams_accounts SET refresh_result=? WHERE slot=?", params![REFRESH_ASKED, slot]).map(drop)
}

pub fn end_refresh(slot: i64, now: i64, result: Option<&str>) -> Res<()> {
    match result {
        None => run(
            "UPDATE teams_accounts SET refreshing=0, refresh_result=CASE WHEN refresh_result=? THEN '' ELSE refresh_result END WHERE slot=?",
            params![REFRESH_ASKED, slot],
        ),
        Some(r) => run("UPDATE teams_accounts SET refreshing=0, refreshed=?, refresh_result=? WHERE slot=?", params![now, r, slot]),
    }
    .map(drop)
}

pub fn is_slot_stopped(slot: i64) -> Res<bool> {
    with(|c| c.query_row("SELECT stopped FROM teams_accounts WHERE slot=?", [slot], |r| r.get::<_, i64>(0)).optional()).map(|v| v.unwrap_or(0) != 0)
}

pub fn set_slot_stopped(slot: i64, stopped: bool) -> Res<()> {
    if stopped {
        run("UPDATE teams_accounts SET stopped=1 WHERE slot=?", [slot]).map(drop)
    } else {
        run("UPDATE teams_accounts SET stopped=0, started=? WHERE slot=?", [now_s(), slot]).map(drop)
    }
}

pub fn slot_owner(slot: i64) -> Res<Option<String>> {
    with(|c| c.query_row("SELECT owner_id FROM teams_accounts WHERE slot=?", [slot], |r| r.get(0)).optional())
}

/// The first free slot for the user, in one IMMEDIATE transaction; 409 past the cap of accounts per user.
pub fn claim_slot(user_id: &str, per_user: Option<i64>) -> Res<i64> {
    with_res(|c| {
        c.execute_batch("BEGIN IMMEDIATE")?;
        let r = (|| -> Res<i64> {
            let mine: i64 = c.query_row("SELECT COUNT(*) FROM teams_accounts WHERE owner_id=?", [user_id], |r| r.get(0))?;
            if let Some(cap) = per_user
                && mine >= cap
            {
                return Err(err(409, format!("You can have at most {cap} accounts")));
            }
            let taken: Vec<i64> = c.prepare("SELECT slot FROM teams_accounts")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            let mut n = 1;
            while taken.contains(&n) {
                n += 1;
            }
            c.execute("INSERT INTO teams_accounts(slot, owner_id, added) VALUES(?,?,?)", params![n, user_id, now_s()])?;
            Ok(n)
        })();
        match r {
            Ok(n) => {
                c.execute_batch("COMMIT")?;
                Ok(n)
            }
            Err(e) => {
                let _ = c.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    })
}

pub fn release_slot(slot: i64) -> Res<()> {
    run("DELETE FROM teams_accounts WHERE slot=?", [slot])?;
    run("DELETE FROM browser_actions WHERE slot=?", [slot]).map(drop)
}

pub fn bind_shared(slot: i64, account: &str, region: &str) -> Res<()> {
    run(
        "UPDATE teams_accounts SET shared_account=?, shared_region=?, light=1, check_every=0, check_due=0 WHERE slot=?",
        params![account.to_lowercase(), region, slot],
    )
    .map(drop)
}

/// The shared account of a slot and its region ("emea" when unset); None for a slot with its own sign-in.
pub fn shared_binding(slot: i64) -> Res<Option<(String, String)>> {
    let r: Option<(String, String)> =
        with(|c| c.query_row("SELECT shared_account, shared_region FROM teams_accounts WHERE slot=?", [slot], |r| Ok((r.get(0)?, r.get(1)?))).optional())?;
    Ok(r.filter(|(a, _)| !a.is_empty()).map(|(a, r)| (a, if r.is_empty() { "emea".into() } else { r })))
}

pub fn set_browser_off(slot: i64, off: bool) -> Res<()> {
    run("UPDATE teams_accounts SET browser_off=? WHERE slot=?", params![i64::from(off), slot]).map(drop)
}

const BROWSER_ACTIONS_KEPT: i64 = 500;

pub fn log_browser_action(user_id: &str, client_id: &str, slot: i64, tool: &str, host: &str, outcome: &str, now_ms: i64) -> Res<()> {
    run(
        "INSERT INTO browser_actions(ts, user_id, client_id, slot, tool, host, outcome) VALUES(?,?,?,?,?,?,?)",
        params![now_ms, user_id, client_id, slot, tool, host, outcome],
    )?;
    run(
        "DELETE FROM browser_actions WHERE slot=? AND id <= (SELECT id FROM browser_actions WHERE slot=? ORDER BY id DESC LIMIT 1 OFFSET ?)",
        params![slot, slot, BROWSER_ACTIONS_KEPT],
    )
    .map(drop)
}

pub fn browser_actions_of(slot: i64, limit: i64) -> Res<Vec<Value>> {
    with(|c| {
        c.prepare("SELECT id, ts, user_id, client_id, slot, tool, host, outcome FROM browser_actions WHERE slot=? ORDER BY id DESC LIMIT ?")?
            .query_map(params![slot, limit], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?, "ts": r.get::<_, i64>(1)?, "user_id": r.get::<_, String>(2)?, "client_id": r.get::<_, String>(3)?,
                    "slot": r.get::<_, i64>(4)?, "tool": r.get::<_, String>(5)?, "host": r.get::<_, String>(6)?, "outcome": r.get::<_, String>(7)?,
                }))
            })?
            .collect()
    })
}

pub fn set_relay_token(slot: i64, digest: &str) -> Res<()> {
    run("UPDATE teams_accounts SET relay_token=? WHERE slot=?", params![digest, slot]).map(drop)
}

/// The account whose relay holds the token of this digest: slot, added, owner.
pub fn relay_account(digest: &str) -> Res<Option<(i64, i64, String)>> {
    if digest.is_empty() {
        return Ok(None);
    }
    with(|c| {
        c.query_row("SELECT slot, added, owner_id FROM teams_accounts WHERE relay_token=?", [digest], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()
    })
}

pub fn save_push_subscription(user_id: &str, endpoint: &str, json: &str) -> Res<()> {
    run("INSERT OR REPLACE INTO push_subscriptions(endpoint, user_id, sub, created) VALUES(?,?,?,?)", params![endpoint, user_id, json, now_s()]).map(drop)
}

/// A phone of the Android app; the same phone of the same user keeps its key, another user gets a new one.
pub fn save_fcm_device(user_id: &str, token: &str, name: &str, session: &str) -> Res<String> {
    let endpoint = format!("fcm:{token}");
    let row: Option<(String, String)> =
        with(|c| c.query_row("SELECT user_id, sub FROM push_subscriptions WHERE endpoint=?", [&endpoint], |r| Ok((r.get(0)?, r.get(1)?))).optional())?;
    let mut key = String::new();
    if let Some((owner, sub)) = row.filter(|(u, _)| u == user_id) {
        let _ = owner;
        if let Ok(v) = serde_json::from_str::<Value>(&sub) {
            key = match v.pointer("/fcm/key") {
                None | Some(Value::Null) => String::new(),
                k => crate::js::string(k),
            };
        }
    }
    if key.is_empty() {
        key = crate::js::base64_url(&crate::auth::crypto::random(32));
    }
    let sub = json!({ "fcm": { "token": token, "key": key, "name": name, "session": session } });
    run(
        "INSERT OR REPLACE INTO push_subscriptions(endpoint, user_id, sub, created) VALUES(?,?,?,?)",
        params![endpoint, user_id, crate::js::stringify(&sub), now_s()],
    )?;
    Ok(key)
}

pub fn forget_fcm_devices_of_session(session: &str) -> Res<()> {
    run("DELETE FROM push_subscriptions WHERE endpoint LIKE 'fcm:%' AND json_extract(sub, '$.fcm.session')=?", [session]).map(drop)
}

pub fn forget_fcm_device(token: &str) -> Res<()> {
    run("DELETE FROM push_subscriptions WHERE endpoint=?", [format!("fcm:{token}")]).map(drop)
}

pub fn count_push_subscriptions(user_id: &str) -> Res<i64> {
    with(|c| c.query_row("SELECT COUNT(*) FROM push_subscriptions WHERE user_id=?", [user_id], |r| r.get(0)))
}

pub fn delete_push_subscriptions_of(user_id: &str) -> Res<()> {
    run("DELETE FROM push_subscriptions WHERE user_id=?", [user_id]).map(drop)
}

/// For code paths that only log a database failure.
pub fn log_err(what: &str, e: Error) {
    eprintln!("{what}: {e}");
}
