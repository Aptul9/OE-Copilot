import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { ErroreAccessoScaduto, GestoreToken, OUTLOOK_WEB_CLIENT_ID, OUTLOOK_WEB_REDIRECT_URI, leggiSessione } from "../src/index.ts";

function fakeJwt(claims: Record<string, unknown>): string {
  const h = Buffer.from(JSON.stringify({ alg: "none", typ: "JWT" })).toString("base64url");
  const p = Buffer.from(JSON.stringify(claims)).toString("base64url");
  return `${h}.${p}.sig`;
}

const CASELLA = "mario.rossi@msc.example";
const ORA_MS = Date.parse("2026-11-08T08:00:00Z");
const ORA_S = Math.floor(ORA_MS / 1000);

const sessione = (cookies = [{ name: "ESTSAUTHPERSISTENT", value: "ests-1", domain: ".login.microsoftonline.com", path: "/", expires: ORA_S + 86400 }]) => ({
  userAgent: "Mozilla/5.0 Chrome",
  loginHint: CASELLA,
  cookies: [...cookies, { name: "altro", value: "x", domain: "teams.microsoft.com", path: "/", expires: -1 }],
});

const jwt = (over: Record<string, unknown> = {}) =>
  fakeJwt({ upn: CASELLA, tid: "tid-msc", aud: "https://outlook.office.com", exp: ORA_S + 3600, scp: "Mail.ReadWrite Calendars.ReadWrite", ...over });

const json = (dati: unknown, status = 200) => new Response(JSON.stringify(dati), { status, headers: { "Content-Type": "application/json" } });

interface Chiamata {
  url: string;
  headers: Record<string, string>;
  body: string;
}

/** Microsoft finto: authorize risponde col codice (o con l'errore dato), token scambia codice e refresh_token. */
function microsoftFinto(o: { erroreAuthorize?: string; rifiutaRefresh?: boolean; tokenAccesso?: string; tokenRefresh?: string; statusCodice?: number; setCookie?: string } = {}) {
  const chiamate: Chiamata[] = [];
  const fetchFn: typeof fetch = async (input, init) => {
    const url = String(input);
    const headers = Object.fromEntries(new Headers(init?.headers).entries());
    const body = String(init?.body ?? "");
    chiamate.push({ url, headers, body });
    if (url.includes("/oauth2/v2.0/authorize?")) {
      const q = new URL(url).searchParams;
      const ritorno = new URL(OUTLOOK_WEB_REDIRECT_URI);
      ritorno.searchParams.set("state", q.get("state") ?? "");
      if (o.erroreAuthorize) ritorno.searchParams.set("error", o.erroreAuthorize);
      else ritorno.searchParams.set("code", "codice-1");
      const h = new Headers({ Location: ritorno.toString() });
      if (o.setCookie) h.append("Set-Cookie", o.setCookie);
      return new Response(null, { status: 302, headers: h });
    }
    if (url.includes("/oauth2/v2.0/token")) {
      const p = new URLSearchParams(body);
      if (p.get("grant_type") === "refresh_token" && o.rifiutaRefresh) return json({ error: "invalid_grant" }, 400);
      if (p.get("grant_type") === "authorization_code" && o.statusCodice) return json({ error: "temporarily_unavailable" }, o.statusCodice);
      const n = chiamate.filter((c) => c.url.includes("/token")).length;
      const token = p.get("grant_type") === "refresh_token" && o.tokenRefresh ? o.tokenRefresh : (o.tokenAccesso ?? jwt());
      return json({ access_token: token, refresh_token: `rt-${n}`, expires_in: 3600 });
    }
    throw new Error(`URL inatteso: ${url}`);
  };
  return { chiamate, fetchFn };
}

