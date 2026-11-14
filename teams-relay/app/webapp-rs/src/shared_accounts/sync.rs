//! One pass over the shared accounts (syncSharedAccounts) and the guided sign-ins of the calendar (syncSignIns).

use super::{BROWSER_PRESERVED, Deps, NO_CALLS_SWITCH, SharedAccount, fail, message, parse_directory, shared_report};
use crate::appdb::{self, Slot};
use crate::web::{Error, Res};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub struct SyncResult {
    pub enabled: bool,
    pub accounts: Vec<SharedAccount>,
    /// account: why it failed
    pub errors: Map<String, Value>,
}

fn off() -> SyncResult {
    SyncResult { enabled: false, accounts: vec![], errors: Map::new() }
}

/// A slot stopped for good: marked stopped only once its stop worked, so a failed one is tried again at the next pass.
async fn stop_slot(d: &Deps, n: i64, errors: &mut Map<String, Value>, account: &str) {
    let r = async {
        d.broker.stop(n).await?;
        appdb::set_slot_stopped(n, true)
    }
    .await;
    if let Err(e) = r {
        errors.insert(account.into(), json!(message(&e)));
    }
}

/// The address of a slot as the matching reads it: trimmed, lower case.
fn address(d: &Deps, s: &Slot) -> String {
    (d.identity)(s).trim().to_lowercase()
}

/// One pass: every account with Teams on has a running slot of the owner, every slot of an account with Teams off (or
/// gone from the list) is stopped. Nothing changes when the list cannot be read. Runs in the queue of the slots, the list
/// read there too: a pass that waited behind a change made from a page reads the list after it.
pub async fn sync_shared_accounts(d: &Deps) -> Res<SyncResult> {
    let Some(owner) = d.owner.clone() else { return Ok(off()) };
    crate::slots::exclusive(async {
        let Some(raw) = d.broker.list().await? else { return Ok(off()) };
        let directory_accounts = parse_directory(&raw)?;
        if let Some(signins) = raw.get("signins").and_then(Value::as_array) {
            let expired = regex::Regex::new("Sessione del browser|Accesso scaduto").expect("fixed");
            for a in directory_accounts.iter().filter(|a| a.status == "error" && a.capabilities.iter().any(|c| c != "chat") && expired.is_match(&a.message)) {
                if signins.iter().any(|j| j.get("account") == Some(&json!(a.account)) && j.get("phase") != Some(&json!("complete"))) {
                    continue;
                }
                let browser =
                    appdb::list_slots()?.into_iter().find(|s| s.owner_id == owner && s.relay == 0 && s.shared_account.is_empty() && address(d, s) == a.account);
                let mut capabilities: Vec<String> = vec![];
                let running = browser.is_some_and(|b| b.stopped == 0).then(|| "chat".to_string());
                for c in a.capabilities.iter().cloned().chain(running) {
                    if !capabilities.contains(&c) {
                        capabilities.push(c);
                    }
                }
                let _ = d.broker.set(json!({ "operation": "connect", "account": a.account, "capabilities": capabilities })).await;
            }
        }
        sync_sign_ins(d, &owner, raw.get("signins")).await?;

        let commands: Vec<Value> = raw.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut results = vec![];
        for c in &commands {
            let mut result = Map::new();
            crate::js::set(&mut result, "id", c.get("id").cloned());
            if let Err(e) = run_command(d, &owner, c).await {
                result.insert("error".into(), json!(message(&e)));
            }
            results.push(Value::Object(result));
        }
        if !commands.is_empty() {
            let mut report = shared_report(&owner, &d.identity)?;
            if let Some(m) = report.as_object_mut() {
                m.insert("results".into(), Value::Array(results));
            }
            d.broker.report(report).await?;
        }
        let removed = |a: &SharedAccount| commands.iter().any(|c| c.get("operation") == Some(&json!("remove")) && c.get("account") == Some(&json!(a.account)));
        let accounts: Vec<SharedAccount> = directory_accounts.iter().filter(|a| !removed(a)).cloned().collect();

        let mut errors = Map::new();
        let slots = appdb::list_slots()?;
        // each slot's address read once per pass, only when an account needs it
        let mut known: HashMap<i64, String> = HashMap::new();
        for a in &accounts {
            let bound = slots.iter().find(|s| s.owner_id == owner && s.shared_account == a.account);
            if !a.has("chat") {
                if let Some(b) = bound.filter(|b| b.stopped == 0) {
                    stop_slot(d, b.slot, &mut errors, &a.account).await;
                }
                continue;
            }
            let r: Res<()> = async {
                if let Some(b) = bound {
                    if b.stopped != 0 || b.light == 0 {
                        let region = if b.shared_region.is_empty() { "emea" } else { &b.shared_region };
                        appdb::bind_shared(b.slot, &a.account, region)?;
                        d.broker.light(b.slot).await?;
                        appdb::set_slot_stopped(b.slot, false)?;
                    }
                    return Ok(());
                }
                // a slot of the owner already signed in to this account in its own browser: kept, not doubled (both would
                // push the same messages). One busy with a check or a sign-in run waits for the next pass.
                let own = slots.iter().find(|s| {
                    s.owner_id == owner
                        && s.shared_account.is_empty()
                        && s.relay == 0
                        && *known.entry(s.slot).or_insert_with(|| (d.identity)(s).to_lowercase()) == a.account
                });
                if let Some(own) = own {
                    if own.checking == 0 && own.refreshing == 0 {
                        return Err(Error::Internal(BROWSER_PRESERVED.into()));
                    }
                    return Ok(());
                }
                let n = appdb::claim_slot(&owner, d.per_user)?;
                let up: Res<()> = async {
                    let _ = d.broker.stop(n).await;
                    d.broker.wipe(n).await?;
                    appdb::bind_shared(n, &a.account, "emea")?;
                    d.broker.light(n).await
                }
                .await;
                if let Err(e) = up {
                    appdb::release_slot(n)?;
                    return Err(e);
                }
                Ok(())
            }
            .await;
            if let Err(e) = r {
                errors.insert(a.account.clone(), json!(message(&e)));
            }
        }
        // a slot whose account left the list of the calendar: stopped, its data kept until its owner removes it
        for s in &slots {
            if s.owner_id != owner || s.shared_account.is_empty() || s.stopped != 0 || accounts.iter().any(|a| a.account == s.shared_account) {
                continue;
            }
            stop_slot(d, s.slot, &mut errors, &s.shared_account).await;
        }
        Ok(SyncResult { enabled: true, accounts, errors })
    })
    .await
}

