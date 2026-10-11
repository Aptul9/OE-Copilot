import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { AgenteNonDisponibile, invia } from "./agente";
import type { Ev } from "./caselle";
import { nelleDati } from "./dati";
import { iso, type Naive } from "./fuso";
import { adesso } from "./orologio";
import { COLORE_FESTIVITA, FESTIVITA_ID, FESTIVITA_NOME, FESTIVITA_URL } from "./personalizzazione";
import { ErroreValore, messaggio, testoPulito } from "./py";
import { colorForAccount, impostaAcceso, modificaConfig, type Config } from "./stato";

// Calendari sottoscritti con un link ICS (era ics_feed.py, fino al commit 142e789): Google, Apple, le festività italiane. Il link è di fatto una chiave: chi ce
// l'ha legge il calendario, quindi sta solo nella configurazione sul server. Il testo si scarica da qui, con la sua copia su disco
// e il suo ritardo; le ricorrenze (RRULE, EXDATE, RECURRENCE-ID, fusi) le espande l'agente, la stessa libreria che legge il telefono.

/** Google rigenera l'ICS ogni qualche ora: scaricarlo a ogni cambio di vista sarebbe inutile e lento. */
export const TTL_S = 1800;
const ATTESA_MS = 25_000;

export type Feed = { id: string; name?: string; url: string; builtin?: boolean };

// Esito dell'ultima lettura per feed, per il pallino di stato: un feed irraggiungibile deve vedersi, non sparire.
const globale = globalThis as typeof globalThis & { __oeIcs?: Record<string, { stato: string; msg: string }> };
export const statoFeed = () => (globale.__oeIcs ??= {});
/** Dimentica l'esito delle letture (per le prove). */
export function azzeraFeed(): void {
  globale.__oeIcs = {};
  (globalThis as typeof globalThis & { __oeEspansioni?: unknown }).__oeEspansioni = undefined;
}

export const feedId = (url: string) => "ics_" + crypto.createHash("sha1").update(url, "utf-8").digest("hex").slice(0, 12);

/** Accetta anche i link webcal:// che usano diversi calendari. */
export function normalizzaUrl(url: unknown): string {
  const u = testoPulito(url);
  return u.toLowerCase().startsWith("webcal://") ? "https://" + u.slice("webcal://".length) : u;
}

/** Il calendario delle festività italiane: sempre presente e non modificabile. */
export const feedFestivita = (): Feed => ({ id: FESTIVITA_ID, name: FESTIVITA_NOME, url: FESTIVITA_URL, builtin: true });

export const feedDi = (cfg: Config): Feed[] => (Array.isArray(cfg.ics_calendars) ? (cfg.ics_calendars as Feed[]) : []);

/** Le voci dell'elenco calendari dei feed: le festività compaiono come gli altri, chi non le vuole le spegne. */
export function elencoFeed(cfg: Config): Ev[] {
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  const out: Ev[] = [];
  for (const f of [feedFestivita(), ...feedDi(cfg)]) {
    const nome = f.name || f.url;
    const stato = statoFeed()[f.id];
    out.push({
      id: f.id,
      account: nome,
      label: nome,
      url: f.url,
      count: null, // un ICS non ha un conteggio a colpo d'occhio
      is_ics: true,
      color: f.builtin ? COLORE_FESTIVITA : colorForAccount(cfg, nome),
      enabled: !disabled.has(f.id),
      builtin: Boolean(f.builtin),
      // Verde solo se una lettura è andata a buon fine davvero
      stato: stato?.stato ?? "avviso",
      stato_msg: stato?.msg ?? "Non ancora letto",
    });
  }
  return out;
}

// ---- Scarico e copia su disco ----------------------------------------------------------------------------------------------
/** I due punti non sono ammessi nei nomi file su Windows e l'id delle festività li contiene: si sostituiscono. */
export const percorsoCache = (id: string) => path.join(nelleDati("ics_cache"), String(id).replace(/[^A-Za-z0-9._-]/g, "_") + ".ics");

export function salvaCache(id: string, testo: string): void {
  try {
    fs.mkdirSync(nelleDati("ics_cache"), { recursive: true });
    fs.writeFileSync(percorsoCache(id), testo, "utf-8");
  } catch (e) {
    // La copia locale è un'ottimizzazione, ma se non si scrive ogni lettura riparte dalla rete: meglio saperlo
    console.warn(`ics: copia locale di ${id} non scritta`, e);
  }
}

export function togliCache(id: string): void {
  fs.rmSync(percorsoCache(id), { force: true });
}

