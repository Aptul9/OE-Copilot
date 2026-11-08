import fs from "node:fs";
import type http from "node:http";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  CalendarioDiretto,
  GestoreToken,
  OUTLOOK_CLIENT_ID,
  OUTLOOK_REDIRECT_URI,
  caricaFonti,
  contestoDi,
  creaServer,
} from "../src/index.ts";

function fakeJwt(claims: Record<string, unknown>): string {
  const h = Buffer.from(JSON.stringify({ alg: "none", typ: "JWT" })).toString("base64url");
  const p = Buffer.from(JSON.stringify(claims)).toString("base64url");
  return `${h}.${p}.sig`;
}

describe("lettura diretta via API (Outlook REST v2.0, Google Calendar v3, OAuth2 PKCE)", () => {
  const dirs: string[] = [];
  const servers: http.Server[] = [];

  afterEach(async () => {
    for (const s of servers.splice(0)) {
      await new Promise<void>((ok) => s.close(() => ok()));
    }
    for (const d of dirs.splice(0)) {
      fs.rmSync(d, { recursive: true, force: true });
    }
  });

  it("rinnova il refresh_token Microsoft, legge i calendari scelti ed espande gli eventi con ShowAs e link riunione", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "diretto-ms-"));
    dirs.push(dir);
    const tokensFile = path.join(dir, "tokens.json");
    const tokenPostaFile = path.join(dir, "token-outlook.json");
    const oraMs = Date.parse("2026-11-05T10:00:00Z");
    const oraS = Math.floor(oraMs / 1000);

    fs.writeFileSync(
      tokensFile,
      JSON.stringify({
        accounts: {
          "mario.rossi@alfa.example": {
            provider: "microsoft",
            client_id: OUTLOOK_CLIENT_ID,
            tenant_id: "tid-alfa",
            refresh_token: "rt-vecchio",
          },
        },
      }),
    );

    const chiamate: string[] = [];
    const nuovoJwt = fakeJwt({
      upn: "mario.rossi@alfa.example",
      tid: "tid-alfa",
      exp: oraS + 7200,
      scp: "Calendars.ReadWrite Mail.ReadWrite",
    });

    const fintoFetch: typeof fetch = async (input, init) => {
      const url = String(input);
      chiamate.push(url);
      if (url.includes("/oauth2/v2.0/token")) {
        const body = String(init?.body ?? "");
        expect(body).toContain("grant_type=refresh_token");
        expect(body).toContain("refresh_token=rt-vecchio");
        return new Response(
          JSON.stringify({
            access_token: nuovoJwt,
            refresh_token: "rt-nuovo",
            expires_in: 7200,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (url.includes("/me/calendars?")) {
        return new Response(
          JSON.stringify({
            value: [
              { Id: "cal-1", Name: "Calendar", IsDefaultCalendar: true },
              { Id: "cal-2", Name: "Condiviso", IsDefaultCalendar: false },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (url.includes("/me/calendars/cal-1/calendarview")) {
        return new Response(
          JSON.stringify({
            value: [
              {
                Id: "ev-1",
                SeriesMasterId: null,
                Type: "SingleInstance",
                Subject: "Allineamento architettura",
                Start: { DateTime: "2026-11-05T11:00:00.0000000", TimeZone: "Europe/Rome" },
                End: { DateTime: "2026-11-05T12:00:00.0000000", TimeZone: "Europe/Rome" },
                IsAllDay: false,
                IsCancelled: false,
                ShowAs: "Busy",
                Location: { DisplayName: "Sala 1" },
                Organizer: { EmailAddress: { Name: "Mario Rossi", Address: "Mario.Rossi@alfa.example" } },
                Attendees: [{ EmailAddress: { Name: "Luca", Address: "Luca@alfa.example" } }],
                OnlineMeeting: { JoinUrl: "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc" },
              },
              {
                Id: "ev-cancellato",
                Subject: "Annullata",
                Start: { DateTime: "2026-11-05T14:00:00.0000000", TimeZone: "Europe/Rome" },
                End: { DateTime: "2026-11-05T15:00:00.0000000", TimeZone: "Europe/Rome" },
                IsAllDay: false,
                IsCancelled: true,
                ShowAs: "Free",
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      throw new Error(`URL inatteso: ${url}`);
    };

    const gestore = new GestoreToken(tokensFile, { tokenPostaPath: tokenPostaFile, adesso: () => oraMs, fetchFn: fintoFetch });
    const ctx = contestoDi({ accessi: path.join(dir, "accessi.json"), zona: "Europe/Rome", adesso: () => oraMs, registro: () => {}, tokens: gestore, fetchFn: fintoFetch });
    const cal = new CalendarioDiretto("android::mario.rossi@alfa.example", "Alfa", "mario.rossi@alfa.example", null, ctx, gestore, fintoFetch);

    const [eventi, errori] = await cal.eventi(
      Temporal.PlainDateTime.from("2026-11-05T00:00:00"),
      Temporal.PlainDateTime.from("2026-11-06T00:00:00"),
    );

    expect(errori).toEqual([]);
    expect(eventi).toHaveLength(1);
    expect(eventi[0]).toEqual({
      id: "android::mario.rossi@alfa.example|mev-1|2026-11-05T11:00:00",
      account: "mario.rossi@alfa.example",
      alias: "Alfa",
      calendar: "mario.rossi@alfa.example · Calendar",
      title: "Allineamento architettura",
      start: "2026-11-05T11:00:00",
      end: "2026-11-05T12:00:00",
      allDay: false,
      location: "Sala 1",
      organizer: "Mario Rossi",
      organizer_email: "mario.rossi@alfa.example",
      attendees: ["luca@alfa.example"],
      online_url: "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc",
      web_link: null,
      fonte: "android",
      tipo: "outlook",
      occupato: true,
    });

    // Verifica persistenza del refresh_token ruotato e di token-outlook.json
    const salvati = JSON.parse(fs.readFileSync(tokensFile, "utf-8"));
    expect(salvati.accounts["mario.rossi@alfa.example"].refresh_token).toBe("rt-nuovo");
    const posta = JSON.parse(fs.readFileSync(tokenPostaFile, "utf-8"));
    expect(posta.account).toHaveLength(1);
    expect(posta.account[0].upn).toBe("mario.rossi@alfa.example");
    expect(posta.account[0].token).toBe(nuovoJwt);
  });

  it("rinnova il master_token aas_et/ Google e legge i calendari selezionati", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "diretto-google-"));
    dirs.push(dir);
    const tokensFile = path.join(dir, "tokens.json");
    const oraMs = Date.parse("2026-11-05T10:00:00Z");

    fs.writeFileSync(
      tokensFile,
      JSON.stringify({
        accounts: {
          "mario@beta.example": {
            provider: "google",
            master_token: "aas_et/PROVA_MASTER_TOKEN",
          },
        },
      }),
    );

    const fintoFetch: typeof fetch = async (input) => {
      const url = String(input);
      if (url === "https://android.clients.google.com/auth") {
        return new Response("Auth=ya29.PROVA_GOOGLE_AT\nExpiresInDurationSec=3600\n", { status: 200 });
      }
      if (url.includes("/users/me/calendarList")) {
        return new Response(
          JSON.stringify({
            items: [
              { id: "mario@beta.example", summary: "mario@beta.example", primary: true },
              { id: "extra@beta.example", summary: "Extra", primary: false },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (url.includes("/calendars/mario%40beta.example/events")) {
        return new Response(
          JSON.stringify({
            items: [
              {
                id: "gev1",
                status: "confirmed",
                summary: "Revisione progetto",
                start: { dateTime: "2026-11-05T14:00:00Z" },
                end: { dateTime: "2026-11-05T15:00:00Z" },
                organizer: { email: "mario@beta.example", displayName: "Mario" },
                attendees: [{ email: "collega@beta.example" }],
                hangoutLink: "https://meet.google.com/abc-defg-hij",
              },
              {
                id: "gev2",
                status: "confirmed",
                summary: "Ferie",
                start: { date: "2026-11-05" },
                end: { date: "2026-11-06" },
                transparency: "transparent",
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      throw new Error(`URL inatteso: ${url}`);
    };

    const gestore = new GestoreToken(tokensFile, { adesso: () => oraMs, fetchFn: fintoFetch });
    const ctx = contestoDi({ accessi: path.join(dir, "accessi.json"), zona: "Europe/Rome", adesso: () => oraMs, registro: () => {}, tokens: gestore, fetchFn: fintoFetch });
    const cal = new CalendarioDiretto("android::mario@beta.example", "Beta", "mario@beta.example", null, ctx, gestore, fintoFetch);

    const [eventi, errori] = await cal.eventi(
      Temporal.PlainDateTime.from("2026-11-05T00:00:00"),
      Temporal.PlainDateTime.from("2026-11-06T00:00:00"),
    );

    expect(errori).toEqual([]);
    expect(eventi).toHaveLength(2);
    expect(eventi[0]).toMatchObject({
      title: "Ferie",
      start: "2026-11-05T00:00:00",
      end: "2026-11-06T00:00:00",
      allDay: true,
      tipo: "caldav",
      occupato: false,
    });
    expect(eventi[1]).toMatchObject({
      title: "Revisione progetto",
      start: "2026-11-05T15:00:00",
      end: "2026-11-05T16:00:00",
      allDay: false,
      online_url: "https://meet.google.com/abc-defg-hij",
      tipo: "caldav",
      occupato: true,
    });
  });

  it("completa il login OAuth2 PKCE via HTTP (/api/auth/inizio, /api/auth/completa, /api/auth/stato, /api/token-posta)", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "diretto-http-"));
    dirs.push(dir);
    const tokensFile = path.join(dir, "tokens.json");
    const fontiFile = path.join(dir, "fonti.json");
    const oraMs = Date.parse("2026-11-05T10:00:00Z");
    const oraS = Math.floor(oraMs / 1000);

    fs.writeFileSync(tokensFile, JSON.stringify({ accounts: {} }));
    fs.writeFileSync(
      fontiFile,
      JSON.stringify([{ tipo: "android", id: "android::mario@alfa.example", nome: "Alfa", account: "mario@alfa.example" }]),
    );

    const jwt = fakeJwt({
      upn: "mario@alfa.example",
      tid: "tid-123",
      exp: oraS + 3600,
      scp: "Calendars.ReadWrite Mail.ReadWrite",
    });

    const fintoFetch: typeof fetch = async (input, init) => {
      const url = String(input);
      if (url === "https://login.microsoftonline.com/common/oauth2/v2.0/token") {
        const body = String(init?.body ?? "");
        expect(body).toContain("grant_type=authorization_code");
        expect(body).toContain("code=CODICE_AUTH_123");
        expect(body).toContain("code_verifier=");
        return new Response(
          JSON.stringify({
            access_token: jwt,
            refresh_token: "rt-pkce-123",
            expires_in: 3600,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      throw new Error(`URL inatteso: ${url}`);
    };

    const gestore = new GestoreToken(tokensFile, { adesso: () => oraMs, fetchFn: fintoFetch });
    const ctx = contestoDi({ accessi: path.join(dir, "accessi.json"), zona: "Europe/Rome", adesso: () => oraMs, registro: () => {}, tokens: gestore, fetchFn: fintoFetch });
    const fonti = caricaFonti(fontiFile, path.join(dir, "nessun-db"), ctx);
    const { server } = creaServer({
      chiave: "segreto-prova",
      fonti,
      battito: path.join(dir, "nessun-battito.json"),
      tokens: gestore,
      zona: "Europe/Rome",
      adesso: () => oraMs,
      registro: () => {},
    });
    servers.push(server);
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
    const base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
    const headers = { Authorization: "Bearer segreto-prova", "Content-Type": "application/json" };

    // 1. /api/salute con fonti dirette restituisce servizio sano anche senza battito.json
    const saluteRes = await fetch(`${base}/api/salute`, { headers });
    const saluteJson = (await saluteRes.json()) as Record<string, unknown>;
    expect(saluteJson.servizio).toEqual({ eta_s: 0, pausa: false, telefono: true });

    // 2. /api/auth/inizio genera URL PKCE col redirect_uri di Outlook Mobile
    const inizioRes = await fetch(`${base}/api/auth/inizio`, {
      method: "POST",
      headers,
      body: JSON.stringify({ email: "mario@alfa.example" }),
    });
    const inizioJson = (await inizioRes.json()) as { ok: boolean; url: string; state: string };
    expect(inizioJson.ok).toBe(true);
    expect(inizioJson.url).toContain(encodeURIComponent(OUTLOOK_CLIENT_ID));
    expect(inizioJson.url).toContain(encodeURIComponent(OUTLOOK_REDIRECT_URI));
    expect(inizioJson.url).toContain("login_hint=mario%40alfa.example");

    // 3. /api/auth/completa accetta l'URL msauth://... completo incollato dal browser
    const redirectIncollato = `${OUTLOOK_REDIRECT_URI}?code=CODICE_AUTH_123&state=${encodeURIComponent(inizioJson.state)}`;
    const compRes = await fetch(`${base}/api/auth/completa`, {
      method: "POST",
      headers,
      body: JSON.stringify({ provider: "microsoft", codice: redirectIncollato }),
    });
    const compJson = (await compRes.json()) as { ok: boolean; account: string };
    expect(compJson).toEqual({ ok: true, account: "mario@alfa.example", provider: "microsoft" });

    // 4. /api/auth/stato e /api/token-posta riflettono l'account configurato
    const statoRes = await fetch(`${base}/api/auth/stato`, { headers });
    const statoJson = (await statoRes.json()) as { caselle: Record<string, unknown>[] };
    expect(statoJson.caselle[0]?.configurato).toBe(true);

    const tpRes = await fetch(`${base}/api/token-posta`, { headers });
    const tpJson = (await tpRes.json()) as { account: { upn: string; token: string }[] };
    expect(tpJson.account).toHaveLength(1);
    expect(tpJson.account[0]?.upn).toBe("mario@alfa.example");
    expect(tpJson.account[0]?.token).toBe(jwt);
  });

  it("completa l'accesso Google OAuth2 PKCE e Microsoft Device Code via /api/auth/*", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "diretto-auth2-"));
    dirs.push(dir);
    const oraMs = Date.parse("2026-11-05T10:00:00Z");
    const tokensFile = path.join(dir, "tokens.json");
    const fontiFile = path.join(dir, "fonti.json");
    fs.writeFileSync(
      fontiFile,
      JSON.stringify([{ tipo: "outlook", id: "outlook::mario@alfa.example", nome: "Alfa", account: "mario@alfa.example" }]),
    );

    const jwtMs = fakeJwt({ upn: "mario@alfa.example", tid: "tid-alfa", exp: Math.floor(oraMs / 1000) + 3600 });
    const idTokenGoogle = fakeJwt({ email: "nuovo@gmail.com", exp: Math.floor(oraMs / 1000) + 3600 });
    let giriDevice = 0;

    const fintoFetch: typeof fetch = async (input, init) => {
      const url = String(input);
      const bodyStr = String(init?.body ?? "");
      if (url.endsWith("/oauth2/v2.0/devicecode")) {
        return new Response(
          JSON.stringify({
            device_code: "dev-code-123",
            user_code: "ABCD-1234",
            verification_uri: "https://microsoft.com/devicelogin",
            interval: 4,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (url.endsWith("/oauth2/v2.0/token") && bodyStr.includes("device_code")) {
        giriDevice++;
        if (giriDevice === 1) {
          return new Response(JSON.stringify({ error: "authorization_pending" }), { status: 400, headers: { "Content-Type": "application/json" } });
        }
        return new Response(JSON.stringify({ access_token: jwtMs, refresh_token: "rt-dev-999", expires_in: 3600 }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      if (url === "https://oauth2.googleapis.com/token") {
        expect(bodyStr).toContain("grant_type=authorization_code");
        expect(bodyStr).toContain("code=4%2F0Agtest123");
        return new Response(
          JSON.stringify({
            access_token: "ya29.google-pkce",
            refresh_token: "1//0g-refresh-pkce",
            id_token: idTokenGoogle,
            expires_in: 3600,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      throw new Error(`URL inatteso: ${url}`);
    };

    const gestore = new GestoreToken(tokensFile, { adesso: () => oraMs, fetchFn: fintoFetch });
    const ctx = contestoDi({ accessi: path.join(dir, "accessi.json"), zona: "Europe/Rome", adesso: () => oraMs, registro: () => {}, tokens: gestore, fetchFn: fintoFetch });
    const fonti = caricaFonti(fontiFile, path.join(dir, "nessun-db"), ctx);
    const { server } = creaServer({
      chiave: "segreto-prova",
      fonti,
      battito: path.join(dir, "nessun-battito.json"),
      tokens: gestore,
      zona: "Europe/Rome",
      adesso: () => oraMs,
      registro: () => {},
    });
    servers.push(server);
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
    const base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
    const headers = { Authorization: "Bearer segreto-prova", "Content-Type": "application/json" };

    // 1. Microsoft Device Code Flow
    const devInizio = (await (
      await fetch(`${base}/api/auth/inizio`, {
        method: "POST",
        headers,
        body: JSON.stringify({ provider: "microsoft", modo: "device", email: "mario@alfa.example" }),
      })
    ).json()) as { ok: boolean; modo: string; user_code: string; state: string };
    expect(devInizio.ok).toBe(true);
    expect(devInizio.modo).toBe("device");
    expect(devInizio.user_code).toBe("ABCD-1234");

    const poll1 = (await (
      await fetch(`${base}/api/auth/controlla`, {
        method: "POST",
        headers,
        body: JSON.stringify({ state: devInizio.state }),
      })
    ).json()) as { ok: boolean; stato: string };
    expect(poll1).toEqual({ ok: true, stato: "in_attesa" });

    const poll2 = (await (
      await fetch(`${base}/api/auth/controlla`, {
        method: "POST",
        headers,
        body: JSON.stringify({ state: devInizio.state }),
      })
    ).json()) as { ok: boolean; stato: string; account: string };
    expect(poll2).toEqual({ ok: true, stato: "completato", account: "mario@alfa.example", provider: "microsoft" });

    // 2. Google OAuth2 PKCE Flow (aggiunge anche la nuova casella Personale in automatico)
    const gInizio = (await (
      await fetch(`${base}/api/auth/inizio`, {
        method: "POST",
        headers,
        body: JSON.stringify({ provider: "google", email: "nuovo@gmail.com" }),
      })
    ).json()) as { ok: boolean; provider: string; url: string; state: string };
    expect(gInizio.ok).toBe(true);
    expect(gInizio.provider).toBe("google");
    expect(gInizio.url).toContain("accounts.google.com/o/oauth2/v2/auth");
    expect(gInizio.url).toContain("login_hint=nuovo%40gmail.com");

    const gComp = (await (
      await fetch(`${base}/api/auth/completa`, {
        method: "POST",
        headers,
        body: JSON.stringify({
          provider: "google",
          codice: `http://localhost/?state=${encodeURIComponent(gInizio.state)}&code=4%2F0Agtest123&scope=https://www.googleapis.com/auth/calendar`,
        }),
      })
    ).json()) as { ok: boolean; account: string; provider: string };
    expect(gComp).toEqual({ ok: true, account: "nuovo@gmail.com", provider: "google" });

    const statoRes = await fetch(`${base}/api/auth/stato`, { headers });
    const statoJson = (await statoRes.json()) as { caselle: { account: string; nome: string; configurato: boolean }[] };
    expect(statoJson.caselle.map((c) => c.account)).toEqual(["mario@alfa.example", "nuovo@gmail.com"]);
    expect(statoJson.caselle[1]?.nome).toBe("Personale");
    expect(statoJson.caselle[1]?.configurato).toBe(true);
  });

  it("rimuove una casella (/api/auth/rimuovi), la tiene fuori dopo un riavvio e la riprende com'era con un nuovo accesso", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "diretto-rimuovi-"));
    dirs.push(dir);
    const oraMs = Date.parse("2026-11-07T10:00:00Z");
    const tokensFile = path.join(dir, "tokens.json");
    const fontiFile = path.join(dir, "fonti.json");
    fs.writeFileSync(
      fontiFile,
      JSON.stringify([
        { tipo: "android", id: "android::mario@alfa.example", nome: "Alfa", account: "mario@alfa.example" },
        { tipo: "android", id: "android::Mario@Beta.example", nome: "Beta", account: "Mario@Beta.example", calendari: ["Calendar", "Gamma"] },
      ]),
    );
    fs.writeFileSync(
      tokensFile,
      JSON.stringify({
        accounts: {
          "mario@alfa.example": { provider: "microsoft", client_id: OUTLOOK_CLIENT_ID, refresh_token: "rt-alfa" },
          "mario@beta.example": { provider: "microsoft", client_id: OUTLOOK_CLIENT_ID, refresh_token: "rt-beta", errore: "invalid_grant" },
        },
      }),
    );
    const jwtBeta = fakeJwt({ upn: "mario@beta.example", tid: "tid-beta", exp: Math.floor(oraMs / 1000) + 3600 });
    const fintoFetch: typeof fetch = async (input, init) => {
      const url = String(input);
      if (url.endsWith("/oauth2/v2.0/devicecode")) {
        return Response.json({ device_code: "dev-beta", user_code: "BETA-0001", verification_uri: "https://microsoft.com/devicelogin", interval: 4 });
      }
      if (url.endsWith("/oauth2/v2.0/token") && String(init?.body ?? "").includes("device_code")) {
        return Response.json({ access_token: jwtBeta, refresh_token: "rt-beta-nuovo", expires_in: 3600 });
      }
      throw new Error(`URL inatteso: ${url}`);
    };
    const avvia = async () => {
      const gestore = new GestoreToken(tokensFile, { adesso: () => oraMs, fetchFn: fintoFetch });
      const ctx = contestoDi({ accessi: path.join(dir, "accessi.json"), zona: "Europe/Rome", adesso: () => oraMs, registro: () => {}, tokens: gestore, fetchFn: fintoFetch });
      const { server } = creaServer({
        chiave: "segreto-prova",
        fonti: caricaFonti(fontiFile, path.join(dir, "nessun-db"), ctx),
        battito: path.join(dir, "nessun-battito.json"),
        tokens: gestore,
        zona: "Europe/Rome",
        adesso: () => oraMs,
        registro: () => {},
      });
      servers.push(server);
      await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
      const base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
      const headers = { Authorization: "Bearer segreto-prova", "Content-Type": "application/json" };
      const post = async (p: string, corpo: unknown) => (await fetch(`${base}${p}`, { method: "POST", headers, body: JSON.stringify(corpo) })).json();
      const stato = async () => ((await (await fetch(`${base}/api/auth/stato`, { headers })).json()) as { caselle: { id: string; account: string; scaduto: boolean }[] }).caselle;
      return { post, stato };
    };

    let a = await avvia();
    expect((await a.stato()).map((c) => [c.account, c.scaduto])).toEqual([
      ["mario@alfa.example", false],
      ["mario@beta.example", true],
    ]);
    expect(await a.post("/api/auth/rimuovi", { account: "nessuno@alfa.example" })).toEqual({ ok: false, errore: "Casella non trovata" });
    expect(await a.post("/api/auth/rimuovi", { account: "  MARIO@beta.example " })).toEqual({ ok: true, account: "mario@beta.example" });
    expect((await a.stato()).map((c) => c.account)).toEqual(["mario@alfa.example"]);
    const suDisco = JSON.parse(fs.readFileSync(tokensFile, "utf-8")) as { accounts: Record<string, unknown>; rimossi?: string[] };
    expect(Object.keys(suDisco.accounts)).toEqual(["mario@alfa.example"]);
    expect(suDisco.rimossi).toEqual(["mario@beta.example"]);

    // riavvio: fonti.json la elenca ancora, ma resta fuori
    a = await avvia();
    expect((await a.stato()).map((c) => c.account)).toEqual(["mario@alfa.example"]);

    // nuovo accesso: torna la fonte di fonti.json (id e calendari scelti), non una nuova
    const inizio = (await a.post("/api/auth/inizio", { provider: "microsoft", modo: "device" })) as { state: string };
    expect(await a.post("/api/auth/controlla", { state: inizio.state })).toEqual({ ok: true, stato: "completato", account: "mario@beta.example", provider: "microsoft" });
    expect((await a.stato()).map((c) => [c.id, c.scaduto])).toEqual([
      ["android::mario@alfa.example", false],
      ["android::Mario@Beta.example", false],
    ]);
    expect(JSON.parse(fs.readFileSync(tokensFile, "utf-8")).rimossi).toBeUndefined();
  });
});
