import crypto from "node:crypto";
import type Database from "better-sqlite3";
import { config } from "../config";
import {
  AMBITI,
  AVANTI,
  type Ambito,
  type CondivisioneEsito,
  type CondivisioniRisposta,
  type CondivisionePubblica,
  INDIETRO,
  LINK_MAX,
  type Livello,
  LIVELLI,
  NOME_CALENDARIO_MAX,
  NOME_MAX,
} from "../condivisione-comune";
import { GRUPPO_MAX } from "../disponibilita";
import { archivio } from "./archivio";
import { adesso } from "./orologio";
import { isDict } from "./py";
import { loadConfig, membriGruppo } from "./stato";

// I link con cui il calendario si condivide con un altro OE Copilot (lib/condividi.ts li serve). Ogni link ha il suo indirizzo segreto, e chi lo ha
// legge il calendario: per questo il testo segreto non esce da qui se non dentro `url` e non si scrive in nessun log né in nessun errore. Stanno in
// una tabella di gateway.db, aggiunta all'avvio (CREATE TABLE IF NOT EXISTS: si aggiunge, non si tocca niente) e mai nella configurazione:
// config.json, il suo mirror e i file registrati delle prove non hanno una voce nuova.

export type Condivisione = {
  id: string;
  nome: string;
  /** Il testo segreto dell'indirizzo; null quando il link è revocato (si cancella, non resta in un database né in una copia). */
  token: string | null;
  /** Entra solo negli identificativi degli eventi: non è il testo segreto e non si pubblica. */
  sale: string;
  livello: Livello;
  ambito: Ambito;
  gruppo: string;
  giorni_indietro: number;
  giorni_avanti: number;
  nome_calendario: string;
  creato: number;
  aggiornato: number;
  ultimo_accesso: number | null;
  accessi: number;
  revocato: number | null;
};

const SCHEMA = `CREATE TABLE IF NOT EXISTS condivisioni (
  id TEXT PRIMARY KEY,
  nome TEXT NOT NULL,
  token TEXT UNIQUE,
  sale TEXT NOT NULL,
  livello TEXT NOT NULL DEFAULT 'occupato' CHECK (livello IN ('occupato', 'titoli')),
  ambito TEXT NOT NULL DEFAULT 'tutti' CHECK (ambito IN ('tutti', 'gruppo')),
  gruppo TEXT NOT NULL DEFAULT '',
  giorni_indietro INTEGER NOT NULL DEFAULT 7,
  giorni_avanti INTEGER NOT NULL DEFAULT 90,
  nome_calendario TEXT NOT NULL DEFAULT '',
  creato INTEGER NOT NULL,
  aggiornato INTEGER NOT NULL,
  ultimo_accesso INTEGER,
  accessi INTEGER NOT NULL DEFAULT 0,
  revocato INTEGER
)`;
const pronti = new WeakSet<Database.Database>();
function db(): Database.Database {
  const d = archivio();
  if (!pronti.has(d)) {
    d.exec(SCHEMA);
    pronti.add(d);
  }
  return d;
}

/** Crea la tabella dei link se non c'è: la chiama l'avvio del gateway, e comunque il primo uso. */
export const preparaCondivisioni = (): void => void db();

// ---- Il testo segreto -------------------------------------------------------------------------------------------------------
/** 32 byte casuali in base64url, senza riempimento: 43 caratteri. */
export const FORMA_TOKEN = /^[A-Za-z0-9_-]{43}$/;
const nuovoToken = () => crypto.randomBytes(32).toString("base64url");
const nuovoId = () => crypto.randomBytes(6).toString("hex");
const FORMA_ID = /^[0-9a-f]{12}$/;
const impronta = (testo: string) => crypto.createHash("sha256").update(testo, "utf8").digest();

/** L'indirizzo pubblico di un link: APP_URL, il percorso fisso e il testo segreto. */
export const indirizzoLink = (token: string): string => `${config.appUrl}/condividi/${token}.ics`;

/**
 * Il link che ha questo testo segreto, o null se non c'è, è revocato o non ha la forma giusta. Il confronto è fra impronte di lunghezza
 * fissa, con timingSafeEqual, e scorre sempre tutti i link: quanto ci mette non dice quanto il testo somigli a uno vero né quale sia.
 * Un testo che non ha la forma giusta fa lo stesso lavoro con un valore nullo.
 */
