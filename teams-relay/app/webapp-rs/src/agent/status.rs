//! The status of the people the status pages of this account track, once per poll of the light agent, into
//! data/status.db (src/agent/light/status.ts, app/status-challenge-spec.md, Collector).

use super::api::{Api, Person};
use super::devices::StatusPageRow;
use super::store::Store;
use super::{Fail, info, text_of, warn};
use crate::status::db::{delete_statuses_except, open_status_db, record_statuses};
use indexmap::IndexMap;
use rusqlite::Connection;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

const PAGES_EVERY_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
/// the data of a page is kept this long after its challenge ends
const KEPT_AFTER_END: i64 = 7 * 86400;

/// presence.availability as Teams draws it next to the name (statusOf).
pub fn status_of(availability: &str) -> &'static str {
    match availability {
        "Available" | "AvailableIdle" => "available",
        "Busy" | "BusyIdle" => "busy",
        "DoNotDisturb" => "dnd",
        "BeRightBack" => "brb",
        "Away" => "away",
        "Offline" => "offline",
        _ => "unknown",
    }
}

/// What kind of failure a presence call had, logged once an hour each.
fn failure_kind(e: &Fail) -> String {
    match e {
        Fail::SignedOut(_) => "signed out".into(),
        Fail::Status { status, .. } => format!("HTTP {status}"),
        _ => "Error".into(),
    }
}

/// The people the agent knows and its chats, to find a person of a page by name.
pub struct Known<'a> {
    pub people: &'a IndexMap<String, Person>,
    pub ids: &'a IndexMap<String, String>,
    pub members: &'a IndexMap<String, Vec<String>>,
}

/// The mri of a person the account knows by this name: of the people named so far, else the other member of the 1:1
/// chat of that name; "" for none.
fn mri_of(k: &Known, name: &str) -> String {
    let want = super::js_trim(name).to_lowercase();
    for (mri, p) in k.people {
        if super::js_trim(&p.display_name).to_lowercase() == want {
            return mri.clone();
        }
    }
    for (chat, id) in k.ids {
        let others = k.members.get(id);
        if super::js_trim(chat).to_lowercase() == want
            && let Some(o) = others.filter(|o| o.len() == 1)
        {
            return o[0].clone();
        }
    }
    String::new()
}

pub struct StatusCollector {
    pages: Box<dyn Fn() -> Option<Vec<StatusPageRow>>>,
    file: PathBuf,
    slot: i64,
    gap: i64,
    state: RefCell<State>,
}

struct State {
    pages_at: i64,
    rows: Option<Vec<StatusPageRow>>,
    db: Option<std::rc::Rc<Connection>>,
    logged: HashMap<String, i64>,
    retained_at: i64,
}

impl StatusCollector {
    pub fn new(pages: Box<dyn Fn() -> Option<Vec<StatusPageRow>>>, file: PathBuf, slot: i64, gap: i64) -> Self {
        StatusCollector {
            pages,
            file,
            slot,
            gap,
            state: RefCell::new(State { pages_at: i64::MIN, rows: None, db: None, logged: HashMap::new(), retained_at: i64::MIN }),
        }
    }

    fn once(&self, key: &str, message: &str, now_ms: i64) {
        let mut s = self.state.borrow_mut();
        if s.logged.get(key).is_some_and(|at| now_ms - at < HOUR_MS) {
            return;
        }
        s.logged.insert(key.into(), now_ms);
        drop(s);
        warn("status", message, &[]);
    }

    fn open(&self) -> Result<std::rc::Rc<Connection>, String> {
        if let Some(db) = self.state.borrow().db.clone() {
            return Ok(db);
        }
        let db = std::rc::Rc::new(open_status_db(&self.file).map_err(|e| e.to_string())?);
        self.state.borrow_mut().db = Some(db.clone());
        Ok(db)
    }

    /// Rows of names no page tracks any more, and of pages ended more than 7 days ago; nothing while app.db is
    /// unreadable.
    fn retain(&self, rows: &Option<Vec<StatusPageRow>>, now: i64, now_ms: i64) -> Result<(), String> {
        let Some(rows) = rows else { return Ok(()) };
        if now_ms.saturating_sub(self.state.borrow().retained_at) < HOUR_MS {
            return Ok(());
        }
        self.state.borrow_mut().retained_at = now_ms;
        let mut keep: Vec<String> = vec![];
        for p in rows.iter().filter(|p| p.ends == 0 || p.ends + KEPT_AFTER_END > now) {
            if !keep.contains(&p.name) {
                keep.push(p.name.clone());
            }
        }
        let db = self.open()?;
        let gone = delete_statuses_except(&db, &keep).map_err(|e| e.to_string())?;
        if gone > 0 {
            info("status", "old status rows deleted", &[("rows", gone.into())]);
        }
        Ok(())
    }

    pub async fn poll(&self, known: Known<'_>, api: &Api, store: &Store, now_ms: i64) {
        if let Err(e) = self.collect(known, api, store, now_ms).await {
            self.once("db:Error", &format!("status not saved: {}", super::error_text(&e)), now_ms);
        }
    }

    async fn collect(&self, known: Known<'_>, api: &Api, store: &Store, now_ms: i64) -> Result<(), String> {
        if now_ms.saturating_sub(self.state.borrow().pages_at) >= PAGES_EVERY_MS {
            let rows = (self.pages)();
            let mut s = self.state.borrow_mut();
            s.pages_at = now_ms;
            s.rows = rows;
        }
        let now = now_ms.div_euclid(1000);
        let rows = self.state.borrow().rows.clone();
        self.retain(&rows, now, now_ms)?;
        let mut names: Vec<String> = vec![];
        for p in rows.iter().flatten().filter(|p| p.slot == self.slot && (p.ends == 0 || p.ends > now)) {
            if !names.contains(&p.name) {
                names.push(p.name.clone());
            }
        }
        if names.is_empty() {
            return Ok(());
        }
        let found: Vec<(String, String)> = names.iter().map(|n| (n.clone(), mri_of(&known, n))).collect();
        let json: Map<String, Value> = found.iter().map(|(n, m)| (n.clone(), Value::String(m.clone()))).collect();
        let saved =
            crate::state::object(Some(&store.get_state(crate::state::key::STATUS_NAMES))).filter(|m| m.values().all(Value::is_string)).unwrap_or_default();
        let text = crate::js::stringify(&Value::Object(json));
        if crate::js::stringify(&Value::Object(saved)) != text {
            store.set_state(crate::state::key::STATUS_NAMES, &text);
        }
        for (name, mri) in &found {
            if mri.is_empty() {
                self.once(&format!("name:{name}"), &format!("status page person not found in this account: {name}"), now_ms);
            }
        }
        let wanted: Vec<&(String, String)> = found.iter().filter(|(_, m)| !m.is_empty()).collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let mut mris: Vec<String> = vec![];
        for (_, m) in &wanted {
            if !mris.contains(m) {
                mris.push(m.clone());
            }
        }
        let presence = match api.presence(&mris).await {
            Ok(p) => p,
            Err(e) => {
                self.once(&format!("presence:{}", failure_kind(&e)), &format!("status not read: {}", text_of(&e)), now_ms);
                return Ok(());
            }
        };
        let seen: Vec<(String, String, String)> =
            wanted.iter().filter_map(|(n, m)| presence.get(m).map(|p| (n.clone(), m.clone(), status_of(&p.availability).to_string()))).collect();
        if !seen.is_empty() {
            let db = self.open()?;
            record_statuses(&db, &seen, now, self.gap).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
