//! The public status pages of the status challenge (src/lib/status-pages.ts, app/status-challenge-spec.md): their rows
//! in app.db, their history in data/status.db, what /api/status/<slug> and the page answer, and the admin card
//! (/api/admin/status-pages).

pub mod db;
pub mod stats;

use crate::appdb;
use crate::auth::require_admin;
use crate::route;
use crate::server::Route;
use crate::state::Identity;
use crate::web::{Error, Req, Res, err, json, json_with, now_s, text};
use db::{delete_statuses_of, first_since, newest_of, stretches_of, with_status_db};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value, json};
use stats::{STATUS_GAP, day_of, day_start, day_stats, next_day};
use std::collections::{HashMap, HashSet};

pub fn routes() -> Vec<Route> {
    vec![
        route!("GET", "/api/admin/status-pages", admin_get),
        route!("POST", "/api/admin/status-pages", admin_post),
        route!("PATCH", "/api/admin/status-pages", admin_patch),
        route!("DELETE", "/api/admin/status-pages", admin_delete),
        route!("GET", "/api/status/[slug]", public_get),
    ]
}

pub fn slug_ok(slug: &str) -> bool {
    regex::Regex::new("^[a-z0-9-]{2,32}$").expect("fixed").is_match(slug)
}

/// The data of a page is kept this long after its challenge ends, then the page goes too.
pub const KEPT_AFTER_END: i64 = 7 * 86400;

/// Paths the server and Caddy own, which the exported UI does not list: the routes of this crate, `status` (the page
/// a slug is served with) and `desktop` (caddy/Caddyfile). `static` is a folder of public.
const OWNED_PATHS: [&str; 16] = [
    "api",
    "files",
    "media",
    "mcp",
    "healthz",
    "desktop",
    "login",
    "settings",
    "admin",
    "remote",
    "consent",
    "status",
    "_next",
    ".well-known",
    "communication",
    "static",
];

/// Every top-level entry of the exported UI, with and without its extension (login.html takes /login), and the paths
/// the server owns. The TypeScript read src/app and public at build time; the export holds the same names.
pub fn reserved_slugs() -> HashSet<String> {
    let mut out: HashSet<String> = OWNED_PATHS.iter().map(|s| s.to_string()).collect();
    if let Ok(entries) = std::fs::read_dir(crate::config::ui_dir()) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some((stem, _)) = name.rsplit_once('.')
                && !stem.is_empty()
            {
                out.insert(stem.to_string());
            }
            out.insert(name);
        }
    }
    out
}

