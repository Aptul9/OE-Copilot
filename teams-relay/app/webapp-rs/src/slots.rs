//! Adding, removing, stopping and starting the accounts, their modes, and the keep-alive loop (src/lib/slots.ts). All of
//! it, the start and stop of a check and the browser runs of light accounts, one at a time: exclusive().

use crate::appdb::{self, REFRESH_ASKED};
use crate::control;
use crate::slotdb::with_slot_or;
use crate::web::{Res, err, now_s};
use std::future::Future;

static QUEUE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One queue for the process: a slot just stopped is not started again by a request that waited behind the stop.
pub async fn exclusive<T>(f: impl Future<Output = T>) -> T {
    let _turn = QUEUE.lock().await;
    f.await
}

pub const SHARED_NO_BROWSER: &str =
    "This Teams account uses a shared Microsoft account: it runs without a browser. Its sign-in is renewed from the Account window of the calendar";

pub const CHECK_INTERVALS: [i64; 3] = [3600, 7200, 14400];

pub async fn slot_up(n: i64) -> Res<()> {
    control::start(n).await
}

pub async fn slot_down(n: i64) -> Res<()> {
    control::stop(n).await
}

/// A start the supervisor did not confirm in time may still be under way: a stop queued after it ends it.
async fn start_or_undo(n: i64) -> Res<()> {
    if let Err(e) = slot_up(n).await {
        let _ = slot_down(n).await;
        return Err(e);
    }
    Ok(())
}

/// The next check starts from what it finds: nothing it would compare with is left from before.
fn forget_last_check(n: i64) -> Res<()> {
    with_slot_or(n, |r| r.forget_last_check(), ())
}

/// Browser profile and agent data of the slot, with the slot stopped: the supervisor deletes config/N, then data/N.
pub async fn wipe_slot(n: i64) -> Res<()> {
    control::wipe(n).await?;
    remove_dir(&crate::config::data_dir().join(n.to_string()))
}

