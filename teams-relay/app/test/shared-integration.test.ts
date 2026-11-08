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
import { tempDir } from "./helpers";
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
