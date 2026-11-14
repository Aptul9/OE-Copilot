//! /api/accounts/*, /api/desktop/[slot], /api/authcheck, /api/health: the accounts of a user and their modes.

use crate::appdb;
use crate::auth::{current_user, login_url, owned_slot, require_account, require_slot, require_user, safe_next};
use crate::route;
use crate::server::Route;
use crate::slots::{self, SHARED_NO_BROWSER, shared_account_of};
use crate::web::{Req, Res, err, json, now_ms, ok, redirect};
use serde_json::{Map, Value, json};

pub fn routes() -> Vec<Route> {
    vec![
        route!("GET", "/api/accounts", list),
        route!("POST", "/api/accounts", add),
        route!("PATCH", "/api/accounts/[slot]", patch),
        route!("DELETE", "/api/accounts/[slot]", remove),
        route!("GET", "/api/accounts/[slot]/browser", browser_get),
        route!("PATCH", "/api/accounts/[slot]/browser", browser_patch),
        route!("POST", "/api/accounts/[slot]/check", check),
        route!("POST", "/api/accounts/[slot]/token", token),
        route!("GET", "/api/desktop/[slot]", desktop_get),
        route!("POST", "/api/desktop/[slot]", desktop_post),
        route!("GET", "/api/authcheck", authcheck),
        route!("GET", "/api/health", health),
    ]
}

async fn list(req: Req) -> Res {
    let u = require_user(&req)?;
    json(crate::accounts::accounts_of(&u.id)?)
}

/// New Teams account; {relay: true}: an account on another computer, with the token of its relay shown this once.
async fn add(req: Req) -> Res {
    let u = require_user(&req)?;
    let b: Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
    if b.get("relay") == Some(&Value::Bool(true)) {
        let (slot, token) = slots::add_relay_account(&u.id).await?;
        return json(json!({ "ok": true, "slot": slot, "token": token, "server": crate::config::app_url() }));
    }
    let slot = slots::add_account(&u.id).await?;
    json(json!({ "ok": true, "slot": slot, "desktop": crate::config::desktop_url_of(slot) }))
}

/// A slot on a shared account changes the calendar's list: only the owner of the shared accounts does it.
fn require_shared_owner(req: &Req, n: i64) -> Res<()> {
    let u = require_user(req)?;
    let owner = match crate::shared_accounts::shared_owner_id()? {
        Some(o) => Some(o),
        None => appdb::slot_owner(n)?,
    };
    if owner.as_deref() != Some(u.id.as_str()) {
        return Err(err(403, "Only the owner of the shared Microsoft accounts changes this Teams account"));
    }
    Ok(())
}

/// Teams off for an account; false when there is nothing to turn off.
async fn chat_off(account: &str) -> Res<bool> {
    match crate::shared_accounts::set_shared_chat(account, false).await {
        Ok(()) => Ok(true),
        Err(crate::web::Error::Http(404, _, _)) => Ok(false),
        Err(e) => Err(e),
    }
}

async fn patch(req: Req) -> Res {
    let n = require_account(&req, &req.param("slot"))?;
    let b = req.json()?;
    let get = |k: &str| b.get(k);
    if let Some(calls) = get("calls") {
        let Some(on) = calls.as_bool() else { return Err(err(400, "calls must be true or false")) };
        appdb::set_calls(n, on)?;
        return json(json!({ "ok": true, "calls": on }));
    }
    let (mobile, desktop) = (get("notifyMobile"), get("notifyDesktop"));
    if mobile.is_some() || desktop.is_some() {
        for (name, v) in [("notifyMobile", mobile), ("notifyDesktop", desktop)] {
            if v.is_some_and(|v| !v.is_boolean()) {
                return Err(err(400, format!("{name} must be true or false")));
            }
        }
        appdb::set_notify(n, mobile.and_then(Value::as_bool), desktop.and_then(Value::as_bool))?;
        let mut o = Map::new();
        o.insert("ok".into(), json!(true));
        crate::js::set(&mut o, "notifyMobile", mobile.cloned());
        crate::js::set(&mut o, "notifyDesktop", desktop.cloned());
        return json(Value::Object(o));
    }
    let (running, check_every, light) = (get("running"), get("checkEvery"), get("light"));
    let shared = shared_account_of(n)?;
    if !shared.is_empty() {
        if check_every.is_some() {
            return Err(err(409, SHARED_NO_BROWSER));
        }
        let on = light == Some(&json!(true)) || running == Some(&json!(true));
        if !on && running != Some(&json!(false)) {
            return Err(err(400, "running must be true or false"));
        }
        require_shared_owner(&req, n)?;
        if on {
            crate::shared_accounts::set_shared_chat(&shared, true).await?;
        } else if !chat_off(&shared).await? {
            slots::set_account_running(n, false).await?;
            return json(json!({ "ok": true, "running": false, "shared": shared }));
        }
        crate::shared_accounts::sync_now().await?;
        return json(json!({ "ok": true, "running": on, "shared": shared }));
    }
    if let Some(l) = light {
        if l != &json!(true) {
            return Err(err(400, "light must be true: choose another mode to end it"));
        }
        slots::set_light_mode(n).await?;
        return json(json!({ "ok": true, "light": l }));
    }
    if let Some(c) = check_every {
        let Some(every) = c.as_f64() else { return Err(err(400, "checkEvery must be a number of seconds")) };
        if every.fract() != 0.0 || every != 0.0 && !slots::CHECK_INTERVALS.contains(&(every as i64)) {
            return Err(err(400, format!("checkEvery must be 0 or one of {}", slots::CHECK_INTERVALS.map(|x| x.to_string()).join(", "))));
        }
        slots::set_check_mode(n, every as i64).await?;
        return json(json!({ "ok": true, "checkEvery": c }));
    }
    let Some(r) = running.and_then(Value::as_bool) else { return Err(err(400, "running must be true or false")) };
    slots::set_account_running(n, r).await?;
    json(json!({ "ok": true, "running": r }))
}

