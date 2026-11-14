// The shared Microsoft accounts of the OE calendar agent, followed by this server: a slot running for each account with
// Teams on, stopped with Teams off, a slot the owner signed in to the same account taken over instead of doubled
import fs from "node:fs";
import path from "node:path";
import type Database from "better-sqlite3";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  askRefresh,
  beginCheck,
  claimSlot,
  migrateAppSchema,
  openAppDb,
  setCheckEvery,
  setSlotStopped,
  slotRow,
  type Slot,
} from "@/lib/appdb";
import type { ControlClient, DirectoryClient } from "@/lib/control";
import {
  NO_CALLS_SWITCH,
  parseDirectory,
  setSharedCapability,
  setSharedChat,
  sharedOwnerId,
  sharedReport,
  syncSharedAccounts,
  type SyncDeps,
} from "@/lib/shared-accounts";
import { exclusive } from "@/lib/slots";
import { tempDir, createSlotDb } from "./helpers";
import { STATE } from "@/shared/slot-db/state";
import { accountSummary } from "@/lib/accounts";

const OWNER = "owner-id";
let db: Database.Database;
let dataDir: string;
let calls: string[];
let accounts: {
  account: string;
  shared: { capabilities: string[]; origin: string } | null;
  status?: string;
  message?: string;
}[];
let identities: Record<number, string>;

const ctl: ControlClient = {
  start: async (n) => void calls.push(`start ${n}`),
  light: async (n) => void calls.push(`light ${n}`),
  stop: async (n) => void calls.push(`stop ${n}`),
  wipe: async (n) => void calls.push(`wipe ${n}`),
  show: async () => true,
};

function dir(
  o: { list?: () => Promise<unknown> } = {},
): DirectoryClient & { set: ReturnType<typeof vi.fn> } {
  return {
    list: o.list ?? (async () => ({ ok: true, accounts })),
    set: vi.fn(async (b: { account: string; capabilities?: string[] }) => ({
      ok: true,
      account: b.account,
      shared: { capabilities: b.capabilities, origin: "mobile" },
    })),
    report: vi.fn(async () => ({ ok: true })),
  };
}

const deps = (o: Partial<SyncDeps> = {}): SyncDeps => ({
  ctl,
  db,
  dir: dir(),
  dataDir,
  perUser: null,
  owner: OWNER,
  identity: (s: Slot) => identities[s.slot] ?? "",
  ...o,
});
const shared = (account: string, capabilities: string[]) => ({
  account,
  shared: { capabilities, origin: "mobile" },
});

beforeEach(() => {
  const root = tempDir();
  dataDir = root;
  db = openAppDb(path.join(root, "app.db"));
  migrateAppSchema(db);
  calls = [];
  accounts = [];
  identities = {};
});

