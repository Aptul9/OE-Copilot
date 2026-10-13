"use client";

import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { LARGHEZZA_PANNELLO, limitaLarghezza } from "@/lib/oe/logic";
import { cn } from "@/lib/utils";

const CHIAVE = "oe.larghezza-pannello";

/**
 * La colonna di sinistra (minicalendario e calendari) con una maniglia sul bordo per allargarla o stringerla: la larghezza resta in questo browser.
 * Mentre si trascina si muove solo la maniglia (con la larghezza che avrà): cambiare la larghezza della colonna a ogni passo farebbe ridisegnare
 * a ogni passo tutta la griglia degli eventi, che è la parte lenta. La larghezza vera si applica al rilascio, una volta.
 */
export function PannelloLaterale({ children }: { children: ReactNode }) {
  const [larghezza, setLarghezza] = useState<number>(LARGHEZZA_PANNELLO.predefinita);
  const [anteprima, setAnteprima] = useState<number | null>(null);
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
    e.preventDefault();
    const maniglia = e.currentTarget;
    const x0 = e.clientX;
    const l0 = larghezza;
    let ultima = l0;
    maniglia.setPointerCapture(e.pointerId);
    const muovi = (m: globalThis.PointerEvent) => {
      ultima = limitaLarghezza(l0 + m.clientX - x0);
      setAnteprima(ultima);
    };
    const chiudi = () => {
      maniglia.removeEventListener("pointermove", muovi);
      maniglia.removeEventListener("pointerup", rilascia);
      maniglia.removeEventListener("pointercancel", annulla);
      setAnteprima(null);
    };
    const rilascia = () => {
      chiudi();
      imposta(ultima);
    };
    const annulla = () => chiudi();
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
      <aside id="sidepane" className="sidepane hidden flex-none overflow-y-auto border-r bg-sidebar py-3 lg:block" style={{ width: larghezza }}>
        {children}
      </aside>
      <div
        id="sidepane-resize"
        role="separator"
        aria-orientation="vertical"
        aria-label="Larghezza della colonna dei calendari"
        aria-valuemin={LARGHEZZA_PANNELLO.min}
        aria-valuemax={LARGHEZZA_PANNELLO.max}
        aria-valuenow={larghezza}
        tabIndex={0}
        title="Trascina per cambiare la larghezza, doppio clic per ripristinarla"
        className="group relative z-10 -mx-[3px] hidden w-1.5 flex-none cursor-col-resize touch-none select-none lg:block"
        // solo trasformazione: non sposta niente nella pagina, quindi non la ridisegna
        style={anteprima === null ? undefined : { transform: `translateX(${anteprima - larghezza}px)` }}
        onPointerDown={trascina}
        onKeyDown={tasto}
        onDoubleClick={() => imposta(LARGHEZZA_PANNELLO.predefinita)}
      >
        <div
          className={cn(
            "mx-auto h-full w-0.5 bg-transparent group-hover:bg-primary/50 group-focus-visible:bg-primary group-active:bg-primary",
            anteprima !== null && "bg-primary",
          )}
        />
        {anteprima !== null && (
          <span className="pointer-events-none absolute left-1/2 top-3 -translate-x-1/2 whitespace-nowrap rounded-md bg-primary px-1.5 py-0.5 text-[11px] font-semibold text-primary-foreground shadow">
            {anteprima} px
          </span>
        )}
      </div>
    </>
  );
}
