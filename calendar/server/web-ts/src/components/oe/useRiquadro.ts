"use client";

import { type RefObject, useEffect, useLayoutEffect, useState } from "react";

/** Un riquadro che si apre accanto al clic sulla griglia (creazione rapida, prenotazione): dove metterlo, e la chiusura con un clic fuori
 * o con Esc. `pronto`: il riquadro c'è nel documento (un giro dopo l'apertura); misurato prima non esisterebbe. */
export function useRiquadro(ref: RefObject<HTMLElement | null>, { aperto, pronto, x, y, onChiudi }: { aperto: boolean; pronto: boolean; x?: number; y?: number; onChiudi: () => void }) {
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // la posizione si calcola all'apertura, non a ogni orario cambiato: il riquadro non salta sotto il mouse
  useLayoutEffect(() => {
    if (!pronto || x === undefined || y === undefined || !ref.current) return;
    const { offsetWidth: w, offsetHeight: h } = ref.current;
    // accanto al clic, a destra se c'è posto, altrimenti a sinistra; mai fuori dalla finestra
    const destra = x + 12 + w <= window.innerWidth - 10;
    setPos({
      left: Math.max(10, destra ? x + 12 : x - 12 - w),
      top: Math.max(10, Math.min(y - 24, window.innerHeight - h - 16)),
    });
  }, [pronto, x, y, ref]);

  useEffect(() => {
    if (!aperto) return;
    // un clic fuori la chiude: su un altro spazio libero la griglia ne sceglie subito un altro, e il riquadro si riapre lì. I menu dei campi
    // data e ora del browser non sono nel documento: non contano come fuori
    const fuori = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onChiudi();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onChiudi();
    document.addEventListener("pointerdown", fuori);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", fuori);
      document.removeEventListener("keydown", esc);
    };
  }, [aperto, onChiudi, ref]);

  return pos;
}