export function trovaPerToken(testo: string): Condivisione | null {
  const valido = FORMA_TOKEN.test(testo);
  const dato = impronta(valido ? testo : "");
  let trovata: Condivisione | null = null;
  for (const r of db().prepare("SELECT * FROM condivisioni WHERE token IS NOT NULL AND revocato IS NULL").all() as Condivisione[]) {
    if (crypto.timingSafeEqual(dato, impronta(r.token as string))) trovata = r;
  }
  return valido ? trovata : null;
}

// ---- Gli accessi ------------------------------------------------------------------------------------------------------------
// Chi legge il link lo fa a ogni scadenza della sua copia, ma un lettore insistente non deve fare una scrittura per richiesta: l'ultimo accesso e il
// conto si tengono in memoria e vanno nel database al più una volta al minuto per link. L'elenco legge quello che c'è in memoria insieme al database.
export const INTERVALLO_ACCESSI_MS = 60_000;
type Accessi = { scritto: number; pendenti: number; ultimo: number };
const accessiInMemoria = new Map<string, Accessi>();

/** Segna che il link è stato letto adesso. Un guasto del database non deve rovinare la risposta a chi legge. */
export function registraAccesso(id: string, ms: number): void {
  const v = accessiInMemoria.get(id) ?? { scritto: 0, pendenti: 0, ultimo: 0 };
  accessiInMemoria.set(id, v);
  v.pendenti++;
  v.ultimo = ms;
  if (ms - v.scritto < INTERVALLO_ACCESSI_MS) return;
  try {
    db().prepare("UPDATE condivisioni SET ultimo_accesso = ?, accessi = accessi + ? WHERE id = ?").run(ms, v.pendenti, id);
    v.scritto = ms;
    v.pendenti = 0;
  } catch (e) {
    console.warn(`condivisione ${id}: accesso non registrato`, e);
  }
}

/** Dimentica gli accessi in memoria (per le prove). */
export const azzeraAccessi = (): void => accessiInMemoria.clear();

// ---- Lettura ----------------------------------------------------------------------------------------------------------------
function righe(): Condivisione[] {
  return db().prepare("SELECT * FROM condivisioni ORDER BY creato, id").all() as Condivisione[];
}

function riga(id: unknown): Condivisione | null {
  if (typeof id !== "string" || !FORMA_ID.test(id)) return null;
  return (db().prepare("SELECT * FROM condivisioni WHERE id = ?").get(id) as Condivisione | undefined) ?? null;
}

/** Il link come lo vede la pagina: con gli accessi che aspettano ancora di andare nel database, senza sale, con l'indirizzo se vale ancora. */
function pubblica(c: Condivisione, gruppi: Record<string, unknown> | null): CondivisionePubblica {
  const m = accessiInMemoria.get(c.id);
  const { token, sale, ...resto } = c;
  void sale;
  return {
    ...resto,
    ultimo_accesso: Math.max(c.ultimo_accesso ?? 0, m?.ultimo ?? 0) || null,
    accessi: c.accessi + (m?.pendenti ?? 0),
    url: token && !c.revocato ? indirizzoLink(token) : null,
    gruppo_mancante: c.ambito === "gruppo" && !c.revocato && !(gruppi && Array.isArray(gruppi[c.gruppo])),
  };
}

const gruppiDiAdesso = (): Record<string, unknown> | null => {
  const g = loadConfig().gruppi_calendari;
  return isDict(g) ? g : null;
};

/** Tutti i link, dal più vecchio. */
export function elencoCondivisioni(): CondivisioniRisposta {
  const gruppi = gruppiDiAdesso();
  return { ok: true, condivisioni: righe().map((c) => pubblica(c, gruppi)), max: LINK_MAX };
}

// ---- Scrittura --------------------------------------------------------------------------------------------------------------
type Campi = Pick<Condivisione, "nome" | "livello" | "ambito" | "gruppo" | "giorni_indietro" | "giorni_avanti" | "nome_calendario">;
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/;

function intero(v: unknown): number | null {
  if (typeof v === "number" && Number.isInteger(v)) return v;
  if (typeof v === "string" && /^\s*\d+\s*$/.test(v)) return Number(v);
  return null;
}

/**
 * I campi di un link: `dati` è quello che arriva dalla pagina, `base` i valori che restano se un campo non c'è (quelli di prima, o quelli di
 * partenza). Il testo dell'errore è quello che la pagina mostra: dice cosa non va, mai il testo segreto di un link.
 */
