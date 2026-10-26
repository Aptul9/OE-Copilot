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
