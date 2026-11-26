import { Calendar } from "@fullcalendar/core";
import dayGridPlugin from "@fullcalendar/daygrid";
import listPlugin from "@fullcalendar/list";
import timeGridPlugin from "@fullcalendar/timegrid";
import { describe, expect, it, vi } from "vitest";
import { VISTE_FC, aOggi, unPasso } from "@/components/oe/Calendario";
import { chiaveGiorno, giorniTra } from "@/lib/oe/logic";

// FullCalendar vero senza disegnare: le date passano dal suo riduttore con le viste della pagina. Il periodo disegnato di una vista che
// scorre comprende i giorni nascosti ai lati, quindi si guarda di quanto si sposta a ogni freccia, non dove comincia.
const calendario = (vista: string) =>
  new Calendar({} as HTMLElement, { plugins: [dayGridPlugin, timeGridPlugin, listPlugin], views: VISTE_FC, initialView: vista, initialDate: "2026-10-21", firstDay: 1 });

describe("frecce della barra", () => {
  // un giorno per freccia in Giorno e a 3 giorni, una settimana in Settimana; la corsa attraversa il cambio dell'ora del 25 ottobre
  it.each([
    ["timeGridGiorno", 1],
    ["timeGridTreGiorni", 1],
    ["timeGridSettimana", 7],
  ])("%s: ogni freccia sposta di %i giorni, in avanti come indietro", (vista, passo) => {
    const cal = calendario(vista);
    const versi = [1, 1, 1, -1, -1, -1, -1] as const;
    const spostamenti = versi.map((verso) => {
      const prima = cal.view.activeStart;
      unPasso(cal, verso);
      return giorniTra(prima, cal.view.activeStart);
    });
    expect(spostamenti).toEqual(versi.map((v) => v * passo));
    // il ridisegno dello zoom (changeView sulla stessa vista) e il dito (incrementDate) partono da dove le frecce sono arrivate
    const qui = cal.view.activeStart;
    cal.changeView(cal.view.type);
    expect(chiaveGiorno(cal.view.activeStart)).toBe(chiaveGiorno(qui));
    cal.incrementDate({ days: passo });
    expect(giorniTra(qui, cal.view.activeStart)).toBe(passo);
  });

  it("Mese e Agenda vanno di un mese e di una settimana", () => {
    const mese = calendario("dayGridMonth");
    unPasso(mese, 1);
    unPasso(mese, 1);
    unPasso(mese, -1);
    expect(chiaveGiorno(mese.getDate())).toBe("2026-11-01");
    const agenda = calendario("listWeek");
    unPasso(agenda, 1);
    expect(chiaveGiorno(agenda.view.activeStart)).toBe("2026-10-26");
  });
});

describe("Oggi", () => {
  const adesso = new Date(2026, 10, 25, 14, 20);

  it.each(["timeGridGiorno", "timeGridTreGiorni", "timeGridSettimana"])("%s: torna a oggi e all'ora di adesso, un'ora prima in cima", (vista) => {
    vi.useFakeTimers({ now: adesso, toFake: ["Date"] });
    try {
      const cal = calendario(vista);
      const scorri = vi.spyOn(cal, "scrollToTime");
      aOggi(cal, adesso);
      expect(chiaveGiorno(cal.getDate())).toBe("2026-11-25");
      expect(scorri).toHaveBeenCalledWith({ minutes: 13 * 60 + 20 });
    } finally {
      vi.useRealTimers();
    }
  });

  it("Mese e Agenda: solo la data, niente ore da scorrere", () => {
    const cal = calendario("dayGridMonth");
    const scorri = vi.spyOn(cal, "scrollToTime");
    aOggi(cal, adesso);
    expect(scorri).not.toHaveBeenCalled();
  });
});
