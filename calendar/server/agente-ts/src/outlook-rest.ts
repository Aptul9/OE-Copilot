// Il nucleo Microsoft che l'agente usa in più punti: gli indirizzi di Microsoft, i claims dei token, il controllo dei token del client web
// di Outlook e il trasporto REST (GET con il bearer della casella; un 401 rinnova il token una volta e riprova). Il trasporto è lo stesso
// per i calendari (CalendarioDiretto, anche Google) e per il gateway di posta e calendari (comunicazione.ts).

export const MS_LOGIN = "https://login.microsoftonline.com";
export const OUTLOOK_REST = "https://outlook.office.com/api/v2.0";
const AUD_OUTLOOK = new Set(["https://outlook.office.com", "https://outlook.office365.com"]);

export type FetchFn = typeof fetch;

/** I claims di un JWT, senza verificarne la firma: servono a leggere casella, risorsa, scadenza e permessi. {} se non è un JWT. */
export function claimsJwt(jwt: string): Record<string, unknown> {
  const parti = jwt.split(".");
  if (parti.length < 2 || !parti[1]) return {};
  try {
    const b64 = parti[1].replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - (parti[1].length % 4)) % 4);
    const parsed = JSON.parse(Buffer.from(b64, "base64").toString("utf-8")) as unknown;
    return parsed !== null && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/** La casella di un token Microsoft, in minuscolo; "" se il token non la dice. */
export function upnDi(claims: Record<string, unknown>): string {
  return String(claims.upn || claims.preferred_username || claims.unique_name || "").toLowerCase();
}

/**
 * Un token del client web di Outlook si accetta solo se è della stessa casella, per la risorsa Outlook, non scaduto e con il permesso sui
 * calendari: vale per l'importazione della sessione, per ogni rinnovo col refresh_token del client web e per ogni nuovo accesso dalla
 * sessione del browser. Restituisce il motivo del rifiuto, o null se il token va bene.
 */
export function motivoTokenWebNonValido(accessToken: unknown, casella: string, adessoS: number, required: readonly ("mail" | "calendar")[] = ["calendar"]): string | null {
  if (typeof accessToken !== "string" || !accessToken) return "Risposta token Microsoft senza access_token";
  const claims = claimsJwt(accessToken);
  if (upnDi(claims) !== casella.trim().toLowerCase()) return "Microsoft ha restituito un'altra casella";
  if (typeof claims.aud !== "string" || !AUD_OUTLOOK.has(claims.aud)) return "Microsoft ha restituito un token per un'altra risorsa";
  const exp = Number(claims.exp);
  if (!Number.isFinite(exp) || exp <= adessoS) return "Microsoft ha restituito un token già scaduto";
  const scp = typeof claims.scp === "string" ? claims.scp.split(" ") : [];
  if (required.includes("calendar") && !scp.some(s => /^Calendars\.(Read|ReadWrite)(\.Shared)?$/.test(s))) return "Il token non ha il permesso sui calendari";
  if (required.includes("mail") && !scp.some(s => /^Mail\.(Read|ReadWrite)(\.Shared)?$/.test(s))) return "Il token non ha il permesso sulla posta";
  return null;
}

/** Chi dà i token di una casella e li rinnova (GestoreToken). */
export interface FonteToken {
  ottieniAccessToken(email: string, forza?: boolean): Promise<string>;
}

export interface OpzioniRest {
  headers?: Record<string, string>;
  fetchFn?: FetchFn;
  timeoutMs?: number;
  /** Oltre questi byte il corpo non si legge: ErroreRispostaGrande. Senza, si legge tutto. */
  maxByte?: number;
  /** Senza, i redirect si seguono (come fetch). */
  redirect?: NonNullable<RequestInit["redirect"]>;
}

export interface RispostaRest {
  status: number;
  statusText: string;
  headers: Headers;
  testo: string;
}

export class ErroreRispostaGrande extends Error {}

export async function leggiTesto(resp: Response, maxByte: number | undefined): Promise<string> {
  if (maxByte === undefined || !resp.body) return resp.text();
  const dichiarata = Number(resp.headers.get("content-length") ?? "");
  if (Number.isFinite(dichiarata) && dichiarata > maxByte) {
    await resp.body.cancel().catch(() => undefined);
    throw new ErroreRispostaGrande(`risposta oltre ${maxByte} byte`);
  }
  const pezzi: Uint8Array[] = [];
  let letti = 0;
  for await (const pezzo of resp.body) {
    letti += pezzo.byteLength;
    // uscire dal ciclo annulla il resto dello stream
    if (letti > maxByte) throw new ErroreRispostaGrande(`risposta oltre ${maxByte} byte`);
    pezzi.push(pezzo);
  }
  return new TextDecoder("utf-8").decode(Buffer.concat(pezzi));
}

/**
 * GET con il token della casella. Un 401 forza il rinnovo del token e riprova una volta; il secondo esito, qualunque sia, torna a chi
 * chiama. Il corpo di una risposta d'errore che non si riesce a leggere diventa "".
 */
export async function getRest(tokens: FonteToken, account: string, url: string, o: OpzioniRest = {}): Promise<RispostaRest> {
  const fetchFn = o.fetchFn ?? fetch;
  for (const forza of [false, true]) {
    const token = await tokens.ottieniAccessToken(account, forza);
    const resp = await fetchFn(url, {
      headers: { ...o.headers, Authorization: `Bearer ${token}`, Accept: "application/json" },
      redirect: o.redirect,
      signal: AbortSignal.timeout(o.timeoutMs ?? 20_000),
    });
    if (resp.status === 401 && !forza) {
      await resp.body?.cancel().catch(() => undefined);
      continue;
    }
    const testo = resp.ok ? await leggiTesto(resp, o.maxByte) : await leggiTesto(resp, o.maxByte).catch(() => "");
    return { status: resp.status, statusText: resp.statusText, headers: resp.headers, testo };
  }
  throw new Error("irraggiungibile");
}

/** GET che vuole un JSON: una risposta non 2xx è un errore "HTTP <codice>: <inizio del corpo>". */
export async function jsonRest(tokens: FonteToken, account: string, url: string, o: OpzioniRest = {}): Promise<Record<string, unknown>> {
  const r = await getRest(tokens, account, url, o);
  if (r.status < 200 || r.status > 299) throw new Error(`HTTP ${r.status}: ${r.testo.slice(0, 160) || r.statusText}`);
  return JSON.parse(r.testo) as Record<string, unknown>;
}
