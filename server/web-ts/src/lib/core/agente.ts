import fs from "node:fs";
import { config } from "../config";
import { accoda, calendariInoltrati, etichetta, leggiCacheEventi, scriviCacheEventi, type Ev } from "./caselle";
import { nelleDati, scriviJson } from "./dati";
import { daSecondi, iso, parseBound, ErroreData } from "./fuso";
import { personalizzazione } from "./personalizzazione";
import { messaggio, pyStr } from "./py";
import { aliasPerAccount, colorForAccount, type Config } from "./stato";

// Le caselle lette dall'agente sul server (era agente_remoto.py, fino al commit 142e789): una richiesta HTTP con la chiave dell'API. L'agente tiene i login,
// legge il telefono e risponde con gli eventi e lo stato di ogni casella; qui si decide il pallino di ogni azienda.

export class AgenteNonDisponibile extends Error {}

// Oltre questi secondi senza segni di vita dal servizio che aggiorna le caselle sul server, i dati non si possono più dare per
// freschi: prima giallo, poi rosso (rosso vuol dire "devi intervenire")
const SERVIZIO_LENTO_S = 180;
const SERVIZIO_FERMO_S = 900;
const ATTESA_MS = 25_000;
const GRAVITA: Record<string, number> = { ok: 0, avviso: 1, errore: 2 };

export type StatoCasella = { stato: string; msg: string };

// Lo stato di ogni casella all'ultima lettura (serve al pallino delle aziende inoltrate, le cui copie stanno in una di queste
// caselle) e le caselle dell'ultima risposta, tenute anche su disco: con l'agente irraggiungibile la barra laterale mostra comunque
// le aziende, in rosso, invece di svuotarsi proprio quando c'è un guasto da far vedere. In globalThis: le rotte di Next hanno
// ciascuna la sua copia dei moduli, lo stato è uno solo.
type Memoria = { ultimoStato: Record<string, StatoCasella>; note: { account: string; tipo: unknown }[] | null };
const globale = globalThis as typeof globalThis & { __oeAgente?: Memoria };
const memoria = (): Memoria => (globale.__oeAgente ??= { ultimoStato: {}, note: null });

/** Lo stato delle caselle all'ultima lettura (per le prove). */
export const ultimoStato = () => memoria().ultimoStato;
/** Dimentica quello che si sa delle caselle (per le prove). */
export function azzeraMemoria(): void {
  globale.__oeAgente = { ultimoStato: {}, note: null };
}

export const idCalendario = (indirizzo: string) => "remoto::" + indirizzo.toLowerCase();

/** GET all'agente. -> dati JSON, o AgenteNonDisponibile. */
export async function chiedi(percorso: string, parametri?: Record<string, string>): Promise<Dict> {
  const url = config.agenteUrl + percorso + (parametri ? "?" + new URLSearchParams(parametri).toString() : "");
  return risposta(() =>
    fetch(url, { headers: { Authorization: "Bearer " + config.agenteChiave }, signal: AbortSignal.timeout(ATTESA_MS), cache: "no-store" }),
  );
}

/** POST all'agente con un corpo JSON (l'espansione dei calendari ICS). */
export async function invia(percorso: string, corpo: unknown): Promise<Dict> {
  return risposta(() =>
    fetch(config.agenteUrl + percorso, {
      method: "POST",
      headers: { Authorization: "Bearer " + config.agenteChiave, "Content-Type": "application/json" },
      body: JSON.stringify(corpo),
      signal: AbortSignal.timeout(ATTESA_MS),
      cache: "no-store",
    }),
  );
}

type Dict = Record<string, unknown>;

async function risposta(richiesta: () => Promise<Response>): Promise<Dict> {
  let r: Response;
  try {
    r = await richiesta();
  } catch (e) {
    console.warn(`agente remoto: richiesta fallita (${messaggio(e)})`);
    const scaduta = e instanceof Error && (e.name === "TimeoutError" || e.name === "AbortError");
    throw new AgenteNonDisponibile(`Il server dei calendari non risponde (${scaduta ? "ReadTimeout" : "ConnectionError"})`);
  }
  if (r.status === 401) throw new AgenteNonDisponibile("Il server dei calendari rifiuta la chiave: va reimpostata");
  if (r.status !== 200) throw new AgenteNonDisponibile(`Il server dei calendari ha risposto ${r.status}`);
  try {
    return (await r.json()) as Dict;
  } catch {
    throw new AgenteNonDisponibile("Risposta del server dei calendari non valida");
  }
}

