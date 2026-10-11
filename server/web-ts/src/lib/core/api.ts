import { AgenteNonDisponibile } from "./agente";
import type { Ev } from "./caselle";
import { calendariCaselle, calendariExtra, occupato, raccogli } from "./eventi";
import { messaggio, pyBool, pyStr } from "./py";
import { loadConfig, ordineCalendario } from "./stato";
import { personalizzazione } from "./personalizzazione";

// I metodi di app.Api che il servizio ui esponeva alla pagina, scritti qui (app.py). Il gateway li serve da sé; gli altri passano
// ancora al servizio ui. Ogni metodo riceve gli argomenti della lista JSON e restituisce quello che finisce nel corpo della risposta.

/** Un argomento mancante o di un altro tipo: Python lo diceva con un TypeError, qui lo stesso con un testo suo. */
export class ErroreArgomenti extends Error {}

const GIORNI_MAX = 62;

/** int(valore) per un valore JSON: numeri (troncati), testi di sole cifre, booleani; il resto è un errore. */
function pyInt(v: unknown): number {
  if (typeof v === "boolean") return v ? 1 : 0;
  if (typeof v === "number" && Number.isFinite(v)) return Math.trunc(v);
  if (typeof v === "string" && /^\s*[+-]?\d+\s*$/.test(v)) return Number(v);
  throw new ErroreArgomenti("int() non riceve questo valore");
}

/** date.fromisoformat (Python 3.12) di un testo: AAAA-MM-GG o AAAAMMGG. */
function pyData(v: unknown): { a: number; m: number; g: number } {
  const m = /^(\d{4})-?(\d{2})-?(\d{2})$/.exec(pyStr(v));
  if (!m) throw new ErroreArgomenti("Invalid isoformat string");
  const [a, mo, g] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const d = new Date(Date.UTC(a, mo - 1, g));
  if (d.getUTCFullYear() !== a || d.getUTCMonth() !== mo - 1 || d.getUTCDate() !== g) throw new ErroreArgomenti("day is out of range for month");
  return { a, m: mo, g };
}

export const apiTs = {
  /** La configurazione di chi usa l'app: i valori predefiniti con sopra quelli salvati. */
  async get_config() {
    return loadConfig();
  },

  /** Le impostazioni dell'interfaccia (sigle, loghi, schede): arrivano da personalizzazione.json, nel codice non ci sono nomi di aziende. */
  async get_personalizzazione() {
    return { interfaccia: personalizzazione().interfaccia };
  },

  /** Calendari delle caselle + quelli sottoscritti da link ICS e quelli locali. */
  async get_calendars(force: unknown = false) {
    const t0 = Date.now();
    const cfg = loadConfig();
    let cals: Ev[] = [];
    let errore: string | null = null;
    try {
      cals = await calendariCaselle(cfg, pyBool(force));
    } catch (e) {
      if (e instanceof AgenteNonDisponibile) console.warn(`get_calendars: caselle non disponibili: ${e.message}`);
      else console.error("get_calendars: eccezione", e);
      errore = messaggio(e);
    }
    // Dopo le caselle, non prima: il pallino di un'azienda inoltrata dipende da com'è andata la lettura della casella che ne ospita le copie.
    // I calendari da link e locali restano leggibili anche senza le caselle.
    const extra = calendariExtra(cfg);
    console.info(`get_calendars: ${cals.length} delle caselle + ${extra.length} extra in ${((Date.now() - t0) / 1000).toFixed(1)}s`);
    // Ordine scelto dall'utente per nome mostrato; stabile per il resto
    const calendars = [...cals, ...extra].sort((a, b) => ordineCalendario(a.alias || a.account || "") - ordineCalendario(b.alias || b.account || ""));
    return { calendars, error: errore };
  },

  async get_events(startIso: string, endIso: string) {
    const t0 = Date.now();
    try {
      const cfg = loadConfig();
      const [events, errors, meta] = await raccogli(cfg, startIso, endIso);
      console.info(`get_events: ${events.length} eventi in ${((Date.now() - t0) / 1000).toFixed(1)}s (stale=${meta.stale}, errori: ${JSON.stringify(errors)})`);
      return { events, errors, ...meta };
    } catch (e) {
      console.error("get_events: eccezione", e);
      return { events: [] as Ev[], errors: [messaggio(e)], stale: false, stale_at: null, outlook_starting: false };
    }
  },

  /** Cosa può sapere la pagina pubblica dei `giorni` giorni da `da` (AAAA-MM-GG): gli orari occupati, le festività, e se tutto ha risposto bene. */
  async get_disponibilita(da: unknown, giorni: unknown = 7, gruppo: unknown = "") {
    let primo, n;
    try {
      primo = pyData(da);
      n = pyInt(giorni);
      if (n < 1 || n > GIORNI_MAX) throw new ErroreArgomenti("giorni");
    } catch (e) {
      if (e instanceof ErroreArgomenti) return { ok: false, error: "Intervallo non valido" };
      throw e;
    }
    try {
      const [impegni, festivita, completo] = await occupato(loadConfig(), primo, n, pyBool(gruppo) ? pyStr(gruppo) : "");
      return { ok: true, impegni, festivita, completo };
    } catch (e) {
      if (e instanceof Error && e.message === "Gruppo non trovato") return { ok: false, error: e.message };
      console.error("get_disponibilita: eccezione", e);
      return { ok: false, error: "Calendari non leggibili" };
    }
  },
};

export type NomeMetodoTs = keyof typeof apiTs;

/** I metodi che il gateway serve da sé. */
export const METODI_TS: ReadonlySet<string> = new Set(Object.keys(apiTs));

/** Chiama un metodo con gli argomenti della richiesta (la lista JSON). */
export async function chiama(metodo: NomeMetodoTs, args: unknown[]): Promise<unknown> {
  const f = apiTs[metodo] as (...a: unknown[]) => Promise<unknown>;
  return f(...args);
}
