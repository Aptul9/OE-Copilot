# Notification targets per account Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each Teams account sends its notifications to Mobile, Desktop, both or none, chosen from a checkbox menu in its Settings row.

**Architecture:** Two integer columns on `teams_accounts` hold the choice. The agents read them through `AppStore.audience()`. `Notifier` filters content pushes by device side (`deviceSide`), a web push subscription carrying a `kind` set by the browser when it subscribes. Account health `alert()` stays unfiltered; two content callers move to a new `notice()`.

**Tech Stack:** TypeScript, Next.js 16, better-sqlite3, web-push, Radix (`radix-ui` DropdownMenu), Tailwind 4, Vitest. Spec: `notify-targets-spec.md`.

## Global Constraints

- Defaults: both columns `INTEGER NOT NULL DEFAULT 1`, so every existing account keeps notifying everywhere.
- Mobile is `fcm:` endpoints plus web push subscriptions with `kind: "mobile"`. Desktop is every other web push subscription. A subscription without `kind` is desktop.
- Off (both false): `message()` still writes its history row; no push and no ntfy. `call()`, `missedCall()` and `notice()` return 0 at once.
- `alert()` is never filtered.
- ntfy is not part of either side: untouched.
- No em dash, no emoji, no first person in code comments. Comments only where behavior is not obvious, matching the surrounding files.
- Commits: Conventional Commits, no AI trailer. PR body empty.

---

### Task 1: Columns, `setNotify`, route, account summary

**Files:**
- Modify: `src/lib/appdb.ts` (type `Slot`, `SLOT_COLUMNS`, migration loop, new `setNotify`)
- Modify: `src/app/api/accounts/[slot]/route.ts`
- Modify: `src/lib/accounts.ts` (`AccountSummary`, `accountSummary`)
- Modify: `test/appdb.test.ts:48`
- Create: `test/notify-targets-route.test.ts`

**Interfaces:**
- Produces: `setNotify(db, slot, { mobile?: boolean; desktop?: boolean }): void`; `Slot.notify_mobile: number`, `Slot.notify_desktop: number`; `AccountSummary.notifyMobile?: boolean`, `AccountSummary.notifyDesktop?: boolean`; `PATCH /api/accounts/[slot]` body `{ notifyMobile?: boolean; notifyDesktop?: boolean }`.

- [ ] **Step 1: Write the failing test** `test/notify-targets-route.test.ts`

```ts
// Where the notifications of an account go, from Settings: stored per account, one side at a time, refused for anything not a boolean
import path from "node:path";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { PATCH } from "@/app/api/accounts/[slot]/route";
import { accountSummary } from "@/lib/accounts";
import { appDb, claimSlot, migrateAppSchema, setNotify, slotRow } from "@/lib/appdb";
import { tempDir } from "./helpers";

vi.mock("@/lib/session", () => ({ requireAccount: vi.fn(async (_req: Request, slot: string) => Number(slot)) }));

let slot: number;
beforeAll(() => {
  process.env.APP_DB = path.join(tempDir(), "app.db");
  migrateAppSchema(appDb());
  slot = claimSlot(appDb(), "u1", { perUser: null });
});

const patch = (body: object) =>
  PATCH(new Request(`http://localhost/api/accounts/${slot}`, { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }), {
    params: Promise.resolve({ slot: String(slot) }),
  });

describe("notification targets of an account", () => {
  it("notify mobile and desktop by default", () => {
    expect(slotRow(appDb(), slot)).toMatchObject({ notify_mobile: 1, notify_desktop: 1 });
  });

  it("change one side without touching the other", async () => {
    expect((await patch({ notifyMobile: false })).status).toBe(200);
    expect(slotRow(appDb(), slot)).toMatchObject({ notify_mobile: 0, notify_desktop: 1 });
    expect((await patch({ notifyDesktop: false })).status).toBe(200);
    expect(slotRow(appDb(), slot)).toMatchObject({ notify_mobile: 0, notify_desktop: 0 });
    expect((await patch({ notifyMobile: true })).status).toBe(200);
    expect(slotRow(appDb(), slot)).toMatchObject({ notify_mobile: 1, notify_desktop: 0 });
  });

  it("refuse a value that is not on or off, and change nothing", async () => {
    expect((await patch({ notifyMobile: "yes" })).status).toBe(400);
    expect((await patch({ notifyMobile: false, notifyDesktop: 0 })).status).toBe(400);
    expect(slotRow(appDb(), slot)).toMatchObject({ notify_mobile: 1, notify_desktop: 0 });
  });

  it("show in the account summary", () => {
    setNotify(appDb(), slot, { mobile: false, desktop: true });
    expect(accountSummary(slotRow(appDb(), slot)!)).toMatchObject({ notifyMobile: false, notifyDesktop: true });
  });
});
```

- [ ] **Step 2: Run it, expect FAIL** (`setNotify` is not exported)

Run: `npx vitest run test/notify-targets-route.test.ts`

- [ ] **Step 3: Implement** in `src/lib/appdb.ts`

In `type Slot`, after `calls: number;`:

```ts
  // where its notifications go: 1 to phones (the Android app, browsers of phones and tablets), 1 to browsers of computers
  notify_mobile: number;
  notify_desktop: number;
