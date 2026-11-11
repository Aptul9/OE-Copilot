//! Le righe di Better Auth in gateway.db, scritte e lette come le scrive il suo adattatore SQLite: le date sono testi ISO con i millisecondi
//! e la Z (toISOString), i booleani interi, gli identificativi 32 caratteri alfanumerici. Le risposte JSON hanno le chiavi nell'ordine di
//! Better Auth (l'id in fondo).

use super::cripto::genera_id;
use crate::py::Esito;
use oe_nucleo::json::{Json, Mappa};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::rc::Rc;

pub fn db() -> Esito<Rc<Connection>> {
    Ok(crate::db::app_db()?)
}

/// toISOString(): AAAA-MM-GGTHH:MM:SS.mmmZ
pub fn iso_ms(ms: i64) -> String {
    let (a, m, g, h, mi, s) = crate::fuso::pezzi_utc(ms);
    format!("{a:04}-{m:02}-{g:02}T{h:02}:{mi:02}:{s:02}.{:03}Z", ms.rem_euclid(1000))
}

/// `new Date(valore)` di una data letta dal database: un testo ISO, o un numero di millisecondi.
pub fn ms_da(v: &rusqlite::types::Value) -> Option<i64> {
    match v {
        rusqlite::types::Value::Text(t) => t.parse::<jiff::Timestamp>().ok().map(|x| x.as_millisecond()).or_else(|| t.parse::<i64>().ok()),
        rusqlite::types::Value::Integer(n) => Some(*n),
        rusqlite::types::Value::Real(f) => Some(*f as i64),
        _ => None,
    }
}

fn data(r: &Row, i: &str) -> rusqlite::Result<Option<i64>> {
    Ok(ms_da(&r.get::<_, rusqlite::types::Value>(i)?))
}

fn booleano(r: &Row, i: &str) -> rusqlite::Result<Option<bool>> {
    Ok(match r.get::<_, rusqlite::types::Value>(i)? {
        rusqlite::types::Value::Integer(n) => Some(n != 0),
        rusqlite::types::Value::Text(t) => Some(t == "true" || t == "1"),
        _ => None,
    })
}

fn data_json(ms: Option<i64>) -> Json {
    ms.map_or(Json::Null, |m| Json::Str(iso_ms(m)))
}

#[derive(Clone, Debug)]
pub struct Utente {
    pub id: String,
    pub name: String,
    pub email: String,
    pub email_verified: bool,
    pub image: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub two_factor_enabled: Option<bool>,
}

impl Utente {
    fn da(r: &Row) -> rusqlite::Result<Self> {
        Ok(Utente {
            id: r.get("id")?,
            name: r.get("name")?,
            email: r.get("email")?,
            email_verified: booleano(r, "emailVerified")?.unwrap_or(false),
            image: r.get("image")?,
            created_at: data(r, "createdAt")?,
            updated_at: data(r, "updatedAt")?,
            two_factor_enabled: booleano(r, "twoFactorEnabled")?,
        })
    }

    pub fn json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("name", self.name.clone());
        m.insert("email", self.email.clone());
        m.insert("emailVerified", self.email_verified);
        m.insert("image", self.image.clone());
        m.insert("createdAt", data_json(self.created_at));
        m.insert("updatedAt", data_json(self.updated_at));
        m.insert("twoFactorEnabled", self.two_factor_enabled);
        m.insert("id", self.id.clone());
        Json::Obj(m)
    }
}

#[derive(Clone, Debug)]
pub struct Sessione {
    pub id: String,
    pub expires_at: i64,
    pub token: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub user_id: String,
}

impl Sessione {
    fn da(r: &Row) -> rusqlite::Result<Self> {
        Ok(Sessione {
            id: r.get("sid")?,
            expires_at: ms_da(&r.get::<_, rusqlite::types::Value>("expiresAt")?).unwrap_or(0),
            token: r.get("token")?,
            created_at: ms_da(&r.get::<_, rusqlite::types::Value>("screatedAt")?),
            updated_at: ms_da(&r.get::<_, rusqlite::types::Value>("supdatedAt")?),
            ip_address: r.get("ipAddress")?,
            user_agent: r.get("userAgent")?,
            user_id: r.get("userId")?,
        })
    }

