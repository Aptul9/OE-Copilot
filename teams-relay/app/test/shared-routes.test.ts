import { accountSummary } from "@/lib/accounts";
// A slot on a shared Microsoft account through the routes of Settings, Check now and the desktop link, and the one list
// of accounts of /api/shared-accounts
import path from "node:path";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { GET as list } from "@/app/api/accounts/route";
import { DELETE, PATCH } from "@/app/api/accounts/[slot]/route";
import { POST as checkNow } from "@/app/api/accounts/[slot]/check/route";
import { GET as desktop } from "@/app/api/desktop/[slot]/route";
import {
  GET as sharedList,
  POST as sharedSet,
} from "@/app/api/shared-accounts/route";
import {
  appDb,
  bindShared,
  claimSlot,
  migrateAppSchema,
  slotRow,
} from "@/lib/appdb";
import {
  controlClient,
  directoryClient,
  type DirectoryClient,
} from "@/lib/control";
import { tempDir } from "./helpers";

const owner = { id: "u1", email: "owner@contoso.example", name: "Owner" };
let user = owner;
vi.mock("@/lib/session", () => ({
  requireAccount: vi.fn(async (_req: Request, slot: string) => Number(slot)),
  requireUser: vi.fn(async () => user),
  currentUser: vi.fn(async () => user),
}));
vi.mock("@/lib/accounts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/accounts")>();
  return { ...actual, accountSummary: vi.fn(actual.accountSummary) };
});
vi.mock("@/lib/control", () => ({
  controlClient: vi.fn(),
  directoryClient: vi.fn(),
}));

let shared: number;
let own: number;
let calls: string[];
let accounts: {
  account: string;
  shared: { capabilities: string[]; origin: string } | null;
}[];
let sets: unknown[];

beforeAll(() => {
  process.env.APP_DB = path.join(tempDir(), "app.db");
  process.env.SHARED_ACCOUNTS_OWNER = "Owner@Contoso.example";
  process.env.SHARED_ACCOUNTS_URL = "https://calendar.example";
  migrateAppSchema(appDb());
  appDb().exec('CREATE TABLE "user"(id TEXT PRIMARY KEY, email TEXT NOT NULL)');
  appDb()
    .prepare('INSERT INTO "user"(id, email) VALUES(?, ?)')
    .run("u1", owner.email);
  shared = claimSlot(appDb(), "u1", { perUser: null });
  bindShared(appDb(), shared, "mario@example.com", "emea");
  own = claimSlot(appDb(), "u1", { perUser: null });
});

beforeEach(() => {
  user = owner;
  calls = [];
  sets = [];
  accounts = [
    {
      account: "mario@example.com",
      shared: { capabilities: ["mail", "chat"], origin: "mobile" },
    },
    { account: "legacy@example.com", shared: null },
  ];
  vi.mocked(controlClient).mockReturnValue({
    start: async (n) => void calls.push(`start ${n}`),
    light: async (n) => void calls.push(`light ${n}`),
    stop: async (n) => void calls.push(`stop ${n}`),
    wipe: async (n) => void calls.push(`wipe ${n}`),
    show: async () => true,
  });
  const dir: DirectoryClient = {
    list: async () => ({ ok: true, accounts }),
    set: async (b) => {
      sets.push(b);
      const a = accounts.find((x) => x.account === b.account)!;
      a.shared = {
        capabilities: "capabilities" in b ? b.capabilities ?? [] : [],
        origin: "mobile",
      };
      return { ok: true, account: b.account, shared: a.shared };
    },
    report: async () => ({ ok: true }),
  };
  vi.mocked(directoryClient).mockReturnValue(dir);
});

