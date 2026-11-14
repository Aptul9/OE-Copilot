import {
  claimSlot,
  listSlots,
  setSlotStopped,
  slotRow,
  releaseSlot,
  askRefresh,
  setLight,
} from "./appdb";
import { config } from "./config";
import { slotDown, startLight, wipeSlot } from "./slots";
import type { SyncDeps } from "./shared-accounts";
import { accountSummary, saveMailboxIdentity } from "./accounts";

type SignIn = {
  id: string;
  account: string;
  phase: string;
  slot?: number;
  capabilities: string[];
  route?: string;
  created?: number;
  ended?: number;
};

// the agent's timeout of a sign-in (agente-ts signin.ts), for a stopped one that does not say when it stopped
const SIGNIN_TIMEOUT_S = 300;

// Runs inside the existing slot queue. Progress is persisted before a browser starts, so a restart never allocates it twice.
export async function syncSignIns(d: SyncDeps, input: unknown) {
  if (!Array.isArray(input) || !d.owner || !d.dir.connect) return;
  const jobs = input as SignIn[];
  const identity = d.identity ?? ((s) => accountSummary(s).email);
  const reservedElsewhere = (job: SignIn, slot: number) =>
    jobs.some(
      (other) =>
        other.id !== job.id &&
        other.slot === slot &&
        ["queued", "browser"].includes(other.phase),
    );
  for (const job of jobs) {
    if (job.phase === "blocked" && job.slot) {
      const row = slotRow(d.db, job.slot);
      const address = row ? identity(row).trim().toLowerCase() : "";
      const ours =
        row?.owner_id === d.owner &&
        !row.relay &&
        !row.shared_account &&
        !reservedElsewhere(job, job.slot);
      // a slot claimed for this sign-in (between its start and its end) that never signed in: given back, so no nameless
      // account stays behind. A slot claimed later, for another account, is never touched.
      const claimedForIt =
        !!job.created &&
        !!row &&
        row.added >= job.created &&
        row.added <= (job.ended ?? job.created + SIGNIN_TIMEOUT_S);
      if (ours && !address && claimedForIt) {
        await slotDown(d.ctl, job.slot);
        await wipeSlot(d.ctl, job.slot, d);
        releaseSlot(d.db, job.slot);
      } else if (ours && row?.stopped && (!address || address === job.account))
        await d.ctl.stop(job.slot);
    }
    if (!["queued", "browser"].includes(job.phase)) continue;
    let slot = job.slot;
    const progress = async (phase: string, error?: string) => {
      const r = await d.dir.set({
        operation: "signin-progress",
        account: job.account,
        id: job.id,
        slot,
        phase,
        ...(slot ? { url: `${config.appUrl}/api/desktop/${slot}` } : {}),
        ...(error ? { error } : {}),
      });
      if (!(r as { ok?: boolean })?.ok)
        throw new Error("Could not persist sign-in progress");
    };
    try {
      const existing = listSlots(d.db).find(
        (s) =>
          s.owner_id === d.owner &&
          !s.relay &&
          !s.shared_account &&
          identity(s).trim().toLowerCase() === job.account,
      );
      if (slot) {
        const hinted = slotRow(d.db, slot);
        const address = hinted ? identity(hinted).trim().toLowerCase() : "";
        if (
          !hinted ||
          hinted.owner_id !== d.owner ||
          hinted.relay ||
          hinted.shared_account ||
          (address && address !== job.account) ||
          reservedElsewhere(job, slot)
        ) {
          if (job.phase !== "queued")
            throw new Error("Sign-in slot ownership changed");
          slot = undefined;
        }
      }
      if (!slot && existing) slot = existing.slot;
      if (!slot) {
        slot = claimSlot(d.db, d.owner, { perUser: d.perUser });
        try {
          await progress("browser");
        } catch (e) {
          releaseSlot(d.db, slot);
          throw e;
        }
        await slotDown(d.ctl, slot);
        await wipeSlot(d.ctl, slot, d);
        if (!job.capabilities.includes("chat") && d.ctl.signin) {
          setSlotStopped(d.db, slot, true);
          setLight(d.db, slot, true);
          await d.ctl.signin(slot);
        } else {
          await d.ctl.start(slot);
          startLight(slot, d.db);
        }
      }
      const row = slotRow(d.db, slot);
      if (!row || row.owner_id !== d.owner || row.relay || row.shared_account)
        throw new Error("Sign-in slot ownership changed");
      const actual = identity(row).trim().toLowerCase();
      if (actual && actual !== job.account)
        throw new Error(
          "A different Microsoft account was signed in. Use the requested address.",
        );
      await progress("browser");
      if (!job.capabilities.some((c) => c !== "chat")) {
        if (actual !== job.account) continue;
        if (row.stopped) {
          await (row.light ? d.ctl.light(slot) : d.ctl.start(slot));
          setSlotStopped(d.db, slot, false);
          continue;
        }
        if (accountSummary(row).teams !== "ok") continue;
        await progress("complete");
        continue;
      }
      const result = job.route
        ? { ok: true }
        : await d.dir.connect({ id: job.id, account: job.account, slot });
      if (result.interaction) {
        if (job.phase === "queued") {
          if (row.stopped && d.ctl.signin) await d.ctl.signin(slot);
          else if (row.light) askRefresh(d.db, slot);
          else await d.ctl.start(slot);
        }
        continue;
      }
      if (!result.ok)
        throw new Error(
          result.errore || "Microsoft refused this sign-in route",
        );
      saveMailboxIdentity(d.dataDir, slot, job.account);
      if (!job.capabilities.includes("chat")) {
        await slotDown(d.ctl, slot);
        setSlotStopped(d.db, slot, true);
      } else if (row.stopped) {
        await (row.light ? d.ctl.light(slot) : d.ctl.start(slot));
        setSlotStopped(d.db, slot, false);
        continue;
      } else if (actual !== job.account || accountSummary(row).teams !== "ok")
        continue;
      await progress("complete");
    } catch (e) {
      await progress(
        "blocked",
        e instanceof Error ? e.message : "Sign-in failed",
      ).catch(() => undefined);
    }
  }
}
