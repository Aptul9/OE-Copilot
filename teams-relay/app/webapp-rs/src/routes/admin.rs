//! /api/admin/users: the users and the slots they own, for administrators (the Better Auth admin plugin underneath).

use crate::appdb;
use crate::auth::rows::{self, iso_ms};
use crate::auth::{PASSWORD_MAX, PASSWORD_MIN, crypto, require_admin};
use crate::route;
use crate::server::Route;
use crate::web::{Req, Res, err, json, now_ms, ok, text};
use serde_json::{Value, json};

pub fn routes() -> Vec<Route> {
    vec![
        route!("GET", "/api/admin/users", list),
        route!("POST", "/api/admin/users", create),
        route!("DELETE", "/api/admin/users/[id]", delete),
        route!("POST", "/api/admin/users/[id]/password", password),
        route!("DELETE", "/api/admin/users/[id]/sessions", sessions),
    ]
}

/// Users with the slots they own. Administrators see who owns a slot, never its chats.
async fn list(req: Req) -> Res {
    require_admin(&req)?;
    let users = rows::list_users(1000)?;
    let slots = appdb::list_slots()?;
    let list: Vec<Value> = users
        .iter()
        .map(|u| {
            json!({
                "id": u.id,
                "name": u.name,
                "email": u.email,
                "role": u.role.clone().unwrap_or_else(|| "user".into()),
                "banned": u.banned == Some(true),
                "managed": crate::config::is_env_admin(Some(&u.email)),
                "createdAt": u.created_at.map_or(Value::Null, |m| Value::String(iso_ms(m))),
                "slots": slots.iter().filter(|s| s.owner_id == u.id).map(|s| s.slot).collect::<Vec<_>>(),
            })
        })
        .collect();
    json(json!({ "users": list, "used": slots.len() }))
}

/// The Better Auth answer of an admin call as route() turns it into {"detail"}.
fn refused(status: u16, message: &str) -> crate::web::Error {
    err(status, message)
}

/// createUser of the admin plugin: email lowercased and checked, one user per email, a credential account.
pub async fn create_user(email: &str, name: &str, password: &str, role: &str) -> Res<String> {
    let email = email.to_lowercase();
    if !crate::auth::routes::email_ok(&email) {
        return Err(refused(400, "Invalid email"));
    }
    if password.encode_utf16().count() > PASSWORD_MAX {
        return Err(refused(400, "Password too long"));
    }
    if rows::user_by_email(&email)?.is_some() {
        return Err(refused(400, "User already exists. Use another email."));
    }
    let now = now_ms();
    let u = rows::create_user(&email, name, role, now)?;
    if !password.is_empty() {
        let hash = crypto::hash_password_async(password.to_string()).await;
        rows::link_account(&u.id, &hash, now)?;
    }
    Ok(u.id)
}

async fn create(req: Req) -> Res {
    require_admin(&req)?;
    let b = req.json()?;
    let role = if b.get("role") == Some(&json!("admin")) { "admin" } else { "user" };
    let password = text(b.get("password"), "password", PASSWORD_MAX)?;
    if password.encode_utf16().count() < PASSWORD_MIN {
        return Err(err(400, format!("Password: at least {PASSWORD_MIN} characters")));
    }
    let email = text(b.get("email"), "email", 254)?.trim().to_string();
    let name = text(b.get("name"), "name", 100)?.trim().to_string();
    let id = create_user(&email, &name, &password, role).await?;
    json(json!({ "ok": true, "id": id }))
}

/// Deleting a user signs their Teams accounts out first: slots stopped and wiped, devices forgotten.
async fn delete(req: Req) -> Res {
    let admin = require_admin(&req)?;
    let id = req.param("id");
    if id == admin.id {
        return Err(err(400, "You cannot delete yourself"));
    }
    for s in appdb::slots_of(&id)? {
        crate::slots::remove_account(s.slot).await?;
    }
    appdb::delete_push_subscriptions_of(&id)?;
    if rows::user_by_id(&id)?.is_none() {
        return Err(refused(404, "User not found"));
    }
    rows::delete_user_sessions(&id)?;
    rows::delete_user(&id)?;
    ok()
}

/// New password set by an administrator; every device of the user signs in again.
async fn password(req: Req) -> Res {
    require_admin(&req)?;
    let id = req.param("id");
    let password = text(req.json()?.get("password"), "password", PASSWORD_MAX)?;
    if password.encode_utf16().count() < PASSWORD_MIN {
        return Err(err(400, format!("Password: at least {PASSWORD_MIN} characters")));
    }
    // the hook of src/lib/auth.ts: the administrator of .env changes its password in .env only
    let user = rows::user_by_id(&id)?;
    if crate::config::is_env_admin(user.as_ref().map(|u| u.email.as_str())) {
        return Err(refused(403, crate::config::ENV_PASSWORD_MESSAGE));
    }
    let Some(user) = user else { return Err(refused(404, "User not found")) };
    let hash = crypto::hash_password_async(password).await;
    let now = now_ms();
    if rows::password_of(&user.id)?.is_some() {
        rows::update_password(&user.id, &hash, now)?;
    } else {
        rows::link_account(&user.id, &hash, now)?;
    }
    rows::delete_user_sessions(&user.id)?;
    ok()
}

async fn sessions(req: Req) -> Res {
    require_admin(&req)?;
    rows::delete_user_sessions(&req.param("id"))?;
    ok()
}
