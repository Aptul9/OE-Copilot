// Accesso Microsoft dalla sessione del browser, senza browser: stessi cookie ESTS che il browser ha gi?, autorizzazione silenziosa (prompt=none, PKCE)
// del client web di Outlook. Serve per le caselle dove il client di Outlook Mobile ? respinto dall'accesso condizionale (AADSTS53003) ma Outlook web no.
// Stesso schema di teamsrelay app/src/agent/light/session-auth.ts, con client, redirect e risorsa di Outlook web.

import crypto from "node:crypto";
import { MS_LOGIN as LOGIN, type FetchFn } from "./outlook-rest.ts";

export const OUTLOOK_WEB_CLIENT_ID = "9199bf20-a13f-4107-85dc-02114787ef48";
export const OUTLOOK_WEB_REDIRECT_URI = "https://outlook.cloud.microsoft/mail/";
export const OUTLOOK_WEB_SCOPES = "https://outlook.office.com/.default offline_access openid profile";

export interface CookieSessione {
  name: string;
  value: string;
  domain: string;
  path: string;
  expires: number;
}

/** Il formato di teamsrelay session.json: si importa cos? com'?. */
export interface SessioneMicrosoft {
  userAgent: string;
  loginHint: string;
  cookies: CookieSessione[];
}

/** Il browser va riaperto dal proprietario: niente da riprovare. */
export class SessioneScaduta extends Error {}

