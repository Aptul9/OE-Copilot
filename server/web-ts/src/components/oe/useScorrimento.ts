"use client";

import type { CalendarApi } from "@fullcalendar/core";
import { useEffect, type RefObject } from "react";
import { chiaveGiorno, giorniDiArrivo, spostaGiorni } from "@/lib/oe/logic";

/** Una vista che scorre: giorni che si vedono, giorni in più per lato che FullCalendar disegna nascosti, giorni di un passo. */
export type Scorrimento = { visibili: number; bordo: number; passo: number };

const DURATA = 230; // ms della corsa fino al passo di arrivo

/**
 * Le viste a colonne di giorni scorrono sotto il dito, come in Outlook. FullCalendar disegna `bordo` giorni in più per lato; le tabelle delle
 * colonne (intestazione e griglia) sono larghe quanto tutti quei giorni e si spostano insieme (left, calendario.css dice perché), mentre l'asse
 * delle ore resta fermo. Al rilascio la griglia corre al passo più vicino, o più in là con un colpo veloce; arrivata, la data cambia di quei
 * giorni e le tabelle tornano al riposo appena FullCalendar ha ridisegnato (MutationObserver: prima che il browser disegni).
 * Dito, penna e mouse trascinano anche sopra le riunioni, e il tocco che ha trascinato non apre la riunione; rotellina e trackpad in
 * orizzontale spostano allo stesso modo; le frecce fanno un passo. `vista()` è null quando la vista attiva non scorre.
 */
