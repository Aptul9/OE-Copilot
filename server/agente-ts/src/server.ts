// L'agente di Oe Copilot sulla VM: legge i calendari del telefono e li serve al gateway. Stessa API di server/agent/agent.py, con le stesse
// risposte, gli stessi codici e gli stessi testi di errore (tutte con "Authorization: Bearer <API_KEY>"):
//   GET /api/eventi?inizio=ISO&fine=ISO  eventi del periodo + stato di ogni casella
//   GET /api/calendari                   le caselle, con stato e numero di appuntamenti
//   GET /api/salute                      stato delle caselle, senza rileggere
//   POST /api/ics                        {"testo": ICS[, "inizio": ISO, "fine": ISO]}: il nome del calendario, o i suoi eventi nel periodo

import crypto from "node:crypto";
import http from "node:http";
import { CalendarioAndroid, leggiUtf8, testoErrore, type Contesto, type Evento } from "./fonte.ts";
import { espandiIcs, validaIcs } from "./ics.ts";
import { isoNaive, oraLocaleDa } from "./iso.ts";
import { jsonPy, pyBool, pyRepr, pyReprStr, pyStrip, pyTipo } from "./pyfmt.ts";
import { analizzaTarget, parseQs } from "./query.ts";
import { ErroreAltro, ErroreSenzaRisposta, ErroreTipo, ErroreValore } from "./tempo.ts";

export type Registro = (livello: "INFO" | "WARNING" | "ERROR", testo: string) => void;

export interface OpzioniAgente {
  chiave: string;
  fonti: CalendarioAndroid[];
  /** Il segno di vita scritto a ogni giro dal servizio che aggiorna il telefono (battito.json). */
  battito: string;
  /** Il fuso locale: TZ, o Europe/Rome. */
  zona: string;
  /** Il testo di un calendario ICS da espandere (POST /api/ics): oltre questi byte la richiesta è rifiutata. */
  maxCorpo?: number;
  /** Letture ravvicinate dello stesso periodo: una sola (il telefono si aggiorna ogni 30 s). */
  cacheS?: number;
  adesso?: () => number;
  registro?: Registro;
}

const MAX_CORPO = 8_000_000;
const CACHE_S = 15;
const ERRORI_HTTP: Record<number, string> = { 200: "OK", 400: "Bad Request", 401: "Unauthorized", 404: "Not Found", 413: "Request Entity Too Large" };

/** Il registro come lo scrive logging.basicConfig di Python: ora locale, livello, nome, messaggio, su stderr. */
export function registroSuStderr(zona: string, adesso: () => number = Date.now): Registro {
  return (livello, testo) => {
    const t = Temporal.Instant.fromEpochMilliseconds(adesso()).toZonedDateTimeISO(zona).toPlainDateTime();
    const ms = String(adesso() % 1000).padStart(3, "0");
    process.stderr.write(`${t.toString({ smallestUnit: "second" }).replace("T", " ")},${ms} ${livello} agente: ${testo}\n`);
  };
}

export function contestoDi(o: { accessi: string; zona: string; adesso?: () => number; registro?: Registro }): Contesto {
  const adesso = o.adesso ?? Date.now;
  return { accessi: o.accessi, zona: o.zona, adesso, registro: o.registro ?? registroSuStderr(o.zona, adesso) };
}

/** _carica_fonti: fonti.json elenca le caselle da leggere dal calendario del telefono (tipo "android"). Un file che manca o non ha la forma giusta ferma l'avvio. */
export function caricaFonti(percorso: string, cartellaDb: string, ctx: Contesto): CalendarioAndroid[] {
  const voci = JSON.parse(leggiUtf8(percorso)) as unknown;
  const elenco = Array.isArray(voci) ? voci : voci !== null && typeof voci === "object" ? Object.keys(voci) : null;
  if (elenco === null) throw new ErroreTipo(`'${pyTipo(voci)}' object is not iterable`);
  const fonti: CalendarioAndroid[] = [];
  for (const v of elenco) {
    if (v === null || typeof v !== "object" || Array.isArray(v)) throw new ErroreAltro(`'${pyTipo(v)}' object has no attribute 'get'`);
    const voce = v as Record<string, unknown>;
    if (voce.tipo === "android") {
      for (const k of ["id", "nome", "account"]) if (!Object.hasOwn(voce, k)) throw new ErroreAltro(`KeyError: ${pyReprStr(k)}`);
      fonti.push(new CalendarioAndroid(voce.id as string, voce.nome as string, voce.account as string, cartellaDb, (voce.calendari ?? null) as string[] | null, ctx));
    } else ctx.registro("ERROR", `fonte sconosciuta, ignorata: ${pyRepr(v)}`);
  }
  return fonti;
}