pub fn remove_dir(p: &std::path::Path) -> Res<()> {
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A new account is light: its browser runs now for the first sign-in, as a sign-in run asked by its owner.
pub fn start_light(n: i64) -> Res<()> {
    appdb::set_light(n, true)?;
    appdb::ask_refresh(n)
}

pub async fn add_account(user_id: &str) -> Res<i64> {
    exclusive(async {
        let n = appdb::claim_slot(user_id, crate::config::accounts_per_user())?;
        let r = async {
            let _ = slot_down(n).await;
            wipe_slot(n).await?;
            slot_up(n).await?;
            start_light(n)
        }
        .await;
        if let Err(e) = r {
            appdb::release_slot(n)?;
            return Err(e);
        }
        Ok(n)
    })
    .await
}

/// New account on another computer: first free slot, its empty database, and the token its relay joins with.
pub async fn add_relay_account(user_id: &str) -> Res<(i64, String)> {
    exclusive(async {
        let n = appdb::claim_slot(user_id, crate::config::accounts_per_user())?;
        let r = (|| {
            crate::relay::forget_relay(n);
            crate::relay::create_relay_slot(n)?;
            let token = crate::relay::new_relay_token();
            appdb::set_relay_token(n, &crate::relay::relay_digest(&token))?;
            Ok(token)
        })();
        match r {
            Ok(token) => Ok((n, token)),
            Err(e) => {
                appdb::release_slot(n)?;
                Err(e)
            }
        }
    })
    .await
}

pub async fn renew_relay_token(n: i64) -> Res<String> {
    exclusive(async {
        let s = appdb::slot_row(n)?.ok_or_else(|| err(404, "Account not found"))?;
        if s.relay == 0 {
            return Err(err(409, "This Teams account runs on this server: it has no relay token"));
        }
        let token = crate::relay::new_relay_token();
        appdb::set_relay_token(n, &crate::relay::relay_digest(&token))?;
        Ok(token)
    })
    .await
}

pub async fn remove_account(n: i64) -> Res<()> {
    exclusive(async {
        if appdb::slot_row(n)?.is_some_and(|s| s.relay != 0) {
            crate::relay::forget_relay(n);
            remove_dir(&crate::config::data_dir().join(n.to_string()))?;
        } else {
            slot_down(n).await?;
            wipe_slot(n).await?;
        }
        appdb::release_slot(n)
    })
    .await
}

pub async fn set_account_running(n: i64, running: bool) -> Res<()> {
    exclusive(async {
        let s = appdb::slot_row(n)?.ok_or_else(|| err(404, "Account not found"))?;
        if s.relay != 0 {
            return Err(err(409, crate::relay::ON_ANOTHER_COMPUTER));
        }
        if running && !s.shared_account.is_empty() {
            return Err(err(409, SHARED_NO_BROWSER));
        }
        if !running {
            slot_down(n).await?;
        } else if s.light != 0 {
            control::light(n).await?;
        } else if s.check_every == 0 {
            start_or_undo(n).await?;
        }
        appdb::set_slot_stopped(n, !running)?;
        if running {
            forget_last_check(n)?;
        }
        if running && s.check_every != 0 {
            appdb::ask_check(n)?;
        }
        Ok(())
    })
    .await
}

pub async fn set_check_mode(n: i64, every: i64) -> Res<()> {
    if every != 0 && !CHECK_INTERVALS.contains(&every) {
        return Err(err(400, format!("checkEvery must be 0 or one of {}", CHECK_INTERVALS.map(|x| x.to_string()).join(", "))));
    }
    let now = now_s();
    exclusive(async {
        let s = appdb::slot_row(n)?.ok_or_else(|| err(404, "Account not found"))?;
        if s.relay != 0 {
            return Err(err(409, crate::relay::ON_ANOTHER_COMPUTER));
        }
        if !s.shared_account.is_empty() {
            return Err(err(409, SHARED_NO_BROWSER));
        }
        if every == 0 {
            start_or_undo(n).await?;
        } else if s.stopped == 0 && s.checking == 0 {
            slot_down(n).await?;
        }
        if s.light != 0 {
            appdb::set_light(n, false)?;
        }
        appdb::set_check_every(n, every, now)?;
        forget_last_check(n)?;
        if every == 0 || s.stopped != 0 {
            appdb::set_slot_stopped(n, false)?;
        }
        if every != 0 && s.stopped != 0 {
            appdb::ask_check(n)?;
        }
        Ok(())
    })
    .await
}

pub async fn set_light_mode(n: i64) -> Res<()> {
    exclusive(async {
        let s = appdb::slot_row(n)?.ok_or_else(|| err(404, "Account not found"))?;
        if s.relay != 0 {
            return Err(err(409, crate::relay::ON_ANOTHER_COMPUTER));
        }
        if !s.shared_account.is_empty() {
            return Err(err(409, SHARED_NO_BROWSER));
        }
        if s.light != 0 && s.stopped == 0 {
            return Ok(());
        }
        appdb::set_light(n, true)?;
        if s.stopped != 0 {
            appdb::set_slot_stopped(n, false)?;
        }
        forget_last_check(n)?;
        if s.stopped != 0 || s.check_every != 0 {
            control::light(n).await?;
        } else {
            appdb::ask_refresh(n)?;
        }
        Ok(())
    })
    .await
}

/// The shared Microsoft account slot n runs on, "" for a slot with its own sign-in.
pub fn shared_account_of(n: i64) -> Res<String> {
    Ok(appdb::shared_binding(n)?.map(|(a, _)| a).unwrap_or_default())
}

/// Owned slots must be running unless their owner stopped them or they run only while checked; a light account runs
/// its light agent unless its browser runs now, or is about to.
pub async fn keep_slots_up_tick() {
    exclusive(async {
        let slots = match appdb::list_slots() {
            Ok(s) => s,
            Err(e) => return eprintln!("slots: {e}"),
        };
        for s in slots {
            if s.stopped != 0 || s.check_every != 0 || s.relay != 0 || s.refreshing != 0 || (s.light != 0 && s.refresh_result == REFRESH_ASKED) {
                continue;
            }
            let r = if s.light != 0 { control::light(s.slot).await } else { slot_up(s.slot).await };
            if let Err(e) = r {
                eprintln!("slot {}: {}", s.slot, detail(&e));
            }
        }
    })
    .await
}

/// The message of an error, as e.message reads.
pub fn detail(e: &crate::web::Error) -> String {
    match e {
        crate::web::Error::Http(_, d, _) => d.clone(),
        other => other.to_string(),
    }
}

pub fn keep_slots_up() {
    tokio::task::spawn_local(async {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            keep_slots_up_tick().await;
        }
    });
}
