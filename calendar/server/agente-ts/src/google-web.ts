// La sessione web di Google (i cookie di un Chrome già entrato) per le parti di Google che non hanno un'API con OAuth usabile: Google Chat.
// Si importa dal PC (scripts/import-google.ts) con POST /api/auth/completa {"provider": "gchat", "sessione": {...}}, sta in
// /stato/google-web.json (0600) e si tiene viva da sola: i cookie che Google ruota nelle risposte si salvano, e quando Google rifiuta la
// sessione si prova una volta accounts.google.com/RotateCookies prima di dire che va importata di nuovo.
// I cookie escono solo verso google.com e i suoi sottodomini, scelti per host e percorso come li sceglie il browser; i salti verso
// googleusercontent.com restano senza cookie.

import fs from "node:fs";
import path from "node:path";
import type { FetchFn } from "./outlook-rest.ts";

export interface CookieGoogle {
  name: string;
  value: string;
  /** ".google.com" vale per il dominio e i sottodomini; senza punto iniziale solo per quell'host. */
  domain: string;
  path: string;
  /** Secondi Unix; 0 per un cookie di sessione. */
  expires: number;
}

export interface SessioneGoogleWeb {
  account: string;
  user_agent: string;
  cookies: CookieGoogle[];
  /** L'id utente di Chat (gaia) di chi ha la sessione. */
  self_id?: string;
  salvata: number;
  errore?: string;
}

interface DatiArchivio {
  sessioni: Record<string, SessioneGoogleWeb>;
  /** id utente di Chat -> nome, per non richiederli a ogni lettura. */
  nomi?: Record<string, string>;
}

export const UA_PREDEFINITO = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";
const NOME_COOKIE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]{1,128}$/;
const VALORE_COOKIE = /^[\x21\x23-\x2b\x2d-\x3a\x3c-\x5b\x5d-\x7e]{0,4096}$/;
// i cookie che RotateCookies rinnova (gweb, store/renew.go)
const RUOTATI = new Set(["SIDCC", "__Secure-1PSIDCC", "__Secure-3PSIDCC", "__Secure-1PSIDTS", "__Secure-3PSIDTS"]);

export class ErroreSessioneGoogle extends Error {}

export function hostGoogle(host: string): boolean {
  return host === "google.com" || host.endsWith(".google.com");
}

function dominioGoogle(dominio: string): boolean {
  return hostGoogle(dominio.replace(/^\./, ""));
}

function corrisponde(c: CookieGoogle, host: string, percorso: string, adessoS: number): boolean {
  if (c.expires > 0 && c.expires <= adessoS) return false;
  if (c.domain.startsWith(".")) {
    const d = c.domain.slice(1);
    if (host !== d && !host.endsWith(`.${d}`)) return false;
  } else if (host !== c.domain) return false;
  const p = c.path || "/";
  return percorso === p || (percorso.startsWith(p) && (p.endsWith("/") || percorso[p.length] === "/"));
}

/** Per ogni nome il cookie che il browser manderebbe a host+percorso: quello dell'host vince su quello del dominio. */
export function cookiePer(cookies: readonly CookieGoogle[], host: string, percorso: string, adessoS: number): Map<string, CookieGoogle> {
  const out = new Map<string, CookieGoogle>();
  if (!hostGoogle(host)) return out;
  for (const c of cookies) {
    if (!corrisponde(c, host, percorso, adessoS)) continue;
    const prima = out.get(c.name);
    if (prima && !prima.domain.startsWith(".") && c.domain.startsWith(".")) continue;
    out.set(c.name, c);
  }
  return out;
}

/** L'intestazione Cookie per host+percorso, "" fuori da google.com. */
export function intestazioneCookie(cookies: readonly CookieGoogle[], host: string, percorso: string, adessoS: number): string {
  return [...cookiePer(cookies, host, percorso, adessoS).values()]
    .map((c) => `${c.name}=${c.value}`)
    .sort()
    .join("; ");
}

