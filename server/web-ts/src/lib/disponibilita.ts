import { config } from "./config";
import type { Evento } from "./oe/types";

// La pagina pubblica (/disponibilita) è il calendario di sempre, senza accesso, con gli impegni ridotti
// a "Occupato". Qui stanno le regole dell'API che la serve (/api/disponibilita): dal servizio ui arrivano
// solo orari e festività (disponibilita.py), e quello che arriva si ricostruisce campo per campo.

export const FUSO = "Europe/Rome";
export const OCCUPATO = "Occupato";
export const COLORE_OCCUPATO = "#6366f1";
/** Quanto si può chiedere in una volta, e fin dove indietro e avanti dal giorno d'oggi. */
export const RANGE_MAX = 62;
export const ANNI_MAX_GIORNI = 400;
export const GRUPPO_MAX = 60;
/** Le letture dal servizio ui sono a blocchi di 28 giorni fissi: chi guarda sposta le viste dentro lo stesso blocco. */
export const GIORNI_BLOCCO = 28;
const ANCORA = "2024-01-01"; // un lunedì: da qui partono i blocchi

export type Coppia = [string, string];
export type Festa = [string, string, string];

// ---- Date: sempre testi AAAA-MM-GG, senza il fuso orario del server ---------------------------------
export function dataValida(s: string): boolean {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return false;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]));
  return d.getUTCFullYear() === +m[1] && d.getUTCMonth() === +m[2] - 1 && d.getUTCDate() === +m[3];
}

const giorno = (s: string) => {
  const [a, m, g] = s.split("-").map(Number);
  return Date.UTC(a, m - 1, g);
};
export const giorniTra = (da: string, a: string) => Math.round((giorno(a) - giorno(da)) / 86400000);
export const spostaData = (data: string, giorni: number) => new Date(giorno(data) + giorni * 86400000).toISOString().slice(0, 10);

/** Che giorno è a Roma, qualunque fuso abbia il server. */
export const oggiARoma = (adesso = new Date()) => new Intl.DateTimeFormat("en-CA", { timeZone: FUSO, year: "numeric", month: "2-digit", day: "2-digit" }).format(adesso);

// ---- La richiesta ----------------------------------------------------------------------------------
export type Richiesta = { da: string; a: string; gruppo: string };

/** `da` e `a` come li manda la pagina (2026-10-12T00:00:00+02:00): conta il giorno, `a` è escluso. */
export function leggiRichiesta(q: URLSearchParams): Richiesta | { errore: string } {
  const da = (q.get("da") ?? "").slice(0, 10);
  const a = (q.get("a") ?? "").slice(0, 10);
  if (!dataValida(da) || !dataValida(a)) return { errore: "da, a: servono date AAAA-MM-GG" };
  const giorni = giorniTra(da, a);
  if (giorni < 1 || giorni > RANGE_MAX) return { errore: `a: da 1 a ${RANGE_MAX} giorni dopo da` };
  const gruppo = (q.get("gruppo") ?? "").trim();
  // eslint-disable-next-line no-control-regex
  if (gruppo.length > GRUPPO_MAX || /[\u0000-\u001f\u007f]/.test(gruppo)) return { errore: "gruppo: nome non valido" };
  return { da, a, gruppo };
}

/** Troppo indietro o troppo avanti: si risponde senza impegni, senza disturbare i calendari. */
export const fuoriFinestra = (r: Richiesta, oggi: string) => r.a < spostaData(oggi, -ANNI_MAX_GIORNI) || r.da > spostaData(oggi, ANNI_MAX_GIORNI);

/** I blocchi fissi (la loro data di inizio) che coprono da `da` a `a`. */
export function blocchi(da: string, a: string): string[] {
  const primo = Math.floor(giorniTra(ANCORA, da) / GIORNI_BLOCCO);
  const ultimo = Math.floor(giorniTra(ANCORA, spostaData(a, -1)) / GIORNI_BLOCCO);
  return Array.from({ length: ultimo - primo + 1 }, (_, i) => spostaData(ANCORA, (primo + i) * GIORNI_BLOCCO));
}