describe("a pass over the shared accounts", () => {
  it("re-adds an account whose earlier sign-in slot was removed", async () => {
    const job={id:"retry",account:"new@example.com",capabilities:["mail"],phase:"queued",slot:9};
    const directory=dir({list:async()=>({ok:true,accounts:[],signins:[job]})});
    await syncSharedAccounts(deps({dir:{...directory,connect:async()=>({ok:false,interaction:true})}}));
    expect(slotRow(db,1)?.owner_id).toBe(OWNER);
    expect(directory.set).toHaveBeenCalledWith(expect.objectContaining({operation:"signin-progress",slot:1,phase:"browser"}));
  });
  it("resumes an unfinished mail-only profile without wiping it or enabling Teams", async () => {
    const slot=claimSlot(db,OWNER,{perUser:null}); setSlotStopped(db,slot,true);
    const job={id:"retry",account:"new@example.com",capabilities:["mail"],phase:"queued",slot};
    const directory=dir({list:async()=>({ok:true,accounts:[],signins:[job]})});
    await syncSharedAccounts(deps({ctl:{...ctl,signin:async n=>void calls.push(`signin ${n}`)},dir:{...directory,connect:async()=>({ok:false,interaction:true})}}));
    expect(calls).toEqual([`signin ${slot}`]);
    expect(slotRow(db,slot)?.stopped).toBe(1);
  });
  it("does not let cleanup of an old blocked job stop a new profile on the same slot", async () => {
    const slot=claimSlot(db,OWNER,{perUser:null}); setSlotStopped(db,slot,true);
    const jobs=[{id:"old",account:"old@example.com",phase:"blocked",slot,capabilities:["mail"]},{id:"new",account:"new@example.com",phase:"browser",slot,capabilities:["mail"]}];
    const directory=dir({list:async()=>({ok:true,accounts:[],signins:jobs})});
    await syncSharedAccounts(deps({dir:{...directory,connect:async()=>({ok:false,interaction:true})}}));
    expect(calls).not.toContain(`stop ${slot}`);
  });
  it("gives back a slot claimed for a sign-in that stopped before anyone signed in to it", async () => {
    const slot = claimSlot(db, OWNER, { perUser: null });
    const now = Math.floor(Date.now() / 1000);
    const job = { id: "bip", account: "new@example.com", capabilities: ["mail", "chat"], phase: "blocked", slot, created: now - 60, ended: now + 5 };
    await syncSharedAccounts(deps({ dir: { ...dir({ list: async () => ({ ok: true, accounts: [], signins: [job] }) }), connect: async () => ({ ok: false, interaction: true }) } }));
    expect(calls).toEqual([`stop ${slot}`, `wipe ${slot}`]);
    expect(slotRow(db, slot)).toBeNull();
  });
  it("never gives back a slot claimed after the sign-in ended, nor one signed in to an account", async () => {
    const later = claimSlot(db, OWNER, { perUser: null });
    const signedIn = claimSlot(db, OWNER, { perUser: null });
    identities[signedIn] = "new@example.com";
    const now = Math.floor(Date.now() / 1000);
    const jobs = [
      { id: "old", account: "old@example.com", capabilities: ["mail"], phase: "blocked", slot: later, created: now - 1000, ended: now - 700 },
      { id: "new", account: "new@example.com", capabilities: ["mail"], phase: "blocked", slot: signedIn, created: now - 60, ended: now + 5 },
    ];
    await syncSharedAccounts(deps({ dir: { ...dir({ list: async () => ({ ok: true, accounts: [], signins: jobs }) }), connect: async () => ({ ok: false, interaction: true }) } }));
    expect(calls).toEqual([]);
    expect(slotRow(db, later)?.owner_id).toBe(OWNER);
    expect(slotRow(db, signedIn)?.owner_id).toBe(OWNER);
  });
  it("queues one automatic session repair without enabling services the owner left off", async () => {
    accounts = [{ account: "expired@example.com", shared: { capabilities: ["mail"], origin: "outlook-spa" }, status: "errore", message: "Sessione del browser scaduta" }];
    let signins: { account: string; phase: string }[] = [];
    const directory = dir({ list: async () => ({ ok: true, accounts, signins }) });
    await syncSharedAccounts(deps({ dir: directory }));
    expect(directory.set).toHaveBeenCalledWith({ operation: "connect", account: "expired@example.com", capabilities: ["mail"] });
    directory.set.mockClear(); signins = [{ account: "expired@example.com", phase: "blocked" }];
    await syncSharedAccounts(deps({ dir: directory }));
    expect(directory.set).not.toHaveBeenCalled();
  });
  it("mail-only sign-in opens only a browser while Teams remains stopped", async () => {
    const signin = { id: "job", account: "new@example.com", capabilities: ["mail"], phase: "queued" };
    const directory = dir({ list: async () => ({ ok: true, accounts: [], signins: [signin] }) });
    await syncSharedAccounts(deps({ ctl: { ...ctl, signin: async n => void calls.push(`signin ${n}`) }, dir: { ...directory, connect: async () => ({ ok: false, interaction: true }) } }));
    expect(calls).toContain("signin 1");
    expect(calls).not.toContain("start 1");
    expect(slotRow(db, 1)).toMatchObject({ stopped: 1, light: 1 });
  });
  it("keeps one verified identity for a mail-only browser profile without a Teams agent", async () => {
    process.env.APP_DB = path.join(dataDir,"app.db");
    const signin={id:"mail-job",account:"mail@example.com",capabilities:["mail"],phase:"queued"};
    const directory=dir({list:async()=>({ok:true,accounts:[],signins:[signin]})});
    await syncSharedAccounts(deps({ctl:{...ctl,signin:async()=>{}},dir:{...directory,connect:async()=>({ok:true,route:"outlook-web"})}}));
    expect(accountSummary(slotRow(db,1)!).email).toBe("mail@example.com");
    expect(sharedReport(db,OWNER).others).toEqual([expect.objectContaining({email:"mail@example.com",slot:1,state:"stopped"})]);
  });
  it("guided sign-in reuses only the owner's matching browser and imports server-side without takeover", async () => {
    const foreign = claimSlot(db, "other", { perUser: null });
    const own = claimSlot(db, OWNER, { perUser: null });
    identities[foreign] = identities[own] = "new@example.com";
    process.env.APP_DB = path.join(dataDir, "app.db");
    const health = createSlotDb(path.join(dataDir, String(own), "messages.db"));
    health.prepare("INSERT INTO state(k,v) VALUES(?,?)").run(STATE.health, JSON.stringify({ ts: Math.floor(Date.now()/1000), teams: "ok", overall: "green" }));
    health.close();
    const signin = { id: "job", account: "new@example.com", capabilities: ["mail", "chat"], phase: "queued" };
    const directory = dir({ list: async () => ({ ok: true, accounts: [], signins: [signin] }) });
    const connect = vi.fn(async () => ({ ok: true, route: "outlook-web" }));
    await syncSharedAccounts(deps({ dir: { ...directory, connect } }));
    expect(connect).toHaveBeenCalledWith({ id: "job", account: signin.account, slot: own });
    expect(directory.set).toHaveBeenCalledWith(expect.objectContaining({ operation: "signin-progress", slot: own, phase: "complete" }));
    expect(slotRow(db, own)?.shared_account).toBe("");
    expect(calls).not.toContain(`stop ${foreign}`);
  });

  it("guided sign-in allocates one browser, exposes interaction progress, never wipes an existing session", async () => {
    const signin = { id: "job", account: "new@example.com", capabilities: ["mail"], phase: "queued" };
    const directory = dir({ list: async () => ({ ok: true, accounts: [], signins: [signin] }) });
    const connect = vi.fn(async () => ({ ok: false, interaction: true }));
    await syncSharedAccounts(deps({ dir: { ...directory, connect } }));
    expect(calls).toContain("start 1");
    expect(directory.set).toHaveBeenCalledWith(expect.objectContaining({ operation: "signin-progress", slot: 1, phase: "browser" }));
    expect(calls.filter(c => c.startsWith("wipe"))).toHaveLength(1);
    // light from the start: its browser is a sign-in run, ended by the sign-in it captures
    expect(slotRow(db, 1)).toMatchObject({ light: 1, refresh_result: "asked" });
  });
  it("an account with Teams on gets a new slot of the owner, wiped, bound, its light agent started", async () => {
    accounts = [
      shared("Mario@Example.com", ["mail", "chat"]),
      shared("anna@example.com", ["mail", "calendar"]),
      { account: "legacy@example.com", shared: null },
    ];
    const r = await syncSharedAccounts(deps());
    expect(r).toMatchObject({ enabled: true, errors: {} });
    expect(calls).toEqual(["stop 1", "wipe 1", "light 1"]);
    expect(slotRow(db, 1)).toMatchObject({
      owner_id: OWNER,
      light: 1,
      stopped: 0,
      shared_account: "mario@example.com",
      shared_region: "emea",
    });
    expect(slotRow(db, 2)).toBeNull();
  });

  it("a second pass changes nothing", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    calls = [];
    await syncSharedAccounts(deps());
    expect(calls).toEqual([]);
  });

  it("Teams off stops the slot and keeps it; on again starts it", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    accounts = [shared("mario@example.com", ["mail"])];
    calls = [];
    await syncSharedAccounts(deps());
    expect(calls).toEqual(["stop 1"]);
    expect(slotRow(db, 1)).toMatchObject({
      stopped: 1,
      shared_account: "mario@example.com",
    });
    accounts = [shared("mario@example.com", ["mail", "chat"])];
    calls = [];
    await syncSharedAccounts(deps());
    expect(calls).toEqual(["light 1"]);
    expect(slotRow(db, 1)?.stopped).toBe(0);
  });

  it("reads the list inside the queue: a pass that waited behind a change applies the change, not the list before it", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    let release: (() => void) | undefined;
    const held = exclusive(() => new Promise<void>((r) => (release = r)));
    await vi.waitFor(() => expect(release).toBeDefined());
    const pass = syncSharedAccounts(deps());
    // Teams turned off from a page while the pass waits
    accounts = [shared("mario@example.com", ["mail"])];
    release!();
    await held;
    await pass;
    expect(calls).toEqual([]);
    expect(slotRow(db, 1)).toBeNull();
  });

  it("a stop that failed is not recorded: the next pass tries it again", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    accounts = [shared("mario@example.com", ["mail"])];
    const busy: ControlClient = {
      ...ctl,
      stop: async () => {
        throw new Error("supervisor busy");
      },
    };
    const r = await syncSharedAccounts(deps({ ctl: busy }));
    expect(r.errors["mario@example.com"]).toMatch(/busy/);
    expect(slotRow(db, 1)?.stopped).toBe(0);
    calls = [];
    await syncSharedAccounts(deps());
    expect(calls).toEqual(["stop 1"]);
    expect(slotRow(db, 1)?.stopped).toBe(1);
  });

  it("an account removed from the calendar stops its slot", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    accounts = [];
    calls = [];
    await syncSharedAccounts(deps());
    expect(calls).toEqual(["stop 1"]);
    expect(slotRow(db, 1)?.stopped).toBe(1);
  });

  it("preserves an existing browser slot when shared Teams is enabled", async () => {
    claimSlot(db, OWNER, { perUser: null });
    const own = claimSlot(db, OWNER, { perUser: null });
    setCheckEvery(db, own, 3600, 1);
    askRefresh(db, own);
    identities = { 1: "anna@example.com", [own]: "mario@example.com" };
    fs.mkdirSync(path.join(dataDir, String(own)), { recursive: true });
    fs.writeFileSync(
      path.join(dataDir, String(own), "light-token.json"),
      JSON.stringify({ region: "amer" }),
    );
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    expect(calls).toEqual([]);
    expect(slotRow(db, own)).toMatchObject({
      shared_account: "",
      check_every: 3600,
    });
    expect(slotRow(db, 1)?.shared_account).toBe("");
    // nothing of its data was wiped
    expect(
      fs.existsSync(path.join(dataDir, String(own), "light-token.json")),
    ).toBe(true);
  });

  it("never takes over a slot of another user, nor one on another computer; a slot busy with a check waits", async () => {
    const other = claimSlot(db, "someone-else", { perUser: null });
    identities = { [other]: "mario@example.com" };
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    expect(slotRow(db, other)?.shared_account).toBe("");
    expect(calls).toContain("light 2");

    const db2 = openAppDb(path.join(tempDir(), "app.db"));
    migrateAppSchema(db2);
    const busy = claimSlot(db2, OWNER, { perUser: null });
    beginCheck(db2, busy, 5);
    calls = [];
    await syncSharedAccounts(
      deps({ db: db2, identity: () => "mario@example.com" }),
    );
    expect(calls).toEqual([]);
    expect(slotRow(db2, busy)?.shared_account).toBe("");
  });

  it("a stopped slot the owner switched off is started again only by Teams on", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    setSlotStopped(db, 1, true);
    calls = [];
    await syncSharedAccounts(deps());
    expect(calls).toEqual(["light 1"]);
  });

  it("the account limit of the owner is an error of that account, the others go on", async () => {
    claimSlot(db, OWNER, { perUser: null });
    accounts = [shared("mario@example.com", ["chat"])];
    const r = await syncSharedAccounts(deps({ perUser: 1 }));
    expect(r.errors["mario@example.com"]).toMatch(/at most 1/);
    expect(calls).toEqual([]);
  });

  it("changes nothing without the list, without an owner, or with an answer of another shape", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    expect(
      await syncSharedAccounts(deps({ dir: dir({ list: async () => null }) })),
    ).toMatchObject({ enabled: false });
    expect(await syncSharedAccounts(deps({ owner: null }))).toMatchObject({
      enabled: false,
    });
    await expect(
      syncSharedAccounts(
        deps({
          dir: dir({
            list: async () => ({
              ok: false,
              errore: "gestore token non configurato",
            }),
          }),
        }),
      ),
    ).rejects.toThrow(/gestore token/);
    await expect(
      syncSharedAccounts(
        deps({ dir: dir({ list: async () => ({ accounts: "x" }) }) }),
      ),
    ).rejects.toThrow();
    expect(calls).toEqual([]);
  });
});