/// A command of the directory on the owner's slots of one address: remove, chat on or off of a browser slot, calls.
async fn run_command(d: &Deps, owner: &str, c: &Value) -> Res<()> {
    let account = c.get("account").and_then(Value::as_str).unwrap_or("");
    let operation = c.get("operation").and_then(Value::as_str).unwrap_or("");
    let on = c.get("on").and_then(Value::as_bool);
    let matching: Vec<Slot> =
        appdb::list_slots()?.into_iter().filter(|s| s.owner_id == owner && s.relay == 0 && (s.shared_account == account || address(d, s) == account)).collect();
    if operation == "calls" {
        let light: Vec<&Slot> = matching.iter().filter(|s| s.light != 0).collect();
        let Some(on) = on.filter(|_| !light.is_empty()) else { return Err(fail(NO_CALLS_SWITCH)) };
        for s in light {
            appdb::set_calls(s.slot, on)?;
        }
        return Ok(());
    }
    for slot in &matching {
        if operation == "remove" {
            if slot.shared_account != account && address(d, slot) != account {
                return Err(fail("Identity changed"));
            }
            d.broker.stop(slot.slot).await?;
            d.broker.wipe(slot.slot).await?;
            appdb::release_slot(slot.slot)?;
        } else if let (true, true, Some(on)) = (operation == "chat", slot.shared_account.is_empty(), on) {
            if !on {
                d.broker.stop(slot.slot).await?;
            } else if slot.light != 0 {
                d.broker.light(slot.slot).await?;
            } else {
                d.broker.start(slot.slot).await?;
            }
            appdb::set_slot_stopped(slot.slot, !on)?;
        } else {
            return Err(fail("Unsupported account command"));
        }
    }
    Ok(())
}

