import { appDb, setCalls } from "@/lib/appdb";
import { config } from "@/lib/config";
import { controlClient, directoryClient } from "@/lib/control";
import { body, HttpError, route } from "@/lib/http";
import { requireAccount } from "@/lib/session";
import { setSharedChat, sharedOwnerId, syncSharedAccounts } from "@/lib/shared-accounts";
import { removeAccount, setAccountRunning, setCheckMode, setLightMode, SHARED_NO_BROWSER, sharedAccountOf } from "@/lib/slots";

type Ctx = { params: Promise<{ slot: string }> };

// Teams of a shared account on or off: the calendar agent keeps the choice, the pass that follows it runs now
async function setSharedTeams(account: string, on: boolean) {
  const dir = directoryClient();
  await setSharedChat(dir, account, on);
  const db = appDb();
  await syncSharedAccounts({ ctl: controlClient(), db, dir, dataDir: config.dataDir, perUser: config.accountsPerUser, owner: sharedOwnerId(db) });
}

// Switches the account off ({running: false}) or on again. Stopped, it keeps its Microsoft session and
// data: no sync, no notifications, no commands until it runs again. {checkEvery}: seconds between two checks of an
// account whose browser runs only while it is checked (3600, 7200, 14400), or 0 for always on. {light: true}: no
// browser of its own, its browser runs only to sign it in again (src/lib/light.ts); another mode ends it. {calls}: calls
// of an account with no browser on or off; its light agent starts or stops its calling worker within seconds.
// A slot on a shared Microsoft account (src/lib/shared-accounts.ts) has no browser mode: on or off is Teams on or off for
// that account, the same switch as in the calendar.
export const PATCH = route<Ctx>(async (req, { params }) => {
  const n = await requireAccount(req, (await params).slot);
  const { running, checkEvery, light, calls } = await body(req);
  if (calls !== undefined) {
    if (typeof calls !== "boolean") throw new HttpError(400, "calls must be true or false");
    setCalls(appDb(), n, calls);
    return Response.json({ ok: true, calls });
  }
  const shared = sharedAccountOf(n, appDb());
  if (shared) {
    if (checkEvery !== undefined) throw new HttpError(409, SHARED_NO_BROWSER);
    const on = light === true || running === true;
    if (!on && running !== false) throw new HttpError(400, "running must be true or false");
    await setSharedTeams(shared, on);
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
// Teams off for that account first, or the next pass would add it again; mail and calendar of the account stay.
export const DELETE = route<Ctx>(async (req, { params }) => {
  const n = await requireAccount(req, (await params).slot);
  const shared = sharedAccountOf(n, appDb());
  if (shared) await setSharedChat(directoryClient(), shared, false);
  await removeAccount(n, controlClient(), { db: appDb(), dataDir: config.dataDir });
  return Response.json({ ok: true });
});
