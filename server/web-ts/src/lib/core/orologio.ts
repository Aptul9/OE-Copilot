import { daIstante } from "./fuso";

// L'ora in un punto solo, così le prove la possono fissare.
let fisso: number | null = null;

/** Millisecondi dall'epoca, adesso (o l'ora fissata dalle prove). */
export const adesso = (): number => fisso ?? Date.now();

/** Secondi dall'epoca con la frazione, come time.time() */
export const secondi = (): number => adesso() / 1000;

/** Fissa l'ora (null la lascia scorrere): solo per le prove. */
export function fissaOra(ms: number | null): void {
  fisso = ms;
}

/** AAAAMMGG-HHMMSS di Roma, come time.strftime("%Y%m%d-%H%M%S") */
export function oraStamp(): string {
  const n = daIstante(adesso());
  const d = (x: number) => String(x).padStart(2, "0");
  return `${n.a}${d(n.m)}${d(n.g)}-${d(n.h)}${d(n.mi)}${d(n.s)}`;
}
