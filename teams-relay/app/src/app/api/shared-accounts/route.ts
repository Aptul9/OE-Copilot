import { accountSummary } from "@/lib/accounts";
import { appDb, listSlots } from "@/lib/appdb";
import { config } from "@/lib/config";
import { controlClient, directoryClient } from "@/lib/control";
import { body, HttpError, route } from "@/lib/http";
import { requireUser } from "@/lib/session";
import { parseDirectory, setSharedCapabilities, sharedOwnerId, syncSharedAccounts, type Capability } from "@/lib/shared-accounts";

// The one list of accounts, as the calendar shows it: each Microsoft account signed in once in the calendar with mail,
// calendar and Teams on or off, the Teams account running on it, and the owner's Teams accounts with only their own
// browser sign-in. Only the owner of the shared accounts (SHARED_ACCOUNTS_OWNER, by default ADMIN_EMAIL) sees it; for
// anyone else, and without a broker in the browsers container, it is off.
//   GET  /api/shared-accounts                                  {enabled, addUrl, accounts, teamsOnly}
//   POST /api/shared-accounts {account, capabilities}          the same, after the change and the pass that follows it

const OFF = { enabled: false, addUrl: "", accounts: [], teamsOnly: [] };

async function listFor(userId: string) {
  const db = appDb();
  if (sharedOwnerId(db) !== userId) return OFF;
  const raw = await directoryClient().list();
  if (raw === null) return OFF;
  const slots = listSlots(db);
  const accounts = parseDirectory(raw).map((a) => {
    const s = slots.find((x) => x.shared_account === a.account);
    const summary = s ? accountSummary(s) : null;
    return { ...a, slot: s?.slot ?? 0, running: !!s && !s.stopped, teams: summary?.teams ?? "", overall: summary?.overall ?? "" };
  });
  const teamsOnly = slots
    .filter((s) => s.owner_id === userId && !s.shared_account && !s.relay)
    .map((s) => {
      const summary = accountSummary(s);
      return { slot: s.slot, email: summary.email, name: summary.name, running: !s.stopped, teams: summary.teams, overall: summary.overall };
    });
  return { enabled: true, addUrl: config.sharedAccountsUrl, accounts, teamsOnly };
}

export const GET = route(async (req) => {
  const user = await requireUser(req);
  return Response.json(await listFor(user.id));
});

export const POST = route(async (req) => {
  const user = await requireUser(req);
  const db = appDb();
  const owner = sharedOwnerId(db);
  if (owner !== user.id) throw new HttpError(404, "Not found");
  const { account, capabilities } = await body(req);
  if (typeof account !== "string" || !account.includes("@")) throw new HttpError(400, "account must be an address");
  if (!Array.isArray(capabilities) || !capabilities.every((c) => c === "mail" || c === "calendar" || c === "chat")) {
    throw new HttpError(400, "capabilities must be a list of mail, calendar, chat");
  }
  const dir = directoryClient();
  await setSharedCapabilities(dir, account, capabilities as Capability[]);
  const r = await syncSharedAccounts({ ctl: controlClient(), db, dir, dataDir: config.dataDir, perUser: config.accountsPerUser, owner });
  const error = r.errors[account.toLowerCase()];
  return Response.json({ ...(await listFor(user.id)), ...(error ? { error } : {}) });
});
