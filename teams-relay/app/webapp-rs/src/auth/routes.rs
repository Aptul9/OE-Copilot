//! /api/auth/*: the Better Auth endpoints the app and its clients use, with the same answers, cookies and order of
//! checks (rate limit, route, body, origin, hooks before, validation, session, handler). Other endpoints answer 404.
//! Port of the calendar gateway (agente-rs/gateway/src/auth/rotte.rs) without two factors and passkeys, with the admin
//! plugin's ban check and the hook of src/lib/auth.ts on the password of the administrator of .env.

use super::cookie::{self, EXPIRES_S, name};
use super::rows::{self, Session, User};
use super::{ONE_DAY_S, PASSWORD_MAX, PASSWORD_MIN, crypto};
use crate::web::{Error, Req, Resp, now_ms};
use serde_json::{Map, Value, json};

/// An APIError: status and body (None: a bare 500).
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub body: Option<Value>,
}

pub fn api_error(status: u16, code: &str, message: &str) -> ApiError {
    ApiError { status, body: Some(json!({ "message": message, "code": code })) }
}

pub fn api_message(status: u16, message: &str) -> ApiError {
    ApiError { status, body: Some(json!({ "message": message })) }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        eprintln!("auth: {e}");
        ApiError { status: 500, body: None }
    }
}

type Out<T> = Result<T, ApiError>;

fn unauthorized() -> ApiError {
    api_error(401, "UNAUTHORIZED", "Unauthorized")
}

struct Ctx<'a> {
    r: &'a Req,
    secret: String,
    body: Option<Value>,
    query: Map<String, Value>,
    out: Vec<(String, String)>,
    session: Option<Option<(Session, User)>>,
    created: bool,
}

impl<'a> Ctx<'a> {
    fn now(&self) -> i64 {
        now_ms()
    }

    fn cookie(&self) -> Option<String> {
        self.r.header("cookie")
    }

    fn signed(&self, which: &str) -> Option<String> {
        cookie::signed(self.cookie().as_deref(), &name(which), &self.secret)
    }

    fn set(&mut self, l: String) {
        self.out.push(("set-cookie".into(), l));
    }

    fn header_out(&mut self, k: &str, v: &str) {
        self.out.retain(|(x, _)| x != k);
        self.out.push((k.into(), v.into()));
    }

    /// removeSetCookieEntries, then expireCookie.
    fn expire(&mut self, which: &str) {
        let n = name(which);
        let (exact, piece) = (format!("{n}="), format!("{n}."));
        self.out.retain(|(k, v)| k != "set-cookie" || !(v.starts_with(&exact) || v.starts_with(&piece)));
        self.set(cookie::expired(&n));
    }

