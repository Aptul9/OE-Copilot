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
import { type DettaglioHttp, ErroreRete, type Opzioni, type Scaricato, scaricaSicuro } from "./rete-sicura";
import { colorForAccount, impostaAcceso, modificaConfig, type Config } from "./stato";
import { posti, reteDiProva, TROPPE_RICHIESTE } from "./uscita";

// Calendari sottoscritti con un link ICS (era ics_feed.py, fino al commit 142e789): Google, Apple, le festività italiane. Il link è di fatto una chiave: chi ce
// l'ha legge il calendario, quindi sta solo nella configurazione sul server. Il testo si scarica da qui, con la sua copia su disco
// e il suo ritardo; le ricorrenze (RRULE, EXDATE, RECURRENCE-ID, fusi) le espande l'agente, la stessa libreria che legge il telefono.
// Il link lo sceglie chi usa l'app: lo scarico passa dalla guardia di rete-sicura.ts (solo siti pubblici, come per i loghi).

/** Google rigenera l'ICS ogni qualche ora: scaricarlo a ogni cambio di vista sarebbe inutile e lento. */
export const TTL_S = 1800;
/** Quanto può essere grande un calendario: si legge e si accetta fino a qui. */
export const LIMITE_ICS = 8 * 1024 * 1024;
const ATTESA_MS = 15_000;
/** Il Content-Type non è richiesto; se c'è, è uno di questi. Quello che decide sono i primi byte (inizioCalendario). */
const TIPI_ICS = new Set(["text/calendar", "text/plain", "application/octet-stream"]);

/** Come si scarica un calendario: i limiti del calendario, non quelli dei loghi; le regole sugli indirizzi sono quelle di sempre. */
export const OPZIONI_ICS: Opzioni = {
  limiteLettura: LIMITE_ICS,
  limiteAccettato: LIMITE_ICS,
  attesaMs: ATTESA_MS,
  accept: "text/calendar, text/plain;q=0.9, */*;q=0.1",
  userAgent: "CalendarHub/1.0",
  testoTroppoGrande: `Il calendario è troppo grande (massimo ${LIMITE_ICS / 1024 / 1024} MB)`,
};

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

function decodifica(corpo: Uint8Array, tipo: string | null): string {
  // Come requests: il charset dell'intestazione; per un text/* senza charset ISO-8859-1; altrimenti UTF-8
  const charset = /charset=["']?([\w-]+)/i.exec(tipo ?? "")?.[1] ?? (/text/i.test(tipo ?? "") ? "iso-8859-1" : "utf-8");
  try {
    return new TextDecoder(charset).decode(corpo);
  } catch {
    return new TextDecoder("utf-8").decode(corpo);
  }
}

/** Il testo con cui requests rifiuta uno stato HTTP da 400 in su (raise_for_status). L'indirizzo è quello dell'ultima tappa: ha passato i controlli, non può essere interno. */
function erroreHttp(d: DettaglioHttp): string {
  return `${d.stato} ${d.stato >= 500 ? "Server" : "Client"} Error: ${d.testo} for url: ${d.url}`;
}

/** Uno stato HTTP da 400 in su: il testo è quello di requests, lo stato serve a chi deve decidere cosa fare. */
export class ErroreHttp extends Error {
  constructor(
    readonly stato: number,
    messaggio: string,
  ) {
    super(messaggio);
  }
}

/** Stati con cui il proprietario di un link dice che non c'è più: un calendario revocato o con l'indirizzo cambiato (anche Google, quando rigenera l'indirizzo segreto). */
const LINK_FINITO = new Set([404, 410]);

/** Una risposta che non è un calendario iCal: lo dice il Content-Type, o i primi byte. */
class ErroreNonIcs extends ErroreValore {
  constructor() {
    super("il link risponde ma non contiene un calendario iCal");
  }
}

const INIZIO_ICS = Buffer.from("BEGIN:VCALENDAR", "latin1");
const minuscolo = (b: number) => (b >= 0x41 && b <= 0x5a ? b + 0x20 : b);

/** Dopo il BOM UTF-8 (se c'è: `bom` byte) e degli spazi comincia BEGIN:VCALENDAR (le maiuscole non contano, come nel formato)? */
function inizioCalendario(d: Uint8Array, bom: number): boolean {
  let i = bom;
  while (i < d.length && (d[i] === 0x20 || d[i] === 0x09 || d[i] === 0x0a || d[i] === 0x0d)) i++;
  return d.length - i >= INIZIO_ICS.length && INIZIO_ICS.every((b, j) => minuscolo(d[i + j]) === minuscolo(b));
}

/**
 * Il testo di un calendario da un link. Passa dalla guardia di rete-sicura.ts (solo siti pubblici, tutte le tappe) e da uno dei posti
 * dei collegamenti verso l'esterno (uscita.ts). Una risposta che non comincia come un calendario non è un calendario: non finisce né
 * nella copia né all'agente. Uno stato HTTP da 400 in su solleva ErroreHttp, con il testo di sempre.
 */
export async function scarica(url: string): Promise<string> {
  let r: Scaricato;
  try {
    r = await posti().esegui(() => scaricaSicuro(url, reteDiProva(), OPZIONI_ICS));
  } catch (e) {
    if (e instanceof ErroreRete && e.http) throw new ErroreHttp(e.http.stato, erroreHttp(e.http));
    throw e;
  }
  const tipo = (r.tipoDichiarato ?? "").split(";")[0].trim().toLowerCase();
  const bom = r.dati[0] === 0xef && r.dati[1] === 0xbb && r.dati[2] === 0xbf ? 3 : 0;
  if ((tipo && !TIPI_ICS.has(tipo)) || !inizioCalendario(r.dati, bom)) throw new ErroreNonIcs();
  return decodifica(r.dati.subarray(bom), r.tipoDichiarato);
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
    // Un link che non c'è più (404, 410) è stato revocato o cambiato da chi lo aveva dato: la copia che si ha non è più un dato che si può mostrare, e si toglie.
    // Per le festività, che sono nostre, no: lì un 404 è un guasto del servizio
    if (e instanceof ErroreHttp && LINK_FINITO.has(e.stato) && id !== FESTIVITA_ID) {
      togliCache(id);
      return [null, `il link non risponde più (${e.stato}): è stato revocato o cambiato da chi lo aveva dato`];
    }
    // Rete assente, o il servizio che non risponde: meglio dati vecchi che nessun dato
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
    if (e instanceof ErroreNonIcs) throw new ErroreValore(ILLEGGIBILE);
    // la coda piena non dice niente del link
    if (messaggio(e) === TROPPE_RICHIESTE) throw new ErroreValore(TROPPE_RICHIESTE);
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
/** I calendari che un altro OE Copilot pubblica con un link di condivisione portano questa riga e questo PRODID (lib/ics-uscita.ts): i loro eventi si riconoscono, ma solo se ci sono tutti e due. */
const DA_CONDIVISIONE = /^X-OE-COPILOT-SHARE:1[ \t]*\r?$/m;
const PRODID_CONDIVISIONE = /^PRODID:-\/\/OE Copilot\/\/Condivisione\/\/IT[ \t]*\r?$/m;

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
    // gli eventi di un calendario condiviso da un altro OE Copilot restano segnati: la disponibilità pubblica non li conta come impegni di chi li ha importato
    const condiviso = DA_CONDIVISIONE.test(testo) && PRODID_CONDIVISIONE.test(testo);
    for (const ev of trovati) {
      eventi.push({
        ...(condiviso ? { da_condivisione: true } : {}),
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
