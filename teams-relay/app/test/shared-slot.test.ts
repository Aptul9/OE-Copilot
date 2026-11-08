import path from "node:path";
import type Database from "better-sqlite3";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Light } from "@/agent/light/context";
import { readList, renewSignIn } from "@/agent/light/loop";
import { askRefresh, bindShared, claimSlot, migrateAppSchema, openAppDb, setCheckEvery, setLight, sharedBinding, slotRow } from "@/lib/appdb";
import type { SlotPort } from "@/lib/checks";
import type { ControlClient } from "@/lib/control";
import { refreshDue, runRefresh } from "@/lib/light";
import { sharedAccountOf } from "@/lib/slots";
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

beforeEach(() => {
  db = openAppDb(path.join(tempDir(), "app.db"));
  migrateAppSchema(db);
  calls = [];
});

describe("binding of a slot to a shared Microsoft account", () => {
  it("makes it a light account never checked, its account in lower case, emea when no region is known", () => {
    const n = claimSlot(db, "u1", { perUser: null });
    setCheckEvery(db, n, 3600, T0);
    bindShared(db, n, "Mario@Example.com", "");
    expect(slotRow(db, n)).toMatchObject({ light: 1, check_every: 0, check_due: 0, shared_account: "mario@example.com" });
    expect(sharedBinding(db, n)).toEqual({ account: "mario@example.com", region: "emea" });
    expect(sharedAccountOf(n, db)).toBe("mario@example.com");
  });

  it("a slot with its own sign-in has none", () => {
    const n = claimSlot(db, "u1", { perUser: null });
    expect(sharedBinding(db, n)).toBeNull();
    expect(sharedAccountOf(n, db)).toBe("");
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

  it("not right after its binding, before its light agent wrote anything", () => {
    const n = claimSlot(db, "u1", { perUser: null });
    bindShared(db, n, "mario@example.com", "emea");
    expect(refreshDue(slotRow(db, n)!, null, T0)).toBe(false);
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
