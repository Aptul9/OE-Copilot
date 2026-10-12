import { readFile } from "node:fs/promises";
import { scriviAtomico } from "../file.js";
import type { Soglie } from "../configurazione.js";
import { ErroreTipo, ErroreValore, comeNumero, dividiPerDifetto, intero, jsonPython, leggiJsonPython, numeroDecimale, righe, secondiDaDataOra } from "../python.js";
import { ErroreTelefono } from "../telefono.js";
import { googleStato, secondiAdesso, type ContestoRilevatore, type Verdetto } from "./tipi.js";

const RIGA_GOOGLE = /^Account (\S+) u\d+ com\.google\s*$/;
const RIGA_ESITO = /^com\.android\.calendar\s.*?\b(?:[A-Z]+) (SUCCESS|FAILURE)\b/;
const RIGA_ORA = /(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d)\s*$/;

export type Verdetti = Map<string, Verdetto>;

/**
 * {indirizzo: [ora di telefono in secondi, riuscita]} dall'ultima sincronizzazione del calendario Google. Si leggono solo l'indirizzo e
 * la riga di com.android.calendar: gli altri adattatori di Google (contatti con "OTHER FAILURE internal-error" sempre, fitness,
 * giochi...) non riguardano il calendario.
 */
export function esitiGoogle(dumpsys: string): Map<string, [number, boolean]> {
  const esiti = new Map<string, [number, boolean]>();
  let conto: string | null = null;
  let rigaEsito: boolean | null = null;
  for (const riga of righe(dumpsys)) {
    const g = RIGA_GOOGLE.exec(riga);
    if (g) {
      conto = (g[1] ?? "").toLowerCase();
      rigaEsito = null;
      continue;
    }
    if (riga.startsWith("Account ")) conto = null;
    if (conto === null) continue;
    const e = RIGA_ESITO.exec(riga);
    if (e) {
      rigaEsito = e[1] === "SUCCESS";
      continue;
    }
    if (riga.startsWith("com.")) rigaEsito = null; // un altro adattatore: l'ora che segue non è del calendario
    if (rigaEsito !== null) {
      const o = RIGA_ORA.exec(riga);
      if (o) {
        esiti.set(conto, [secondiDaDataOra(o[1] ?? ""), rigaEsito]);
        rigaEsito = null;
      }
    }
  }
  return esiti;
}

/**
 * Verdetti per i calendari Google, e il ricordo aggiornato ({indirizzo: secondi dell'ultimo successo visto}). L'ora dell'esito è del
 * telefono: la si porta ai secondi del server con la differenza fra le due ore.
 */
export function accessiGoogle(
  dumpsys: string,
  oraTelefono: number,
  adesso: number,
  ricordo: ReadonlyMap<string, number>,
  soglie: Soglie,
): [Verdetti, Map<string, number>] {
  const nuovo = new Map(ricordo);
  const uscita: Verdetti = new Map();
  for (const [email, [quando, riuscita]] of esitiGoogle(dumpsys)) {
    const epoca = adesso - (oraTelefono - quando);
    if (riuscita) nuovo.set(email, Math.max(nuovo.get(email) ?? 0, epoca));
    if (!nuovo.has(email)) nuovo.set(email, adesso); // mai visto: si parte da ora
    const senza = intero(adesso - (nuovo.get(email) ?? adesso));
    if (!riuscita && senza > soglie.googleRossoS) {
      uscita.set(email, {
        stato: "errore",
        msg: `Google non sincronizza da ${dividiPerDifetto(senza, 60)} min sul telefono: probabile accesso scaduto, rifai il login Google`,
      });
    } else if (!riuscita && senza > soglie.googleGialloS) {
      uscita.set(email, { stato: "avviso", msg: `Google non sincronizza da ${dividiPerDifetto(senza, 60)} min sul telefono` });
    } else if (adesso - epoca > soglie.googleGialloS) {
      uscita.set(email, {
        stato: "avviso",
        msg: `La sincronizzazione Google sul telefono non parte da ${dividiPerDifetto(adesso - epoca, 60)} min`,
      });
    } else {
      uscita.set(email, { stato: "ok", msg: "" });
    }
  }
  return [uscita, nuovo];
}

/** Il ricordo come lo scriveva json.dump: numeri con la frazione, ", " e ": " fra le voci. */
export function testoRicordo(ricordo: ReadonlyMap<string, number>): string {
  return "{" + [...ricordo].map(([k, v]) => `${jsonPython(k)}: ${numeroDecimale(v)}`).join(", ") + "}";
}

async function leggiRicordo(percorso: string): Promise<Map<string, number>> {
  try {
    const dati: unknown = leggiJsonPython(await readFile(percorso, "utf8"));
    if (typeof dati !== "object" || dati === null || Array.isArray(dati)) return new Map();
    return new Map(Object.entries(dati).map(([k, v]) => [k, comeNumero(v)]));
  } catch (e) {
    if (e instanceof ErroreTipo) throw e;
    return new Map();
  }
}

/**
 * Legge dumpsys content e l'ora dal telefono e applica accessiGoogle(). Vuoto se non si riesce a leggerli: un errore del telefono
 * o un'ora illeggibile non danno un verdetto.
 */
export async function verificaGoogle(c: ContestoRilevatore, adessoDato?: number): Promise<Verdetti> {
  const adesso = adessoDato ?? secondiAdesso(c);
  let dump: string;
  let oraTelefono: number;
  try {
    dump = await c.telefono.leggi(["dumpsys", "content"], { timeoutMs: 25_000 });
    oraTelefono = secondiDaDataOra((await c.telefono.leggi(["date", "+%Y-%m-%d %H:%M:%S"], { timeoutMs: 10_000 })).trim());
  } catch (e) {
    if (e instanceof ErroreTelefono || e instanceof ErroreValore) return new Map();
    throw e;
  }
  const [uscita, ricordo] = accessiGoogle(dump, oraTelefono, adesso, await leggiRicordo(googleStato(c)), c.soglie);
  if (uscita.size > 0) {
    await scriviAtomico(googleStato(c), testoRicordo(ricordo));
  }
  return uscita;
}
