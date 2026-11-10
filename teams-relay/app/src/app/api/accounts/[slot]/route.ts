import { appDb, setCalls, setNotify, slotOwner } from "@/lib/appdb";
import { config } from "@/lib/config";
import { controlClient, directoryClient } from "@/lib/control";
import { body, HttpError, route } from "@/lib/http";
import { requireAccount, requireUser } from "@/lib/session";
import { setSharedChat, sharedOwnerId, syncSharedAccounts } from "@/lib/shared-accounts";
import { removeAccount, setAccountRunning, setCheckMode, setLightMode, SHARED_NO_BROWSER, sharedAccountOf } from "@/lib/slots";

type Ctx = { params: Promise<{ slot: string }> };

// A slot on a shared account changes the calendar's list: only the owner of the shared accounts (SHARED_ACCOUNTS_OWNER,
// by default ADMIN_EMAIL) does it, never an administrator acting on someone else's slot. With no such owner, the slot's.
async function requireSharedOwner(req: Request, n: number) {
  const user = await requireUser(req);
  const db = appDb();
  if ((sharedOwnerId(db) ?? slotOwner(db, n)) !== user.id) throw new HttpError(403, "Only the owner of the shared Microsoft accounts changes this Teams account");
}

// Teams off for an account; false when there is nothing to turn off: the account left the calendar, or this server has
// no shared accounts any more
async function chatOff(account: string): Promise<boolean> {
  try {
    await setSharedChat(directoryClient(), account, false);
    return true;
  } catch (e) {
    if (e instanceof HttpError && e.status === 404) return false;
    throw e;
  }
}

const syncNow = () => {
  const db = appDb();
  return syncSharedAccounts({ ctl: controlClient(), db, dir: directoryClient(), dataDir: config.dataDir, perUser: config.accountsPerUser, owner: sharedOwnerId(db) });
};

// Switches the account off ({running: false}) or on again. Stopped, it keeps its Microsoft session and
// data: no sync, no notifications, no commands until it runs again. {checkEvery}: seconds between two checks of an
// account whose browser runs only while it is checked (3600, 7200, 14400), or 0 for always on. {light: true}: no
// browser of its own, its browser runs only to sign it in again (src/lib/light.ts); another mode ends it. {calls}: calls
// of an account with no browser on or off; its light agent starts or stops its calling worker within seconds.
// {notifyMobile, notifyDesktop}: where its notifications go, one side at a time; any account.
// A slot on a shared Microsoft account (src/lib/shared-accounts.ts) has no browser mode: on or off is Teams on or off for
// that account, the same switch as in the calendar; the pass that follows the change runs now.
export const PATCH = route<Ctx>(async (req, { params }) => {
  const n = await requireAccount(req, (await params).slot);
  const { running, checkEvery, light, calls, notifyMobile, notifyDesktop } = await body(req);
  if (calls !== undefined) {
    if (typeof calls !== "boolean") throw new HttpError(400, "calls must be true or false");
    setCalls(appDb(), n, calls);
    return Response.json({ ok: true, calls });
  }
  if (notifyMobile !== undefined || notifyDesktop !== undefined) {
    for (const [name, v] of [["notifyMobile", notifyMobile], ["notifyDesktop", notifyDesktop]] as const) {
      if (v !== undefined && typeof v !== "boolean") throw new HttpError(400, `${name} must be true or false`);
    }
    setNotify(appDb(), n, { mobile: notifyMobile as boolean | undefined, desktop: notifyDesktop as boolean | undefined });
    return Response.json({ ok: true, notifyMobile, notifyDesktop });
  }
  const shared = sharedAccountOf(n, appDb());
  if (shared) {
    if (checkEvery !== undefined) throw new HttpError(409, SHARED_NO_BROWSER);
    const on = light === true || running === true;
    if (!on && running !== false) throw new HttpError(400, "running must be true or false");
    await requireSharedOwner(req, n);
    if (on) await setSharedChat(directoryClient(), shared, true);
    else if (!(await chatOff(shared))) {
      // nothing left to follow: the slot stops here, its data kept
      await setAccountRunning(n, false, controlClient(), appDb());
      return Response.json({ ok: true, running: false, shared });
    }
    await syncNow();
    return Response.json({ ok: true, running: on, shared });
  }
  if (light !== undefined) {
    if (light !== true) throw new HttpError(400, "light must be true: choose another mode to end it");
    await setLightMode(n, controlClient(), appDb());
    return Response.json({ ok: true, light });
  }
  if (checkEvery !== undefined) {
    if (typeof checkEvery !== "number") throw new HttpError(400, "checkEvery must be a number of seconds");
    await setCheckMode(n, checkEvery, controlClient(), appDb());
    return Response.json({ ok: true, checkEvery });
  }
  if (typeof running !== "boolean") throw new HttpError(400, "running must be true or false");
  await setAccountRunning(n, running, controlClient(), appDb());
  return Response.json({ ok: true, running });
});

// Like "Sign out" in Teams: stops the slot and deletes its Microsoft session and data. A slot on a shared account turns
// Teams off for that account first, or the next pass would add it again; mail and calendar of the account stay. An
// account already gone from the calendar, or a server without shared accounts, has nothing to turn off.
export const DELETE = route<Ctx>(async (req, { params }) => {
  const n = await requireAccount(req, (await params).slot);
  const shared = sharedAccountOf(n, appDb());
  if (shared) {
    await requireSharedOwner(req, n);
    await chatOff(shared);
  }
  await removeAccount(n, controlClient(), { db: appDb(), dataDir: config.dataDir });
  return Response.json({ ok: true });
});
