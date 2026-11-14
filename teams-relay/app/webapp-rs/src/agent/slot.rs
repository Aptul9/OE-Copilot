//! One light slot on the environment the supervisor gave it (src/agent/light/slot.ts): runs until its abort.

use super::calls::fork_worker;
use super::config::{Env, load};
use super::devices::AppDb;
use super::host::Abort;
use super::run::{Light, Parts, run_light};
use super::status::StatusCollector;
use super::store::Store;
use super::token::Token;
use super::{info, warn};
use crate::push::{FcmSender, Notifier, NotifierOptions, load_service_account, load_vapid_keys};
use std::rc::Rc;

pub async fn start_slot(_slot: i64, env: Env, abort: Abort) -> Result<(), String> {
    let config = load(&env).map_err(|e| format!("configuration: {e}"))?;
    let store = Store::open(&config.db_path).map_err(|e| e.to_string())?;
    let push = &config.push;
    let vapid = load_vapid_keys(&push.vapid_private_key_file, &push.vapid_app_key_file)?;
    if vapid.is_none() {
        warn("push", "no VAPID private key: push notifications off", &[("file", push.vapid_private_key_file.as_str().into())]);
    }
    let sa = load_service_account(&push.fcm_credentials)?;
    let fcm = sa.is_some();
    let notifier = Notifier::new(NotifierOptions {
        store_file: config.db_path.clone(),
        devices: Box::new(AppDb::new(config.app_db.clone(), config.slot)),
        vapid: vapid.clone(),
        subject: push.vapid_subject.clone(),
        ntfy: push.ntfy.clone(),
        fcm: sa.map(FcmSender::new),
        answerable: config.native_calls,
    });
    // each slot its own connections, as each account had in a process of its own
    let http = super::http::slot_http();
    let token = Rc::new(Token::new(config.light_token_file.clone(), config.shared_broker.clone(), http.clone()));
    let app = Rc::new(AppDb::new(config.app_db.clone(), config.slot));
    let pages = app.clone();
    let status = StatusCollector::new(Box::new(move || pages.status_pages()), config.status_db.clone(), config.slot, 3 * config.light_poll);
    info(
        "light",
        "start",
        &[
            ("slot", config.slot.into()),
            ("poll", config.light_poll.into()),
            ("push", vapid.is_some().into()),
            ("fcm", fcm.into()),
            ("signIn", token.stored().is_some().into()),
        ],
    );
    let light = Light::new(Parts {
        config,
        store,
        notifier: Rc::new(notifier),
        token,
        http,
        calls_wanted: Box::new(move || app.calls_on()),
        status: Some(status),
        spawn: fork_worker(),
    });
    run_light(light, abort).await;
    Ok(())
}
