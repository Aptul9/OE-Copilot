//! Users, sessions and /api/auth/* as Better Auth 1.7 with the admin plugin served them (src/lib/auth.ts), on the same
//! tables and cookies: a session signed in on the Next.js app stays signed in here.

pub mod cookie;
pub mod crypto;
pub mod limit;
pub mod routes;
pub mod rows;

use crate::web::{Req, Res, err, now_ms};
use rows::{Session, User};

pub const PASSWORD_MIN: usize = 10;
pub const PASSWORD_MAX: usize = 256;

/// A day: past it, a session is renewed at its next read (session.updateAge).
pub const ONE_DAY_S: i64 = 60 * 60 * 24;

/// The user of a session, as the routes see it.
#[derive(Clone, Debug)]
pub struct SessionUser {
    pub id: String,
    pub email: String,
    pub name: String,
    pub role: Option<String>,
}

impl SessionUser {
    pub fn of(u: &User) -> Self {
        SessionUser { id: u.id.clone(), email: u.email.clone(), name: u.name.clone(), role: u.role.clone() }
    }
    pub fn is_admin(&self) -> bool {
        self.role.as_deref() == Some("admin")
    }
    /// user.role ?? "user"
    pub fn role_or_user(&self) -> String {
        self.role.clone().unwrap_or_else(|| "user".into())
    }
}

/// getSession on a cookie header, as auth.api.getSession does it on the server: the session renewed in the database
/// when older than a day, no cookie sent back. An expired session is deleted.
pub fn read_session(cookie_header: Option<&str>, no_renew: bool) -> Res<Option<(Session, User)>> {
    let secret = crate::config::auth_secret();
    let now = now_ms();
    let Some(token) = cookie::signed(cookie_header, &cookie::name(cookie::SESSION), &secret) else { return Ok(None) };
    let dont_remember = cookie::signed(cookie_header, &cookie::name(cookie::DONT_REMEMBER), &secret).is_some();
    let Some((s, u)) = rows::find_session(&token)? else { return Ok(None) };
    if s.expires_at < now {
        rows::delete_session(&token)?;
        return Ok(None);
    }
    if dont_remember || no_renew {
        return Ok(Some((s, u)));
    }
    if s.expires_at - cookie::EXPIRES_S * 1000 + ONE_DAY_S * 1000 <= now {
        return Ok(rows::renew_session(&s.token, now + cookie::EXPIRES_S * 1000, now)?.map(|r| (r, u)));
    }
    Ok(Some((s, u)))
}

/// currentUser of src/lib/session.ts: the user of the session cookie, None without a valid one.
pub fn current_user(req: &Req) -> Res<Option<SessionUser>> {
    Ok(read_session(req.header("cookie").as_deref(), false)?.map(|(_, u)| SessionUser::of(&u)))
}

pub fn require_user(req: &Req) -> Res<SessionUser> {
    current_user(req)?.ok_or_else(|| err(401, "Not signed in"))
}

/// The user and the id of the session of a request.
pub fn require_session(req: &Req) -> Res<(SessionUser, String)> {
    let (s, u) = read_session(req.header("cookie").as_deref(), false)?.ok_or_else(|| err(401, "Not signed in"))?;
    Ok((SessionUser::of(&u), s.id))
}

pub fn require_admin(req: &Req) -> Res<SessionUser> {
    let u = require_user(req)?;
    if !u.is_admin() {
        return Err(err(403, "Administrators only"));
    }
    Ok(u)
}

/// Per-account requests: ?a=N must be a slot of the user, the first owned one when missing.
pub fn require_slot(req: &Req) -> Res<(SessionUser, i64)> {
    let u = require_user(req)?;
    let owned: Vec<i64> = crate::appdb::slots_of(&u.id)?.iter().map(|s| s.slot).collect();
    let slot = pick_slot(&owned, req.query_param("a").as_deref())?;
    Ok((u, slot))
}

