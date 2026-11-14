//! data/app.db as a light agent reads it (src/agent/store/app-store.ts): the devices of the owner of its slot, where
//! its notifications go, whether its calls ring, the status pages. It never creates the file: until the web app has,
//! there is nobody to notify. Opened for each read, as the TypeScript did.

use super::{error_text, warn};
use crate::push::app_store::{audience_in, owner_has_many_in, targets_in};
use crate::push::{Audience, PushDevices, PushTarget, account_label};
use crate::state::Identity;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::cell::Cell;
use std::path::PathBuf;

/// A status page as the agents read it: slug, slot, name, ends (Unix s, 0 none).
#[derive(Clone, Debug, PartialEq)]
pub struct StatusPageRow {
    pub slug: String,
    pub slot: i64,
    pub name: String,
    pub ends: i64,
}

pub struct AppDb {
    file: PathBuf,
    slot: i64,
    /// read at most every 5 s: when, and what
    sides: Cell<(i64, Audience)>,
    calls: Cell<(i64, bool)>,
    checked: Cell<(i64, bool)>,
    light: Cell<(i64, bool)>,
}

impl AppDb {
    pub fn new(file: PathBuf, slot: i64) -> Self {
        AppDb {
            file,
            slot,
            sides: Cell::new((0, Audience::ALL)),
            calls: Cell::new((0, true)),
            checked: Cell::new((i64::MIN / 2, false)),
            light: Cell::new((i64::MIN / 2, false)),
        }
    }

    fn use_db<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>, fallback: T) -> T {
        if !self.file.exists() {
            return fallback;
        }
        let run = || -> rusqlite::Result<T> {
            let c = Connection::open_with_flags(&self.file, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            c.busy_timeout(std::time::Duration::from_millis(8000))?;
            f(&c)
        };
        match run() {
            Ok(v) => v,
            Err(e) => {
                warn("appdb", &error_text(&e.to_string()), &[]);
                fallback
            }
        }
    }

    /// Calls of this account (teams_accounts.calls, on unless switched off in Settings). Read at most every 5 s.
    pub fn calls_on(&self) -> bool {
        let now = super::now_ms();
        let (at, value) = self.calls.get();
        if now - at <= 5_000 {
            return value;
        }
        let value = self.use_db(
            |c| {
                let mut st = c.prepare("SELECT * FROM teams_accounts WHERE slot=?")?;
                let at = st.column_names().iter().position(|n| *n == "calls");
                st.query_row([self.slot], |r| at.map(|i| r.get::<_, rusqlite::types::Value>(i)).transpose()).optional()
            },
            None,
        );
        let on = match value.flatten() {
            None => true,
            Some(rusqlite::types::Value::Integer(i)) => i > 0,
            Some(rusqlite::types::Value::Real(f)) => f > 0.0,
            Some(rusqlite::types::Value::Text(t)) => t.trim().parse::<f64>().is_ok_and(|n| n > 0.0),
            Some(_) => false,
        };
        self.calls.set((now, on));
        on
    }

    /// Number(row?.[column]) of this account's row, 0 when there is none (NaN counts as 0 too: never > 0).
    fn column(&self, column: &str) -> f64 {
        let value = self.use_db(
            |c| {
                let mut st = c.prepare("SELECT * FROM teams_accounts WHERE slot=?")?;
                let at = st.column_names().iter().position(|n| *n == column);
                st.query_row([self.slot], |r| at.map(|i| r.get::<_, rusqlite::types::Value>(i)).transpose()).optional()
            },
            None,
        );
        match value.flatten() {
            Some(rusqlite::types::Value::Integer(i)) => i as f64,
            Some(rusqlite::types::Value::Real(f)) => f,
            Some(rusqlite::types::Value::Text(t)) => t.trim().parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// The web app starts this account only to check it every few hours (check_every > 0). Read at most every minute.
    pub fn checked_only(&self) -> bool {
        let now = super::now_ms();
        let (at, value) = self.checked.get();
        if now - at <= 60_000 {
            return value;
        }
        let v = self.column("check_every") > 0.0;
        self.checked.set((now, v));
        v
    }

    /// A light account whose browser runs to sign it in again (light and refreshing). Read at most every 15 s.
    pub fn light_mode(&self) -> bool {
        let now = super::now_ms();
        let (at, value) = self.light.get();
        if now - at <= 15_000 {
            return value;
        }
        let v = self.column("light") > 0.0 && self.column("refreshing") > 0.0;
        self.light.set((now, v));
        v
    }

    /// Every status page of the server, none while the web app has not created the table; None when app.db cannot be
    /// read.
    pub fn status_pages(&self) -> Option<Vec<StatusPageRow>> {
        if !self.file.exists() {
            return None;
        }
        self.use_db(
            |c| {
                let table = c.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='status_pages'", [], |_| Ok(())).optional()?;
                if table.is_none() {
                    return Ok(Some(vec![]));
                }
                let rows = c
                    .prepare("SELECT slug, slot, name, ends FROM status_pages")?
                    .query_map([], |r| Ok(StatusPageRow { slug: r.get(0)?, slot: r.get(1)?, name: r.get(2)?, ends: r.get(3)? }))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(Some(rows))
            },
            None,
        )
    }
}

impl PushDevices for AppDb {
    /// Devices of the user who owns this slot; a phone of the Android app while the session that registered it runs.
    fn targets(&self) -> Vec<PushTarget> {
        let now = crate::auth::rows::iso_ms(super::now_ms());
        self.use_db(|c| targets_in(c, self.slot, &now), vec![])
    }

    fn remove(&self, endpoint: &str) {
        self.use_db(|c| c.execute("DELETE FROM push_subscriptions WHERE endpoint=?", [endpoint]).map(drop), ());
    }

    fn account(&self, me: &Identity) -> Option<(i64, String)> {
        let many = self.use_db(|c| owner_has_many_in(c, self.slot), false);
        Some((self.slot, account_label(many, me, self.slot)))
    }

    /// On when the row or its columns cannot be read. Read at most every 5 s.
    fn audience(&self) -> Option<Audience> {
        let now = super::now_ms();
        let (at, value) = self.sides.get();
        if now - at <= 5_000 {
            return Some(value);
        }
        let value = self.use_db(|c| audience_in(c, self.slot), None).unwrap_or(Audience::ALL);
        self.sides.set((now, value));
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::temp_dir;

    #[test]
    fn reads_the_row_of_its_slot_and_never_creates_the_file() {
        let dir = temp_dir();
        let file = dir.join("app.db");
        let a = AppDb::new(file.clone(), 1);
        assert!(a.calls_on() && a.status_pages().is_none() && a.targets().is_empty());
        assert!(!file.exists());
        let c = crate::appdb::open(file.to_str().expect("path")).expect("db");
        crate::appdb::migrate(&c).expect("schema");
        c.execute_batch("INSERT INTO teams_accounts(slot, owner_id, added, calls) VALUES(1, 'u1', 0, 0); INSERT INTO status_pages(slug, slot, name, added, ends) VALUES('kamila', 1, 'Camilla', 0, 0)").expect("rows");
        let a = AppDb::new(file, 1);
        assert!(!a.calls_on());
        assert_eq!(a.status_pages(), Some(vec![StatusPageRow { slug: "kamila".into(), slot: 1, name: "Camilla".into(), ends: 0 }]));
        assert_eq!(a.audience(), Some(Audience::ALL));
    }
}