function leggiCampi(dati: unknown, base: Campi | null): Campi | { errore: string } {
  if (!isDict(dati)) return { errore: "I dati del link non sono validi." };
  const c: Campi = base ?? {
    nome: "",
    livello: "occupato",
    ambito: "tutti",
    gruppo: "",
    giorni_indietro: INDIETRO.predefinito,
    giorni_avanti: AVANTI.predefinito,
    nome_calendario: "",
  };
  // il gruppo si controlla quando lo si sceglie, non a ogni modifica: un link il cui gruppo è sparito si può comunque rinominare o revocare
  const sceltaDelGruppo = base === null || dati.ambito !== undefined || dati.gruppo !== undefined;
  if (dati.nome !== undefined) {
    if (typeof dati.nome !== "string") return { errore: "Il nome non è valido." };
    c.nome = dati.nome.trim();
  }
  if (!c.nome) return { errore: "Scrivi per chi è il link: il nome di chi lo riceve." };
  if ([...c.nome].length > NOME_MAX) return { errore: `Il nome è troppo lungo: al più ${NOME_MAX} caratteri.` };
  if (CONTROLLO.test(c.nome)) return { errore: "Il nome non può avere a capo né caratteri di controllo." };

  if (dati.livello !== undefined) {
    if (!(LIVELLI as readonly unknown[]).includes(dati.livello)) return { errore: "Il livello non è valido: scegli Occupato oppure Con titoli." };
    c.livello = dati.livello as Livello;
  }
  if (dati.ambito !== undefined) {
    if (!(AMBITI as readonly unknown[]).includes(dati.ambito)) return { errore: "Cosa condividere non è valido: tutti i calendari, oppure un gruppo." };
    c.ambito = dati.ambito as Ambito;
  }
  if (dati.gruppo !== undefined) {
    if (typeof dati.gruppo !== "string") return { errore: "Il nome del gruppo non è valido." };
    c.gruppo = dati.gruppo.trim();
  }
  if (c.ambito === "tutti") c.gruppo = "";
  else {
    if (!c.gruppo) return { errore: "Scegli il gruppo da condividere." };
    if ([...c.gruppo].length > GRUPPO_MAX || CONTROLLO.test(c.gruppo)) return { errore: "Il nome del gruppo non è valido." };
    if (sceltaDelGruppo && membriGruppo(loadConfig(), c.gruppo) === null) return { errore: `Il gruppo «${c.gruppo}» non esiste: si crea in Gestisci calendari.` };
  }

  if (dati.giorni_indietro !== undefined) {
    const n = intero(dati.giorni_indietro);
    if (n === null || n < INDIETRO.min || n > INDIETRO.max) return { errore: `Giorni indietro: un numero intero da ${INDIETRO.min} a ${INDIETRO.max}.` };
    c.giorni_indietro = n;
  }
  if (dati.giorni_avanti !== undefined) {
    const n = intero(dati.giorni_avanti);
    if (n === null || n < AVANTI.min || n > AVANTI.max) return { errore: `Giorni avanti: un numero intero da ${AVANTI.min} a ${AVANTI.max}.` };
    c.giorni_avanti = n;
  }
  if (dati.nome_calendario !== undefined) {
    if (typeof dati.nome_calendario !== "string") return { errore: "Il nome del calendario non è valido." };
    c.nome_calendario = dati.nome_calendario.trim();
  }
  if ([...c.nome_calendario].length > NOME_CALENDARIO_MAX) return { errore: `Il nome del calendario è troppo lungo: al più ${NOME_CALENDARIO_MAX} caratteri.` };
  if (CONTROLLO.test(c.nome_calendario)) return { errore: "Il nome del calendario non può avere a capo né caratteri di controllo." };
  return c;
}