export function useScorrimento(radice: RefObject<HTMLElement | null>, vista: () => { api: CalendarApi; s: Scorrimento } | null) {
  useEffect(() => {
    const el = radice.current;
    if (!el) return;
    el.style.setProperty("--durata", `${DURATA}ms`);

    let fermo = false; // trascinamento, rotellina o corsa in atto: niente riallineamenti
    let attesa: string | null = null; // la prima colonna che FullCalendar deve mostrare dopo un cambio di data
    let scadenza: ReturnType<typeof setTimeout> | undefined;
    let trascinato = -Infinity;

    // sulle due tabelle e non con una variabile sulla radice: a ogni movimento si ricalcolano due elementi, non tutto il calendario
    const posa = (x: number, corsa: boolean) => {
      el.classList.toggle("scivola", corsa);
      for (const t of el.querySelectorAll<HTMLElement>(".fc-col-header, .fc-timegrid-cols > table")) t.style.left = `${x}px`;
    };
    // la larghezza di una colonna e delle tabelle, e il punto di riposo: i giorni nascosti di sinistra fuori dalla vista
    const misura = (s: Scorrimento) => {
      const sc = el.querySelector<HTMLElement>(".fc-scroller-liquid-absolute");
      const asse = el.querySelector<HTMLElement>(".fc-timegrid-slot-label")?.getBoundingClientRect().width;
      if (!sc || !asse) return null;
      const giorni = sc.clientWidth - asse;
      const colonna = giorni / s.visibili;
      el.style.setProperty("--asse", `${asse}px`);
      el.style.setProperty("--giorni", `${giorni}px`);
      el.style.setProperty("--tabella", `${asse + colonna * (s.visibili + 2 * s.bordo)}px`);
      return { colonna, riposo: -colonna * s.bordo, bordo: s.bordo };
    };
    const primaColonna = () => el.querySelector<HTMLElement>(".fc-col-header-cell[data-date]")?.dataset.date ?? null;

    const allinea = () => {
      if (fermo) return;
      const v = vista();
      if (!v) return;
      if (attesa && primaColonna() !== attesa) return; // FullCalendar non ha ancora ridisegnato: la griglia resta dove è arrivata
      attesa = null;
      clearTimeout(scadenza);
      const m = misura(v.s);
      if (m) posa(m.riposo, false);
    };

    const arriva = (giorni: number, riposo: number, colonna: number) => {
      fermo = true;
      posa(riposo - giorni * colonna, true);
      setTimeout(() => {
        fermo = false;
        const v = vista();
        if (!giorni || !v) return allinea();
        const prima = primaColonna();
        if (prima) {
          const [a, m, g] = prima.split("-").map(Number);
          attesa = chiaveGiorno(spostaGiorni(new Date(a, m - 1, g), giorni));
        }
        // se FullCalendar non cambia data la griglia torna comunque a riposo
        scadenza = setTimeout(() => {
          attesa = null;
          allinea();
        }, 600);
        v.api.incrementDate({ days: giorni });
      }, DURATA);
    };

    // Trascinamento: la griglia segue il dito, mai oltre i giorni disegnati. Il movimento verticale resta allo scorrimento della griglia.
    let presa: { id: number; x: number; y: number; attiva: boolean; colonna: number; riposo: number; bordo: number; campioni: [number, number][] } | null = null;
    const giu = (e: PointerEvent) => {
      const v = vista();
      if (!v || fermo || (e.pointerType === "mouse" && e.button !== 0)) return;
      const t = e.target as HTMLElement;
      if (!t.closest(".fc-timegrid-body, .fc-col-header") || t.closest("a[href], button")) return;
      const m = misura(v.s);
      if (m) presa = { id: e.pointerId, x: e.clientX, y: e.clientY, attiva: false, ...m, campioni: [[e.timeStamp, e.clientX]] };
    };
    const muovi = (e: PointerEvent) => {
      if (!presa || e.pointerId !== presa.id) return;
      const dx = e.clientX - presa.x;
      if (!presa.attiva) {
        if (Math.abs(dx) < 8 || Math.abs(dx) < Math.abs(e.clientY - presa.y)) return;
        presa.attiva = true;
        fermo = true;
        el.dataset.trascina = "1";
      }
      presa.campioni.push([e.timeStamp, e.clientX]);
      if (presa.campioni.length > 6) presa.campioni.shift();
      const lim = presa.colonna * presa.bordo;
      posa(presa.riposo + Math.max(-lim, Math.min(lim, dx)), false);
    };
    const lascia = (e: PointerEvent) => {
      if (!presa || e.pointerId !== presa.id) return;
      const p = presa;
      presa = null;
      if (!p.attiva) return;
      delete el.dataset.trascina;
      trascinato = e.timeStamp;
      fermo = false;
      const v = vista();
      if (!v) return allinea();
      // pointercancel non ha la posizione: vale l'ultima vista, senza slancio
      const [tFine, xFine] = e.type === "pointerup" ? [e.timeStamp, e.clientX] : p.campioni[p.campioni.length - 1];
      const recenti = p.campioni.filter(([t]) => tFine - t <= 100);
      const velocita = e.type === "pointerup" && recenti.length && tFine > recenti[0][0] ? (xFine - recenti[0][1]) / (tFine - recenti[0][0]) : 0;
      arriva(giorniDiArrivo(xFine - p.x, velocita, p.colonna, v.s.passo, v.s.bordo), p.riposo, p.colonna);
    };
    // il clic che chiude un trascinamento non apre la riunione su cui il dito si è fermato
    const clic = (e: MouseEvent) => {
      if (e.timeStamp - trascinato < 350) {
        e.stopPropagation();
        e.preventDefault();
      }
    };

    // Rotellina e trackpad in orizzontale: la griglia segue, e quando i colpi finiscono va al passo più vicino. L'inerzia del trackpad
    // manda colpi per quasi un secondo: finché ne arrivano la griglia continua a seguirli.
    let ruota: { dx: number; colonna: number; riposo: number; bordo: number } | null = null;
    let fineRuota: ReturnType<typeof setTimeout> | undefined;
    const rotella = (e: WheelEvent) => {
      const v = vista();
      const d = e.deltaX || (e.shiftKey ? e.deltaY : 0);
      if (!v || !d || (!e.shiftKey && Math.abs(d) <= Math.abs(e.deltaY))) return;
      e.preventDefault(); // niente scorrimento laterale del browser, né "indietro" a due dita
      if (presa?.attiva || (fermo && !ruota)) return;
      if (!ruota) {
        const m = misura(v.s);
        if (!m) return;
        ruota = { dx: 0, ...m };
        fermo = true;
      }
      const lim = ruota.colonna * ruota.bordo;
      ruota.dx = Math.max(-lim, Math.min(lim, ruota.dx - d));
      posa(ruota.riposo + ruota.dx, false);
      clearTimeout(fineRuota);
      fineRuota = setTimeout(() => {
        const r = ruota!;
        ruota = null;
        fermo = false;
        const ora = vista();
        if (ora) arriva(giorniDiArrivo(r.dx, 0, r.colonna, ora.s.passo, ora.s.bordo), r.riposo, r.colonna);
        else allinea();
      }, 140);
    };

    const tasto = (e: KeyboardEvent) => {
      const v = vista();
      if (!v || fermo || !el.offsetParent || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return; // offsetParent: calendario nascosto (Timesheet)
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      const t = e.target as HTMLElement;
      if (t.closest("input, textarea, select, [contenteditable], [role=dialog], [role=separator]")) return;
      e.preventDefault();
      const m = misura(v.s);
      if (m) arriva((e.key === "ArrowRight" ? 1 : -1) * v.s.passo, m.riposo, m.colonna);
    };

    const osserva = new MutationObserver(allinea);
    osserva.observe(el, { childList: true, subtree: true, attributes: true, attributeFilter: ["data-date"] });
    const dimensioni = new ResizeObserver(allinea);
    dimensioni.observe(el);
    el.addEventListener("wheel", rotella, { passive: false });
    el.addEventListener("pointerdown", giu);
    el.addEventListener("click", clic, true);
    window.addEventListener("pointermove", muovi);
    window.addEventListener("pointerup", lascia);
    window.addEventListener("pointercancel", lascia);
    document.addEventListener("keydown", tasto);
    allinea();
    return () => {
      clearTimeout(scadenza);
      clearTimeout(fineRuota);
      osserva.disconnect();
      dimensioni.disconnect();
      el.removeEventListener("wheel", rotella);
      el.removeEventListener("pointerdown", giu);
      el.removeEventListener("click", clic, true);
      window.removeEventListener("pointermove", muovi);
      window.removeEventListener("pointerup", lascia);
      window.removeEventListener("pointercancel", lascia);
      document.removeEventListener("keydown", tasto);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
