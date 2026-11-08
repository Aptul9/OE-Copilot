// Lettura diretta dei calendari via API (Outlook REST v2.0 e Google Calendar v3), senza emulatore Android.
// Usa i token di aggiornamento in tokens.json (OAuth2 con client_id di Outlook Mobile per Microsoft, master token aas_et/ o refresh_token per Google)
// e rinnova da sé gli access token quando scadono, salvando su disco il nuovo refresh_token.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { Contesto, Evento, Stato } from "./fonte.ts";
import { pulisciLink } from "./html.ts";
import { MEETING_URL } from "./ics.ts";
import { confrontaPy, pyStrip } from "./pyfmt.ts";
import { contieneEvento } from "./serie.ts";
import { ErroreAltro, ora, testoNaive } from "./tempo.ts";
import { MS_LOGIN, OUTLOOK_REST, claimsJwt, jsonRest, leggiTesto, motivoTokenWebNonValido, upnDi, type FetchFn } from "./outlook-rest.ts";
import { OUTLOOK_WEB_CLIENT_ID, OUTLOOK_WEB_REDIRECT_URI, OUTLOOK_WEB_SCOPES, SessioneScaduta, autorizzaDaSessione, leggiSessione, type SessioneMicrosoft } from "./sessione-web.ts";

export type { FetchFn } from "./outlook-rest.ts";

// Client ID e redirect URI estratti da com.microsoft.office.outlook 5.2641.2 (TokenRestApi.AAD_CLIENT_ID e AuthUtil.getOneAuthRedirectUri)
export const OUTLOOK_CLIENT_ID = "27922004-5251-4030-b22d-91ecd9a37ea4";
export const OUTLOOK_REDIRECT_URI = "msauth://com.microsoft.office.outlook/fcg80qvoM1YMKJZibjBwQcDfOno%3D";
export const OUTLOOK_SCOPES = "https://outlook.office.com/.default offline_access openid profile";

// Parametri Google Calendar SyncAdapter (com.google.android.syncadapters.calendar)
export const GOOGLE_CALENDAR_APP = "com.google.android.syncadapters.calendar";
export const GOOGLE_CLIENT_SIG = "38918a453d07199354f8b19af05ec6562ced5788";
export const GOOGLE_CALENDAR_SERVICE = "oauth2:https://www.googleapis.com/auth/calendar";

// Parametri OAuth2 PKCE Desktop per accesso Google da browser senza emulatore
export const GOOGLE_OAUTH_CLIENT_ID = "406964657835-aq8lmia8j95dhl1a2bvharmfk3t1hgqj.apps.googleusercontent.com";
export const GOOGLE_OAUTH_CLIENT_SECRET = "kSmqreRr0qwBWJgbf5Y-PjSU";
export const GOOGLE_OAUTH_REDIRECT_URI = "http://localhost";
export const GOOGLE_OAUTH_SCOPES = "https://www.googleapis.com/auth/calendar openid email";
export const GOOGLE_TOKEN_MAX_BYTES = 64 * 1024;

const MARGINE_SCADENZA_S = 120;
const TTL_CALENDARI_S = 300;
const MAX_PKCE_IN_ATTESA = 20;
const TTL_PKCE_S = 900;

export class ErroreAccessoScaduto extends Error {}

export interface VoceToken {
  provider: "microsoft" | "google";
  client_id?: string;
  client_secret?: string;
  tenant_id?: string;
  refresh_token?: string;
  master_token?: string;
  access_token?: string;
  expires_at?: number;
  scopes?: string;
  errore?: string;
  /** Accesso da sessione del browser (client web di Outlook): i cookie ESTS per riautorizzare quando il refresh_token non basta più. */
  sessione?: SessioneMicrosoft;
}

export interface ArchivioToken {
  accounts: Record<string, VoceToken>;
  /** Le caselle tolte dalla pagina (Rimuovi account): anche se fonti.json le elenca non si leggono, finché non si rifà l'accesso. */
  rimossi?: string[];
}

export interface TokenPostaInfo {
  upn: string;
  token: string;
  exp: number;
  endpoint_type: "outlook_rest" | "graph";
  base_url: string;
  scopes: string;
  phone_status: string;
  phone_msg: string;
}

/** Una riga di token-outlook.json e di GET /api/token-posta (il broker dei token di prima, che resta). */
function infoPosta(upn: string, voce: VoceToken, token: string, exp: number, errore: boolean, msg: string): TokenPostaInfo {
  return {
    upn,
    token,
    exp: Math.trunc(exp),
    endpoint_type: "outlook_rest",
    base_url: OUTLOOK_REST,
    scopes: voce.scopes ?? OUTLOOK_SCOPES,
    phone_status: errore ? "errore" : "ok",
    phone_msg: msg,
  };
}

