//! data/status.db: the status history of the status challenge (src/shared/status-db.ts), apart from data/N/ so it
//! outlives an account removed and added again. The light agents write it, the web app reads and deletes; whichever
//! opens it first creates it.

use super::stats::Stretch;
use crate::web::Res;
use rusqlite::{Connection, OptionalExtension};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub fn open_status_db(file: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let c = Connection::open(file)?;
    c.busy_timeout(std::time::Duration::from_millis(8000))?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS presence_log(
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           name TEXT NOT NULL,
           mri TEXT NOT NULL,
           since INTEGER NOT NULL,
           until INTEGER NOT NULL,
           status TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS presence_log_name ON presence_log(name, since);",
    )?;
    Ok(c)
}

pub fn status_db_file() -> PathBuf {
    crate::config::data_dir().join("status.db")
}

thread_local! {
    static OPENED: RefCell<HashMap<PathBuf, Rc<Connection>>> = RefCell::new(HashMap::new());
}

/// data/status.db of the data folder, opened once per file.
pub fn with_status_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Res<T> {
    let file = status_db_file();
    let known = OPENED.with(|o| o.borrow().get(&file).cloned());
    let c = match known {
        Some(c) => c,
        None => {
            let c = Rc::new(open_status_db(&file)?);
            OPENED.with(|o| o.borrow_mut().insert(file, c.clone()));
            c
        }
    };
    Ok(f(&c)?)
}

/// The stretches of a person that overlap [from, to), oldest first.
pub fn stretches_of(c: &Connection, name: &str, from: i64, to: i64) -> rusqlite::Result<Vec<Stretch>> {
    c.prepare("SELECT since, until, status FROM presence_log WHERE name=? AND since < ? AND until >= ? ORDER BY since, id")?
        .query_map(rusqlite::params![name, to, from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect()
}

/// The newest stretch of a person: since, until, status.
pub fn newest_of(c: &Connection, name: &str) -> rusqlite::Result<Option<Stretch>> {
    c.query_row("SELECT since, until, status FROM presence_log WHERE name=? ORDER BY since DESC, id DESC LIMIT 1", [name], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .optional()
}

pub fn first_since(c: &Connection, name: &str) -> rusqlite::Result<Option<i64>> {
    c.query_row("SELECT MIN(since) FROM presence_log WHERE name=?", [name], |r| r.get(0))
}

pub fn delete_statuses_of(c: &Connection, name: &str) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM presence_log WHERE name=?", [name])
}
