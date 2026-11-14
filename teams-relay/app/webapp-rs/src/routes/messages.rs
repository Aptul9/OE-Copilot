//! Chats, messages, the Activity feed and the commands of the app; files and images of an account; push devices.

use crate::auth::{require_session, require_slot, require_user};
use crate::commands::{chat_name, command_route, idle_reason, mention_names, mention_parts, message_text, queue};
use crate::route;
use crate::server::Route;
use crate::slotdb::{slot_dir, with_slot};
use crate::uploads::{attachment_name, content_disposition, file_response, image_type, save_image, uploads_dir};
use crate::web::{Req, Res, err, json, now_ms, ok, text};
use serde_json::{Value, json};

pub fn routes() -> Vec<Route> {
    vec![
        route!("GET", "/api/chats", chats),
        route!("GET", "/api/messages", messages),
        route!("GET", "/api/feed", feed),
        route!("GET", "/api/activity", activity),
        route!("POST", "/api/activity/refresh", activity_refresh),
        route!("POST", "/api/send", send),
        route!("POST", "/api/reply", reply),
        route!("POST", "/api/edit", edit),
        route!("POST", "/api/delete", delete),
        route!("POST", "/api/undodelete", undodelete),
        route!("POST", "/api/react", react),
        route!("POST", "/api/open", open),
        route!("POST", "/api/resync", resync),
        route!("POST", "/api/recheck", recheck),
        route!("POST", "/api/members", members),
        route!("POST", "/api/download", download),
        route!("POST", "/api/sendimage", sendimage),
        route!("GET", "/api/cmd/[id]", cmd),
        route!("POST", "/api/viewing", viewing),
        route!("GET", "/files/[file]", files),
        route!("GET", "/media/[file]", media),
        route!("POST", "/api/push/subscribe", push_subscribe),
        route!("POST", "/api/push/fcm", push_fcm),
        route!("DELETE", "/api/push/fcm", push_fcm_delete),
        route!("GET", "/api/vapidkey", vapidkey),
        route!("GET", "/healthz", healthz),
        route!("GET", "/api/oauth/clients", oauth_clients),
        route!("DELETE", "/api/oauth/clients/[id]", oauth_client_delete),
    ]
}

async fn chats(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    json(Value::Array(with_slot(slot, |r| Ok(r.chats()))?))
}

async fn messages(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let name = chat_name(req.query_param("name").map(Value::String).as_ref())?;
    json(Value::Array(with_slot(slot, |r| Ok(r.messages(&name)))?))
}

async fn feed(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    json(Value::Array(with_slot(slot, |r| Ok(r.feed()))?.into_iter().map(Value::Object).collect()))
}

async fn activity(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    json(with_slot(slot, |r| Ok(r.activity()))?)
}

fn queued(slot: i64, kind: &str) -> Res {
    json(json!({ "ok": true, "id": queue(slot, kind, "", "")? }))
}

async fn activity_refresh(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    queued(slot, "activity")
}

async fn resync(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    queued(slot, "resync")
}

async fn recheck(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    queued(slot, "recheck")
}

/// {name, text, mentions?}: a message that still tags someone goes in parts, otherwise as plain text.
async fn send(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let b = req.json()?;
    let name = chat_name(b.get("name"))?;
    let t = message_text(b.get("text"))?;
    let parts = mention_parts(&t, &mention_names(b.get("mentions"))?);
    if !parts.iter().any(|p| p.get("mention").is_some()) {
        return json(json!({ "ok": true, "id": queue(slot, "send", &name, &t)? }));
    }
    json(json!({ "ok": true, "id": queue(slot, "sendmentions", &name, &crate::js::stringify(&json!({ "parts": parts })))? }))
}

async fn reply(req: Req) -> Res {
    command_route("reply", req).await
}
async fn edit(req: Req) -> Res {
    command_route("edit", req).await
}
async fn delete(req: Req) -> Res {
    command_route("delete", req).await
}
async fn undodelete(req: Req) -> Res {
    command_route("undodelete", req).await
}
async fn react(req: Req) -> Res {
    command_route("react", req).await
}
async fn open(req: Req) -> Res {
    command_route("open", req).await
}

const MEMBERS_MAX_AGE: f64 = 3600.0;

/// {name}: the people of a chat for the @ of the compose box; older than an hour, the agent reads them again.
async fn members(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let name = chat_name(req.json()?.get("name"))?;
    let (m, pending, me) = with_slot(slot, |r| Ok((r.members(&name)?, r.pending_command("members", &name), r.identity().name)))?;
    let names: Vec<Value> = m["names"].as_array().cloned().unwrap_or_default().into_iter().filter(|n| n.as_str() != Some(me.as_str())).collect();
    let ts = m["ts"].as_f64().unwrap_or(0.0);
    if now_ms() as f64 / 1000.0 - ts < MEMBERS_MAX_AGE || idle_reason(slot)?.is_some() {
        return json(json!({ "names": names }));
    }
    let id = if pending != 0 { pending } else { queue(slot, "members", &name, "")? };
    json(json!({ "names": names, "id": id }))
}

