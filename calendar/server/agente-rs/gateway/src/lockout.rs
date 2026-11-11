//! Tentativi falliti ammessi prima di bloccare l'account, e per quanto (era lib/lockout.ts). Il limite per indirizzo qui non serve: dietro
//! sslh e docker-proxy ogni client ha lo stesso indirizzo. Si conta per account: una password si indovina a cinque tentativi ogni quarto
//! d'ora, da qualunque indirizzo.

use crate::py::Esito;
use rusqlite::{params, Connection, OptionalExtension};

pub const MAX_FAILED_LOGINS: i64 = 5;
pub const LOCKOUT_MINUTES: i64 = 15;
const WINDOW_S: f64 = (LOCKOUT_MINUTES * 60) as f64;

pub fn migrate_lockout(db: &Connection) -> Esito<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS login_failures(
    email TEXT PRIMARY KEY,
    attempts INTEGER NOT NULL DEFAULT 0,
    window_start INTEGER NOT NULL DEFAULT 0,
    locked_until INTEGER NOT NULL DEFAULT 0
  )",
    )?;
    Ok(())
}

/// String(email ?? "").trim().toLowerCase()
pub fn normalize_email(email: Option<&oe_nucleo::json::Json>) -> String {
    let s = match email {
        None | Some(oe_nucleo::json::Json::Null) => String::new(),
        x => crate::py::string_js(x),
    };
    crate::py::js_trim(&s).to_lowercase()
}

/// Secondi che mancano alla fine del blocco, 0 se l'account non è bloccato.
pub fn locked_for(db: &Connection, email: &str, now: f64) -> Esito<i64> {
    let until: Option<f64> = db.query_row("SELECT locked_until FROM login_failures WHERE email = ?", [email], |r| r.get(0)).optional()?;
    Ok(match until {
        Some(u) if u != 0.0 && u > now => (u - now).ceil() as i64,
        _ => 0,
    })
}

/// Registra un accesso fallito. Il conteggio riparte da zero quando la finestra è scaduta o il blocco è finito.
pub fn register_failure(db: &Connection, email: &str, now: f64) -> Esito<()> {
    let riga: Option<(f64, f64, f64)> = db.query_row("SELECT attempts, window_start, locked_until FROM login_failures WHERE email = ?", [email], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
    let ricomincia = match riga {
        None => true,
        Some((_, inizio, fino)) => now - inizio > WINDOW_S || (fino > 0.0 && fino <= now),
    };
    let (attempts, start) = match riga {
        Some((a, inizio, _)) if !ricomincia => (a + 1.0, inizio),
        _ => (1.0, now),
    };
    let until = if attempts >= MAX_FAILED_LOGINS as f64 { now + WINDOW_S } else { 0.0 };
    db.execute(
        "INSERT INTO login_failures(email, attempts, window_start, locked_until) VALUES (?, ?, ?, ?)
     ON CONFLICT(email) DO UPDATE SET attempts = excluded.attempts, window_start = excluded.window_start, locked_until = excluded.locked_until",
        params![email, attempts as i64, start.floor() as i64, until.floor() as i64],
    )?;
    Ok(())
}

pub fn clear_failures(db: &Connection, email: &str) -> Esito<()> {
    db.execute("DELETE FROM login_failures WHERE email = ?", [email])?;
    Ok(())
}