// ---- Stato: il pallino di ogni casella -----------------------------------------------------------------------------------
function note(): { account: string; tipo: unknown }[] {
  const m = memoria();
  if (m.note === null) {
    try {
      const dati = JSON.parse(fs.readFileSync(nelleDati("agente_caselle.json"), "utf-8"));
      m.note = (dati as Ev[]).filter((c) => c.account && !personalizzazione().accountNascosti.has(String(c.account).toLowerCase())) as { account: string; tipo: unknown }[];
    } catch {
      m.note = [];
    }
  }
  return m.note as { account: string; tipo: unknown }[];
}

function ricorda(visibili: Ev[]): void {
  const nuove = visibili.map((c) => ({ account: String(c.account).toLowerCase(), tipo: c.tipo ?? null }));
  if (JSON.stringify(nuove) === JSON.stringify(note())) return;
  memoria().note = nuove;
  try {
    scriviJson(nelleDati("agente_caselle.json"), nuove);
  } catch (e) {
    console.warn("agente remoto: elenco delle caselle non salvato", e);
  }
}

/** L'agente non risponde: ogni casella nota è in errore. -> i loro stati */
function tutteFerme(errore: unknown): Record<string, StatoCasella> {
  const stati: Record<string, StatoCasella> = {};
  for (const c of note()) stati[c.account] = { stato: "errore", msg: String(messaggio(errore)) };
  memoria().ultimoStato = stati;
  return stati;
}

/** (stato, messaggio) se il meccanismo che aggiorna le caselle non gira bene: la lettura riesce anche con i dati fermi. */
function guastoServizio(servizio: Ev | null | undefined): [string, string] | null {
  if (!servizio || (typeof servizio === "object" && !Object.keys(servizio).length)) return ["avviso", "Dal server non arriva il segno di vita dell'aggiornamento"];
  if (!("telefono" in servizio ? servizio.telefono : true)) return ["errore", "Sul server il lettore delle caselle è spento"];
  const eta = Math.trunc(Number(servizio.eta_s) || 0);
  if (eta > SERVIZIO_FERMO_S) return ["errore", `Sul server l'aggiornamento è fermo da ${Math.floor(eta / 60)} minuti`];
  if (eta > SERVIZIO_LENTO_S) return ["avviso", `Sul server l'aggiornamento è in ritardo di ${Math.floor(eta / 60)} minuti`];
  if (servizio.pausa) return ["avviso", "Sul server l'aggiornamento è in pausa (accesso in corso?)"];
  return null;
}

function statoCasella(casella: Ev, servizio: Ev | null | undefined): StatoCasella {
  let stato = (casella.stato as string) || "ok";
  let msg = (casella.msg as string) || "";
  if (stato === "ok") {
    const guasto = guastoServizio(servizio);
    if (guasto) {
      stato = guasto[0];
      msg = [guasto[1], msg].filter(Boolean).join(" · ");
    }
  }
  return { stato, msg };
}

/** Di due stati della stessa azienda tiene il più grave, con tutti i messaggi. */
function peggiore(a: StatoCasella | undefined, b: StatoCasella): StatoCasella {
  if (!a) return b;
  const grave = (GRAVITA[b.stato] ?? 0) > (GRAVITA[a.stato] ?? 0) ? b : a;
  return { stato: grave.stato, msg: [...new Set([a.msg, b.msg].filter(Boolean))].join(" · ") };
}

const caselleVisibili = (dati: Dict): Ev[] =>
  (Array.isArray(dati.caselle) ? (dati.caselle as Ev[]) : []).filter((c) => c.account && !personalizzazione().accountNascosti.has(String(c.account).toLowerCase()));

