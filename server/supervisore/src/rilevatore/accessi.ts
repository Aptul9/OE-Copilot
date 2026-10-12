import { mkdir } from "node:fs/promises";
import path from "node:path";
import { scriviAtomico } from "../file.js";
import { jsonPython, type ValoreJson } from "../python.js";
import { verificaGoogle } from "./google.js";
import { accessiOutlook, freschezzaOutlook, type Verdetti } from "./outlook.js";
import type { ContestoRilevatore, Verdetto } from "./tipi.js";

const GRAVITA: Record<string, number> = { ok: 0, avviso: 1, errore: 2 };

const messaggio = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** Dei due verdetti tiene il più grave, con tutti e due i messaggi (senza ripetizioni, nell'ordine). */
export function peggiore(a: Verdetto | undefined, b: Verdetto): Verdetto {
  if (a === undefined) return b;
  const grave = (GRAVITA[b.stato] ?? 0) > (GRAVITA[a.stato] ?? 0) ? b : a;
  return { stato: grave.stato, msg: [...new Set([a.msg, b.msg].filter((x) => x !== ""))].join(" · ") };
}

/** Il riepilogo per il log: il Python lo scartava (>/dev/null), qui va senza gli indirizzi delle caselle, che stanno solo in accessi.json. */
export const senzaIndirizzi = (riepilogo: string): string => riepilogo.replace(/da sistemare: (?!nessuna$).*$/, (m) => `da sistemare: ${m.split(' (').length - 1}`);

const comeValore = (v: Verdetto): ValoreJson =>
  new Map<string, ValoreJson>([
    ["stato", v.stato],
    ["msg", v.msg],
  ]);

/**
 * Il rilevatore degli accessi (main di rileva_accessi.py): per ogni casella uno stato (ok, avviso, errore) e un messaggio in
 * accessi.json, che l'agente legge per il pallino. Un controllo che si rompe non ferma gli altri. Restituisce la riga di riepilogo.
 */
export async function rilevaAccessi(c: ContestoRilevatore): Promise<string> {
  const caselle: Verdetti = new Map();
  const controlli: [string, () => Promise<Verdetti>][] = [
    ["accessi Outlook", () => accessiOutlook(c)],
    ["freschezza Outlook", () => freschezzaOutlook(c)],
    ["Google", () => verificaGoogle(c)],
  ];
  for (const [nome, controllo] of controlli) {
    let fonte: Verdetti;
    try {
      fonte = await controllo();
    } catch (e) {
      c.log(`rilevatore: ${nome} non riuscito (${messaggio(e)})`);
      continue;
    }
    for (const [email, verdetto] of fonte) caselle.set(email, peggiore(caselle.get(email), verdetto));
  }
  const dati = new Map<string, ValoreJson>([
    ["generato", Math.trunc(c.orologio.adesso() / 1000)],
    ["caselle", new Map([...caselle].map(([e, v]) => [e, comeValore(v)]))],
  ]);
  await mkdir(path.dirname(c.uscita), { recursive: true });
  await scriviAtomico(c.uscita, jsonPython(dati));
  const problemi = [...caselle].filter(([, v]) => v.stato !== "ok").map(([e, v]) => `${e} (${v.stato})`).sort();
  return `accessi: ${caselle.size} caselle esaminate, da sistemare: ${problemi.join(", ") || "nessuna"}`;
}
