import fs from "node:fs";
import path from "node:path";
import type Database from "better-sqlite3";
import { accountSummary } from "./accounts";
import { bindShared, claimSlot, endRefresh, listSlots, releaseSlot, setSlotStopped, type Slot } from "./appdb";
import { config } from "./config";
import type { ControlClient, DirectoryClient } from "./control";
import { HttpError } from "./http";
import { exclusive, slotDown, wipeSlot } from "./slots";

// Shared Microsoft accounts: one sign-in, in the OE calendar agent (the Account window of the calendar), for mail,
// calendar and Teams. The agent keeps the list and, per account, which of the three are on; this server follows it. An
// account with Teams on has a slot of the owner running on it without a browser (its light agent asks the agent for
// resource tokens); with Teams off that slot is stopped, its data kept. A slot the owner signed in to that account in its
// own browser is taken over instead of a second one. The owner is the user of SHARED_ACCOUNTS_OWNER, by default the
// administrator of ADMIN_EMAIL: the accounts of the calendar are theirs, no other user sees them.

export type Capability = "mail" | "calendar" | "chat";
const CAPABILITIES: readonly Capability[] = ["mail", "calendar", "chat"];
// a mailbox that never chose: mail and calendar, as before the shared accounts
const LEGACY: Capability[] = ["mail", "calendar"];

export type SharedAccount = {
  account: string;
  capabilities: Capability[];
  // false: the mailbox never chose, it counts as mail and calendar
  shared: boolean;
  // the sign-in needs to be made again in the calendar
  status: "ok" | "error";
  message: string;
};

// The list of the agent (GET /api/auth/shared), checked: an answer of another shape is an error, never an empty list
// that would stop every slot
export function parseDirectory(raw: unknown): SharedAccount[] {
  const r = (raw ?? {}) as { ok?: unknown; accounts?: unknown; errore?: unknown };
  if (r.ok !== true || !Array.isArray(r.accounts)) throw new HttpError(502, `Shared accounts not read: ${typeof r.errore === "string" ? r.errore : "unexpected answer"}`);
  const out: SharedAccount[] = [];
  for (const a of r.accounts as { account?: unknown; shared?: { capabilities?: unknown } | null; status?: unknown; message?: unknown }[]) {
    if (typeof a?.account !== "string" || !a.account.includes("@")) continue;
    const caps = Array.isArray(a.shared?.capabilities) ? (a.shared.capabilities as unknown[]).filter((c): c is Capability => CAPABILITIES.includes(c as Capability)) : null;
    out.push({
      account: a.account.toLowerCase(),
      capabilities: caps ?? LEGACY,
      shared: caps !== null,
      status: a.status === "errore" ? "error" : "ok",
      message: typeof a.message === "string" ? a.message : "",
    });
  }
  return out;
}

// The user the shared accounts belong to, null when SHARED_ACCOUNTS_OWNER (or ADMIN_EMAIL) names nobody
export function sharedOwnerId(db: Database.Database, email = config.sharedAccountsOwner): string | null {
  if (!email) return null;
  const id = db.prepare('SELECT id FROM "user" WHERE lower(email) = lower(?)').pluck().get(email.trim()) as string | undefined;
  return id ?? null;
}

// The chat service region a slot's own browser captured, for a slot taken over; a new slot has none and uses emea, which
// the Teams gateway routes for every region (2026-11-08: the region lookup of authsvc answers 410 ApiRestricted)
function capturedRegion(dataDir: string, n: number): string {
  try {
    const region = (JSON.parse(fs.readFileSync(path.join(dataDir, String(n), "light-token.json"), "utf8")) as { region?: unknown }).region;
    return typeof region === "string" && /^[a-z][a-z0-9]{1,15}$/.test(region) ? region : "emea";
  } catch {
    return "emea";
  }
}

export type SyncDeps = {
  ctl: ControlClient;
  db: Database.Database;
  dir: DirectoryClient;
  dataDir: string;
  perUser: number | null;
  owner: string | null;
  // the Microsoft address a slot is signed in as, "" before its agent read it
  identity?: (s: Slot) => string;
};

export type SyncResult = { enabled: boolean; accounts: SharedAccount[]; errors: Record<string, string> };

const identityOf = (s: Slot) => accountSummary(s).email.toLowerCase();
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