    pub fn json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("expiresAt", iso_ms(self.expires_at));
        m.insert("token", self.token.clone());
        m.insert("createdAt", data_json(self.created_at));
        m.insert("updatedAt", data_json(self.updated_at));
        m.insert("ipAddress", self.ip_address.clone());
        m.insert("userAgent", self.user_agent.clone());
        m.insert("userId", self.user_id.clone());
        m.insert("id", self.id.clone());
        Json::Obj(m)
    }
}

pub fn utente_per_email(email: &str) -> Esito<Option<Utente>> {
    Ok(db()?.query_row(r#"SELECT * FROM "user" WHERE email = ?"#, [email.to_lowercase()], Utente::da).optional()?)
}

pub fn utente_per_id(id: &str) -> Esito<Option<Utente>> {
    if id.is_empty() {
        return Ok(None);
    }
    Ok(db()?.query_row(r#"SELECT * FROM "user" WHERE id = ?"#, [id], Utente::da).optional()?)
}

pub fn conta_utenti() -> Esito<i64> {
    Ok(db()?.query_row(r#"SELECT COUNT(*) FROM "user""#, [], |r| r.get(0))?)
}

/// La password del primo account "credential" fra quelli dell'utente (accounts.find di Better Auth): None se l'account non c'è, Some(None)
/// se non ha password.
pub fn password_di(user_id: &str) -> Esito<Option<Option<String>>> {
    Ok(db()?.query_row(r#"SELECT password FROM "account" WHERE userId = ? AND providerId = 'credential' ORDER BY rowid LIMIT 1"#, [user_id], |r| r.get::<_, Option<String>>(0)).optional()?)
}

pub fn crea_utente(email: &str, nome: &str, ora: i64) -> Esito<Utente> {
    let id = genera_id();
    db()?.execute(
        r#"INSERT INTO "user" (id, name, email, emailVerified, image, createdAt, updatedAt, twoFactorEnabled) VALUES (?, ?, ?, 1, NULL, ?, ?, 0)"#,
        params![id, nome, email.to_lowercase(), iso_ms(ora), iso_ms(ora)],
    )?;
    Ok(utente_per_id(&id)?.expect("appena inserito"))
}

pub fn collega_account(user_id: &str, hash: &str, ora: i64) -> Esito<()> {
    db()?.execute(
        r#"INSERT INTO "account" (id, accountId, providerId, userId, password, createdAt, updatedAt) VALUES (?, ?, 'credential', ?, ?, ?, ?)"#,
        params![genera_id(), user_id, user_id, hash, iso_ms(ora), iso_ms(ora)],
    )?;
    Ok(())
}

pub fn aggiorna_password(user_id: &str, hash: &str, ora: i64) -> Esito<()> {
    db()?.execute(
        r#"UPDATE "account" SET password = ?, updatedAt = ? WHERE userId = ? AND providerId = 'credential'"#,
        params![hash, iso_ms(ora), user_id],
    )?;
    Ok(())
}

/// updateUser(userId, { twoFactorEnabled }): la riga cambiata, con updatedAt di adesso.
pub fn imposta_due_fattori(user_id: &str, attivo: bool, ora: i64) -> Esito<Option<Utente>> {
    db()?.execute(r#"UPDATE "user" SET twoFactorEnabled = ?, updatedAt = ? WHERE id = ?"#, params![i64::from(attivo), iso_ms(ora), user_id])?;
    utente_per_id(user_id)
}

// ---- Sessioni -----------------------------------------------------------------------------------------------------------
const SESSIONE_E_UTENTE: &str = r#"SELECT s.id AS sid, s.expiresAt, s.token, s.createdAt AS screatedAt, s.updatedAt AS supdatedAt, s.ipAddress, s.userAgent, s.userId,
    u.id, u.name, u.email, u.emailVerified, u.image, u.createdAt, u.updatedAt, u.twoFactorEnabled
    FROM "session" s JOIN "user" u ON u.id = s.userId WHERE s.token = ?"#;

pub fn trova_sessione(token: &str) -> Esito<Option<(Sessione, Utente)>> {
    Ok(db()?.query_row(SESSIONE_E_UTENTE, [token], |r| Ok((Sessione::da(r)?, Utente::da(r)?))).optional()?)
}

/// createSession: id e token nuovi, scadenza fra 30 giorni (un giorno se non va ricordata), indirizzo e browser di chi chiede.
pub fn crea_sessione(user_id: &str, non_ricordare: bool, ip: &str, browser: &str, ora: i64) -> Esito<Sessione> {
    let scadenza = ora + if non_ricordare { 3600 * 24 } else { super::cookie::SCADENZA_S } * 1000;
    let token = genera_id();
    db()?.execute(
        r#"INSERT INTO "session" (id, expiresAt, token, createdAt, updatedAt, ipAddress, userAgent, userId) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        params![genera_id(), iso_ms(scadenza), token, iso_ms(ora), iso_ms(ora), ip, browser, user_id],
    )?;
    Ok(trova_sessione(&token)?.expect("appena inserita").0)
}

pub fn elimina_sessione(token: &str) -> Esito<()> {
    db()?.execute(r#"DELETE FROM "session" WHERE token = ?"#, [token])?;
    Ok(())
}

pub fn elimina_sessioni_utente(user_id: &str) -> Esito<()> {
    db()?.execute(r#"DELETE FROM "session" WHERE userId = ?"#, [user_id])?;
    Ok(())
}

/// updateSession(token, { expiresAt, updatedAt }): la riga cambiata, o None se nel frattempo non c'è più.
pub fn rinnova_sessione(token: &str, scadenza: i64, ora: i64) -> Esito<Option<Sessione>> {
    let n = db()?.execute(r#"UPDATE "session" SET expiresAt = ?, updatedAt = ? WHERE token = ?"#, params![iso_ms(scadenza), iso_ms(ora), token])?;
    if n == 0 {
        return Ok(None);
    }
    Ok(trova_sessione(token)?.map(|(s, _)| s))
}

// ---- Verifiche (sfide del secondo fattore e delle passkey) ------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Verifica {
    pub id: String,
    pub value: String,
    pub expires_at: i64,
}

fn verifica_da(r: &Row) -> rusqlite::Result<Verifica> {
    Ok(Verifica { id: r.get("id")?, value: r.get("value")?, expires_at: ms_da(&r.get::<_, rusqlite::types::Value>("expiresAt")?).unwrap_or(0) })
}

pub fn crea_verifica(identificativo: &str, valore: &str, scadenza: i64, ora: i64) -> Esito<()> {
    db()?.execute(
        r#"INSERT INTO "verification" (id, identifier, value, expiresAt, createdAt, updatedAt) VALUES (?, ?, ?, ?, ?, ?)"#,
        params![genera_id(), identificativo, valore, iso_ms(scadenza), iso_ms(ora), iso_ms(ora)],
    )?;
    Ok(())
}

fn ultima(c: &Connection, identificativo: &str) -> Esito<Option<Verifica>> {
    Ok(c.query_row(r#"SELECT id, value, expiresAt FROM "verification" WHERE identifier = ? ORDER BY createdAt DESC LIMIT 1"#, [identificativo], verifica_da).optional()?)
}

/// findVerificationValue: l'ultima riga con quell'identificativo (anche scaduta), e intanto si tolgono tutte le righe scadute.
pub fn trova_verifica(identificativo: &str, ora: i64) -> Esito<Option<Verifica>> {
    let c = db()?;
    let v = ultima(&c, identificativo)?;
    c.execute(r#"DELETE FROM "verification" WHERE expiresAt < ?"#, [iso_ms(ora)])?;
    Ok(v)
}

/// consumeVerificationValue: l'ultima riga si prende una volta sola (tutte quelle con l'identificativo si tolgono); scaduta vale None.
pub fn consuma_verifica(identificativo: &str, ora: i64) -> Esito<Option<Verifica>> {
    let c = db()?;
    let v = ultima(&c, identificativo)?;
    let Some(v) = v else { return Ok(None) };
    let tolte = c.execute(r#"DELETE FROM "verification" WHERE id = ?"#, [&v.id])?;
    if tolte == 0 {
        return Ok(None);
    }
    c.execute(r#"DELETE FROM "verification" WHERE identifier = ?"#, [identificativo])?;
    Ok((v.expires_at >= ora).then_some(v))
}

pub fn elimina_verifica(identificativo: &str) -> Esito<()> {
    db()?.execute(r#"DELETE FROM "verification" WHERE identifier = ?"#, [identificativo])?;
    Ok(())
}

/// Lo schema di Better Auth con tutti i plugin (due fattori e passkey), come lo crea getMigrations su un database nuovo, e le colonne che
/// mancano a un database fatto senza i plugin. Si aggiunge e basta: niente si cambia o si toglie.
pub fn migra() -> Esito<()> {
    let c = db()?;
    let tabelle = [
        ("user", r#"CREATE TABLE "user" ("id" text not null primary key, "name" text not null, "email" text not null unique, "emailVerified" integer not null, "image" text, "createdAt" date not null, "updatedAt" date not null, "twoFactorEnabled" integer)"#),
        ("session", r#"CREATE TABLE "session" ("id" text not null primary key, "expiresAt" date not null, "token" text not null unique, "createdAt" date not null, "updatedAt" date not null, "ipAddress" text, "userAgent" text, "userId" text not null references "user" ("id") on delete cascade)"#),
        ("account", r#"CREATE TABLE "account" ("id" text not null primary key, "accountId" text not null, "providerId" text not null, "userId" text not null references "user" ("id") on delete cascade, "accessToken" text, "refreshToken" text, "idToken" text, "accessTokenExpiresAt" date, "refreshTokenExpiresAt" date, "scope" text, "password" text, "createdAt" date not null, "updatedAt" date not null)"#),
        ("verification", r#"CREATE TABLE "verification" ("id" text not null primary key, "identifier" text not null, "value" text not null, "expiresAt" date not null, "createdAt" date not null, "updatedAt" date not null)"#),
        ("twoFactor", r#"CREATE TABLE "twoFactor" ("id" text not null primary key, "secret" text not null, "backupCodes" text not null, "userId" text not null references "user" ("id") on delete cascade, "verified" integer, "failedVerificationCount" integer, "lockedUntil" date)"#),
        ("passkey", r#"CREATE TABLE "passkey" ("id" text not null primary key, "name" text, "publicKey" text not null, "userId" text not null references "user" ("id") on delete cascade, "credentialID" text not null, "counter" integer not null, "deviceType" text not null, "backedUp" integer not null, "transports" text, "createdAt" date, "aaguid" text)"#),
    ];
    let indici = [
        ("session_userId_idx", r#"CREATE INDEX "session_userId_idx" on "session" ("userId")"#),
        ("account_userId_idx", r#"CREATE INDEX "account_userId_idx" on "account" ("userId")"#),
        ("verification_identifier_idx", r#"CREATE INDEX "verification_identifier_idx" on "verification" ("identifier")"#),
        ("twoFactor_secret_idx", r#"CREATE INDEX "twoFactor_secret_idx" on "twoFactor" ("secret")"#),
        ("twoFactor_userId_idx", r#"CREATE INDEX "twoFactor_userId_idx" on "twoFactor" ("userId")"#),
        ("passkey_userId_idx", r#"CREATE INDEX "passkey_userId_idx" on "passkey" ("userId")"#),
        ("passkey_credentialID_idx", r#"CREATE INDEX "passkey_credentialID_idx" on "passkey" ("credentialID")"#),
    ];
    let esiste = |tipo: &str, nome: &str| -> Esito<bool> { Ok(c.query_row("SELECT 1 FROM sqlite_master WHERE type = ? AND name = ?", [tipo, nome], |_| Ok(())).optional()?.is_some()) };
    for (nome, sql) in tabelle {
        if !esiste("table", nome)? {
            c.execute_batch(sql)?;
        }
    }
    // le colonne dei plugin su un database fatto prima del secondo fattore
    // con il valore di partenza del campo, come getMigrations quando aggiunge una colonna
    let colonne = [("user", "twoFactorEnabled", "integer default 0"), ("twoFactor", "verified", "integer default 1"), ("twoFactor", "failedVerificationCount", "integer default 0"), ("twoFactor", "lockedUntil", "date")];
    for (tabella, colonna, tipo) in colonne {
        let presenti: Vec<String> = c.prepare(&format!(r#"PRAGMA table_info("{tabella}")"#))?.query_map([], |r| r.get::<_, String>("name"))?.collect::<Result<_, _>>()?;
        if !presenti.iter().any(|x| x == colonna) {
            c.execute_batch(&format!(r#"ALTER TABLE "{tabella}" ADD COLUMN "{colonna}" {tipo}"#))?;
        }
    }
    for (nome, sql) in indici {
        if !esiste("index", nome)? {
            c.execute_batch(sql)?;
        }
    }
    Ok(())
}
