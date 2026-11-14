//! The accounts of a user as the app shows them (src/lib/accounts.ts): identity, Teams state, unread counts, mode.

use crate::appdb::{self, Slot};
use crate::slotdb::with_slot_or;
use crate::state::{health_of, key};
use crate::web::{Res, err, now_ms};
use serde_json::{Map, Value, json};

pub fn save_mailbox_identity(slot: i64, account: &str) -> Res<()> {
    let folder = crate::config::data_dir().join(slot.to_string());
    std::fs::create_dir_all(&folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700));
    }
    let file = folder.join("mailbox-identity.json");
    let tmp = folder.join("mailbox-identity.json.new");
    std::fs::write(&tmp, crate::js::stringify(&Value::String(account.to_string())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(tmp, file)?;
    Ok(())
}

fn mailbox_identity(slot: i64) -> String {
    let file = crate::config::data_dir().join(slot.to_string()).join("mailbox-identity.json");
    let Ok(text) = std::fs::read_to_string(file) else { return String::new() };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::String(e)) if regex::Regex::new(r"^[^\s@]+@[^\s@]+$").expect("fixed").is_match(&e) => e.to_lowercase(),
        _ => String::new(),
    }
}

/// upSince: the grace of a browser still starting runs from the last start.
pub fn up_since(s: &Slot) -> i64 {
    s.added.max(s.started).max(s.checking).max(s.refreshing)
}

/// A stopped account is grey, whatever its agent wrote; so is an account checked every N hours between two checks.
pub fn slot_health(h: Map<String, Value>, s: &Slot) -> Map<String, Value> {
    let mut h = h;
    let grey = |h: &mut Map<String, Value>, teams: &str| {
        crate::js::set(h, "teams", Some(json!(teams)));
        crate::js::set(h, "agent", Some(json!("stopped")));
        crate::js::set(h, "watcher", Some(json!("stopped")));
        crate::js::set(h, "overall", Some(json!("grey")));
    };
    if s.stopped != 0 {
        grey(&mut h, "stopped");
    } else if s.check_every != 0 && s.checking == 0 {
        grey(&mut h, "checked");
    }
    h
}

pub fn no_health(s: &Slot) -> Map<String, Value> {
    health_of(&json!({}), up_since(s), now_ms() as f64 / 1000.0)
}

pub fn account_summary(s: &Slot) -> Res<Value> {
    struct Read {
        me: crate::state::Identity,
        health: Map<String, Value>,
        unread: i64,
        unread_activity: Option<Vec<Value>>,
        missed_calls: Option<Vec<Value>>,
        activity_ids: Option<Vec<Value>>,
        link: crate::state::RelayLink,
        call_audio: bool,
        presence: Value,
    }
    let d = with_slot_or(
        s.slot,
        |r| {
            Ok(Some(Read {
                me: r.identity(),
                health: r.health(up_since(s)),
                unread: r.unread_count(),
                unread_activity: r.unread_activity(),
                missed_calls: r.missed_calls(),
                activity_ids: r.activity_ids(),
                link: r.relay_link(),
                call_audio: r.state(key::CALL_AUDIO, json!(0)) == json!(1),
                presence: r.state(key::PRESENCE, json!("")),
            }))
        },
        None,
    )?;
    let health = slot_health(d.as_ref().map(|d| d.health.clone()).unwrap_or_else(|| no_health(s)), s);
    let me = d.as_ref().map(|d| d.me.clone());
    let s_or = |v: Option<&Value>| match v {
        None | Some(Value::Null) => String::new(),
        v => crate::js::string(v),
    };
    let ids = |v: Option<Vec<Value>>| v.map(Value::Array).unwrap_or(Value::Null);
    let email = me.as_ref().map(|m| m.email.clone()).filter(|e| !e.is_empty()).unwrap_or_else(|| mailbox_identity(s.slot));
    Ok(json!({
        "slot": s.slot,
        "name": me.as_ref().map(|m| m.name.clone()).unwrap_or_default(),
        "email": email,
        "tenant": me.as_ref().map(|m| m.tenant.clone()).unwrap_or_default(),
        "av": me.as_ref().map(|m| m.av.clone()).unwrap_or_default(),
        "presence": d.as_ref().map(|d| d.presence.clone()).unwrap_or(json!("")),
        "teams": s_or(health.get("teams")),
        "overall": s_or(health.get("overall")),
        "stopped": s.stopped != 0,
        "unread": d.as_ref().map_or(0, |d| d.unread),
        "unreadActivity": ids(d.as_ref().and_then(|d| d.unread_activity.clone())),
        "missedCalls": ids(d.as_ref().and_then(|d| d.missed_calls.clone())),
        "activityIds": ids(d.as_ref().and_then(|d| d.activity_ids.clone())),
        "added": s.added,
        "desktop": if s.relay != 0 || (s.light != 0 && s.refreshing == 0) { String::new() } else { crate::config::desktop_url_of(s.slot) },
        "checkEvery": s.check_every,
        "checked": s.checked,
        "checkResult": s.check_result,
        "nextCheck": s.check_due,
        "checking": s.checking != 0,
        "relay": s.relay != 0,
        "host": d.as_ref().map(|d| d.link.host.clone()).unwrap_or_default(),
        "relaySeen": d.as_ref().map(|d| d.link.seen.clone()).unwrap_or(json!(0)),
        "callAudio": (s.relay != 0 || s.light != 0) && d.as_ref().is_some_and(|d| d.call_audio),
        "light": s.light != 0,
        "calls": s.calls != 0,
        "notifyMobile": s.notify_mobile != 0,
        "notifyDesktop": s.notify_desktop != 0,
        "refreshing": s.refreshing != 0,
        "shared": s.shared_account,
    }))
}

pub fn accounts_of(user_id: &str) -> Res<Value> {
    let list: Vec<Value> = appdb::slots_of(user_id)?.iter().map(account_summary).collect::<Res<_>>()?;
    Ok(json!({ "accounts": list, "max": crate::config::accounts_per_user() }))
}

pub fn health_for(user_id: &str, slot: i64) -> Res<Value> {
    let s = appdb::slots_of(user_id)?.into_iter().find(|x| x.slot == slot).ok_or_else(|| err(404, "Account not found"))?;
    let h = with_slot_or(slot, |r| Ok(r.health(up_since(&s))), no_health(&s))?;
    let mut h = slot_health(h, &s);
    crate::js::set(&mut h, "push_subs", Some(json!(appdb::count_push_subscriptions(user_id)?)));
    Ok(Value::Object(h))
}