```

`SLOT_COLUMNS`: replace `calls, shared_account` with `calls, notify_mobile, notify_desktop, shared_account`.

In the migration list after the `calls` entry:

```ts
    ["notify_mobile", "INTEGER NOT NULL DEFAULT 1"],
    ["notify_desktop", "INTEGER NOT NULL DEFAULT 1"],
```

After `setCalls`:

```ts
// Where the notifications of one account go; only the sides given change
export function setNotify(db: Database.Database, slot: number, to: { mobile?: boolean; desktop?: boolean }) {
  if (to.mobile !== undefined) db.prepare("UPDATE teams_accounts SET notify_mobile=? WHERE slot=?").run(to.mobile ? 1 : 0, slot);
  if (to.desktop !== undefined) db.prepare("UPDATE teams_accounts SET notify_desktop=? WHERE slot=?").run(to.desktop ? 1 : 0, slot);
}
```

In `src/app/api/accounts/[slot]/route.ts`: import `setNotify` with `setCalls`; extend the doc comment with `{notifyMobile, notifyDesktop}: where its notifications go, one side at a time; any account.`; destructure both; after the `calls` block:

```ts
  if (notifyMobile !== undefined || notifyDesktop !== undefined) {
    for (const [name, v] of [["notifyMobile", notifyMobile], ["notifyDesktop", notifyDesktop]] as const) {
      if (v !== undefined && typeof v !== "boolean") throw new HttpError(400, `${name} must be true or false`);
    }
    setNotify(appDb(), n, { mobile: notifyMobile as boolean | undefined, desktop: notifyDesktop as boolean | undefined });
    return Response.json({ ok: true, notifyMobile, notifyDesktop });
  }
```

In `src/lib/accounts.ts`, `AccountSummary` after `calls?: boolean;`:

```ts
  // where its notifications go, on unless switched off in Settings; absent (a server of an earlier release): on
  notifyMobile?: boolean;
  notifyDesktop?: boolean;
```

and in `accountSummary` after `calls: s.calls !== 0,`:

```ts
    notifyMobile: s.notify_mobile !== 0,
    notifyDesktop: s.notify_desktop !== 0,
```

In `test/appdb.test.ts:48` add `notify_mobile: 1, notify_desktop: 1` to the expected row.

- [ ] **Step 4: Run, expect PASS**

Run: `npx vitest run test/notify-targets-route.test.ts test/appdb.test.ts test/account-calls-route.test.ts test/accounts.test.ts`

- [ ] **Step 5: Commit** `feat(teams-relay): per-account notification targets in the account row data`

---

### Task 2: Device kind of a web push subscription

**Files:**
- Modify: `src/shared/push-subscription.ts`
- Modify: `src/lib/push.ts:41`
- Create: `test/push-subscription.test.ts`

**Interfaces:**
- Produces: `type DeviceKind = "mobile" | "desktop"`; `deviceKindOf(userAgent: string, touchPoints: number): DeviceKind`; `subscriptionOf(b)` returns `json` of `{endpoint, keys, kind}`, `kind` from `b.device`.

- [ ] **Step 1: Write the failing test** `test/push-subscription.test.ts`

```ts
import { describe, expect, it } from "vitest";
import { deviceKindOf, subscriptionOf } from "@/shared/push-subscription";

const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const IPAD_AS_MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const ANDROID = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

