//! The devices of the owner of a slot in app.db, as the agents read them (src/agent/store/app-store.ts): the push
//! subscriptions of the owner, where its notifications go, and the subscriptions the push service reports as gone.

use super::notify::{account_label, error_text, warn};
use super::{Audience, PushTarget, is_phone};
use crate::state::Identity;
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension};
use std::cell::Cell;

/// The devices of the owner of a slot in app.db, as the agents read them (AppStore).
pub struct AppStore {
    pub slot: i64,
    /// read at most every 5 s: when, and what
    sides: Cell<(i64, Audience)>,
}

impl AppStore {
    pub fn new(slot: i64) -> Self {
        AppStore { slot, sides: Cell::new((0, Audience::ALL)) }
    }

    fn use_db<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>, fallback: T) -> T {
        match crate::appdb::with(f) {
            Ok(v) => v,
            Err(e) => {
                warn("appdb", &error_text(&e.to_string()), &[]);
                fallback
            }
        }
    }

    /// Devices of the user who owns this slot; a phone of the Android app while the session that registered it runs.
    pub fn targets(&self) -> Vec<PushTarget> {
        let now = crate::auth::rows::iso_ms(crate::web::now_ms());
        self.use_db(|c| targets_in(c, self.slot, &now), vec![])
    }

    pub fn owner_has_many_accounts(&self) -> bool {
        self.use_db(|c| owner_has_many_in(c, self.slot), false)
    }

    /// Where the notifications of this account go; on when the row or its columns cannot be read.
    pub fn audience(&self) -> Audience {
        let now = crate::web::now_ms();
        let (at, value) = self.sides.get();
        if now - at <= 5_000 {
            return value;
        }
        let value = self.use_db(|c| audience_in(c, self.slot), None).unwrap_or(Audience::ALL);
        self.sides.set((now, value));
        value
    }

    pub fn remove(&self, endpoint: &str) {
        self.use_db(|c| c.execute("DELETE FROM push_subscriptions WHERE endpoint=?", [endpoint]).map(|_| ()), ());
    }

    /// acc: the notification opens the app on this account.
    pub fn account(&self, me: &Identity) -> (i64, String) {
        (self.slot, account_label(self.owner_has_many_accounts(), me, self.slot))
    }
}

