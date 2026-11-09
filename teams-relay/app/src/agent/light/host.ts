// The light host: every light slot of the container in this one process, driven by the supervisor over the IPC channel
// of fork (src/supervisor/light-host.ts). Each slot runs in its own slot scope (src/agent/slot-scope.ts).
import { setGlobalDispatcher } from "undici";
import { restartDelay, TIMING, type Timing } from "@/supervisor/process";
import { errorText, log } from "../log";
import { dropSlot, inSlot } from "../slot-scope";
import { slotConnections } from "./connections";
import type { FromHost, ToHost } from "./host-protocol";
import { startSlot } from "./slot";

export type SlotRunner = (env: Record<string, string>, signal: AbortSignal) => Promise<void>;

type Entry = {
  key: string;
  env: Record<string, string>;
  ctrl: AbortController;
  // the run in progress, never rejects
  current: Promise<void>;
  timer: NodeJS.Timeout | null;
  failures: number;
};

const SHUTDOWN_GRACE_MS = 30_000;

// Resolves when `p` does, or after `ms`
async function within(p: Promise<unknown>, ms: number) {
  let timer: NodeJS.Timeout | undefined;
  await Promise.race([p, new Promise((resolve) => (timer = setTimeout(resolve, ms)))]);
  clearTimeout(timer);
}

const keyOf = (env: Record<string, string>) => JSON.stringify(Object.entries(env).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));

export class SlotHost {
  private readonly slots = new Map<number, Entry>();
  // messages of one slot are handled in order: a stop must not overtake the start before it
  private readonly queue = new Map<number, Promise<void>>();

  constructor(
    private readonly run: SlotRunner,
    private readonly send: (m: FromHost) => void,
    private readonly timing: Timing = TIMING,
  ) {}

  handle(m: ToHost): Promise<void> {
    const next = (this.queue.get(m.slot) ?? Promise.resolve()).then(() => this.apply(m));
    this.queue.set(m.slot, next);
    return next;
  }

  running(): number[] {
    return [...this.slots.keys()];
  }

  async shutdown(graceMs = SHUTDOWN_GRACE_MS): Promise<void> {
    const all = [...this.slots.values()];
    this.slots.clear();
    for (const e of all) this.abort(e);
    await within(Promise.all(all.map((e) => e.current)), graceMs);
  }

  private async apply(m: ToHost) {
    if (m.op === "stop") {
      await this.end(m.slot);
      this.send({ slot: m.slot, event: "stopped" });
      return;
    }
    const key = keyOf(m.env);
    const known = this.slots.get(m.slot);
    if (known?.key === key) return;
    await this.end(m.slot);
    const e: Entry = { key, env: m.env, ctrl: new AbortController(), current: Promise.resolve(), timer: null, failures: 0 };
    this.slots.set(m.slot, e);
    this.begin(m.slot, e);
  }

  private abort(e: Entry) {
    e.ctrl.abort();
    if (e.timer) clearTimeout(e.timer);
    e.timer = null;
  }

  private async end(slot: number) {
    const e = this.slots.get(slot);
    if (!e) return;
    this.slots.delete(slot);
    this.abort(e);
    await within(e.current, this.timing.stopGraceMs);
    dropSlot(slot);
  }

  // A run that ends while its slot is wanted is reported and started again, like Supervised does with a process
  private begin(slot: number, e: Entry) {
    e.timer = null;
    const startedAt = Date.now();
    e.current = inSlot(slot, async () => this.run(e.env, e.ctrl.signal))
      .then(() => "returned", errorText)
      .then((how) => {
        if (e.ctrl.signal.aborted) return;
        const next = restartDelay(Date.now() - startedAt, e.failures, this.timing);
        e.failures = next.failures;
        this.send({ slot, event: "exited", how });
        inSlot(slot, () => log.warn("light", `slot exited (${how}), starting again in ${next.delayMs / 1000} s`));
        e.timer = setTimeout(() => this.begin(slot, e), next.delayMs);
      });
    this.send({ slot, event: "started" });
  }
}

export function runHost(): void {
  setGlobalDispatcher(slotConnections());
  const host = new SlotHost(startSlot, (m) => {
    if (process.connected) process.send?.(m);
  });
  log.info("light", "host start", { pid: process.pid });
  process.on("message", (m) => void host.handle(m as ToHost));
  let closing = false;
  const close = () => {
    if (closing) return;
    closing = true;
    void host.shutdown().then(() => process.exit(0));
  };
  for (const event of ["disconnect", "SIGTERM", "SIGINT"] as const) process.on(event, close);
  const crashed = (e: unknown) => {
    log.warn("light", `host crashed: ${errorText(e)}`);
    process.exit(1);
  };
  process.on("uncaughtException", crashed);
  process.on("unhandledRejection", crashed);
}
