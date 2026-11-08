// Gateway di sola lettura su posta e calendari Outlook per chi non deve avere i token (l'MCP della posta): l'agente tiene le credenziali,
// le rinnova e fa la richiesta; fuori escono solo la risposta di Outlook e i nomi delle caselle.
//   GET  /api/communication/accounts  {"accounts": [{"upn", "status", "message"}]}: le caselle Microsoft con accesso, senza le rimosse
//   POST /api/communication/read      {"account": casella esatta, "path": percorso REST relativo[, "prefer": 'outlook.timezone="Zona"']}
// Solo GET verso https://outlook.office.com/api/v2.0, solo i percorsi di lettura di posta e calendari elencati qui sotto, solo i parametri
// OData elencati, nessuna intestazione oltre al Prefer del fuso.

import { ErroreAccessoScaduto, type GestoreToken } from "./diretto.ts";
import { ErroreRispostaGrande, OUTLOOK_REST, claimsJwt, getRest, upnDi, type FetchFn } from "./outlook-rest.ts";

export const MAX_PERCORSO = 4096;
/** 8 MiB, come il trasporto dell'MCP (HTTP e SSH): un allegato più grande torna 502 e si legge da Outlook stesso. */
export const MAX_RISPOSTA = 8 * 1024 * 1024;
const MAX_TOP = 1000;

/** Errore con il codice HTTP da dare a chi chiama; il messaggio va in {"errore": ...}. */
export class ErroreComunicazione extends Error {
  readonly codice: number;
  readonly intestazioni: Record<string, string>;
  constructor(codice: number, messaggio: string, intestazioni: Record<string, string> = {}) {
    super(messaggio);
    this.codice = codice;
    this.intestazioni = intestazioni;
  }
}

export interface CasellaComunicazione {
  capabilities?: ('mail' | 'calendar' | 'chat')[];
  upn: string;
  status: "ok" | "errore";
  message: string;
}

// Le forme dei percorsi ammessi: segmenti fissi (minuscoli, il confronto ignora le maiuscole) e "*" per un identificativo.
const FORME: readonly (readonly string[])[] = [
  ["me", "mailfolders"],
  ["me", "mailfolders", "*"],
  ["me", "mailfolders", "*", "childfolders"],
  ["me", "mailfolders", "*", "messages"],
  ["me", "messages"],
  ["me", "messages", "*"],
  ["me", "messages", "*", "attachments"],
  ["me", "messages", "*", "attachments", "*"],
  ["me", "calendars"],
  ["me", "calendars", "*"],
  ["me", "calendars", "*", "calendarview"],
  ["me", "calendars", "*", "events"],
  ["me", "calendar"],
  ["me", "calendar", "calendarview"],
  ["me", "calendar", "events"],
  ["me", "calendarview"],
  ["me", "events"],
  ["me", "events", "*"],
  ["me", "events", "*", "instances"],
  ["me", "events", "*", "attachments"],
];