/// pickSlot: the ?a=N parameter, or the first owned slot; a slot of someone else answers like one that does not exist.
pub fn pick_slot(owned: &[i64], param: Option<&str>) -> Res<i64> {
    let Some(p) = param.filter(|p| !p.is_empty()) else {
        return owned.first().copied().ok_or_else(|| err(404, "Account not found"));
    };
    let n = p.bytes().all(|b| b.is_ascii_digit()).then(|| p.parse::<i64>().ok()).flatten();
    match n {
        Some(n) if owned.contains(&n) => Ok(n),
        _ => Err(err(404, "Account not found")),
    }
}

/// Number() of a path parameter, the integer it is or None (Number.isInteger).
pub fn int_param(p: &str) -> Option<i64> {
    let f = crate::js::number(Some(&serde_json::Value::String(p.to_string())));
    (f.is_finite() && f == f.trunc() && f.abs() < 9e15).then_some(f as i64)
}

/// requireAccount: the owner, or an administrator, of /api/accounts/N.
pub fn require_account(req: &Req, slot_param: &str) -> Res<i64> {
    let u = require_user(req)?;
    let n = int_param(slot_param);
    let owner = match n {
        Some(n) => crate::appdb::slot_owner(n)?,
        None => None,
    };
    match (n, owner) {
        (Some(n), Some(o)) if o == u.id || u.is_admin() => Ok(n),
        _ => Err(err(404, "Account not found")),
    }
}

/// ownedSlot: the row of an account of this user named by a path parameter.
pub fn owned_slot(user_id: &str, param: &str) -> Res<crate::appdb::Slot> {
    let row = match int_param(param) {
        Some(n) => crate::appdb::slot_row(n)?,
        None => None,
    };
    row.filter(|r| r.owner_id == user_id).ok_or_else(|| err(404, "Account not found"))
}

/// safeNext: a path of this site to go to after the sign-in; anything that would leave it goes to /.
pub fn safe_next(next: Option<&str>) -> String {
    let Some(n) = next else { return "/".into() };
    if !n.starts_with('/') || n.starts_with("//") || n.chars().any(|c| (c as u32) < 0x20 || c == '\\') {
        return "/".into();
    }
    match url::Url::parse("http://relay.invalid").and_then(|b| b.join(n)) {
        Ok(u) if u.origin().ascii_serialization() == "http://relay.invalid" => n.to_string(),
        _ => "/".into(),
    }
}

pub fn login_url(next: &str) -> String {
    format!("/login?next={}", crate::js::encode_uri_component(next))
}

/// loginFor: the sign-in page, with the account (a) and the start page of the Android app (app) to come back to.
pub fn login_for(params: &[(String, String)]) -> String {
    let mut back = form_urlencoded::Serializer::new(String::new());
    let mut any = false;
    for k in ["a", "app"] {
        if let Some((_, v)) = params.iter().find(|(x, v)| x == k && !v.is_empty()) {
            back.append_pair(k, v);
            any = true;
        }
    }
    if any { login_url(&format!("/?{}", back.finish())) } else { "/login".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_paths() {
        assert_eq!(safe_next(Some("/settings?x=1")), "/settings?x=1");
        for bad in ["//evil.example", "/\\evil.example", "/\tx", "http://x", ""] {
            assert_eq!(safe_next(Some(bad)), "/", "{bad}");
        }
        assert_eq!(login_url("/?a=2"), "/login?next=%2F%3Fa%3D2");
        assert_eq!(login_for(&[("a".into(), "2".into()), ("x".into(), "1".into())]), "/login?next=%2F%3Fa%3D2");
        assert_eq!(pick_slot(&[3, 5], None).ok(), Some(3));
        assert!(pick_slot(&[3, 5], Some("4")).is_err());
        assert!(pick_slot(&[3, 5], Some("+5")).is_err());
        assert_eq!(int_param(" 5 "), Some(5));
        assert_eq!(int_param("5.5"), None);
    }
}