/** Un Set-Cookie ricevuto da `host`, nella forma salvata; null se non va tenuto (fuori da google.com, nome o valore strani). */
export function leggiSetCookie(riga: string, host: string, adessoS: number): (CookieGoogle & { via: boolean }) | null {
  const [coppia, ...attributi] = riga.split(";");
  const uguale = coppia!.indexOf("=");
  if (uguale < 1) return null;
  const name = coppia!.slice(0, uguale).trim();
  const value = coppia!.slice(uguale + 1).trim();
  if (!NOME_COOKIE.test(name) || !VALORE_COOKIE.test(value) || !hostGoogle(host)) return null;
  let domain = host;
  let percorso = "/";
  let expires = 0;
  let maxAge: number | null = null;
  for (const a of attributi) {
    const i = a.indexOf("=");
    const k = (i < 0 ? a : a.slice(0, i)).trim().toLowerCase();
    const v = i < 0 ? "" : a.slice(i + 1).trim();
    if (k === "domain" && v) {
      const d = v.replace(/^\./, "").toLowerCase();
      // un dominio che non contiene l'host, o fuori da google.com, rende il cookie non valido
      if ((host !== d && !host.endsWith(`.${d}`)) || !hostGoogle(d)) return null;
      domain = `.${d}`;
    } else if (k === "path" && v.startsWith("/")) percorso = v;
    else if (k === "max-age" && /^-?\d+$/.test(v)) maxAge = Number(v);
    else if (k === "expires") {
      const t = Date.parse(v);
      if (Number.isFinite(t)) expires = Math.floor(t / 1000);
    }
  }
  if (maxAge !== null) expires = maxAge <= 0 ? -1 : adessoS + maxAge;
  return { name, value, domain, path: percorso, expires, via: expires !== 0 && expires <= adessoS };
}

/** Assorbe i Set-Cookie di una risposta di `host`: sostituisce lo stesso nome, dominio e percorso, toglie quelli scaduti. true se è cambiato qualcosa. */
export function assorbiCookie(cookies: CookieGoogle[], host: string, righe: readonly string[], adessoS: number, soloNomi?: ReadonlySet<string>): boolean {
  let cambiato = false;
  for (const riga of righe) {
    const c = leggiSetCookie(riga, host, adessoS);
    if (!c || (soloNomi && !soloNomi.has(c.name))) continue;
    const i = cookies.findIndex((x) => x.name === c.name && x.domain === c.domain && x.path === c.path);
    if (c.via) {
      if (i >= 0) {
        cookies.splice(i, 1);
        cambiato = true;
      }
      continue;
    }
    const nuovo: CookieGoogle = { name: c.name, value: c.value, domain: c.domain, path: c.path, expires: c.expires };
    if (i < 0) cookies.push(nuovo);
    else if (cookies[i]!.value !== nuovo.value || cookies[i]!.expires !== nuovo.expires) cookies[i] = nuovo;
    else continue;
    cambiato = true;
  }
  return cambiato;
}

/**
 * La sessione importata: user_agent e cookie nel formato di gweb e di Network.getCookies (name, value, domain, path, expires). Restano solo
 * i cookie di google.com; senza SID non è una sessione Google e si rifiuta.
 */
export function leggiSessioneImportata(dati: unknown, adessoS: number): { user_agent: string; cookies: CookieGoogle[] } {
  if (dati === null || typeof dati !== "object" || Array.isArray(dati)) throw new ErroreSessioneGoogle('serve {"user_agent": ..., "cookies": [...]}');
  const r = dati as Record<string, unknown>;
  if (!Array.isArray(r.cookies)) throw new ErroreSessioneGoogle("nella sessione mancano i cookies");
  if (r.cookies.length > 1000) throw new ErroreSessioneGoogle("too many cookies");
  const cookies: CookieGoogle[] = [];
  for (const x of r.cookies) {
    if (x === null || typeof x !== "object") continue;
    const c = x as Record<string, unknown>;
    if (typeof c.name !== "string" || typeof c.value !== "string" || typeof c.domain !== "string") continue;
    if (!NOME_COOKIE.test(c.name) || !VALORE_COOKIE.test(c.value) || !dominioGoogle(c.domain.toLowerCase())) continue;
    const expires = typeof c.expires === "number" && Number.isFinite(c.expires) && c.expires > 0 ? Math.floor(c.expires) : 0;
    if (expires && expires <= adessoS) continue;
    const percorso = typeof c.path === "string" && c.path.startsWith("/") ? c.path : "/";
    cookies.push({ name: c.name, value: c.value, domain: c.domain.toLowerCase(), path: percorso, expires });
  }
  if (!cookies.some((c) => c.name === "SID" && c.domain === ".google.com")) throw new ErroreSessioneGoogle("nella sessione manca il cookie SID di google.com: il profilo non è entrato in Google");
  const ua = typeof r.user_agent === "string" && /^[\x20-\x7e]{1,512}$/.test(r.user_agent) ? r.user_agent.trim().replace("HeadlessChrome", "Chrome") : UA_PREDEFINITO;
  return { user_agent: ua, cookies };
}

/**
 * Rinnova i cookie di verifica (SIDCC, __Secure-*PSIDTS...) con accounts.google.com/RotateCookies, come fa il browser. true se Google ne ha
 * dati di nuovi: chi chiama deve comunque riprovare la lettura vera, una rotazione riuscita non prova che la sessione valga ancora.
 */