/** float() di un valore JSON: numeri, booleani e stringhe che Python converte; il resto è un TypeError o un ValueError. */
export function pyFloat(v: unknown): number {
  if (typeof v === "number") return v;
  if (typeof v === "boolean") return v ? 1 : 0;
  if (typeof v !== "string") throw new ErroreTipo("float() argument must be a string or a real number");
  // Python legge anche le cifre decimali di ogni alfabeto (٣ è 3): ogni cifra torna a quella ASCII del suo gruppo di dieci
  const cifra = (c: string) => {
    let inizio = c.codePointAt(0)!;
    while (/^\p{Nd}$/u.test(String.fromCodePoint(inizio - 1))) inizio--;
    return String((c.codePointAt(0)! - inizio) % 10);
  };
  const t = pyStrip(v.replace(/\p{Nd}/gu, cifra));
  if (/^[+-]?(inf|infinity)$/i.test(t)) return t.startsWith("-") ? -Infinity : Infinity;
  if (/^[+-]?nan$/i.test(t)) return NaN;
  if (/^[+-]?(\d+(_\d+)*)?(\.(\d+(_\d+)*)?)?([eE][+-]?\d+(_\d+)*)?$/.test(t) && /\d/.test(t.split(/[eE]/)[0]!)) return Number(t.replaceAll("_", ""));
  throw new ErroreValore(`could not convert string to float: ${pyReprStr(v)}`);
}

export class Agente {
  private readonly o: Required<Omit<OpzioniAgente, "registro">> & { registro: Registro };
  private readonly cache = new Map<string, { t: number; risposta: Record<string, unknown> }>();

  constructor(o: OpzioniAgente) {
    const adesso = o.adesso ?? Date.now;
    this.o = { ...o, maxCorpo: o.maxCorpo ?? MAX_CORPO, cacheS: o.cacheS ?? CACHE_S, adesso, registro: o.registro ?? registroSuStderr(o.zona, adesso) };
  }

  /** Il segno di vita del servizio che aggiorna il telefono, o null se manca. "eta_s" dice da quanti secondi non passa: se cresce, l'app lo dice. */
  servizio(): Record<string, unknown> | null {
    try {
      const b = JSON.parse(leggiUtf8(this.o.battito)) as unknown;
      if (b === null || typeof b !== "object" || Array.isArray(b) || !Object.hasOwn(b, "quando")) return null;
      const dati = b as Record<string, unknown>;
      const differenza = this.o.adesso() / 1000 - pyFloat(dati.quando);
      if (Number.isNaN(differenza)) return null;
      if (!Number.isFinite(differenza)) throw new ErroreSenzaRisposta("OverflowError: cannot convert float infinity to integer");
      return { eta_s: Math.max(0, Math.trunc(differenza)), pausa: pyBool(dati.pausa), telefono: Object.hasOwn(dati, "telefono") ? pyBool(dati.telefono) : true };
    } catch (e) {
      if (e instanceof ErroreSenzaRisposta) throw e;
      return null;
    }
  }

  private fontiStato(): Map<string, unknown> {
    const m = new Map<string, unknown>();
    for (const f of this.o.fonti) m.set(f.nome, f.stato);
    return m;
  }

