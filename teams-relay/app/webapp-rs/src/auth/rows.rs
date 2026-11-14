//! Better Auth rows in app.db as its SQLite adapter writes and reads them: dates are ISO text with milliseconds and Z,
//! booleans integers, ids 32 alphanumeric characters. JSON answers keep Better Auth's key order (the id last). The
//! admin plugin adds role, banned, banReason, banExpires to the user and impersonatedBy to the session.

use super::crypto::generate_id;
use crate::appdb::with;
use crate::web::Res;
use rusqlite::{Connection, OptionalExtension, Row, params, types::Value as Sql};
use serde_json::{Map, Value};

/// toISOString(): YYYY-MM-DDTHH:MM:SS.mmmZ
pub fn iso_ms(ms: i64) -> String {
    let ts = jiff::Timestamp::from_millisecond(ms).unwrap_or_default();
    format!("{}.{:03}Z", ts.strftime("%Y-%m-%dT%H:%M:%S"), ms.rem_euclid(1000))
}

/// `new Date(value)` of a date read from the database: ISO text, or milliseconds.
pub fn ms_of(v: &Sql) -> Option<i64> {
    match v {
        Sql::Text(t) => t.parse::<jiff::Timestamp>().ok().map(|x| x.as_millisecond()).or_else(|| t.parse::<i64>().ok()),
        Sql::Integer(n) => Some(*n),
        Sql::Real(f) => Some(*f as i64),
        _ => None,
    }
}

fn date(r: &Row, c: &str) -> rusqlite::Result<Option<i64>> {
    Ok(ms_of(&r.get::<_, Sql>(c)?))
}

fn boolean(r: &Row, c: &str) -> rusqlite::Result<Option<bool>> {
    Ok(match r.get::<_, Sql>(c)? {
        Sql::Integer(n) => Some(n != 0),
        Sql::Text(t) => Some(t == "true" || t == "1"),
        _ => None,
    })
}

fn date_json(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, |m| Value::String(iso_ms(m)))
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub email_verified: bool,
    pub image: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub role: Option<String>,
    pub banned: Option<bool>,
    pub ban_reason: Option<String>,
    pub ban_expires: Option<i64>,
}

impl User {
    fn from(r: &Row) -> rusqlite::Result<Self> {
        Ok(User {
            id: r.get("id")?,
            name: r.get("name")?,
            email: r.get("email")?,
            email_verified: boolean(r, "emailVerified")?.unwrap_or(false),
            image: r.get("image")?,
            created_at: date(r, "createdAt")?,
            updated_at: date(r, "updatedAt")?,
            role: r.get("role")?,
            banned: boolean(r, "banned")?,
            ban_reason: r.get("banReason")?,
            ban_expires: date(r, "banExpires")?,
        })
    }

    pub fn json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), self.name.clone().into());
        m.insert("email".into(), self.email.clone().into());
        m.insert("emailVerified".into(), self.email_verified.into());
        m.insert("image".into(), self.image.clone().map_or(Value::Null, Value::String));
        m.insert("createdAt".into(), date_json(self.created_at));
        m.insert("updatedAt".into(), date_json(self.updated_at));
        m.insert("role".into(), self.role.clone().map_or(Value::Null, Value::String));
        m.insert("banned".into(), self.banned.map_or(Value::Null, Value::Bool));
        m.insert("banReason".into(), self.ban_reason.clone().map_or(Value::Null, Value::String));
        m.insert("banExpires".into(), date_json(self.ban_expires));
        m.insert("id".into(), self.id.clone().into());
        Value::Object(m)
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub expires_at: i64,
    pub token: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub user_id: String,
    pub impersonated_by: Option<String>,
}