function scriviJsonAtomico(percorso: string, dati: unknown): void {
  const dir = path.dirname(percorso);
  fs.mkdirSync(dir, { recursive: true });
  const tmp = `${percorso}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(dati, null, 2) + "\n", { encoding: "utf-8", mode: 0o600 });
  fs.renameSync(tmp, percorso);
}

function pesoMaggiore(a: boolean[], b: boolean[]): boolean {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return Number(a[i]) > Number(b[i]);
  return false;
}

function estraiCodiceDaInput(codiceOUrl: string): { code: string; state: string | null } {
  const pulito = codiceOUrl.trim();
  if (!pulito) throw new Error("Manca l'indirizzo o il codice di autorizzazione");
  if (pulito.includes("://") || pulito.startsWith("?") || pulito.includes("code=") || pulito.includes("error=")) {
    const qIndex = pulito.indexOf("?");
    const hIndex = pulito.indexOf("#");
    const queryStr = qIndex >= 0 ? pulito.slice(qIndex + 1, hIndex > qIndex ? hIndex : undefined) : hIndex >= 0 ? pulito.slice(hIndex + 1) : pulito;
    const params = new URLSearchParams(queryStr);
    const err = params.get("error_description") || params.get("error");
    if (err) throw new Error(`Accesso rifiutato: ${err}`);
    const code = params.get("code");
    if (!code) throw new Error("L'indirizzo incollato non contiene il parametro code=...");
    return { code: code.trim(), state: params.get("state") };
  }
  return { code: pulito, state: null };
}

export class GestoreToken {
  readonly percorso: string;
  readonly tokenPostaPath: string | null;
  private readonly adesso: () => number;
  private readonly strictPersistence: boolean;
  /** Lo stesso fetch per i rinnovi e per il gateway di posta e calendari (comunicazione.ts). */
  readonly fetchFn: FetchFn;
  private mtimeMs = -1;
  private dati: ArchivioToken = { accounts: {} };
  private readonly inCorso = new Map<string, Promise<string>>();
  private readonly pkceInAttesa = new Map<string, { verifier: string; provider: "microsoft" | "google"; email?: string; quandoS: number }>();
  private readonly deviceInAttesa = new Map<string, { deviceCode: string; quandoS: number }>();

  constructor(percorso: string, opzioni: { tokenPostaPath?: string | null; adesso?: () => number; fetchFn?: FetchFn; strictPersistence?: boolean } = {}) {
    this.percorso = percorso;
    this.tokenPostaPath = opzioni.tokenPostaPath ?? null;
    this.adesso = opzioni.adesso ?? Date.now;
    this.strictPersistence = opzioni.strictPersistence ?? false;
    this.fetchFn = opzioni.fetchFn ?? fetch;
  }

  private adessoS(): number {
    return Math.floor(this.adesso() / 1000);
  }

  private caricaSeCambiato(): void {
    try {
      if (!fs.existsSync(this.percorso)) {
        if (this.mtimeMs !== 0) {
          this.dati = { accounts: {} };
          this.mtimeMs = 0;
        }
        return;
      }
      const st = fs.statSync(this.percorso);
      if (st.mtimeMs === this.mtimeMs) return;
      const raw = JSON.parse(fs.readFileSync(this.percorso, "utf-8")) as unknown;
      if (raw !== null && typeof raw === "object" && !Array.isArray(raw)) {
        const obj = raw as Record<string, unknown>;
        const accs = obj.accounts !== null && typeof obj.accounts === "object" && !Array.isArray(obj.accounts) ? (obj.accounts as Record<string, VoceToken>) : {};
        const normalizzati: Record<string, VoceToken> = {};
        for (const [k, v] of Object.entries(accs)) {
          if (v && typeof v === "object") normalizzati[k.toLowerCase()] = { ...v };
        }
        const rimossi = Array.isArray(obj.rimossi) ? obj.rimossi.filter((x): x is string => typeof x === "string").map((x) => x.toLowerCase()) : [];
        this.dati = rimossi.length ? { accounts: normalizzati, rimossi } : { accounts: normalizzati };
      }
      this.mtimeMs = st.mtimeMs;
    } catch (error) {
      // Mantiene i dati in memoria se il file è in corso di scrittura
      if (this.strictPersistence) throw error;
    }
  }

  private salva(): void {
    try {
      scriviJsonAtomico(this.percorso, this.dati);
      this.mtimeMs = fs.statSync(this.percorso).mtimeMs;
    } catch (error) {
      // In ambiente a sola lettura i token rinnovati restano in memoria
      if (this.strictPersistence) throw error;
    }
    this.scriviTokenPosta();
  }

  private scriviTokenPosta(): void {
    if (!this.tokenPostaPath) return;
    try {
      const adessoS = this.adessoS();
      const account: TokenPostaInfo[] = [];
      for (const [upn, v] of Object.entries(this.dati.accounts)) {
        if (v.provider !== "microsoft" || !v.access_token || (v.expires_at ?? 0) <= adessoS) continue;
        account.push(infoPosta(upn, v, v.access_token, v.expires_at ?? 0, Boolean(v.errore), v.errore ?? ""));
      }
      scriviJsonAtomico(this.tokenPostaPath, { generato: adessoS, account });
    } catch {
      // Facoltativo
    }
  }

  haAccount(email: string): boolean {
    this.caricaSeCambiato();
    return Object.hasOwn(this.dati.accounts, email.toLowerCase());
  }

  voceAccount(email: string): VoceToken | undefined {
    this.caricaSeCambiato();
    return this.dati.accounts[email.toLowerCase()];
  }

  elencoAccount(): Record<string, VoceToken> {
    this.caricaSeCambiato();
    return this.dati.accounts;
  }

  rimosso(email: string): boolean {
    this.caricaSeCambiato();
    return (this.dati.rimossi ?? []).includes(email.toLowerCase());
  }

  /** Rimuovi account: via l'accesso salvato, e la casella resta fuori anche da fonti.json. */
  rimuovi(email: string): void {
    const chiave = email.toLowerCase();
    this.caricaSeCambiato();
    delete this.dati.accounts[chiave];
    this.inCorso.delete(chiave);
    this.dati.rimossi = [...new Set([...(this.dati.rimossi ?? []), chiave])];
    this.salva();
  }

  /** Un nuovo accesso riporta dentro una casella rimossa. */
  riammetti(email: string): void {
    const chiave = email.toLowerCase();
    this.caricaSeCambiato();
    if (!this.dati.rimossi?.includes(chiave)) return;
    const resto = this.dati.rimossi.filter((x) => x !== chiave);
    if (resto.length) this.dati.rimossi = resto;
    else delete this.dati.rimossi;
    this.salva();
  }

  async ottieniAccessToken(email: string, forza = false): Promise<string> {
    const chiave = email.toLowerCase();
    this.caricaSeCambiato();
    const voce = this.dati.accounts[chiave];
    if (!voce) {
      throw new ErroreAccessoScaduto(`La casella ${email} non ha un accesso configurato (Accedi di nuovo nella finestra Account)`);
    }
    const adessoS = this.adessoS();
    if (!forza && voce.access_token && (voce.expires_at ?? 0) > adessoS + MARGINE_SCADENZA_S) {
      return voce.access_token;
    }
    const pendente = this.inCorso.get(chiave);
    if (pendente) return pendente;
    const promessa = (voce.provider === "google" ? this.rinnovaGoogle(chiave, voce) : this.rinnovaMicrosoft(chiave, voce)).finally(() => {
      this.inCorso.delete(chiave);
    });
    this.inCorso.set(chiave, promessa);
    return promessa;
  }

  private async rinnovaMicrosoft(chiave: string, voce: VoceToken): Promise<string> {
    if (!voce.refresh_token) {
      if (voce.sessione) return this.rinnovaDaSessione(chiave, voce);
      voce.errore = "Accesso scaduto: Accedi di nuovo nella finestra Account";
      this.salva();
      throw new ErroreAccessoScaduto(voce.errore);
    }
    const web = voce.client_id === OUTLOOK_WEB_CLIENT_ID;
    const tenant = voce.tenant_id || "common";
    const url = `${MS_LOGIN}/${encodeURIComponent(tenant)}/oauth2/v2.0/token`;
    const body = new URLSearchParams({
      client_id: voce.client_id || OUTLOOK_CLIENT_ID,
      grant_type: "refresh_token",
      refresh_token: voce.refresh_token,
      scope: web ? OUTLOOK_WEB_SCOPES : OUTLOOK_SCOPES,
    }).toString();

    // Il refresh_token del client web (SPA) si riscatta solo da una richiesta cross-origin: serve l'Origin di Outlook web.
    const headers: Record<string, string> = { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" };
    if (web) headers.Origin = new URL(OUTLOOK_WEB_REDIRECT_URI).origin;
    const resp = await this.fetchFn(url, {
      method: "POST",
      headers,
      body,
      signal: AbortSignal.timeout(15_000),
    });
    const testo = await resp.text();
    let json: Record<string, unknown> = {};
    try {
      json = JSON.parse(testo) as Record<string, unknown>;
    } catch {
      // Ignora
    }
    if (!resp.ok || typeof json.access_token !== "string") {
      const code = String(json.error ?? "");
      if (resp.status === 400 || resp.status === 401 || code === "invalid_grant" || code === "interaction_required") {
        // Il refresh_token SPA dura 24 ore al massimo: la sessione del browser ne dà uno nuovo.
        if (voce.sessione) return this.rinnovaDaSessione(chiave, voce);
        voce.errore = "Accesso scaduto: Accedi di nuovo nella finestra Account";
        this.salva();
        throw new ErroreAccessoScaduto(voce.errore);
      }
      throw new Error(`Rinnovo token Microsoft fallito (HTTP ${resp.status}: ${String(json.error_description || json.error || testo).slice(0, 160)})`);
    }
    if (web) {
      // Il token del client web vale solo se è della stessa casella, per Outlook, non scaduto e con i calendari, come all'importazione.
      // Uno che non torna non si salva, nemmeno il suo refresh_token: si riprova dalla sessione del browser, se c'è.
      const motivo = motivoTokenWebNonValido(json.access_token, chiave, this.adessoS());
      if (motivo) {
        if (voce.sessione) return this.rinnovaDaSessione(chiave, voce);
        voce.errore = `Rinnovo di Outlook web rifiutato: ${motivo}`;
        this.salva();
        throw new ErroreAccessoScaduto(voce.errore);
      }
    }
    return this.applicaTokenMicrosoft(voce, json);
  }

  /** Riautorizza con i cookie della sessione del browser. I cookie ruotati si salvano anche quando Microsoft rifiuta o non risponde. */
  private async rinnovaDaSessione(chiave: string, voce: VoceToken): Promise<string> {
    // La sessione salvata ripassa dal filtro dell'importazione prima che un cookie esca: solo i cookie di login.microsoftonline.com, e
    // solo per la casella a cui appartiene.
    const sessione = leggiSessione(voce.sessione);
    if (!sessione || sessione.loginHint.trim().toLowerCase() !== chiave) {
      voce.errore = "Sessione del browser non valida: accedi di nuovo da Outlook web e importa la sessione";
      this.salva();
      throw new ErroreAccessoScaduto(voce.errore);
    }
    // autorizzaDaSessione ruota i cookie su questo oggetto: sta già nella voce, così ogni salvataggio da qui in poi li tiene
    voce.sessione = sessione;
    let json: Record<string, unknown>;
    try {
      json = await autorizzaDaSessione(sessione, voce.tenant_id || "organizations", { fetchFn: this.fetchFn, adessoS: this.adessoS() });
    } catch (e) {
      if (e instanceof SessioneScaduta) {
        voce.errore = "Sessione del browser scaduta: accedi di nuovo da Outlook web e importa la sessione";
        this.salva();
        throw new ErroreAccessoScaduto(voce.errore);
      }
      this.salva();
      throw e;
    }
    const motivo = motivoTokenWebNonValido(json.access_token, chiave, this.adessoS());
    if (motivo) {
      voce.errore = `Accesso dalla sessione del browser rifiutato: ${motivo}`;
      this.salva();
      throw new ErroreAccessoScaduto(voce.errore);
    }
    return this.applicaTokenMicrosoft(voce, json);
  }

  private applicaTokenMicrosoft(voce: VoceToken, json: Record<string, unknown>): string {
    if (typeof json.access_token !== "string") throw new Error("Risposta token Microsoft senza access_token");
    const adessoS = this.adessoS();
    const claims = claimsJwt(json.access_token);
    const expClaim = Number(claims.exp ?? 0);
    const expIn = Number(json.expires_in ?? 3600);
    voce.access_token = json.access_token;
    voce.expires_at = expClaim > adessoS ? expClaim : adessoS + (Number.isFinite(expIn) ? expIn : 3600);
    if (typeof json.refresh_token === "string" && json.refresh_token) {
      voce.refresh_token = json.refresh_token;
    }
    if (typeof claims.tid === "string" && claims.tid) {
      voce.tenant_id = claims.tid;
    }
    if (typeof claims.scp === "string" && claims.scp) {
      voce.scopes = claims.scp;
    }
    delete voce.errore;
    this.salva();
    return voce.access_token;
  }

  private async rinnovaGoogle(chiave: string, voce: VoceToken): Promise<string> {
    if (voce.master_token) {
      const body = new URLSearchParams({
        accountType: "HOSTED_OR_GOOGLE",
        Email: chiave,
        has_permission: "1",
        EncryptedPasswd: voce.master_token,
        service: GOOGLE_CALENDAR_SERVICE,
        source: "android",
        app: GOOGLE_CALENDAR_APP,
        client_sig: GOOGLE_CLIENT_SIG,
        device_country: "it",
        operatorCountry: "it",
        lang: "it",
        sdk_version: "33",
      }).toString();
      const resp = await this.fetchFn("https://android.clients.google.com/auth", {
        method: "POST",
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
          "User-Agent": "GoogleAuth/1.4 (Linux; U; Android 13; it-it); gzip",
        },
        body,
        signal: AbortSignal.timeout(15_000),
      });
      const testo = await resp.text();
      const righe = Object.fromEntries(
        testo
          .split(/\r?\n/)
          .filter((l) => l.includes("="))
          .map((l) => {
            const idx = l.indexOf("=");
            return [l.slice(0, idx), l.slice(idx + 1)];
          }),
      ) as Record<string, string>;
      if (!resp.ok || !righe.Auth) {
        const err = righe.Error || `HTTP ${resp.status}`;
        if (resp.status === 401 || resp.status === 403 || err === "BadAuthentication" || err === "NeedsBrowser") {
          voce.errore = "Accesso Google scaduto: Accedi di nuovo nella finestra Account";
          this.salva();
          throw new ErroreAccessoScaduto(voce.errore);
        }
        throw new Error(`Rinnovo token Google fallito (${err})`);
      }
      const adessoS = this.adessoS();
      const durata = Number(righe.ExpiresInDurationSec ?? 3600);
      voce.access_token = righe.Auth;
      voce.expires_at = adessoS + (Number.isFinite(durata) && durata > 0 ? durata : 3600);
      delete voce.errore;
      this.salva();
      return voce.access_token;
    }
    if (voce.refresh_token && voce.client_id) {
      const params: Record<string, string> = {
        client_id: voce.client_id,
        grant_type: "refresh_token",
        refresh_token: voce.refresh_token,
      };
      if (voce.client_secret) params.client_secret = voce.client_secret;
      const resp = await this.fetchFn("https://oauth2.googleapis.com/token", {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
        body: new URLSearchParams(params).toString(),
        redirect: "manual",
        signal: AbortSignal.timeout(15_000),
      });
      const json = (this.strictPersistence
        ? JSON.parse(await leggiTesto(resp, GOOGLE_TOKEN_MAX_BYTES))
        : await resp.json().catch(() => ({}))) as Record<string, unknown>;
      if (this.strictPersistence) {
        this.caricaSeCambiato();
        if (this.dati.accounts[chiave] !== voce) throw new ErroreAccessoScaduto("Google access changed during refresh; retry request");
      }
      if (!resp.ok || typeof json.access_token !== "string") {
        voce.errore = "Accesso Google scaduto: Accedi di nuovo nella finestra Account";
        this.salva();
        throw new ErroreAccessoScaduto(voce.errore);
      }
      const adessoS = this.adessoS();
      const durata = Number(json.expires_in ?? 3600);
      voce.access_token = json.access_token;
      voce.expires_at = adessoS + (Number.isFinite(durata) && durata > 0 ? durata : 3600);
      if (typeof json.refresh_token === "string" && json.refresh_token) voce.refresh_token = json.refresh_token;
      delete voce.errore;
      this.salva();
      return voce.access_token;
    }
    voce.errore = "Accesso Google mancante: Accedi di nuovo nella finestra Account";
    this.salva();
    throw new ErroreAccessoScaduto(voce.errore);
  }

  /** Restituisce i token Microsoft attivi (rinnovando quelli in scadenza) per GET /api/token-posta. */
  async tokenPostaAttivi(): Promise<TokenPostaInfo[]> {
    this.caricaSeCambiato();
    const out: TokenPostaInfo[] = [];
    for (const [upn, voce] of Object.entries(this.dati.accounts)) {
      if (voce.provider !== "microsoft") continue;
      try {
        const tok = await this.ottieniAccessToken(upn);
        out.push(infoPosta(upn, voce, tok, voce.expires_at ?? this.adessoS() + 3600, false, ""));
      } catch (e) {
        if (voce.access_token && (voce.expires_at ?? 0) > this.adessoS()) {
          out.push(infoPosta(upn, voce, voce.access_token, voce.expires_at ?? 0, true, e instanceof Error ? e.message : String(e)));
        }
      }
    }
    return out;
  }

  private preparaPkce(provider: "microsoft" | "google", email?: string): { verifier: string; challenge: string; state: string } {
    const adessoS = this.adessoS();
    for (const [k, v] of this.pkceInAttesa.entries()) {
      if (adessoS - v.quandoS > TTL_PKCE_S) this.pkceInAttesa.delete(k);
    }
    if (this.pkceInAttesa.size >= MAX_PKCE_IN_ATTESA) {
      const primo = this.pkceInAttesa.keys().next().value;
      if (primo) this.pkceInAttesa.delete(primo);
    }
    const verifier = crypto.randomBytes(32).toString("base64url");
    const challenge = crypto.createHash("sha256").update(verifier).digest("base64url");
    const state = crypto.randomBytes(16).toString("base64url");
    this.pkceInAttesa.set(state, { verifier, provider, email: email?.trim(), quandoS: adessoS });
    return { verifier, challenge, state };
  }

  /** Avvia il flusso OAuth2 PKCE di Outlook Mobile e restituisce l'indirizzo di login Microsoft. */
  iniziaPkce(email?: string): { url: string; state: string } {
    const { challenge, state } = this.preparaPkce("microsoft", email);
    const params = new URLSearchParams({
      client_id: OUTLOOK_CLIENT_ID,
      response_type: "code",
      redirect_uri: OUTLOOK_REDIRECT_URI,
      response_mode: "query",
      scope: OUTLOOK_SCOPES,
      code_challenge: challenge,
      code_challenge_method: "S256",
      state,
      prompt: "select_account",
      nux: "1",
      msafed: "0",
    });
    if (email && email.trim()) {
      params.set("login_hint", email.trim());
    }
    return {
      url: `${MS_LOGIN}/common/oauth2/v2.0/authorize?${params.toString()}`,
      state,
    };
  }

  /** Avvia il flusso OAuth2 Device Code di Outlook Mobile (zero copia-incolla di URL: codice + polling automatico). */
  async iniziaDeviceMicrosoft(email?: string): Promise<{ url: string; user_code: string; state: string; intervallo_s: number; url_pkce: string }> {
    const { url: url_pkce, state } = this.iniziaPkce(email);
    const adessoS = this.adessoS();
    for (const [k, v] of this.deviceInAttesa.entries()) {
      if (adessoS - v.quandoS > TTL_PKCE_S) this.deviceInAttesa.delete(k);
    }
    const resp = await this.fetchFn(`${MS_LOGIN}/common/oauth2/v2.0/devicecode`, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
      body: new URLSearchParams({
        client_id: OUTLOOK_CLIENT_ID,
        scope: OUTLOOK_SCOPES,
      }).toString(),
      signal: AbortSignal.timeout(15_000),
    });
    const json = (await resp.json().catch(() => ({}))) as Record<string, unknown>;
    if (!resp.ok || typeof json.device_code !== "string" || typeof json.user_code !== "string") {
      throw new Error(`Avvio codice dispositivo Microsoft fallito: ${String(json.error_description || json.error || `HTTP ${resp.status}`)}`);
    }
    this.deviceInAttesa.set(state, { deviceCode: json.device_code, quandoS: adessoS });
    const urlBase = typeof json.verification_uri === "string" && json.verification_uri ? json.verification_uri : "https://microsoft.com/devicelogin";
    const intervallo = Number(json.interval ?? 5);
    return {
      url: urlBase,
      user_code: json.user_code,
      state,
      intervallo_s: Number.isFinite(intervallo) && intervallo > 0 ? intervallo : 5,
      url_pkce,
    };
  }

  /** Controlla se l'utente ha completato il login Microsoft Device Code nella scheda del browser. */
  async controllaDeviceMicrosoft(state: string): Promise<{ stato: "in_attesa" } | { stato: "completato"; account: string; tenant_id: string }> {
    const attesa = this.deviceInAttesa.get(state);
    if (!attesa) {
      throw new Error("Sessione di accesso scaduta o non trovata: clicca di nuovo su 'Accedi'");
    }
    const resp = await this.fetchFn(`${MS_LOGIN}/common/oauth2/v2.0/token`, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
      body: new URLSearchParams({
        client_id: OUTLOOK_CLIENT_ID,
        grant_type: "urn:ietf:params:oauth:grant-type:device_code",
        device_code: attesa.deviceCode,
      }).toString(),
      signal: AbortSignal.timeout(15_000),
    });
    const json = (await resp.json().catch(() => ({}))) as Record<string, unknown>;
    const err = String(json.error ?? "");
    if (!resp.ok && (err === "authorization_pending" || err === "slow_down")) {
      return { stato: "in_attesa" };
    }
    this.deviceInAttesa.delete(state);
    this.pkceInAttesa.delete(state);
    if (!resp.ok || typeof json.access_token !== "string") {
      throw new Error(`Accesso Microsoft non riuscito: ${String(json.error_description || json.error || `HTTP ${resp.status}`)}`);
    }
    return this.salvaRispostaTokenMicrosoft(json);
  }

  private salvaRispostaTokenMicrosoft(json: Record<string, unknown>): { stato: "completato"; account: string; tenant_id: string } {
    const claims = {
      ...claimsJwt(typeof json.id_token === "string" ? json.id_token : ""),
      ...claimsJwt(String(json.access_token ?? "")),
    };
    const upn = String(claims.upn || claims.preferred_username || claims.unique_name || claims.email || "").trim().toLowerCase();
    if (!upn) {
      throw new Error("Impossibile determinare l'indirizzo email dal token Microsoft");
    }
    const tid = String(claims.tid || "common");
    const adessoS = this.adessoS();
    const expClaim = Number(claims.exp ?? 0);
    const expIn = Number(json.expires_in ?? 3600);

    this.caricaSeCambiato();
    this.dati.accounts[upn] = {
      provider: "microsoft",
      client_id: OUTLOOK_CLIENT_ID,
      tenant_id: tid,
      refresh_token: typeof json.refresh_token === "string" ? json.refresh_token : this.dati.accounts[upn]?.refresh_token,
      access_token: String(json.access_token),
      expires_at: expClaim > adessoS ? expClaim : adessoS + (Number.isFinite(expIn) ? expIn : 3600),
      scopes: typeof claims.scp === "string" ? claims.scp : OUTLOOK_SCOPES,
    };
    this.salva();
    return { stato: "completato", account: upn, tenant_id: tid };
  }

  /**
   * Importa la sessione del browser (formato teamsrelay session.json) per le caselle dove Outlook Mobile è respinto dall'accesso
   * condizionale. Prima di salvare prova l'autorizzazione: casella, risorsa Outlook e permesso di posta devono tornare.
   */
  async importaSessione(dati: unknown, tenant: string): Promise<{ account: string; tenant_id: string }> {
    const sessione = leggiSessione(dati);
    if (!sessione) throw new Error("Sessione del browser non valida (servono userAgent, loginHint e cookies)");
    const atteso = sessione.loginHint.trim().toLowerCase();
    if (!atteso.includes("@")) throw new Error("Nella sessione manca loginHint con l'indirizzo della casella");
    let json: Record<string, unknown>;
    try {
      json = await autorizzaDaSessione(sessione, tenant.trim() || "organizations", { fetchFn: this.fetchFn, adessoS: this.adessoS() });
    } catch (e) {
      if (e instanceof SessioneScaduta) throw new Error(`Accesso Microsoft non riuscito: ${e.message}`);
      throw e;
    }
    // stesso controllo di ogni rinnovo del client web: casella, risorsa Outlook, scadenza, calendari
    const motivo = motivoTokenWebNonValido(json.access_token, atteso, this.adessoS());
    if (motivo) throw new Error(motivo);
    const claims = claimsJwt(String(json.access_token));
    const upn = upnDi(claims);
    const tid = String(claims.tid || tenant);
    this.caricaSeCambiato();
    const voce: VoceToken = { provider: "microsoft", client_id: OUTLOOK_WEB_CLIENT_ID, tenant_id: tid, sessione };
    this.dati.accounts[upn] = voce;
    this.applicaTokenMicrosoft(voce, json);
    return { account: upn, tenant_id: tid };
  }

  /** Completa il login OAuth2 PKCE di Outlook Mobile con l'indirizzo msauth://... o il codice ricevuto. */
  async completaPkce(codiceOUrl: string, stateDato?: string): Promise<{ account: string; tenant_id: string }> {
    const { code, state } = estraiCodiceDaInput(codiceOUrl);
    const candidati = [...this.pkceInAttesa.entries()].filter(([, v]) => v.provider === "microsoft").map(([k]) => k);
    const chiaveState = state || stateDato || candidati.pop() || "";
    const attesa = this.pkceInAttesa.get(chiaveState);
    if (!attesa) {
      throw new Error("Sessione di accesso scaduta o non trovata: clicca di nuovo su 'Accedi con Microsoft'");
    }
    this.pkceInAttesa.delete(chiaveState);
    this.deviceInAttesa.delete(chiaveState);

    const body = new URLSearchParams({
      client_id: OUTLOOK_CLIENT_ID,
      grant_type: "authorization_code",
      code,
      redirect_uri: OUTLOOK_REDIRECT_URI,
      code_verifier: attesa.verifier,
      scope: OUTLOOK_SCOPES,
    }).toString();

    const resp = await this.fetchFn(`${MS_LOGIN}/common/oauth2/v2.0/token`, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
      body,
      signal: AbortSignal.timeout(15_000),
    });
    const json = (await resp.json().catch(() => ({}))) as Record<string, unknown>;
    if (!resp.ok || typeof json.access_token !== "string") {
      throw new Error(`Scambio codice Microsoft fallito: ${String(json.error_description || json.error || `HTTP ${resp.status}`)}`);
    }
    const { account, tenant_id } = this.salvaRispostaTokenMicrosoft(json);
    return { account, tenant_id };
  }

  /** Avvia il flusso OAuth2 PKCE per Google Calendar e restituisce l'indirizzo di login Google. */
  iniziaGooglePkce(email?: string): { url: string; state: string } {
    const { challenge, state } = this.preparaPkce("google", email);
    const params = new URLSearchParams({
      client_id: GOOGLE_OAUTH_CLIENT_ID,
      redirect_uri: GOOGLE_OAUTH_REDIRECT_URI,
      response_type: "code",
      scope: GOOGLE_OAUTH_SCOPES,
      access_type: "offline",
      prompt: "consent",
      code_challenge: challenge,
      code_challenge_method: "S256",
      state,
    });
    if (email && email.trim()) {
      params.set("login_hint", email.trim());
    }
    return {
      url: `https://accounts.google.com/o/oauth2/v2/auth?${params.toString()}`,
      state,
    };
  }

  /** Completa il login OAuth2 PKCE di Google Calendar con l'indirizzo http://localhost/?code=... o il codice ricevuto. */
  async completaGooglePkce(codiceOUrl: string, stateDato?: string, emailDato?: string): Promise<{ account: string }> {
    const { code, state } = estraiCodiceDaInput(codiceOUrl);
    const candidati = [...this.pkceInAttesa.entries()].filter(([, v]) => v.provider === "google").map(([k]) => k);
    const chiaveState = state || stateDato || candidati.pop() || "";
    const attesa = this.pkceInAttesa.get(chiaveState);
    if (!attesa) {
      throw new Error("Sessione di accesso Google scaduta o non trovata: clicca di nuovo su 'Accedi con Google'");
    }
    this.pkceInAttesa.delete(chiaveState);

    const body = new URLSearchParams({
      client_id: GOOGLE_OAUTH_CLIENT_ID,
      client_secret: GOOGLE_OAUTH_CLIENT_SECRET,
      grant_type: "authorization_code",
      code,
      redirect_uri: GOOGLE_OAUTH_REDIRECT_URI,
      code_verifier: attesa.verifier,
    }).toString();

    const resp = await this.fetchFn("https://oauth2.googleapis.com/token", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
      body,
      signal: AbortSignal.timeout(15_000),
    });
    const json = (await resp.json().catch(() => ({}))) as Record<string, unknown>;
    if (!resp.ok || typeof json.access_token !== "string") {
      throw new Error(`Scambio codice Google fallito: ${String(json.error_description || json.error || `HTTP ${resp.status}`)}`);
    }

    const claims = claimsJwt(typeof json.id_token === "string" ? json.id_token : "");
    let email = String(claims.email || emailDato || attesa.email || "").trim().toLowerCase();
    if (!email) {
      const uResp = await this.fetchFn("https://openidconnect.googleapis.com/v1/userinfo", {
        headers: { Authorization: `Bearer ${json.access_token}`, Accept: "application/json" },
        signal: AbortSignal.timeout(10_000),
      });
      const uJson = (await uResp.json().catch(() => ({}))) as Record<string, unknown>;
      email = String(uJson.email || "").trim().toLowerCase();
    }
    if (!email || !email.includes("@")) {
      throw new Error("Impossibile determinare l'indirizzo email dal login Google");
    }

    const adessoS = this.adessoS();
    const durata = Number(json.expires_in ?? 3600);
    this.caricaSeCambiato();
    const precedente = this.dati.accounts[email];
    this.dati.accounts[email] = {
      provider: "google",
      client_id: GOOGLE_OAUTH_CLIENT_ID,
      client_secret: GOOGLE_OAUTH_CLIENT_SECRET,
      refresh_token: typeof json.refresh_token === "string" && json.refresh_token ? json.refresh_token : precedente?.refresh_token,
      access_token: json.access_token,
      expires_at: adessoS + (Number.isFinite(durata) && durata > 0 ? durata : 3600),
      scopes: GOOGLE_OAUTH_SCOPES,
    };
    this.salva();
    return { account: email };
  }

  /** Salva e verifica un master token (aas_et/...) o access token per una casella Google. */
  async salvaGoogle(email: string, token: string): Promise<{ account: string }> {
    const pulita = email.trim().toLowerCase();
    const tok = token.trim();
    if (!pulita || !pulita.includes("@")) throw new Error("Indirizzo email Google non valido");
    if (!tok) throw new Error("Token Google mancante");
    this.caricaSeCambiato();
    const precedente = this.dati.accounts[pulita];
    const nuova: VoceToken = tok.startsWith("ya29.")
      ? { provider: "google", master_token: precedente?.master_token, access_token: tok, expires_at: this.adessoS() + 3500 }
      : { provider: "google", master_token: tok };
    this.dati.accounts[pulita] = nuova;
    try {
      await this.ottieniAccessToken(pulita, !tok.startsWith("ya29."));
    } catch (e) {
      if (precedente) this.dati.accounts[pulita] = precedente;
      else delete this.dati.accounts[pulita];
      throw e;
    }
    this.salva();
    return { account: pulita };
  }
}