/** Crea un link: l'indirizzo segreto lo fa il server e la pagina lo riceve nella risposta. */
export function creaCondivisione(dati: unknown): CondivisioneEsito {
  const campi = leggiCampi(dati, null);
  if ("errore" in campi) return { ok: false, error: campi.errore };
  const d = db();
  if ((d.prepare("SELECT COUNT(*) FROM condivisioni").pluck().get() as number) >= LINK_MAX) {
    return { ok: false, error: `Ci sono già ${LINK_MAX} link: elimina quelli che non servono più.` };
  }
  const ora = adesso();
  const nuovo: Condivisione = {
    id: nuovoId(),
    token: nuovoToken(),
    sale: crypto.randomBytes(16).toString("hex"),
    ...campi,
    creato: ora,
    aggiornato: ora,
    ultimo_accesso: null,
    accessi: 0,
    revocato: null,
  };
  d.prepare(
    `INSERT INTO condivisioni (id, nome, token, sale, livello, ambito, gruppo, giorni_indietro, giorni_avanti, nome_calendario, creato, aggiornato, ultimo_accesso, accessi, revocato)
     VALUES (@id, @nome, @token, @sale, @livello, @ambito, @gruppo, @giorni_indietro, @giorni_avanti, @nome_calendario, @creato, @aggiornato, @ultimo_accesso, @accessi, @revocato)`,
  ).run(nuovo);
  console.info(`condivisione ${nuovo.id}: creata (${nuovo.livello}, ${nuovo.ambito})`);
  return { ok: true, condivisione: pubblica(nuovo, gruppiDiAdesso()) };
}

/** Cambia i campi dati (gli altri restano): il nome, il livello, cosa si condivide, i giorni, il nome del calendario. */
export function aggiornaCondivisione(id: unknown, dati: unknown): CondivisioneEsito {
  const c = riga(id);
  if (!c) return { ok: false, error: "Link non trovato." };
  if (c.revocato) return { ok: false, error: "Il link è revocato: non si può più cambiare." };
  const { nome, livello, ambito, gruppo, giorni_indietro, giorni_avanti, nome_calendario } = c;
  const campi = leggiCampi(dati, { nome, livello, ambito, gruppo, giorni_indietro, giorni_avanti, nome_calendario });
  if ("errore" in campi) return { ok: false, error: campi.errore };
  const aggiornato = Math.max(adesso(), c.aggiornato + 1);
  db()
    .prepare("UPDATE condivisioni SET nome = @nome, livello = @livello, ambito = @ambito, gruppo = @gruppo, giorni_indietro = @giorni_indietro, giorni_avanti = @giorni_avanti, nome_calendario = @nome_calendario, aggiornato = @aggiornato WHERE id = @id")
    .run({ ...campi, aggiornato, id: c.id });
  console.info(`condivisione ${c.id}: cambiata`);
  return { ok: true, condivisione: pubblica({ ...c, ...campi, aggiornato }, gruppiDiAdesso()) };
}

/** Revoca: l'indirizzo smette di valere subito e il suo testo segreto si cancella. La riga resta, per vedere chi era e quando l'ha letto per ultimo. */
export function revocaCondivisione(id: unknown): CondivisioneEsito {
  const c = riga(id);
  if (!c) return { ok: false, error: "Link non trovato." };
  if (c.revocato) return { ok: true, condivisione: pubblica(c, gruppiDiAdesso()) };
  const ora = adesso();
  const aggiornato = Math.max(ora, c.aggiornato + 1);
  db().prepare("UPDATE condivisioni SET token = NULL, revocato = ?, aggiornato = ? WHERE id = ?").run(ora, aggiornato, c.id);
  console.info(`condivisione ${c.id}: revocata`);
  return { ok: true, condivisione: pubblica({ ...c, token: null, revocato: ora, aggiornato }, gruppiDiAdesso()) };
}

/** Toglie il link dall'elenco, qualunque sia il suo stato: uno che vale ancora smette di valere. Un id che non c'è non cambia niente. */
export function eliminaCondivisione(id: unknown): CondivisioneEsito {
  const c = riga(id);
  if (c) {
    db().prepare("DELETE FROM condivisioni WHERE id = ?").run(c.id);
    accessiInMemoria.delete(c.id);
    console.info(`condivisione ${c.id}: eliminata`);
  }
  return { ok: true };
}

/** Cambia l'indirizzo segreto: il vecchio smette di valere subito, il nuovo si manda a chi deve leggerlo. */
export function ruotaCondivisione(id: unknown): CondivisioneEsito {
  const c = riga(id);
  if (!c) return { ok: false, error: "Link non trovato." };
  if (c.revocato) return { ok: false, error: "Il link è revocato: se ne crea uno nuovo." };
  const token = nuovoToken();
  const aggiornato = Math.max(adesso(), c.aggiornato + 1);
  db().prepare("UPDATE condivisioni SET token = ?, aggiornato = ? WHERE id = ?").run(token, aggiornato, c.id);
  console.info(`condivisione ${c.id}: indirizzo cambiato`);
  return { ok: true, condivisione: pubblica({ ...c, token, aggiornato }, gruppiDiAdesso()) };
}