    /// The pieces of the session cache cookie the request carries (session_data and session_data.N): expired.
    fn clear_session_data(&mut self) {
        let n = name(cookie::SESSION_DATA);
        let present: Vec<String> = cookie::parse_all(self.cookie().as_deref().unwrap_or(""))
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| *k == n || k.strip_prefix(&format!("{n}.")).is_some_and(|i| i.parse::<u64>().is_ok_and(|x| x.to_string() == i)))
            .collect();
        for k in present {
            self.set(cookie::expired(&k));
        }
    }

    fn delete_session_cookie(&mut self) {
        self.expire(cookie::SESSION);
        self.expire(cookie::SESSION_DATA);
        self.clear_session_data();
        self.expire(cookie::DONT_REMEMBER);
    }

    fn set_session_cookie(&mut self, s: &Session, dont_remember: Option<bool>) {
        let had = self.signed(cookie::DONT_REMEMBER).is_some();
        let dont_remember = dont_remember.unwrap_or(had);
        let l = cookie::write_signed(&name(cookie::SESSION), &s.token, &self.secret, (!dont_remember).then_some(EXPIRES_S));
        self.set(l);
        if dont_remember {
            let l = cookie::write_signed(&name(cookie::DONT_REMEMBER), "true", &self.secret, None);
            self.set(l);
        } else if had {
            self.expire(cookie::DONT_REMEMBER);
        }
        self.created = true;
    }

    /// getSession: the session of the cookie, renewed when older than a day; expired or unknown, the cookies go.
    fn read_session(&mut self, no_renew: bool) -> Out<Option<(Session, User)>> {
        let now = self.now();
        let token = self.signed(cookie::SESSION);
        let data = name(cookie::SESSION_DATA);
        if cookie::parse_all(self.cookie().as_deref().unwrap_or("")).iter().any(|(k, _)| *k == data || k.starts_with(&format!("{data}."))) {
            self.clear_session_data();
        }
        let Some(token) = token else { return Ok(None) };
        let dont_remember = self.signed(cookie::DONT_REMEMBER).is_some();
        let found = rows::find_session(&token).map_err(|_| api_error(500, "FAILED_TO_GET_SESSION", "Failed to get session"))?;
        let Some((s, u)) = found.filter(|(s, _)| s.expires_at >= now) else {
            self.delete_session_cookie();
            if let Ok(Some(_)) = rows::find_session(&token) {
                let _ = rows::delete_session(&token);
            }
            return Ok(None);
        };
        if dont_remember || no_renew {
            return Ok(Some((s, u)));
        }
        if s.expires_at - EXPIRES_S * 1000 + ONE_DAY_S * 1000 <= now {
            let Some(renewed) =
                rows::renew_session(&s.token, now + EXPIRES_S * 1000, now).map_err(|_| api_error(500, "FAILED_TO_GET_SESSION", "Failed to get session"))?
            else {
                self.delete_session_cookie();
                return Err(api_error(401, "FAILED_TO_GET_SESSION", "Failed to get session"));
            };
            let l = cookie::write_signed(&name(cookie::SESSION), &renewed.token, &self.secret, Some(EXPIRES_S));
            self.set(l);
            return Ok(Some((renewed, u)));
        }
        Ok(Some((s, u)))
    }

    /// getSessionFromCtx: read once per request.
    fn session(&mut self) -> Option<(Session, User)> {
        if let Some(s) = &self.session {
            return s.clone();
        }
        let s = self.read_session(false).unwrap_or(None);
        self.session = Some(s.clone());
        s
    }

    fn ip(&self) -> String {
        super::limit::ip(self.r.header("x-forwarded-for").as_deref()).unwrap_or_default()
    }
}

fn zod_type(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Bool,
}

/// The body against its schema: Err with the text of better-call ("[body.email] Invalid input: expected string, ...").
fn validate(body: Option<&Value>, fields: &[(&str, Kind, bool)], optional_body: bool) -> Out<Map<String, Value>> {
    let o = match body {
        Some(Value::Object(m)) => m.clone(),
        None if optional_body => return Ok(Map::new()),
        other => return Err(api_error(400, "VALIDATION_ERROR", &format!("[body] Invalid input: expected object, received {}", zod_type(other)))),
    };
    let mut problems = vec![];
    for (n, kind, required) in fields {
        let v = o.get(*n);
        let good = match (v, kind) {
            (None, _) => !required,
            (Some(Value::String(_)), Kind::Text) | (Some(Value::Bool(_)), Kind::Bool) => true,
            _ => false,
        };
        if !good {
            let expected = if *kind == Kind::Text { "string" } else { "boolean" };
            problems.push(format!("[body.{n}] Invalid input: expected {expected}, received {}", zod_type(v)));
        }
    }
    if !problems.is_empty() {
        return Err(api_error(400, "VALIDATION_ERROR", &problems.join("; ")));
    }
    Ok(o)
}

fn origin_of(u: &str) -> Option<String> {
    let o = url::Url::parse(u).ok()?.origin();
    o.is_tuple().then(|| o.ascii_serialization())
}

/// isSafeRelativeURL
fn safe_relative(v: &str) -> bool {
    if !v.starts_with('/') || v.starts_with("//") || v.contains('\\') || v.chars().any(|c| matches!(c as u32, 0..=0x1f | 0x7f..=0x9f)) {
        return false;
    }
    let path = v[..v.find(['?', '#']).unwrap_or(v.len())].to_ascii_lowercase();
    if path.contains("%2f") || path.contains("%5c") {
        return false;
    }
    url::Url::parse("https://better-auth.invalid").and_then(|b| b.join(v)).is_ok_and(|u| u.origin().ascii_serialization() == "https://better-auth.invalid")
}