// ---- Gli eventi della pagina -----------------------------------------------------------------------
/** Impegni e festività dei blocchi, tenuti solo se toccano da-a, nella forma degli eventi del calendario. */
export function eventiPubblici(impegni: Coppia[], festivita: Festa[], da: string, a: string): Evento[] {
  const inizio = `${da}T00:00:00`;
  const fine = `${a}T00:00:00`;
  const visti = new Set<string>();
  const out: Evento[] = [];
  for (const [i, f] of unisci(impegni)) {
    if (f <= inizio || i >= fine) continue;
    out.push({ id: `occ${out.length}`, title: OCCUPATO, start: i, end: f, allDay: false, color: COLORE_OCCUPATO, account: "" });
  }
  for (const [i, f, nome] of festivita) {
    const chiave = `${i}|${f}|${nome}`;
    if (visti.has(chiave) || f <= inizio || i >= fine) continue;
    visti.add(chiave);
    out.push({ id: `festa${out.length}`, title: nome, start: i, end: f, allDay: true, color: "#8a8a8a", account: "Festività" });
  }
  return out;
}

/**
 * Se si è occupati, si è occupati: impegni che si sovrappongono o si toccano sono un blocco solo. Il servizio
 * ui li unisce già, ma ogni blocco di 28 giorni lo fa per conto suo: un impegno lungo a cavallo di due blocchi
 * arriva due volte, con inizi diversi. Gli orari sono testi AAAA-MM-GGTHH:MM:SS, si confrontano come testi.
 */
export function unisci(impegni: Coppia[]): Coppia[] {
  const uniti: Coppia[] = [];
  for (const [i, f] of [...impegni].sort((x, y) => (x[0] < y[0] ? -1 : x[0] > y[0] ? 1 : 0))) {
    const ultimo = uniti[uniti.length - 1];
    if (ultimo && i <= ultimo[1]) {
      if (f > ultimo[1]) ultimo[1] = f;
    } else uniti.push([i, f]);
  }
  return uniti;
}

// ---- Lettura dal servizio ui -----------------------------------------------------------------------
export type Blocco = { impegni: Coppia[]; festivita: Festa[]; completo: boolean };
export type Esito = ({ ok: true } & Blocco) | { ok: false; stato: 404 | 429 | 502 };
type Deps = { fetch?: typeof fetch; upstream?: string; adesso?: () => number };

const TTL_MS = 60_000;
const TTL_GUASTO_MS = 15_000;
const CACHE_MAX = 64;
const LETTURE_MAX = 20;
const FINESTRA_MS = 60_000;
const cache = new Map<string, { fino: number; esito: Esito }>();
const inCorso = new Map<string, Promise<Esito>>();
const letture: number[] = [];

/** Svuota la memoria (per le prove). */
export function azzera() {
  cache.clear();
  inCorso.clear();
  letture.length = 0;
}

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/;
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u001f\u007f]/g;

/** Quello che arriva dal servizio ui si ricostruisce: solo coppie di orari e nomi di festa, il resto non passa. */
function ripulisci(dati: unknown): Esito {
  const guasto: Esito = { ok: false, stato: 502 };
  if (!dati || typeof dati !== "object") return guasto;
  const d = dati as { ok?: unknown; error?: unknown; impegni?: unknown; festivita?: unknown; completo?: unknown };
  if (d.ok === false) return d.error === "Gruppo non trovato" ? { ok: false, stato: 404 } : guasto;
  if (d.ok !== true || !Array.isArray(d.impegni) || !Array.isArray(d.festivita) || d.impegni.length > 5000 || d.festivita.length > 200) return guasto;
  const impegni: Coppia[] = [];
  for (const x of d.impegni) {
    if (!Array.isArray(x) || x.length !== 2 || !ISO.test(x[0]) || !ISO.test(x[1]) || x[0] >= x[1]) return guasto;
    impegni.push([x[0], x[1]]);
  }
  const festivita: Festa[] = [];
  for (const x of d.festivita) {
    if (!Array.isArray(x) || x.length !== 3 || !ISO.test(x[0]) || !ISO.test(x[1]) || typeof x[2] !== "string") return guasto;
    festivita.push([x[0], x[1], x[2].replace(CONTROLLO, " ").slice(0, 80)]);
  }
  return { ok: true, impegni, festivita, completo: d.completo === true };
}

