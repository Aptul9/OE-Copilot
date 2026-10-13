"use client";

import { useEffect, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { LARGHEZZA_PANNELLO, limitaLarghezza } from "@/lib/oe/logic";

const CHIAVE = "oe.larghezza-pannello";

/** La colonna di sinistra (minicalendario e calendari) con una maniglia sul bordo per allargarla o stringerla: la larghezza resta in questo browser. */
export function PannelloLaterale({ children }: { children: ReactNode }) {
  const [larghezza, setLarghezza] = useState<number>(LARGHEZZA_PANNELLO.predefinita);
  useEffect(() => {
    try {
      const v = Number(localStorage.getItem(CHIAVE));
      if (v) setLarghezza(limitaLarghezza(v));
    } catch {
      // senza archivio nel browser resta la larghezza di partenza
    }
  }, []);
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
    maniglia.setPointerCapture(e.pointerId);
    const muovi = (m: globalThis.PointerEvent) => imposta(l0 + m.clientX - x0);
    const fine = () => {
      maniglia.removeEventListener("pointermove", muovi);
      maniglia.removeEventListener("pointerup", fine);
      maniglia.removeEventListener("pointercancel", fine);
    };
    maniglia.addEventListener("pointermove", muovi);
    maniglia.addEventListener("pointerup", fine);
    maniglia.addEventListener("pointercancel", fine);
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
        className="group z-10 -mx-[3px] hidden w-1.5 flex-none cursor-col-resize touch-none select-none lg:block"
        onPointerDown={trascina}
        onKeyDown={tasto}
        onDoubleClick={() => imposta(LARGHEZZA_PANNELLO.predefinita)}
      >
        <div className="mx-auto h-full w-0.5 bg-transparent group-hover:bg-primary/50 group-focus-visible:bg-primary group-active:bg-primary" />
      </div>
    </>
  );
}
