// Le poche regole di Python su cui il servizio ui faceva affidamento senza dirlo: str() e bool() di un valore JSON.

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
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
