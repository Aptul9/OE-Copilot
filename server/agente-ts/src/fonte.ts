// Una casella di Outlook (o di Google) sul telefono Android virtuale, come la leggeva l'agente Python (tolto dal repository): una copia di calendar.db
// con i suoi file di appoggio, la scelta dei calendari, gli invitati, le serie espanse, i doppioni tolti, lo stato di ogni lettura.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { espandiRigheAndroid, type OccorrenzaAndroid, type RigaAndroid } from "./android.ts";
import { pulisciLink } from "./html.ts";
import { MEETING_URL } from "./ics.ts";
import { confrontaPy, pyBool, pyStrip, pyTipo } from "./pyfmt.ts";
import { ErroreAltro } from "./tempo.ts";

export const DB = "calendar.db";
const TIPI_ACCOUNT = ["com.microsoft.office.outlook", "com.google"];
const ANNULLATO = 2; // Events.STATUS_CANCELED
const ORGANIZZATORE = 2; // Attendees.RELATIONSHIP_ORGANIZER
const GRAVITA = new Map<unknown, number>([
  ["ok", 0],
  ["avviso", 1],
  ["errore", 2],
]);

export type RigaDb = RigaAndroid & { calendar_id: number; organizer: string | null };
export type Persona = { invitati: string[]; organizzatore: string | null };
export interface Lettura {
  cal: Map<number, string>;
  eventi: RigaDb[];
  persone: Map<number, Persona>;
}
export type Stato = { stato: string; msg: unknown };
export type Evento = Record<string, unknown>;

/** Il contesto dell'agente che serve a una casella: dove sta lo stato degli accessi, che ore sono, in che fuso, dove scrivere i log. */
export interface Contesto {
  accessi: string;
  zona: string;
  adesso: () => number;
  registro: (livello: "INFO" | "WARNING" | "ERROR", testo: string) => void;
}

/** Un errore di SQLite (sqlite3.DatabaseError in Python): l'unico che una lettura ritenta. */
const èErroreSqlite = (e: unknown) => e instanceof Error && (e as { code?: string }).code === "ERR_SQLITE_ERROR";

const pausa = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** str.lower() di un valore che per Python deve essere una stringa. */
function minuscolo(v: unknown): string {
  if (typeof v !== "string") throw new ErroreAltro(`'${pyTipo(v)}' object has no attribute 'lower'`);
  return v.toLowerCase();
}

/** Un file di testo UTF-8 che si legge tutto o niente (come open() con encoding="utf-8": un byte non valido è un errore). */
export function leggiUtf8(percorso: string): string {
  return new TextDecoder("utf-8", { fatal: true }).decode(fs.readFileSync(percorso));
}

/** Python: {email: {"stato", "msg"}} scritto dal rilevatore degli accessi, o {} se manca o non si legge. */
function leggiAccessi(percorso: string): unknown {
  try {
    const dati = JSON.parse(leggiUtf8(percorso)) as unknown;
    if (dati === null || typeof dati !== "object" || Array.isArray(dati)) return {};
    const caselle = (dati as Record<string, unknown>).caselle;
    return pyBool(caselle) ? caselle : {};
  } catch {
    return {};
  }
}

/** dict.get() di un valore JSON: None se la chiave non c'è; per un valore che non è un dizionario Python solleva AttributeError. */
function prendi(valore: unknown, chiave: string): unknown {
  if (valore === null || typeof valore !== "object" || Array.isArray(valore)) throw new ErroreAltro(`'${pyTipo(valore)}' object has no attribute 'get'`);
  return Object.hasOwn(valore, chiave) ? (valore as Record<string, unknown>)[chiave] : null;
}

/** Il messaggio di un errore di sistema come lo scrive Python: "[Errno 13] Permission denied: '/percorso'". */
function messaggioSistema(e: unknown): string {
  const x = e as { code?: string; errno?: number; path?: string; dest?: string; message?: string };
  const noti: Record<string, [number, string]> = {
    ENOENT: [2, "No such file or directory"],
    EIO: [5, "Input/output error"],
    EACCES: [13, "Permission denied"],
    EEXIST: [17, "File exists"],
    ENOTDIR: [20, "Not a directory"],
    EISDIR: [21, "Is a directory"],
    EMFILE: [24, "Too many open files"],
    ENOSPC: [28, "No space left on device"],
    EROFS: [30, "Read-only file system"],
    EPERM: [1, "Operation not permitted"],
  };
  const voce = x.code ? noti[x.code] : undefined;
  if (!voce) return x.message ?? String(e);
  const file = x.code === "ENOSPC" || x.code === "EROFS" ? (x.dest ?? x.path) : x.path;
  return `[Errno ${voce[0]}] ${voce[1]}${file ? `: ${pyRepr1(file)}` : ""}`;
}
const pyRepr1 = (s: string) => (s.includes("'") && !s.includes('"') ? `"${s}"` : `'${s.replaceAll("\\", "\\\\").replaceAll("'", "\\'")}'`);