impl Session {
    fn from(r: &Row) -> rusqlite::Result<Self> {
        Ok(Session {
            id: r.get("sid")?,
            expires_at: ms_of(&r.get::<_, Sql>("expiresAt")?).unwrap_or(0),
            token: r.get("token")?,
            created_at: ms_of(&r.get::<_, Sql>("screatedAt")?),
            updated_at: ms_of(&r.get::<_, Sql>("supdatedAt")?),
            ip_address: r.get("ipAddress")?,
            user_agent: r.get("userAgent")?,
            user_id: r.get("userId")?,
            impersonated_by: r.get("impersonatedBy")?,
        })
    }

    pub fn json(&self) -> Value {
        let mut m = Map::new();
        m.insert("expiresAt".into(), iso_ms(self.expires_at).into());
        m.insert("token".into(), self.token.clone().into());
        m.insert("createdAt".into(), date_json(self.created_at));
        m.insert("updatedAt".into(), date_json(self.updated_at));
        m.insert("ipAddress".into(), self.ip_address.clone().map_or(Value::Null, Value::String));
        m.insert("userAgent".into(), self.user_agent.clone().map_or(Value::Null, Value::String));
        m.insert("userId".into(), self.user_id.clone().into());
        m.insert("impersonatedBy".into(), self.impersonated_by.clone().map_or(Value::Null, Value::String));
        m.insert("id".into(), self.id.clone().into());
        Value::Object(m)
    }
}

