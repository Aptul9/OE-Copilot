//! The shared Microsoft accounts of the OE calendar agent (src/lib/shared-accounts.ts, src/lib/shared-signin.ts,
//! src/app/api/shared-accounts/route.ts): one sign-in, in the calendar, for mail, calendar and Teams. The agent keeps the
//! list and, per account, which of the three are on; this server follows it. An account with Teams on has a slot of the
//! owner running on it without a browser; with Teams off that slot is stopped, its data kept. The owner is the user of
//! SHARED_ACCOUNTS_OWNER, by default the administrator of ADMIN_EMAIL.

mod sync;

pub use sync::{SyncResult, sync_shared_accounts};

use crate::appdb::{self, Slot};
use crate::auth::require_user;
use crate::route;
use crate::server::Route;
use crate::web::{Error, Req, Res, err, json};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value, json};
use std::rc::Rc;

pub const CAPABILITIES: [&str; 3] = ["mail", "calendar", "chat"];
/// a mailbox that never chose: mail and calendar, as before the shared accounts
const LEGACY: [&str; 2] = ["mail", "calendar"];

pub const NO_CALLS_SWITCH: &str = "Calls are switched only for Teams accounts without a browser";
pub const NOT_SHARED: &str = "not in the shared accounts of the calendar";
const NO_SHARED: &str = "No shared Microsoft accounts on this server";
pub const BROWSER_PRESERVED: &str = "Existing browser sign-in preserved. Shared Teams cannot take over this slot.";