/** Il testo di un'eccezione come lo darebbe str(e) in Python. */
export function testoErrore(e: unknown): string {
  if (e instanceof Error && (e as { syscall?: string }).syscall) return messaggioSistema(e);
  return e instanceof Error ? e.message : String(e);
}

/** Copia il database con i suoi file di appoggio in una cartella nuova (Android lo tiene aperto) e lo apre, in sola lettura per le nostre richieste. */
function copiaEApri(cartella: string): { tmp: string; db: DatabaseSync } {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "android_"));
  try {
    for (const suffisso of ["", "-journal", "-wal", "-shm"]) {
      const sorgente = path.join(cartella, DB + suffisso);
      if (fs.existsSync(sorgente)) fs.copyFileSync(sorgente, path.join(tmp, DB + suffisso));
    }
    return { tmp, db: new DatabaseSync(path.join(tmp, DB)) };
  } catch (e) {
    fs.rmSync(tmp, { recursive: true, force: true });
    throw e;
  }
}

const SQL_CALENDARI = "select _id, account_name, account_type, calendar_displayName, isPrimary, deleted, sync_events from Calendars";
const SQL_EVENTI =
  "select _id, _sync_id, calendar_id, title, eventLocation, description, eventStatus, dtstart, dtend, eventTimezone, eventEndTimezone, duration, allDay, rrule, rdate, " +
  "exrule, exdate, original_id, original_sync_id, originalInstanceTime, originalAllDay, organizer from Events where deleted = 0 and calendar_id in (%s)";
const SQL_INVITATI =
  "select event_id, attendeeName, attendeeEmail, attendeeRelationship from Attendees where event_id in (select _id from Events where calendar_id in (%s))";

/** _estrai: i calendari della casella, i loro eventi e chi partecipa. null se la casella non è sul telefono. `impostaTipo` riceve il tipo di ogni calendario della casella che si incontra. */
export function estrai(db: DatabaseSync, account: string, calendari: string[], impostaTipo: (tipo: string) => void = () => {}): Lettura | null {
  const cal = new Map<number, string>();
  const sincronizzati = new Map<number, string>();
  for (const r of db.prepare(SQL_CALENDARI).all() as Record<string, unknown>[]) {
    const id = r._id as number;
    const conto = r.account_name;
    const tp = r.account_type;
    const nome = r.calendar_displayName;
    if (minuscolo(pyBool(conto) ? conto : "") !== account || !TIPI_ACCOUNT.some((t) => String(pyBool(tp) ? tp : "").startsWith(t))) continue;
    if (pyBool(r.deleted)) continue;
    impostaTipo(String(tp).startsWith("com.google") ? "caldav" : "outlook");
    const etichetta = pyBool(nome) ? String(nome) : "Calendario";
    if (pyBool(r.sync_events)) sincronizzati.set(id, etichetta);
    const scelto = calendari.length ? calendari.includes(minuscolo(pyBool(nome) ? nome : "")) : pyBool(r.isPrimary);
    if (scelto) cal.set(id, etichetta);
  }
  // Senza un calendario principale e senza scelta esplicita si prendono quelli che si sincronizzano.
  const scelti = !cal.size && !calendari.length ? sincronizzati : cal;
  if (!scelti.size) return null;
  const segnaposti = Array.from(scelti.keys(), () => "?").join(",");
  const ids = [...scelti.keys()];
  const eventi = db.prepare(SQL_EVENTI.replace("%s", segnaposti)).all(...ids) as unknown as RigaDb[];
  const persone = new Map<number, Persona>();
  for (const r of db.prepare(SQL_INVITATI.replaceAll("%s", segnaposti)).all(...ids) as Record<string, unknown>[]) {
    const evento = r.event_id as number;
    let p = persone.get(evento);
    if (!p) persone.set(evento, (p = { invitati: [], organizzatore: null }));
    const email = r.attendeeEmail;
    if (pyBool(email)) p.invitati.push(minuscolo(email));
    if (r.attendeeRelationship === ORGANIZZATORE) p.organizzatore = (pyBool(r.attendeeName) ? r.attendeeName : email) as string | null;
  }
  return { cal: scelti, eventi, persone };
}

type Dettagli = { calendario: string; email: string; organizzatore: string; invitati: string[]; serie?: boolean };