/// SharePoint and OneDrive attachments only: the agent downloads them with the Teams session of the account.
async fn download(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let b = req.json()?;
    let url = text(b.get("url"), "url", 4000)?;
    let re = regex::Regex::new(r"^https://[a-z0-9-]+\.sharepoint\.com/\S+$").expect("fixed");
    if !re.is_match(&url) {
        return Err(err(400, "Unsupported link"));
    }
    let name = match b.get("name") {
        None | Some(Value::Null) => "file".to_string(),
        v => text(v, "name", 300)?,
    };
    json(json!({ "ok": true, "id": queue(slot, "download", &url, &crate::js::stringify(&json!({ "name": name })))? }))
}

/// A part of a multipart/form-data body: its name, file name and bytes.
struct Part {
    name: String,
    file: Option<String>,
    data: Vec<u8>,
}

fn multipart(content_type: &str, body: &[u8]) -> Option<Vec<Part>> {
    let boundary = content_type.split(';').map(str::trim).find_map(|p| p.strip_prefix("boundary="))?.trim_matches('"').to_string();
    let delim = format!("--{boundary}").into_bytes();
    let find = |hay: &[u8], needle: &[u8], from: usize| hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from);
    let mut parts = vec![];
    let mut at = find(body, &delim, 0)? + delim.len();
    loop {
        if body.get(at..at + 2) == Some(b"--") {
            return Some(parts);
        }
        at += 2;
        let head_end = find(body, b"\r\n\r\n", at)?;
        let head = String::from_utf8_lossy(&body[at..head_end]).into_owned();
        let next = find(body, &[b"\r\n".as_slice(), &delim].concat(), head_end + 4)?;
        let data = body[head_end + 4..next].to_vec();
        let disp = head.lines().find(|l| l.to_ascii_lowercase().starts_with("content-disposition"))?.to_string();
        let attr = |k: &str| {
            let re = regex::Regex::new(&format!(r#"(?i);\s*{k}="([^"]*)""#)).expect("fixed");
            re.captures(&disp).map(|c| c[1].to_string())
        };
        parts.push(Part { name: attr("name").unwrap_or_default(), file: attr("filename"), data });
        at = next + 2 + delim.len();
    }
}

/// multipart/form-data: name (chat), file (image), text (caption, optional).
async fn sendimage(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    if req.declared_length().unwrap_or(0.0) > (crate::uploads::MAX_UPLOAD + 100_000) as f64 {
        return Err(err(413, "Image larger than 10 MB"));
    }
    if let Some(why) = idle_reason(slot)? {
        return Err(err(409, why));
    }
    let ct = req.header("content-type").unwrap_or_default();
    let form = if ct.to_ascii_lowercase().starts_with("multipart/form-data") { multipart(&ct, &req.body) } else { None };
    let Some(form) = form else { return Err(err(400, "Invalid form")) };
    let field = |n: &str| form.iter().find(|p| p.name == n);
    let name = chat_name(field("name").filter(|p| p.file.is_none()).map(|p| Value::String(String::from_utf8_lossy(&p.data).into_owned())).as_ref())?;
    let caption = match field("text") {
        None => String::new(),
        Some(p) if p.file.is_some() => return Err(err(400, "Missing text")),
        Some(p) => text(Some(&Value::String(String::from_utf8_lossy(&p.data).into_owned())), "text", 20000)?,
    };
    let Some(file) = field("file").filter(|p| p.file.is_some() && !p.data.is_empty()) else { return Err(err(400, "Missing image")) };
    if file.data.len() > crate::uploads::MAX_UPLOAD {
        return Err(err(413, "Image larger than 10 MB"));
    }
    let saved = save_image(slot, &file.data)?;
    match queue(slot, "sendimage", &name, &crate::js::stringify(&json!({ "file": saved, "text": caption }))) {
        Ok(id) => json(json!({ "ok": true, "id": id })),
        Err(e) => {
            let _ = std::fs::remove_file(uploads_dir(slot).join(&saved));
            Err(e)
        }
    }
}

/// Outcome of a queued command: pending, done or failed.
async fn cmd(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let Some(id) = crate::auth::int_param(&req.param("id")) else { return Err(err(404, "Command not found")) };
    let status = with_slot(slot, |r| Ok(r.command_status(id)))?;
    json(status.ok_or_else(|| err(404, "Command not found"))?)
}

/// The chat on screen in the app ({chat}), or the one it stopped showing ({left}).
async fn viewing(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let b = req.json()?;
    if b.contains_key("left") {
        let chat = chat_name(b.get("left"))?;
        with_slot(slot, |r| r.leave_viewing(&chat))?;
    } else {
        let chat = chat_name(b.get("chat"))?;
        with_slot(slot, |r| r.mark_viewing(&chat))?;
    }
    ok()
}

async fn files(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let file = req.param("file");
    if !crate::relay::file_name().is_match(&file) {
        return Err(err(404, "Not found"));
    }
    let name = attachment_name(req.query_param("name").as_deref());
    file_response(
        &slot_dir(slot).join("files").join(&file),
        &[
            ("content-type", "application/octet-stream".into()),
            ("content-disposition", content_disposition(&name)),
            ("cache-control", "private, no-store".into()),
        ],
    )
}

async fn media(req: Req) -> Res {
    let (_, slot) = require_slot(&req)?;
    let file = req.param("file");
    let Some(m) = crate::relay::media_name().captures(&file) else { return Err(err(404, "Not found")) };
    file_response(
        &slot_dir(slot).join("media").join(&file),
        &[("content-type", image_type(&m[1]).into()), ("cache-control", "private, max-age=31536000, immutable".into())],
    )
}

/// A Web Push subscription as the browser gives it: the device gets the notifications of every account of its user.
async fn push_subscribe(req: Req) -> Res {
    let u = require_user(&req)?;
    let b = req.json()?;
    let endpoint = match b.get("endpoint") {
        Some(Value::String(e)) if e.starts_with("https://") && e.encode_utf16().count() <= 2000 => e.clone(),
        _ => return Err(err(400, "Invalid subscription endpoint")),
    };
    let key_re = regex::Regex::new(r"^[A-Za-z0-9_-]{16,200}=*$").expect("fixed");
    let keys = b.get("keys");
    let (p256dh, auth) = (keys.and_then(|k| k.get("p256dh")).and_then(Value::as_str), keys.and_then(|k| k.get("auth")).and_then(Value::as_str));
    let (Some(p256dh), Some(auth)) = (p256dh.filter(|k| key_re.is_match(k)), auth.filter(|k| key_re.is_match(k))) else {
        return Err(err(400, "Invalid subscription keys"));
    };
    let kind = if b.get("device") == Some(&json!("mobile")) { "mobile" } else { "desktop" };
    let sub = json!({ "endpoint": endpoint, "keys": { "p256dh": p256dh, "auth": auth }, "kind": kind });
    crate::appdb::save_push_subscription(&u.id, &endpoint, &crate::js::stringify(&sub))?;
    ok()
}

fn fcm_token(v: Option<&Value>) -> Res<String> {
    let t = text(v, "token", 4096)?;
    if !regex::Regex::new(r"^[\w:-]{20,}$").expect("fixed").is_match(&t) || !t.is_ascii() {
        return Err(err(400, "Not an FCM token"));
    }
    Ok(t)
}

/// A phone of the Android app registers with the session cookie of its page: notifications for as long as that
/// session lasts, sealed with the key of the answer.
async fn push_fcm(req: Req) -> Res {
    let (u, session) = require_session(&req)?;
    let b = req.json()?;
    let name = match b.get("name") {
        Some(Value::String(n)) => String::from_utf16_lossy(&n.encode_utf16().take(80).collect::<Vec<u16>>()),
        _ => String::new(),
    };
    let token = fcm_token(b.get("token"))?;
    json(json!({ "key": crate::appdb::save_fcm_device(&u.id, &token, &name, &session)? }))
}

async fn push_fcm_delete(req: Req) -> Res {
    let b = req.json()?;
    crate::appdb::forget_fcm_device(&fcm_token(b.get("token"))?)?;
    ok()
}

/// The public key for the push subscription of the browser; "" without VAPID keys.
async fn vapidkey(_req: Req) -> Res {
    let file = crate::config::push().map(|p| p.vapid_app_key_file).unwrap_or_else(|_| "/vapid/appkey.txt".into());
    let key = std::fs::read_to_string(file).map(|k| k.trim().to_string()).unwrap_or_default();
    json(json!({ "key": key }))
}

async fn healthz(_req: Req) -> Res {
    ok()
}

/// No OAuth for MCP clients on this server: none allowed, nothing to revoke.
async fn oauth_clients(req: Req) -> Res {
    require_user(&req)?;
    json(json!({ "clients": [] }))
}

async fn oauth_client_delete(req: Req) -> Res {
    require_user(&req)?;
    ok()
}
