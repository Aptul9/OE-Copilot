import { access, constants } from "node:fs/promises";
import path from "node:path";
import { dbOutlook, logOutlook } from "./tipi.js";

/**
 * Perché il rilevatore non potrebbe leggere i dati di Outlook, o null se li legge. Il rilevatore in Python, senza permesso, non dava
 * nessun verdetto per Outlook senza dirlo (os.path.exists risponde falso): qui il motivo finisce nel log.
 */
export async function problemaIngressi(dati: string): Promise<string | null> {
  const utente = process.getuid?.();
  const mancanti: string[] = [];
  for (const [nome, percorso] of [
    ["database", dbOutlook({ dati })],
    ["calendarSync.log", path.join(logOutlook({ dati }), "calendarSync.log")],
  ] as const) {
    try {
      await access(percorso, constants.R_OK);
    } catch (e) {
      const codice = (e as NodeJS.ErrnoException).code;
      mancanti.push(codice === "ENOENT" ? `${nome} non c'è` : `${nome} non si legge (${codice ?? "errore"}, utente ${utente ?? "sconosciuto"})`);
    }
  }
  return mancanti.length === 0 ? null : `dati di Outlook: ${mancanti.join(", ")}`;
}