/// Why a slug cannot be a new page, None when it can.
pub fn slug_problem(slug: &str) -> Res<Option<(u16, String)>> {
    if !slug_ok(slug) {
        return Ok(Some((400, "Path: 2 to 32 lowercase letters, digits or -".into())));
    }
    if reserved_slugs().contains(slug) {
        return Ok(Some((409, format!("/{slug} is a page of TeamsRelay"))));
    }
    if page_of(slug)?.is_some() {
        return Ok(Some((409, format!("/{slug} is already a status page"))));
    }
    Ok(None)
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatusPage {
    pub slug: String,
    pub slot: i64,
    pub name: String,
    pub added: i64,
    pub ends: i64,
}

impl StatusPage {
    fn from(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(StatusPage { slug: r.get(0)?, slot: r.get(1)?, name: r.get(2)?, added: r.get(3)?, ends: r.get(4)? })
    }

    fn json(&self) -> Map<String, Value> {
        let v = json!({ "slug": self.slug, "slot": self.slot, "name": self.name, "added": self.added, "ends": self.ends });
        v.as_object().cloned().unwrap_or_default()
    }
}

pub fn list_pages() -> Res<Vec<StatusPage>> {
    appdb::with(|c| c.prepare("SELECT slug, slot, name, added, ends FROM status_pages ORDER BY added, slug")?.query_map([], StatusPage::from)?.collect())
}

fn page_in(c: &Connection, slug: &str) -> rusqlite::Result<Option<StatusPage>> {
    c.query_row("SELECT slug, slot, name, added, ends FROM status_pages WHERE slug=?", [slug], StatusPage::from).optional()
}

pub fn page_of(slug: &str) -> Res<Option<StatusPage>> {
    appdb::with(|c| page_in(c, slug))
}

pub fn add_page(p: &StatusPage) -> Res<()> {
    appdb::with(|c| {
        c.execute("INSERT INTO status_pages(slug, slot, name, added, ends) VALUES(?,?,?,?,?)", rusqlite::params![p.slug, p.slot, p.name, p.added, p.ends])
    })?;
    Ok(())
}

pub fn set_page_ends(slug: &str, ends: i64) -> Res<bool> {
    Ok(appdb::with(|c| c.execute("UPDATE status_pages SET ends=? WHERE slug=?", rusqlite::params![ends, slug]))? > 0)
}

/// The page and, unless another page tracks the same person, its history.
pub fn delete_page(slug: &str) -> Res<bool> {
    let Some(page) = page_of(slug)? else { return Ok(false) };
    let others = appdb::with(|c| {
        c.execute("DELETE FROM status_pages WHERE slug=?", [slug])?;
        c.query_row("SELECT 1 FROM status_pages WHERE name=?", [&page.name], |_| Ok(())).optional()
    })?;
    if others.is_none() {
        with_status_db(|c| delete_statuses_of(c, &page.name))?;
    }
    Ok(true)
}

/// Pages whose challenge ended more than 7 days ago, with their data.
pub fn purge_ended_pages(now: i64) -> Res<()> {
    let ended: Vec<String> = appdb::with(|c| {
        c.prepare("SELECT slug FROM status_pages WHERE ends > 0 AND ends + ? <= ?")?.query_map([KEPT_AFTER_END, now], |r| r.get(0))?.collect()
    })?;
    for slug in ended {
        delete_page(&slug)?;
    }
    Ok(())
}

/// The zone of the days and clock times of the challenge: TZ of the process as Node reads it, else the zone of the
/// system, else UTC.
pub fn server_zone() -> String {
    let tz = std::env::var("TZ").unwrap_or_default();
    if !tz.is_empty() {
        return tz;
    }
    jiff::tz::TimeZone::try_system().ok().and_then(|z| z.iana_name().map(String::from)).unwrap_or_else(|| "UTC".into())
}

/// At most this many days in one answer, the newest.
const MAX_DAYS: usize = 400;

/// statusAnswer: what /api/status/<slug> and the page get, None when there is no such page.
pub fn status_answer(slug: &str, now: i64) -> Res<Option<Value>> {
    purge_ended_pages(now)?;
    let Some(page) = page_of(slug)? else { return Ok(None) };
    let zone = server_zone();
    let tz = stats::zone(&zone);
    let ended = page.ends > 0 && page.ends <= now;
    // an ended challenge shows its days up to the end
    let stop = if ended { page.ends } else { now };
    let last = day_of(if ended { stop - 1 } else { stop }, &tz);
    let (newest, first) = with_status_db(|c| Ok((newest_of(c, &page.name)?, first_since(c, &page.name)?)))?;
    let mut days = vec![];
    let mut day = day_of(page.added.min(first.unwrap_or(page.added)), &tz);
    while day <= last {
        let next = next_day(&day);
        days.push(day);
        day = next;
    }
    let shown = &days[days.len().saturating_sub(MAX_DAYS)..];
    let from = day_start(shown.first().unwrap_or(&last), &tz);
    let stretches = with_status_db(|c| stretches_of(c, &page.name, from, stop))?;
    let now_status = match &newest {
        Some((since, until, status)) => json!({ "status": status, "since": since, "fresh": !ended && now - until <= STATUS_GAP }),
        None => Value::Null,
    };
    // the timeline draws today and yesterday; older days carry the totals only
    let days: Vec<Value> = shown.iter().enumerate().map(|(i, d)| day_stats(&stretches, d, &tz, stop, STATUS_GAP, i + 2 >= shown.len())).collect();
    Ok(Some(json!({
        "name": page.name,
        "now": now_status,
        "days": days,
        "ends": page.ends,
        "updated": newest.as_ref().map_or(0, |n| n.1),
        "zone": zone,
    })))
}

/// The answer of a public status page for /[slug], None when the slug is no page.
pub fn page_answer(slug: &str) -> Res<Option<Value>> {
    if !slug_ok(slug) {
        return Ok(None);
    }
    status_answer(slug, now_s())
}

/// The status of the person of a status page, public: no session, no key.
async fn public_get(req: Req) -> Res {
    let Some(answer) = page_answer(&req.param("slug"))? else { return Err(err(404, "Page not found")) };
    let mut r = json_with(200, &answer);
    r.headers_mut().insert(hyper::header::CACHE_CONTROL, hyper::header::HeaderValue::from_static("public, max-age=30"));
    Ok(r)
}

/// The identity of an account and the people its agent found (StatusNames: a name with a value not a string is none).
fn identity_of(slot: i64) -> Res<Option<(Identity, HashMap<String, String>)>> {
    crate::slotdb::with_slot_or(
        slot,
        |r| {
            let names = match r.state(crate::state::key::STATUS_NAMES, json!({})) {
                Value::Object(m) if m.values().all(Value::is_string) => m.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_string())).collect(),
                _ => HashMap::new(),
            };
            Ok(Some((r.identity(), names)))
        },
        None,
    )
}

