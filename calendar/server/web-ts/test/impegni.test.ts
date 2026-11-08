import { describe, expect, it } from "vitest";
import type { Ev } from "@/lib/core/caselle";
import { impegni } from "@/lib/core/impegni";

// Gli orari occupati della pagina pubblica: un giorno intero e una finestra da un giorno in su (un change di ServiceNow di sei settimane) non
// rendono occupato; una riunione lunga, anche a cavallo della mezzanotte, sì.

const ev = (start: string, end: string, extra: Partial<Ev> = {}): Ev => ({ id: start, title: "x", start, end, allDay: false, account: "a@x.it", ...extra }) as Ev;
const SETTIMANA = { a: 2026, m: 10, g: 26 };

describe("impegni", () => {
  it("una finestra di sei settimane non occupa la settimana, le riunioni dentro sì", () => {
    const r = impegni(
      [
        ev("2026-10-19T14:45:54", "2026-11-30T09:20:04"),
        ev("2026-10-26T09:00:00", "2026-11-30T18:00:00"),
        ev("2026-10-26T10:00:00", "2026-10-26T11:00:00"),
      ],
      SETTIMANA,
      7,
    );
    expect(r).toEqual([["2026-10-26T10:00:00", "2026-10-26T11:00:00"]]);
  });
  it("sotto le 24 ore conta, anche di notte; da 24 ore in su e un giorno intero no", () => {
    const r = impegni(
      [
        ev("2026-10-27T22:00:00", "2026-10-28T21:59:59"),
        ev("2026-10-29T09:00:00", "2026-10-30T09:00:00"),
        ev("2026-10-30", "2026-10-31", { allDay: true }),
      ],
      SETTIMANA,
      7,
    );
    expect(r).toEqual([["2026-10-27T22:00:00", "2026-10-28T21:59:59"]]);
  });
});

describe("blocchi voluti", () => {
  const mio = (start: string, end: string, extra: Partial<Ev> = {}) => ev(start, end, { is_local: true, account: "Altro", blocca: true, ...extra });
  it("un evento personale di un giorno intero occupa da mezzanotte a mezzanotte, anche salvato con le ore", () => {
    expect(impegni([mio("2026-10-27", "2026-10-28", { allDay: true })], SETTIMANA, 7)).toEqual([["2026-10-27T00:00:00", "2026-10-28T00:00:00"]]);
    expect(impegni([mio("2026-10-27T09:00", "2026-10-27T10:00", { allDay: true })], SETTIMANA, 7)).toEqual([["2026-10-27T00:00:00", "2026-10-28T00:00:00"]]);
  });
  it("più giorni personali e un impegno personale da 24 ore in su contano tutti", () => {
    expect(impegni([mio("2026-10-27", "2026-10-29", { allDay: true })], SETTIMANA, 7)).toEqual([["2026-10-27T00:00:00", "2026-10-29T00:00:00"]]);
    expect(impegni([mio("2026-10-29T08:00:00", "2026-10-30T20:00:00")], SETTIMANA, 7)).toEqual([["2026-10-29T08:00:00", "2026-10-30T20:00:00"]]);
  });
  it("un evento personale senza Blocca il tempo non conta, nemmeno di un'ora", () => {
    expect(impegni([mio("2026-10-27T09:00:00", "2026-10-27T10:00:00", { blocca: false }), mio("2026-10-28", "2026-10-29", { allDay: true, blocca: false })], SETTIMANA, 7)).toEqual([]);
  });
  it("un evento personale senza la scelta (fatto prima) resta come prima: un'ora conta, un giorno intero no", () => {
    const r = impegni([mio("2026-10-27T09:00:00", "2026-10-27T10:00:00", { blocca: undefined }), mio("2026-10-28", "2026-10-29", { allDay: true, blocca: undefined })], SETTIMANA, 7);
    expect(r).toEqual([["2026-10-27T09:00:00", "2026-10-27T10:00:00"]]);
  });
  it("dalle caselle conta un giorno intero o un impegno lungo segnato Occupato; libero o provvisorio no", () => {
    const r = impegni(
      [
        ev("2026-10-27", "2026-10-28", { allDay: true, occupato: true }),
        ev("2026-10-28", "2026-10-29", { allDay: true, occupato: false }),
        ev("2026-10-29T09:00:00", "2026-10-30T18:00:00", { occupato: true }),
        ev("2026-10-26T09:00:00", "2026-11-30T18:00:00", { occupato: false }),
      ],
      SETTIMANA,
      7,
    );
    expect(r).toEqual([
      ["2026-10-27T00:00:00", "2026-10-28T00:00:00"],
      ["2026-10-29T09:00:00", "2026-10-30T18:00:00"],
    ]);
  });
});
