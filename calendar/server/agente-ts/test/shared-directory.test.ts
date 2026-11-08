import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SharedAuthorizationRefused } from "../src/shared-account.ts";
import { GestoreToken } from "../src/diretto.ts";
import type { FonteCalendario } from "../src/fonte.ts";
import { creaServer } from "../src/index.ts";
import { STANTIO_S, leggiRapportoTeams } from "../src/shared-teams.ts";

const TID = "22222222-2222-4222-8222-222222222222";
const OID = "11111111-1111-4111-8111-111111111111";
const shared = (capabilities: string[]) => ({
  tenantId: TID,
  objectId: OID,
  capabilities,
  origin: "mobile",
});

function fonte(account: string, letture: string[]): FonteCalendario {
  return {
    id: `microsoft::${account}`,
    nome: account,
    account,
    tipo: "microsoft",
    stato: { stato: "ok", msg: "" },
    conteggio: 0,
    descrizione: () => ({ account }),
    riepilogo: async () => {
      letture.push(`riepilogo ${account}`);
      return { id: `microsoft::${account}`, account };
    },
    eventi: async () => {
      letture.push(`eventi ${account}`);
      return [[], []];
    },
  };
}

const chiusure: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const c of chiusure.splice(0)) await c();
});

async function avvia(o: { chiaveTeams?: string; adesso?: () => number } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "elenco-"));
  const file = path.join(dir, "tokens.json");
  fs.writeFileSync(
    file,
    JSON.stringify({
      accounts: {
        "on@example.com": {
          provider: "microsoft",
          refresh_token: "r",
          shared: shared(["mail", "calendar", "chat"]),
        },
        "off@example.com": {
          provider: "microsoft",
          refresh_token: "r",
          shared: shared(["chat"]),
        },
        "legacy@example.com": { provider: "microsoft", refresh_token: "r" },
      },
    }),
  );
  const letture: string[] = [];
  const tokens = new GestoreToken(file, {
    fetchFn: (async () => Response.json({}, { status: 400 })) as typeof fetch,
  });
  const { server } = creaServer({
    chiave: "piena",
    chiaveTeams: o.chiaveTeams ?? "stretta",
    fonti: ["on@example.com", "off@example.com", "legacy@example.com"].map(
      (a) => fonte(a, letture),
    ),
    battito: path.join(dir, "battito.json"),
    tokens,
    zona: "Europe/Rome",
    adesso: o.adesso,
    registro: () => {},
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  chiusure.push(async () => {
    await new Promise<void>((r) => server.close(() => r()));
    fs.rmSync(dir, { recursive: true });
  });
  const base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  const chiama = (p: string, chiave: string, corpo?: unknown) =>
    fetch(base + p, {
      method: corpo === undefined ? "GET" : "POST",
      headers: {
        Authorization: `Bearer ${chiave}`,
        "Content-Type": "application/json",
      },
      body: corpo === undefined ? undefined : JSON.stringify(corpo),
    });
  return { chiama, letture, tokens };
}

describe("chiave di TeamsRelay", () => {
  it("turning a service off cancels pending onboarding before it can restore old choices", async () => {
    const { chiama } = await avvia();
    await chiama("/api/auth/shared/teams", "stretta", {accounts:[],others:[{email:"on@example.com",slot:4,state:"ok",detail:"green"}]});
    await chiama("/api/auth/shared", "stretta", {operation:"connect",account:"on@example.com",capabilities:["mail"]});
    const off=await chiama("/api/auth/shared", "stretta", {account:"on@example.com",capabilities:["mail","calendar"]});
    expect(off.status).toBe(200);
    const after=await(await chiama("/api/auth/shared","stretta")).json() as {signins:{phase:string;error:string}[]};
    expect(after.signins[0]).toMatchObject({phase:"blocked",error:expect.stringContaining("cancelled")});
  });
  it("connecting mail preserves already enabled calendar and browser Teams services", async () => {
    const { chiama } = await avvia();
    await chiama("/api/auth/shared/teams", "stretta", { accounts: [], others: [{ email:"legacy@example.com",slot:4,state:"ok",detail:"green" }] });
    const response = await chiama("/api/auth/shared", "stretta", {operation:"connect",account:"legacy@example.com",capabilities:["mail"]});
    expect(await response.json()).toMatchObject({ok:true,signin:{capabilities:expect.arrayContaining(["mail","calendar","chat"]),phase:"queued"}});
  });
  it("continues through the browser when the family client is refused for Teams, preserving mailbox access", async () => {
    const { chiama, tokens } = await avvia();
    await chiama("/api/auth/shared/teams", "stretta", { accounts: [], others: [] });
    const token = "x."+Buffer.from(JSON.stringify({ upn:"legacy@example.com",tid:TID,oid:OID,aud:"https://outlook.office.com",exp:Math.floor(Date.now()/1000)+3600,scp:"Mail.ReadWrite Calendars.ReadWrite" })).toString("base64url")+".x";
    vi.spyOn(tokens,"ottieniAccessToken").mockResolvedValue(token);
    vi.spyOn(tokens,"ottieniRisorsa").mockImplementation(async (_a,r) => { if(r!=="outlook") throw new SharedAuthorizationRefused("Microsoft refused this client"); return token; });
    const r = await chiama("/api/auth/shared", "stretta", { operation:"connect",account:"legacy@example.com",capabilities:["mail","chat"] });
    expect(await r.json()).toMatchObject({ok:true,signin:{phase:"queued"}});
    expect(tokens.voceAccount("legacy@example.com")?.refresh_token).toBe("r");
  });
  it("queues guided sign-in without credentials, correlates progress and rejects mismatched import", async () => {
    const { chiama } = await avvia();
    await chiama("/api/auth/shared/teams", "stretta", { accounts: [], others: [] });
    const response = await chiama("/api/auth/shared", "stretta", { operation: "connect", account: " NEW@EXAMPLE.COM ", capabilities: ["mail", "chat"] });
    expect(response.status).toBe(200);
    const { signin } = await response.json() as { signin: { id: string; account: string } };
    expect(signin).toMatchObject({ account: "new@example.com", phase: "queued" });
    expect((await (await chiama("/api/auth/shared", "stretta")).json() as { signins: unknown[] }).signins).toHaveLength(1);
    expect((await chiama("/api/auth/shared", "stretta", { operation: "signin-progress", account: "other@example.com", id: signin.id, phase: "browser", slot: 4 })).status).toBe(400);
    expect((await chiama("/api/auth/shared", "stretta", { operation: "signin-progress", account: signin.account, id: signin.id, phase: "browser", slot: 4, url: "https://teams.example/api/desktop/4" })).status).toBe(200);
    expect((await chiama("/api/auth/shared", "stretta", { operation: "signin-import", account: signin.account, id: signin.id, slot: 5, sessione: {} })).status).toBe(400);
  });
  it("apre solo l'elenco degli account condivisi, il rapporto Teams e i token della chat", async () => {
    const { chiama } = await avvia();
    expect((await chiama("/api/auth/shared", "stretta")).status).toBe(200);
    expect(
      (
        await chiama("/api/auth/shared/teams", "stretta", {
          accounts: [],
          others: [],
        })
      ).status,
    ).toBe(200);
    expect(
      (
        await chiama("/api/auth/shared", "stretta", {
          account: "legacy@example.com",
          enabled: false,
        })
      ).status,
    ).toBe(200);
    // la richiesta passa l'autorizzazione: il rifiuto viene dal broker (Microsoft finto), non dalla chiave
    expect(
      (
        await chiama(
          "/api/token-resource?account=on@example.com&resource=csa",
          "stretta",
        )
      ).status,
    ).toBe(403);
    for (const p of [
      "/api/token-resource?account=on@example.com&resource=outlook",
      // the same parameter twice, a fragment, a percent-encoded name: two parsers never disagree in favour of outlook
      "/api/token-resource?account=on@example.com&resource=outlook&resource=csa",
      "/api/token-resource?account=on@example.com&resource=outlook#&resource=csa",
      "/api/token-resource?account=on@example.com&resource=%6Futlook",
      "/api/token-posta",
      "/api/communication/accounts",
      "/api/auth/stato",
      "/api/calendari",
      "/api/salute",
    ]) {
      expect((await chiama(p, "stretta")).status, p).toBe(401);
    }
    for (const p of [
      "/api/communication/read",
      "/api/auth/rimuovi",
      "/api/auth/inizio",
      "/api/communication/gmail/read",
    ]) {
      expect(
        (await chiama(p, "stretta", { account: "on@example.com" })).status,
        p,
      ).toBe(401);
    }
  });

  it("la chiave piena apre tutto, una chiave sbagliata niente, e senza TEAMS_BROKER_KEY TeamsRelay non entra", async () => {
    const { chiama } = await avvia({ chiaveTeams: "" });
    expect((await chiama("/api/auth/shared", "piena")).status).toBe(200);
    expect((await chiama("/api/token-posta", "piena")).status).toBe(200);
    expect((await chiama("/api/auth/shared", "")).status).toBe(401);
    expect((await chiama("/api/auth/shared", "stretta")).status).toBe(401);
  });
});

describe("rapporto degli account Teams", () => {
  it("l'elenco mostra l'account Teams di ogni casella e quelli con il solo accesso del browser, finché il rapporto è recente", async () => {
    let ora = 1_000_000_000_000;
    const { chiama } = await avvia({ adesso: () => ora });
    const rapporto = {
      accounts: [
        { account: "ON@example.com", slot: 3, state: "running", detail: "ok" },
      ],
      others: [
        { email: "browser@example.com", slot: 4, state: "running", detail: "" },
        { email: "", slot: 5, state: "starting", detail: "" },
      ],
    };
    expect(
      (await chiama("/api/auth/shared/teams", "stretta", rapporto)).status,
    ).toBe(200);
    const elenco = (await (
      await chiama("/api/auth/shared", "piena")
    ).json()) as Record<string, unknown> & {
      accounts: { account: string; teams: unknown }[];
    };
    expect(
      elenco.accounts.find((a) => a.account === "on@example.com")?.teams,
    ).toEqual({ slot: 3, state: "running", detail: "ok" });
    expect(
      elenco.accounts.find((a) => a.account === "legacy@example.com")?.teams,
    ).toBeNull();
    expect(elenco.teamsOnly).toEqual([
      { email: "browser@example.com", slot: 4, state: "running", detail: "" },
      { email: "", slot: 5, state: "starting", detail: "" },
    ]);
    ora += (STANTIO_S + 1) * 1000;
    const vecchio = (await (
      await chiama("/api/auth/shared", "piena")
    ).json()) as Record<string, unknown> & { accounts: { teams: unknown }[] };
    expect(vecchio.accounts.every((a) => a.teams === null)).toBe(true);
    expect(vecchio.teamsOnly).toEqual([]);
  });

  it.each([
    ["un campo in più", { accounts: [], others: [], extra: 1 }],
    [
      "un indirizzo non valido",
      { accounts: [{ account: "nope", slot: 1, state: "running" }] },
    ],
    [
      "uno slot non intero",
      { accounts: [{ account: "a@example.com", slot: 1.5, state: "running" }] },
    ],
    [
      "uno stato che non è una parola",
      { accounts: [{ account: "a@example.com", slot: 1, state: "<b>" }] },
    ],
    [
      "un account senza indirizzo",
      { accounts: [{ account: "", slot: 1, state: "running" }] },
    ],
    [
      "troppe voci",
      {
        others: Array.from({ length: 65 }, (_, i) => ({
          email: "",
          slot: i + 1,
          state: "running",
        })),
      },
    ],
  ])("rifiuta %s", (_n, corpo) => {
    expect(() => leggiRapportoTeams(corpo, 0)).toThrow();
  });

  it("toglie i caratteri di controllo dal dettaglio e lo accorcia", () => {
    const r = leggiRapportoTeams(
      {
        accounts: [
          {
            account: "a@example.com",
            slot: 1,
            state: "signed-out",
            detail: "a\nb" + "x".repeat(300),
          },
        ],
      },
      0,
    );
    const d = r.accounts.get("a@example.com")!.detail;
    expect(d.startsWith("a b")).toBe(true);
    expect(d).toHaveLength(200);
  });
});

describe("capacità calendario spenta", () => {
  it("il calendario non legge né mostra la casella, senza errori; le altre restano, anche quelle non condivise", async () => {
    const { chiama, letture } = await avvia();
    const eventi = (await (
      await chiama(
        "/api/eventi?inizio=2026-11-09T00:00:00&fine=2026-11-10T00:00:00",
        "piena",
      )
    ).json()) as { errori: string[] };
    expect(eventi.errori).toEqual([]);
    const calendari = (await (
      await chiama("/api/calendari", "piena")
    ).json()) as { caselle: { account: string }[] };
    expect(calendari.caselle.map((c) => c.account)).toEqual([
      "on@example.com",
      "legacy@example.com",
    ]);
    expect(letture).toEqual([
      "eventi on@example.com",
      "eventi legacy@example.com",
      "riepilogo on@example.com",
      "riepilogo legacy@example.com",
    ]);
  });
});

describe("shared management commands", () => {
  it("queues removal with narrow key, removes mailbox, retains command until report acknowledgement", async () => {
    const { chiama } = await avvia();
    await chiama("/api/auth/shared/teams", "stretta", {
      accounts: [{ account: "on@example.com", slot: 1, state: "ok" }],
      others: [],
    });
    expect(
      (
        await chiama("/api/auth/rimuovi", "stretta", {
          account: "on@example.com",
        })
      ).status,
    ).toBe(401);
    const remove = await chiama("/api/auth/shared", "stretta", {
      account: "ON@example.com",
      operation: "remove",
    });
    expect(await remove.json()).toMatchObject({ ok: true, pending: true });
    let state = (await (
      await chiama("/api/auth/shared", "stretta")
    ).json()) as {
      accounts: { account: string }[];
      commands: {
        id: string;
        account: string;
        operation: string;
        on?: boolean;
      }[];
    };
    expect(
      state.accounts.some(
        (a: { account: string }) => a.account === "on@example.com",
      ),
    ).toBe(false);
    expect(state.commands).toHaveLength(1);
    await chiama("/api/auth/shared/teams", "stretta", {
      accounts: [],
      others: [],
      results: [{ id: state.commands[0]!.id }],
    });
    state = (await (await chiama("/api/auth/shared", "stretta")).json()) as {
      accounts: { account: string }[];
      commands: {
        id: string;
        account: string;
        operation: string;
        on?: boolean;
      }[];
    };
    expect(state.commands).toEqual([]);
  });
  it("supports browser-only stop/resume and remove, rejects unknown identity and stale reports", async () => {
    const { chiama } = await avvia();
    expect(
      (
        await chiama("/api/auth/shared", "stretta", {
          account: "browser@example.com",
          operation: "remove",
        })
      ).status,
    ).toBe(409);
    await chiama("/api/auth/shared/teams", "stretta", {
      accounts: [],
      others: [{ email: "browser@example.com", slot: 4, state: "ok" }],
    });
    expect(
      (
        await chiama("/api/auth/shared", "stretta", {
          account: "other@example.com",
          operation: "remove",
        })
      ).status,
    ).toBe(404);
    expect(
      await (
        await chiama("/api/auth/shared", "stretta", {
          account: "browser@example.com",
          operation: "chat",
          on: false,
        })
      ).json(),
    ).toMatchObject({ ok: true, pending: true });
    const state = (await (
      await chiama("/api/auth/shared", "stretta")
    ).json()) as {
      accounts: { account: string }[];
      commands: {
        id: string;
        account: string;
        operation: string;
        on?: boolean;
      }[];
    };
    expect(state.commands[0]).toMatchObject({
      account: "browser@example.com",
      operation: "chat",
      on: false,
    });
  });
});