const ctx = (n: number) => ({ params: Promise.resolve({ slot: String(n) }) });
const patch = (n: number, body: object) =>
  PATCH(
    new Request(`http://localhost/api/accounts/${n}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
    ctx(n),
  );
const post = (body: object) =>
  sharedSet(
    new Request("http://localhost/api/shared-accounts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
    undefined,
  );
it("never forwards onboarding to an older agent that would interpret it as a capability change", async () => {
  expect((await post({operation:"connect",account:"legacy@example.com",capabilities:["mail"]})).status).toBe(503);
  expect(sets).toEqual([]);
});

describe("a slot on a shared account in Settings", () => {
  it("is listed with its account", async () => {
    const r = (await (
      await list(new Request("http://localhost/api/accounts"), undefined)
    ).json()) as { accounts: { slot: number; shared: string }[] };
    expect(r.accounts.map((a) => [a.slot, a.shared])).toEqual([
      [shared, "mario@example.com"],
      [own, ""],
    ]);
  });

  it("off and on is Teams off and on for its account, in the calendar's list, and the pass follows at once", async () => {
    expect(await (await patch(shared, { running: false })).json()).toEqual({
      ok: true,
      running: false,
      shared: "mario@example.com",
    });
    expect(sets).toEqual([
      { account: "mario@example.com", capabilities: ["mail"] },
    ]);
    expect(calls).toEqual([`stop ${shared}`]);
    expect(slotRow(appDb(), shared)?.stopped).toBe(1);
    expect(await (await patch(shared, { light: true })).json()).toEqual({
      ok: true,
      running: true,
      shared: "mario@example.com",
    });
    expect(sets.at(-1)).toEqual({
      account: "mario@example.com",
      capabilities: ["mail", "chat"],
    });
    expect(calls).toEqual([`stop ${shared}`, `light ${shared}`]);
    expect(slotRow(appDb(), shared)?.stopped).toBe(0);
  });

  it("has no browser mode: a checked mode is refused, never a browser start", async () => {
    expect((await patch(shared, { checkEvery: 3600 })).status).toBe(409);
    expect((await patch(shared, { checkEvery: 0 })).status).toBe(409);
    expect(calls).toEqual([]);
  });

  it("a slot with its own sign-in keeps its own modes", async () => {
    expect((await patch(own, { checkEvery: 0 })).status).toBe(200);
    expect(calls).toEqual([`start ${own}`]);
    expect(sets).toEqual([]);
  });

  it("Check now and the desktop link refuse a browser", async () => {
    const check = await checkNow(
      new Request(`http://localhost/api/accounts/${shared}/check`, {
        method: "POST",
      }),
      ctx(shared),
    );
    expect(check.status).toBe(409);
    expect(slotRow(appDb(), shared)?.refresh_result).not.toBe("asked");
    const open = await desktop(
      new Request(`http://localhost:8090/api/desktop/${shared}`),
      ctx(shared),
    );
    expect(open.status).toBe(409);
    expect(calls).toEqual([]);
  });

  it("an administrator who is not the owner of the shared accounts neither switches nor removes it", async () => {
    user = { id: "u2", email: "admin2@contoso.example", name: "Admin 2" };
    expect((await patch(shared, { running: false })).status).toBe(403);
    const res = await DELETE(
      new Request(`http://localhost/api/accounts/${shared}`, {
        method: "DELETE",
      }),
      ctx(shared),
    );
    expect(res.status).toBe(403);
    expect(sets).toEqual([]);
    expect(calls).toEqual([]);
    expect(slotRow(appDb(), shared)).not.toBeNull();
  });

  it("an account gone from the calendar: its slot still stops and is removed, with nothing to turn off", async () => {
    const gone = claimSlot(appDb(), "u1", { perUser: null });
    bindShared(appDb(), gone, "gone@example.com", "emea");
    expect(await (await patch(gone, { running: false })).json()).toEqual({
      ok: true,
      running: false,
      shared: "gone@example.com",
    });
    expect(calls).toEqual([`stop ${gone}`]);
    expect(slotRow(appDb(), gone)?.stopped).toBe(1);
    // and a server whose browsers container has no broker any more
    vi.mocked(directoryClient).mockReturnValue({
      list: async () => null,
      set: async () => null,
      report: async () => null,
    });
    const res = await DELETE(
      new Request(`http://localhost/api/accounts/${gone}`, {
        method: "DELETE",
      }),
      ctx(gone),
    );
    expect(res.status).toBe(200);
    expect(slotRow(appDb(), gone)).toBeNull();
    expect(sets).toEqual([]);
  });
});

