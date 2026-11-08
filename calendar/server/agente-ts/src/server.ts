// L'agente di Oe Copilot sulla VM: legge i calendari del telefono e li serve al gateway. Stessa API dell'agente Python (tolto dal repository), con le stesse
// risposte, gli stessi codici e gli stessi testi di errore (tutte con "Authorization: Bearer <API_KEY>"):
//   GET /api/eventi?inizio=ISO&fine=ISO  eventi del periodo + stato di ogni casella
//   GET /api/calendari                   le caselle, con stato e numero di appuntamenti
//   GET /api/salute                      stato delle caselle, senza rileggere
//   POST /api/ics                        {"testo": ICS[, "inizio": ISO, "fine": ISO]}: il nome del calendario, o i suoi eventi nel periodo
// e, solo dell'agente TypeScript, gli accessi alle caselle (/api/auth/*), il broker dei token della posta (GET /api/token-posta) e il gateway
// di sola lettura su posta e calendari che non fa uscire i token (comunicazione.ts):
//   GET /api/communication/accounts      {"accounts": [{"upn", "status", "message"}]}
//   POST /api/communication/read         {"account", "path"[, "prefer"]}: il JSON di Outlook, o {"errore"} con il codice HTTP

import crypto from "node:crypto";
import { readCommands, saveCommands } from "./shared-commands.ts";
import { SignIns } from "./signin.ts";
import { ProxyBrowserRequired } from "./mcas-outlook.ts";
import { SessioneScaduta } from "./sessione-web.ts";
import { SharedAuthorizationRefused } from "./shared-account.ts";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { ErroreComunicazione, GatewayComunicazione } from "./comunicazione.ts";
import { CalendarioDiretto, GestoreToken } from "./diretto.ts";
import { GoogleCommunication } from "./google-communication.ts";
import { richiestaCondivisione, identitaDaToken, attivaCondivisione } from "./shared-provisioning.ts";
import { ErroreRapportoTeams, leggiRapportoTeams, rapportoValido, type RapportoTeams } from "./shared-teams.ts";
import { GoogleChatError } from "./gchat.ts";
import { GmailError } from "./gmail.ts";
import { ErroreSessioneGoogle } from "./google-web.ts";
import { CalendarioAndroid, DB, leggiUtf8, testoErrore, type Contesto, type Evento, type FonteCalendario } from "./fonte.ts";
import { espandiIcs, validaIcs } from "./ics.ts";
import { isoNaive, oraLocaleDa } from "./iso.ts";
import { jsonPy, pyBool, pyRepr, pyReprStr, pyStrip, pyTipo } from "./pyfmt.ts";
import { analizzaTarget, parseQs } from "./query.ts";
import { ErroreAltro, ErroreSenzaRisposta, ErroreTipo, ErroreValore } from "./tempo.ts";

export type Registro = (livello: "INFO" | "WARNING" | "ERROR", testo: string) => void;

export interface OpzioniAgente {
  chiave: string;
  /**
   * La chiave di TeamsRelay (TEAMS_BROKER_KEY): apre solo l'elenco degli account condivisi (leggerlo, cambiarne le capacità, mandare lo
   * stato degli account Teams) e i token delle risorse della chat. Mai i token di Outlook, la posta, i calendari o gli accessi.
   */
  chiaveTeams?: string;
  fonti: FonteCalendario[];
  /** Il segno di vita scritto a ogni giro dal servizio che aggiorna il telefono (battito.json). */
  battito: string;
  /** Il fuso locale: TZ, o Europe/Rome. */
  zona: string;
  /** Token OAuth decifrati per l'MCP della posta (token-outlook.json). */
  tokenPosta?: string;
  /** Stato degli accessi scritto dal rilevatore (accessi.json). */
  accessi?: string;
  /** Gestore dei token OAuth per la lettura diretta senza emulatore (tokens.json). */
  tokens?: GestoreToken;
  /** Credential-owned Gmail and Google Chat, independent from calendar grants. */
  google?: GoogleCommunication;
  /** Il testo di un calendario ICS da espandere (POST /api/ics): oltre questi byte la richiesta è rifiutata. */
  maxCorpo?: number;
  /** Letture ravvicinate dello stesso periodo: una sola. */
  cacheS?: number;
  adesso?: () => number;
  registro?: Registro;
}

const MAX_CORPO = 8_000_000;
/** Le risorse dei token che la chiave di TeamsRelay può chiedere: quelle della chat, mai outlook. */
const RISORSE_CHAT = new Set(["csa", "ic3", "spaces", "presence"]);
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

export function contestoDi(o: {
  accessi: string;
  zona: string;
  adesso?: () => number;
  registro?: Registro;
  tokens?: GestoreToken;
  fetchFn?: typeof fetch;
}): Contesto {
  const adesso = o.adesso ?? Date.now;
  return {
    accessi: o.accessi,
    zona: o.zona,
    adesso,
    registro: o.registro ?? registroSuStderr(o.zona, adesso),
    tokens: o.tokens,
    fetchFn: o.fetchFn,
  };
}