const cookieDiLogin = (c: CookieSessione): boolean =>
  /^\.?login\.microsoftonline\.com$/i.test(c.domain) &&
  /^[!#$%&'*+.^_`|~\w-]+$/.test(c.name) &&
  !/[\x00-\x20\x7f;]/.test(c.value) &&
  c.path.startsWith("/");

const tenantValido = (t: string): boolean => /^[A-Za-z0-9.-]+$/.test(t);

/** Tiene solo i cookie di login.microsoftonline.com: gli altri non servono e non escono. Null se la forma non torna. */
export function leggiSessione(dati: unknown): SessioneMicrosoft | null {
  if (dati === null || typeof dati !== "object" || Array.isArray(dati)) return null;
  const s = dati as Record<string, unknown>;
  if (typeof s.userAgent !== "string" || !s.userAgent || /[\r\n]/.test(s.userAgent)) return null;
  if (typeof s.loginHint !== "string" || !Array.isArray(s.cookies)) return null;
  const cookies = (s.cookies as unknown[]).filter(
    (c): c is CookieSessione =>
      c !== null &&
      typeof c === "object" &&
      ["name", "value", "domain", "path"].every((k) => typeof (c as Record<string, unknown>)[k] === "string") &&
      Number.isFinite((c as Record<string, unknown>).expires) &&
      cookieDiLogin(c as CookieSessione),
  );
  return { userAgent: s.userAgent, loginHint: s.loginHint, cookies };
}

/** Aggiorna i cookie con i Set-Cookie di Microsoft (ESTS ruota), come fa il browser. */
function ruota(s: SessioneMicrosoft, r: Response, adessoS: number): void {
  for (const raw of r.headers.getSetCookie()) {
    const [coppia = "", ...attributi] = raw.split(";");
    const uguale = coppia.indexOf("=");
    if (uguale < 1) continue;
    const c: CookieSessione = { name: coppia.slice(0, uguale).trim(), value: coppia.slice(uguale + 1).trim(), domain: "login.microsoftonline.com", path: "/", expires: -1 };
    let maxAge: number | undefined;
    for (const a of attributi) {
      const i = a.indexOf("=");
      const k = (i < 0 ? a : a.slice(0, i)).trim().toLowerCase();
      const v = i < 0 ? "" : a.slice(i + 1).trim();
      if (k === "domain") c.domain = v;
      else if (k === "path") c.path = v;
      else if (k === "expires" && Number.isFinite(Date.parse(v))) c.expires = Date.parse(v) / 1000;
      else if (k === "max-age" && /^-?\d+$/.test(v)) maxAge = Number(v);
    }
    if (maxAge !== undefined) c.expires = adessoS + maxAge;
    if (!cookieDiLogin(c)) continue;
    const dominio = c.domain.replace(/^\./, "").toLowerCase();
    s.cookies = s.cookies.filter((v) => !(v.name === c.name && v.domain.replace(/^\./, "").toLowerCase() === dominio && v.path === c.path));
    if (c.expires === -1 || c.expires > adessoS) s.cookies.push(c);
  }
}

/**
 * Autorizzazione silenziosa con la sessione del browser, poi scambio del codice. Restituisce la risposta del token endpoint
 * (access_token, refresh_token, ...). La sessione viene aggiornata sul posto con i cookie ruotati: chi chiama la salva.
 */
export async function autorizzaDaSessione(
  s: SessioneMicrosoft,
  tenant: string,
  opzioni: { fetchFn?: FetchFn; adessoS?: number } = {},
): Promise<Record<string, unknown>> {
  const fetchFn = opzioni.fetchFn ?? fetch;
  const adessoS = opzioni.adessoS ?? Math.floor(Date.now() / 1000);
  if (!tenantValido(tenant)) throw new Error("Tenant Microsoft non valido");
  const endpoint = `${LOGIN}/${encodeURIComponent(tenant)}/oauth2/v2.0`;
  const url = new URL(`${endpoint}/authorize`);
  const validi = s.cookies.filter(
    (c) => (c.expires === -1 || c.expires > adessoS) && (c.path === "/" || url.pathname === c.path || url.pathname.startsWith(c.path.endsWith("/") ? c.path : `${c.path}/`)),
  );
  if (!validi.some((c) => /^ESTSAUTH(?:PERSISTENT|LIGHT)?$/i.test(c.name))) throw new SessioneScaduta("Sessione Microsoft finita: accedi di nuovo da Outlook web");
  const verifier = crypto.randomBytes(32).toString("base64url");
  const state = crypto.randomBytes(24).toString("base64url");
  url.search = new URLSearchParams({
    client_id: OUTLOOK_WEB_CLIENT_ID,
    redirect_uri: OUTLOOK_WEB_REDIRECT_URI,
    response_type: "code",
    response_mode: "query",
    scope: OUTLOOK_WEB_SCOPES,
    prompt: "none",
    login_hint: s.loginHint,
    state,
    code_challenge: crypto.createHash("sha256").update(verifier).digest("base64url"),
    code_challenge_method: "S256",
  }).toString();
  const cookie = validi
    .sort((a, b) => b.path.length - a.path.length)
    .map((c) => `${c.name}=${c.value}`)
    .join("; ");
  const r = await fetchFn(url, { headers: { Cookie: cookie, "User-Agent": s.userAgent }, redirect: "manual", signal: AbortSignal.timeout(20_000) });
  ruota(s, r, adessoS);
  if (r.status === 429 || r.status >= 500) throw new Error(`Autorizzazione Microsoft non disponibile (HTTP ${r.status})`);
  const location = r.headers.get("Location");
  if (r.status < 300 || r.status >= 400 || !location) throw new SessioneScaduta("Microsoft chiede di accedere di nuovo da Outlook web");
  const ritorno = new URL(location, LOGIN);
  if (ritorno.origin + ritorno.pathname !== OUTLOOK_WEB_REDIRECT_URI || ritorno.username || ritorno.password) throw new SessioneScaduta("Microsoft chiede di accedere di nuovo da Outlook web");
  if (ritorno.searchParams.get("state") !== state) throw new Error("Autorizzazione Microsoft: state diverso");
  const errore = ritorno.searchParams.get("error");
  if (errore === "server_error" || errore === "temporarily_unavailable") throw new Error("Autorizzazione Microsoft momentaneamente non disponibile");
  const code = ritorno.searchParams.get("code");
  if (errore || !code) throw new SessioneScaduta("Microsoft chiede di accedere di nuovo da Outlook web");
  const t = await fetchFn(`${endpoint}/token`, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json", Origin: new URL(OUTLOOK_WEB_REDIRECT_URI).origin, "User-Agent": s.userAgent },
    body: new URLSearchParams({ client_id: OUTLOOK_WEB_CLIENT_ID, redirect_uri: OUTLOOK_WEB_REDIRECT_URI, grant_type: "authorization_code", code, code_verifier: verifier, scope: OUTLOOK_WEB_SCOPES }).toString(),
    redirect: "manual",
    signal: AbortSignal.timeout(20_000),
  });
  ruota(s, t, adessoS);
  const json = (await t.json().catch(() => ({}))) as Record<string, unknown>;
  if (t.status === 429 || t.status >= 500) throw new Error(`Scambio codice Microsoft non disponibile (HTTP ${t.status})`);
  if (!t.ok || typeof json.access_token !== "string") throw new SessioneScaduta("Microsoft chiede di accedere di nuovo da Outlook web");
  return json;
}
