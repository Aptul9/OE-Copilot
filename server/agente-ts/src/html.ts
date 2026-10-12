// html.unescape di Python 3.12 e _pulisci_link dell'agente Python (tolto dal repository): le entità HTML5 con la tabella completa (entita.ts, generata da
// html.entities.html5), i riferimenti numerici con le regole dello standard (caratteri non validi, intervalli proibiti).

import { HTML5 } from "./entita.ts";
import { ErroreValore } from "./tempo.ts";

const NON_VALIDI = new Map<number, string>([
  [0x00, "\ufffd"], [0x0d, "\r"], [0x80, "\u20ac"], [0x81, "\x81"], [0x82, "\u201a"], [0x83, "\u0192"], [0x84, "\u201e"], [0x85, "\u2026"],
  [0x86, "\u2020"], [0x87, "\u2021"], [0x88, "\u02c6"], [0x89, "\u2030"], [0x8a, "\u0160"], [0x8b, "\u2039"], [0x8c, "\u0152"], [0x8d, "\x8d"],
  [0x8e, "\u017d"], [0x8f, "\x8f"], [0x90, "\x90"], [0x91, "\u2018"], [0x92, "\u2019"], [0x93, "\u201c"], [0x94, "\u201d"], [0x95, "\u2022"],
  [0x96, "\u2013"], [0x97, "\u2014"], [0x98, "\u02dc"], [0x99, "\u2122"], [0x9a, "\u0161"], [0x9b, "\u203a"], [0x9c, "\u0153"], [0x9d, "\x9d"],
  [0x9e, "\u017e"], [0x9f, "\u0178"],
]);

function codiceProibito(n: number): boolean {
  if ((n >= 0x1 && n <= 0x8) || (n >= 0xe && n <= 0x1f) || (n >= 0x7f && n <= 0x9f) || (n >= 0xfdd0 && n <= 0xfdef) || n === 0xb) return true;
  return (n & 0xfffe) === 0xfffe && n <= 0x10ffff;
}

const RIFERIMENTO = /&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\f <&#;]{1,32};?)/gu;
/** Python limita le cifre di un intero a 4300: oltre, int() solleva ValueError e l'errore esce dall'agente com'è. */
const MAX_CIFRE = 4300;

function sostituisci(gruppo: string): string {
  if (gruppo[0] === "#") {
    const esadecimale = gruppo[1] === "x" || gruppo[1] === "X";
    const cifre = gruppo.slice(esadecimale ? 2 : 1).replace(/;$/, "");
    if (!esadecimale && cifre.length > MAX_CIFRE) throw new ErroreValore(`Exceeds the limit (${MAX_CIFRE} digits) for integer string conversion: value has ${cifre.length} digits; use sys.set_int_max_str_digits() to increase the limit`);
    const significative = cifre.replace(/^0+(?=.)/, "");
    const n = significative.length > (esadecimale ? 13 : 15) ? Infinity : parseInt(significative, esadecimale ? 16 : 10);
    if (NON_VALIDI.has(n)) return NON_VALIDI.get(n)!;
    if ((n >= 0xd800 && n <= 0xdfff) || n > 0x10ffff) return "\ufffd";
    if (codiceProibito(n)) return "";
    return String.fromCodePoint(n);
  }
  const intero = HTML5.get(gruppo);
  if (intero !== undefined) return intero;
  // il nome più lungo che esiste (come dice lo standard), contando i punti di codice
  const punti = Array.from(gruppo);
  for (let x = punti.length - 1; x > 1; x--) {
    const testa = HTML5.get(punti.slice(0, x).join(""));
    if (testa !== undefined) return testa + punti.slice(x).join("");
  }
  return "&" + gruppo;
}

/** html.unescape(). */
export function htmlUnescape(s: string): string {
  if (!s.includes("&")) return s;
  return s.replace(RIFERIMENTO, (_, gruppo: string) => sostituisci(gruppo));
}

/** Link di riunione come lo si trova nella descrizione -> link usabile (_pulisci_link): "&amp;" letto come "&", la copia protetta dei safelinks tolta, la punteggiatura in coda tolta. */
export function pulisciLink(url: string): string {
  const pulito = htmlUnescape(url).split(/%3[Cc](?=https?:\/\/)/)[0]!;
  return pulito.replace(/[>).,;"']+$/, "");
}
