import type { Evento } from "./oe/types";

// La pagina pubblica (/disponibilita) è il calendario di sempre, senza accesso, con gli impegni ridotti
// a "Occupato". Qui stanno le regole dell'API che la serve (/api/disponibilita): dalla lettura dei calendari
// (get_disponibilita, lib/core) arrivano solo orari e festività, e quello che arriva si ricostruisce campo per campo.

export const FUSO = "Europe/Rome";
export const OCCUPATO = "Occupato";
export const COLORE_OCCUPATO = "#6366f1";
/** Quanto si può chiedere in una volta, e fin dove indietro e avanti dal giorno d'oggi. */
export const RANGE_MAX = 62;
export const ANNI_MAX_GIORNI = 400;
export const GRUPPO_MAX = 60;
/** Le letture sono a blocchi di 28 giorni fissi: chi guarda sposta le viste dentro lo stesso blocco. */
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
 * Se si è occupati, si è occupati: impegni che si sovrappongono o si toccano sono un blocco solo. La lettura
 * li unisce già, ma ogni blocco di 28 giorni lo fa per conto suo: un impegno lungo a cavallo di due blocchi
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

// ---- Lettura dei blocchi ---------------------------------------------------------------------------
export type Blocco = { impegni: Coppia[]; festivita: Festa[]; completo: boolean };
export type Esito = ({ ok: true } & Blocco) | { ok: false; stato: 404 | 429 | 502 };
type Deps = {
  adesso?: () => number;
  /** La lettura dei calendari (lib/core): l'inizio del blocco, i giorni e il gruppo. */
  core: (da: string, giorni: number, gruppo: string) => Promise<unknown>;
};

const TTL_MS = 60_000;
const TTL_GUASTO_MS = 15_000;
/** Fin quando un blocco letto bene si può ancora dare subito, mentre se ne legge uno nuovo: la prenotazione ricontrolla comunque i calendari. */
export const VECCHIO_MAX_MS = 6 * 3600_000;
const CACHE_MAX = 64;
const LETTURE_MAX = 20;
const FINESTRA_MS = 60_000;
/** Il giro che tiene pronti i blocchi: ogni due minuti, da una settimana fa a 61 giorni avanti (le prenotazioni arrivano a 60). */
export const CALDO_OGNI_MS = 120_000;
const CALDO_INDIETRO = 7;
const CALDO_AVANTI = 61;
/** I gruppi chiesti dalla pagina (?gruppo=) restano caldi per un giorno, al più otto. */
const RICHIESTI_PER_MS = 24 * 3600_000;
const RICHIESTI_MAX = 8;

type Voce = { fino: number; scade: number; esito: Esito };
type Memoria = { cache: Map<string, Voce>; inCorso: Map<string, Promise<Esito>>; letture: number[]; richiesti: Map<string, number> };
// In globalThis: le rotte di Next e l'avvio (che fa il giro) hanno ciascuno la sua copia dei moduli, e la memoria deve essere una sola
const globale = globalThis as typeof globalThis & { __oeDisponibilita?: Memoria; __oeDisponibilitaCaldo?: ReturnType<typeof setTimeout> | true };
const memoria = (): Memoria => (globale.__oeDisponibilita ??= { cache: new Map(), inCorso: new Map(), letture: [], richiesti: new Map() });

/** Dimentica i blocchi letti, non il conteggio delle letture: dopo una prenotazione la pagina rilegge e la vede subito. */
export function dimenticaDisponibilita() {
  memoria().cache.clear();
}

/** Svuota la memoria (per le prove). */
export function azzera() {
  globale.__oeDisponibilita = undefined;
}

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/;
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u001f\u007f]/g;

/** Quello che arriva dalla lettura si ricostruisce: solo coppie di orari e nomi di festa, il resto non passa. */
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
    return ripulisci(JSON.parse(JSON.stringify(await deps.core(da, GIORNI_BLOCCO, gruppo))));
  } catch {
    return { ok: false, stato: 502 };
  }
}

/**
 * Legge il blocco e lo mette in memoria (richieste uguali in corso si fondono in una lettura). Una lettura che non riesce non prende il
 * posto di un blocco letto bene: quello resta, e si riprova fra quindici secondi.
 */
function aggiorna(chiave: string, da: string, gruppo: string, deps: Deps, adesso: number): Promise<Esito> {
  const m = memoria();
  const inLettura = m.inCorso.get(chiave);
  if (inLettura) return inLettura;
  const promessa = leggi(da, gruppo, deps)
    .then((esito) => {
      const prima = m.cache.get(chiave);
      if (!esito.ok && esito.stato !== 404 && prima?.esito.ok && prima.scade > adesso) {
        m.cache.set(chiave, { ...prima, fino: adesso + TTL_GUASTO_MS });
        return prima.esito;
      }
      m.cache.delete(chiave);
      if (m.cache.size >= CACHE_MAX) m.cache.delete(m.cache.keys().next().value as string);
      const buono = esito.ok ? esito.completo : esito.stato === 404;
      m.cache.set(chiave, { fino: adesso + (buono ? TTL_MS : TTL_GUASTO_MS), scade: esito.ok ? adesso + VECCHIO_MAX_MS : 0, esito });
      return esito;
    })
    .finally(() => m.inCorso.delete(chiave));
  m.inCorso.set(chiave, promessa);
  return promessa;
}