export async function ruotaCookie(s: SessioneGoogleWeb, fetchFn: FetchFn, adessoS: number): Promise<boolean> {
  let resp: Response;
  try {
    resp = await fetchFn("https://accounts.google.com/RotateCookies", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Origin: "https://accounts.google.com",
        "User-Agent": s.user_agent,
        Cookie: intestazioneCookie(s.cookies, "accounts.google.com", "/RotateCookies", adessoS),
      },
      body: '[000,"-0000000000000000000"]',
      redirect: "manual",
      signal: AbortSignal.timeout(25_000),
    });
  } catch {
    return false;
  }
  await resp.body?.cancel().catch(() => undefined);
  if (resp.status !== 200) return false;
  const valid = resp.headers.getSetCookie().filter(line => {
    const cookie = leggiSetCookie(line, "accounts.google.com", adessoS);
    return cookie && cookie.domain === ".google.com" && cookie.path === "/" && cookie.expires > adessoS && !cookie.via && cookie.value;
  });
  return assorbiCookie(s.cookies, "accounts.google.com", valid, adessoS, RUOTATI);
}

/** /stato/google-web.json: le sessioni per casella e la cache dei nomi. Si rilegge quando cambia sul disco, si scrive atomico in 0600. */
export class ArchivioGoogleWeb {
  readonly percorso: string;
  private mtimeMs = -1;
  private dati: DatiArchivio = { sessioni: {} };

  constructor(percorso: string) {
    this.percorso = percorso;
  }

  private carica(): void {
    if (!fs.existsSync(this.percorso)) {
      if (this.mtimeMs !== 0) this.dati = { sessioni: {} };
      this.mtimeMs = 0;
      return;
    }
    const st = fs.statSync(this.percorso);
    if (st.mtimeMs === this.mtimeMs) return;
    const raw = JSON.parse(fs.readFileSync(this.percorso, "utf-8")) as Partial<DatiArchivio>;
    if (!raw || typeof raw !== "object" || !raw.sessioni || typeof raw.sessioni !== "object" || Array.isArray(raw.sessioni)) throw new Error("Invalid Google session store");
    const sessioni: Record<string, SessioneGoogleWeb> = {};
    for (const [k, v] of Object.entries(raw.sessioni)) {
      if (!v || typeof v !== "object" || !Array.isArray(v.cookies) || v.account?.toLowerCase() !== k.toLowerCase()) throw new Error("Invalid saved Google session");
      sessioni[k.toLowerCase()] = v;
    }
    this.dati = { sessioni, nomi: raw.nomi && typeof raw.nomi === "object" ? raw.nomi : {} };
    this.mtimeMs = st.mtimeMs;
  }

  salva(): void {
    fs.mkdirSync(path.dirname(this.percorso), { recursive: true, mode: 0o700 });
    const tmp = `${this.percorso}.${process.pid}.${Date.now()}.tmp`;
    try {
      fs.writeFileSync(tmp, JSON.stringify(this.dati, null, 2) + "\n", { encoding: "utf-8", mode: 0o600 });
      fs.renameSync(tmp, this.percorso);
      this.mtimeMs = fs.statSync(this.percorso).mtimeMs;
    } catch (error) {
      fs.rmSync(tmp, { force: true });
      throw error;
    }
  }

  elenco(): SessioneGoogleWeb[] {
    this.carica();
    return Object.values(this.dati.sessioni);
  }

  sessione(account: string): SessioneGoogleWeb | undefined {
    this.carica();
    return this.dati.sessioni[account.toLowerCase()];
  }

  metti(s: SessioneGoogleWeb): void {
    this.carica();
    const previous = this.dati.sessioni[s.account.toLowerCase()];
    this.dati.sessioni[s.account.toLowerCase()] = s;
    try { this.salva(); }
    catch (error) {
      if (previous) this.dati.sessioni[s.account.toLowerCase()] = previous;
      else delete this.dati.sessioni[s.account.toLowerCase()];
      throw error;
    }
  }

  rimuovi(account: string): boolean {
    this.carica();
    const c = account.toLowerCase();
    if (!this.dati.sessioni[c]) return false;
    const previous = this.dati.sessioni[c]!;
    delete this.dati.sessioni[c];
    try { this.salva(); }
    catch (error) { this.dati.sessioni[c] = previous; throw error; }
    return true;
  }

  nome(id: string): string | undefined {
    this.carica();
    return this.dati.nomi?.[id];
  }

  /** Aggiunge nomi alla cache; si salva solo se qualcosa è cambiato. */
  aggiungiNomi(nuovi: Record<string, string>): void {
    this.carica();
    const nomi = (this.dati.nomi ??= {});
    let cambiato = false;
    for (const [id, n] of Object.entries(nuovi)) {
      if (nomi[id] !== n) {
        nomi[id] = n;
        cambiato = true;
      }
    }
    if (cambiato) this.salva();
  }
}
