//! Light accounts (src/lib/light.ts): direct Microsoft sessions renew over HTTP in the light agent; refused sessions,
//! MCAS, and sign-ins asked by the owner get a bounded browser run with the existing profile, one account at a time.

use crate::appdb::{self, REFRESH_ASKED, Slot};
use crate::checks::{SIGN_IN_WAIT, SIGNED_OUT, busy, health, light_sign_in};
use crate::control;
use crate::slots::{exclusive, slot_up};
use crate::state::LightSignIn;
use crate::web::{Res, now_ms};
use std::time::Duration;

pub const REFRESH_WAIT: i64 = 360;
pub const MIN_LIFE: f64 = 600.0;
const AFTER_OK: i64 = 600;
const AFTER_FAILED: i64 = 1800;
pub const BROWSER_WAIT: i64 = 90;

fn now() -> i64 {
    now_ms().div_euclid(1000)
}

fn report(n: i64, e: &crate::web::Error) {
    eprintln!("sign-in run of account {n}: {}", crate::slots::detail(e));
}

pub fn refresh_due(s: &Slot, sign: Option<&LightSignIn>, now: i64) -> bool {
    if s.light == 0 || s.stopped != 0 || s.relay != 0 || s.refreshing != 0 {
        return false;
    }
    if s.refresh_result == REFRESH_ASKED {
        return true;
    }
    if !s.shared_account.is_empty() || sign.is_some_and(|x| x.broker == Some(true)) {
        return false;
    }
    if sign.is_some_and(|x| x.session == Some(true) && x.browser != Some(true) && !x.refused) {
        return false;
    }
    if s.refresh_result == "login" && sign.map_or(0.0, |x| x.at) <= s.refreshed as f64 {
        return false;
    }
    if s.refreshed != 0 && now - s.refreshed < if s.refresh_result == "ok" { AFTER_OK } else { AFTER_FAILED } {
        return false;
    }
    let Some(sign) = sign.filter(|x| x.at != 0.0 && !x.refused) else { return true };
    sign.expires_on - (now as f64) < 60.0
}

fn usable(sign: Option<&LightSignIn>, now: i64) -> bool {
    sign.is_some_and(|s| s.at != 0.0 && !s.refused && ((s.session == Some(true) && s.browser != Some(true)) || s.expires_on - now as f64 >= MIN_LIFE))
}

/// The browser of the account until a sign-in it captured holds MIN_LIFE (ok), Teams signed out (login), or nothing
/// in REFRESH_WAIT (failed); the light agent runs again after it.
pub async fn run_refresh(n: i64) -> Option<&'static str> {
    let begun = now();
    let started: Res<Result<bool, &'static str>> = exclusive(async {
        let Some(s) = appdb::slot_row(n)? else { return Ok(Ok(false)) };
        if s.stopped != 0 || s.light == 0 || busy(n) {
            return Ok(Ok(false));
        }
        if !s.shared_account.is_empty() || light_sign_in(n).is_some_and(|x| x.broker == Some(true)) {
            appdb::end_refresh(n, begun, None)?;
            return Ok(Ok(false));
        }
        appdb::begin_refresh(n, begun)?;
        if let Err(e) = slot_up(n).await {
            report(n, &e);
            if let Err(e) = control::light(n).await {
                report(n, &e);
            }
            appdb::end_refresh(n, now(), Some("failed"))?;
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
    let current = || appdb::slot_row(n).ok().flatten().is_some_and(|s| s.stopped == 0 && s.light != 0);
    let asked = || appdb::slot_row(n).ok().flatten().is_some_and(|s| s.refresh_result == REFRESH_ASKED);
    let mut result: Option<&'static str> = Some("failed");
    let mut signed_out_since = 0;
    let mut owner_seen = begun;
    loop {
        if !current() {
            result = None;
            break;
        }
        let sign = light_sign_in(n);
        if usable(sign.as_ref(), now()) && sign.as_ref().map_or(0.0, |x| x.at) >= begun as f64 {
            result = Some("ok");
            break;
        }
        let h = health(n);
        if h.as_ref().is_some_and(|h| h.ts >= begun as f64 && h.teams == "login") {
            if signed_out_since == 0 {
                signed_out_since = now();
            }
        } else {
            signed_out_since = 0;
        }
        if h.as_ref().is_some_and(|h| h.ts >= begun as f64 && h.in_use) {
            owner_seen = now();
        }
        let wait_sign_in = asked();
        if !wait_sign_in && signed_out_since != 0 && now() - signed_out_since >= SIGNED_OUT {
            result = Some("login");
            break;
        }
        if if wait_sign_in { now() - owner_seen >= SIGN_IN_WAIT } else { now() - begun >= REFRESH_WAIT } {
            if wait_sign_in && signed_out_since != 0 {
                result = Some("login");
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
    loop {
        let ended = exclusive(async {
            let s = appdb::slot_row(n).ok().flatten();
            if s.as_ref().is_some_and(|s| s.stopped == 0 && s.light != 0) {
                if busy(n) {
                    return false;
                }
                if let Err(e) = control::light(n).await {
                    report(n, &e);
                }
            }
            if s.is_some()
                && let Err(e) = appdb::end_refresh(n, now(), result)
            {
                report(n, &e);
            }
            true
        })
        .await;
        if ended {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
    result
}

/// The browser of a light account, started on demand for its owner on the remote desktop: false when it did not
/// start in BROWSER_WAIT seconds.
pub async fn browser_on_demand(n: i64) -> Res<bool> {
    let Some(s) = appdb::slot_row(n)? else { return Ok(true) };
    if s.light == 0 || s.stopped != 0 {
        return Ok(true);
    }
    if s.refreshing == 0 {
        appdb::ask_refresh(n)?;
    }
    let until = now() + BROWSER_WAIT;
    while now() < until {
        match appdb::slot_row(n)? {
            Some(r) if r.light != 0 && r.stopped == 0 && r.refreshing == 0 => {}
            _ => return Ok(true),
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    Ok(false)
}

pub async fn run_due_refreshes() {
    let now = now();
    let Ok(slots) = appdb::list_slots() else { return };
    for s in slots {
        if refresh_due(&s, light_sign_in(s.slot).as_ref(), now) {
            run_refresh(s.slot).await;
        }
    }
}