describe("the kind of device a browser subscribes from", () => {
  it("is mobile for an iPhone, an iPad reporting itself as a Mac with a touch screen, and Chrome on Android", () => {
    expect(deviceKindOf(IPHONE, 5)).toBe("mobile");
    expect(deviceKindOf(IPAD_AS_MAC, 5)).toBe("mobile");
    expect(deviceKindOf(ANDROID, 5)).toBe("mobile");
  });

  it("is desktop for Chrome on Windows and Safari on a Mac with no touch screen", () => {
    expect(deviceKindOf(WINDOWS, 0)).toBe("desktop");
    expect(deviceKindOf(IPAD_AS_MAC, 0)).toBe("desktop");
  });
});

describe("a subscription as stored", () => {
  const keys = { p256dh: "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I99e8QcYP7DkM", auth: "tBHItJI5svbpez7KI4CCXg" };
  const sub = { endpoint: "https://push.example/abc", keys };

  it("keeps endpoint and keys, and says mobile when the browser said so", () => {
    expect(JSON.parse(subscriptionOf({ ...sub, device: "mobile" }).json)).toEqual({ ...sub, kind: "mobile" });
  });

  it("says desktop when the browser said nothing, or something else", () => {
    expect(JSON.parse(subscriptionOf(sub).json)).toEqual({ ...sub, kind: "desktop" });
    expect(JSON.parse(subscriptionOf({ ...sub, device: "toaster" }).json)).toEqual({ ...sub, kind: "desktop" });
  });
});
```

- [ ] **Step 2: Run, expect FAIL** (`deviceKindOf` missing)

Run: `npx vitest run test/push-subscription.test.ts`

- [ ] **Step 3: Implement** in `src/shared/push-subscription.ts`

Replace the last line of `subscriptionOf` and add the helper:

```ts
export type DeviceKind = "mobile" | "desktop";

// Phone or tablet, from what the browser says about itself. iPadOS reports itself as a Mac, with a touch screen.
export function deviceKindOf(userAgent: string, touchPoints: number): DeviceKind {
  return /Android|iPhone|iPad|iPod|Mobile/i.test(userAgent) || (/Macintosh/.test(userAgent) && touchPoints > 1) ? "mobile" : "desktop";
}
```

and in `subscriptionOf`: update the header comment to "kept as JSON of its endpoint, keys and kind (the device it was made on: `device` of the body)", then:

```ts
  const kind: DeviceKind = b.device === "mobile" ? "mobile" : "desktop";
  return { endpoint, json: JSON.stringify({ endpoint, keys: { p256dh: keys.p256dh, auth: keys.auth }, kind }) };
```

In `src/lib/push.ts` add `import { deviceKindOf } from "@/shared/push-subscription";` and replace the post:

```ts
  await post("/api/push/subscribe", { ...sub.toJSON(), device: deviceKindOf(navigator.userAgent, navigator.maxTouchPoints) }, 0);
```

- [ ] **Step 4: Run, expect PASS; then the suites that read stored subscriptions**

Run: `npx vitest run test/push-subscription.test.ts test/appdb.test.ts test/push-fcm-route.test.ts`

- [ ] **Step 5: Commit** `feat(teams-relay): record the kind of device a push subscription comes from`

---

### Task 3: Notifier audience, `notice`, relay op, callers

**Files:**
- Modify: `src/agent/store/app-store.ts` (new `audience()`)
- Modify: `src/agent/push/notifier.ts`
- Modify: `src/shared/relay-sync.ts` (`PushBody`)
- Modify: `src/lib/relay.ts` (`relayNotifier` devices, `relayPush`)
- Modify: `src/local/server-link.ts` (`ServerNotifier.notice`)
- Modify: `src/agent/jobs/missed-calls.ts:48`, `src/agent/commands/check.ts:48`
- Modify: `test/agent/check-command.test.ts`, `test/agent/missed-calls.test.ts`, `test/agent/loop-missed-calls.test.ts`, `test/agent/light-loop.test.ts` (content `alert` mocks become `notice`)
- Create: `test/agent/notify-audience.test.ts`

**Interfaces:**
- Consumes: Task 1 columns; Task 2 `kind` in the subscription JSON.
- Produces: `type Audience = { mobile: boolean; desktop: boolean }`; `PushDevices.audience?(): Audience`; `AppStore.audience(): Audience`; `deviceSide(t: PushTarget): "mobile" | "desktop"`; `Notifier.notice(title: string, body: string, urgency?: webpush.Urgency): Promise<number>`; `Notify` includes `"notice"`; `PushBody` includes `{ op: "notice", title, body, urgency }`.

- [ ] **Step 1: Write the failing test** `test/agent/notify-audience.test.ts`

```ts
// Where the notifications of an account go: its phones (the Android app, the browsers of phones and tablets), its
// browsers, both or none. Account health reaches every device whatever the choice.
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { deviceSide, Notifier, type Audience, type PushTarget } from "@/agent/push/notifier";
import type { FcmSender } from "@/agent/push/fcm";
import { newDeviceKey } from "@/agent/push/seal";
import { SlotStore } from "@/agent/store/slot-store";
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
```

- [ ] **Step 2: Run, expect FAIL** (`deviceSide`, `Audience`, `notice` missing)

Run: `npx vitest run test/agent/notify-audience.test.ts`

- [ ] **Step 3: Implement** `src/agent/push/notifier.ts`

Types and helper, after `PushTarget`:

```ts
export type Audience = { mobile: boolean; desktop: boolean };
```

In `PushDevices` add:

```ts
  // where the notifications of the account go; absent, everywhere
  audience?(): Audience;