function nomeDaEmail(email: string): string {
  const dominio = email.split("@")[1]?.toLowerCase() ?? "";
  if (dominio === "gmail.com" || dominio === "googlemail.com") return "Personale";
  const radice = dominio.split(".")[0] || email.split("@")[0] || "Calendario";
  return radice.slice(0, 1).toUpperCase() + radice.slice(1);
}

/** _carica_fonti: fonti.json elenca le caselle da leggere (tipo "android", "outlook" o "google"). Un file che manca o non ha la forma giusta ferma l'avvio. */
export function caricaFonti(percorso: string, cartellaDb: string, ctx: Contesto): FonteCalendario[] {
  const voci = JSON.parse(leggiUtf8(percorso)) as unknown;
  const elenco = Array.isArray(voci) ? voci : voci !== null && typeof voci === "object" ? Object.keys(voci) : null;
  if (elenco === null) throw new ErroreTipo(`'${pyTipo(voci)}' object is not iterable`);
  const fonti: FonteCalendario[] = [];
  const usaDirettoPerAndroid = Boolean(ctx.tokens && (fs.existsSync(ctx.tokens.percorso) || !fs.existsSync(path.join(cartellaDb, DB))));
  for (const v of elenco) {
    if (v === null || typeof v !== "object" || Array.isArray(v)) throw new ErroreAltro(`'${pyTipo(v)}' object has no attribute 'get'`);
    const voce = v as Record<string, unknown>;
    if (voce.tipo === "android" || voce.tipo === "outlook" || voce.tipo === "google") {
      for (const k of ["id", "nome", "account"]) if (!Object.hasOwn(voce, k)) throw new ErroreAltro(`KeyError: ${pyReprStr(k)}`);
      const cals = (voce.calendari ?? null) as string[] | null;
      if (voce.tipo === "outlook" || voce.tipo === "google" || usaDirettoPerAndroid) {
        const gestore = ctx.tokens ?? new GestoreToken("/stato/tokens.json", { adesso: ctx.adesso, fetchFn: ctx.fetchFn });
        fonti.push(new CalendarioDiretto(voce.id as string, voce.nome as string, voce.account as string, cals, ctx, gestore, ctx.fetchFn));
      } else {
        fonti.push(new CalendarioAndroid(voce.id as string, voce.nome as string, voce.account as string, cartellaDb, cals, ctx));
      }
    } else ctx.registro("ERROR", `fonte sconosciuta, ignorata: ${pyRepr(v)}`);
  }
  if (ctx.tokens && usaDirettoPerAndroid) {
    for (const [email, voce] of Object.entries(ctx.tokens.elencoAccount())) {
      if (!fonti.some((f) => f.account.toLowerCase() === email.toLowerCase())) {
        fonti.push(new CalendarioDiretto(`${voce.provider}::${email}`, nomeDaEmail(email), email, null, ctx, ctx.tokens, ctx.fetchFn));
      }
    }
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
  private readonly o: Required<Omit<OpzioniAgente, "registro" | "tokens" | "google" | "chiaveTeams">> & { registro: Registro; tokens: GestoreToken | undefined; chiaveTeams: string };
  /** L'ultimo stato degli account Teams mandato da TeamsRelay (shared-teams.ts). */
  private rapportoTeams: RapportoTeams | undefined;
  private readonly google: GoogleCommunication | undefined;
  private readonly cache = new Map<string, { t: number; risposta: Record<string, unknown> }>();
  /** Le caselle rimosse dalla pagina: ferme qui, così un nuovo accesso le riprende come fonti.json le descrive (id, calendari). */
  private readonly ferme: FonteCalendario[] = [];
  private readonly comunicazione: GatewayComunicazione | undefined;

  constructor(o: OpzioniAgente) {
    const adesso = o.adesso ?? Date.now;
    this.o = {
      ...o,
      tokenPosta: o.tokenPosta ?? "/stato/token-outlook.json",
      accessi: o.accessi ?? "/stato/accessi.json",
      tokens: o.tokens,
      chiaveTeams: o.chiaveTeams ?? "",
      maxCorpo: o.maxCorpo ?? MAX_CORPO,
      cacheS: o.cacheS ?? CACHE_S,
      adesso,
      registro: o.registro ?? registroSuStderr(o.zona, adesso),
    };
    if (this.o.tokens) for (const f of [...this.o.fonti]) if (this.o.tokens.rimosso(f.account)) this.sposta(f, this.o.fonti, this.ferme);
    this.comunicazione = this.o.tokens ? new GatewayComunicazione(this.o.tokens, this.o.tokens.fetchFn) : undefined;
    this.google = o.google;
  }

  private sposta(f: FonteCalendario, da: FonteCalendario[], a: FonteCalendario[]): void {
    da.splice(da.indexOf(f), 1);
    a.push(f);
  }

  private haFontiDirette(): boolean {
    return this.o.fonti.some((f) => f instanceof CalendarioDiretto);
  }

  /** Il segno di vita del servizio: da battito.json se presente, altrimenti attivo per le fonti dirette via API. */
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
      if (this.haFontiDirette()) return { eta_s: 0, pausa: false, telefono: true };
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
      if (this.calendarioSpento(fonte)) continue;
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
  private confronta(req: http.IncomingMessage, chiave: string): boolean {
    if (!chiave) return false;
    const atteso = "Bearer " + chiave;
    const dato = req.headers.authorization ?? "";
    if (/[^\x00-\x7f]/.test(dato) || /[^\x00-\x7f]/.test(atteso)) throw new ErroreSenzaRisposta("TypeError: comparing strings with non-ASCII characters is not supported");
    const a = Buffer.from(dato, "latin1");
    const b = Buffer.from(atteso, "latin1");
    return a.length === b.length && crypto.timingSafeEqual(a, b);
  }

  private autorizzato(req: http.IncomingMessage): boolean {
    return this.confronta(req, this.o.chiave);
  }

  /**
   * La chiave di TeamsRelay, solo dove serve a lui: l'elenco degli account condivisi e i token delle risorse della chat. Una chiave uguale a
   * quella piena vale come piena.
   */
  private autorizzatoTeams(req: http.IncomingMessage, metodo: "GET" | "POST", percorso: string, query: string): boolean {
    if (this.o.chiaveTeams === this.o.chiave || !this.confronta(req, this.o.chiaveTeams)) return false;
    if (metodo === "POST") return percorso === "/api/auth/shared" || percorso === "/api/auth/shared/teams";
    if (percorso === "/api/auth/shared") return true;
    return percorso === "/api/token-resource" && RISORSE_CHAT.has(new URLSearchParams(query).get("resource") ?? "");
  }

  /** Una casella condivisa senza la capacità calendario: il calendario non la legge e non la mostra, senza errori. */
  private calendarioSpento(f: FonteCalendario): boolean {
    const shared = this.o.tokens?.voceAccount(f.account)?.shared;
    return Boolean(shared && !shared.capabilities.includes("calendar"));
  }

  private invia(req: http.IncomingMessage, res: http.ServerResponse, codice: number, dati: unknown, scartaCorpo = false, intestazioni: Record<string, string> = {}): void {
    this.inviaJson(req, res, codice, Buffer.from(jsonPy(dati), "utf-8"), scartaCorpo, intestazioni);
  }

  /** Un corpo JSON già pronto (la risposta di Outlook del gateway passa così com'è). */
  private inviaJson(req: http.IncomingMessage, res: http.ServerResponse, codice: number, corpo: Buffer, scartaCorpo = false, intestazioni: Record<string, string> = {}): void {
    res.statusCode = codice;
    res.statusMessage = ERRORI_HTTP[codice] ?? http.STATUS_CODES[codice] ?? "";
    res.setHeader("Content-Type", "application/json; charset=utf-8");
    res.setHeader("Content-Length", String(corpo.length));
    for (const [k, v] of Object.entries(intestazioni)) res.setHeader(k, v);
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

  private assicuraFonteDiretta(account: string, provider: "microsoft" | "google"): void {
    const email = account.toLowerCase();
    this.o.tokens?.riammetti(email);
    for (const f of this.ferme.filter((x) => x.account.toLowerCase() === email)) this.sposta(f, this.ferme, this.o.fonti);
    let trovata = false;
    for (const f of this.o.fonti) {
      if (f.account.toLowerCase() === email) {
        f.svuotaCache?.();
        trovata = true;
      }
    }
    if (!trovata && this.o.tokens) {
      const ctx = contestoDi({ accessi: this.o.accessi, zona: this.o.zona, adesso: this.o.adesso, registro: this.o.registro, tokens: this.o.tokens });
      this.o.fonti.push(new CalendarioDiretto(`${provider}::${email}`, nomeDaEmail(email), email, null, ctx, this.o.tokens));
    }
  }

  private cancelSignIn(account: string): void {
    if (!this.o.tokens) return;
    const jobs = new SignIns(this.o.tokens.percorso + ".signins.json", () => Math.floor(this.o.adesso() / 1000));
    for (const job of jobs.list().filter(j => j.account === account && ["queued", "browser"].includes(j.phase))) jobs.update(job.id, { phase: "blocked", error: "Connection cancelled because service choices changed. Start a new connection if needed." });
  }

  private async post(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    const { percorso, query } = analizzaTarget(req.url ?? "");
    if (!this.autorizzato(req) && !this.autorizzatoTeams(req, "POST", percorso, query)) return this.invia(req, res, 401, { errore: "non autorizzato" });
    if (!["/api/ics", "/api/auth/inizio", "/api/auth/completa", "/api/auth/controlla", "/api/auth/rimuovi", "/api/auth/shared", "/api/auth/shared/teams", "/api/communication/read", "/api/communication/gmail/read", "/api/communication/gmail/write", "/api/communication/gchat/read", "/api/communication/gchat/write"].includes(percorso)) {
      return this.invia(req, res, 404, { errore: "sconosciuto" });
    }
    const dichiarata = req.headers["content-length"] || "0";
    const lunghezza = /^\s*[+-]?\d+\s*$/.test(dichiarata) ? Number(dichiarata.trim()) : -1;
    if (!(lunghezza > 0 && lunghezza <= this.o.maxCorpo)) return this.invia(req, res, lunghezza > this.o.maxCorpo ? 413 : 400, { errore: "lunghezza della richiesta non valida" }, true);
    const corpo = await leggiCorpo(req);
    if (corpo === null) return;
    if (percorso.startsWith("/api/communication/gmail/") || percorso.startsWith("/api/communication/gchat/")) return this.googleRequest(req, res, percorso, corpo);
    if (["/api/auth/inizio", "/api/auth/completa", "/api/auth/rimuovi"].includes(percorso)) {
      let parsed: unknown;
      try { parsed = leggiJson(corpo); } catch { return this.invia(req, res, 400, { errore: "invalid JSON" }); }
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        const data = parsed as Record<string, unknown>;
        if (["gmail", "gchat", "google-web"].includes(String(data.provider))) return this.googleAuth(req, res, percorso, data);
      }
    }
    if (percorso === "/api/communication/read") return this.leggiComunicazione(req, res, corpo);
    if (percorso === "/api/auth/shared") {
      let dati: unknown;
      try {
        dati = leggiJson(corpo);
      } catch {
        return this.invia(req, res, 400, { errore: "invalid JSON" });
      }
      const data = dati as Record<string, unknown>;
      if (data && ["connect", "signin-progress", "signin-import"].includes(String(data.operation))) {
        if (!this.o.tokens) return this.invia(req, res, 400, { ok: false, errore: "Token store unavailable" });
        const tokens = this.o.tokens;
        const jobs = new SignIns(tokens.percorso + ".signins.json", () => Math.floor(this.o.adesso() / 1000));
        try {
          const account = typeof data.account === "string" ? data.account.trim().toLowerCase() : "";
          if (data.operation === "connect") {
            const report = rapportoValido(this.rapportoTeams, Math.floor(this.o.adesso() / 1000));
            if (!report) throw new Error("Microsoft sign-in service unavailable. TeamsRelay must be connected.");
            const previous = tokens.voceAccount(account);
            if (!Array.isArray(data.capabilities) || !data.capabilities.length || data.capabilities.some(c => !["mail", "calendar", "chat"].includes(c))) throw new Error("Choose mail, calendar or Teams");
            if (previous && previous.provider !== "microsoft" && data.capabilities.some(c => c !== "chat")) throw new Error("This address already has a different mail provider. Its credentials will not be replaced.");
            const existing = previous?.provider === "microsoft" ? previous.shared?.capabilities ?? ["mail", "calendar"] : [];
            const browserOn = report.others.some(t => t.email === account && t.state !== "stopped");
            const signin = jobs.start(account, [...new Set([...data.capabilities, ...existing, ...(browserOn ? ["chat"] : [])])]);
            if (signin.phase === "queued" && previous?.provider === "microsoft" && previous.refresh_token && previous.client_id !== "9199bf20-a13f-4107-85dc-02114787ef48" && !previous.errore && report && !report.others.some(t => t.email === account)) {
              try {
                await attivaCondivisione(tokens, account, signin.capabilities);
                jobs.update(signin.id, { phase: "complete", route: "outlook-mobile" });
                this.cache.clear();
              } catch (e) {
                // Only expired/refused credentials enter the browser flow; an outage must not request a fresh login.
                if (!tokens.voceAccount(account)?.errore && !(e instanceof Error && e.cause instanceof SharedAuthorizationRefused)) jobs.update(signin.id, { phase: "blocked", error: testoErrore(e) });
              }
            }
            return this.invia(req, res, 200, { ok: true, signin: jobs.list().find(j => j.id === signin.id) });
          }
          const job = jobs.find(data.id, account);
          const firstBinding = data.operation === "signin-progress" && data.phase === "browser" && job.phase === "queued";
          if (!(data.operation === "signin-progress" && data.phase === "blocked" && data.slot === undefined) && (!Number.isInteger(data.slot) || Number(data.slot) < 1 || Number(data.slot) > 9999 || (job.slot && job.slot !== data.slot && !firstBinding))) throw new Error("Sign-in slot mismatch");
          if (data.operation === "signin-progress") {
            if (data.phase !== "browser" && data.phase !== "blocked" && data.phase !== "complete") throw new Error("Invalid sign-in phase");
            const url = data.url ? new URL(String(data.url)) : undefined;
            if (url && (!/^https?:$/.test(url.protocol) || url.username || url.password || url.pathname !== `/api/desktop/${data.slot}` || url.search || url.hash)) throw new Error("Invalid sign-in URL");
            if (data.phase === "complete" && job.capabilities.some(c => c !== "chat") && !tokens.haAccount(account)) throw new Error("Mailbox not imported");
            return this.invia(req, res, 200, { ok: true, signin: jobs.update(job.id, { phase: data.phase, ...(data.slot ? { slot: Number(data.slot) } : {}), ...(url ? { url: url.href } : {}), error: typeof data.error === "string" ? data.error.slice(0, 300) : undefined }) });
          }
          const session = data.sessione as { loginHint?: string } | null;
          if (!job.slot || job.phase !== "browser" || session?.loginHint?.trim().toLowerCase() !== account) throw new Error("Microsoft session identity mismatch");
          const result = await tokens.importaGuidata(session, typeof data.tenant === "string" ? data.tenant : "organizations", token => {
            jobs.find(job.id, account);
            if (tokens.voceAccount(account)?.provider === "google") throw new Error("The account provider changed during sign-in");
            identitaDaToken(token, account);
          }, job.capabilities.filter((c): c is "mail" | "calendar" => c !== "chat"));
          const token = tokens.voceAccount(account)?.access_token;
          if (!token) throw new Error("Microsoft did not return a mailbox token");
          const identity = identitaDaToken(token, account);
          const caps = job.capabilities.filter(c => c !== "chat");
          tokens.impostaShared(account, { ...identity, capabilities: caps, origin: result.route === "outlook-web" ? "outlook-spa" : "mobile" });
          this.assicuraFonteDiretta(account, "microsoft");
          this.cache.clear();
          jobs.update(job.id, { route: result.route });
          return this.invia(req, res, 200, { ok: true, route: result.route });
        } catch (e) {
          return this.invia(req, res, 400, { ok: false, ...(e instanceof ProxyBrowserRequired ? { needsProxy: true } : {}), ...(e instanceof SessioneScaduta && !testoErrore(e).includes("AADSTS53003") ? { needsBrowser: true } : {}), errore: testoErrore(e) });
        }
      }
      if (data && (data.operation === "remove" || data.operation === "chat")) {
        if (!this.o.tokens || typeof data.account !== "string")
          return this.invia(req, res, 400, {
            ok: false,
            errore: "account required",
          });
        const account = data.account.trim().toLowerCase();
        const teams = rapportoValido(
          this.rapportoTeams,
          Math.floor(this.o.adesso() / 1000),
        );
        if (!teams)
          return this.invia(req, res, 409, {
            ok: false,
            errore: "Teams report unavailable",
          });
        const mailbox = this.o.tokens.voceAccount(account);
        if (mailbox && mailbox.provider !== "microsoft")
          return this.invia(req, res, 400, {
            ok: false,
            errore: "Microsoft account required",
          });
        const browser = teams.others.some((t) => t.email === account);
        if (!mailbox && !browser && !teams.accounts.has(account))
          return this.invia(req, res, 404, {
            ok: false,
            errore: "Account not found",
          });
        if (
          data.operation === "chat" &&
          (typeof data.on !== "boolean" || !browser)
        )
          return this.invia(req, res, 400, {
            ok: false,
            errore: "Browser Teams account required",
          });
        const file = this.o.tokens.percorso + ".commands.json";
        const commands = readCommands(file);
        if (commands.some((c) => c.account === account))
          return this.invia(req, res, 409, {
            ok: false,
            errore: "Account operation already pending",
          });
        commands.push({
          id: crypto.randomUUID(),
          account,
          operation: data.operation,
          ...(data.operation === "chat" ? { on: data.on as boolean } : {}),
        });
        saveCommands(file, commands);
        if (data.operation === "remove" || (data.operation === "chat" && data.on === false)) this.cancelSignIn(account);
        if (data.operation === "remove" && mailbox) {
          for (const f of this.o.fonti.filter(
            (f) => f.account.toLowerCase() === account,
          ))
            this.sposta(f, this.o.fonti, this.ferme);
          this.o.tokens.rimuovi(account);
          this.cache.clear();
        }
        return this.invia(req, res, 200, { ok: true, account, pending: true });
      }
      if (this.o.tokens && typeof data?.account === "string" && Array.isArray(data.capabilities)) {
        const account = data.account.trim().toLowerCase();
        const current = this.o.tokens.voceAccount(account);
        const before = current?.shared?.capabilities ?? ["mail", "calendar"];
        const removingOnly = data.capabilities.every(c => before.includes(c));
        if (removingOnly) this.cancelSignIn(account);
        else if (new SignIns(this.o.tokens.percorso + ".signins.json").list().some(j => j.account === account && ["queued", "browser"].includes(j.phase))) return this.invia(req, res, 409, { ok: false, errore: "Finish the connection before enabling more services" });
      }
      const esito = await richiestaCondivisione(this.o.tokens, "POST", dati);
      if (esito.ok === true) {
        this.cache.clear();
        this.o.registro(
          "INFO",
          `condivisione casella aggiornata: ${String(esito.account ?? "")}`,
        );
      }
      return this.invia(req, res, 200, esito);
    }
    if (percorso === "/api/auth/shared/teams") {
      let dati: unknown;
      try {
        dati = leggiJson(corpo);
      } catch {
        return this.invia(req, res, 400, { errore: "invalid JSON" });
      }
      try {
        const input = dati as Record<string, unknown>;
        if (
          !input ||
          typeof input !== "object" ||
          Object.keys(input).some(
            (k) => !["accounts", "others", "results"].includes(k),
          )
        )
          throw new ErroreRapportoTeams("Invalid report fields");
        this.rapportoTeams = leggiRapportoTeams(
          { accounts: input.accounts, others: input.others },
          Math.floor(this.o.adesso() / 1000),
        );
        if (this.o.tokens && Array.isArray(input.results)) {
          const file = this.o.tokens.percorso + ".commands.json";
          const results = input.results as { id: string; error?: string }[];
          saveCommands(
            file,
            readCommands(file).flatMap((c) => {
              const result = results.find((r) => r.id === c.id);
              return !result
                ? [c]
                : result.error
                  ? [{ ...c, error: String(result.error).slice(0, 300) }]
                  : [];
            }),
          );
        }
      } catch (e) {
        if (e instanceof ErroreRapportoTeams)
          return this.invia(req, res, 400, { errore: e.message });
        throw e;
      }
      return this.invia(req, res, 200, { ok: true });
    }
    if (percorso === "/api/auth/inizio") {
      if (!this.o.tokens) return this.invia(req, res, 400, { ok: false, errore: "gestore token non configurato" });
      try {
        const dati = leggiJson(corpo);
        const r = dati !== null && typeof dati === "object" && !Array.isArray(dati) ? (dati as Record<string, unknown>) : {};
        const email = typeof r.email === "string" ? r.email.trim() : undefined;
        const provider = r.provider === "google" ? "google" : "microsoft";
        if (provider === "google") {
          const { url, state } = this.o.tokens.iniziaGooglePkce(email);
          return this.invia(req, res, 200, { ok: true, provider: "google", modo: "pkce", url, state });
        }
        if (r.modo === "device") {
          try {
            const dev = await this.o.tokens.iniziaDeviceMicrosoft(email);
            return this.invia(req, res, 200, { ok: true, provider: "microsoft", modo: "device", ...dev });
          } catch {
            // Ripiega sul link PKCE diretto se il codice dispositivo non è raggiungibile
          }
        }
        const { url, state } = this.o.tokens.iniziaPkce(email);
        return this.invia(req, res, 200, { ok: true, provider: "microsoft", modo: "pkce", url, state });
      } catch (e) {
        if (e instanceof ErroreSenzaRisposta) throw e;
        return this.invia(req, res, 200, { ok: false, errore: testoErrore(e) });
      }
    }
    if (percorso === "/api/auth/rimuovi") {
      if (!this.o.tokens) return this.invia(req, res, 400, { ok: false, errore: "gestore token non configurato" });
      try {
        const dati = leggiJson(corpo);
        const r = dati !== null && typeof dati === "object" && !Array.isArray(dati) ? (dati as Record<string, unknown>) : {};
        const email = typeof r.account === "string" ? r.account.trim().toLowerCase() : "";
        const sue = this.o.fonti.filter((f) => f.account.toLowerCase() === email);
        if (!email || (!sue.length && !this.o.tokens.haAccount(email))) return this.invia(req, res, 200, { ok: false, errore: "Casella non trovata" });
        for (const f of sue) this.sposta(f, this.o.fonti, this.ferme);
        this.o.tokens.rimuovi(email);
        this.cache.clear();
        this.o.registro("INFO", `casella rimossa: ${email}`);
        return this.invia(req, res, 200, { ok: true, account: email });
      } catch (e) {
        if (e instanceof ErroreSenzaRisposta) throw e;
        return this.invia(req, res, 200, { ok: false, errore: testoErrore(e) });
      }
    }
    if (percorso === "/api/auth/controlla") {
      if (!this.o.tokens) return this.invia(req, res, 400, { ok: false, errore: "gestore token non configurato" });
      try {
        const dati = leggiJson(corpo);
        const r = dati !== null && typeof dati === "object" && !Array.isArray(dati) ? (dati as Record<string, unknown>) : {};
        const state = typeof r.state === "string" ? r.state.trim() : "";
        const esito = await this.o.tokens.controllaDeviceMicrosoft(state);
        if (esito.stato === "completato") {
          this.cache.clear();
          this.assicuraFonteDiretta(esito.account, "microsoft");
          return this.invia(req, res, 200, { ok: true, stato: "completato", account: esito.account, provider: "microsoft" });
        }
        return this.invia(req, res, 200, { ok: true, stato: "in_attesa" });
      } catch (e) {
        if (e instanceof ErroreSenzaRisposta) throw e;
        return this.invia(req, res, 200, { ok: false, errore: testoErrore(e) });
      }
    }
    if (percorso === "/api/auth/completa") {
      if (!this.o.tokens) return this.invia(req, res, 400, { ok: false, errore: "gestore token non configurato" });
      try {
        const dati = leggiJson(corpo);
        if (dati === null || typeof dati !== "object" || Array.isArray(dati)) throw new ErroreValore("not a dict");
        const r = dati as Record<string, unknown>;
        const provider = r.provider === "google" ? "google" : "microsoft";
        const state = typeof r.state === "string" ? r.state : undefined;
        let account: string;
        if (provider === "google") {
          const email = typeof r.email === "string" ? r.email : "";
          const codice = typeof r.codice === "string" ? r.codice : "";
          const token = typeof r.token === "string" ? r.token : "";
          if (codice.trim()) {
            ({ account } = await this.o.tokens.completaGooglePkce(codice, state, email));
          } else {
            ({ account } = await this.o.tokens.salvaGoogle(email, token));
          }
        } else if (r.sessione !== undefined) {
          // Sessione del browser (formato teamsrelay session.json) per le caselle dove Outlook Mobile è respinto
          ({ account } = await this.o.tokens.importaSessione(r.sessione, typeof r.tenant === "string" ? r.tenant : ""));
        } else {
          const codice = typeof r.codice === "string" ? r.codice : "";
          ({ account } = await this.o.tokens.completaPkce(codice, state));
        }
        this.cache.clear();
        this.assicuraFonteDiretta(account, provider);
        return this.invia(req, res, 200, { ok: true, account, provider });
      } catch (e) {
        if (e instanceof ErroreSenzaRisposta) throw e;
        return this.invia(req, res, 200, { ok: false, errore: testoErrore(e) });
      }
    }
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

  private googleFailure(req: http.IncomingMessage, res: http.ServerResponse, error: unknown): void {
    if (error instanceof GmailError || error instanceof GoogleChatError) return this.invia(req, res, error.status, { errore: error.message });
    if (error instanceof ErroreSessioneGoogle) return this.invia(req, res, 400, { errore: error.message });
    // Network, filesystem and parsing errors can contain credentials or upstream content.
    return this.invia(req, res, 502, { errore: "Google operation failed" });
  }

  private async googleRequest(req: http.IncomingMessage, res: http.ServerResponse, route: string, body: Buffer): Promise<void> {
    try {
      if (!this.google) throw new GoogleChatError("Google service not configured", 404);
      let data: unknown;
      try { data = leggiJson(body); } catch { throw new GoogleChatError("invalid JSON", 400); }
      if (!data || typeof data !== "object" || Array.isArray(data)) throw new GoogleChatError("JSON object required", 400);
      const input = data as Record<string, unknown>;
      const result = route === "/api/communication/gmail/read" ? await this.google.gmail.read(input)
        : route === "/api/communication/gmail/write" ? await this.google.gmail.write(input)
        : route === "/api/communication/gchat/read" ? await this.google.readChat(input) : await this.google.writeChat(input);
      return this.invia(req, res, 200, result);
    } catch (e) { return this.googleFailure(req, res, e); }
  }

  private async googleAuth(req: http.IncomingMessage, res: http.ServerResponse, route: string, data: Record<string, unknown>): Promise<void> {
    try {
      if (!this.google) throw new GoogleChatError("Google service not configured", 404);
      const provider = data.provider === "gmail" ? "gmail" : "gchat";
      if (route === "/api/auth/rimuovi") return this.invia(req, res, 200, { ok: this.google.remove(provider, data.account) });
      if (route === "/api/auth/inizio") {
        if (provider !== "gmail") throw new GoogleChatError("import a signed-in PC session for Google Chat", 400);
        return this.invia(req, res, 200, { ok: true, modo: "pkce", ...this.google.gmail.begin(typeof data.email === "string" ? data.email : undefined) });
      }
      const result = provider === "gchat" ? await this.google.importChat(data)
        : data.credentials ? await this.google.gmail.import(data)
        : await this.google.gmail.complete({ code: typeof data.codice === "string" ? data.codice : "", state: typeof data.state === "string" ? data.state : undefined });
      return this.invia(req, res, 200, { ok: true, ...result });
    } catch (e) { return this.googleFailure(req, res, e); }
  }

  /** POST /api/communication/read: una lettura di posta o calendario con le credenziali dell'agente; il token non esce. */
  private async leggiComunicazione(req: http.IncomingMessage, res: http.ServerResponse, corpo: Buffer): Promise<void> {
    try {
      if (!this.comunicazione) throw new ErroreComunicazione(404, "casella non trovata");
      let dati: unknown;
      try {
        dati = leggiJson(corpo);
      } catch {
        throw new ErroreComunicazione(400, "il corpo non è JSON");
      }
      return this.inviaJson(req, res, 200, Buffer.from(await this.comunicazione.leggi(dati), "utf-8"));
    } catch (e) {
      if (e instanceof ErroreComunicazione) return this.invia(req, res, e.codice, { errore: e.message }, false, e.intestazioni);
      if (e instanceof ErroreSenzaRisposta) throw e;
      this.o.registro("ERROR", `lettura posta e calendari fallita: ${testoErrore(e)}`);
      return this.invia(req, res, 502, { errore: "lettura non riuscita" });
    }
  }

  private async get(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    const { percorso, query } = analizzaTarget(req.url ?? "");
    const piena = this.autorizzato(req);
    if (!piena && !this.autorizzatoTeams(req, "GET", percorso, query)) return this.invia(req, res, 401, { errore: "non autorizzato" });
    const q = parseQs(query);
    if (percorso === "/api/salute") return this.invia(req, res, 200, { fonti: this.fontiStato(), servizio: this.servizio() });
    if (percorso === "/api/communication/accounts") return this.invia(req, res, 200, { accounts: this.comunicazione?.caselle() ?? [] });
    if (percorso === "/api/communication/google/accounts") return this.invia(req, res, 200, { accounts: this.google?.accounts() ?? [] });
    if (percorso === "/api/auth/shared") {
      const teams = rapportoValido(
        this.rapportoTeams,
        Math.floor(this.o.adesso() / 1000),
      );
      return this.invia(req, res, 200, {
        ...(await richiestaCondivisione(this.o.tokens, "GET", null, teams)),
        signins: this.o.tokens ? new SignIns(this.o.tokens.percorso + ".signins.json", () => Math.floor(this.o.adesso() / 1000)).list() : [],
        commands: this.o.tokens
          ? readCommands(this.o.tokens.percorso + ".commands.json")
          : [],
      });
    }
    if (percorso === "/api/auth/stato") {
      const caselle = this.o.fonti.map((f) => {
        const voce = this.o.tokens?.voceAccount(f.account);
        const provider = voce?.provider ?? (f.tipo === "caldav" ? "google" : "microsoft");
        return {
          id: f.id,
          nome: f.nome,
          account: f.account,
          provider,
          configurato: Boolean(voce),
          // senza accesso salvato, o con un accesso che il fornitore non accetta più, serve un nuovo accesso
          scaduto: !voce || Boolean(voce.errore),
          stato: voce?.errore ? "errore" : f.stato.stato,
          msg: voce?.errore || String(f.stato.msg ?? ""),
        };
      });
      return this.invia(req, res, 200, { caselle });
    }
    // Trusted service endpoint. Capabilities do not downscope raw tokens; gateways filter reads.
    if (percorso === '/api/token-resource') {
 res.setHeader('Cache-Control', 'no-store');
 const query = new URL(req.url!, 'http://localhost').searchParams;
 const risorsa = query.get('resource') || '';
 // la chiave di TeamsRelay solo per la chat, controllato di nuovo sul valore che si usa davvero
 if (!piena && !RISORSE_CHAT.has(risorsa)) return this.invia(req, res, 401, { errore: 'non autorizzato' });
 try {
 if (!this.o.tokens) throw new Error('broker unavailable');
 const token = await this.o.tokens.ottieniRisorsa(query.get('account') || '', risorsa);
 return this.invia(req, res, 200, { token });
 } catch { return this.invia(req, res, 403, { errore: 'shared token unavailable' }); }
 }
    if (percorso === "/api/token-posta") {
      if (this.o.tokens && Object.keys(this.o.tokens.elencoAccount()).length > 0) {
        const diretti = await this.o.tokens.tokenPostaAttivi();
        if (diretti.length > 0) {
          return this.invia(req, res, 200, {
            generato: Math.floor(this.o.adesso() / 1000),
            account: diretti,
            servizio: this.servizio(),
          });
        }
      }
      let generato = 0;
      let account: Record<string, unknown>[] = [];
      try {
        const raw = JSON.parse(leggiUtf8(this.o.tokenPosta)) as Record<string, unknown>;
        if (typeof raw.generato === "number") generato = raw.generato;
        if (Array.isArray(raw.account)) account = raw.account as Record<string, unknown>[];
      } catch {
        // Se il file non è ancora stato scritto dal supervisore, si risponde con elenco vuoto.
      }
      let caselleStato: Record<string, { stato?: string; msg?: string }> = {};
      try {
        const accRaw = JSON.parse(leggiUtf8(this.o.accessi)) as Record<string, unknown>;
        if (accRaw.caselle && typeof accRaw.caselle === "object") {
          caselleStato = accRaw.caselle as Record<string, { stato?: string; msg?: string }>;
        }
      } catch {
        // Ignora assenza accessi.json
      }
      const arricchiti = account.filter((a) => !this.o.tokens?.voceAccount(String(a.upn ?? ""))?.shared).map((a) => {
        const upn = String(a.upn ?? "").toLowerCase();
        const st = caselleStato[upn];
        return {
          ...a,
          phone_status: st?.stato ?? "ok",
          phone_msg: st?.msg ?? "",
        };
      });
      return this.invia(req, res, 200, { generato, account: arricchiti, servizio: this.servizio() });
    }
    if (percorso === "/api/calendari") {
      const caselle: Record<string, unknown>[] = [];
      for (const f of this.o.fonti) {
        if (this.calendarioSpento(f)) continue;
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