interface CalInfo {
  id: string;
  nome: string;
}

export class CalendarioDiretto {
  readonly id: string;
  readonly nome: string;
  readonly account: string;
  readonly calendari: string[];
  private readonly ctx: Contesto;
  private readonly tokens: GestoreToken;
  private readonly fetchFn: FetchFn;
  stato: Stato = { stato: "avviso", msg: "Non ancora letto" };
  tipo: string | null = null;
  conteggio: number | null = null;
  private cacheCalendari: { quandoS: number; scelti: CalInfo[] } | null = null;

  constructor(
    id: string,
    nome: string,
    account: string,
    calendari: string[] | null | undefined,
    ctx: Contesto,
    tokens: GestoreToken,
    fetchFn: FetchFn = fetch,
  ) {
    this.id = id;
    this.nome = nome;
    this.account = pyStrip(account).toLowerCase();
    this.calendari = (calendari ?? []).map((c) => pyStrip(c).toLowerCase());
    this.ctx = ctx;
    this.tokens = tokens;
    this.fetchFn = fetchFn;
    const voce = this.tokens.voceAccount(this.account);
    if (voce) {
      this.tipo = voce.provider === "google" ? "caldav" : "outlook";
    }
  }

  svuotaCache(): void {
    this.cacheCalendari = null;
  }

