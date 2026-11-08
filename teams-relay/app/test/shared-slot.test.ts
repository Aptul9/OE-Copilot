import path from "node:path";
import type Database from "better-sqlite3";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Light } from "@/agent/light/context";
import { readList, renewSignIn } from "@/agent/light/loop";
import { askRefresh, claimSlot, migrateAppSchema, openAppDb, setCheckEvery, setLight, setRelayToken, setSlotStopped, slotRow } from "@/lib/appdb";
import type { SlotPort } from "@/lib/checks";
import type { ControlClient } from "@/lib/control";
import { refreshDue, runRefresh } from "@/lib/light";
import { ON_ANOTHER_COMPUTER } from "@/lib/relay";
import { setSharedMode, sharedAccountOf } from "@/lib/slots";
import { LightSignIn, parseState, STATE } from "@/shared/slot-db/state";
import { tempDir } from "./helpers";

const T0 = 1_790_000_000;
let db: Database.Database;
let calls: string[];

const ctl: ControlClient = {
  start: async (n) => void calls.push(`start ${n}`),
  light: async (n) => void calls.push(`light ${n}`),
  stop: async (n) => void calls.push(`stop ${n}`),
  wipe: async () => undefined,
  show: async () => true,
};
const mapped = async () => ({ "1": { account: "mario@example.com" } });

beforeEach(() => {
  db = openAppDb(path.join(tempDir(), "app.db"));
  migrateAppSchema(db);
  calls = [];
});

describe("opt-in of a slot to its shared Microsoft account", () => {
  it("runs the light agent of a mapped slot, back in service, any browser run forgotten", async () => {
    const n = claimSlot(db, "u1", { perUser: null });
    setCheckEvery(db, n, 3600, T0);
    setSlotStopped(db, n, true);
    askRefresh(db, n);
    expect(await setSharedMode(n, ctl, db, mapped)).toBe("mario@example.com");
    expect(calls).toEqual([`light ${n}`]);
    expect(slotRow(db, n)).toMatchObject({ light: 1, stopped: 0, check_every: 0, refreshing: 0, refresh_result: "" });
  });

  it("refuses a slot the server did not map, and changes nothing", async () => {
    claimSlot(db, "u1", { perUser: null });
    const n = claimSlot(db, "u1", { perUser: null });
    await expect(setSharedMode(n, ctl, db, mapped)).rejects.toMatchObject({ status: 409 });
    expect(calls).toEqual([]);
    expect(slotRow(db, n)).toMatchObject({ light: 0, stopped: 0 });
    expect(await sharedAccountOf(n, mapped)).toBe("");
  });

  it("refuses an account on another computer", async () => {
    const n = claimSlot(db, "u1", { perUser: null });
    setRelayToken(db, n, "digest");
    await expect(setSharedMode(n, ctl, db, mapped)).rejects.toThrow(ON_ANOTHER_COMPUTER);
    expect(calls).toEqual([]);
  });
});

describe("a slot on a shared account never gets a browser run", () => {
  const sign = { at: T0, expiresOn: 0, refused: true, broker: true } satisfies LightSignIn;

  it("not when its token is refused", () => {
    const n = claimSlot(db, "u1", { perUser: null });
    setLight(db, n, true);
    expect(refreshDue(slotRow(db, n)!, sign, T0)).toBe(false);
    expect(refreshDue(slotRow(db, n)!, { ...sign, broker: undefined }, T0)).toBe(true);
  });

  it("an ask is forgotten without starting anything, so the keep-alive runs its light agent again", async () => {
    const n = claimSlot(db, "u1", { perUser: null });
    setLight(db, n, true);
    askRefresh(db, n);
    expect(refreshDue(slotRow(db, n)!, sign, T0)).toBe(true);
    const port: SlotPort = { health: () => null, signInAlert: () => ({ armed: true, alerted: false }), enqueue: () => 1, commandStatus: () => null, lightSignIn: () => sign };
    expect(await runRefresh(n, { ctl, db, slot: port, now: () => T0 * 1000, sleep: async () => undefined })).toBeNull();
    expect(calls).toEqual([]);
    expect(slotRow(db, n)).toMatchObject({ refreshing: 0, refresh_result: "" });
  });
});

describe("light agent on a shared account", () => {
  function light(upn: string, savedEmail = "") {
    const state = new Map<string, string>();
    if (savedEmail) state.set(STATE.me, JSON.stringify({ name: "Mario", email: savedEmail, tenant: "", av: "", logo: "" }));
    const l = {
      config: { sharedBroker: { url: "http://agente", key: "k", account: "Mario@Example.com", region: "amer" }, lightTokenFile: "unused" },
      token: { brokerBacked: true },
      store: { getState: (k: string) => state.get(k) ?? "", setState: (k: string, v: string) => void state.set(k, v) },
      api: { auth: vi.fn(async () => ({ token: "t", region: "amer", oid: "oid", upn, name: "Mario" })), chats: vi.fn(async () => []) },
      me: { oid: "", name: "" },
      signedOut: false,
      error: "",
    } as unknown as Light;
    return { l, state };
  }

  it("marks the slot as broker-backed, with nothing stored to renew", async () => {
    const { l, state } = light("mario@example.com");
    await renewSignIn(l, { for: 0, retryAt: 0 });
    expect(parseState(LightSignIn, state.get(STATE.lightToken)!)).toMatchObject({ broker: true, refused: false });
  });

  it("clears a refusal once the broker gives tokens again", async () => {
    const { l, state } = light("mario@example.com");
    state.set(STATE.lightToken, JSON.stringify({ at: 1, expiresOn: 0, refused: true, broker: true }));
    l.signedOut = true;
    await renewSignIn(l, { for: 0, retryAt: 0 });
    expect(parseState(LightSignIn, state.get(STATE.lightToken)!).refused).toBe(true);
    l.signedOut = false;
    await renewSignIn(l, { for: 0, retryAt: 0 });
    expect(parseState(LightSignIn, state.get(STATE.lightToken)!).refused).toBe(false);
  });

  it.each([
    ["a token of another account", "anna@example.com", ""],
    ["a slot signed in as someone else", "mario@example.com", "anna@example.com"],
  ])("refuses %s: signed out, no chat read", async (_n, upn, saved) => {
    const { l, state } = light(upn, saved);
    expect(await readList(l)).toBe(false);
    expect(l.signedOut).toBe(true);
    expect(l.me.oid).toBe("");
    expect(parseState(LightSignIn, state.get(STATE.lightToken) ?? "{}").refused).toBe(true);
    expect((l.api as unknown as { chats: ReturnType<typeof vi.fn> }).chats).not.toHaveBeenCalled();
  });
});
