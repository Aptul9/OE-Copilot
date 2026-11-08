import { accountsOf } from "@/lib/accounts";
import { appDb } from "@/lib/appdb";
import { config, desktopUrlOf } from "@/lib/config";
import { brokerSlots, controlClient } from "@/lib/control";
import { route } from "@/lib/http";
import { requireUser } from "@/lib/session";
import { addAccount, addRelayAccount } from "@/lib/slots";

// shared: the shared Microsoft account the server maps the slot to; a supervisor not reachable maps none
export const GET = route(async (req) => {
  const user = await requireUser(req);
  const list = accountsOf(user.id);
  const mapped = await brokerSlots().catch(() => ({}) as Record<string, { account: string }>);
  return Response.json({ ...list, accounts: list.accounts.map((a) => ({ ...a, shared: mapped[String(a.slot)]?.account ?? "" })) });
});

// New Teams account: first free slot, wiped, browser and agent started. The Microsoft login happens
// afterwards in the remote desktop of the slot. {relay: true}: an account on another computer, whose local relay
// joins with the token of the answer, shown this once.
export const POST = route(async (req) => {
  const user = await requireUser(req);
  const b = (await req.json().catch(() => ({}))) as { relay?: unknown };
  const o = { db: appDb(), dataDir: config.dataDir, perUser: config.accountsPerUser };
  if (b?.relay === true) {
    const { slot, token } = await addRelayAccount(user.id, o);
    return Response.json({ ok: true, slot, token, server: config.appUrl });
  }
  const slot = await addAccount(user.id, controlClient(), o);
  return Response.json({ ok: true, slot, desktop: desktopUrlOf(slot) });
});