```

After `isPhone`:

```ts
// The side a device is on: the phones of the Android app and the browsers that subscribed from a phone or tablet
// (kind of the subscription, src/shared/push-subscription.ts) are mobile, every other browser is desktop
export function deviceSide(t: PushTarget): "mobile" | "desktop" {
  if (isPhone(t)) return "mobile";
  try {
    return (JSON.parse(t.sub) as { kind?: unknown }).kind === "mobile" ? "mobile" : "desktop";
  } catch {
    return "desktop";
  }
}
```

`Notify`: add `"notice"` to the `Pick`.

In `Notifier`: private helpers after the constructor:

```ts
  private audience(): Audience {
    return this.o.devices.audience?.() ?? { mobile: true, desktop: true };
  }

  private off(): boolean {
    const a = this.audience();
    return !a.mobile && !a.desktop;
  }
```

`message()`:

```ts
    this.o.store.addNotification(title, body);
    if (this.off()) return;
    await this.ntfy(title, body);
```

`alert()` sends to everyone:

```ts
  async alert(title: string, body: string, urgency: webpush.Urgency = "high"): Promise<number> {
    await this.ntfy(title, body);
    return this.push(title, body, "", urgency, "", 0, true);
  }

  // What a job found, like a message: through the choice of the account, nothing when both sides are off
  async notice(title: string, body: string, urgency: webpush.Urgency = "high"): Promise<number> {
    if (this.off()) return 0;
    await this.ntfy(title, body);
    return this.push(title, body, "", urgency);
  }
```

`push()` gets a final `everyone = false` parameter and passes it:

```ts
  async push(title: string, body: string, chat = "", urgency: webpush.Urgency = "high", group = "", ts = 0, everyone = false): Promise<number> {
    return this.deliver((acc) => ({ ... }), { urgency, ttl: PUSH_TTL, retry: true, everyone });
  }
```

`call()`: first line `if (this.off()) return 0;`. `missedCall()`: first line `if (this.off()) return 0;`.

`deliver()` option type gains `everyone?: boolean` and the target filter becomes:

```ts
    const side = o.everyone ? null : this.audience();
    const targets = devices.targets().filter((t) => !o.skip?.(t) && (!side || side[deviceSide(t)]));
```

`src/agent/store/app-store.ts`, after `callsOn()`:

```ts
  // Where the notifications of this account go (teams_accounts.notify_mobile and notify_desktop, on unless switched off
  // in Settings); on when the row or its columns cannot be read. Read at most every 5 s.
  private sides = { at: 0, value: { mobile: true, desktop: true } };
  audience(): Audience {
    if (Date.now() - this.sides.at > 5_000) {
      const row = this.use((db) => db.prepare("SELECT * FROM teams_accounts WHERE slot=?").get(this.slot) as { notify_mobile?: number; notify_desktop?: number } | undefined, undefined);
      this.sides = {
        at: Date.now(),
        value: { mobile: row?.notify_mobile === undefined || Number(row.notify_mobile) > 0, desktop: row?.notify_desktop === undefined || Number(row.notify_desktop) > 0 },
      };
    }
    return this.sides.value;
  }
```

with `import { isPhone, type Audience, type PushDevices, type PushTarget } from "../push/notifier";`.

`src/shared/relay-sync.ts`: add to `PushBody` after the `alert` line:

```ts
  z.object({ op: z.literal("notice"), title: text(1000), body: text(20_000), urgency: z.enum(["very-low", "low", "normal", "high"]) }),
```

`src/lib/relay.ts`: in the `devices` literal add `audience: () => app.audience(),`; in `relayPush` add before `case "call"`:

```ts
    case "notice":
      return n.notice(b.title, b.body, b.urgency);