/** Il pallino di un'azienda inoltrata: le copie stanno nel calendario di un'altra casella, se quella si legge le copie arrivano. */
export function statoInoltro(azienda: string): Ev {
  const via = pyStr(personalizzazione().aziendeInoltrate.get(azienda)?.via ?? "").toLowerCase();
  const ospite = memoria().ultimoStato[via];
  if (!ospite) return { stato: "avviso", stato_msg: "Non ancora letto" };
  if (ospite.stato !== "ok") return { stato: ospite.stato, stato_msg: ospite.msg };
  return { stato: "ok", stato_msg: `Copie lette dal calendario di ${via}; flusso non verificabile da remoto` };
}

export const calendariDiInoltro = (cfg: Config) => calendariInoltrati(cfg, statoInoltro);

// ---- Calendari ed eventi -------------------------------------------------------------------------------------------------
/**
 * Una voce per casella letta dall'agente, con colore, stato e interruttore. Se l'agente non risponde si dà l'ultimo elenco
 * riuscito, con ogni casella in errore; con force, o se un elenco non c'è ancora, si solleva AgenteNonDisponibile.
 */
export async function elencoCalendari(cfg: Config, force = false): Promise<Ev[]> {
  let lette: Ev[];
  let stati: Record<string, StatoCasella>;
  try {
    const dati = await chiedi("/api/calendari");
    lette = caselleVisibili(dati);
    ricorda(lette);
    const servizio = dati.servizio as Ev | null;
    stati = {};
    for (const c of lette) stati[String(c.account).toLowerCase()] = statoCasella(c, servizio);
    memoria().ultimoStato = stati;
  } catch (e) {
    if (!(e instanceof AgenteNonDisponibile) || force || !note().length) throw e;
    lette = note();
    stati = tutteFerme(e);
  }
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  const out: Ev[] = [];
  for (const c of lette) {
    const account = String(c.account).toLowerCase();
    const calId = idCalendario(account);
    out.push({
      id: calId,
      account,
      label: `${account} · Calendario`,
      count: c.eventi || 0,
      color: colorForAccount(cfg, account),
      enabled: !disabled.has(calId),
      stato: stati[account].stato,
      stato_msg: stati[account].msg,
    });
  }
  const alias = aliasPerAccount(cfg, out.map((c) => c.account));
  for (const c of out) c.alias = alias[c.account] ?? c.account;
  return out;
}

export type Meta = Ev;
type Esito = [Ev[], string[], Meta];

/** L'agente non si raggiunge: dati salvati e pallini rossi. */
function senzaAgente(cfg: Config, errore: AgenteNonDisponibile, chiaveCache: string, allowCache: boolean, meta: Meta): Esito {
  meta.outlook_starting = true; // l'interfaccia mostra i dati salvati e riprova
  meta.avviso_sorgente = "Il server dei calendari non risponde";
  const stati = tutteFerme(errore);
  const alias = aliasPerAccount(cfg, Object.keys(stati));
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  meta.aziende = {};
  meta.stato_aziende = {};
  for (const [account, stato] of Object.entries(stati)) {
    const azienda = alias[account];
    meta.aziende[azienda] = Boolean(meta.aziende[azienda]) || !disabled.has(idCalendario(account));
    meta.stato_aziende[azienda] = stato;
  }
  if (allowCache) {
    const salvato = leggiCacheEventi()[chiaveCache];
    if (salvato && Object.keys(salvato).length) {
      meta.stale = true;
      meta.stale_at = daSecondi(salvato.stamp ?? 0);
      return [salvato.eventi ?? [], [errore.message], meta];
    }
  }
  return [[], [errore.message], meta];
}

/**
 * Gli eventi di tutte le caselle nell'intervallo: (eventi, errori, meta). In più meta.stato_aziende dà il pallino di ogni azienda
 * per QUESTA lettura.
 */