describe("accesso da sessione del browser (client web di Outlook)", () => {
  const dirs: string[] = [];
  afterEach(() => {
    for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true });
  });
  const archivio = (contenuto?: unknown) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sessione-web-"));
    dirs.push(dir);
    const file = path.join(dir, "tokens.json");
    if (contenuto !== undefined) fs.writeFileSync(file, JSON.stringify(contenuto));
    return file;
  };

  it("leggiSessione tiene solo i cookie di login.microsoftonline.com e rifiuta forme sbagliate", () => {
    const s = leggiSessione(sessione());
    expect(s?.cookies.map((c) => c.name)).toEqual(["ESTSAUTHPERSISTENT"]);
    expect(leggiSessione({ ...sessione(), userAgent: "a\r\nb" })).toBeNull();
    expect(leggiSessione({ loginHint: CASELLA, cookies: [] })).toBeNull();
    expect(leggiSessione(null)).toBeNull();
  });

  it("importa la sessione: autorizzazione silenziosa del client web, cookie ruotati e refresh_token salvati", async () => {
    const file = archivio();
    const ms = microsoftFinto({ setCookie: "ESTSAUTHPERSISTENT=ests-2; domain=.login.microsoftonline.com; path=/; max-age=7776000; secure; HttpOnly" });
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });

    expect(await g.importaSessione(sessione(), "tid-msc")).toEqual({ account: CASELLA, tenant_id: "tid-msc" });

    const auth = new URL(ms.chiamate[0]!.url);
    expect(auth.pathname).toBe("/tid-msc/oauth2/v2.0/authorize");
    expect(auth.searchParams.get("client_id")).toBe(OUTLOOK_WEB_CLIENT_ID);
    expect(auth.searchParams.get("prompt")).toBe("none");
    expect(auth.searchParams.get("login_hint")).toBe(CASELLA);
    expect(ms.chiamate[0]!.headers.cookie).toBe("ESTSAUTHPERSISTENT=ests-1");
    expect(ms.chiamate[1]!.headers.origin).toBe("https://outlook.cloud.microsoft");
    expect(new URLSearchParams(ms.chiamate[1]!.body).get("code")).toBe("codice-1");

    const voce = JSON.parse(fs.readFileSync(file, "utf-8")).accounts[CASELLA];
    expect(voce.client_id).toBe(OUTLOOK_WEB_CLIENT_ID);
    expect(voce.refresh_token).toBe("rt-1");
    expect(voce.sessione.cookies).toEqual([expect.objectContaining({ name: "ESTSAUTHPERSISTENT", value: "ests-2" })]);
  });

  it("importa la sessione solo se Microsoft restituisce la stessa casella, risorsa Outlook e calendari", async () => {
    for (const [token, testo] of [
      [jwt({ upn: "altra@msc.example" }), "altra casella"],
      [jwt({ aud: "https://graph.microsoft.com" }), "altra risorsa"],
      [jwt({ scp: "Mail.Read" }), "calendari"],
    ] as const) {
      const file = archivio();
      const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ tokenAccesso: token }).fetchFn });
      await expect(g.importaSessione(sessione(), "tid-msc")).rejects.toThrow(testo);
      expect(fs.existsSync(file)).toBe(false);
    }
  });

  it("importazione respinta dall'accesso condizionale o senza cookie ESTS: errore leggibile, niente salvato", async () => {
    const file = archivio();
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ erroreAuthorize: "interaction_required" }).fetchFn });
    await expect(g.importaSessione(sessione(), "tid-msc")).rejects.toThrow("Accesso Microsoft non riuscito");
    await expect(g.importaSessione(sessione([]), "tid-msc")).rejects.toThrow("Sessione Microsoft finita");
    await expect(g.importaSessione(sessione(), "../common")).rejects.toThrow("Tenant");
    expect(fs.existsSync(file)).toBe(false);
  });

  const voceWeb = (over: Record<string, unknown> = {}) => ({
    accounts: {
      [CASELLA]: {
        provider: "microsoft",
        client_id: OUTLOOK_WEB_CLIENT_ID,
        tenant_id: "tid-msc",
        refresh_token: "rt-vecchio",
        access_token: "scaduto",
        expires_at: ORA_S - 10,
        sessione: leggiSessione(sessione()),
        ...over,
      },
    },
  });

  it("rinnova col refresh_token del client web mandando l'Origin di Outlook web", async () => {
    const file = archivio(voceWeb());
    const ms = microsoftFinto();
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
    expect(await g.ottieniAccessToken(CASELLA)).toBe(jwt());
    expect(ms.chiamate).toHaveLength(1);
    expect(ms.chiamate[0]!.headers.origin).toBe("https://outlook.cloud.microsoft");
    const p = new URLSearchParams(ms.chiamate[0]!.body);
    expect(p.get("client_id")).toBe(OUTLOOK_WEB_CLIENT_ID);
    expect(p.get("refresh_token")).toBe("rt-vecchio");
  });

  it("refresh_token rifiutato (24 ore SPA): nuovo accesso dalla sessione del browser", async () => {
    const file = archivio(voceWeb());
    const ms = microsoftFinto({ rifiutaRefresh: true });
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
    expect(await g.ottieniAccessToken(CASELLA)).toBe(jwt());
    expect(ms.chiamate.map((c) => new URL(c.url).pathname.split("/").pop())).toEqual(["token", "authorize", "token"]);
    const voce = JSON.parse(fs.readFileSync(file, "utf-8")).accounts[CASELLA];
    expect(voce.refresh_token).toBe("rt-2");
    expect(voce.errore).toBeUndefined();
  });

  it("sessione del browser finita anche lei: accesso scaduto con il motivo salvato", async () => {
    const file = archivio(voceWeb());
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ rifiutaRefresh: true, erroreAuthorize: "login_required" }).fetchFn });
    await expect(g.ottieniAccessToken(CASELLA)).rejects.toBeInstanceOf(ErroreAccessoScaduto);
    expect(JSON.parse(fs.readFileSync(file, "utf-8")).accounts[CASELLA].errore).toContain("Sessione del browser scaduta");
  });

  it("la sessione che riporta un'altra casella non sostituisce il token", async () => {
    const file = archivio(voceWeb({ refresh_token: undefined }));
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ tokenAccesso: jwt({ upn: "altra@msc.example" }) }).fetchFn });
    await expect(g.ottieniAccessToken(CASELLA)).rejects.toThrow("altra casella");
    expect(JSON.parse(fs.readFileSync(file, "utf-8")).accounts[CASELLA].access_token).toBe("scaduto");
  });

  const suDisco = (file: string) => JSON.parse(fs.readFileSync(file, "utf-8")).accounts[CASELLA];
  const TOKEN_SBAGLIATI = [
    [jwt({ upn: "altra@msc.example" }), "altra casella"],
    [jwt({ aud: "https://graph.microsoft.com" }), "altra risorsa"],
    [jwt({ exp: ORA_S - 1 }), "scaduto"],
    [jwt({ exp: undefined }), "scaduto"],
    [jwt({ scp: "Mail.ReadWrite" }), "calendari"],
    ["non-un-jwt", "altra casella"],
  ] as const;

  it("importazione: anche un token già scaduto è rifiutato e niente si salva", async () => {
    const file = archivio();
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ tokenAccesso: jwt({ exp: ORA_S - 1 }) }).fetchFn });
    await expect(g.importaSessione(sessione(), "tid-msc")).rejects.toThrow("scaduto");
    expect(fs.existsSync(file)).toBe(false);
  });

  it("rinnovo col refresh_token del client web senza sessione: un token sbagliato non si salva, nemmeno il suo refresh_token", async () => {
    for (const [token, testo] of TOKEN_SBAGLIATI) {
      const file = archivio(voceWeb({ sessione: undefined }));
      const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ tokenRefresh: token }).fetchFn });
      const e = await g.ottieniAccessToken(CASELLA).catch((x: unknown) => x);
      expect([testo, e]).toEqual([testo, expect.any(ErroreAccessoScaduto)]);
      expect((e as Error).message).toContain(testo);
      const voce = suDisco(file);
      expect([testo, voce.access_token, voce.refresh_token, voce.expires_at]).toEqual([testo, "scaduto", "rt-vecchio", ORA_S - 10]);
      expect(voce.errore).toContain(testo);
    }
  });

  it("rinnovo col refresh_token del client web che restituisce un token sbagliato: ripiega sulla sessione del browser, che dà quello giusto", async () => {
    const file = archivio(voceWeb());
    const ms = microsoftFinto({ tokenRefresh: jwt({ aud: "https://graph.microsoft.com" }) });
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
    expect(await g.ottieniAccessToken(CASELLA)).toBe(jwt());
    expect(ms.chiamate.map((c) => new URL(c.url).pathname.split("/").pop())).toEqual(["token", "authorize", "token"]);
    const voce = suDisco(file);
    expect([voce.access_token, voce.refresh_token, voce.errore]).toEqual([jwt(), "rt-2", undefined]);
  });

  it("nuovo accesso dalla sessione: lo stesso controllo (casella, risorsa, scadenza, calendari), con i cookie ruotati salvati", async () => {
    for (const [token, testo] of TOKEN_SBAGLIATI) {
      const file = archivio(voceWeb());
      const ms = microsoftFinto({
        rifiutaRefresh: true,
        tokenAccesso: token,
        setCookie: "ESTSAUTHPERSISTENT=ests-2; domain=.login.microsoftonline.com; path=/; max-age=7776000; secure; HttpOnly",
      });
      const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
      await expect(g.ottieniAccessToken(CASELLA)).rejects.toBeInstanceOf(ErroreAccessoScaduto);
      const voce = suDisco(file);
      expect([testo, voce.access_token, voce.refresh_token]).toEqual([testo, "scaduto", "rt-vecchio"]);
      expect(voce.errore).toContain(testo);
      expect(voce.sessione.cookies).toEqual([expect.objectContaining({ name: "ESTSAUTHPERSISTENT", value: "ests-2" })]);
    }
  });

  it("errore passeggero di Microsoft dopo l'autorizzazione: i cookie ruotati restano, la casella non va in errore", async () => {
    const file = archivio(voceWeb());
    const ms = microsoftFinto({
      rifiutaRefresh: true,
      statusCodice: 503,
      setCookie: "ESTSAUTHPERSISTENT=ests-2; domain=.login.microsoftonline.com; path=/; max-age=7776000; secure; HttpOnly",
    });
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
    const e = await g.ottieniAccessToken(CASELLA).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(Error);
    expect(e).not.toBeInstanceOf(ErroreAccessoScaduto);
    expect((e as Error).message).toContain("non disponibile");
    const voce = suDisco(file);
    expect(voce.errore).toBeUndefined();
    expect(voce.access_token).toBe("scaduto");
    expect(voce.sessione.cookies).toEqual([expect.objectContaining({ name: "ESTSAUTHPERSISTENT", value: "ests-2" })]);
  });

  it("la sessione salvata ripassa dal filtro: escono solo i cookie di login.microsoftonline.com ben formati", async () => {
    const sporca = {
      userAgent: "Mozilla/5.0 Chrome",
      loginHint: CASELLA,
      cookies: [
        { name: "ESTSAUTHPERSISTENT", value: "ests-1", domain: ".login.microsoftonline.com", path: "/", expires: ORA_S + 86400 },
        { name: "rubami", value: "x", domain: "evil.example", path: "/", expires: -1 },
        { name: "iniezione", value: "a; ESTSAUTH=falso", domain: "login.microsoftonline.com", path: "/", expires: -1 },
        { name: "senza-scadenza", value: "x", domain: "login.microsoftonline.com", path: "/" },
      ],
    };
    const file = archivio(voceWeb({ refresh_token: undefined, sessione: sporca }));
    const ms = microsoftFinto();
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
    expect(await g.ottieniAccessToken(CASELLA)).toBe(jwt());
    expect(ms.chiamate[0]!.headers.cookie).toBe("ESTSAUTHPERSISTENT=ests-1");
    expect(suDisco(file).sessione.cookies.map((c: { name: string }) => c.name)).toEqual(["ESTSAUTHPERSISTENT"]);
  });

  it("sessione salvata rotta o di un'altra casella: nessuna chiamata a Microsoft, accesso da rifare", async () => {
    for (const s of [{ ...leggiSessione(sessione()), loginHint: "altra@msc.example" }, { ...leggiSessione(sessione()), userAgent: "a\r\nb" }, { cookies: "x" }]) {
      const file = archivio(voceWeb({ refresh_token: undefined, sessione: s }));
      const ms = microsoftFinto();
      const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
      await expect(g.ottieniAccessToken(CASELLA)).rejects.toThrow("Sessione del browser non valida");
      expect(ms.chiamate).toHaveLength(0);
    }
  });

  it("le caselle di Outlook Mobile non controllano il token rinnovato (come prima)", async () => {
    const file = archivio(voceWeb({ client_id: undefined, sessione: undefined }));
    const altro = jwt({ aud: "https://graph.microsoft.com", scp: "Mail.Read" });
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: microsoftFinto({ tokenRefresh: altro }).fetchFn });
    expect(await g.ottieniAccessToken(CASELLA)).toBe(altro);
    expect(suDisco(file).refresh_token).toBe("rt-1");
  });

  it("le caselle di Outlook Mobile non cambiano: niente Origin e nessuna sessione", async () => {
    const file = archivio(voceWeb({ client_id: undefined, sessione: undefined }));
    const ms = microsoftFinto({ rifiutaRefresh: true });
    const g = new GestoreToken(file, { adesso: () => ORA_MS, fetchFn: ms.fetchFn });
    await expect(g.ottieniAccessToken(CASELLA)).rejects.toBeInstanceOf(ErroreAccessoScaduto);
    expect(ms.chiamate).toHaveLength(1);
    expect(ms.chiamate[0]!.headers.origin).toBeUndefined();
  });
});