  /** Lo stesso trasporto del gateway di posta e calendari (outlook-rest.ts): un 401 rinnova il token una volta e riprova. */
  private richiestaJson(url: string, headers: Record<string, string>): Promise<Record<string, unknown>> {
    return jsonRest(this.tokens, this.account, url, { headers, fetchFn: this.fetchFn });
  }

  private async calendariScelti(forza = false): Promise<CalInfo[]> {
    const adessoS = Math.floor(this.ctx.adesso() / 1000);
    if (!forza && this.cacheCalendari && adessoS - this.cacheCalendari.quandoS < TTL_CALENDARI_S) {
      return this.cacheCalendari.scelti;
    }
    const voce = this.tokens.voceAccount(this.account);
    if (!voce) {
      throw new ErroreAccessoScaduto(`La casella ${this.account} non ha credenziali salvate (Accedi di nuovo nella finestra Account)`);
    }
    this.tipo = voce.provider === "google" ? "caldav" : "outlook";
    const scelti: CalInfo[] = [];

    if (voce.provider === "google") {
      const json = await this.richiestaJson("https://www.googleapis.com/calendar/v3/users/me/calendarList?maxResults=250", {});
      const items = Array.isArray(json.items) ? (json.items as Record<string, unknown>[]) : [];
      const tutti: CalInfo[] = [];
      for (const it of items) {
        const id = String(it.id ?? "");
        const summary = String(it.summary ?? id);
        if (!id) continue;
        tutti.push({ id, nome: summary });
        const scelto = this.calendari.length
          ? this.calendari.includes(summary.toLowerCase()) || this.calendari.includes(id.toLowerCase())
          : Boolean(it.primary);
        if (scelto) scelti.push({ id, nome: summary });
      }
      if (!scelti.length && !this.calendari.length && tutti.length) scelti.push(tutti[0]!);
    } else {
      const json = await this.richiestaJson(
        `${OUTLOOK_REST}/me/calendars?$top=50&$select=Id,Name,IsDefaultCalendar`,
        {},
      );
      const items = Array.isArray(json.value) ? (json.value as Record<string, unknown>[]) : [];
      const tutti: CalInfo[] = [];
      for (const it of items) {
        const id = String(it.Id ?? "");
        const nome = String(it.Name ?? "Calendario");
        if (!id) continue;
        tutti.push({ id, nome });
        const scelto = this.calendari.length ? this.calendari.includes(nome.toLowerCase()) : Boolean(it.IsDefaultCalendar);
        if (scelto) scelti.push({ id, nome });
      }
      if (!scelti.length && !this.calendari.length && tutti.length) scelti.push(tutti[0]!);
    }

    if (!scelti.length) {
      throw new Error(`Nessun calendario trovato per ${this.account}`);
    }
    this.cacheCalendari = { quandoS: adessoS, scelti };
    return scelti;
  }

