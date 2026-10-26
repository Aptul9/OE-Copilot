import { describe, expect, it } from "vitest";
import { estrai, eventiDaLettura, type RigaDb } from "../src/index.ts";
import { creaDbInMemoria } from "./ausiliari/memdb.ts";

// "Mostra come" di Outlook e Google (colonna availability di calendar.db): un evento segnato "Occupato" esce con occupato true, uno libero o
// provvisorio con false. Il gateway lo usa per contare nella disponibilità un giorno intero o un impegno lungo che si è voluto bloccare.
// Senza la colonna (i database di prova dei golden) gli eventi restano come prima, senza il campo.

const ms = (iso: string) => Date.parse(iso);
const riga = (extra: Partial<RigaDb>): RigaDb =>
  ({
    _sync_id: null, calendar_id: 1, eventLocation: null, description: null, eventStatus: null, eventEndTimezone: null, duration: null,
    allDay: 0, rrule: null, rdate: null, exrule: null, exdate: null, original_id: null, original_sync_id: null, originalInstanceTime: null,
    originalAllDay: 0, organizer: "mario@alfa.example", isOrganizer: 1, hasAttendeeData: 0, eventTimezone: "Europe/Rome", ...extra,
  }) as RigaDb;
const RIGHE = [
  riga({ _id: 1, title: "Ferie", allDay: 1, eventTimezone: "UTC", dtstart: ms("2026-11-05T00:00:00Z"), dtend: ms("2026-11-07T00:00:00Z") }),
  riga({ _id: 2, title: "Compleanno", allDay: 1, eventTimezone: "UTC", dtstart: ms("2026-11-05T00:00:00Z"), dtend: ms("2026-11-06T00:00:00Z") }),
  riga({ _id: 3, title: "Change", dtstart: ms("2026-11-04T08:00:00Z"), dtend: ms("2026-11-20T17:00:00Z") }),
];
const DISPONIBILITA: Record<number, number> = { 1: 0, 2: 1, 3: 2 };

function leggi(conColonna: boolean) {
  const sql = creaDbInMemoria({ calendari: [[1, "mario@alfa.example", "com.microsoft.office.outlook", "Calendar", 1, 0, 1]], eventi: RIGHE, invitati: [] });
  try {
    if (conColonna) {
      sql.exec("ALTER TABLE Events ADD COLUMN availability INTEGER DEFAULT 0");
      for (const [id, v] of Object.entries(DISPONIBILITA)) sql.prepare("UPDATE Events SET availability = ? WHERE _id = ?").run(v, Number(id));
    }
    const lettura = estrai(sql, "mario@alfa.example", [])!;
    const eventi = eventiDaLettura(lettura, { id: "android::mario@alfa.example", nome: "Alfa", account: "mario@alfa.example", tipo: "outlook" }, "2026-11-01T00:00:00", "2026-12-01T00:00:00", "Europe/Rome");
    const per = Object.fromEntries(eventi.map((e) => [e.title as string, e]));
    return (t: string) => per[t]!;
  } finally {
    sql.close();
  }
}

describe("occupato dal Mostra come", () => {
  it("occupato true solo per la riga segnata Occupato; libero e provvisorio false", () => {
    const e = leggi(true);
    expect(e("Ferie").occupato).toBe(true);
    expect(e("Compleanno").occupato).toBe(false);
    expect(e("Change").occupato).toBe(false);
  });
  it("senza la colonna niente campo, e il resto dell'evento è lo stesso", () => {
    const con = leggi(true);
    const senza = leggi(false);
    for (const t of ["Ferie", "Compleanno", "Change"]) {
      expect("occupato" in senza(t)).toBe(false);
      const { occupato: _, ...resto } = con(t);
      expect(resto).toEqual(senza(t));
    }
  });
});