async function leggi(da: string, gruppo: string, deps: Deps): Promise<Esito> {
  try {
    const r = await (deps.fetch ?? fetch)((deps.upstream ?? config.upstream) + "/api/get_disponibilita", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify([da, GIORNI_BLOCCO, gruppo]),
      cache: "no-store",
      signal: AbortSignal.timeout(30_000),
    });
    return ripulisci(await r.json());
  } catch {
    return { ok: false, stato: 502 };
  }
}

/**
 * Un blocco di 28 giorni, per una richiesta pubblica. Chiunque può chiamarla: il risultato resta in
 * memoria un minuto (un guasto quindici secondi), richieste uguali in corso si fondono in una lettura,
 * e le letture vere dei calendari sono al più venti al minuto in tutto (dietro sslh i client non si
 * distinguono, quindi il limite è uno solo): oltre, 429.
 */
export async function leggiBlocco(da: string, gruppo: string, deps: Deps = {}): Promise<Esito> {
  const adesso = deps.adesso?.() ?? Date.now();
  const chiave = `${da}|${gruppo}`;
  const salvata = cache.get(chiave);
  if (salvata && salvata.fino > adesso) return salvata.esito;
  const inLettura = inCorso.get(chiave);
  if (inLettura) return inLettura;
  while (letture.length && letture[0] <= adesso - FINESTRA_MS) letture.shift();
  if (letture.length >= LETTURE_MAX) return { ok: false, stato: 429 };
  letture.push(adesso);
  const promessa = leggi(da, gruppo, deps)
    .then((esito) => {
      if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value as string);
      const buono = esito.ok ? esito.completo : esito.stato === 404;
      cache.set(chiave, { fino: adesso + (buono ? TTL_MS : TTL_GUASTO_MS), esito });
      return esito;
    })
    .finally(() => inCorso.delete(chiave));
  inCorso.set(chiave, promessa);
  return promessa;
}

export const MESSAGGIO_INCOMPLETO = "Alcuni calendari non sono aggiornati al momento: gli orari occupati potrebbero essere incompleti.";

/** Il corpo di /api/disponibilita: nella forma di get_events, così la pagina legge tutto come sempre. */
export async function leggiDisponibilita(r: Richiesta, deps: Deps = {}): Promise<{ ok: true; events: Evento[]; errors: string[] } | { ok: false; stato: 404 | 429 | 502 }> {
  const esiti = await Promise.all(blocchi(r.da, r.a).map((b) => leggiBlocco(b, r.gruppo, deps)));
  const guasto = esiti.find((e) => !e.ok);
  if (guasto && !guasto.ok) {
    // il più specifico: gruppo che non esiste, poi troppe richieste
    const ordine = [404, 429, 502] as const;
    const stati = esiti.flatMap((e) => (e.ok ? [] : [e.stato]));
    return { ok: false, stato: ordine.find((s) => stati.includes(s)) ?? 502 };
  }
  const buoni = esiti as Extract<Esito, { ok: true }>[];
  return {
    ok: true,
    events: eventiPubblici(
      buoni.flatMap((e) => e.impegni),
      buoni.flatMap((e) => e.festivita),
      r.da,
      r.a,
    ),
    errors: buoni.every((e) => e.completo) ? [] : [MESSAGGIO_INCOMPLETO],
  };
}