describe("one list of accounts", () => {
  it("the owner sees every account with mail, calendar and Teams, its slot, and the slots with only their own sign-in", async () => {
    const r = (await (
      await sharedList(
        new Request("http://localhost/api/shared-accounts"),
        undefined,
      )
    ).json()) as {
      enabled: boolean;
      addUrl: string;
      accounts: {
        account: string;
        capabilities: string[];
        shared: boolean;
        slot: number;
      }[];
      teamsOnly: { slot: number }[];
    };
    expect(r.enabled).toBe(true);
    expect(r.addUrl).toBe("https://calendar.example");
    expect(
      r.accounts.map((a) => [a.account, a.capabilities, a.shared, a.slot]),
    ).toEqual([
      ["mario@example.com", ["mail", "chat"], true, shared],
      ["legacy@example.com", ["mail", "calendar"], false, 0],
    ]);
    expect(r.teamsOnly.map((t) => t.slot)).toEqual([own]);
  });

  it("anyone else sees nothing and changes nothing", async () => {
    user = { id: "u2", email: "other@contoso.example", name: "Other" };
    expect(
      await (
        await sharedList(
          new Request("http://localhost/api/shared-accounts"),
          undefined,
        )
      ).json(),
    ).toEqual({ enabled: false, addUrl: "", accounts: [], teamsOnly: [] });
    expect(
      (
        await post({
          account: "mario@example.com",
          capability: "chat",
          on: false,
        })
      ).status,
    ).toBe(404);
    expect(sets).toEqual([]);
  });

  it("the owner switches Teams on for a mailbox: a new slot runs on it", async () => {
    const r = (await (
      await post({
        account: "legacy@example.com",
        capability: "chat",
        on: true,
      })
    ).json()) as { accounts: { account: string; slot: number }[] };
    // a mailbox that never chose keeps mail and calendar
    expect(sets).toEqual([
      {
        account: "legacy@example.com",
        capabilities: ["mail", "calendar", "chat"],
      },
    ]);
    const slot = r.accounts.find(
      (a) => a.account === "legacy@example.com",
    )!.slot;
    expect(slot).toBeGreaterThan(own);
    expect(slotRow(appDb(), slot)).toMatchObject({
      shared_account: "legacy@example.com",
      light: 1,
      owner_id: "u1",
    });
    expect(calls).toEqual([`stop ${slot}`, `wipe ${slot}`, `light ${slot}`]);
  });

  it("refuses capabilities it does not know", async () => {
    expect(
      (
        await post({
          account: "mario@example.com",
          capability: "admin",
          on: true,
        })
      ).status,
    ).toBe(400);
    expect(
      (
        await post({
          account: "mario@example.com",
          capability: "chat",
          on: "yes",
        })
      ).status,
    ).toBe(400);
    expect(
      (await post({ account: "mario@example.com", capabilities: ["chat"] }))
        .status,
    ).toBe(400);
    expect(
      (await post({ account: "x", capability: "chat", on: true })).status,
    ).toBe(400);
    expect(sets).toEqual([]);
  });

  it("the owner switches calls of a slot without a browser; a slot with a browser has no switch", async () => {
    type Listed = {
      accounts: { account: string; calls: boolean | null }[];
      teamsOnly: { slot: number; calls: boolean | null }[];
    };
    const read = async (res: Response) => (await res.json()) as Listed;
    const before = await read(
      await sharedList(
        new Request("http://localhost/api/shared-accounts"),
        undefined,
      ),
    );
    expect(
      before.accounts.find((a) => a.account === "mario@example.com")?.calls,
    ).toBe(true);
    expect(before.teamsOnly.find((t) => t.slot === own)?.calls).toBeNull();
    const off = await post({
      account: "Mario@Example.com",
      operation: "calls",
      on: false,
    });
    expect(off.status).toBe(200);
    expect(slotRow(appDb(), shared)?.calls).toBe(0);
    expect(
      (await read(off)).accounts.find((a) => a.account === "mario@example.com")
        ?.calls,
    ).toBe(false);
    expect(
      (await post({ account: "mario@example.com", operation: "calls", on: "no" }))
        .status,
    ).toBe(400);
    expect(
      (
        await post({
          account: "nobody@example.com",
          operation: "calls",
          on: true,
        })
      ).status,
    ).toBe(409);
    expect(sets).toEqual([]);
    expect(
      (await post({ account: "mario@example.com", operation: "calls", on: true }))
        .status,
    ).toBe(200);
    expect(slotRow(appDb(), shared)?.calls).toBe(1);
  });

  it("removing a slot on a shared account turns Teams off for it first; mail stays", async () => {
    const res = await DELETE(
      new Request(`http://localhost/api/accounts/${shared}`, {
        method: "DELETE",
      }),
      ctx(shared),
    );
    expect(res.status).toBe(200);
    expect(sets).toEqual([
      { account: "mario@example.com", capabilities: ["mail"] },
    ]);
    expect(slotRow(appDb(), shared)).toBeNull();
  });
});

it("routes browser Teams control through narrow directory command without takeover", async () => {
  vi.mocked(accountSummary).mockReturnValueOnce({
    email: "LEGACY@example.com",
  } as ReturnType<typeof accountSummary>);
  expect(
    (
      await post({
        account: "legacy@example.com",
        capability: "chat",
        on: true,
      })
    ).status,
  ).toBe(200);
  expect(sets).toEqual([
    { account: "legacy@example.com", operation: "chat", on: true },
  ]);
  expect(calls).not.toContain("start " + own);
});

it("never displays another owner's bound slot", async () => {
  const foreign = claimSlot(appDb(), "foreign", { perUser: null });
  bindShared(appDb(), foreign, "foreign-only@example.com", "emea");
  accounts.push({ account: "foreign-only@example.com", shared: null });
  const response = await sharedList(
    new Request("http://localhost/api/shared-accounts"),
    undefined,
  );
  const result = await response.json();
  expect(
    result.accounts.find(
      (a: { account: string }) => a.account === "foreign-only@example.com",
    ).slot,
  ).toBe(0);
});
