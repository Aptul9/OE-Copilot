"use client";

import type { CalendarApi } from "@fullcalendar/core";
import { useEffect, type RefObject } from "react";

/**
 * Scorrimento di un giorno alla volta nella vista a 3 giorni: rotellina o trackpad in orizzontale,
 * trascinamento col mouse, la penna o il dito, frecce. L'asse delle ore resta fermo perché cambia
 * solo il periodo mostrato. `api()` restituisce l'API di FullCalendar solo quando la vista a 3 giorni
 * è quella attiva, altrimenti null e i gesti non fanno niente.
 */
export function useScorrimentoTreGiorni(radice: RefObject<HTMLElement | null>, api: () => CalendarApi | null) {
  useEffect(() => {
    const el = radice.current;
    if (!el) return;
    const sposta = (n: number) => {
      if (n) api()?.incrementDate({ days: n });
    };

    // Rotellina e trackpad: un giorno per gesto. Una spinta del trackpad manda eventi per quasi un
    // secondo (inerzia): finché ne arrivano si resta fermi, altrimenti un colpetto salterebbe una settimana.
    let ruotato = false;
    let accumulo = 0;
    let fine: ReturnType<typeof setTimeout> | undefined;
    const rotella = (e: WheelEvent) => {
      const dx = e.deltaX || (e.shiftKey ? e.deltaY : 0);
      if (!api() || !dx || (!e.shiftKey && Math.abs(dx) <= Math.abs(e.deltaY))) return;
      e.preventDefault();   // niente scorrimento laterale del browser, né "indietro" a due dita
      clearTimeout(fine);
      fine = setTimeout(() => {
        ruotato = false;
        accumulo = 0;
      }, 140);
      if (ruotato) return;
      accumulo += dx;
      if (Math.abs(accumulo) < 40) return;
      ruotato = true;
      sposta(accumulo > 0 ? 1 : -1);
    };

    // Trascinamento: un giorno ogni larghezza di colonna. Il movimento verticale resta allo
    // scorrimento normale della griglia.
    let presa: { id: number; x: number; y: number; larghezza: number; fatti: number; attiva: boolean } | null = null;
    const giu = (e: PointerEvent) => {
      const t = e.target as HTMLElement;
      if (!api() || (e.pointerType === "mouse" && e.button !== 0)) return;
      if (t.closest(".fc-event, a, button") || !t.closest(".fc-timegrid-body, .fc-col-header")) return;
      const colonna = el.querySelector(".fc-timegrid-col[data-date]");
      const larghezza = colonna ? colonna.getBoundingClientRect().width : 0;
      if (larghezza) presa = { id: e.pointerId, x: e.clientX, y: e.clientY, larghezza, fatti: 0, attiva: false };
    };
    const muovi = (e: PointerEvent) => {
      if (!presa || e.pointerId !== presa.id) return;
      const dx = e.clientX - presa.x;
      if (!presa.attiva) {
        if (Math.abs(dx) < 8 || Math.abs(dx) < Math.abs(e.clientY - presa.y)) return;
        presa.attiva = true;
        el.dataset.trascina = "1";
      }
      const voluti = Math.round(-dx / presa.larghezza);
      sposta(voluti - presa.fatti);
      presa.fatti = voluti;
    };
    const lascia = (e: PointerEvent) => {
      if (!presa || e.pointerId !== presa.id) return;
      presa = null;
      delete el.dataset.trascina;
    };

    const tasto = (e: KeyboardEvent) => {
      if (!api() || !el.offsetParent || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;   // offsetParent: calendario nascosto (Timesheet)
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      const t = e.target as HTMLElement;
      if (t.closest("input, textarea, select, [contenteditable], [role=dialog]")) return;
      e.preventDefault();
      sposta(e.key === "ArrowRight" ? 1 : -1);
    };

    el.addEventListener("wheel", rotella, { passive: false });
    el.addEventListener("pointerdown", giu);
    window.addEventListener("pointermove", muovi);
    window.addEventListener("pointerup", lascia);
    window.addEventListener("pointercancel", lascia);
    document.addEventListener("keydown", tasto);
    return () => {
      clearTimeout(fine);
      el.removeEventListener("wheel", rotella);
      el.removeEventListener("pointerdown", giu);
      window.removeEventListener("pointermove", muovi);
      window.removeEventListener("pointerup", lascia);
      window.removeEventListener("pointercancel", lascia);
      document.removeEventListener("keydown", tasto);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