function decodifica(corpo: ArrayBuffer, tipo: string | null): string {
  // Come requests: il charset dell'intestazione; per un text/* senza charset ISO-8859-1; altrimenti UTF-8
  const charset = /charset=["']?([\w-]+)/i.exec(tipo ?? "")?.[1] ?? (/text/i.test(tipo ?? "") ? "iso-8859-1" : "utf-8");
  try {
    return new TextDecoder(charset).decode(corpo);
  } catch {
    return new TextDecoder("utf-8").decode(corpo);
  }
}

/** Il testo con cui requests rifiuta uno stato HTTP da 400 in su (raise_for_status). */
function erroreHttp(r: Response): string {
  return `${r.status} ${r.status >= 500 ? "Server" : "Client"} Error: ${r.statusText} for url: ${r.url}`;
}

export async function scarica(url: string): Promise<string> {
  const r = await fetch(url, { headers: { "User-Agent": "CalendarHub/1.0" }, signal: AbortSignal.timeout(ATTESA_MS), cache: "no-store" });
  if (r.status >= 400) throw new Error(erroreHttp(r));
  return decodifica(await r.arrayBuffer(), r.headers.get("content-type"));
}

/** Perché una richiesta di rete non è andata a buon fine: fetch dice solo "fetch failed", il motivo sta nella causa. */
function motivoDiRete(e: unknown): string {
  const causa = e instanceof Error ? (e.cause as { code?: string; message?: string } | undefined) : undefined;
  const dettaglio = causa?.code || causa?.message;
  return dettaglio ? `${messaggio(e)} (${dettaglio})` : messaggio(e);
}

/** Il testo ICS, dalla copia se fresca. -> [testo, errore morbido]; testo null se non c'è né rete né copia. */
export async function testoCalendario(id: string, url: string): Promise<[string | null, string | null]> {
  const percorso = percorsoCache(id);
  let fresca = false;
  try {
    fresca = adesso() / 1000 - fs.statSync(percorso).mtimeMs / 1000 < TTL_S;
  } catch {
    // non c'è
  }
  if (fresca) {
    try {
      return [fs.readFileSync(percorso, "utf-8"), null];
    } catch {
      // illeggibile: si riscarica
    }
  }
  try {
    const testo = await scarica(url);
    salvaCache(id, testo);
    return [testo, null];
  } catch (e) {
    // Rete assente: meglio dati vecchi che nessun dato
    try {
      return [fs.readFileSync(percorso, "utf-8"), `aggiornamento non riuscito (${motivoDiRete(e)})`];
    } catch {
      return [null, motivoDiRete(e)];
    }
  }
}

// ---- Aggiunta e rimozione di un calendario da link ---------------------------------------------------------------------------
const ILLEGGIBILE = "Il link risponde ma non contiene un calendario iCal valido. Assicurati di aver copiato l'indirizzo segreto in formato iCal.";

/**
 * Aggiunge un calendario da link dopo aver controllato che il link risponda e che sia un calendario: il controllo
 * lo fa l'agente, con la stessa libreria che poi espande le ricorrenze. Solleva ErroreValore con il testo che la pagina mostra.
 */
export async function aggiungiFeed(nome: unknown, url: unknown): Promise<{ id: string; name: string; url: string }> {
  const indirizzo = normalizzaUrl(url);
  if (!/^https?:\/\//i.test(indirizzo)) throw new ErroreValore("Il link deve iniziare con https:// (o webcal://).");
  let testo: string;
  try {
    testo = await scarica(indirizzo);
  } catch (e) {
    throw new ErroreValore(`Il link non risponde: ${motivoDiRete(e)}`);
  }
  const verifica = await invia("/api/ics", { testo });
  if (verifica.ok !== true) {
    if (verifica.valore === true) throw new ErroreValore(ILLEGGIBILE);
    throw new Error(String(verifica.errore ?? "risposta dell'agente non valida"));
  }
  const id = feedId(indirizzo);
  // Il confronto con i calendari che ci sono e l'aggiunta sono un blocco solo, sulla configurazione di adesso: il download può essere lento
  const feed = modificaConfig((cfg) => {
    if (!Array.isArray(cfg.ics_calendars)) cfg.ics_calendars = [];
    const feeds = cfg.ics_calendars as Feed[];
    if (feeds.some((f) => f.id === id)) throw new ErroreValore("Questo calendario è già presente.");
    const nuovo = { id, name: testoPulito(nome) || String(verifica.nome || "Calendario ICS"), url: indirizzo };
    feeds.push(nuovo);
    return nuovo;
  });
  salvaCache(id, testo);
  return feed;
}

/** Toglie un calendario da link: la voce, l'interruttore e la copia del testo. Un id che non c'è non cambia niente. */
export function rimuoviFeed(id: unknown): void {
  modificaConfig((cfg) => {
    cfg.ics_calendars = feedDi(cfg).filter((f) => f.id !== id);
    impostaAcceso(cfg, String(id), true);
  });
  togliCache(String(id));
}

// ---- Lettura eventi ------------------------------------------------------------------------------------------------------
type DaAgente = { uid: string; inizio: string; fine: string; tutto_giorno: boolean; titolo: string; luogo: string; organizzatore: string; link: string | null };

// L'ultima espansione riuscita di ogni calendario e intervallo: se l'agente non risponde, le festività e gli altri link restano
// quelli di un minuto fa invece di sparire (il servizio Python li espandeva da sé e non dipendeva dall'agente).
const MAX_ESPANSIONI = 64;
const espansioni = () => ((globalThis as typeof globalThis & { __oeEspansioni?: Map<string, DaAgente[]> }).__oeEspansioni ??= new Map());

/** Le ricorrenze del testo nell'intervallo, espanse dall'agente. Solleva con il testo dell'errore se il calendario non si legge. */
export async function espandi(testo: string, inizio: Naive, fine: Naive): Promise<DaAgente[]> {
  const chiave = `${crypto.createHash("sha1").update(testo, "utf-8").digest("hex")}|${iso(inizio)}|${iso(fine)}`;
  let r;
  try {
    r = await invia("/api/ics", { testo, inizio: iso(inizio), fine: iso(fine) });
  } catch (e) {
    const ultima = e instanceof AgenteNonDisponibile ? espansioni().get(chiave) : undefined;
    if (!ultima) throw e;
    console.warn(`ics: l'agente non risponde (${messaggio(e)}), si usa l'ultima lettura`);
    return ultima;
  }
  if (r.ok !== true) throw new Error(String(r.errore ?? "risposta dell'agente non valida"));
  const cache = espansioni();
  cache.delete(chiave);
  cache.set(chiave, r.eventi as DaAgente[]);
  if (cache.size > MAX_ESPANSIONI) cache.delete(cache.keys().next().value as string);
  return r.eventi as DaAgente[];
}

/** Gli eventi dei calendari ICS accesi nell'intervallo (ora locale senza fuso). -> [eventi, errori] */
export async function eventiFeed(cfg: Config, inizio: Naive, fine: Naive): Promise<[Ev[], string[]]> {
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  const eventi: Ev[] = [];
  const errori: string[] = [];
  for (const feed of [feedFestivita(), ...feedDi(cfg)]) {
    const builtin = Boolean(feed.builtin);
    const spento = disabled.has(feed.id);
    if (spento && !builtin) continue;
    // Le festività si leggono anche da spente (costa nulla: c'è la copia locale) ma partono marcate "nascosto": l'interfaccia le
    // scarta sia dalla griglia sia dalla riga sotto il giorno
    const nome = feed.name || feed.url;
    const [testo, errore] = await testoCalendario(feed.id, feed.url);
    // Se il download fallisce ma la copia c'è, il calendario si vede comunque: è un avviso, non un guasto
    statoFeed()[feed.id] = errore ? { stato: testo ? "avviso" : "errore", msg: errore } : { stato: "ok", msg: "Aggiornato" };
    if (errore) errori.push(`${nome}: ${errore}`);
    if (!testo) continue;
    let trovati: DaAgente[];
    try {
      trovati = await espandi(testo, inizio, fine);
    } catch (e) {
      // Un calendario malformato non deve far cadere il resto
      errori.push(`${nome}: calendario illeggibile (${messaggio(e)})`);
      continue;
    }
    const colore = builtin ? COLORE_FESTIVITA : colorForAccount(cfg, nome);
    for (const ev of trovati) {
      eventi.push({
        id: `${feed.id}|${ev.uid}|${ev.inizio}`,
        account: nome,
        alias: nome, // per gli ICS il nome scelto è già l'etichetta
        calendar: nome,
        color: colore,
        title: ev.titolo || "(senza titolo)",
        start: ev.inizio,
        end: ev.fine,
        allDay: Boolean(ev.tutto_giorno),
        location: ev.luogo,
        organizer: ev.organizzatore,
        online_url: ev.link,
        web_link: null,
        nascosto: spento,
      });
    }
  }
  return [eventi, errori];
}