// One pass: every account with Teams on has a running slot of the owner, every slot of an account with Teams off (or
// gone from the list) is stopped. Nothing changes when the list cannot be read. Runs in the queue of the slots.
export async function syncSharedAccounts(d: SyncDeps): Promise<SyncResult> {
  const raw = await d.dir.list();
  if (raw === null || !d.owner) return { enabled: false, accounts: [], errors: {} };
  const accounts = parseDirectory(raw);
  const owner = d.owner;
  const errors: Record<string, string> = {};
  await exclusive(async () => {
    const slots = listSlots(d.db);
    // each slot's address read once per pass, only when an account needs it
    const known = new Map<number, string>();
    const identity = (s: Slot) => {
      if (!known.has(s.slot)) known.set(s.slot, (d.identity ?? identityOf)(s).toLowerCase());
      return known.get(s.slot)!;
    };
    for (const a of accounts) {
      const bound = slots.find((s) => s.shared_account === a.account);
      if (!a.capabilities.includes("chat")) {
        if (bound && !bound.stopped) {
          await slotDown(d.ctl, bound.slot).catch((e: Error) => void (errors[a.account] = e.message));
          setSlotStopped(d.db, bound.slot, true);
        }
        continue;
      }
      try {
        if (bound) {
          if (bound.stopped || !bound.light) {
            bindShared(d.db, bound.slot, a.account, bound.shared_region || "emea");
            await d.ctl.light(bound.slot);
            setSlotStopped(d.db, bound.slot, false);
          }
          continue;
        }
        // a slot of the owner already signed in to this account in its own browser: taken over, not doubled (both would
        // push the same messages). One busy with a check or a sign-in run waits for the next pass.
        const own = slots.find((s) => s.owner_id === owner && !s.shared_account && !s.relay && identity(s) === a.account);
        if (own) {
          if (own.checking || own.refreshing) continue;
          bindShared(d.db, own.slot, a.account, capturedRegion(d.dataDir, own.slot));
          endRefresh(d.db, own.slot, { now: 0, result: null });
          await d.ctl.light(own.slot);
          setSlotStopped(d.db, own.slot, false);
          continue;
        }
        const n = claimSlot(d.db, owner, { perUser: d.perUser });
        try {
          await slotDown(d.ctl, n).catch(() => undefined);
          await wipeSlot(d.ctl, n, { dataDir: d.dataDir });
          bindShared(d.db, n, a.account, "emea");
          await d.ctl.light(n);
        } catch (e) {
          releaseSlot(d.db, n);
          throw e;
        }
      } catch (e) {
        errors[a.account] = message(e);
      }
    }
    // a slot whose account left the list of the calendar (removed there): stopped, its data kept until its owner removes it
    for (const s of slots) {
      if (!s.shared_account || s.stopped || accounts.some((a) => a.account === s.shared_account)) continue;
      await slotDown(d.ctl, s.slot).catch((e: Error) => void (errors[s.shared_account] = e.message));
      setSlotStopped(d.db, s.slot, true);
    }
  });
  return { enabled: true, accounts, errors };
}

// The state of a slot as one word for the calendar: stopped, or what its agent says of Teams (ok, starting, login, ...)
function stateOf(s: Slot): { state: string; detail: string } {
  if (s.stopped) return { state: "stopped", detail: "" };
  const a = accountSummary(s);
  const word = a.teams.toLowerCase().replace(/[^a-z-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 24);
  return { state: /^[a-z]/.test(word) ? word : "unknown", detail: a.overall };
}

// What the calendar shows next to each account: the slot on it and its state, and the owner's slots with only their own
// browser sign-in (the calendar has no mailbox for them)
export function sharedReport(db: Database.Database, owner: string, identity: (s: Slot) => string = identityOf) {
  const accounts: { account: string; slot: number; state: string; detail: string }[] = [];
  const others: { email: string; slot: number; state: string; detail: string }[] = [];
  for (const s of listSlots(db)) {
    if (s.shared_account) accounts.push({ account: s.shared_account, slot: s.slot, ...stateOf(s) });
    else if (s.owner_id === owner && !s.relay) others.push({ email: identity(s), slot: s.slot, ...stateOf(s) });
  }
  return { accounts, others };
}

// Teams on or off for one account, from this app: the agent keeps the choice, the next pass follows it. A mailbox that
// never chose keeps mail and calendar.
export async function setSharedChat(dir: DirectoryClient, account: string, on: boolean): Promise<Capability[]> {
  const a = parseDirectory(await dir.list()).find((x) => x.account === account.toLowerCase());
  if (!a) throw new HttpError(404, `${account} is not in the shared accounts of the calendar`);
  return setSharedCapabilities(dir, a.account, on ? [...new Set([...a.capabilities, "chat" as const])] : a.capabilities.filter((c) => c !== "chat"));
}

export async function setSharedCapabilities(dir: DirectoryClient, account: string, capabilities: Capability[]): Promise<Capability[]> {
  const caps = CAPABILITIES.filter((c) => capabilities.includes(c));
  const r = (await dir.set({ account: account.toLowerCase(), capabilities: caps })) as { ok?: unknown; errore?: unknown; shared?: { capabilities?: unknown } } | null;
  if (r === null) throw new HttpError(404, "No shared Microsoft accounts on this server");
  if (r.ok !== true) throw new HttpError(409, typeof r.errore === "string" ? r.errore : "Capabilities not changed");
  return Array.isArray(r.shared?.capabilities) ? (r.shared.capabilities as Capability[]) : caps;
}

// Every minute: the pass, then the report to the agent. A failure is logged when it changes, not every minute.
export function keepSharedAccountsInSync(d: Omit<SyncDeps, "owner">, everyMs = 60_000) {
  let last = "";
  const say = (line: string) => {
    if (line !== last) console.log(line);
    last = line;
  };
  const tick = async () => {
    try {
      const owner = sharedOwnerId(d.db);
      const r = await syncSharedAccounts({ ...d, owner });
      if (!r.enabled || !owner) return say(owner ? "shared accounts: off (no broker in the browsers container)" : "shared accounts: off (no owner)");
      await d.dir.report(sharedReport(d.db, owner, d.identity));
      const failed = Object.entries(r.errors);
      say(failed.length ? `shared accounts: ${failed.map(([a, e]) => `${a}: ${e}`).join("; ")}` : `shared accounts: ${r.accounts.filter((a) => a.capabilities.includes("chat")).length} with Teams on`);
    } catch (e) {
      say(`shared accounts: ${message(e)}`);
    }
  };
  void tick();
  return setInterval(() => void tick(), everyMs);
}
