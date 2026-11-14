//! Accounts checked every N hours run only while they are checked: started, read, stopped, one check at a time
//! (src/lib/checks.ts); the browser runs of light accounts come after, in the same loop (light.rs).

use crate::appdb;
use crate::slotdb::{with_slot, with_slot_or};
use crate::slots::{exclusive, slot_down, slot_up};
use crate::state::{self, CallState, InCall, LightSignIn, key};
use crate::web::{Res, now_ms};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Duration;

pub const TEAMS_WAIT: i64 = 240;
/// SIGN_IN_TRY_AFTER + SIGN_IN_TRY_WAIT + 15
pub const SIGNED_OUT: i64 = 15 + 60 + 15;
const COMMAND_WAIT: i64 = 180;
pub const ALERT_WAIT: i64 = 30;
pub const SIGN_IN_WAIT: i64 = 600;

fn now() -> i64 {
    now_ms().div_euclid(1000)
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

fn report(n: i64, e: &crate::web::Error) {
    eprintln!("check of account {n}: {}", crate::slots::detail(e));
}

/// The agent's health row: when written, the Teams state, whether the owner uses Teams on the desktop.
pub struct Health {
    pub ts: f64,
    pub teams: String,
    pub in_use: bool,
}

pub fn health(n: i64) -> Option<Health> {
    with_slot_or(
        n,
        |r| {
            let h = r.state(key::HEALTH, json!({}));
            Ok(match (h.get("ts").and_then(Value::as_f64), h.get("teams").and_then(Value::as_str)) {
                (Some(ts), Some(teams)) => Some(Health { ts, teams: teams.into(), in_use: h.get("desktop") == Some(&json!("in-use")) }),
                _ => None,
            })
        },
        None,
    )
    .ok()
    .flatten()
}

/// armed: a signed-in identity saved once; alerted: the agent pushed its "Teams signed out".
pub fn sign_in_alert(n: i64) -> (bool, bool) {
    with_slot_or(
        n,
        |r| {
            let armed = !r.state(key::ME, Value::Null).is_null();
            let w = r.state(key::LOGIN_WATCH, json!({}));
            // Watch.safeParse: an object; alerted falls back to false
            let alerted = w.is_object() && w.get("alerted").and_then(Value::as_bool).unwrap_or(false);
            Ok((armed, alerted))
        },
        (false, false),
    )
    .unwrap_or((false, false))
}

pub fn light_sign_in(n: i64) -> Option<LightSignIn> {
    with_slot_or(
        n,
        |r| {
            let raw = r.state(key::LIGHT_TOKEN, Value::Null);
            Ok(if raw.is_null() { None } else { Some(LightSignIn::parse_value(&raw)) })
        },
        None,
    )
    .ok()
    .flatten()
}

/// A call rings or is in progress on the account.
pub fn busy(n: i64) -> bool {
    with_slot_or(
        n,
        |r| {
            let t = now_ms() as f64;
            let in_call = InCall::parse(Some(&crate::js::stringify(&r.state(key::IN_CALL, Value::Null))));
            let call = CallState::parse(Some(&crate::js::stringify(&r.state(key::CALL, Value::Null))));
            Ok(state::in_call_of(in_call.as_ref(), t).is_some() || state::ringing_call(call.as_ref(), t).is_some())
        },
        false,
    )
    .unwrap_or(false)
}

fn command_status(n: i64, id: i64) -> Option<String> {
    with_slot_or(n, |r| Ok(r.command_status(id).and_then(|v| v.get("status").and_then(Value::as_str).map(String::from))), None).ok().flatten()
}

fn current(n: i64) -> bool {
    appdb::slot_row(n).ok().flatten().is_some_and(|s| s.stopped == 0 && s.check_every != 0)
}

/// Starts the account, waits for Teams, has the agent read everything, stops it. Some(result) recorded, None: the
/// account changed meanwhile.
pub async fn run_check(n: i64) -> Option<&'static str> {
    let begun = now();
    let mut asked = false;
    let mut sign_in = false;
    let started: Res<Result<bool, &'static str>> = exclusive(async {
        let Some(s) = appdb::slot_row(n)? else { return Ok(Ok(false)) };
        if s.stopped != 0 || s.check_every == 0 {
            return Ok(Ok(false));
        }
        asked = s.check_due == 0;
        sign_in = asked && (s.check_result == "login" || !sign_in_alert(n).0);
        appdb::begin_check(n, begun)?;
        if let Err(e) = slot_up(n).await {
            report(n, &e);
            if let Err(e) = slot_down(n).await {
                report(n, &e);
            }
            appdb::end_check(n, now(), Some("failed"), asked)?;
            return Ok(Err("failed"));
        }
        Ok(Ok(true))
    })
    .await;
    match started {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => return None,
        Ok(Err(r)) => return Some(r),
        Err(e) => {
            report(n, &e);
            return None;
        }
    }
    let result = wait_and_read(n, begun, sign_in).await;
    exclusive(async {
        let s = appdb::slot_row(n).ok().flatten();
        if s.as_ref().is_some_and(|s| s.stopped == 0 && s.check_every != 0)
            && let Err(e) = slot_down(n).await
        {
            report(n, &e);
        }
        if s.is_some()
            && let Err(e) = appdb::end_check(n, now(), result, asked)
        {
            report(n, &e);
        }
    })
    .await;
    result
}