const PARAMETRI = new Set(["$top", "$skip", "$select", "$filter", "$orderby", "$search", "$expand", "$count", "$skiptoken", "startdatetime", "enddatetime"]);
// Gli @odata.nextLink di Outlook sono indirizzi interi: si accettano solo con questa base e si chiedono sempre a OUTLOOK_REST.
const BASI_ASSOLUTE = ["https://outlook.office.com/api/v2.0/", "https://outlook.office365.com/api/v2.0/"];
const IDENTIFICATIVO = /^(?:[A-Za-z0-9._~!$&'()*+,;=:@-]|%[0-9A-Fa-f]{2})+$/;
// La chiave OData di un nextLink, MailFolders('AAMk...='): diventa MailFolders/AAMk...= prima del confronto con le forme.
const CHIAVE_ODATA = /^([A-Za-z]+)\('((?:[A-Za-z0-9._~!$&*+,;=:@-]|%[0-9A-Fa-f]{2})+)'\)$/;
const VALORE = /^[\x21\x22\x24-\x7e]*$/;
const PREFER_FUSO = /^outlook\.timezone="([A-Za-z0-9 _+./-]{1,64})"$/;

const rifiuta = (motivo: string): never => {
  throw new ErroreComunicazione(400, motivo);
};

function decodifica(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return rifiuta("codifica % non valida");
  }
}

/**
 * Un identificativo di Outlook in un solo segmento. Gli Id sono base64 e possono avere "/" (mandato come %2F, come fa encodeURIComponent):
 * si accetta, ma nessun pezzo tra due "/" può essere "." o "..", né iniziare con "$" ($value, $count, $ref non sono letture JSON).
 */
function identificativoValido(s: string): boolean {
  if (!IDENTIFICATIVO.test(s)) return false;
  const chiaro = decodifica(s);
  if (chiaro.startsWith("$") || /[\x00-\x1f\x7f\\?#]/.test(chiaro)) return false;
  return chiaro.split("/").every((pezzo) => pezzo !== "." && pezzo !== "..");
}

/** Il percorso relativo da chiedere a OUTLOOK_REST, se è una lettura ammessa; altrimenti ErroreComunicazione 400 con il motivo. */
export function percorsoLettura(dato: unknown): string {
  if (typeof dato !== "string" || !dato) return rifiuta("manca path");
  if (dato.length > MAX_PERCORSO) return rifiuta(`path oltre ${MAX_PERCORSO} caratteri`);
  let p = dato;
  const base = BASI_ASSOLUTE.find((b) => p.slice(0, b.length).toLowerCase() === b);
  if (base) p = p.slice(base.length);
  else p = p.replace(/^\/+/, "");
  if (!/^[\x21-\x7e]+$/.test(p) || p.includes("#") || p.includes("\\")) return rifiuta("path con caratteri non ammessi");
  const punto = p.indexOf("?");
  const parteP = punto < 0 ? p : p.slice(0, punto);
  const query = punto < 0 ? "" : p.slice(punto + 1);

  // gli @odata.nextLink hanno a volte "/" prima di "?": si toglie un solo "/" finale, mai segmenti vuoti in mezzo
  const segmenti: string[] = [];
  for (const s of parteP.replace(/\/$/, "").split("/")) {
    const chiave = CHIAVE_ODATA.exec(s);
    if (chiave) segmenti.push(chiave[1]!, chiave[2]!);
    else segmenti.push(s);
  }
  const forma = FORME.find((f) => f.length === segmenti.length && f.every((atteso, i) => (atteso === "*" ? identificativoValido(segmenti[i]!) : segmenti[i]!.toLowerCase() === atteso)));
  if (!forma) return rifiuta("path non è una lettura di posta o calendario ammessa");

  const visti = new Set<string>();
  for (const campo of query ? query.split("&") : []) {
    const uguale = campo.indexOf("=");
    if (uguale < 1) return rifiuta("parametro della query senza nome o valore");
    const nome = decodifica(campo.slice(0, uguale)).toLowerCase();
    const valore = campo.slice(uguale + 1);
    if (!PARAMETRI.has(nome)) return rifiuta(`parametro non ammesso: ${nome.slice(0, 40)}`);
    if (visti.has(nome)) return rifiuta(`parametro ripetuto: ${nome}`);
    visti.add(nome);
    if (!VALORE.test(valore)) return rifiuta(`valore non ammesso per ${nome}`);
    const chiaro = decodifica(valore.replaceAll("+", " "));
    if (nome === "$top" && !(/^\d{1,4}$/.test(chiaro) && Number(chiaro) >= 1 && Number(chiaro) <= MAX_TOP)) return rifiuta(`$top va da 1 a ${MAX_TOP}`);
    if (nome === "$skip" && !/^\d{1,7}$/.test(chiaro)) return rifiuta("$skip non valido");
  }

  const percorso = segmenti.join("/");
  const relativo = percorso + (query ? `?${query}` : "");
  // il parser degli URL non deve cambiare il percorso (punti, codifiche): si chiede quello che si è controllato
  const base0 = new URL(OUTLOOK_REST);
  const url = new URL(`${OUTLOOK_REST}/${relativo}`);
  if (url.origin !== base0.origin || url.pathname !== `${base0.pathname}/${percorso}` || url.hash) return rifiuta("path non ammesso");
  return relativo;
}

/** L'intestazione Prefer ammessa: solo il fuso delle date di Outlook. Nessun'altra intestazione passa. */
export function preferFuso(dato: unknown): string | undefined {
  if (dato === undefined || dato === null) return undefined;
  if (typeof dato !== "string" || !PREFER_FUSO.test(dato) || dato.includes("..")) return rifiuta('prefer ammette solo outlook.timezone="Zona"');
  return dato;
}

/** Il codice d'errore di Outlook ({"error": {"code": "ErrorItemNotFound"}}), solo se ha la forma di un codice. */
function codiceErroreOutlook(testo: string): string {
  try {
    const codice = ((JSON.parse(testo) as { error?: { code?: unknown } }).error ?? {}).code;
    return typeof codice === "string" && /^[A-Za-z][A-Za-z0-9_.]{0,63}$/.test(codice) ? codice : "";
  } catch {
    return "";
  }
}

export class GatewayComunicazione {
  private readonly tokens: GestoreToken;
  private readonly fetchFn: FetchFn;

  constructor(tokens: GestoreToken, fetchFn: FetchFn = fetch) {
    this.tokens = tokens;
    this.fetchFn = fetchFn;
  }

  /** Le caselle Microsoft con un accesso salvato, senza quelle rimosse. Nessun token e nessun rinnovo: si legge solo l'archivio. */
  caselle(): CasellaComunicazione[] {
    const out: CasellaComunicazione[] = [];
    for (const [upn, voce] of Object.entries(this.tokens.elencoAccount())) {
      if (voce.provider !== "microsoft" || this.tokens.rimosso(upn)) continue;
      out.push({ ...(voce.shared ? { capabilities: voce.shared.capabilities } : {}), upn, status: voce.errore ? "errore" : "ok", message: voce.errore ?? "" });
    }
    return out;
  }

  /** Il corpo della richiesta di lettura, controllato: casella esatta, percorso ammesso, Prefer del fuso, nessun altro campo. */
  richiesta(dati: unknown): { account: string; percorso: string; prefer: string | undefined } {
    if (dati === null || typeof dati !== "object" || Array.isArray(dati)) return rifiuta('serve {"account": ..., "path": ...}');
    const r = dati as Record<string, unknown>;
    const estranei = Object.keys(r).filter((k) => !["account", "path", "prefer"].includes(k));
    if (estranei.length) return rifiuta(`campi non ammessi: ${estranei.join(", ").slice(0, 80)}`);
    if (typeof r.account !== "string" || !/^[^\s@]+@[^\s@]+$/.test(r.account)) return rifiuta("account deve essere l'indirizzo esatto della casella");
    const percorso = percorsoLettura(r.path);
    const prefer = preferFuso(r.prefer);
    const account = r.account.toLowerCase();
    if (!this.caselle().some((c) => c.upn === account)) throw new ErroreComunicazione(404, "casella non trovata");
    const shared = this.tokens.voceAccount(account)?.shared;
    if (shared && !shared.capabilities.includes(/^me\/(?:calendars?|calendarview|events)(?:[\/(?]|$)/i.test(percorso) ? 'calendar' : 'mail')) throw new ErroreComunicazione(403, 'capability disabled');
    return { account, percorso, prefer };
  }

  /**
   * La lettura: il testo JSON di Outlook così com'è. Il token resta qui; l'agente lo rinnova quando serve (GestoreToken). Gli errori dicono
   * solo il codice HTTP di Outlook e il suo codice d'errore (ErrorItemNotFound...): mai il testo della risposta, che può citare la posta,
   * mai il messaggio di un errore di rete, che può citare l'indirizzo chiesto.
   */
  async leggi(dati: unknown): Promise<string> {
    const { account, percorso, prefer } = this.richiesta(dati);
    const conCasella = {
      ottieniAccessToken: async (email: string, forza?: boolean) => {
        let token: string;
        try {
          token = await this.tokens.ottieniAccessToken(email, forza);
        } catch (e) {
          // i testi di ErroreAccessoScaduto sono dell'agente (Accedi di nuovo...); gli altri errori di rinnovo possono citare Microsoft
          if (e instanceof ErroreAccessoScaduto) throw new ErroreComunicazione(409, e.message);
          throw new ErroreComunicazione(502, "rinnovo dell'accesso Microsoft non riuscito");
        }
        // un token che dice un'altra casella non esce verso Outlook (i token MSA non sono JWT e non dicono la casella)
        const upn = upnDi(claimsJwt(token));
        if (upn && upn !== account) throw new ErroreComunicazione(409, "il token salvato è di un'altra casella: accedi di nuovo");
        if (this.tokens.rimosso(account) || !this.tokens.haAccount(account)) throw new ErroreComunicazione(404, "casella non trovata");
        return token;
      },
    };
    let r;
    try {
      r = await getRest(conCasella, account, `${OUTLOOK_REST}/${percorso}`, {
        headers: prefer ? { Prefer: prefer } : {},
        fetchFn: this.fetchFn,
        timeoutMs: 30_000,
        maxByte: MAX_RISPOSTA,
        redirect: "manual",
      });
    } catch (e) {
      if (e instanceof ErroreComunicazione) throw e;
      if (e instanceof ErroreRispostaGrande) throw new ErroreComunicazione(502, `risposta di Outlook oltre ${MAX_RISPOSTA} byte`);
      if (e instanceof Error && e.name === "TimeoutError") throw new ErroreComunicazione(504, "Outlook non ha risposto in tempo");
      throw new ErroreComunicazione(502, "Outlook non raggiungibile");
    }
    if (r.status < 200 || r.status > 299) {
      // un 401 dopo il rinnovo, un redirect o un 5xx sono un problema di Outlook, non della richiesta: 502
      const codice = [400, 403, 404, 429].includes(r.status) ? r.status : 502;
      const attesa = r.headers.get("retry-after");
      const codiceOutlook = codiceErroreOutlook(r.testo);
      throw new ErroreComunicazione(
        codice,
        `Outlook HTTP ${r.status}${codiceOutlook ? ` (${codiceOutlook})` : ""}`,
        codice === 429 && attesa && /^\d{1,6}$/.test(attesa) ? { "Retry-After": attesa } : {},
      );
    }
    let json: unknown;
    try {
      json = JSON.parse(r.testo || "{}");
    } catch {
      throw new ErroreComunicazione(502, "Outlook ha risposto senza JSON");
    }
    if (json === null || typeof json !== "object" || Array.isArray(json)) throw new ErroreComunicazione(502, "Outlook ha risposto senza un oggetto JSON");
    return r.testo || "{}";
  }
}