/** Per ogni UID di iCalendar i campi che iCalendar non ha: il calendario, l'organizzatore, gli invitati, se viene da una serie. */
function dettagliDi(lettura: Lettura): Map<string, Dettagli> {
  const { cal, eventi, persone } = lettura;
  const dettagli = new Map<string, Dettagli>();
  const perSync = new Map<string, number>();
  for (const r of eventi) if (r._sync_id) perSync.set(r._sync_id, r._id);
  for (const r of eventi) {
    const serie = r.original_id || (r.original_sync_id ? perSync.get(r.original_sync_id) : undefined);
    if (serie && r.eventStatus === ANNULLATO && r.originalInstanceTime !== null) continue;
    if (r.eventStatus === ANNULLATO || r.dtstart === null) continue;
    const uid = `a${serie || r._id}`;
    const p = persone.get(r._id);
    if (!dettagli.has(uid))
      dettagli.set(uid, {
        calendario: cal.get(r.calendar_id) ?? "",
        email: (r.organizer ?? "").toLowerCase(),
        organizzatore: p?.organizzatore || r.organizer || "",
        invitati: [...new Set(p?.invitati ?? [])].sort(confrontaPy),
      });
    if (serie || r.rrule || r.rdate) dettagli.get(uid)!.serie = true;
  }
  return dettagli;
}

/** Il confronto fra tuple di Python: la prima posizione che differisce decide. */
function pesoMaggiore(a: boolean[], b: boolean[]): boolean {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return Number(a[i]) > Number(b[i]);
  return false;
}

export interface InfoCasella {
  id: string;
  nome: string;
  account: string;
  tipo: string | null;
}

/**
 * Gli eventi come li restituisce CalendarioAndroid.eventi: un'occorrenza per titolo, inizio e fine (la più completa), in ordine di inizio
 * e titolo. Outlook scrive una serie due volte, la serie con la sua regola e ogni occorrenza come appuntamento a sé: espandendo la serie le
 * occorrenze raddoppiano. Si tiene prima quella scritta da Outlook per l'occorrenza (ha il luogo e il link di quel giorno), poi quella con
 * link, luogo e organizzatore.
 */
export function eventiDaLettura(lettura: Lettura, info: InfoCasella, inizio: Temporal.PlainDateTime | string, fine: Temporal.PlainDateTime | string, zona: string, grezzi?: OccorrenzaAndroid[]): Evento[] {
  const occorrenze = grezzi ?? espandiRigheAndroid(lettura.eventi, inizio, fine, { fuso: zona });
  const dettagli = dettagliDi(lettura);
  const candidati = new Map<string, { peso: boolean[]; e: Evento }>();
  for (const o of occorrenze) {
    const d = dettagli.get(o.uid);
    const link = MEETING_URL.exec(o.luogo) ?? MEETING_URL.exec(o.descrizione);
    const e: Evento = {
      id: `${info.id}|${o.uid}|${o.inizio}`,
      account: info.account,
      alias: info.nome,
      calendar: `${info.account} · ${d?.calendario ?? ""}`,
      title: o.titolo || "(senza titolo)",
      start: o.inizio,
      end: o.fine,
      allDay: o.tuttoGiorno,
      location: o.luogo,
      organizer: d?.organizzatore ?? "",
      organizer_email: d?.email ?? "",
      attendees: d?.invitati ?? [],
      online_url: link ? pulisciLink(link[0]) : null,
      web_link: null,
      fonte: "android",
      tipo: info.tipo,
    };
    const peso = [!d?.serie, Boolean(e.online_url), Boolean(o.luogo), Boolean(e.organizer_email)];
    const chiave = JSON.stringify([e.title, e.start, e.end]);
    const attuale = candidati.get(chiave);
    if (!attuale || pesoMaggiore(peso, attuale.peso)) candidati.set(chiave, { peso, e });
  }
  return [...candidati.values()]
    .map((c) => c.e)
    .sort((a, b) => confrontaPy(a.start as string, b.start as string) || confrontaPy(a.title as string, b.title as string));
}

export class CalendarioAndroid {
  readonly id: string;
  readonly nome: string;
  readonly account: string;
  readonly cartella: string;
  readonly calendari: string[];
  private readonly ctx: Contesto;
  stato: Stato = { stato: "avviso", msg: "Non ancora letto" };
  // Chi tiene aggiornata la casella sul telefono: "outlook" o "caldav" (servizi Google: il nome resta "caldav" perché l'app sul PC lo conosce)
  tipo: string | null = null;
  conteggio: number | null = null; // appuntamenti presenti sul telefono

  constructor(id: string, nome: string, account: string, cartella: string, calendari: string[] | null | undefined, ctx: Contesto) {
    this.id = id;
    this.nome = nome;
    this.account = pyStrip(account).toLowerCase();
    this.cartella = cartella;
    this.calendari = (calendari ?? []).map((c) => pyStrip(c).toLowerCase());
    this.ctx = ctx;
  }