describe("list of the agent", () => {
  it("a mailbox that never chose counts as mail and calendar; an unknown capability is dropped", () => {
    expect(
      parseDirectory({
        ok: true,
        accounts: [
          { account: "A@example.com", shared: null, status: "ok" },
          {
            account: "b@example.com",
            shared: { capabilities: ["chat", "admin"] },
            status: "errore",
            message: "Accesso scaduto",
          },
          { account: "not-an-address", shared: null },
        ],
      }),
    ).toEqual([
      {
        account: "a@example.com",
        capabilities: ["mail", "calendar"],
        shared: false,
        status: "ok",
        message: "",
      },
      {
        account: "b@example.com",
        capabilities: ["chat"],
        shared: true,
        status: "error",
        message: "Accesso scaduto",
      },
    ]);
  });
});

describe("Teams on or off from this app", () => {
  it("adds or removes chat, keeping the rest; a mailbox that never chose keeps mail and calendar", async () => {
    accounts = [
      { account: "legacy@example.com", shared: null },
      shared("mario@example.com", ["mail", "chat"]),
    ];
    const d = dir();
    expect(await setSharedChat(d, "Legacy@Example.com", true)).toEqual([
      "mail",
      "calendar",
      "chat",
    ]);
    expect(await setSharedChat(d, "mario@example.com", false)).toEqual([
      "mail",
    ]);
    expect(d.set.mock.calls.map((c) => c[0])).toEqual([
      {
        account: "legacy@example.com",
        capabilities: ["mail", "calendar", "chat"],
      },
      { account: "mario@example.com", capabilities: ["mail"] },
    ]);
    await expect(
      setSharedChat(d, "nobody@example.com", true),
    ).rejects.toMatchObject({ status: 404 });
  });

  it("one service at a time, on the list as the agent has it now; nothing sent when it is already so; 404 without a list", async () => {
    accounts = [shared("mario@example.com", ["mail", "chat"])];
    const d = dir();
    expect(
      await setSharedCapability(d, "Mario@Example.com", "calendar", true),
    ).toEqual(["mail", "calendar", "chat"]);
    expect(d.set.mock.calls.map((c) => c[0])).toEqual([
      {
        account: "mario@example.com",
        capabilities: ["mail", "calendar", "chat"],
      },
    ]);
    expect(
      await setSharedCapability(d, "mario@example.com", "mail", true),
    ).toEqual(["mail", "chat"]);
    expect(d.set).toHaveBeenCalledTimes(1);
    await expect(
      setSharedCapability(
        dir({ list: async () => null }),
        "mario@example.com",
        "chat",
        false,
      ),
    ).rejects.toMatchObject({ status: 404 });
  });

  it("a refusal of the agent is a 409 with its reason", async () => {
    accounts = [shared("mario@example.com", ["mail"])];
    const d = dir();
    d.set.mockResolvedValueOnce({
      ok: false,
      errore: "Capacità chat non disponibile per questa casella (csa)",
    });
    await expect(
      setSharedChat(d, "mario@example.com", true),
    ).rejects.toMatchObject({
      status: 409,
      message: expect.stringContaining("csa"),
    });
  });
});