  async raccogli(inizio: Temporal.PlainDateTime, fine: Temporal.PlainDateTime): Promise<Record<string, unknown>> {
    const chiave = `${isoNaive(inizio)}\u0000${isoNaive(fine)}`;
    const voce = this.cache.get(chiave);
    if (voce && this.o.adesso() / 1000 - voce.t < this.o.cacheS) return voce.risposta;
    const eventi: Evento[] = [];
    const errori: string[] = [];
    for (const fonte of this.o.fonti) {
      let trovati: Evento[];
      let err: string[];
      try {
        [trovati, err] = await fonte.eventi(inizio, fine);
      } catch (e) {
        if (e instanceof ErroreSenzaRisposta) throw e;
        // Una fonte rotta non deve portarsi via le altre.
        this.o.registro("ERROR", `fonte ${fonte.nome} fallita: ${testoErrore(e)}`);
        trovati = [];
        err = [`${fonte.nome}: ${testoErrore(e)}`];
      }
      eventi.push(...trovati);
      errori.push(...err);
    }
    const risposta = {
      eventi,
      errori,
      fonti: this.fontiStato(),
      caselle: this.o.fonti.map((f) => f.descrizione()),
      servizio: this.servizio(),
      generato: Temporal.Instant.fromEpochMilliseconds(this.o.adesso()).toZonedDateTimeISO(this.o.zona).toPlainDateTime().toString({ smallestUnit: "second" }),
    };
    this.cache.set(chiave, { t: this.o.adesso() / 1000, risposta });
    // La cache non deve crescere all'infinito: si tengono le 50 più recenti.
    if (this.cache.size > 50) for (const [k] of [...this.cache.entries()].sort((a, b) => a[1].t - b[1].t).slice(0, -50)) this.cache.delete(k);
    return risposta;
  }

  /** hmac.compare_digest sulle intestazioni: gli stessi byte, in tempo costante. Con caratteri non ASCII Python solleva TypeError e il gestore muore. */
  private autorizzato(req: http.IncomingMessage): boolean {
    if (!this.o.chiave) return false;
    const atteso = "Bearer " + this.o.chiave;
    const dato = req.headers.authorization ?? "";
    if (/[^\x00-\x7f]/.test(dato) || /[^\x00-\x7f]/.test(atteso)) throw new ErroreSenzaRisposta("TypeError: comparing strings with non-ASCII characters is not supported");
    const a = Buffer.from(dato, "latin1");
    const b = Buffer.from(atteso, "latin1");
    return a.length === b.length && crypto.timingSafeEqual(a, b);
  }

  private invia(req: http.IncomingMessage, res: http.ServerResponse, codice: number, dati: unknown, scartaCorpo = false): void {
    const corpo = Buffer.from(jsonPy(dati), "utf-8");
    res.statusCode = codice;
    res.statusMessage = ERRORI_HTTP[codice] ?? "";
    res.setHeader("Content-Type", "application/json; charset=utf-8");
    res.setHeader("Content-Length", String(corpo.length));
    // Python risponde in HTTP/1.0: una richiesta, una connessione. Se il corpo non è stato letto Node lo scarta da sé mentre il cliente
    // lo manda e poi chiude: chiudere subito azzererebbe la connessione e la risposta potrebbe non arrivare
    if (scartaCorpo) {
      // il cliente ha chiesto di chiudere, ma chiudere ora azzererebbe la connessione e la risposta potrebbe non arrivargli: si tiene aperta
      // finché finisce di mandare (Node scarta il corpo) e la chiude il cliente o il timeout
      res.setHeader("Connection", "keep-alive");
      req.resume();
    } else res.setHeader("Connection", "close");
    this.registraRichiesta(req, codice);
    res.end(corpo);
  }

  private registraRichiesta(req: http.IncomingMessage, codice: number): void {
    this.o.registro("INFO", `${req.socket.remoteAddress ?? "-"} "${req.method} ${req.url} HTTP/${req.httpVersion}" ${codice} -`);
  }

  /** send_error di http.server: una pagina HTML con il codice (qui solo 501, per i metodi che l'agente non ha). */
  private paginaErrore(req: http.IncomingMessage, res: http.ServerResponse, codice: number, messaggio: string, spiegazione: string): void {
    const corpo = Buffer.from(
      `<!DOCTYPE HTML>\n<html lang="en">\n    <head>\n        <meta charset="utf-8">\n        <title>Error response</title>\n    </head>\n    <body>\n        <h1>Error response</h1>\n` +
        `        <p>Error code: ${codice}</p>\n        <p>Message: ${messaggio.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;")}.</p>\n` +
        `        <p>Error code explanation: ${codice} - ${spiegazione}.</p>\n    </body>\n</html>\n`,
      "utf-8",
    );
    res.statusCode = codice;
    res.statusMessage = messaggio.replace(/[^\x20-\x7e]/g, "?");
    res.setHeader("Connection", "close");
    res.setHeader("Content-Type", "text/html;charset=utf-8");
    res.setHeader("Content-Length", String(corpo.length));
    this.registraRichiesta(req, codice);
    res.end(req.method === "HEAD" ? undefined : corpo);
  }