pub fn user_by_email(email: &str) -> Res<Option<User>> {
    with(|c| c.query_row(r#"SELECT * FROM "user" WHERE email = ?"#, [email.to_lowercase()], User::from).optional())
}

pub fn user_by_id(id: &str) -> Res<Option<User>> {
    if id.is_empty() {
        return Ok(None);
    }
    with(|c| c.query_row(r#"SELECT * FROM "user" WHERE id = ?"#, [id], User::from).optional())
}

pub fn count_users() -> Res<i64> {
    with(|c| c.query_row(r#"SELECT COUNT(*) FROM "user""#, [], |r| r.get(0)))
}

/// Users sorted by createdAt ascending, at most `limit` (listUsers of src/app/api/admin/users).
pub fn list_users(limit: i64) -> Res<Vec<User>> {
    with(|c| c.prepare(r#"SELECT * FROM "user" ORDER BY "createdAt" ASC LIMIT ?"#)?.query_map([limit], User::from)?.collect())
}

/// The password of the user's credential account: None without the account, Some(None) without a password.
pub fn password_of(user_id: &str) -> Res<Option<Option<String>>> {
    with(|c| {
        c.query_row(r#"SELECT password FROM "account" WHERE userId = ? AND providerId = 'credential' ORDER BY rowid LIMIT 1"#, [user_id], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()
    })
}

pub fn create_user(email: &str, name: &str, role: &str, now: i64) -> Res<User> {
    let id = generate_id();
    with(|c| {
        c.execute(
            r#"INSERT INTO "user" (id, name, email, emailVerified, image, createdAt, updatedAt, role, banned, banReason, banExpires) VALUES (?, ?, ?, 0, NULL, ?, ?, ?, 0, NULL, NULL)"#,
            params![id, name, email.to_lowercase(), iso_ms(now), iso_ms(now), role],
        )
    })?;
    Ok(user_by_id(&id)?.expect("just inserted"))
}

pub fn update_user_admin(id: &str, now: i64) -> Res<()> {
    with(|c| {
        c.execute(r#"UPDATE "user" SET role = 'admin', banned = 0, banReason = NULL, banExpires = NULL, updatedAt = ? WHERE id = ?"#, params![iso_ms(now), id])
    })
    .map(drop)
}

pub fn unban(id: &str, now: i64) -> Res<()> {
    with(|c| c.execute(r#"UPDATE "user" SET banned = 0, banReason = NULL, banExpires = NULL, updatedAt = ? WHERE id = ?"#, params![iso_ms(now), id])).map(drop)
}

pub fn link_account(user_id: &str, hash: &str, now: i64) -> Res<()> {
    with(|c| {
        c.execute(
            r#"INSERT INTO "account" (id, accountId, providerId, userId, password, createdAt, updatedAt) VALUES (?, ?, 'credential', ?, ?, ?, ?)"#,
            params![generate_id(), user_id, user_id, hash, iso_ms(now), iso_ms(now)],
        )
    })
    .map(drop)
}

pub fn update_password(user_id: &str, hash: &str, now: i64) -> Res<()> {
    with(|c| {
        c.execute(r#"UPDATE "account" SET password = ?, updatedAt = ? WHERE userId = ? AND providerId = 'credential'"#, params![hash, iso_ms(now), user_id])
    })
    .map(drop)
}

/// deleteUser: accounts and the user (sessions go first, through delete_user_sessions).
pub fn delete_user(id: &str) -> Res<()> {
    with(|c| {
        c.execute(r#"DELETE FROM "account" WHERE userId = ?"#, [id])?;
        c.execute(r#"DELETE FROM "user" WHERE id = ?"#, [id])
    })
    .map(drop)
}

const SESSION_AND_USER: &str = r#"SELECT s.id AS sid, s.expiresAt, s.token, s.createdAt AS screatedAt, s.updatedAt AS supdatedAt, s.ipAddress, s.userAgent, s.userId, s.impersonatedBy,
    u.id, u.name, u.email, u.emailVerified, u.image, u.createdAt, u.updatedAt, u.role, u.banned, u.banReason, u.banExpires
    FROM "session" s JOIN "user" u ON u.id = s.userId WHERE s.token = ?"#;

pub fn find_session(token: &str) -> Res<Option<(Session, User)>> {
    with(|c| c.query_row(SESSION_AND_USER, [token], |r| Ok((Session::from(r)?, User::from(r)?))).optional())
}

pub fn create_session(user_id: &str, dont_remember: bool, ip: &str, agent: &str, now: i64) -> Res<Session> {
    let expires = now + if dont_remember { 3600 * 24 } else { super::cookie::EXPIRES_S } * 1000;
    let token = generate_id();
    with(|c| {
        c.execute(
            r#"INSERT INTO "session" (id, expiresAt, token, createdAt, updatedAt, ipAddress, userAgent, userId) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![generate_id(), iso_ms(expires), token, iso_ms(now), iso_ms(now), ip, agent, user_id],
        )
    })?;
    Ok(find_session(&token)?.expect("just inserted").0)
}

/// The ids of the sessions a delete is about to remove: their phones of the Android app go with them (src/lib/auth.ts).
fn forget_phones(ids: Vec<String>) {
    for id in ids {
        if let Err(e) = crate::appdb::forget_fcm_devices_of_session(&id) {
            eprintln!("phones of session: {e}");
        }
    }
}

fn ids(c: &Connection, sql: &str, p: &str) -> rusqlite::Result<Vec<String>> {
    c.prepare(sql)?.query_map([p], |r| r.get(0))?.collect()
}

pub fn delete_session(token: &str) -> Res<()> {
    let gone = with(|c| {
        let ids = ids(c, r#"SELECT id FROM "session" WHERE token = ?"#, token)?;
        c.execute(r#"DELETE FROM "session" WHERE token = ?"#, [token])?;
        Ok(ids)
    })?;
    forget_phones(gone);
    Ok(())
}

pub fn delete_user_sessions(user_id: &str) -> Res<()> {
    let gone = with(|c| {
        let ids = ids(c, r#"SELECT id FROM "session" WHERE userId = ?"#, user_id)?;
        c.execute(r#"DELETE FROM "session" WHERE userId = ?"#, [user_id])?;
        Ok(ids)
    })?;
    forget_phones(gone);
    Ok(())
}

/// Sessions of the user still valid, except the one of this token (revoke-other-sessions).
pub fn delete_other_sessions(user_id: &str, keep_token: &str, now: i64) -> Res<()> {
    let tokens: Vec<(String, Option<i64>)> = with(|c| {
        c.prepare(r#"SELECT token, expiresAt FROM "session" WHERE userId = ?"#)?
            .query_map([user_id], |r| Ok((r.get(0)?, ms_of(&r.get::<_, Sql>(1)?))))?
            .collect()
    })?;
    for (t, exp) in tokens {
        if t != keep_token && exp.is_some_and(|e| e > now) {
            delete_session(&t)?;
        }
    }
    Ok(())
}

/// updateSession(token, {expiresAt, updatedAt}): the changed row, None when it is gone meanwhile.
pub fn renew_session(token: &str, expires: i64, now: i64) -> Res<Option<Session>> {
    let n = with(|c| c.execute(r#"UPDATE "session" SET expiresAt = ?, updatedAt = ? WHERE token = ?"#, params![iso_ms(expires), iso_ms(now), token]))?;
    if n == 0 {
        return Ok(None);
    }
    Ok(find_session(token)?.map(|(s, _)| s))
}

/// The Better Auth schema with the admin plugin, as getMigrations makes it on a new database; columns of the plugin
/// added to an older one. Nothing is changed or dropped.
pub fn migrate(c: &Connection) -> rusqlite::Result<()> {
    let tables = [
        (
            "user",
            r#"CREATE TABLE "user" ("id" text not null primary key, "name" text not null, "email" text not null unique, "emailVerified" integer not null, "image" text, "createdAt" date not null, "updatedAt" date not null, "role" text, "banned" integer, "banReason" text, "banExpires" date)"#,
        ),
        (
            "session",
            r#"CREATE TABLE "session" ("id" text not null primary key, "expiresAt" date not null, "token" text not null unique, "createdAt" date not null, "updatedAt" date not null, "ipAddress" text, "userAgent" text, "userId" text not null references "user" ("id") on delete cascade, "impersonatedBy" text)"#,
        ),
        (
            "account",
            r#"CREATE TABLE "account" ("id" text not null primary key, "accountId" text not null, "providerId" text not null, "userId" text not null references "user" ("id") on delete cascade, "accessToken" text, "refreshToken" text, "idToken" text, "accessTokenExpiresAt" date, "refreshTokenExpiresAt" date, "scope" text, "password" text, "createdAt" date not null, "updatedAt" date not null)"#,
        ),
        (
            "verification",
            r#"CREATE TABLE "verification" ("id" text not null primary key, "identifier" text not null, "value" text not null, "expiresAt" date not null, "createdAt" date not null, "updatedAt" date not null)"#,
        ),
    ];
    let indexes = [
        ("session_userId_idx", r#"CREATE INDEX "session_userId_idx" on "session" ("userId")"#),
        ("account_userId_idx", r#"CREATE INDEX "account_userId_idx" on "account" ("userId")"#),
        ("verification_identifier_idx", r#"CREATE INDEX "verification_identifier_idx" on "verification" ("identifier")"#),
    ];
    let exists = |kind: &str, name: &str| -> rusqlite::Result<bool> {
        Ok(c.query_row("SELECT 1 FROM sqlite_master WHERE type = ? AND name = ?", [kind, name], |_| Ok(())).optional()?.is_some())
    };
    for (name, sql) in tables {
        if !exists("table", name)? {
            c.execute_batch(sql)?;
        }
    }
    for (table, column, kind) in [
        ("user", "role", "text"),
        ("user", "banned", "integer"),
        ("user", "banReason", "text"),
        ("user", "banExpires", "date"),
        ("session", "impersonatedBy", "text"),
    ] {
        let present: Vec<String> =
            c.prepare(&format!(r#"PRAGMA table_info("{table}")"#))?.query_map([], |r| r.get::<_, String>("name"))?.collect::<Result<_, _>>()?;
        if !present.iter().any(|x| x == column) {
            c.execute_batch(&format!(r#"ALTER TABLE "{table}" ADD COLUMN "{column}" {kind}"#))?;
        }
    }
    for (name, sql) in indexes {
        if !exists("index", name)? {
            c.execute_batch(sql)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso() {
        assert_eq!(iso_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_ms(86_400_007), "1970-01-02T00:00:00.007Z");
        assert_eq!(ms_of(&Sql::Text(iso_ms(1_794_647_035_123))), Some(1_794_647_035_123));
    }
}