export async function tuttiGliEventi(cfg: Config, startIso: string, endIso: string, allowCache = true): Promise<Esito> {
  const meta: Meta = { stale: false, stale_at: null, outlook_starting: false };
  let start, end;
  try {
    start = parseBound(startIso);
    end = parseBound(endIso);
  } catch (e) {
    if (!(e instanceof ErroreData)) throw e;
    return [[], [`Intervallo non valido: ${e.message}`], meta];
  }
  const chiaveCache = `${iso(start)}|${iso(end)}`;
  let dati: Dict;
  try {
    dati = await chiedi("/api/eventi", { inizio: iso(start), fine: iso(end) });
  } catch (e) {
    if (!(e instanceof AgenteNonDisponibile)) throw e;
    return senzaAgente(cfg, e, chiaveCache, allowCache, meta);
  }

  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  const servizio = dati.servizio as Ev | null;
  const visibili = caselleVisibili(dati);
  ricorda(visibili);
  const mie = new Set(visibili.map((c) => String(c.account).toLowerCase()));
  const aliasCasella = aliasPerAccount(cfg, [...mie].sort());
  // Per ogni azienda: se ha almeno una casella accesa, e com'è andata la lettura. Anche le caselle spente si leggono: possono
  // contenere riunioni di un'ALTRA azienda, che devono restare visibili se quella è accesa (a scartare pensa il filtro per
  // azienda, a valle della riclassificazione).
  const aziende: Record<string, boolean> = {};
  const statoAziende: Record<string, StatoCasella> = {};
  const errors: string[] = [];
  const stati: Record<string, StatoCasella> = {};
  for (const c of visibili) {
    const account = String(c.account).toLowerCase();
    const azienda = aliasCasella[account];
    const stato = statoCasella(c, servizio);
    stati[account] = stato;
    aziende[azienda] = Boolean(aziende[azienda]) || !disabled.has(idCalendario(account));
    statoAziende[azienda] = peggiore(statoAziende[azienda], stato);
    if (stato.stato === "errore") errors.push(`${azienda}: ${stato.msg}`);
  }
  // Sostituzione in un colpo solo: chi legge da un'altra richiesta non deve mai trovare l'elenco a metà
  memoria().ultimoStato = stati;

  const events: Ev[] = [];
  const seen = new Map<string, number>();
  for (const e of Array.isArray(dati.eventi) ? (dati.eventi as Ev[]) : []) {
    const account = String(e.account || "").toLowerCase();
    if (!mie.has(account) || !e.start) continue;
    // Il nome del calendario lo dice l'agente: una casella ne può portare più d'uno (i calendari condivisi da altre aziende)
    const label = e.calendar || `${account} · Calendario`;
    const ev: Ev = {
      id: e.id || `${account}|${e.start}`,
      account,
      calendar: label,
      color: colorForAccount(cfg, account),
      title: e.title || "(senza titolo)",
      start: e.start,
      end: e.end || e.start,
      allDay: Boolean(e.allDay),
      location: e.location || "",
      organizer: e.organizer || "",
      organizer_email: e.organizer_email || "",
      online_url: e.online_url ?? null,
      web_link: null,
      // Non c'è un EntryID di Outlook: l'interfaccia lo deve sapere
      remoto: true,
    };
    // Calendario Google: è il calendario dell'azienda e la sua etichetta vince su quella della casella in cui Outlook ha archiviato lo stesso invito
    if (e.tipo === "caldav") ev.is_google = true;
    // "Mostra come: Occupato" in Outlook o Google: un giorno intero o un impegno lungo così conta nella disponibilità (impegni.ts)
    if (typeof e.occupato === "boolean") ev.occupato = e.occupato;
    const invitati = (Array.isArray(e.attendees) ? e.attendees : []).map((x: unknown) => pyStr(x).toLowerCase());
    accoda(events, seen, ev, label, () => invitati.find((i: string) => mie.has(i)) ?? "");
  }
  events.sort((a, b) => ((a.start || "") < (b.start || "") ? -1 : (a.start || "") > (b.start || "") ? 1 : 0));
  etichetta(cfg, events);
  meta.aziende = aziende;
  meta.stato_aziende = statoAziende;
  if (allowCache) scriviCacheEventi(chiaveCache, events);
  return [events, errors, meta];
}
