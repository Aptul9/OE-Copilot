// Where the notifications of an account go: its phones (the Android app, the browsers of phones and tablets), its
// browsers, both or none. Account health reaches every device whatever the choice.
import path from "node:path";
import Database from "better-sqlite3";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FcmSender } from "@/agent/push/fcm";
import { deviceSide, Notifier, type Audience, type PushTarget } from "@/agent/push/notifier";
import { newDeviceKey } from "@/agent/push/seal";
import { AppStore } from "@/agent/store/app-store";
import { SlotStore } from "@/agent/store/slot-store";
import { migrateAppSchema, openAppDb } from "@/lib/appdb";
import { tempDir } from "../helpers";

const web = (endpoint: string, kind?: string): PushTarget => ({ endpoint, sub: JSON.stringify({ endpoint, keys: { p256dh: "k", auth: "a" }, ...(kind && { kind }) }) });
const PHONE: PushTarget = { endpoint: "fcm:token-1", sub: JSON.stringify({ fcm: { token: "token-1", key: newDeviceKey(), name: "Pixel", session: "s" } }) };
const MOBILE_BROWSER = web("https://push.test/iphone", "mobile");
const DESKTOP_BROWSER = web("https://push.test/windows", "desktop");
const OLD_BROWSER = web("https://push.test/old");

let store: SlotStore;
let webSent: string[];
let fcmSent: string[];
let audience: Audience;
let ntfy: ReturnType<typeof vi.fn>;

function notifier() {
  return new Notifier({
    store,
    devices: { targets: () => [PHONE, MOBILE_BROWSER, DESKTOP_BROWSER, OLD_BROWSER], remove: () => undefined, audience: () => audience },
    vapid: { publicKey: "BPublic", privateKey: "private" },
    subject: "mailto:admin@example.com",
    ntfy: { url: "https://ntfy.test", topic: "t" },
    send: async (s) => void webSent.push(s.endpoint.replace("https://push.test/", "")),
    fcm: { send: async (token: string) => (fcmSent.push(token), { ok: true }) } as unknown as FcmSender,
  });
}

beforeEach(() => {
  store = SlotStore.open(path.join(tempDir(), "1", "messages.db"));
  webSent = [];
  fcmSent = [];
  audience = { mobile: true, desktop: true };
  ntfy = vi.fn(async () => new Response("ok"));
  vi.stubGlobal("fetch", ntfy);
});

describe("the side of a device", () => {
  it("is mobile for a phone of the Android app and a browser that said mobile; desktop otherwise", () => {
    expect([PHONE, MOBILE_BROWSER, DESKTOP_BROWSER, OLD_BROWSER].map(deviceSide)).toEqual(["mobile", "mobile", "desktop", "desktop"]);
    expect(deviceSide({ endpoint: "https://x", sub: "not json" })).toBe("desktop");
  });
});

describe("a message", () => {
  it("goes to every device when both sides are on", async () => {
    await notifier().message("Anna", "ciao");
    expect(fcmSent).toEqual(["token-1"]);
    expect(webSent.sort()).toEqual(["iphone", "old", "windows"]);
  });

  it("goes to the phones and the browsers of phones only, when only mobile is on", async () => {
    audience = { mobile: true, desktop: false };
    await notifier().message("Anna", "ciao");
    expect(fcmSent).toEqual(["token-1"]);
    expect(webSent).toEqual(["iphone"]);
  });

  it("goes to the browsers of computers only, when only desktop is on", async () => {
    audience = { mobile: false, desktop: true };
    await notifier().message("Anna", "ciao");
    expect(fcmSent).toEqual([]);
    expect(webSent.sort()).toEqual(["old", "windows"]);
  });

  it("goes nowhere when both are off, ntfy included, and is still in the history", async () => {
    audience = { mobile: false, desktop: false };
    await notifier().message("Anna", "ciao");
    expect(fcmSent).toEqual([]);
    expect(webSent).toEqual([]);
    expect(ntfy).not.toHaveBeenCalled();
    expect(store.lastNotificationTs()).toBeGreaterThan(0);
  });
});

describe("a call, a missed call and what a job found", () => {
  it("follow the same sides", async () => {
    audience = { mobile: true, desktop: false };
    const n = notifier();
    await n.call("Anna", "ringing", 1_000);
    await n.missedCall("Anna", "10:05");
    await n.notice("Missed call from Anna", "Teams call, not answered");
    expect(webSent).toEqual(["iphone", "iphone", "iphone"]);
    expect(fcmSent).toEqual(["token-1", "token-1", "token-1"]);
  });

  it("go nowhere when both are off", async () => {
    audience = { mobile: false, desktop: false };
    const n = notifier();
    expect(await n.call("Anna", "ringing", 1_000)).toBe(0);
    expect(await n.missedCall("Anna", "10:05")).toBe(0);
    expect(await n.notice("Found", "by the check")).toBe(0);
    expect(webSent).toEqual([]);
    expect(fcmSent).toEqual([]);
  });
});

describe("an alert about the account itself", () => {
  it("reaches every device, whatever the choice", async () => {
    audience = { mobile: false, desktop: false };
    await notifier().alert("Teams signed out", "Sign in again");
    expect(fcmSent).toEqual(["token-1"]);
    expect(webSent.sort()).toEqual(["iphone", "old", "windows"]);
  });
});

describe("the choice of an account, read by its agent", () => {
  const seed = () => {
    const file = path.join(tempDir(), "app.db");
    const db = openAppDb(file);
    migrateAppSchema(db);
    db.prepare("INSERT INTO teams_accounts(slot, owner_id, added) VALUES(1, 'u1', 0), (2, 'u1', 0)").run();
    db.prepare("UPDATE teams_accounts SET notify_mobile=0 WHERE slot=2").run();
    db.close();
    return file;
  };

  it("reads both sides on by default and the stored values of an account", () => {
    const file = seed();
    expect(new AppStore(file, 1).audience()).toEqual({ mobile: true, desktop: true });
    expect(new AppStore(file, 2).audience()).toEqual({ mobile: false, desktop: true });
  });

  it("reads both on when there is no app.db, no row, or no column yet", () => {
    expect(new AppStore(path.join(tempDir(), "missing.db"), 1).audience()).toEqual({ mobile: true, desktop: true });
    const old = path.join(tempDir(), "app.db");
    const db = new Database(old);
    db.exec("CREATE TABLE teams_accounts(slot INTEGER PRIMARY KEY, owner_id TEXT NOT NULL, added INTEGER NOT NULL)");
    db.prepare("INSERT INTO teams_accounts VALUES(1, 'u1', 0)").run();
    db.close();
    expect(new AppStore(old, 1).audience()).toEqual({ mobile: true, desktop: true });
    expect(new AppStore(old, 9).audience()).toEqual({ mobile: true, desktop: true });
  });

  it("is read again after five seconds, not before", () => {
    const file = seed();
    vi.useFakeTimers();
    try {
      const account = new AppStore(file, 1);
      expect(account.audience().mobile).toBe(true);
      const db = new Database(file);
      db.prepare("UPDATE teams_accounts SET notify_mobile=0 WHERE slot=1").run();
      db.close();
      expect(account.audience().mobile).toBe(true);
      vi.advanceTimersByTime(5_001);
      expect(account.audience().mobile).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
