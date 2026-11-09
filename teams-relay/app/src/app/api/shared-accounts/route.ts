import { accountSummary } from "@/lib/accounts";
import { appDb, listSlots, setCalls } from "@/lib/appdb";
import { config } from "@/lib/config";
import { controlClient, directoryClient } from "@/lib/control";
import { body, HttpError, route } from "@/lib/http";
import { requireUser } from "@/lib/session";
import {
  NO_CALLS_SWITCH,
  parseDirectory,
  setSharedCapability,
  sharedOwnerId,
  sharedReport,
  syncSharedAccounts,
  type Capability,
} from "@/lib/shared-accounts";

// The one list of accounts, as the calendar shows it: each Microsoft account signed in once in the calendar with mail,
// calendar and Teams on or off, the Teams account running on it, and the owner's Teams accounts with only their own
// browser sign-in. Only the owner of the shared accounts (SHARED_ACCOUNTS_OWNER, by default ADMIN_EMAIL) sees it; for
// anyone else, and without a broker in the browsers container, it is off.
//   GET  /api/shared-accounts                                  {enabled, addUrl, accounts, teamsOnly}
//   POST /api/shared-accounts {account, capability, on}        the same, after the change and the pass that follows it;
//                                                              one service per change, applied to the list as it is now
//   POST /api/shared-accounts {account, operation: "calls", on} calls of its Teams account without a browser on or off

const OFF = { enabled: false, addUrl: "", accounts: [], teamsOnly: [] };

async function listFor(userId: string) {
  const db = appDb();
  if (sharedOwnerId(db) !== userId) return OFF;
  const raw = await directoryClient().list();
  if (raw === null) return OFF;
  const slots = listSlots(db);
  const accounts = parseDirectory(raw).map((a) => {
    const s = slots.find(
      (x) => x.owner_id === userId && x.shared_account === a.account,
    );
    const summary = s ? accountSummary(s) : null;
    return {
      ...a,
      slot: s?.slot ?? 0,
      running: !!s && !s.stopped,
      teams: summary?.teams ?? "",
      overall: summary?.overall ?? "",
      calls: s?.light ? s.calls !== 0 : null,
    };
  });
  const teamsOnly = slots
    .filter((s) => s.owner_id === userId && !s.shared_account && !s.relay)
    .map((s) => {
      const summary = accountSummary(s);
      return {
        slot: s.slot,
        email: summary.email,
        name: summary.name,
        running: !s.stopped,
        teams: summary.teams,
        overall: summary.overall,
        calls: s.light ? s.calls !== 0 : null,
      };
    });
  return {
    enabled: true,
    addUrl: config.sharedAccountsUrl,
    accounts,
    teamsOnly,
    commands: (raw as { commands?: unknown[] }).commands ?? [],
    signins: (raw as { signins?: unknown[] }).signins ?? [],
  };
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
  const { account, capability, on, operation, capabilities } = await body(req);
  if (typeof account !== "string" || !account.includes("@"))
    throw new HttpError(400, "account must be an address");
  if (operation === "connect") {
    if (
      !Array.isArray(capabilities) ||
      capabilities.some((c) => !["mail", "calendar", "chat"].includes(c))
    )
      throw new HttpError(400, "Invalid services");
    const current = await directoryClient().list();
    if (!Array.isArray((current as { signins?: unknown } | null)?.signins))
      throw new HttpError(
        503,
        "Microsoft sign-in service is upgrading or unavailable",
      );
    const result = await directoryClient().set({
      operation: "connect",
      account,
      capabilities,
    });
    if (!(result as { ok?: boolean })?.ok)
      throw new HttpError(
        409,
        String(
          (result as { errore?: string })?.errore || "Sign-in unavailable",
        ),
      );
    return Response.json(await listFor(user.id));
  }
  if (operation === "calls") {
    if (typeof on !== "boolean")
      throw new HttpError(400, "on must be true or false");
    const key = account.trim().toLowerCase();
    const light = listSlots(db).filter(
      (s) =>
        s.owner_id === owner &&
        !s.relay &&
        s.light &&
        (s.shared_account === key ||
          accountSummary(s).email.trim().toLowerCase() === key),
    );
    if (!light.length) throw new HttpError(409, NO_CALLS_SWITCH);
    for (const s of light) setCalls(db, s.slot, on);
    // the calendar shows the new state now, not at the next minute's report
    await directoryClient()
      .report(sharedReport(db, owner))
      .catch(() => null);
    return Response.json(await listFor(user.id));
  }
  const browser = listSlots(db).find(
    (s) =>
      s.owner_id === owner &&
      !s.shared_account &&
      !s.relay &&
      accountSummary(s).email.trim().toLowerCase() ===
        account.trim().toLowerCase(),
  );
  if (operation === "remove" || (capability === "chat" && browser)) {
    if (operation !== "remove" && typeof on !== "boolean")
      throw new HttpError(400, "on must be true or false");
    const result = await directoryClient().set({
      account,
      operation: operation === "remove" ? "remove" : "chat",
      on: typeof on === "boolean" ? on : undefined,
    });
    if (!(result as { ok?: boolean })?.ok)
      throw new HttpError(
        409,
        String((result as { errore?: string })?.errore || "Operation failed"),
      );
    await syncSharedAccounts({
      ctl: controlClient(),
      db,
      dir: directoryClient(),
      dataDir: config.dataDir,
      perUser: config.accountsPerUser,
      owner,
    });
    return Response.json(await listFor(user.id));
  }
  if (
    capability !== "mail" &&
    capability !== "calendar" &&
    capability !== "chat"
  )
    throw new HttpError(400, "capability must be mail, calendar or chat");
  if (typeof on !== "boolean")
    throw new HttpError(400, "on must be true or false");
  if (on && capability === "chat") {
    const browser = listSlots(db).find(
      (s) =>
        s.owner_id === owner &&
        !s.shared_account &&
        !s.relay &&
        accountSummary(s).email.trim().toLowerCase() ===
          account.trim().toLowerCase(),
    );
    if (browser)
      throw new HttpError(
        409,
        "Existing browser sign-in preserved. Shared Teams cannot take over this slot.",
      );
  }
  const dir = directoryClient();
  await setSharedCapability(dir, account, capability as Capability, on);
  const r = await syncSharedAccounts({
    ctl: controlClient(),
    db,
    dir,
    dataDir: config.dataDir,
    perUser: config.accountsPerUser,
    owner,
  });
  const error = r.errors[account.toLowerCase()];
  return Response.json({
    ...(await listFor(user.id)),
    ...(error ? { error } : {}),
  });
});
