//! /api/relay/*: the local relay of an account on another computer, with the token of its account.

use crate::relay::{self, COMMANDS_WAIT_MS, require_relay};
use crate::route;
use crate::server::{Route, take_stream};
use crate::uploads::{file_response, image_type, uploads_dir};
use crate::web::{Req, Res, err, json, now_ms, ok};
use serde_json::{Value, json};

pub fn routes() -> Vec<Route> {
    vec![
        route!("GET", "/api/relay/browser", browser),
        route!("GET", "/api/relay/commands", commands),
        route!("PUT", "/api/relay/files/[file]", files, stream),
        route!("POST", "/api/relay/have", have),
        route!("PUT", "/api/relay/media/[file]", media, stream),
        route!("POST", "/api/relay/push", push),
        route!("POST", "/api/relay/sync", sync),
        route!("GET", "/api/relay/uploads/[file]", uploads),
    ]
}

async fn browser(req: Req) -> Res {
    json(json!({ "slot": require_relay(&req)?.slot }))
}

fn count(v: Option<String>) -> f64 {
    let n = crate::js::number(v.map(Value::String).as_ref().or(Some(&Value::Null)));
    let n = if n.is_nan() { 0.0 } else { n };
    n.floor().max(0.0)
}

/// The commands queued after ?after=<id>, and the chat the app shows when newer than ?vts=<ts>.
async fn commands(req: Req) -> Res {
    let caller = require_relay(&req)?;
    let wait = if req.query_param("wait").as_deref() == Some("0") { 0 } else { COMMANDS_WAIT_MS };
    let (cmds, viewing) = relay::wait_for_relay_commands(&caller, count(req.query_param("after")) as i64, count(req.query_param("vts")), wait).await?;
    let devices = match crate::appdb::slot_owner(caller.slot)? {
        Some(o) => crate::appdb::count_push_subscriptions(&o)?,
        None => 0,
    };
    json(relay::commands_answer(caller.added, cmds, viewing, devices))
}

async fn files(req: Req) -> Res {
    upload(req, "files").await
}

async fn media(req: Req) -> Res {
    upload(req, "media").await
}

async fn upload(req: Req, kind: &str) -> Res {
    let caller = require_relay(&req)?;
    let body = take_stream(&req);
    relay::save_relay_file(&caller, kind, &req.param("file"), body, req.declared_length()).await?;
    ok()
}

async fn have(req: Req) -> Res {
    let caller = require_relay(&req)?;
    let b = relay::relay_json(&req, &relay::have_body(), 4e6)?;
    json(relay::missing_relay_files(caller.slot, &b))
}

async fn push(req: Req) -> Res {
    let caller = require_relay(&req)?;
    let b = relay::relay_json(&req, &relay::push_body(), 1e6)?;
    let sent = relay::relay_push(&caller, &b).await?;
    json(json!({ "ok": true, "sent": sent }))
}

async fn sync(req: Req) -> Res {
    let caller = require_relay(&req)?;
    let b = relay::relay_json(&req, &relay::sync_body(), 64e6)?;
    relay::apply_sync(&caller, &b, now_ms())?;
    ok()
}

/// The image of a sendimage command, left by the app in data/N/uploads.
async fn uploads(req: Req) -> Res {
    let caller = require_relay(&req)?;
    let file = req.param("file");
    let Some(m) = relay::media_name().captures(&file) else { return Err(err(404, "Not found")) };
    file_response(&uploads_dir(caller.slot).join(&file), &[("content-type", image_type(&m[1]).into()), ("cache-control", "private, no-store".into())])
}