```

`src/local/server-link.ts`, after `alert` in `ServerNotifier`:

```ts
  notice(title: string, body: string, urgency: webpush.Urgency = "high"): Promise<number> {
    return this.link.push({ op: "notice", title, body, urgency });
  }
```

Callers: `src/agent/jobs/missed-calls.ts:48` and `src/agent/commands/check.ts:48`: `a.notifier.alert(` becomes `a.notifier.notice(`; the comments above them say "alerts" for missed calls: reword to "pushes" only if they name the method.

Existing tests: in `check-command.test.ts`, `missed-calls.test.ts`, `loop-missed-calls.test.ts` rename the mocked `alert:` key to `notice:`; in `light-loop.test.ts` rename the `alert` mock and its `notifier.alert` assertions for missed calls to `notice`, leaving the sign-out alert assertions on `alert` (the mock keeps both).

- [ ] **Step 4: Run, expect PASS**

Run: `npx vitest run test/agent/notify-audience.test.ts test/agent/check-command.test.ts test/agent/missed-calls.test.ts test/agent/loop-missed-calls.test.ts test/agent/light-loop.test.ts test/agent/push.test.ts test/relay-push.test.ts`

- [ ] **Step 5: AppStore and relay tests.** Append to `test/agent/notify-audience.test.ts`:

```ts
import Database from "better-sqlite3";
import { AppStore } from "@/agent/store/app-store";
import { migrateAppSchema, openAppDb } from "@/lib/appdb";

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
      const store = new AppStore(file, 1);
      expect(store.audience().mobile).toBe(true);
      const db = new Database(file);
      db.prepare("UPDATE teams_accounts SET notify_mobile=0 WHERE slot=1").run();
      db.close();
      expect(store.audience().mobile).toBe(true);
      vi.advanceTimersByTime(5_001);
      expect(store.audience().mobile).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
```

Add to `test/relay-push.test.ts` (inside the existing describe, after the first test):

```ts
  it("keeps what a job found from the phones of an account that is off, and sends its account health", async () => {
    process.env.FCM_CREDENTIALS = keyFile;
    const sentTo = google();
    await phone("phone-token-relay-off-eeeeeeeee");
    const { slot, caller } = await relayAccount();
    appDb().prepare("UPDATE teams_accounts SET notify_mobile=0, notify_desktop=0 WHERE slot=?").run(slot);
    expect(await relayPush(caller, { op: "notice", title: "Missed call", body: "Teams call, not answered", urgency: "high" })).toBe(0);
    expect(sentTo("phone-token-relay-off-eeeeeeeee")).toEqual([]);
    expect(await relayPush(caller, { op: "alert", title: "Teams signed out", body: "Sign in", urgency: "high" })).toBeGreaterThanOrEqual(1);
    expect(sentTo("phone-token-relay-off-eeeeeeeee")).toHaveLength(1);
  });
```

The notifier of this account is made on its first push, so its first read of the audience sees the update.

Run: `npx vitest run test/agent/notify-audience.test.ts test/relay-push.test.ts`

- [ ] **Step 6: Commit** `feat(teams-relay): notifications follow the mobile and desktop choice of the account`

---

### Task 4: Checkbox menu in the Teams accounts row

**Files:**
- Modify: `src/components/ui/dropdown-menu.tsx` (`DropdownMenuCheckboxItem`)
- Create: `src/components/NotifyMenu.tsx`
- Modify: `src/components/Settings.tsx`

**Interfaces:**
- Consumes: `Account.notifyMobile?`, `Account.notifyDesktop?`; `patch` of `@/lib/client`.
- Produces: `NotifyMenu({ account, onChange }: { account: Account; onChange: (to: { notifyMobile?: boolean; notifyDesktop?: boolean }) => void })`.

- [ ] **Step 1: The checkbox item.** In `DropdownMenuCheckboxItem` replace the classes `py-1 pr-8 pl-1.5` with `py-1.5 pr-1.5 pl-8` and the indicator span with a box on the left:

```tsx
      <span
        className="pointer-events-none absolute left-2 flex size-4 items-center justify-center rounded-[4px] border border-input bg-card transition-colors group-data-[state=checked]/checkbox-item:border-primary group-data-[state=checked]/checkbox-item:bg-primary"
        data-slot="dropdown-menu-checkbox-item-indicator"
      >
        <DropdownMenuPrimitive.ItemIndicator>
          <CheckIcon className="size-3 text-primary-foreground" />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
```

and add `group/checkbox-item` to the item class list.

- [ ] **Step 2: `src/components/NotifyMenu.tsx`**

```tsx
"use client";

import { BellOffIcon, ChevronDownIcon, MonitorIcon, MonitorSmartphoneIcon, SmartphoneIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { accName, type Account } from "@/lib/client";

// Where the notifications of an account go: tick Mobile, Desktop or both, none ticked is off
export function NotifyMenu({ account, onChange }: { account: Account; onChange: (to: { notifyMobile?: boolean; notifyDesktop?: boolean }) => void }) {
  const mobile = account.notifyMobile !== false;
  const desktop = account.notifyDesktop !== false;
  const Icon = mobile && desktop ? MonitorSmartphoneIcon : mobile ? SmartphoneIcon : desktop ? MonitorIcon : BellOffIcon;
  const word = mobile && desktop ? "Both" : mobile ? "Mobile" : desktop ? "Desktop" : "Off";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" className="h-10 w-full justify-between gap-1.5 px-2.5 font-normal md:h-9 sm:w-36" aria-label={`Notifications of ${accName(account)}`}>
          <span className="flex items-center gap-1.5">
            <Icon />
            {word}
          </span>
          <ChevronDownIcon className="text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-48">
        <DropdownMenuCheckboxItem checked={mobile} onCheckedChange={(on) => onChange({ notifyMobile: on === true })} onSelect={(e) => e.preventDefault()}>
          <SmartphoneIcon className="text-muted-foreground" />
          Mobile
        </DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem checked={desktop} onCheckedChange={(on) => onChange({ notifyDesktop: on === true })} onSelect={(e) => e.preventDefault()}>
          <MonitorIcon className="text-muted-foreground" />
          Desktop
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
```

- [ ] **Step 3: `Settings.tsx`.** Import `NotifyMenu`. In `TeamsAccounts`, after `setCalls`:

```tsx
  // where the notifications of an account go: the box changes at once, and goes back when the server refuses
  async function setNotify(a: Account, to: { notifyMobile?: boolean; notifyDesktop?: boolean }) {
    const before = accounts;
    setAccounts((list) => list && list.map((x) => (x.slot === a.slot ? { ...x, ...to } : x)));
    const now = { notifyMobile: a.notifyMobile !== false, notifyDesktop: a.notifyDesktop !== false, ...to };
    try {
      await patch(`/api/accounts/${a.slot}`, to, 0);
      const words = now.notifyMobile && now.notifyDesktop ? "mobile and desktop" : now.notifyMobile ? "mobile only" : now.notifyDesktop ? "desktop only" : "notifications off";
      toast.success(`${accName(a)}: ${words}`, {
        id: `notify-${a.slot}`,
        description: now.notifyMobile || now.notifyDesktop ? "Messages, calls and what a check finds go to those devices." : "Nothing from this account. Sign-in alerts still arrive.",
      });
    } catch (e) {
      setAccounts(before);
      toast.error(errorText(e, "Notifications not changed"), { id: `notify-${a.slot}` });
    }
  }
```

Render `<NotifyMenu account={a} onChange={(to) => void setNotify(a, to)} />` in both control groups: in the relay one after the "New token" `AlertDialog`; in the other one after the status select block and the shared-account note, before the Calls label. Description sentence added to the `CardDescription`: `Notifications: tick Mobile, Desktop or both; with none ticked the account sends no notifications.` and the fold summary text becomes `Teams accounts: notifications, run mode and relay tokens`.

- [ ] **Step 4: Typecheck, lint**

Run: `npx tsc --noEmit` then `npx eslint src/components src/agent src/lib src/shared src/local`

- [ ] **Step 5: Commit** `feat(teams-relay): Mobile and Desktop checkbox menu in the Teams accounts row`

---

### Task 5: Verification

- [ ] **Step 1:** `npx vitest run` (whole suite), `npx tsc --noEmit`, `npx eslint`, `npm run build`. Expected: all green, no new warnings.
- [ ] **Step 2:** Local build: gateway with a throwaway `data/` folder, two accounts seeded, Settings open in Chrome through the extension at 1280 px and 360 px: the menu opens, ticks save and survive a reload, none ticked reads Off, a refused save (server stopped) rolls the box back.
- [ ] **Step 3:** Push `feat/teams-notification-targets`, open the PR with an empty body. Merge deploys: wait for the owner.