async fn wait_and_read(n: i64, begun: i64, sign_in: bool) -> Option<&'static str> {
    let mut signed_out_since = 0;
    loop {
        if !current(n) {
            return None;
        }
        let h = health(n);
        let fresh = h.as_ref().is_some_and(|h| h.ts >= begun as f64);
        if fresh && h.as_ref().is_some_and(|h| h.teams == "ok") {
            return read_all(n, begun).await;
        }
        if fresh && h.as_ref().is_some_and(|h| h.teams == "login") {
            if signed_out_since == 0 {
                signed_out_since = now();
            }
        } else if fresh {
            signed_out_since = 0;
        }
        if !sign_in && signed_out_since != 0 && now() - signed_out_since >= SIGNED_OUT {
            wait_for_alert(n).await;
            return Some("login");
        }
        if now() - begun >= if sign_in { SIGN_IN_WAIT } else { TEAMS_WAIT } {
            if signed_out_since == 0 {
                return Some("failed");
            }
            wait_for_alert(n).await;
            return Some("login");
        }
        sleep(2000).await;
    }
}

async fn read_all(n: i64, begun: i64) -> Option<&'static str> {
    let id = match with_slot(n, |r| r.enqueue("check", "", "")) {
        Ok(id) => id,
        Err(e) => {
            report(n, &e);
            return Some("failed");
        }
    };
    let until = now() + COMMAND_WAIT;
    let mut signed_out_since = 0;
    while now() < until {
        if !current(n) {
            return None;
        }
        match command_status(n, id).as_deref() {
            Some("done") => return Some("ok"),
            Some("failed") => return Some("failed"),
            _ => {}
        }
        let h = health(n);
        if h.as_ref().is_some_and(|h| h.ts >= begun as f64 && h.teams == "login") {
            if signed_out_since == 0 {
                signed_out_since = now();
            }
        } else {
            signed_out_since = 0;
        }
        if signed_out_since != 0 && now() - signed_out_since >= SIGNED_OUT {
            wait_for_alert(n).await;
            return Some("login");
        }
        sleep(1000).await;
    }
    Some("failed")
}

async fn wait_for_alert(n: i64) {
    let until = now() + ALERT_WAIT;
    while now() < until {
        let (armed, alerted) = sign_in_alert(n);
        if !armed || alerted {
            return;
        }
        sleep(2000).await;
    }
}

/// Every check due now, one after the other; an account comes once per call.
pub async fn run_due_checks() {
    let mut done = HashSet::new();
    loop {
        let s = match appdb::next_check(now()) {
            Ok(Some(s)) => s,
            Ok(None) => return,
            Err(e) => return eprintln!("checks: {e}"),
        };
        if !done.insert(s.slot) {
            return;
        }
        run_check(s.slot).await;
    }
}

/// The loop of the web app: the checks due, then the browser runs of the light accounts due, every 10 s.
pub fn run_checks() {
    tokio::task::spawn_local(async {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            run_due_checks().await;
            crate::light::run_due_refreshes().await;
        }
    });
}

/// At boot: a check or a sign-in run cut by the restart left its browser running.
pub async fn settle_checks() {
    exclusive(async {
        let Ok(slots) = appdb::list_slots() else { return };
        for s in slots {
            if s.light != 0 && s.refreshing != 0 {
                let _ = appdb::end_refresh(s.slot, 0, None);
            }
            if s.check_every == 0 || s.stopped != 0 {
                continue;
            }
            if let Err(e) = slot_down(s.slot).await {
                report(s.slot, &e);
            }
            if s.checking != 0 {
                let _ = appdb::end_check(s.slot, 0, None, false);
            }
        }
    })
    .await
}