/// Whether the better-auth session a phone registered with still runs; no sessions table is none.
fn session_runs(c: &Connection, sub: &str, now_iso: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(sub) else { return false };
    let Some(session) = v.get("fcm").and_then(|f| f.get("session")).and_then(serde_json::Value::as_str) else { return false };
    c.query_row(r#"SELECT 1 FROM "session" WHERE id=? AND expiresAt > ?"#, [session, now_iso], |_| Ok(())).optional().ok().flatten().is_some()
}

pub(crate) fn targets_in(c: &Connection, slot: i64, now_iso: &str) -> rusqlite::Result<Vec<PushTarget>> {
    let rows: Vec<PushTarget> = c
        .prepare("SELECT p.endpoint, p.sub FROM push_subscriptions p JOIN teams_accounts a ON a.owner_id=p.user_id WHERE a.slot=?")?
        .query_map([slot], |r| Ok(PushTarget { endpoint: r.get(0)?, sub: r.get(1)? }))?
        .collect::<Result<_, _>>()?;
    Ok(rows.into_iter().filter(|t| !is_phone(&t.endpoint) || session_runs(c, &t.sub, now_iso)).collect())
}

pub(crate) fn owner_has_many_in(c: &Connection, slot: i64) -> rusqlite::Result<bool> {
    let n: i64 = c.query_row("SELECT COUNT(*) FROM teams_accounts WHERE owner_id=(SELECT owner_id FROM teams_accounts WHERE slot=?)", [slot], |r| r.get(0))?;
    Ok(n > 1)
}

/// Number(v) > 0 of JavaScript for a column value.
fn positive(v: &Sql) -> bool {
    match v {
        Sql::Null | Sql::Blob(_) => false,
        Sql::Integer(i) => *i > 0,
        Sql::Real(f) => *f > 0.0,
        Sql::Text(t) => crate::js::number(Some(&serde_json::Value::String(t.clone()))) > 0.0,
    }
}

/// The two choices of the row: a column that is not there yet counts as on; None without the row.
pub(crate) fn audience_in(c: &Connection, slot: i64) -> rusqlite::Result<Option<Audience>> {
    let mut st = c.prepare("SELECT * FROM teams_accounts WHERE slot=?")?;
    let names: Vec<String> = st.column_names().into_iter().map(String::from).collect();
    st.query_row([slot], |r| {
        let side = |name: &str| -> rusqlite::Result<bool> {
            match names.iter().position(|n| n == name) {
                None => Ok(true),
                Some(i) => Ok(positive(&r.get::<_, Sql>(i)?)),
            }
        };
        Ok(Audience { mobile: side("notify_mobile")?, desktop: side("notify_desktop")? })
    })
    .optional()
}

#[cfg(test)]
pub(crate) fn seed_app_db(devices: &[(&str, &str, &str)]) -> Connection {
    let c = Connection::open_in_memory().expect("db");
    crate::appdb::migrate(&c).expect("schema");
    c.execute_batch("INSERT INTO teams_accounts(slot, owner_id, added) VALUES(1, 'u1', 0), (2, 'u2', 0)").expect("accounts");
    for (endpoint, user, sub) in devices {
        c.execute("INSERT INTO push_subscriptions VALUES(?, ?, ?, 0)", [endpoint, user, sub]).expect("device");
    }
    c.execute_batch(r#"CREATE TABLE "session"(id TEXT PRIMARY KEY, expiresAt date); INSERT INTO "session" VALUES('s-live', '2099-01-01T00:00:00.000Z'), ('s-over', '2020-01-01T00:00:00.000Z');"#)
        .expect("sessions");
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn devices_of_the_owner_and_phones_with_a_running_session() {
        let web = |e: &str| json!({ "endpoint": e, "keys": { "p256dh": "k", "auth": "a" } }).to_string();
        let phone = |t: &str, s: &str| json!({ "fcm": { "token": t, "key": "k", "name": "Pixel", "session": s } }).to_string();
        let c = seed_app_db(&[
            ("https://push/u1-phone", "u1", &web("https://push/u1-phone")),
            ("https://push/u2-phone", "u2", &web("https://push/u2-phone")),
            ("fcm:live", "u1", &phone("live", "s-live")),
            ("fcm:over", "u1", &phone("over", "s-over")),
            ("fcm:gone", "u1", &phone("gone", "s-gone")),
        ]);
        let now = crate::auth::rows::iso_ms(1_790_000_000_000);
        let endpoints: Vec<String> = targets_in(&c, 1, &now).expect("targets").into_iter().map(|t| t.endpoint).collect();
        assert_eq!(endpoints, ["https://push/u1-phone", "fcm:live"]);
        assert!(!owner_has_many_in(&c, 1).expect("count"));
        c.execute_batch("INSERT INTO teams_accounts(slot, owner_id, added) VALUES(3, 'u1', 0)").expect("insert");
        assert!(owner_has_many_in(&c, 1).expect("count"));
        // no sessions table: no phone
        c.execute_batch(r#"DROP TABLE "session""#).expect("drop");
        assert_eq!(targets_in(&c, 1, &now).expect("targets").len(), 1);
    }

    #[test]
    fn audience_of_the_row() {
        let c = seed_app_db(&[]);
        c.execute_batch("UPDATE teams_accounts SET notify_mobile=0 WHERE slot=2").expect("update");
        assert_eq!(audience_in(&c, 1).expect("read"), Some(Audience { mobile: true, desktop: true }));
        assert_eq!(audience_in(&c, 2).expect("read"), Some(Audience { mobile: false, desktop: true }));
        assert_eq!(audience_in(&c, 9).expect("read"), None);
        let old = Connection::open_in_memory().expect("db");
        old.execute_batch("CREATE TABLE teams_accounts(slot INTEGER PRIMARY KEY, owner_id TEXT NOT NULL, added INTEGER NOT NULL); INSERT INTO teams_accounts VALUES(1, 'u1', 0);")
            .expect("old");
        assert_eq!(audience_in(&old, 1).expect("read"), Some(Audience::ALL));
    }
}
