// Le poche regole di Python su cui il servizio ui faceva affidamento senza dirlo: str() e bool() di un valore JSON.

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

/** Il ValueError con cui Python rifiuta un dato di chi usa la pagina: il testo è quello che la pagina mostra. */
export class ErroreValore extends Error {}
export type Dict = Record<string, unknown>;

/** str(valore) per i valori di un file JSON: True, False, None come in Python. */
export function pyStr(v: unknown): string {
  if (v === null || v === undefined) return "None";
  if (v === true) return "True";
  if (v === false) return "False";
  return typeof v === "string" ? v : typeof v === "number" ? String(v) : JSON.stringify(v);
}

/** bool(valore): vuoto e zero sono falsi. */
export function pyBool(v: unknown): boolean {
  if (v === null || v === undefined || v === false || v === 0 || v === "") return false;
  if (Array.isArray(v)) return v.length > 0;
  if (typeof v === "object") return Object.keys(v as object).length > 0;
  return true;
}

export const isDict = (v: unknown): v is Dict => typeof v === "object" && v !== null && !Array.isArray(v);

// Gli spazi per str.strip() di Python: il White_Space di Unicode e i separatori \x1c-\x1f (non il BOM, che trim() di JavaScript toglie)
const SPAZI = "\\t\\n\\v\\f\\r\\x1c-\\x1f \\x85\\xa0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000";
const AI_LATI = new RegExp(`^[${SPAZI}]+|[${SPAZI}]+$`, "g");

/** str.strip() */
export const pyStrip = (s: string): string => s.replace(AI_LATI, "");

/** Il nome del tipo di un valore JSON come Python lo scrive negli errori. */
export function tipoPy(v: unknown): string {
  if (v === null || v === undefined) return "NoneType";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "number") return Number.isInteger(v) ? "int" : "float";
  if (typeof v === "string") return "str";
  return Array.isArray(v) ? "list" : "dict";
}

/** (valore or "").strip(): un valore falso vale "", un testo si ripulisce, qualunque altra cosa è l'AttributeError di Python. */
export function testoPulito(v: unknown): string {
  if (!pyBool(v)) return "";
  if (typeof v !== "string") throw new TypeError(`'${tipoPy(v)}' object has no attribute 'strip'`);
  return pyStrip(v);
}

/** `valore or predefinito`, come in Python (un valore falso cede il posto). */
export const oppure = <T>(v: unknown, predefinito: T): T | unknown => (pyBool(v) ? v : predefinito);

/** Un elenco di testi: gli elementi di una lista, str() di ciascuno; qualunque altra cosa è vuota. */
export const elencoTesti = (v: unknown): string[] => (Array.isArray(v) ? v.map(pyStr) : []);

/** {str(v).strip().lower() per ogni voce non vuota}: l'insieme di nomi in minuscolo di personalizzazione.json. */
export function minuscole(v: unknown): Set<string> {
  const out = new Set<string>();
  for (const x of Array.isArray(v) ? v : []) {
    const s = pyStr(x).trim();
    if (s) out.add(s.toLowerCase());
  }
  return out;
}

export const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));