async fn remove(req: Req) -> Res {
    let n = require_account(&req, &req.param("slot"))?;
    let shared = shared_account_of(n)?;
    if !shared.is_empty() {
        require_shared_owner(&req, n)?;
        chat_off(&shared).await?;
    }
    slots::remove_account(n).await?;
    ok()
}

async fn browser_get(req: Req) -> Res {
    let row = owned_slot(&require_user(&req)?.id, &req.param("slot"))?;
    let actions: Vec<Value> = appdb::browser_actions_of(row.slot, 50)?
        .into_iter()
        .map(|a| json!({ "ts": a["ts"], "client": a["client_id"], "tool": a["tool"], "host": a["host"], "outcome": a["outcome"] }))
        .collect();
    // the browsers of the relays served only OAuth clients of /mcp, which this server does not have: never connected
    json(json!({ "off": row.browser_off != 0, "connected": false, "actions": actions }))
}

async fn browser_patch(req: Req) -> Res {
    let row = owned_slot(&require_user(&req)?.id, &req.param("slot"))?;
    let b = req.json()?;
    let Some(off) = b.get("off").and_then(Value::as_bool) else { return Err(err(400, "off must be true or false")) };
    if row.relay == 0 {
        return Err(err(409, "Only the relay of an account on another computer has a browser for AI clients"));
    }
    appdb::set_browser_off(row.slot, off)?;
    json(json!({ "ok": true, "off": off }))
}

/// "Check now": the check of an account checked every N hours is due at once; for a light account, its browser run.
async fn check(req: Req) -> Res {
    let n = require_account(&req, &req.param("slot"))?;
    let s = appdb::slot_row(n)?;
    if s.as_ref().is_some_and(|s| s.stopped != 0) {
        return Err(err(409, "This Teams account is stopped: start it from its page or from Settings"));
    }
    if s.as_ref().is_some_and(|s| s.light != 0) {
        if !shared_account_of(n)?.is_empty() {
            return Err(err(409, SHARED_NO_BROWSER));
        }
        appdb::ask_refresh(n)?;
        return ok();
    }
    if !s.as_ref().is_some_and(|s| s.check_every != 0) {
        return Err(err(409, "This Teams account is always on: there is nothing to check"));
    }
    appdb::ask_check(n)?;
    ok()
}

async fn token(req: Req) -> Res {
    let row = owned_slot(&require_user(&req)?.id, &req.param("slot"))?;
    let token = slots::renew_relay_token(row.slot).await?;
    json(json!({ "ok": true, "token": token, "server": crate::config::app_url() }))
}

const WINDOW_WAIT: usize = 45;
const NOT_STARTED: &str = "The browser of this account did not start: open it again in a minute";

async fn own_account(user_id: &str, slot: &str) -> Res<(i64, bool)> {
    let row = owned_slot(user_id, slot)?;
    if row.relay != 0 {
        return Err(err(409, crate::relay::ON_ANOTHER_COMPUTER));
    }
    if !shared_account_of(row.slot)?.is_empty() {
        return Err(err(409, SHARED_NO_BROWSER));
    }
    if !crate::light::browser_on_demand(row.slot).await? {
        return Err(err(503, NOT_STARTED));
    }
    Ok((row.slot, row.light != 0))
}

/// The window of the account to the front of the desktop; the window of a browser just started waited for.
async fn to_front(n: i64, wait_window: bool) -> bool {
    let _ = crate::slotdb::with_slot_or(n, |r| r.mark_desktop(), ());
    let mut i = 0;
    loop {
        let shown = crate::control::show(n).await.unwrap_or(false);
        if shown || !wait_window || i >= WINDOW_WAIT {
            return shown;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        i += 1;
    }
}

async fn desktop_get(req: Req) -> Res {
    let Some(u) = current_user(&req)? else { return redirect(&login_url(&req.path)) };
    let (n, light) = own_account(&u.id, &req.param("slot")).await?;
    if !to_front(n, light).await && light {
        return Err(err(503, NOT_STARTED));
    }
    redirect("/desktop/")
}

async fn desktop_post(req: Req) -> Res {
    let u = current_user(&req)?.ok_or_else(|| err(401, "Not signed in"))?;
    let (n, light) = own_account(&u.id, &req.param("slot")).await?;
    json(json!({ "ok": true, "shown": to_front(n, light).await }))
}

/// Caddy forward_auth for /desktop/: only users with a Teams account of the browsers container get through.
async fn authcheck(req: Req) -> Res {
    let uri = req.header("x-forwarded-uri").filter(|u| !u.is_empty()).unwrap_or_else(|| "/".into());
    let Some(u) = current_user(&req)? else { return redirect(&login_url(&safe_next(Some(&uri)))) };
    if !appdb::slots_of(&u.id)?.iter().any(|s| s.relay == 0) {
        return Ok(crate::web::json_with(403, &json!({ "detail": "No Teams account on this user" })));
    }
    ok()
}

async fn health(req: Req) -> Res {
    let (u, slot) = require_slot(&req)?;
    let _ = now_ms();
    json(crate::accounts::health_for(&u.id, slot)?)
}