/// The organization the page collects in, and why it does not collect ("" when it does).
fn why(p: &StatusPage, now: i64) -> Res<(String, String)> {
    let row = appdb::slot_row(p.slot)?;
    let slot = if row.is_some() { identity_of(p.slot)? } else { None };
    let tenant = slot.as_ref().map(|s| s.0.tenant.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("account {}", p.slot));
    let reason = |r: &str| Ok((tenant.clone(), r.to_string()));
    if p.ends != 0 && p.ends <= now {
        return reason("challenge ended");
    }
    let Some(row) = row else { return reason("account removed") };
    if row.light == 0 {
        return reason("account not light");
    }
    if row.stopped != 0 {
        return reason("account stopped");
    }
    if crate::accounts::account_summary(&row)?.get("teams") == Some(&json!("login")) {
        return reason("account signed out");
    }
    match slot.as_ref().and_then(|s| s.1.get(&p.name)) {
        None => reason("waiting for the agent"),
        Some(mri) if mri.is_empty() => reason(&format!("not found in {tenant}")),
        Some(_) => reason(""),
    }
}

/// The pages with whether their account collects and why not, the last status seen; the accounts of the administrator
/// a page can collect from.
pub fn admin_list(admin_id: &str, now: i64) -> Res<Value> {
    purge_ended_pages(now)?;
    let mut pages = vec![];
    for p in list_pages()? {
        let (tenant, reason) = why(&p, now)?;
        let last = with_status_db(|c| newest_of(c, &p.name))?;
        let mut m = p.json();
        m.insert("tenant".into(), json!(tenant));
        m.insert("collecting".into(), json!(reason.is_empty()));
        m.insert("reason".into(), json!(reason));
        m.insert("last".into(), last.map_or(Value::Null, |(since, until, status)| json!({ "since": since, "until": until, "status": status })));
        pages.push(Value::Object(m));
    }
    let mut accounts = vec![];
    for s in appdb::slots_of(admin_id)? {
        let me = identity_of(s.slot)?.map(|x| x.0);
        let label = match me {
            Some(me) if !me.name.is_empty() => {
                if me.tenant.is_empty() {
                    me.name
                } else {
                    format!("{} ({})", me.name, me.tenant)
                }
            }
            _ => format!("Account {}", s.slot),
        };
        accounts.push(json!({ "slot": s.slot, "label": label, "light": s.light != 0 }));
    }
    Ok(json!({ "pages": pages, "accounts": accounts, "zone": server_zone() }))
}

pub fn admin_add(admin_id: &str, b: &Map<String, Value>, now: i64) -> Res<()> {
    let slug = text(b.get("slug"), "slug", 64)?;
    if let Some((status, reason)) = slug_problem(&slug)? {
        return Err(err(status, reason));
    }
    let slot = crate::js::number(b.get("slot"));
    if !appdb::slots_of(admin_id)?.iter().any(|s| s.slot as f64 == slot && s.light != 0) {
        return Err(err(400, format!("Account {} is not a light account of yours", crate::js::string(b.get("slot")))));
    }
    let name = text(b.get("name"), "name", 100)?.trim().to_string();
    if name.is_empty() {
        return Err(err(400, "Missing name"));
    }
    add_page(&StatusPage { slug, slot: slot as i64, name, added: now, ends: 0 })
}

/// ends: the last day of the challenge (YYYY-MM-DD, it ends at the midnight after), "" for none.
pub fn admin_set_end(b: &Map<String, Value>) -> Res<i64> {
    let slug = text(b.get("slug"), "slug", 64)?;
    let day = text(b.get("ends"), "ends", 10)?;
    if !day.is_empty() && !regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("fixed").is_match(&day) {
        return Err(err(400, "End date: YYYY-MM-DD"));
    }
    let ends = if day.is_empty() { 0 } else { day_start(&next_day(&day), &stats::zone(&server_zone())) };
    if page_of(&slug)?.is_none() || !set_page_ends(&slug, ends)? {
        return Err(err(404, "Page not found"));
    }
    Ok(ends)
}

async fn admin_get(req: Req) -> Res {
    let admin = require_admin(&req)?;
    json(admin_list(&admin.id, now_s())?)
}

async fn admin_post(req: Req) -> Res {
    let admin = require_admin(&req)?;
    admin_add(&admin.id, &req.json()?, now_s())?;
    json(json!({ "ok": true }))
}

async fn admin_patch(req: Req) -> Res {
    require_admin(&req)?;
    let ends = admin_set_end(&req.json()?)?;
    json(json!({ "ok": true, "ends": ends }))
}

async fn admin_delete(req: Req) -> Res {
    require_admin(&req)?;
    if !delete_page(&text(req.json()?.get("slug"), "slug", 64)?)? {
        return Err(Error::Http(404, "Page not found".into(), vec![]));
    }
    json(json!({ "ok": true }))
}

#[cfg(test)]
mod tests;
