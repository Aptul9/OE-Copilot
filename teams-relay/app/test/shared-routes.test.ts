// A slot the server maps to a shared Microsoft account, through the routes of Settings, Check now and the desktop link
import path from "node:path";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { GET as list } from "@/app/api/accounts/route";
import { PATCH } from "@/app/api/accounts/[slot]/route";
import { POST as checkNow } from "@/app/api/accounts/[slot]/check/route";
import { GET as desktop } from "@/app/api/desktop/[slot]/route";
import { appDb, claimSlot, migrateAppSchema, setLight, slotRow } from "@/lib/appdb";
import { brokerSlots, controlClient } from "@/lib/control";
import { tempDir } from "./helpers";

const user = { id: "u1", email: "u1@contoso.example", name: "U1" };
vi.mock("@/lib/session", () => ({
  requireAccount: vi.fn(async (_req: Request, slot: string) => Number(slot)),
  requireUser: vi.fn(async () => user),
  currentUser: vi.fn(async () => user),
}));
vi.mock("@/lib/control", () => ({ controlClient: vi.fn(), brokerSlots: vi.fn() }));

let shared: number;
let own: number;
let calls: string[];

beforeAll(() => {
  process.env.APP_DB = path.join(tempDir(), "app.db");
  migrateAppSchema(appDb());
  shared = claimSlot(appDb(), "u1", { perUser: null });
  own = claimSlot(appDb(), "u1", { perUser: null });
});

beforeEach(() => {
  calls = [];
  vi.mocked(brokerSlots).mockResolvedValue({ [String(shared)]: { account: "mario@example.com" } });
  vi.mocked(controlClient).mockReturnValue({
    start: async (n) => void calls.push(`start ${n}`),
    light: async (n) => void calls.push(`light ${n}`),
    stop: async (n) => void calls.push(`stop ${n}`),
    wipe: async () => undefined,
    show: async () => true,
  });
});

const patch = (n: number, body: object) =>
  PATCH(new Request(`http://localhost/api/accounts/${n}`, { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }), {
    params: Promise.resolve({ slot: String(n) }),
  });

describe("shared Microsoft account of a slot", () => {
  it("is listed with each account", async () => {
    const r = (await (await list(new Request("http://localhost/api/accounts"), undefined)).json()) as { accounts: { slot: number; shared: string }[] };
    expect(r.accounts.map((a) => [a.slot, a.shared])).toEqual([
      [shared, "mario@example.com"],
      [own, ""],
    ]);
  });

  it("a supervisor not reachable maps none", async () => {
    vi.mocked(brokerSlots).mockRejectedValue(new Error("not reachable"));
    const r = (await (await list(new Request("http://localhost/api/accounts"), undefined)).json()) as { accounts: { shared: string }[] };
    expect(r.accounts.every((a) => a.shared === "")).toBe(true);
  });

  it("the owner opts in: its light agent runs on the shared account", async () => {
    const r = await patch(shared, { shared: true });
    expect(await r.json()).toEqual({ ok: true, shared: "mario@example.com" });
    expect(calls).toEqual([`light ${shared}`]);
    expect(slotRow(appDb(), shared)).toMatchObject({ light: 1, stopped: 0 });
  });

  it("a start or no browser mean the same; a checked mode is refused", async () => {
    expect(await (await patch(shared, { running: true })).json()).toEqual({ ok: true, shared: "mario@example.com" });
    expect(await (await patch(shared, { light: true })).json()).toEqual({ ok: true, shared: "mario@example.com" });
    expect((await patch(shared, { checkEvery: 3600 })).status).toBe(409);
    expect((await patch(shared, { checkEvery: 0 })).status).toBe(409);
    expect(calls).toEqual([`light ${shared}`, `light ${shared}`]);
    expect(calls).not.toContain(`start ${shared}`);
  });

  it("an unmapped slot cannot opt in and keeps its own modes", async () => {
    expect((await patch(own, { shared: true })).status).toBe(409);
    expect(slotRow(appDb(), own)?.light).toBe(0);
    expect((await patch(own, { checkEvery: 0 })).status).toBe(200);
    expect(calls).toEqual([`start ${own}`]);
  });

  it("Check now and the desktop link refuse a browser", async () => {
    setLight(appDb(), shared, true);
    const check = await checkNow(new Request(`http://localhost/api/accounts/${shared}/check`, { method: "POST" }), { params: Promise.resolve({ slot: String(shared) }) });
    expect(check.status).toBe(409);
    expect(slotRow(appDb(), shared)?.refresh_result).not.toBe("asked");
    const open = await desktop(new Request(`http://localhost:8090/api/desktop/${shared}`), { params: Promise.resolve({ slot: String(shared) }) });
    expect(open.status).toBe(409);
    expect(calls).toEqual([]);
  });
});
