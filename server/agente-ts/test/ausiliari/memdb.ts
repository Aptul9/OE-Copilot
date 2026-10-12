// Un calendar.db come quello del telefono (tabelle Calendars, Events, Attendees), costruito con node:sqlite da righe in memoria: stesse tabelle
// di server/strumenti/prova_agente_golden.py, così le prove passano dalla vera lettura SQL dell'agente.

import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { RigaDb } from "../../src/index.ts";

export interface Db {
  calendari: [number, string, string, string, number, number, number][];
  eventi: (RigaDb & { deleted?: number })[];
  invitati: [number, string | null, string | null, number][];
}
export interface Fonte {
  id: string;
  nome: string;
  account: string;
  calendari?: string[];
}

const SCHEMA = `
  CREATE TABLE Calendars (_id INTEGER PRIMARY KEY, account_name TEXT, account_type TEXT, calendar_displayName TEXT,
                          isPrimary INTEGER, deleted INTEGER DEFAULT 0, sync_events INTEGER DEFAULT 1);
  CREATE TABLE Events (_id INTEGER PRIMARY KEY, _sync_id TEXT, calendar_id INTEGER, title TEXT, eventLocation TEXT,
                       description TEXT, eventStatus INTEGER, dtstart INTEGER, dtend INTEGER, eventTimezone TEXT,
                       eventEndTimezone TEXT, duration TEXT, allDay INTEGER DEFAULT 0, rrule TEXT, rdate TEXT,
                       exrule TEXT, exdate TEXT, original_id INTEGER, original_sync_id TEXT,
                       originalInstanceTime INTEGER, originalAllDay INTEGER, organizer TEXT, deleted INTEGER DEFAULT 0);
  CREATE TABLE Attendees (_id INTEGER PRIMARY KEY, event_id INTEGER, attendeeName TEXT, attendeeEmail TEXT,
                          attendeeRelationship INTEGER);
`;

function riempi(sql: DatabaseSync, db: Db): void {
  sql.exec(SCHEMA);
  const calendario = sql.prepare("INSERT INTO Calendars VALUES (?,?,?,?,?,?,?)");
  for (const r of db.calendari) calendario.run(...r);
  for (const r of db.eventi) {
    const colonne = Object.keys(r);
    sql.prepare(`INSERT INTO Events (${colonne.join(",")}) VALUES (${colonne.map(() => "?").join(",")})`).run(...(Object.values(r) as (string | number | null)[]));
  }
  const invitato = sql.prepare("INSERT INTO Attendees (event_id, attendeeName, attendeeEmail, attendeeRelationship) VALUES (?,?,?,?)");
  for (const r of db.invitati) invitato.run(...r);
}

export function creaDbInMemoria(db: Db): DatabaseSync {
  const sql = new DatabaseSync(":memory:");
  riempi(sql, db);
  return sql;
}

/** Scrive calendar.db nella cartella (la cartella c'è già). */
export function scriviDb(cartella: string, db: Db): string {
  const file = path.join(cartella, "calendar.db");
  fs.rmSync(file, { force: true });
  const sql = new DatabaseSync(file);
  riempi(sql, db);
  sql.close();
  return file;
}
