import { AgenteNonDisponibile } from "./agente";
import type { Ev } from "./caselle";
import { calendariCaselle, calendariExtra, occupato, raccogli } from "./eventi";
import { aggiungiFeed, rimuoviFeed } from "./ics";
import { aggiungiEvento, eliminaEvento } from "./locali";
import { ErroreValore, isDict, messaggio, pyBool, pyStr, testoPulito } from "./py";
import {
  aliasPerAccount,
  confrontaPunti,
  type Config,
  impostaAcceso,
  loadConfig,
  membriGruppo,
  modificaConfig,
  ordineCalendario,
  rinominaNeiGruppi,
} from "./stato";
import { personalizzazione } from "./personalizzazione";

// I metodi di app.Api che il servizio ui esponeva alla pagina, scritti qui (app.py): la lettura e la scrittura. Il gateway li serve da
// sé. Ogni metodo riceve gli argomenti della lista JSON e restituisce quello che finisce nel corpo della risposta.

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

/** Tutte le sorgenti di calendari: le caselle (se l'agente non risponde si va avanti senza) e le altre voci. */
async function tutteLeSorgenti(cfg: Config): Promise<Ev[]> {
  let caselle: Ev[] = [];
  try {
    caselle = await calendariCaselle(cfg);
  } catch (e) {
    if (!(e instanceof AgenteNonDisponibile)) throw e;
  }
  return [...caselle, ...calendariExtra(cfg)];
}

/** Il nome mostrato di un calendario: l'alias, se no l'account. */
const nomeMostrato = (c: Ev): string => c.alias || c.account;

/** Dopo aver letto le sorgenti (che è lento e può assegnare colori), si cambia l'elenco dei calendari spenti sulla configurazione di adesso. */
function cambiaSpenti(daCambiare: (cfg: Config, spenti: Set<string>) => void): void {
  modificaConfig((cfg) => {
    const spenti = new Set<string>(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
    daCambiare(cfg, spenti);
    cfg.disabled_calendars = [...spenti].sort(confrontaPunti);
  });
}

/** Accende le aziende per cui `accesa(nome)` è vera e spegne tutte le altre. */
async function accendiSolo(accesa: (nome: string) => boolean): Promise<void> {
  const tutti = await tutteLeSorgenti(loadConfig());
  cambiaSpenti((_, spenti) => {
    for (const c of tutti) {
      if (accesa(nomeMostrato(c))) spenti.delete(c.id);
      else spenti.add(c.id);
    }
  });
}

/** Il corpo di un risultato che dice "no" con il testo che la pagina mostra: gli errori di Python che finivano in {"ok": False, "error": ...}. */
function rifiuto(nome: string, e: unknown): { ok: false; error: string } {
  if (!(e instanceof ErroreValore)) console.error(`${nome}: eccezione`, e);
  return { ok: false, error: messaggio(e) };
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

  // ---- Scritture ----------------------------------------------------------------------------------------------------
  /** Unisce `nuova` alla configurazione: le voci che ci sono cambiano, le altre restano. */
  async save_config(nuova: unknown) {
    const voci = pyBool(nuova) ? nuova : {};
    if (!isDict(voci)) throw new TypeError("cannot convert dictionary update sequence element #0 to a sequence");
    modificaConfig((cfg) => void Object.assign(cfg, voci));
    return { ok: true };
  },

  async set_calendar_enabled(calId: unknown, acceso: unknown) {
    modificaConfig((cfg) => impostaAcceso(cfg, pyStr(calId), pyBool(acceso)));
    return { ok: true };
  },

  /** Accende o spegne in blocco tutti i calendari di un account: anche le voci che arrivano dal calendario locale, non solo caselle e link. */
  async set_account_enabled(account: unknown, acceso: unknown) {
    const tutti = await tutteLeSorgenti(loadConfig());
    const ids = tutti.filter((c) => c.account === account).map((c) => c.id as string);
    if (ids.length) modificaConfig((cfg) => ids.forEach((id) => impostaAcceso(cfg, id, pyBool(acceso))));
    return { ok: true };
  },

  /** Accende o spegne TUTTI i calendari di un'azienda, in qualunque casella: nella barra laterale è una voce sola e deve comandarli tutti. */
  async set_azienda_enabled(azienda: unknown, acceso: unknown) {
    const tutti = await tutteLeSorgenti(loadConfig());
    const suoi = tutti.filter((c) => nomeMostrato(c) === azienda);
    cambiaSpenti((_, spenti) => {
      for (const c of suoi) {
        if (pyBool(acceso)) spenti.delete(c.id);
        else spenti.add(c.id);
      }
    });
    console.info(`set_azienda_enabled: ${pyStr(azienda)} -> ${pyBool(acceso) ? "acceso" : "spento"} (${suoi.length} calendari)`);
    return { ok: true, calendari: suoi.length };
  },

  /** Lascia accesa una sola azienda, o tutte se `azienda` è vuota. */
  async mostra_solo_azienda(azienda: unknown) {
    await accendiSolo((nome) => !pyBool(azienda) || nome === azienda);
    console.info(`mostra_solo_azienda: ${pyBool(azienda) ? pyStr(azienda) : "tutte"}`);
    return { ok: true };
  },

  /** Lascia accese le aziende di un gruppo (config "gruppi_calendari"), spegne le altre. */
  async mostra_solo_gruppo(gruppo: unknown) {
    const membri = membriGruppo(loadConfig(), pyStr(gruppo));
    if (membri === null) return { ok: false, error: "Gruppo non trovato" };
    await accendiSolo((nome) => membri.has(nome));
    console.info(`mostra_solo_gruppo: ${pyStr(gruppo)} (${membri.size} aziende)`);
    return { ok: true };
  },

  /** Il nome da mostrare per un account. Vuoto = torna a quello automatico. Le aziende nei gruppi seguono il nuovo nome. */
  async set_account_alias(account: unknown, alias: unknown) {
    modificaConfig((cfg) => {
      if (!isDict(cfg.account_aliases)) cfg.account_aliases = {};
      const scelti = cfg.account_aliases as Record<string, string>;
      const conto = account as string;
      const prima = aliasPerAccount(cfg, [conto])[conto];
      const nuovo = testoPulito(alias);
      if (nuovo) scelti[conto] = nuovo;
      else delete scelti[conto];
      rinominaNeiGruppi(cfg, prima, aliasPerAccount(cfg, [conto])[conto]);
    });
    return { ok: true };
  },

  async add_ics_calendar(nome: unknown, url: unknown) {
    try {
      const feed = await aggiungiFeed(nome, url);
      console.info(`add_ics_calendar: aggiunto ${feed.name}`);
      return { ok: true, feed };
    } catch (e) {
      return rifiuto("add_ics_calendar", e);
    }
  },

  async remove_ics_calendar(feedId: unknown) {
    rimuoviFeed(feedId);
    return { ok: true };
  },

  async add_local_event(dati: unknown) {
    try {
      aggiungiEvento(dati);
      return { ok: true };
    } catch (e) {
      return rifiuto("add_local_event", e);
    }
  },

  async delete_local_event(uid: unknown) {
    eliminaEvento(uid);
    return { ok: true };
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
