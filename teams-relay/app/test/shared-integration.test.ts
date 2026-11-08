import fs from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";
import { directory } from "../src/supervisor/broker";
import {
  claimSlot,
  migrateAppSchema,
  openAppDb,
  slotRow,
} from "../src/lib/appdb";
import { sharedReport, syncSharedAccounts } from "../src/lib/shared-accounts";
import { tempDir, createSlotDb } from "./helpers";
import { STATE } from "@/shared/slot-db/state";
it("real HTTP producer consumer and acknowledgement", async () => {
  const agentRoot = new URL(
    "../../../calendar/server/agente-ts/src/",
    import.meta.url,
  ).href;
  const { GestoreToken } = await import(agentRoot + "diretto.ts");
  const { creaServer } = await import(agentRoot + "index.ts");
  const root = tempDir(),
    file = path.join(root, "tokens.json");
  fs.writeFileSync(
    file,
    JSON.stringify({
      accounts: {
        "fixture@example.com": {
          provider: "microsoft",
          refresh_token: "fixture",
        },
      },
    }),
  );
  const tokens = new GestoreToken(file);
  const { server } = creaServer({
    chiave: "full-fixture",
    chiaveTeams: "narrow-fixture",
    fonti: [],
    tokens,
    battito: path.join(root, "heartbeat"),
    zona: "Europe/Rome",
    registro: () => {},
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const base =
    "http://127.0.0.1:" + (server.address() as { port: number }).port;
  const dir = directory({ url: base, key: "narrow-fixture" });
  const db = openAppDb(path.join(root, "app.db"));
  migrateAppSchema(db);
  const own = claimSlot(db, "owner", { perUser: null }),
    foreign = claimSlot(db, "foreign", { perUser: null }),
    wipes: number[] = [];
  const identity = () => "fixture@example.com";
  const deps = {
    db,
    dir,
    dataDir: root,
    owner: "owner",
    perUser: null,
    identity,
    ctl: {
      show: async () => true,
      start: async () => {},
      light: async () => {},
      stop: async () => {},
      wipe: async (n: number) => {
        wipes.push(n);
      },
    },
  };
  try {
    await dir.report(sharedReport(db, "owner", identity));
    expect(
      await dir.set({ account: "fixture@example.com", operation: "remove" }),
    ).toMatchObject({ ok: true, pending: true });
    expect(
      JSON.parse(fs.readFileSync(file + ".commands.json", "utf8")),
    ).toHaveLength(1);
    await syncSharedAccounts(deps);
    expect(slotRow(db, own)).toBeNull();
    expect(slotRow(db, foreign)).not.toBeNull();
    expect(wipes).toEqual([own]);
    expect(((await dir.list()) as { commands: unknown[] }).commands).toEqual(
      [],
    );
    expect(tokens.voceAccount("fixture@example.com")).toBeFalsy();
    await syncSharedAccounts(deps);
    expect(wipes).toEqual([own]);
    expect((await fetch(base + "/api/auth/shared")).status).toBe(401);
  } finally {
    db.close();
    await new Promise<void>((r) => server.close(() => r()));
  }
});

it("guided onboarding crosses the real broker/agent HTTP boundary, preserves Teams, and renews after restart", async () => {
  const agentRoot = new URL("../../../calendar/server/agente-ts/src/", import.meta.url).href;
  const { GestoreToken } = await import(agentRoot + "diretto.ts");
  const { creaServer } = await import(agentRoot + "index.ts");
  const root = tempDir(), file = path.join(root, "tokens.json");
  fs.writeFileSync(file, JSON.stringify({ accounts: {} }));
  const address = "fixture@example.com", clients: string[] = [];
  const jwt = "x." + Buffer.from(JSON.stringify({ upn: address, tid: "22222222-2222-4222-8222-222222222222", oid: "11111111-1111-4111-8111-111111111111", aud: "https://outlook.office.com", exp: Math.floor(Date.now() / 1000) + 3600, scp: "Mail.ReadWrite Calendars.ReadWrite" })).toString("base64url") + ".x";
  const microsoft: typeof fetch = async (input, init) => {
    const url = new URL(String(input));
    if (url.pathname.endsWith("/authorize")) {
      const client = url.searchParams.get("client_id")!;
      clients.push(client);
      const back = new URL(url.searchParams.get("redirect_uri")!);
      back.searchParams.set("state", url.searchParams.get("state")!);
      back.searchParams.set(client.startsWith("27922004") ? "error" : "code", client.startsWith("27922004") ? "interaction_required" : "fixture-code");
      return new Response(null, { status: 302, headers: { Location: back.href, "Set-Cookie": "ESTSAUTH=rotated; Path=/; Secure; HttpOnly" } });
    }
    if (String(init?.body).includes("grant_type=refresh_token")) return Response.json({ error: "invalid_grant" }, { status: 400 });
    return Response.json({ access_token: jwt, refresh_token: "private-refresh", expires_in: 3600 });
  };
  const tokens = new GestoreToken(file, { fetchFn: microsoft });
  const { server } = creaServer({ chiave: "full", chiaveTeams: "narrow", fonti: [], tokens, battito: path.join(root, "heartbeat"), zona: "Europe/Rome", registro: () => {} });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const db = openAppDb(path.join(root, "app.db")); migrateAppSchema(db);
  const own = claimSlot(db, "owner", { perUser: null }), foreign = claimSlot(db, "foreign", { perUser: null });
  fs.mkdirSync(path.join(root, String(own)));
  fs.writeFileSync(path.join(root, String(own), "session.json"), JSON.stringify({ loginHint: address, userAgent: "Chrome", cookies: [{ name: "ESTSAUTH", value: "private-cookie", domain: "login.microsoftonline.com", path: "/", expires: -1 }] }));
  process.env.APP_DB = path.join(root, "app.db");
  const health = createSlotDb(path.join(root, String(own), "messages.db"));
  health.prepare("INSERT INTO state(k,v) VALUES(?,?)").run(STATE.health, JSON.stringify({ ts: Math.floor(Date.now()/1000), teams: "ok", overall: "green" })); health.close();
  const dir = directory({ url: "http://127.0.0.1:" + (server.address() as { port: number }).port, key: "narrow" }, fetch, root);
  try {
    await dir.report(sharedReport(db, "owner", () => address));
    await dir.set({ operation: "connect", account: address, capabilities: ["mail", "calendar", "chat"] });
    await syncSharedAccounts({ db, dir, dataDir: root, owner: "owner", perUser: null, identity: () => address, ctl: { show: async () => true, start: async () => {}, light: async () => {}, stop: async () => {}, wipe: async () => { throw Error("Existing session must not be wiped"); } } });
    const output = await dir.list() as { signins: { phase: string; route: string }[] };
    expect(output.signins[0]).toMatchObject({ phase: "complete", route: "outlook-web" });
    expect(JSON.stringify(output)).not.toMatch(/private-cookie|private-refresh|access_token/);
    expect(slotRow(db, own)?.shared_account).toBe("");
    expect(slotRow(db, foreign)?.owner_id).toBe("foreign");
    expect(clients.map(c => c.slice(0, 8))).toEqual(["27922004", "9199bf20"]);
    const saved = JSON.parse(fs.readFileSync(file, "utf8")); saved.accounts[address].expires_at = 0; fs.writeFileSync(file, JSON.stringify(saved));
    clients.length = 0;
    const restarted = new GestoreToken(file, { fetchFn: microsoft });
    expect(await restarted.ottieniAccessToken(address)).toBe(jwt);
    expect(clients.map(c => c.slice(0, 8))).toEqual(["9199bf20"]);
  } finally { db.close(); await new Promise<void>(resolve => server.close(() => resolve())); }
});