/** Il limite delle letture vere: al più venti al minuto in tutto. Vero se c'è posto (e lo prende). */
function postoLibero(adesso: number): boolean {
  const { letture } = memoria();
  while (letture.length && letture[0] <= adesso - FINESTRA_MS) letture.shift();
  if (letture.length >= LETTURE_MAX) return false;
  letture.push(adesso);
  return true;
}

/**
 * Un blocco di 28 giorni, per una richiesta pubblica. Chiunque può chiamarla: il risultato resta in memoria un minuto (un guasto
 * quindici secondi), e dopo si dà ancora subito, fino a sei ore, mentre se ne legge uno nuovo: una lettura dei calendari dura secondi
 * e chi apre la pagina non la aspetta. Le letture vere sono al più venti al minuto in tutto (dietro sslh i client non si distinguono,
 * quindi il limite è uno solo): oltre, 429 se non c'è niente da dare.
 */
export async function leggiBlocco(da: string, gruppo: string, deps: Deps): Promise<Esito> {
  const adesso = deps.adesso?.() ?? Date.now();
  const chiave = `${da}|${gruppo}`;
  const m = memoria();
  const salvata = m.cache.get(chiave);
  if (salvata && salvata.fino > adesso) return salvata.esito;
  if (salvata?.esito.ok && salvata.scade > adesso) {
    if (!m.inCorso.has(chiave) && postoLibero(adesso)) void aggiorna(chiave, da, gruppo, deps, adesso);
    return salvata.esito;
  }
  const inLettura = m.inCorso.get(chiave);
  if (inLettura) return inLettura;
  if (!postoLibero(adesso)) return { ok: false, stato: 429 };
  return aggiorna(chiave, da, gruppo, deps, adesso);
}

/** Un gruppo chiesto dalla pagina pubblica: il giro lo tiene caldo per un giorno. */
function ricordaGruppo(gruppo: string, adesso: number) {
  if (!gruppo) return;
  const { richiesti } = memoria();
  richiesti.delete(gruppo);
  richiesti.set(gruppo, adesso);
  if (richiesti.size > RICHIESTI_MAX) richiesti.delete(richiesti.keys().next().value as string);
}

/**
 * Un giro: rilegge, uno alla volta, i blocchi da una settimana fa a 61 giorni avanti, per tutti i calendari e per i gruppi chiesti
 * nell'ultimo giorno. Fuori dal limite delle letture pubbliche, che resta tutto per chi apre la pagina. Restituisce quanti blocchi ha letto.
 */
export async function rinfresca(deps: Deps, oggi: string): Promise<number> {
  const adesso = deps.adesso?.() ?? Date.now();
  const { richiesti } = memoria();
  for (const [g, quando] of richiesti) if (adesso - quando > RICHIESTI_PER_MS) richiesti.delete(g);
  let letti = 0;
  for (const gruppo of ["", ...richiesti.keys()]) {
    for (const b of blocchi(spostaData(oggi, -CALDO_INDIETRO), spostaData(oggi, CALDO_AVANTI))) {
      const esito = await aggiorna(`${b}|${gruppo}`, b, gruppo, deps, deps.adesso?.() ?? Date.now());
      letti++;
      // un gruppo che non c'è più non si rilegge
      if (!esito.ok && esito.stato === 404) {
        richiesti.delete(gruppo);
        break;
      }
    }
  }
  return letti;
}

/** Fa partire il giro, una volta sola per processo: il primo dopo cinque secondi, così la pagina è pronta subito dopo un riavvio. */
export function avviaDisponibilita(deps: Deps): void {
  if (globale.__oeDisponibilitaCaldo) return;
  globale.__oeDisponibilitaCaldo = true;
  const giro = (ms: number) => {
    const t = setTimeout(() => {
      rinfresca(deps, oggiARoma())
        .catch((e) => console.warn("disponibilità: giro non riuscito", e instanceof Error ? e.name : e))
        .finally(() => giro(CALDO_OGNI_MS));
    }, ms);
    t.unref();
    globale.__oeDisponibilitaCaldo = t;
  };
  giro(5_000);
  console.info(`disponibilità: blocchi tenuti pronti, un giro ogni ${CALDO_OGNI_MS / 1000} s`);
}

/** Il corpo di /api/disponibilita: nella forma di get_events, così la pagina legge tutto come sempre. */
export async function leggiDisponibilita(r: Richiesta, deps: Deps): Promise<{ ok: true; events: Evento[]; errors: string[] } | { ok: false; stato: 404 | 429 | 502 }> {
  ricordaGruppo(r.gruppo, deps.adesso?.() ?? Date.now());
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
    // un calendario che non si legge (un accesso scaduto) si vede nella pagina principale, con il pallino rosso: qui non si dice
    errors: [],
  };
}
