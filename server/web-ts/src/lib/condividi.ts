import crypto from "node:crypto";
import { isIP } from "node:net";
import { nomePubblicato } from "./condivisione-comune";
import type { Condivisione } from "./core/condivisione";
import { inEventiUscita, LetturaNonDisponibile, type Lettura } from "./core/condivisione-eventi";
import { adesso as oraDelCore } from "./core/orologio";
import { oggiARoma } from "./disponibilita";
import { costruisciIcs } from "./ics-uscita";

// GET /condividi/<testo segreto>.ics: il calendario di un link di condivisione (lib/core/condivisione.ts), senza sessione. Chi ha il link legge il
// calendario, e basta: l'indirizzo è la sola chiave. Un indirizzo che non vale (non c'è, è revocato, non ha la forma giusta) riceve sempre la stessa
// risposta, la stessa che dà un percorso che non c'è. Il testo di un calendario si tiene un minuto per link, così un lettore insistente non carica l'agente.

export const TTL_CORPO_MS = 60_000;
export const TTL_GUASTO_MS = 15_000;
/** Quanto chi legge può tenere la risposta senza richiederla: 5 minuti, un sesto dei 30 che il calendario stesso dichiara. */
export const MAX_AGE_S = 300;
/** Gli indirizzi sbagliati che un indirizzo di rete può provare in una finestra prima di ricevere 429 al posto di 404. */
export const FALLIMENTI_MAX = 20;
export const FINESTRA_FALLIMENTI_MS = 60_000;
const INDIRIZZI_MAX = 1000;
const MEMORIA_MAX = 100;

export type Deps = {
  adesso?: () => number;
  /** Il link che ha questo testo segreto, o null (lib/core/condivisione.ts: il confronto è in tempo costante). */
  verifica: (testo: string) => Condivisione | null;
  /** Cosa pubblica il link oggi. Solleva LetturaNonDisponibile se non si può dire. */
  leggi: (link: Condivisione, oggi: string) => Promise<Lettura>;
  /** Segna che il link è stato letto. */
  accesso: (id: string, ms: number) => void;
};

type Buono = { ok: true; versione: number; fino: number; corpo: string; etag: string; modificato: number };
type Guasto = { ok: false; versione: number; fino: number; buono?: Buono };
type Voce = Buono | Guasto;
const memoria = new Map<string, Voce>();
const inCorso = new Map<string, Promise<Voce>>();
const fallimenti = new Map<string, { da: number; n: number }>();

/** Svuota la memoria (per le prove). */
export function azzera(): void {
  memoria.clear();
  inCorso.clear();
  fallimenti.clear();
}

// ---- Le risposte --------------------------------------------------------------------------------------------------------------
const COMUNI = { "X-Robots-Tag": "noindex, nofollow", "X-Content-Type-Options": "nosniff" };
const senzaCorpo = (req: Request) => req.method === "HEAD";

/** Il 404 di un percorso che non c'è: lo stesso corpo e le stesse intestazioni per ogni indirizzo che non vale, qualunque sia il motivo. */
function nonTrovato(req: Request): Response {
  return new Response(senzaCorpo(req) ? null : JSON.stringify({ error: "non trovato" }), {
    status: 404,
    headers: { "Content-Type": "application/json", "Cache-Control": "no-store", ...COMUNI },
  });
}