fn trusted(u: &str, relative: bool) -> bool {
    if u.starts_with('/') {
        return relative && safe_relative(u);
    }
    origin_of(u).is_some_and(|o| origin_of(&crate::config::app_url()).as_deref() == Some(o.as_str()))
}

/// validateOrigin: with cookies (or when forced) the Origin, or the Referer, must be the app's.
fn check_origin(ctx: &Ctx, force: bool) -> Out<()> {
    let origin = ctx.r.header("origin");
    let text = origin.clone().filter(|o| !o.is_empty()).or_else(|| ctx.r.header("referer")).unwrap_or_default();
    if !(force || ctx.r.header("cookie").is_some()) {
        return Ok(());
    }
    let inferred = (origin.as_deref() == Some("null") && ctx.r.header("sec-fetch-site").as_deref() == Some("same-origin"))
        .then(|| origin_of(&crate::config::app_url()).unwrap_or_default());
    let check = inferred.unwrap_or(text);
    if check.is_empty() || check == "null" {
        return Err(api_error(403, "MISSING_OR_NULL_ORIGIN", "Missing or null Origin"));
    }
    if !trusted(&check, false) {
        eprintln!("auth: Invalid origin: {check}");
        return Err(api_error(403, "INVALID_ORIGIN", "Invalid origin"));
    }
    Ok(())
}

/// originCheckMiddleware: requests that change something, and the return addresses in the body.
fn origin_middleware(ctx: &Ctx) -> Out<()> {
    if matches!(ctx.r.method.as_str(), "GET" | "OPTIONS" | "HEAD") {
        return Ok(());
    }
    check_origin(ctx, false)?;
    let body = ctx.body.as_ref();
    let items = [
        ("callbackURL", "INVALID_CALLBACK_URL", "Invalid callbackURL", "callbackURL"),
        ("redirectTo", "INVALID_REDIRECT_URL", "Invalid redirectURL", "redirectURL"),
        ("errorCallbackURL", "INVALID_ERROR_CALLBACK_URL", "Invalid errorCallbackURL", "errorCallbackURL"),
        ("newUserCallbackURL", "INVALID_NEW_USER_CALLBACK_URL", "Invalid newUserCallbackURL", "newUserCallbackURL"),
    ];
    for (i, (field, code, message, label)) in items.iter().enumerate() {
        let v = body.and_then(|b| b.get(field)).or_else(|| if i == 0 { ctx.query.get("callbackURL") } else { None });
        let Some(v) = v.filter(|v| crate::js::truthy(v)) else { continue };
        let Some(t) = v.as_str() else { return Err(api_message(400, &format!("Invalid {label}: expected a string"))) };
        if !trusted(t, true) {
            return Err(api_error(403, code, message));
        }
    }
    Ok(())
}

/// formCsrfMiddleware of the password sign-in.
fn form_check(ctx: &Ctx) -> Out<()> {
    if ctx.r.header("cookie").is_some() {
        return check_origin(ctx, false);
    }
    let (site, mode, dest) = (ctx.r.header("sec-fetch-site"), ctx.r.header("sec-fetch-mode"), ctx.r.header("sec-fetch-dest"));
    let full = |v: &Option<String>| v.as_deref().is_some_and(|s| !cookie::js_trim(s).is_empty());
    if full(&site) || full(&mode) || full(&dest) {
        if site.as_deref() == Some("cross-site") && mode.as_deref() == Some("navigate") {
            eprintln!("auth: Blocked cross-site navigation login attempt (CSRF protection)");
            return Err(api_error(
                403,
                "CROSS_SITE_NAVIGATION_LOGIN_BLOCKED",
                "Cross-site navigation login blocked. This request appears to be a CSRF attack.",
            ));
        }
        return check_origin(ctx, true);
    }
    if ctx.r.header("origin").is_some() || ctx.r.header("referer").is_some() {
        return check_origin(ctx, true);
    }
    Ok(())
}