/// What the sync and the routes reach: the directory of the agent and the supervisor of the browsers container
/// (src/lib/control.ts); the tests replace it with a recording fake. The directory answers None for a 404.
pub trait Broker {
    fn list(&self) -> LocalBoxFuture<'_, Res<Option<Value>>>;
    fn set(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>>;
    fn report(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>>;
    fn connect(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>>;
    fn start(&self, n: i64) -> LocalBoxFuture<'_, Res<()>>;
    fn light(&self, n: i64) -> LocalBoxFuture<'_, Res<()>>;
    fn stop(&self, n: i64) -> LocalBoxFuture<'_, Res<()>>;
    fn signin(&self, n: i64) -> LocalBoxFuture<'_, Res<()>>;
    /// wipeSlot: browser profile and agent data of the slot
    fn wipe(&self, n: i64) -> LocalBoxFuture<'_, Res<()>>;
}

/// The socket of the supervisor.
pub struct Live;

impl Broker for Live {
    fn list(&self) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        Box::pin(crate::control::directory_list())
    }
    fn set(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        Box::pin(async move { crate::control::directory_set(&body).await })
    }
    fn report(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        Box::pin(async move { crate::control::directory_report(&body).await })
    }
    fn connect(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        Box::pin(async move { crate::control::directory_connect(&body).await })
    }
    fn start(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        Box::pin(crate::control::start(n))
    }
    fn light(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        Box::pin(crate::control::light(n))
    }
    fn stop(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        Box::pin(crate::slots::slot_down(n))
    }
    fn signin(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        Box::pin(crate::control::signin(n))
    }
    fn wipe(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        Box::pin(crate::slots::wipe_slot(n))
    }
}

/// The Microsoft address a slot is signed in as, "" before its agent read it.
pub type IdentityOf = Rc<dyn Fn(&Slot) -> String>;

/// What a pass works with (SyncDeps); app.db and the data folder are those of APP_DB.
pub struct Deps {
    pub broker: Rc<dyn Broker>,
    pub per_user: Option<i64>,
    pub owner: Option<String>,
    pub identity: IdentityOf,
}

impl Deps {
    pub fn live(owner: Option<String>) -> Deps {
        Deps { broker: Rc::new(Live), per_user: crate::config::accounts_per_user(), owner, identity: Rc::new(email_of) }
    }
}

/// The address of accountSummary, lower case.
fn email_of(s: &Slot) -> String {
    crate::accounts::account_summary(s).ok().and_then(|v| v["email"].as_str().map(str::to_lowercase)).unwrap_or_default()
}

/// e.message of a failure.
fn message(e: &Error) -> String {
    crate::slots::detail(e)
}

/// A failure with only its text, as `new Error(text)`.
fn fail(text: impl Into<String>) -> Error {
    Error::Internal(text.into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedAccount {
    pub account: String,
    pub capabilities: Vec<String>,
    /// false: the mailbox never chose, it counts as mail and calendar
    pub shared: bool,
    /// "error": the sign-in needs to be made again in the calendar
    pub status: String,
    pub message: String,
}

impl SharedAccount {
    pub fn json(&self) -> Map<String, Value> {
        let v = json!({ "account": self.account, "capabilities": self.capabilities, "shared": self.shared, "status": self.status, "message": self.message });
        v.as_object().cloned().unwrap_or_default()
    }

    fn has(&self, c: &str) -> bool {
        self.capabilities.iter().any(|x| x == c)
    }
}

/// The list of the agent (GET /api/auth/shared), checked: an answer of another shape is an error, never an empty list
/// that would stop every slot.
pub fn parse_directory(raw: &Value) -> Res<Vec<SharedAccount>> {
    let accounts = match (raw.get("ok"), raw.get("accounts")) {
        (Some(Value::Bool(true)), Some(Value::Array(a))) => a,
        _ => {
            let why = raw.get("errore").and_then(Value::as_str).unwrap_or("unexpected answer");
            return Err(err(502, format!("Shared accounts not read: {why}")));
        }
    };
    let mut out = vec![];
    for a in accounts {
        let Some(account) = a.get("account").and_then(Value::as_str).filter(|x| x.contains('@')) else { continue };
        let caps: Option<Vec<String>> = a
            .get("shared")
            .and_then(|s| s.get("capabilities"))
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(Value::as_str).filter(|c| CAPABILITIES.contains(c)).map(String::from).collect());
        out.push(SharedAccount {
            account: account.to_lowercase(),
            shared: caps.is_some(),
            capabilities: caps.unwrap_or_else(|| LEGACY.map(String::from).to_vec()),
            status: if a.get("status") == Some(&json!("errore")) { "error" } else { "ok" }.into(),
            message: a.get("message").and_then(Value::as_str).unwrap_or("").into(),
        });
    }
    Ok(out)
}

/// The user of an address, in any case; None for "" or nobody.
pub fn owner_of(email: &str) -> Res<Option<String>> {
    if email.is_empty() {
        return Ok(None);
    }
    use rusqlite::OptionalExtension;
    appdb::with(|c| c.query_row(r#"SELECT id FROM "user" WHERE lower(email) = lower(?)"#, [email.trim()], |r| r.get(0)).optional())
}

/// The id of the user the shared accounts belong to (SHARED_ACCOUNTS_OWNER, else ADMIN_EMAIL); None when none.
pub fn shared_owner_id() -> Res<Option<String>> {
    owner_of(&crate::config::shared_accounts_owner())
}

/// The state of a slot as one word for the calendar: stopped, or what its agent says of Teams, and, for a slot without a
/// browser, whether its calls ring.
fn state_of(s: &Slot, m: &mut Map<String, Value>) -> Res<()> {
    let (state, detail) = if s.stopped != 0 {
        ("stopped".to_string(), String::new())
    } else {
        let a = crate::accounts::account_summary(s)?;
        let teams = a["teams"].as_str().unwrap_or("").to_lowercase();
        let dashed = regex::Regex::new(r"[^a-z-]+").expect("fixed").replace_all(&teams, "-");
        let word: String = dashed.trim_matches('-').chars().take(24).collect();
        let state = if word.starts_with(|c: char| c.is_ascii_lowercase()) { word } else { "unknown".into() };
        (state, a["overall"].as_str().unwrap_or("").to_string())
    };
    m.insert("state".into(), json!(state));
    m.insert("detail".into(), json!(detail));
    if s.light != 0 {
        m.insert("calls".into(), json!(s.calls != 0));
    }
    Ok(())
}

/// What the calendar shows next to each account: the slot on it and its state, and the owner's slots with only their own
/// browser sign-in.
pub fn shared_report(owner: &str, identity: &IdentityOf) -> Res<Value> {
    let mut accounts = vec![];
    let mut others = vec![];
    for s in appdb::list_slots()? {
        if s.owner_id != owner {
            continue;
        }
        let mut m = Map::new();
        if !s.shared_account.is_empty() {
            m.insert("account".into(), json!(s.shared_account));
            m.insert("slot".into(), json!(s.slot));
            state_of(&s, &mut m)?;
            accounts.push(Value::Object(m));
        } else if s.relay == 0 {
            m.insert("email".into(), json!(identity(&s)));
            m.insert("slot".into(), json!(s.slot));
            state_of(&s, &mut m)?;
            others.push(Value::Object(m));
        }
    }
    Ok(json!({ "accounts": accounts, "others": others }))
}

/// One of mail, calendar and Teams on or off for one account, applied to the list as the agent has it now. A mailbox
/// that never chose keeps mail and calendar. 404: the account is not in the list, or this server has no shared accounts.
pub async fn set_shared_capability(broker: &dyn Broker, account: &str, capability: &str, on: bool) -> Res<Vec<String>> {
    let Some(raw) = broker.list().await? else { return Err(err(404, NO_SHARED)) };
    let wanted = account.to_lowercase();
    let Some(a) = parse_directory(&raw)?.into_iter().find(|x| x.account == wanted) else { return Err(err(404, format!("{account} is {NOT_SHARED}"))) };
    if on && a.status == "error" {
        return Err(err(409, "Sign in again before enabling a service"));
    }
    if a.has(capability) == on {
        return Ok(a.capabilities);
    }
    let caps: Vec<String> = if on {
        a.capabilities.iter().cloned().chain([capability.to_string()]).collect()
    } else {
        a.capabilities.into_iter().filter(|c| c != capability).collect()
    };
    set_shared_capabilities(broker, &a.account, &caps).await
}

pub async fn set_shared_capabilities(broker: &dyn Broker, account: &str, capabilities: &[String]) -> Res<Vec<String>> {
    let caps: Vec<String> = CAPABILITIES.iter().filter(|c| capabilities.iter().any(|x| x == *c)).map(|c| c.to_string()).collect();
    let Some(r) = broker.set(json!({ "account": account.to_lowercase(), "capabilities": caps })).await? else { return Err(err(404, NO_SHARED)) };
    if r.get("ok") != Some(&json!(true)) {
        return Err(err(409, r.get("errore").and_then(Value::as_str).unwrap_or("Capabilities not changed")));
    }
    Ok(match r.get("shared").and_then(|s| s.get("capabilities")).and_then(Value::as_array) {
        Some(a) => a.iter().map(|c| crate::js::string(Some(c))).collect(),
        None => caps,
    })
}

/// Teams on or off for a shared account, in the directory of the agent; 404 when the account is not there.
pub async fn set_shared_chat(account: &str, on: bool) -> Res<()> {
    set_shared_capability(&Live, account, "chat", on).await.map(drop)
}

/// One pass of the sync, now (syncSharedAccounts with the options of the web app).
pub async fn sync_now() -> Res<()> {
    sync_shared_accounts(&Deps::live(shared_owner_id()?)).await.map(drop)
}

/// The loop of the sync, started at boot (keepSharedAccountsInSync): every minute the pass, then the report to the
/// agent. A failure is logged when it changes, not every minute.
pub fn keep_in_sync() {
    tokio::task::spawn_local(async {
        let mut last = String::new();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let line = match sync_tick().await {
                Ok(line) => line,
                Err(e) => format!("shared accounts: {}", message(&e)),
            };
            if line != last {
                println!("{line}");
            }
            last = line;
        }
    });
}

async fn sync_tick() -> Res<String> {
    let owner = shared_owner_id()?;
    let d = Deps::live(owner.clone());
    let r = sync_shared_accounts(&d).await?;
    let Some(owner) = owner.clone().filter(|_| r.enabled) else {
        return Ok(if owner.is_some() { "shared accounts: off (no broker in the browsers container)" } else { "shared accounts: off (no owner)" }.into());
    };
    d.broker.report(shared_report(&owner, &d.identity)?).await?;
    Ok(if r.errors.is_empty() {
        format!("shared accounts: {} with Teams on", r.accounts.iter().filter(|a| a.has("chat")).count())
    } else {
        let failed: Vec<String> = r.errors.iter().map(|(a, e)| format!("{a}: {}", e.as_str().unwrap_or(""))).collect();
        format!("shared accounts: {}", failed.join("; "))
    })
}

/// /api/shared-accounts (GET, POST).
pub fn routes() -> Vec<Route> {
    vec![route!("GET", "/api/shared-accounts", get), route!("POST", "/api/shared-accounts", post)]
}

fn off() -> Value {
    json!({ "enabled": false, "addUrl": "", "accounts": [], "teamsOnly": [] })
}

/// The one list of accounts, as the calendar shows it: each Microsoft account with mail, calendar and Teams on or off,
/// the Teams account running on it, and the owner's Teams accounts with only their own browser sign-in. Only the owner
/// of the shared accounts sees it.
pub async fn list_for(broker: &dyn Broker, user_id: &str) -> Res<Value> {
    if shared_owner_id()?.as_deref() != Some(user_id) {
        return Ok(off());
    }
    let Some(raw) = broker.list().await? else { return Ok(off()) };
    let slots = appdb::list_slots()?;
    let mut accounts = vec![];
    for a in parse_directory(&raw)? {
        let s = slots.iter().find(|x| x.owner_id == user_id && x.shared_account == a.account);
        let summary = s.map(crate::accounts::account_summary).transpose()?;
        let mut m = a.json();
        m.insert("slot".into(), json!(s.map_or(0, |s| s.slot)));
        m.insert("running".into(), json!(s.is_some_and(|s| s.stopped == 0)));
        m.insert("teams".into(), summary.as_ref().map_or(json!(""), |x| x["teams"].clone()));
        m.insert("overall".into(), summary.as_ref().map_or(json!(""), |x| x["overall"].clone()));
        m.insert("calls".into(), s.filter(|s| s.light != 0).map_or(Value::Null, |s| json!(s.calls != 0)));
        accounts.push(Value::Object(m));
    }
    let mut teams_only = vec![];
    for s in slots.iter().filter(|s| s.owner_id == user_id && s.shared_account.is_empty() && s.relay == 0) {
        let summary = crate::accounts::account_summary(s)?;
        teams_only.push(json!({
            "slot": s.slot,
            "email": summary["email"],
            "name": summary["name"],
            "running": s.stopped == 0,
            "teams": summary["teams"],
            "overall": summary["overall"],
            "calls": if s.light != 0 { json!(s.calls != 0) } else { Value::Null },
        }));
    }
    let or = |k: &str, fallback: Value| raw.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(fallback);
    Ok(json!({
        "enabled": true,
        "addUrl": crate::config::shared_accounts_url(),
        "accounts": accounts,
        "teamsOnly": teams_only,
        "commands": or("commands", json!([])),
        "signins": or("signins", json!([])),
        "labels": or("labels", Value::Null),
    }))
}

async fn get(req: Req) -> Res {
    let user = require_user(&req)?;
    json(list_for(&Live, &user.id).await?)
}

async fn post(req: Req) -> Res {
    let user = require_user(&req)?;
    let b = req.json()?;
    json(change(Rc::new(Live), &user.id, &b).await?)
}

/// `errore` of an answer of the directory, else the fallback (String(r.errore || fallback)).
fn refusal(r: &Option<Value>, fallback: &str) -> String {
    match r.as_ref().and_then(|r| r.get("errore")) {
        Some(e) if crate::js::truthy(e) => crate::js::string(Some(e)),
        _ => fallback.to_string(),
    }
}

fn accepted(r: &Option<Value>) -> bool {
    r.as_ref().and_then(|r| r.get("ok")).is_some_and(crate::js::truthy)
}

/// POST /api/shared-accounts:
/// {account, capability, on}: one service on or off, then the pass when it is Teams;
/// {account, operation: "calls", on}: calls of its Teams account without a browser;
/// {account, operation: "signin-cancel", id}: a guided sign-in stopped, its slot given back;
/// {account, operation: "connect", capabilities}: a guided sign-in asked;
/// {account, operation: "remove"}, or Teams of a browser slot: a command of the directory, then the pass.
pub async fn change(broker: Rc<dyn Broker>, user_id: &str, b: &Map<String, Value>) -> Res<Value> {
    let owner = shared_owner_id()?;
    if owner.as_deref() != Some(user_id) {
        return Err(err(404, "Not found"));
    }
    let owner = user_id.to_string();
    let deps = || Deps { broker: broker.clone(), per_user: crate::config::accounts_per_user(), owner: Some(owner.clone()), identity: Rc::new(email_of) };
    let Some(account) = b.get("account").and_then(Value::as_str).filter(|a| a.contains('@')) else { return Err(err(400, "account must be an address")) };
    let operation = b.get("operation").and_then(Value::as_str).unwrap_or("");
    let on = b.get("on").and_then(Value::as_bool);
    let capability = b.get("capability");
    if operation == "signin-cancel" {
        let Some(id) = b.get("id").and_then(Value::as_str) else { return Err(err(400, "id required")) };
        let r = broker.set(json!({ "operation": "signin-cancel", "account": account, "id": id })).await?;
        if !accepted(&r) {
            return Err(err(409, refusal(&r, "Sign-in not cancelled")));
        }
        // the slot of the sign-in is given back now, not at the next minute's pass
        sync_shared_accounts(&deps()).await?;
        return list_for(broker.as_ref(), user_id).await;
    }
    if operation == "connect" {
        let services = b.get("capabilities").and_then(Value::as_array);
        let Some(services) = services.filter(|c| c.iter().all(|x| x.as_str().is_some_and(|x| CAPABILITIES.contains(&x)))) else {
            return Err(err(400, "Invalid services"));
        };
        let current = broker.list().await?;
        if !current.as_ref().and_then(|c| c.get("signins")).is_some_and(Value::is_array) {
            return Err(err(503, "Microsoft sign-in service is upgrading or unavailable"));
        }
        let r = broker.set(json!({ "operation": "connect", "account": account, "capabilities": services })).await?;
        if !accepted(&r) {
            return Err(err(409, refusal(&r, "Sign-in unavailable")));
        }
        return list_for(broker.as_ref(), user_id).await;
    }
    if operation == "calls" {
        let Some(on) = on else { return Err(err(400, "on must be true or false")) };
        let key = account.trim().to_lowercase();
        let light: Vec<Slot> = appdb::list_slots()?
            .into_iter()
            .filter(|s| s.owner_id == owner && s.relay == 0 && s.light != 0 && (s.shared_account == key || email_of(s).trim() == key))
            .collect();
        if light.is_empty() {
            return Err(err(409, NO_CALLS_SWITCH));
        }
        for s in &light {
            appdb::set_calls(s.slot, on)?;
        }
        // the calendar shows the new state now, not at the next minute's report
        let _ = broker.report(shared_report(&owner, &deps().identity)?).await;
        return list_for(broker.as_ref(), user_id).await;
    }
    let wanted = account.trim().to_lowercase();
    let browser = appdb::list_slots()?.into_iter().any(|s| s.owner_id == owner && s.shared_account.is_empty() && s.relay == 0 && email_of(&s).trim() == wanted);
    if operation == "remove" || (capability == Some(&json!("chat")) && browser) {
        if operation != "remove" && on.is_none() {
            return Err(err(400, "on must be true or false"));
        }
        let mut body = Map::new();
        body.insert("account".into(), json!(account));
        body.insert("operation".into(), json!(if operation == "remove" { "remove" } else { "chat" }));
        crate::js::set(&mut body, "on", on.map(Value::Bool));
        let r = broker.set(Value::Object(body)).await?;
        if !accepted(&r) {
            return Err(err(409, refusal(&r, "Operation failed")));
        }
        sync_shared_accounts(&deps()).await?;
        return list_for(broker.as_ref(), user_id).await;
    }
    let Some(capability) = capability.and_then(Value::as_str).filter(|c| CAPABILITIES.contains(c)) else {
        return Err(err(400, "capability must be mail, calendar or chat"));
    };
    let Some(on) = on else { return Err(err(400, "on must be true or false")) };
    if on && capability == "chat" && browser {
        return Err(err(409, BROWSER_PRESERVED));
    }
    set_shared_capability(broker.as_ref(), account, capability, on).await?;
    // mail and calendar change no Teams slot: no pass, which waits in the queue of every slot's changes
    if capability != "chat" {
        return list_for(broker.as_ref(), user_id).await;
    }
    let r = sync_shared_accounts(&deps()).await?;
    let mut answer = list_for(broker.as_ref(), user_id).await?;
    if let (Some(e), Some(m)) = (r.errors.get(&account.to_lowercase()), answer.as_object_mut()) {
        m.insert("error".into(), e.clone());
    }
    Ok(answer)
}

#[cfg(test)]
mod tests;