/// The agent's timeout of a sign-in, for a stopped one that does not say when it stopped.
const SIGNIN_TIMEOUT_S: i64 = 300;

struct SignIn {
    id: String,
    account: String,
    phase: String,
    slot: Option<i64>,
    capabilities: Vec<String>,
    route: bool,
    created: Option<i64>,
    ended: Option<i64>,
}

impl SignIn {
    fn of(v: &Value) -> SignIn {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| *x != 0.0).map(|x| x as i64);
        SignIn {
            id: s("id"),
            account: s("account"),
            phase: s("phase"),
            slot: n("slot"),
            capabilities: v
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            route: v.get("route").is_some_and(crate::js::truthy),
            created: n("created"),
            ended: n("ended"),
        }
    }

    fn running(&self) -> bool {
        self.phase == "queued" || self.phase == "browser"
    }

    fn has(&self, c: &str) -> bool {
        self.capabilities.iter().any(|x| x == c)
    }
}

/// The guided sign-ins of the calendar, inside the queue of the slots. Progress is persisted before a browser starts, so
/// a restart never allocates it twice.
async fn sync_sign_ins(d: &Deps, owner: &str, input: Option<&Value>) -> Res<()> {
    let Some(input) = input.and_then(Value::as_array) else { return Ok(()) };
    let jobs: Vec<SignIn> = input.iter().map(SignIn::of).collect();
    let reserved_elsewhere = |job: &SignIn, slot: i64| jobs.iter().any(|o| o.id != job.id && o.slot == Some(slot) && o.running());
    let ours = |row: &Slot| row.owner_id == owner && row.relay == 0 && row.shared_account.is_empty();
    for job in &jobs {
        if job.phase == "blocked"
            && let Some(n) = job.slot
        {
            let row = appdb::slot_row(n)?;
            let addr = row.as_ref().map(|r| address(d, r)).unwrap_or_default();
            let mine = row.as_ref().is_some_and(ours) && !reserved_elsewhere(job, n);
            // a slot claimed for this sign-in (between its start and its end) that never signed in: given back, so no
            // nameless account stays behind. A slot claimed later, for another account, is never touched.
            let claimed_for_it = job
                .created
                .is_some_and(|created| row.as_ref().is_some_and(|r| r.added >= created && r.added <= job.ended.unwrap_or(created + SIGNIN_TIMEOUT_S)));
            if mine && addr.is_empty() && claimed_for_it {
                d.broker.stop(n).await?;
                d.broker.wipe(n).await?;
                appdb::release_slot(n)?;
            } else if mine && row.as_ref().is_some_and(|r| r.stopped != 0) && (addr.is_empty() || addr == job.account) {
                d.broker.stop(n).await?;
            }
        }
        if !job.running() {
            continue;
        }
        let mut slot = job.slot;
        if let Err(e) = sign_in(d, owner, job, &mut slot, &reserved_elsewhere).await {
            let _ = progress(d, job, slot, "blocked", Some(&message(&e))).await;
        }
    }
    Ok(())
}

async fn progress(d: &Deps, job: &SignIn, slot: Option<i64>, phase: &str, error: Option<&str>) -> Res<()> {
    let mut body = Map::new();
    body.insert("operation".into(), json!("signin-progress"));
    body.insert("account".into(), json!(job.account));
    body.insert("id".into(), json!(job.id));
    crate::js::set(&mut body, "slot", slot.map(|n| json!(n)));
    body.insert("phase".into(), json!(phase));
    if let Some(n) = slot {
        body.insert("url".into(), json!(format!("{}/api/desktop/{n}", crate::config::app_url())));
    }
    if let Some(e) = error {
        body.insert("error".into(), json!(e));
    }
    let r = d.broker.set(Value::Object(body)).await?;
    if !super::accepted(&r) {
        return Err(fail("Could not persist sign-in progress"));
    }
    Ok(())
}

fn teams_ok(row: &Slot) -> Res<bool> {
    Ok(crate::accounts::account_summary(row)?["teams"] == "ok")
}