describe("report and owner", () => {
  it("each bound slot with its state, and the owner's other slots of this server", async () => {
    accounts = [shared("mario@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    const own = claimSlot(db, OWNER, { perUser: null });
    claimSlot(db, "someone-else", { perUser: null });
    setSlotStopped(db, own, true);
    identities = { [own]: "anna@example.com" };
    const r = sharedReport(db, OWNER, (s) => identities[s.slot] ?? "");
    expect(r.accounts).toEqual([
      {
        account: "mario@example.com",
        slot: 1,
        state: expect.stringMatching(/^[a-z][a-z-]*$/),
        detail: expect.any(String),
        calls: true,
      },
    ]);
    expect(r.others).toEqual([
      { email: "anna@example.com", slot: own, state: "stopped", detail: "" },
    ]);
  });

  it("the owner is found by address, in any case", () => {
    db.exec('CREATE TABLE "user"(id TEXT PRIMARY KEY, email TEXT NOT NULL)');
    db.prepare('INSERT INTO "user"(id, email) VALUES(?, ?)').run(
      OWNER,
      "Owner@Example.com",
    );
    expect(sharedOwnerId(db, "owner@example.com")).toBe(OWNER);
    expect(sharedOwnerId(db, "nobody@example.com")).toBeNull();
    expect(sharedOwnerId(db, "")).toBeNull();
  });
});

describe("owner-bound directory commands", () => {
  it("removes all matching browser/shared slots, never another owner", async () => {
    const own = claimSlot(db, OWNER, { perUser: null });
    const foreign = claimSlot(db, "other", { perUser: null });
    identities[own] = "a@example.com";
    identities[foreign] = "a@example.com";
    const directory = dir({
      list: async () => ({
        ok: true,
        accounts: [],
        commands: [
          { id: "remove-1", account: "a@example.com", operation: "remove" },
        ],
      }),
    });
    const report = vi.mocked(directory.report);
    await syncSharedAccounts(deps({ dir: directory }));
    expect(slotRow(db, own)).toBeNull();
    expect(slotRow(db, foreign)).not.toBeNull();
    expect(calls).toEqual(["stop " + own, "wipe " + own]);
    expect(report).toHaveBeenCalledWith(
      expect.objectContaining({ results: [{ id: "remove-1" }] }),
    );
  });
  it("stops and resumes browser slot without wiping or rebinding its session", async () => {
    const n = claimSlot(db, OWNER, { perUser: null });
    identities[n] = "a@example.com";
    let on = false;
    const directory = dir({
      list: async () => ({
        ok: true,
        accounts: [],
        commands: [
          { id: "chat", account: "a@example.com", operation: "chat", on },
        ],
      }),
    });
    await syncSharedAccounts(deps({ dir: directory }));
    expect(slotRow(db, n)?.stopped).toBe(1);
    on = true;
    await syncSharedAccounts(deps({ dir: directory }));
    expect(slotRow(db, n)?.stopped).toBe(0);
    expect(slotRow(db, n)?.shared_account).toBe("");
    expect(calls).toEqual(["stop " + n, "start " + n]);
  });
  it("switches calls of a slot without a browser and reports the new state with the result", async () => {
    accounts = [shared("a@example.com", ["chat"])];
    await syncSharedAccounts(deps());
    expect(sharedReport(db, OWNER).accounts[0]?.calls).toBe(true);
    const directory = dir({
      list: async () => ({
        ok: true,
        accounts,
        commands: [
          {
            id: "calls",
            account: "a@example.com",
            operation: "calls",
            on: false,
          },
        ],
      }),
    });
    await syncSharedAccounts(deps({ dir: directory }));
    expect(slotRow(db, 1)?.calls).toBe(0);
    expect(directory.report).toHaveBeenCalledWith(
      expect.objectContaining({
        accounts: [
          expect.objectContaining({ account: "a@example.com", calls: false }),
        ],
        results: [{ id: "calls" }],
      }),
    );
  });
  it("refuses a calls switch for a browser slot and for an address with no slot", async () => {
    const n = claimSlot(db, OWNER, { perUser: null });
    identities[n] = "b@example.com";
    const directory = dir({
      list: async () => ({
        ok: true,
        accounts: [],
        commands: [
          {
            id: "browser",
            account: "b@example.com",
            operation: "calls",
            on: false,
          },
          {
            id: "none",
            account: "nobody@example.com",
            operation: "calls",
            on: true,
          },
          { id: "bad", account: "b@example.com", operation: "calls" },
        ],
      }),
    });
    await syncSharedAccounts(deps({ dir: directory }));
    expect(slotRow(db, n)?.calls).toBe(1);
    expect(directory.report).toHaveBeenCalledWith(
      expect.objectContaining({
        results: [
          { id: "browser", error: NO_CALLS_SWITCH },
          { id: "none", error: NO_CALLS_SWITCH },
          { id: "bad", error: NO_CALLS_SWITCH },
        ],
      }),
    );
  });
  it("reports failure without claiming completion or releasing the failed slot", async () => {
    const n = claimSlot(db, OWNER, { perUser: null });
    identities[n] = "a@example.com";
    const directory = dir({
      list: async () => ({
        ok: true,
        accounts: [],
        commands: [
          { id: "remove", account: "a@example.com", operation: "remove" },
        ],
      }),
    });
    await syncSharedAccounts(
      deps({
        dir: directory,
        ctl: {
          ...ctl,
          stop: async () => {
            throw Error("stop failed");
          },
        },
      }),
    );
    expect(slotRow(db, n)).not.toBeNull();
    expect(directory.report).toHaveBeenCalledWith(
      expect.objectContaining({
        results: [{ id: "remove", error: "stop failed" }],
      }),
    );
  });
});

it("removes both browser and bound slots for one identity", async () => {
  const one = claimSlot(db, OWNER, { perUser: null });
  const two = claimSlot(db, OWNER, { perUser: null });
  identities[two] = "a@example.com";
  const { bindShared } = await import("@/lib/appdb");
  bindShared(db, one, "a@example.com", "emea");
  const directory = dir({
    list: async () => ({
      ok: true,
      accounts: [],
      commands: [{ id: "both", account: "a@example.com", operation: "remove" }],
    }),
  });
  await syncSharedAccounts(deps({ dir: directory }));
  expect(slotRow(db, one)).toBeNull();
  expect(slotRow(db, two)).toBeNull();
  expect(calls).toEqual([
    "stop " + one,
    "wipe " + one,
    "stop " + two,
    "wipe " + two,
  ]);
});
it("acknowledges mailbox-only removal without touching unrelated Teams slots", async () => {
  const n = claimSlot(db, OWNER, { perUser: null });
  identities[n] = "other@example.com";
  const directory = dir({
    list: async () => ({
      ok: true,
      accounts: [],
      commands: [{ id: "mail", account: "a@example.com", operation: "remove" }],
    }),
  });
  await syncSharedAccounts(deps({ dir: directory }));
  expect(slotRow(db, n)).not.toBeNull();
  expect(calls).toEqual([]);
  expect(directory.report).toHaveBeenCalledWith(
    expect.objectContaining({ results: [{ id: "mail" }] }),
  );
});

it("rejects malformed directory before executing destructive commands", async () => {
  const n = claimSlot(db, OWNER, { perUser: null });
  identities[n] = "a@example.com";
  const directory = dir({
    list: async () => ({
      ok: false,
      commands: [{ id: "bad", account: "a@example.com", operation: "remove" }],
    }),
  });
  await expect(syncSharedAccounts(deps({ dir: directory }))).rejects.toThrow(
    "Shared accounts not read",
  );
  expect(slotRow(db, n)).not.toBeNull();
  expect(calls).toEqual([]);
});