function nonDisponibile(req: Request): Response {
  return new Response(senzaCorpo(req) ? null : "Calendario non disponibile in questo momento: riprova fra poco.\n", {
    status: 503,
    headers: { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store", "Retry-After": "120", ...COMUNI },
  });
}

// ---- L'indirizzo di chi chiede ------------------------------------------------------------------------------------------------
/**
 * Chi sta chiedendo, per contare gli errori. Il gateway sta dietro Caddy (e davanti a Caddy c'è sslh): quello che conta è l'ultimo
 * indirizzo di X-Forwarded-For, che è quello che Caddy ha visto e ha scritto lui; i precedenti li può scrivere chi chiede, e non si guardano. Dietro
 * sslh Caddy vede sempre lo stesso indirizzo, quindi il conto è uno solo per tutti. Senza l'intestazione (una richiesta diretta) o con un valore
 * che non è un indirizzo, la chiave è una sola.
 */
export function chiaveClient(h: Headers): string {
  const ultimo = h.get("x-forwarded-for")?.split(",").pop()?.trim() ?? "";
  return isIP(ultimo) ? ultimo : "sconosciuto";
}

function contaFallimento(chiave: string, ora: number): { n: number; fra: number } {
  let v = fallimenti.get(chiave);
  if (!v || ora - v.da >= FINESTRA_FALLIMENTI_MS) v = { da: ora, n: 0 };
  v.n++;
  fallimenti.delete(chiave);
  fallimenti.set(chiave, v);
  // gli indirizzi si contano a decine di migliaia solo se qualcuno li inventa: si tengono gli ultimi
  if (fallimenti.size > INDIRIZZI_MAX) fallimenti.delete(fallimenti.keys().next().value as string);
  if (v.n === FALLIMENTI_MAX + 1) console.warn(`condivisione: troppi indirizzi sbagliati da ${chiave}, per un minuto la risposta è 429`);
  return { n: v.n, fra: Math.max(1, Math.ceil((v.da + FINESTRA_FALLIMENTI_MS - ora) / 1000)) };
}

/** Un indirizzo che non vale: 404, o 429 se chi chiede ne ha provati troppi. Un indirizzo che vale non passa di qui, e non si ferma mai. */
function rifiuta(req: Request, ora: number): Response {
  const { n, fra } = contaFallimento(chiaveClient(req.headers), ora);
  if (n <= FALLIMENTI_MAX) return nonTrovato(req);
  return new Response(senzaCorpo(req) ? null : "Troppe richieste: riprova fra poco.\n", {
    status: 429,
    headers: { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store", "Retry-After": String(fra), ...COMUNI },
  });
}

// ---- Il testo del calendario --------------------------------------------------------------------------------------------------
async function genera(link: Condivisione, ora: number, precedente: Voce | undefined, deps: Deps): Promise<Voce> {
  const versione = link.aggiornato;
  const buonoPrima = precedente?.ok ? precedente : precedente?.buono;
  try {
    const eventi = inEventiUscita(await deps.leggi(link, oggiARoma(new Date(ora))));
    const costruisci = (stamp: number) => costruisciIcs({ nome: nomePubblicato(link.nome_calendario), sale: link.sale, livello: link.livello, stamp, eventi });
    // DTSTAMP è l'ora in cui il contenuto è cambiato, non quella della lettura: se non è cambiato niente il testo, e con lui ETag e Last-Modified, restano gli stessi
    let modificato = buonoPrima?.modificato ?? ora;
    let corpo = costruisci(modificato);
    if (buonoPrima && corpo !== buonoPrima.corpo) {
      modificato = ora;
      corpo = costruisci(modificato);
    }
    const etag = `"${crypto.createHash("sha256").update(corpo, "utf8").digest("hex").slice(0, 32)}"`;
    return { ok: true, versione, fino: ora + TTL_CORPO_MS, corpo, etag, modificato };
  } catch (e) {
    if (!(e instanceof LetturaNonDisponibile)) console.error(`condivisione ${link.id}: lettura non riuscita`, e);
    return { ok: false, versione, fino: ora + TTL_GUASTO_MS, buono: buonoPrima };
  }
}

/** Il testo del calendario di un link: quello che si è generato da meno di un minuto, o uno nuovo (richieste uguali in corso fanno una lettura sola). */
async function corpoDi(link: Condivisione, ora: number, deps: Deps): Promise<Voce> {
  const salvata = memoria.get(link.id);
  if (salvata && salvata.versione === link.aggiornato && salvata.fino > ora) return salvata;
  const chiave = `${link.id}|${link.aggiornato}`;
  const incorso = inCorso.get(chiave);
  if (incorso) return incorso;
  const promessa = genera(link, ora, salvata, deps)
    .then((voce) => {
      memoria.delete(link.id);
      memoria.set(link.id, voce);
      if (memoria.size > MEMORIA_MAX) memoria.delete(memoria.keys().next().value as string);
      return voce;
    })
    .finally(() => inCorso.delete(chiave));
  inCorso.set(chiave, promessa);
  return promessa;
}

/** La richiesta ripete quello che ha già (If-None-Match, o If-Modified-Since se non c'è l'altra): 304. */
function giaAggiornato(h: Headers, voce: Buono): boolean {
  const corrisponde = h.get("if-none-match");
  if (corrisponde !== null) return corrisponde.trim() === "*" || corrisponde.split(",").some((x) => x.trim().replace(/^W\//, "") === voce.etag);
  const dal = Date.parse(h.get("if-modified-since") ?? "");
  return Number.isFinite(dal) && Math.floor(voce.modificato / 1000) <= Math.floor(dal / 1000);
}

/** Il percorso /condividi/<testo>.ics: il testo fra /condividi/ e .ics, o una stringa vuota per qualunque altro percorso. */
const testoDelPercorso = (percorso: string) => /^\/condividi\/([^/]{1,200})\.ics$/.exec(percorso)?.[1] ?? "";

/** GET e HEAD di /condividi/<testo segreto>.ics. */
export async function condividi(req: Request, deps: Deps): Promise<Response> {
  const ora = deps.adesso?.() ?? oraDelCore();
  if (req.method !== "GET" && req.method !== "HEAD") {
    return new Response(null, { status: 405, headers: { Allow: "GET, HEAD", "Cache-Control": "no-store", ...COMUNI } });
  }
  let link: Condivisione | null;
  try {
    link = deps.verifica(testoDelPercorso(new URL(req.url).pathname));
  } catch (e) {
    console.error("condivisione: link non verificabile", e);
    return nonDisponibile(req);
  }
  if (!link) return rifiuta(req, ora);
  deps.accesso(link.id, ora);
  const voce = await corpoDi(link, ora, deps);
  if (!voce.ok) return nonDisponibile(req);
  const intestazioni = {
    "Cache-Control": `private, max-age=${MAX_AGE_S}`,
    ETag: voce.etag,
    "Last-Modified": new Date(voce.modificato).toUTCString(),
    ...COMUNI,
  };
  if (giaAggiornato(req.headers, voce)) return new Response(null, { status: 304, headers: intestazioni });
  return new Response(senzaCorpo(req) ? null : voce.corpo, {
    status: 200,
    headers: {
      ...intestazioni,
      "Content-Type": "text/calendar; charset=utf-8",
      "Content-Length": String(Buffer.byteLength(voce.corpo, "utf8")),
      "Content-Disposition": 'inline; filename="calendario.ics"',
    },
  });
}