async fn resume(d: &Deps, row: &Slot) -> Res<()> {
    if row.light != 0 { d.broker.light(row.slot).await } else { d.broker.start(row.slot).await }?;
    appdb::set_slot_stopped(row.slot, false)
}

/// One sign-in job queued or with its browser open; `slot` is the slot it ends up on, for the progress of a failure.
async fn sign_in(d: &Deps, owner: &str, job: &SignIn, slot: &mut Option<i64>, reserved_elsewhere: &impl Fn(&SignIn, i64) -> bool) -> Res<()> {
    let ownership_changed = || fail("Sign-in slot ownership changed");
    let existing =
        appdb::list_slots()?.into_iter().find(|s| s.owner_id == owner && s.relay == 0 && s.shared_account.is_empty() && address(d, s) == job.account);
    if let Some(n) = *slot {
        let hinted = appdb::slot_row(n)?;
        let usable = hinted.as_ref().is_some_and(|h| {
            let addr = address(d, h);
            h.owner_id == owner && h.relay == 0 && h.shared_account.is_empty() && (addr.is_empty() || addr == job.account) && !reserved_elsewhere(job, n)
        });
        if !usable {
            if job.phase != "queued" {
                return Err(ownership_changed());
            }
            *slot = None;
        }
    }
    if slot.is_none() {
        *slot = existing.map(|s| s.slot);
    }
    let n = match *slot {
        Some(n) => n,
        None => {
            let n = appdb::claim_slot(owner, d.per_user)?;
            *slot = Some(n);
            if let Err(e) = progress(d, job, *slot, "browser", None).await {
                appdb::release_slot(n)?;
                return Err(e);
            }
            d.broker.stop(n).await?;
            d.broker.wipe(n).await?;
            if !job.has("chat") {
                appdb::set_slot_stopped(n, true)?;
                appdb::set_light(n, true)?;
                d.broker.signin(n).await?;
            } else {
                d.broker.start(n).await?;
                crate::slots::start_light(n)?;
            }
            n
        }
    };
    let row = appdb::slot_row(n)?.filter(|r| r.owner_id == owner && r.relay == 0 && r.shared_account.is_empty()).ok_or_else(ownership_changed)?;
    let actual = address(d, &row);
    if !actual.is_empty() && actual != job.account {
        return Err(fail("A different Microsoft account was signed in. Use the requested address."));
    }
    progress(d, job, *slot, "browser", None).await?;
    // Teams only: complete once its agent runs on the address asked
    if !job.capabilities.iter().any(|c| c != "chat") {
        if actual != job.account {
            return Ok(());
        }
        if row.stopped != 0 {
            return resume(d, &row).await;
        }
        if !teams_ok(&row)? {
            return Ok(());
        }
        return progress(d, job, *slot, "complete", None).await;
    }
    let result = if job.route {
        json!({ "ok": true })
    } else {
        d.broker.connect(json!({ "id": job.id, "account": job.account, "slot": n })).await?.unwrap_or(Value::Null)
    };
    if result.get("interaction").is_some_and(crate::js::truthy) {
        if job.phase == "queued" {
            if row.stopped != 0 {
                d.broker.signin(n).await?;
            } else if row.light != 0 {
                appdb::ask_refresh(n)?;
            } else {
                d.broker.start(n).await?;
            }
        }
        return Ok(());
    }
    if !result.get("ok").is_some_and(crate::js::truthy) {
        let why = result.get("errore").filter(|e| crate::js::truthy(e)).map(|e| crate::js::string(Some(e)));
        return Err(fail(why.unwrap_or_else(|| "Microsoft refused this sign-in route".into())));
    }
    crate::accounts::save_mailbox_identity(n, &job.account)?;
    if !job.has("chat") {
        d.broker.stop(n).await?;
        appdb::set_slot_stopped(n, true)?;
    } else if row.stopped != 0 {
        return resume(d, &row).await;
    } else if actual != job.account || !teams_ok(&row)? {
        return Ok(());
    }
    progress(d, job, *slot, "complete", None).await
}
