import { execFile } from "node:child_process";
import fs from "node:fs";
import { errorText } from "../agent/log";

// The remote desktop of the image (linuxserver Selkies) as s6-rc services: svc-de pulls in xorg, nginx and selkies; dbus and
// xsettingsd hang on xorg without being dependencies of it
const START = ["svc-de", "svc-xsettingsd", "svc-dbus"];
const ALL = [...START, "svc-selkies", "svc-nginx", "svc-xorg", "svc-pulseaudio"];

export type Run = (file: string, args: string[]) => Promise<void>;

export const runCommand: Run = (file, args) =>
  new Promise((resolve, reject) => {
    execFile(file, args, { timeout: 60_000 }, (e) => (e ? reject(e) : resolve()));
  });

export type DesktopOptions = {
  // the Wayland socket of the session: the desktop is up when it exists
  socket: string;
  // /command/s6-rc; null where there is no s6 (tests, a supervisor started by hand): the desktop is taken as running
  s6rc: string | null;
  // script that makes the virtual microphone, run once the desktop is up; null: none
  microphone: string | null;
  // stop after this long without a browser; Infinity: never
  idleMs: number;
  log: (line: string) => void;
  run?: Run;
  exists?: (file: string) => boolean;
  now?: () => number;
  pollMs?: number;
  waitMs?: number;
};

// The desktop of the accounts, up while a browser needs it. Start and stop go one after the other, so a start asked while
// the desktop goes down waits for it.
export class Desktop {
  private up = false;
  private lastUsed: number;
  private chain: Promise<unknown> = Promise.resolve();
  private readonly exec: Run;
  private readonly exists: (file: string) => boolean;
  private readonly now: () => number;

  constructor(private readonly o: DesktopOptions) {
    this.exec = o.run ?? runCommand;
    this.exists = o.exists ?? fs.existsSync;
    this.now = o.now ?? Date.now;
    this.lastUsed = this.now();
  }

  get running() {
    return this.up;
  }

  // Up when this resolves, whoever else asked at the same time
  ensure(): Promise<void> {
    this.lastUsed = this.now();
    return this.queue(() => this.start());
  }

  // Once a minute, with whether a browser runs: the desktop stops after idleMs without one
  async tick(browserRunning: boolean): Promise<void> {
    if (browserRunning) {
      this.lastUsed = this.now();
      return;
    }
    if (!this.up || !this.o.s6rc || this.now() - this.lastUsed < this.o.idleMs) return;
    await this.queue(() => this.stop());
  }

  private queue(fn: () => Promise<void>): Promise<void> {
    const next = this.chain.then(fn);
    this.chain = next.catch(() => undefined);
    return next;
  }

  private async start() {
    if (this.up) return;
    // s6-rc takes what is already up as it is, so this is also right when the desktop never went down
    if (this.o.s6rc) {
      this.o.log("desktop starting");
      // -b: wait for the lock of the database instead of failing; the boot transition of s6 holds it while this already runs
      await this.exec(this.o.s6rc, ["-b", "-u", "change", ...START]);
    }
    await this.waitForSocket();
    if (this.o.microphone) await this.exec(this.o.microphone, []).catch((e: unknown) => this.o.log(`desktop microphone: ${errorText(e)}`));
    this.up = true;
    this.o.log("desktop up");
  }

  private async stop() {
    // a browser, or a start that came while this waited its turn, keeps the desktop
    if (!this.up || !this.o.s6rc || this.now() - this.lastUsed < this.o.idleMs) return;
    this.o.log(`desktop stopping: no browser for ${Math.round((this.now() - this.lastUsed) / 60_000)} min`);
    await this.exec(this.o.s6rc, ["-b", "-d", "change", ...ALL]);
    this.up = false;
  }

  private async waitForSocket() {
    const wait = this.o.waitMs ?? 60_000;
    const end = this.now() + wait;
    while (!this.exists(this.o.socket)) {
      if (this.now() > end) throw new Error(`the desktop did not come up within ${wait / 1000} s`);
      await new Promise((resolve) => setTimeout(resolve, this.o.pollMs ?? 250));
    }
  }
}