  /** Legge il telefono: una copia di calendar.db presa a metà scrittura si ritenta una volta, dopo un secondo. */
  private async leggi(): Promise<Lettura | null> {
    if (!fs.existsSync(path.join(this.cartella, DB))) throw new ErroreAltro("calendario del telefono non trovato");
    let ultimo: unknown = null;
    for (let tentativo = 0; tentativo < 2; tentativo++) {
      const { tmp, db } = copiaEApri(this.cartella);
      try {
        // la copia è nostra: un journal "caldo" (copia presa a metà scrittura) si riavvolge, come con sqlite3.connect di Python; query_only
        // impedisce che una nostra istruzione scriva
        db.exec("PRAGMA query_only = ON");
        return estrai(db, this.account, this.calendari, (tipo) => (this.tipo = tipo));
      } catch (e) {
        if (!èErroreSqlite(e)) throw e;
        ultimo = e;
      } finally {
        db.close();
        fs.rmSync(tmp, { recursive: true, force: true });
      }
      if (tentativo === 0) await pausa(1000);
    }
    throw ultimo;
  }

  /** Legge il telefono e aggiorna lo stato. -> (dati grezzi o null, errori) */
  private async leggiESegna(): Promise<[Lettura | null, string[]]> {
    let grezzi: Lettura | null;
    try {
      grezzi = await this.leggi();
    } catch (e) {
      const testo = testoErrore(e);
      this.ctx.registro("WARNING", `${this.nome}: lettura non riuscita (${testo})`);
      this.stato = { stato: "errore", msg: `Calendario del telefono non leggibile (${testo})` };
      return [null, [`${this.nome}: ${testo}`]];
    }
    if (grezzi === null) {
      const msg = `La casella ${this.account} non e' sul telefono (login da rifare?)`;
      this.stato = { stato: "errore", msg };
      return [null, [`${this.nome}: ${msg}`]];
    }
    this.conteggio = grezzi.eventi.length;
    // Si dice solo cosa si è verificato davvero: la lettura è riuscita. L'ora di scrittura di calendar.db non dice niente di Outlook (la
    // muove ogni 2 minuti la sincronizzazione Google): la freschezza di ogni casella la misura il rilevatore del supervisore (supervisore/src/rilevatore/).
    const ora = new Date(this.ctx.adesso()).toLocaleTimeString("en-GB", { timeZone: this.ctx.zona, hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
    this.stato = { stato: "ok", msg: `Letto dal telefono alle ${ora}` };
    this.applicaStatoAccesso();
    return [grezzi, []];
  }

  /**
   * Se l'accesso alla casella sul telefono non è a posto, lo dice il pallino. La lettura riesce anche con il login scaduto (sono dati già sul
   * telefono): il verdetto (ok, avviso, errore e un messaggio) arriva dal rilevatore. Si applica solo se peggiora lo stato della lettura.
   */
  private applicaStatoAccesso(): void {
    const v = prendi(leggiAccessi(this.ctx.accessi), this.account);
    if (!pyBool(v)) return;
    const s = prendi(v, "stato");
    const stato = pyBool(s) ? s : "ok";
    if (typeof stato === "object") throw new ErroreAltro(`unhashable type: '${pyTipo(stato)}'`);
    if ((GRAVITA.get(stato) ?? 0) > (GRAVITA.get(this.stato.stato) ?? 0)) {
      const msg = prendi(v, "msg");
      this.stato = { stato: stato as string, msg: pyBool(msg) ? msg : this.stato.msg };
    }
  }

  /** La casella come la vede l'app: chi è, com'è andata l'ultima lettura. */
  descrizione(): Record<string, unknown> {
    return { id: this.id, account: this.account, nome: this.nome, tipo: this.tipo, eventi: this.conteggio, ...this.stato };
  }

  /** Come descrizione(), ma dopo una lettura fresca (per l'elenco dei calendari). */
  async riepilogo(): Promise<Record<string, unknown>> {
    await this.leggiESegna();
    return this.descrizione();
  }

  /** Eventi fra due date locali senza fuso. -> (eventi, errori) */
  async eventi(inizio: Temporal.PlainDateTime, fine: Temporal.PlainDateTime): Promise<[Evento[], string[]]> {
    const [grezzi, errori] = await this.leggiESegna();
    if (grezzi === null) return [[], errori];
    return [eventiDaLettura(grezzi, { id: this.id, nome: this.nome, account: this.account, tipo: this.tipo }, inizio, fine, this.ctx.zona), []];
  }
}