/// getBody: JSON (or a form, where allowed), with better-call's 415 for another Content-Type.
fn read_body(r: &Req, form_too: bool) -> Out<Option<Value>> {
    let kind = r.header("content-type").unwrap_or_default();
    let lower = kind.to_lowercase();
    if r.body.is_empty() && kind.is_empty() {
        return Ok(None);
    }
    let allowed: &[&str] = if form_too { &["application/x-www-form-urlencoded", "application/json"] } else { &["application/json"] };
    let base = lower.split(';').next().unwrap_or("").trim().to_string();
    if !allowed.iter().any(|a| base == *a || base.contains(a)) {
        let list = allowed.join(", ");
        if lower.is_empty() {
            return Err(api_error(415, "UNSUPPORTED_MEDIA_TYPE", &format!("Content-Type is required. Allowed types: {list}")));
        }
        return Err(api_error(415, "UNSUPPORTED_MEDIA_TYPE", &format!("Content-Type \"{kind}\" is not allowed. Allowed types: {list}")));
    }
    let is_json = regex::Regex::new(r"(?i)^application/([a-z0-9.+-]*\+)?json").expect("fixed").is_match(&lower);
    if is_json {
        let text = String::from_utf8_lossy(&r.body);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        return serde_json::from_str::<Value>(text).map(Some).map_err(|_| api_error(400, "BAD_REQUEST", "Invalid JSON in request body"));
    }
    if lower.contains("application/x-www-form-urlencoded") {
        let mut m = Map::new();
        for (k, v) in form_urlencoded::parse(&r.body) {
            m.insert(k.into_owned(), Value::String(v.into_owned()));
        }
        return Ok(Some(Value::Object(m)));
    }
    Ok(Some(Value::String(String::from_utf8_lossy(&r.body).into_owned())))
}

fn json_response(status: u16, headers: Vec<(String, String)>, body: Option<Value>) -> Resp {
    let mut h = headers;
    if body.is_some() || status != 404 {
        h.push(("content-type".into(), "application/json".into()));
    }
    response(status, h, body.map(|b| crate::js::stringify(&b)))
}

fn response(status: u16, headers: Vec<(String, String)>, body: Option<String>) -> Resp {
    let mut b = hyper::Response::builder().status(status);
    for (k, v) in &headers {
        b = b.header(k.as_str(), v.as_str());
    }
    use http_body_util::BodyExt;
    let body = match body {
        Some(t) => http_body_util::Full::new(bytes::Bytes::from(t)).boxed_unsync(),
        None => http_body_util::Empty::new().boxed_unsync(),
    };
    b.body(body).expect("response")
}

fn exists(method: &str, p: &str) -> bool {
    matches!(
        (method, p),
        ("GET", "/get-session" | "/ok") | ("POST", "/get-session" | "/sign-in/email" | "/sign-out" | "/change-password" | "/revoke-other-sessions")
    )
}

