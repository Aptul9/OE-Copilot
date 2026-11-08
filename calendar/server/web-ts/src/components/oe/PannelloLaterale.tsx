"use client";

import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { LARGHEZZA_PANNELLO, limitaLarghezza } from "@/lib/oe/logic";

const CHIAVE = "oe.larghezza-pannello";

/**
 * La colonna di sinistra (minicalendario e calendari) con una maniglia sul bordo per allargarla o stringerla: la larghezza resta in questo browser.
 * Trascinando, la larghezza si scrive sullo stile della colonna al più una volta per fotogramma e senza passare da React, e si manda un evento
 * resize perché FullCalendar rimisuri gli eventi (rimisura solo su quello: Calendario.tsx gli toglie i 100 ms di attesa). Misurato su un giorno con
 * 24 riunioni: ricalcolo del browser 0,3 ms a passo e rimisura di FullCalendar 2 ms. Stato e archivio arrivano una volta sola, al rilascio.
 */
export function PannelloLaterale({ children }: { children: ReactNode }) {
  const [larghezza, setLarghezza] = useState<number>(LARGHEZZA_PANNELLO.predefinita);
  const colonna = useRef<HTMLElement>(null);
  const etichetta = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    try {
      const v = Number(localStorage.getItem(CHIAVE));
      if (v) setLarghezza(limitaLarghezza(v));
    } catch {
      // senza archivio nel browser resta la larghezza di partenza
    }
  }, []);
  // FullCalendar rimisura la griglia solo quando la finestra cambia: gli si dice che la colonna accanto è cambiata
  const primo = useRef(true);
  useEffect(() => {
    if (primo.current) {
      primo.current = false;
      return;
    }
    window.dispatchEvent(new Event("resize"));
  }, [larghezza]);
  const imposta = (px: number) => {
    const v = limitaLarghezza(px);
    setLarghezza(v);
    try {
      localStorage.setItem(CHIAVE, String(v));
    } catch {
      // vale in questa pagina e non resta
    }
  };
  const trascina = (e: PointerEvent<HTMLDivElement>) => {
    const el = colonna.current;
    if (!el) return;
    e.preventDefault();
    const maniglia = e.currentTarget;
    const x0 = e.clientX;
    const l0 = larghezza;
    let ultima = l0;
    let fotogramma = 0;
    maniglia.setPointerCapture(e.pointerId);
    // durante il trascinamento il puntatore non deve scatenare passaggi del mouse e selezioni sul calendario (calendario.css)
    document.body.setAttribute("data-trascina-pannello", "");
    const mostra = () => {
      fotogramma = 0;
      el.style.width = `${ultima}px`;
      window.dispatchEvent(new Event("resize"));
      if (etichetta.current) etichetta.current.textContent = `${ultima} px`;
    };
    const muovi = (m: globalThis.PointerEvent) => {
      ultima = limitaLarghezza(l0 + m.clientX - x0);
      if (!fotogramma) fotogramma = requestAnimationFrame(mostra);
    };
    const chiudi = (conferma: boolean) => {
      maniglia.removeEventListener("pointermove", muovi);
      maniglia.removeEventListener("pointerup", rilascia);
      maniglia.removeEventListener("pointercancel", annulla);
      cancelAnimationFrame(fotogramma);
      document.body.removeAttribute("data-trascina-pannello");
      if (conferma) {
        el.style.width = `${ultima}px`;
        imposta(ultima);
      } else el.style.width = `${l0}px`;
    };
    const rilascia = () => chiudi(true);
    const annulla = () => chiudi(false);
    maniglia.addEventListener("pointermove", muovi);
    maniglia.addEventListener("pointerup", rilascia);
    maniglia.addEventListener("pointercancel", annulla);
  };
  const tasto = (e: KeyboardEvent<HTMLDivElement>) => {
    const passo = e.key === "ArrowLeft" ? -16 : e.key === "ArrowRight" ? 16 : 0;
    if (!passo) return;
    e.preventDefault();
    imposta(larghezza + passo);
  };
  return (
    <>
      <aside ref={colonna} id="sidepane" className="sidepane hidden flex-none overflow-y-auto border-r bg-sidebar py-3 lg:block" style={{ width: larghezza }}>
        {children}
      </aside>
      <div
        id="sidepane-resize"
        role="separator"
        aria-orientation="vertical"
        aria-label="Larghezza del pannello"
        aria-valuemin={LARGHEZZA_PANNELLO.min}
        aria-valuemax={LARGHEZZA_PANNELLO.max}
        aria-valuenow={larghezza}
        tabIndex={0}
        title="Trascina per ridimensionare, doppio clic per ripristinare"
        className="group relative z-10 -mx-[3px] hidden w-1.5 flex-none cursor-col-resize touch-none select-none lg:block"
        onPointerDown={trascina}
        onKeyDown={tasto}
        onDoubleClick={() => imposta(LARGHEZZA_PANNELLO.predefinita)}
      >
        <div className="mx-auto h-full w-0.5 bg-transparent group-hover:bg-primary/50 group-focus-visible:bg-primary group-active:bg-primary" />
        <span
          ref={etichetta}
          className="pointer-events-none absolute left-1/2 top-3 hidden -translate-x-1/2 whitespace-nowrap rounded-md bg-primary px-1.5 py-0.5 text-[11px] font-semibold text-primary-foreground shadow group-active:block"
        />
      </div>
    </>
  );
}
