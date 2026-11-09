import { fork } from "node:child_process";
import readline from "node:readline";
import { HOST_FLAG, type FromHost, type ToHost } from "../agent/light/host-protocol";
import { restartDelay, signal, TIMING, type Command, type Timing } from "./process";
import type { ProcessLike } from "./accounts";

// the channel to one host child; the real one forks, tests give a fake
export interface HostChild {
  readonly pid: number | null;
  send(m: ToHost): void;
  kill(sig: NodeJS.Signals): void;
  on(event: "message", f: (m: FromHost) => void): void;
  on(event: "exit", f: (how: string) => void): void;
}

export type SpawnHost = () => HostChild;

export interface LightHostLike {
  slot(n: number, command: () => Command): ProcessLike;
}

// One light slot in the host: what Supervised is to a process of its own
class HostedLight implements ProcessLike {
  restarts = 0;
  lastExit: string | null = null;
  up = false;
  wanted = false;
  private seen = false;
  private waiting: (() => void)[] = [];

  constructor(
    readonly name: string,
    readonly n: number,
    readonly command: () => Command,
    private readonly host: LightHost,
  ) {}

  get running() {
    return this.up;
  }

  get active() {
    return this.wanted;
  }

  get pid() {
    return this.up ? this.host.pid : null;
  }

  start() {
    this.wanted = true;
    this.host.want(this);
  }

  async stop(): Promise<void> {
    this.wanted = false;
    await this.host.release(this);
  }

  started() {
    this.up = true;
    if (this.seen) this.restarts++;
    this.seen = true;
  }

  exited(how: string) {
    this.up = false;
    this.lastExit = how;
  }

  stopped() {
    this.up = false;
    this.flush();
  }

  // the host is gone: the slot with it
  lost(how: string) {
    this.exited(how);
    this.flush();
  }

  answer() {
    return new Promise<void>((resolve) => this.waiting.push(resolve));
  }

  private flush() {
    for (const resolve of this.waiting.splice(0)) resolve();
  }
}

// All the light slots in one process (node light.cjs --host, src/agent/light/host.ts), kept running while a slot is
// wanted: started with the first, started again after an exit, stopped with the last.
export class LightHost implements LightHostLike {
  private child: HostChild | null = null;
  private readonly slots = new Map<number, HostedLight>();
  private timer: NodeJS.Timeout | null = null;
  private failures = 0;
  private startedAt = 0;

  constructor(
    private readonly spawn: SpawnHost,
    private readonly log: (line: string) => void,
    private readonly timing: Timing = TIMING,
  ) {}

  get pid() {
    return this.child?.pid ?? null;
  }

  slot(n: number, command: () => Command): ProcessLike {
    let s = this.slots.get(n);
    if (!s) this.slots.set(n, (s = new HostedLight(`light-${n}`, n, command, this)));
    return s;
  }

  want(s: HostedLight) {
    if (this.child) this.startSlot(this.child, s);
    else if (!this.timer) this.open();
  }

  // Answered when the host says the slot stopped, when the host exits, or after the grace. The last slot to go takes
  // the host with it.
  async release(s: HostedLight) {
    const child = this.child;
    if (child) {
      const answered = s.answer();
      child.send({ op: "stop", slot: s.n });
      let grace: NodeJS.Timeout | undefined;
      await Promise.race([answered, new Promise<void>((resolve) => (grace = setTimeout(resolve, this.timing.stopGraceMs)))]);
      clearTimeout(grace);
      s.stopped();
    }
    if (this.wanted()) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    await this.close();
  }

  private wanted() {
    return [...this.slots.values()].some((s) => s.wanted);
  }

  private startSlot(child: HostChild, s: HostedLight) {
    child.send({ op: "start", slot: s.n, env: s.command().env ?? {} });
  }

  private open() {
    this.timer = null;
    const child = this.spawn();
    this.child = child;
    this.startedAt = Date.now();
    child.on("message", (m) => {
      if (this.child !== child) return;
      const s = this.slots.get(m.slot);
      if (!s) return;
      if (m.event === "started") s.started();
      else if (m.event === "exited") s.exited(m.how);
      else s.stopped();
    });
    child.on("exit", (how) => this.exited(child, how));
    for (const s of this.slots.values()) if (s.wanted) this.startSlot(child, s);
  }

  private exited(child: HostChild, how: string) {
    if (this.child !== child) return;
    this.child = null;
    for (const s of this.slots.values()) s.lost(how);
    if (!this.wanted()) return;
    const next = restartDelay(Date.now() - this.startedAt, this.failures, this.timing);
    this.failures = next.failures;
    this.log(`light host exited (${how}), starting again in ${next.delayMs / 1000} s`);
    this.timer = setTimeout(() => this.open(), next.delayMs);
  }

  private async close() {
    const child = this.child;
    if (!child) return;
    const gone = new Promise<void>((resolve) => child.on("exit", () => resolve()));
    child.kill("SIGTERM");
    const kill = setTimeout(() => child.kill("SIGKILL"), this.timing.stopGraceMs);
    await gone;
    clearTimeout(kill);
  }
}

// The host as a child of this process with an IPC channel, in its own process group, so that a stop reaches the
// calling hosts it forked too
export function forkHost(file: string, node: string, env: Record<string, string>, output: (line: string) => void): HostChild {
  const child = fork(file, [HOST_FLAG], { execPath: node, env: env as NodeJS.ProcessEnv, stdio: ["ignore", "pipe", "pipe", "ipc"], detached: true, windowsHide: true });
  for (const stream of [child.stdout, child.stderr]) {
    if (stream) readline.createInterface({ input: stream }).on("line", output);
  }
  const messages: ((m: FromHost) => void)[] = [];
  const exits: ((how: string) => void)[] = [];
  let done = false;
  // "exit" and "error" can both come: only the first counts
  const exited = (how: string) => {
    if (done) return;
    done = true;
    for (const f of exits) f(how);
  };
  child.on("message", (m) => {
    for (const f of messages) f(m as FromHost);
  });
  // an error of a child that started (a message that could not be sent) is not an exit
  child.on("error", (e) => {
    if (child.pid === undefined) exited(e.message);
  });
  child.on("exit", (code, sig) => exited(sig ? `signal ${sig}` : `code ${code}`));
  return {
    get pid() {
      return child.pid ?? null;
    },
    send: (m) => void child.send(m),
    kill: (sig) => signal(child, sig),
    on(event: "message" | "exit", f: (m: never) => void) {
      if (event === "message") messages.push(f as (m: FromHost) => void);
      else exits.push(f as (how: string) => void);
    },
  };
}