/// A request to /api/auth/*.
pub async fn handle(r: &Req) -> Resp {
    if let Some(wait) = super::limit::consume(&r.path, r.header("x-forwarded-for").as_deref(), now_ms()) {
        return response(
            429,
            vec![("content-type".into(), "application/json".into()), ("x-retry-after".into(), wait.to_string())],
            Some(r#"{"message":"Too many requests. Please try again later."}"#.into()),
        );
    }
    let Some(p) = r.path.strip_prefix("/api/auth").filter(|p| p.starts_with('/') && p.len() > 1 && !p.contains("//")) else {
        return response(404, vec![], None);
    };
    if !exists(&r.method, p) {
        return response(404, vec![], None);
    }
    // EverID on with PASSWORD_LOGIN=0: the pages sign in through EverID only (everid.rs)
    if p == "/sign-in/email" && !crate::config::password_login() {
        return json_response(403, vec![], Some(json!({ "message": "Password sign-in is off: sign in with EverID", "code": "PASSWORD_LOGIN_OFF" })));
    }
    let query = form_urlencoded::parse(r.query.as_bytes()).fold(Map::new(), |mut m, (k, v)| {
        m.insert(k.into_owned(), Value::String(v.into_owned()));
        m
    });
    let mut ctx = Ctx { r, secret: crate::config::auth_secret(), body: None, query, out: vec![], session: None, created: false };
    let result = run(&mut ctx, p).await;
    let mut out = std::mem::take(&mut ctx.out);
    match result {
        Ok((status, body)) => json_response(status, out, Some(body)),
        Err(e) if e.status == 500 && e.body.is_none() => response(500, vec![], None),
        Err(e) => match e.body {
            None => {
                out.push(("content-type".into(), "application/json".into()));
                response(e.status, out, None)
            }
            body => json_response(e.status, out, body),
        },
    }
}

async fn run(ctx: &mut Ctx<'_>, p: &str) -> Out<(u16, Value)> {
    ctx.body = if ctx.r.method == "GET" { None } else { read_body(ctx.r, p == "/sign-in/email")? };
    origin_middleware(ctx)?;
    match p {
        "/get-session" => get_session(ctx).map(|b| (200, b)),
        "/ok" => Ok((200, json!({ "ok": true }))),
        "/sign-in/email" => sign_in(ctx).await.map(|b| (200, b)),
        "/sign-out" => sign_out(ctx).map(|b| (200, b)),
        "/change-password" => change_password(ctx).await.map(|b| (200, b)),
        _ => revoke_other_sessions(ctx).map(|b| (200, b)),
    }
}

fn get_session(ctx: &mut Ctx) -> Out<Value> {
    ctx.header_out("cache-control", "no-store");
    ctx.header_out("pragma", "no-cache");
    if ctx.r.method == "POST" {
        return Err(api_error(405, "METHOD_NOT_ALLOWED_DEFER_SESSION_REQUIRED", "POST method requires deferSessionRefresh to be enabled in session config"));
    }
    let no_renew = ctx.query.get("disableRefresh").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    Ok(match ctx.read_session(no_renew)? {
        Some((s, u)) => json!({ "session": s.json(), "user": u.json() }),
        None => Value::Null,
    })
}

/// z.email() of zod 4.
fn valid_email(e: &str) -> bool {
    let Some(at) = e.rfind('@') else { return false };
    let (local, domain) = (&e[..at], &e[at + 1..]);
    if local.is_empty() || local.starts_with('.') || e.contains("..") {
        return false;
    }
    let last = local.chars().last().unwrap_or('.');
    if !local.chars().all(|c| c.is_ascii_alphanumeric() || "_'+-.".contains(c)) || !(last.is_ascii_alphanumeric() || "_+-".contains(last)) {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let (tld, rest) = labels.split_last().expect("two labels");
    tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && rest.iter().all(|l| l.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

pub fn email_ok(e: &str) -> bool {
    valid_email(e)
}

fn too_long(p: &str) -> bool {
    p.encode_utf16().count() > PASSWORD_MAX
}

/// The admin plugin's hook on a new session: a banned user gets none, a ban that expired is lifted.
fn ban_check(u: &User, now: i64) -> Out<()> {
    if u.banned != Some(true) {
        return Ok(());
    }
    if u.ban_expires.is_some_and(|e| e < now) {
        rows::unban(&u.id, now)?;
        return Ok(());
    }
    Err(api_error(403, "BANNED_USER", "You have been banned from this application"))
}

async fn sign_in(ctx: &mut Ctx<'_>) -> Out<Value> {
    let body = validate(
        ctx.body.as_ref(),
        &[("email", Kind::Text, true), ("password", Kind::Text, true), ("callbackURL", Kind::Text, false), ("rememberMe", Kind::Bool, false)],
        false,
    )?;
    form_check(ctx)?;
    let email = body.get("email").and_then(Value::as_str).unwrap_or("").to_string();
    let password = body.get("password").and_then(Value::as_str).unwrap_or("").to_string();
    if !valid_email(&email) {
        return Err(api_error(400, "INVALID_EMAIL", "Invalid email"));
    }
    if too_long(&password) {
        return Err(api_error(400, "PASSWORD_TOO_LONG", "Password too long"));
    }
    let user = rows::user_by_email(&email)?;
    let hash = match &user {
        Some(u) => rows::password_of(&u.id)?,
        None => None,
    };
    let invalid = || api_error(401, "INVALID_EMAIL_OR_PASSWORD", "Invalid email or password");
    let (Some(u), Some(Some(hash))) = (user, hash) else {
        // the time of a wrong password: whoever tries cannot tell whether the user exists
        crypto::hash_password_async(password).await;
        return Err(invalid());
    };
    if !crypto::verify_password_async(hash, password).await.map_err(Error::Internal)? {
        return Err(invalid());
    }
    ban_check(&u, ctx.now())?;
    let dont_remember = body.get("rememberMe") == Some(&Value::Bool(false));
    let agent = ctx.r.header("user-agent").unwrap_or_default();
    let s = rows::create_session(&u.id, dont_remember, &ctx.ip(), &agent, ctx.now())?;
    ctx.set_session_cookie(&s, Some(dont_remember));
    let back = body.get("callbackURL").and_then(Value::as_str).map(str::to_string);
    let mut m = Map::new();
    m.insert("redirect".into(), back.is_some().into());
    m.insert("token".into(), s.token.clone().into());
    if let Some(b) = &back {
        ctx.header_out("location", b);
        m.insert("url".into(), b.clone().into());
    }
    m.insert("user".into(), u.json());
    Ok(Value::Object(m))
}

fn sign_out(ctx: &mut Ctx) -> Out<Value> {
    validate(ctx.body.as_ref(), &[("callbackURL", Kind::Text, false), ("disableRedirect", Kind::Bool, false), ("state", Kind::Text, false)], true)?;
    if let Some(token) = ctx.signed(cookie::SESSION)
        && let Err(e) = rows::delete_session(&token)
    {
        eprintln!("auth: Failed to delete session from database {e}");
    }
    ctx.delete_session_cookie();
    Ok(json!({ "success": true }))
}

async fn change_password(ctx: &mut Ctx<'_>) -> Out<Value> {
    // the hook of src/lib/auth.ts: the administrator of .env changes its password in .env only
    let email = ctx.session().map(|(_, u)| u.email);
    if crate::config::is_env_admin(email.as_deref()) {
        return Err(api_message(403, crate::config::ENV_PASSWORD_MESSAGE));
    }
    let body = validate(
        ctx.body.as_ref(),
        &[("newPassword", Kind::Text, true), ("currentPassword", Kind::Text, true), ("revokeOtherSessions", Kind::Bool, false)],
        false,
    )?;
    // sensitiveSessionMiddleware: the session read from the database
    let (session, user) = ctx.read_session(false)?.ok_or_else(unauthorized)?;
    let new = body.get("newPassword").and_then(Value::as_str).unwrap_or("").to_string();
    let current = body.get("currentPassword").and_then(Value::as_str).unwrap_or("").to_string();
    if new.encode_utf16().count() < PASSWORD_MIN {
        return Err(api_error(400, "PASSWORD_TOO_SHORT", "Password too short"));
    }
    if too_long(&new) || too_long(&current) {
        return Err(api_error(400, "PASSWORD_TOO_LONG", "Password too long"));
    }
    let Some(Some(hash)) = rows::password_of(&user.id)? else { return Err(api_error(400, "CREDENTIAL_ACCOUNT_NOT_FOUND", "Credential account not found")) };
    let new_hash = crypto::hash_password_async(new).await;
    if !crypto::verify_password_async(hash, current).await.map_err(Error::Internal)? {
        return Err(api_error(400, "INVALID_PASSWORD", "Invalid password"));
    }
    let revoke = body.get("revokeOtherSessions") == Some(&Value::Bool(true));
    if revoke {
        rows::delete_user_sessions(&user.id)?;
    }
    rows::update_password(&user.id, &new_hash, ctx.now())?;
    let mut token = Value::Null;
    if revoke {
        let agent = ctx.r.header("user-agent").unwrap_or_default();
        let s = rows::create_session(&user.id, false, &ctx.ip(), &agent, ctx.now())?;
        ctx.set_session_cookie(&s, None);
        token = s.token.clone().into();
    }
    let _ = session;
    Ok(json!({ "token": token, "user": user.json() }))
}

fn revoke_other_sessions(ctx: &mut Ctx) -> Out<Value> {
    let (session, user) = ctx.read_session(false)?.ok_or_else(unauthorized)?;
    rows::delete_other_sessions(&user.id, &session.token, ctx.now())?;
    Ok(json!({ "status": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails() {
        assert!(valid_email("a.b@x.it"));
        assert!(!valid_email("a@x"));
        assert!(!valid_email(".a@x.it"));
        assert!(!valid_email("a..b@x.it"));
    }
}
