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