  private async post(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    if (!this.autorizzato(req)) return this.invia(req, res, 401, { errore: "non autorizzato" });
    if (analizzaTarget(req.url ?? "").percorso !== "/api/ics") return this.invia(req, res, 404, { errore: "sconosciuto" });
    const dichiarata = req.headers["content-length"] || "0";
    const lunghezza = /^\s*[+-]?\d+\s*$/.test(dichiarata) ? Number(dichiarata.trim()) : -1;
    if (!(lunghezza > 0 && lunghezza <= this.o.maxCorpo)) return this.invia(req, res, lunghezza > this.o.maxCorpo ? 413 : 400, { errore: "lunghezza della richiesta non valida" }, true);
    const corpo = await leggiCorpo(req);
    if (corpo === null) return;
    const formaSbagliata = () => this.invia(req, res, 400, { errore: 'serve {"testo": ..., "inizio": ..., "fine": ...}' });
    let testo: string;
    let intervallo: [Temporal.PlainDateTime, Temporal.PlainDateTime] | null;
    try {
      const richiesta = leggiJson(corpo);
      if (richiesta === null || typeof richiesta !== "object" || Array.isArray(richiesta)) throw new ErroreTipo("not a dict");
      const r = richiesta as Record<string, unknown>;
      if (!Object.hasOwn(r, "testo")) throw new ErroreAltro("KeyError: 'testo'");
      const t = r.testo;
      const da = Object.hasOwn(r, "inizio") ? r.inizio : null;
      const a = Object.hasOwn(r, "fine") ? r.fine : null;
      if (typeof t !== "string" || ((da === null || a === null) && !(da === null && a === null))) throw new ErroreValore("periodo");
      const locale = (x: unknown) => {
        if (typeof x !== "string") throw new ErroreAltro(`'${pyTipo(x)}' object has no attribute 'replace'`);
        return oraLocaleDa(x, this.o.zona);
      };
      testo = t;
      intervallo = da === null ? null : [locale(da), locale(a)];
    } catch (e) {
      if (e instanceof ErroreSenzaRisposta) throw e;
      return formaSbagliata();
    }
    try {
      if (intervallo === null) return this.invia(req, res, 200, { ok: true, nome: validaIcs(testo) });
      return this.invia(req, res, 200, { ok: true, eventi: espandiIcs(testo, isoNaive(intervallo[0]), isoNaive(intervallo[1]), { fuso: this.o.zona }) });
    } catch (e) {
      // un testo che non è un calendario non deve fermare l'agente: lo si dice a chi l'ha mandato
      if (e instanceof ErroreSenzaRisposta) throw e;
      return this.invia(req, res, 200, { ok: false, errore: testoErrore(e), valore: e instanceof ErroreValore || e instanceof ErroreTipo });
    }
  }

  private async get(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    if (!this.autorizzato(req)) return this.invia(req, res, 401, { errore: "non autorizzato" });
    const { percorso, query } = analizzaTarget(req.url ?? "");
    const q = parseQs(query);
    if (percorso === "/api/salute") return this.invia(req, res, 200, { fonti: this.fontiStato(), servizio: this.servizio() });
    if (percorso === "/api/calendari") {
      const caselle: Record<string, unknown>[] = [];
      for (const f of this.o.fonti) {
        try {
          caselle.push(await f.riepilogo());
        } catch (e) {
          if (e instanceof ErroreSenzaRisposta) throw e;
          this.o.registro("ERROR", `fonte ${f.nome}: riepilogo fallito: ${testoErrore(e)}`);
          caselle.push({ id: f.id, account: f.account, nome: f.nome, tipo: f.tipo, eventi: null, stato: "errore", msg: testoErrore(e) });
        }
      }
      return this.invia(req, res, 200, { caselle, servizio: this.servizio() });
    }
    if (percorso === "/api/eventi") {
      let inizio: Temporal.PlainDateTime;
      let fine: Temporal.PlainDateTime;
      try {
        const primo = q.get("inizio");
        if (!primo) throw new ErroreAltro("'inizio'");
        inizio = oraLocaleDa(primo[0]!, this.o.zona);
        const secondo = q.get("fine");
        if (!secondo) throw new ErroreAltro("'fine'");
        fine = oraLocaleDa(secondo[0]!, this.o.zona);
      } catch (e) {
        if (e instanceof ErroreSenzaRisposta) throw e;
        if (e instanceof ErroreAltro || e instanceof ErroreValore) return this.invia(req, res, 400, { errore: `periodo non valido (${e.message})` });
        throw e;
      }
      return this.invia(req, res, 200, await this.raccogli(inizio, fine));
    }
    return this.invia(req, res, 404, { errore: "sconosciuto" });
  }

