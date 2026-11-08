import type Database from "better-sqlite3";

// Tentativi falliti ammessi prima di bloccare l'account, e per quanto.
// Il limite per indirizzo qui non serve: dietro sslh e docker-proxy ogni client ha
// lo stesso indirizzo, quindi sarebbe un solo contatore per tutti. Si conta per account:
// una password si indovina a cinque tentativi ogni quarto d'ora, da qualunque indirizzo.
export const MAX_FAILED_LOGINS = 5;
export const LOCKOUT_MINUTES = 15;

const WINDOW_S = LOCKOUT_MINUTES * 60;

type Row = { attempts: number; window_start: number; locked_until: number };

export function migrateLockout(db: Database.Database) {
  db.exec(`CREATE TABLE IF NOT EXISTS login_failures(
    email TEXT PRIMARY KEY,
    attempts INTEGER NOT NULL DEFAULT 0,
    window_start INTEGER NOT NULL DEFAULT 0,
    locked_until INTEGER NOT NULL DEFAULT 0
  )`);
}

export const normalizeEmail = (email: unknown) => String(email ?? "").trim().toLowerCase();

/** Secondi che mancano alla fine del blocco, 0 se l'account non è bloccato. */
export function lockedFor(db: Database.Database, email: string, now = Date.now() / 1000): number {
  const until = db.prepare("SELECT locked_until FROM login_failures WHERE email = ?").pluck().get(email) as number | undefined;
  return until && until > now ? Math.ceil(until - now) : 0;
}

/**
 * Registra un accesso fallito. Il conteggio riparte da zero quando la finestra è scaduta o
 * il blocco è finito: senza questo un account arrivato una volta al limite avrebbe il quinto
 * errore a ogni password sbagliata successiva, e verrebbe rifiutato prima ancora di guardarla.
 */
export function registerFailure(db: Database.Database, email: string, now = Date.now() / 1000) {
  const row = db.prepare("SELECT attempts, window_start, locked_until FROM login_failures WHERE email = ?").get(email) as Row | undefined;
  const ricomincia = !row || now - row.window_start > WINDOW_S || (row.locked_until > 0 && row.locked_until <= now);
  const attempts = ricomincia ? 1 : row.attempts + 1;
  const start = ricomincia ? now : row.window_start;
  const until = attempts >= MAX_FAILED_LOGINS ? now + WINDOW_S : 0;
  db.prepare(
    `INSERT INTO login_failures(email, attempts, window_start, locked_until) VALUES (?, ?, ?, ?)
     ON CONFLICT(email) DO UPDATE SET attempts = excluded.attempts, window_start = excluded.window_start, locked_until = excluded.locked_until`,
  ).run(email, attempts, Math.floor(start), Math.floor(until));
}

export function clearFailures(db: Database.Database, email: string) {
  db.prepare("DELETE FROM login_failures WHERE email = ?").run(email);
}