  private segnaOk(): void {
    const ora = new Date(this.ctx.adesso()).toLocaleTimeString("en-GB", {
      timeZone: this.ctx.zona,
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    });
    this.stato = { stato: "ok", msg: `Letto alle ${ora}` };
  }

  private segnaErrore(e: unknown): [null, string[]] {
    const msg = e instanceof Error ? e.message : String(e);
    this.ctx.registro("WARNING", `${this.nome}: lettura non riuscita (${msg})`);
    this.stato = { stato: "errore", msg };
    return [null, [`${this.nome}: ${msg}`]];
  }

  descrizione(): Record<string, unknown> {
    return { id: this.id, account: this.account, nome: this.nome, tipo: this.tipo, eventi: this.conteggio, ...this.stato };
  }

  async riepilogo(): Promise<Record<string, unknown>> {
    try {
      await this.calendariScelti(true);
      if (this.conteggio === null) this.conteggio = 0;
      this.segnaOk();
    } catch (e) {
      this.segnaErrore(e);
    }
    return this.descrizione();
  }

  async eventi(inizio: Temporal.PlainDateTime, fine: Temporal.PlainDateTime): Promise<[Evento[], string[]]> {
    if (Temporal.PlainDateTime.compare(inizio, fine) > 0) {
      const daOra = ora(inizio, this.ctx.zona);
      const aOra = ora(fine, this.ctx.zona);
      contieneEvento(daOra, aOra, daOra, daOra);
    }
    let cals: CalInfo[];
    try {
      cals = await this.calendariScelti(false);
    } catch (e) {
      const [, err] = this.segnaErrore(e);
      return [[], err];
    }

    const voce = this.tokens.voceAccount(this.account)!;
    // Allarga di 1 giorno in UTC la query HTTP per coprire i giorni interi a cavallo della mezzanotte
    const utcInizio = inizio
      .toZonedDateTime(this.ctx.zona, { disambiguation: "compatible" })
      .toInstant()
      .subtract({ hours: 24 })
      .toString();
    const utcFine = fine
      .toZonedDateTime(this.ctx.zona, { disambiguation: "compatible" })
      .toInstant()
      .add({ hours: 24 })
      .toString();

    const daFinestra = ora(inizio, this.ctx.zona);
    const aFinestra = ora(fine, this.ctx.zona);
    const candidati = new Map<string, { peso: boolean[]; e: Evento }>();
    let totaleGrezz = 0;

    try {
      for (const cal of cals) {
        if (voce.provider === "google") {
          const url =
            `https://www.googleapis.com/calendar/v3/calendars/${encodeURIComponent(cal.id)}/events` +
            `?singleEvents=true&timeMin=${encodeURIComponent(utcInizio)}&timeMax=${encodeURIComponent(utcFine)}&maxResults=2500`;
          const json = await this.richiestaJson(url, {});
          const items = Array.isArray(json.items) ? (json.items as Record<string, unknown>[]) : [];
          totaleGrezz += items.length;
          for (const it of items) {
            if (it.status === "cancelled") continue;
            const startObj = (it.start ?? {}) as Record<string, unknown>;
            const endObj = (it.end ?? {}) as Record<string, unknown>;
            const tuttoGiorno = typeof startObj.date === "string" && !startObj.dateTime;
            let inizioEv: Temporal.PlainDateTime;
            let fineEv: Temporal.PlainDateTime;
            if (tuttoGiorno) {
              inizioEv = Temporal.PlainDate.from(String(startObj.date)).toPlainDateTime();
              fineEv = endObj.date ? Temporal.PlainDate.from(String(endObj.date)).toPlainDateTime() : inizioEv.add({ days: 1 });
            } else if (typeof startObj.dateTime === "string") {
              inizioEv = Temporal.Instant.from(startObj.dateTime).toZonedDateTimeISO(this.ctx.zona).toPlainDateTime();
              fineEv =
                typeof endObj.dateTime === "string"
                  ? Temporal.Instant.from(endObj.dateTime).toZonedDateTimeISO(this.ctx.zona).toPlainDateTime()
                  : inizioEv;
            } else {
              continue;
            }
            if (!contieneEvento(daFinestra, aFinestra, ora(inizioEv, this.ctx.zona), ora(fineEv, this.ctx.zona))) continue;

            const luogo = String(it.location ?? "");
            const descrizione = String(it.description ?? "");
            let linkGrezzo = (MEETING_URL.exec(luogo) ?? MEETING_URL.exec(descrizione))?.[0] ?? null;
            if (!linkGrezzo && typeof it.hangoutLink === "string" && it.hangoutLink) {
              linkGrezzo = it.hangoutLink;
            }
            if (!linkGrezzo && it.conferenceData && typeof it.conferenceData === "object") {
              const eps = (it.conferenceData as Record<string, unknown>).entryPoints;
              if (Array.isArray(eps)) {
                for (const ep of eps as Record<string, unknown>[]) {
                  if (typeof ep?.uri === "string" && MEETING_URL.test(ep.uri)) {
                    linkGrezzo = ep.uri;
                    break;
                  }
                }
              }
            }
            const orgObj = (it.organizer ?? {}) as Record<string, unknown>;
            const orgEmail = String(orgObj.email ?? "").toLowerCase();
            const orgNome = String(orgObj.displayName || orgObj.email || "");
            const attList = Array.isArray(it.attendees) ? (it.attendees as Record<string, unknown>[]) : [];
            const invitati = [...new Set(attList.map((a) => String(a?.email ?? "").toLowerCase()).filter(Boolean))].sort(confrontaPy);
            const uid = String(it.recurringEventId || it.id || "");
            const serie = Boolean(it.recurringEventId);
            const occupato = it.transparency !== "transparent";
            const inizioStr = testoNaive(inizioEv);
            const fineStr = testoNaive(fineEv);
            const titolo = String(it.summary || "") || "(senza titolo)";

            const e: Evento = {
              id: `${this.id}|g${uid}|${inizioStr}`,
              account: this.account,
              alias: this.nome,
              calendar: `${this.account} · ${cal.nome}`,
              title: titolo,
              start: inizioStr,
              end: fineStr,
              allDay: tuttoGiorno,
              location: luogo,
              organizer: orgNome,
              organizer_email: orgEmail,
              attendees: invitati,
              online_url: linkGrezzo ? pulisciLink(linkGrezzo) : null,
              web_link: null,
              fonte: "android",
              tipo: "caldav",
              occupato,
            };
            const peso = [!serie, Boolean(e.online_url), Boolean(luogo), Boolean(e.organizer_email)];
            const chiave = JSON.stringify([e.title, e.start, e.end]);
            const attuale = candidati.get(chiave);
            if (!attuale || pesoMaggiore(peso, attuale.peso)) candidati.set(chiave, { peso, e });
          }
        } else {
          const url =
            `${OUTLOOK_REST}/me/calendars/${encodeURIComponent(cal.id)}/calendarview` +
            `?startDateTime=${encodeURIComponent(utcInizio)}&endDateTime=${encodeURIComponent(utcFine)}` +
            `&$top=1000&$select=Id,SeriesMasterId,Type,Subject,BodyPreview,Body,Start,End,IsAllDay,IsCancelled,ShowAs,Location,Organizer,Attendees,OnlineMeetingUrl,OnlineMeeting`;
          const json = await this.richiestaJson(url, { Prefer: `outlook.timezone="${this.ctx.zona}"` });
          const items = Array.isArray(json.value) ? (json.value as Record<string, unknown>[]) : [];
          totaleGrezz += items.length;
          for (const it of items) {
            if (it.IsCancelled === true) continue;
            const startObj = (it.Start ?? {}) as Record<string, unknown>;
            const endObj = (it.End ?? {}) as Record<string, unknown>;
            const startRaw = String(startObj.DateTime ?? "").slice(0, 19);
            const endRaw = String(endObj.DateTime ?? "").slice(0, 19);
            if (startRaw.length < 19) continue;
            const tuttoGiorno = Boolean(it.IsAllDay);
            const inizioEv = tuttoGiorno ? Temporal.PlainDate.from(startRaw.slice(0, 10)).toPlainDateTime() : Temporal.PlainDateTime.from(startRaw);
            const fineEv =
              endRaw.length >= 19
                ? tuttoGiorno
                  ? Temporal.PlainDate.from(endRaw.slice(0, 10)).toPlainDateTime()
                  : Temporal.PlainDateTime.from(endRaw)
                : inizioEv;
            if (!contieneEvento(daFinestra, aFinestra, ora(inizioEv, this.ctx.zona), ora(fineEv, this.ctx.zona))) continue;

            const locObj = (it.Location ?? {}) as Record<string, unknown>;
            const luogo = String(locObj.DisplayName ?? "");
            const bodyObj = (it.Body ?? {}) as Record<string, unknown>;
            const descrizione = String(bodyObj.Content ?? it.BodyPreview ?? "");
            const onlineObj = (it.OnlineMeeting ?? {}) as Record<string, unknown>;
            const joinUrl = String(onlineObj.JoinUrl || it.OnlineMeetingUrl || "");
            const linkMatch = MEETING_URL.exec(luogo) ?? MEETING_URL.exec(descrizione) ?? (joinUrl ? MEETING_URL.exec(joinUrl) : null);
            const linkGrezzo = linkMatch ? linkMatch[0] : joinUrl || null;

            const orgWrapper = ((it.Organizer ?? {}) as Record<string, unknown>).EmailAddress as Record<string, unknown> | undefined;
            const orgEmail = String(orgWrapper?.Address ?? "").toLowerCase();
            const orgNome = String(orgWrapper?.Name || orgWrapper?.Address || "");
            const attList = Array.isArray(it.Attendees) ? (it.Attendees as Record<string, unknown>[]) : [];
            const invitati = [
              ...new Set(
                attList
                  .map((a) => {
                    const ea = (a?.EmailAddress ?? {}) as Record<string, unknown>;
                    return String(ea.Address ?? "").toLowerCase();
                  })
                  .filter(Boolean),
              ),
            ].sort(confrontaPy);

            const serie = it.Type === "Occurrence" || Boolean(it.SeriesMasterId && it.Type !== "Exception");
            const uid = String(it.SeriesMasterId || it.Id || "");
            const showAs = String(it.ShowAs ?? "");
            const occupato = showAs === "Busy" || showAs === "Oof";
            const inizioStr = testoNaive(inizioEv);
            const fineStr = testoNaive(fineEv);
            const titolo = String(it.Subject || "") || "(senza titolo)";

            const e: Evento = {
              id: `${this.id}|m${uid}|${inizioStr}`,
              account: this.account,
              alias: this.nome,
              calendar: `${this.account} · ${cal.nome}`,
              title: titolo,
              start: inizioStr,
              end: fineStr,
              allDay: tuttoGiorno,
              location: luogo,
              organizer: orgNome,
              organizer_email: orgEmail,
              attendees: invitati,
              online_url: linkGrezzo ? pulisciLink(linkGrezzo) : null,
              web_link: null,
              fonte: "android",
              tipo: "outlook",
              occupato,
            };
            const peso = [!serie, Boolean(e.online_url), Boolean(luogo), Boolean(e.organizer_email)];
            const chiave = JSON.stringify([e.title, e.start, e.end]);
            const attuale = candidati.get(chiave);
            if (!attuale || pesoMaggiore(peso, attuale.peso)) candidati.set(chiave, { peso, e });
          }
        }
      }
    } catch (e) {
      if (e instanceof ErroreAltro) throw e;
      const [, err] = this.segnaErrore(e);
      return [[], err];
    }

    const risultati = [...candidati.values()]
      .map((c) => c.e)
      .sort((a, b) => confrontaPy(a.start as string, b.start as string) || confrontaPy(a.title as string, b.title as string));
    this.conteggio = Math.max(this.conteggio ?? 0, totaleGrezz, risultati.length);
    this.segnaOk();
    return [risultati, []];
  }
}