  /** Un gestore per http.createServer. Un'eccezione che Python non prende chiude la connessione senza risposta, e qui lo stesso. */
  gestore(): http.RequestListener {
    return (req, res) => {
      const sotto =
        req.method === "GET" ? this.get(req, res) : req.method === "POST" ? this.post(req, res) : Promise.resolve(this.paginaErrore(req, res, 501, `Unsupported method (${pyReprStr(req.method ?? "")})`, "Server does not support this operation"));
      sotto.catch((e: unknown) => {
        if (e instanceof ErroreSenzaRisposta) this.o.registro("ERROR", `gestore fermato: ${e.message}`);
        else this.o.registro("ERROR", `gestore fermato da un errore inatteso: ${e instanceof Error ? (e.stack ?? e.message) : String(e)}`);
        req.socket.destroy();
      });
    };
  }
}

/** Il corpo di una richiesta POST, tutto; null se il cliente se ne va prima della fine. */
function leggiCorpo(req: http.IncomingMessage): Promise<Buffer | null> {
  return new Promise((ok) => {
    const pezzi: Buffer[] = [];
    req.on("data", (c: Buffer) => pezzi.push(c));
    req.on("end", () => ok(Buffer.concat(pezzi)));
    req.on("error", () => ok(null));
    req.on("close", () => ok(null));
  });
}

/** json.loads accetta NaN, Infinity e -Infinity, JSON.parse no: fuori dalle stringhe diventano un oggetto (non è né nullo né una stringa, come un float in Python). */
function conEstensioniDiPython(testo: string): string {
  if (!/NaN|Infinity/.test(testo)) return testo;
  let out = "";
  let dentro = false;
  for (let i = 0; i < testo.length; i++) {
    const c = testo[i]!;
    if (dentro) {
      out += c;
      if (c === "\\") out += testo[++i] ?? "";
      else if (c === '"') dentro = false;
      continue;
    }
    if (c === '"') dentro = true;
    const m = /^-?(NaN|Infinity)/.exec(testo.slice(i, i + 10));
    if (m && !/[A-Za-z0-9_]/.test(testo[i + m[0].length] ?? "")) {
      out += '{"\\u0000nan":1}';
      i += m[0].length - 1;
      continue;
    }
    out += c;
  }
  return out;
}

/** json.loads(bytes): la codifica si riconosce dai primi byte (UTF-8 con o senza BOM, UTF-16), un byte non valido è un errore. */
function leggiJson(corpo: Buffer): unknown {
  let codifica = "utf-8";
  const b = corpo;
  if (b.length >= 4 && ((b[0] === 0xff && b[1] === 0xfe && b[2] === 0 && b[3] === 0) || (b[0] === 0 && b[1] === 0 && b[2] === 0xfe && b[3] === 0xff))) throw new ErroreValore("utf-32 non supportato");
  if (b[0] === 0xff && b[1] === 0xfe) codifica = "utf-16le";
  else if (b[0] === 0xfe && b[1] === 0xff) codifica = "utf-16be";
  else if (b.length >= 4) {
    if (!b[0]) {
      if (!b[1]) throw new ErroreValore("utf-32 non supportato");
      codifica = "utf-16be";
    } else if (!b[1]) {
      if (!b[2] && !b[3]) throw new ErroreValore("utf-32 non supportato");
      codifica = "utf-16le";
    }
  } else if (b.length === 2) {
    if (!b[0]) codifica = "utf-16be";
    else if (!b[1]) codifica = "utf-16le";
  }
  try {
    return JSON.parse(conEstensioniDiPython(new TextDecoder(codifica, { fatal: true }).decode(b)));
  } catch (e) {
    throw new ErroreValore(e instanceof Error ? e.message : String(e));
  }
}

export function creaServer(o: OpzioniAgente): { server: http.Server; agente: Agente } {
  const agente = new Agente(o);
  return { server: http.createServer(agente.gestore()), agente };
}
