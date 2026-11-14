import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { FromHost, ToHost } from "@/agent/light/host-protocol";
import { forkHost, LightHost, type HostChild } from "@/supervisor/light-host";
import type { Timing } from "@/supervisor/process";
import { tempDir } from "../helpers";

const fast: Timing = { minDelayMs: 40, maxDelayMs: 160, stableMs: 60_000, stopGraceMs: 150 };

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until(cond: () => boolean, ms = 10_000) {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error("condition not met in time");
    await wait(10);
  }
}

class FakeChild implements HostChild {
  readonly sent: ToHost[] = [];
  readonly killed: NodeJS.Signals[] = [];
  // a child that dies on SIGTERM, as the host does
  dies = true;
  private readonly messages: ((m: FromHost) => void)[] = [];
  private readonly exits: ((how: string) => void)[] = [];

  constructor(readonly pid: number) {}

  send(m: ToHost) {
    this.sent.push(m);
  }

  kill(sig: NodeJS.Signals) {
    this.killed.push(sig);
    if (this.dies || sig === "SIGKILL") this.exit(`signal ${sig}`);
  }

  on(event: "message" | "exit", f: (m: never) => void) {
    if (event === "message") this.messages.push(f as (m: FromHost) => void);
    else this.exits.push(f as (how: string) => void);
  }

  emit(m: FromHost) {
    for (const f of this.messages) f(m);
  }

  exit(how: string) {
    for (const f of this.exits) f(how);
  }
}

let children: FakeChild[] = [];
let lines: string[] = [];

function make(timing = fast) {
  children = [];
  lines = [];
  return new LightHost(
    () => {
      const c = new FakeChild(5000 + children.length);
      children.push(c);
      return c;
    },
    (line) => lines.push(line),
    timing,
  );
}

const command = (tag: string) => () => ({ file: "node", args: [], env: { ACCOUNT: tag } });
const starts = (c: FakeChild) => c.sent.filter((m) => m.op === "start");

describe("LightHost", () => {
  it("spawns one host for all the slots and sends a start for each", () => {
    const host = make();
    host.slot(2, command("2")).start();
    host.slot(3, command("3")).start();

    expect(children).toHaveLength(1);
    expect(children[0]!.sent).toEqual([
      { op: "start", slot: 2, env: { ACCOUNT: "2" } },
      { op: "start", slot: 3, env: { ACCOUNT: "3" } },
    ]);
  });

  it("is running after the started event, with the pid of the host", () => {
    const host = make();
    const a = host.slot(2, command("2"));
    expect(a.name).toBe("light-2");
    expect(a).toMatchObject({ running: false, active: false, pid: null, restarts: 0, lastExit: null });

    a.start();
    expect(a).toMatchObject({ running: false, active: true, pid: null });
    children[0]!.emit({ slot: 2, event: "started" });

    expect(a).toMatchObject({ running: true, active: true, pid: 5000 });
  });

  it("sends the start at once when the host is already up", () => {
    const host = make();
    host.slot(2, command("2")).start();
    host.slot(3, command("3")).start();

    expect(children).toHaveLength(1);
    expect(starts(children[0]!).map((m) => m.slot)).toEqual([2, 3]);
  });

  it("marks a slot exited with the reason the host gave, the other slot keeps running", () => {
    const host = make();
    const a = host.slot(2, command("2"));
    const b = host.slot(3, command("3"));
    a.start();
    b.start();
    children[0]!.emit({ slot: 2, event: "started" });
    children[0]!.emit({ slot: 3, event: "started" });

    children[0]!.emit({ slot: 2, event: "exited", how: "Error: boom" });
    expect(a).toMatchObject({ running: false, active: true, lastExit: "Error: boom", pid: null });
    expect(b.running).toBe(true);

    children[0]!.emit({ slot: 2, event: "started" });
    expect(a).toMatchObject({ running: true, restarts: 1 });
  });

  it("counts as restarts only the starts that follow an exit, as Supervised does, not a stop and a start", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    const b = host.slot(3, command("3"));
    b.start();
    a.start();
    children[0]!.emit({ slot: 2, event: "started" });
    const stopping = a.stop();
    children[0]!.emit({ slot: 2, event: "stopped" });
    await stopping;
    a.start();
    children[0]!.emit({ slot: 2, event: "started" });
    expect(a).toMatchObject({ running: true, restarts: 0 });
  });

  it("starts the host again after it exits and sends the start of every wanted slot, with a fresh command", async () => {
    const host = make();
    let n = 0;
    const a = host.slot(2, () => ({ file: "node", args: [], env: { ACCOUNT: "2", RUN: String(++n) } }));
    const b = host.slot(3, command("3"));
    a.start();
    b.start();
    children[0]!.emit({ slot: 2, event: "started" });
    children[0]!.emit({ slot: 3, event: "started" });

    children[0]!.exit("code 1");
    expect(a).toMatchObject({ running: false, active: true, lastExit: "code 1", pid: null });
    expect(b).toMatchObject({ running: false, active: true, lastExit: "code 1" });
    expect(lines).toEqual(["light host exited (code 1), starting again in 0.04 s"]);

    await until(() => children.length === 2);
    expect(children[1]!.sent).toEqual([
      { op: "start", slot: 2, env: { ACCOUNT: "2", RUN: "2" } },
      { op: "start", slot: 3, env: { ACCOUNT: "3" } },
    ]);
    children[1]!.emit({ slot: 2, event: "started" });
    expect(a).toMatchObject({ running: true, restarts: 1, pid: 5001 });
  });

  it("doubles the delay while the host keeps dying", async () => {
    const host = make();
    host.slot(2, command("2")).start();
    children[0]!.exit("code 1");
    await until(() => children.length === 2);
    children[1]!.exit("code 1");

    expect(lines).toEqual(["light host exited (code 1), starting again in 0.04 s", "light host exited (code 1), starting again in 0.08 s"]);
    await until(() => children.length === 3);
  });

  it("resolves a stop on the stopped event and leaves the host and the other slots up", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    const b = host.slot(3, command("3"));
    a.start();
    b.start();
    children[0]!.emit({ slot: 2, event: "started" });
    children[0]!.emit({ slot: 3, event: "started" });

    const stopped = a.stop();
    expect(children[0]!.sent.at(-1)).toEqual({ op: "stop", slot: 2 });
    expect(a.active).toBe(false);
    children[0]!.emit({ slot: 2, event: "stopped" });
    await stopped;

    expect(a.running).toBe(false);
    expect(b.running).toBe(true);
    expect(children[0]!.killed).toEqual([]);
  });

  it("resolves a stop after the grace when the host never answers", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    const b = host.slot(3, command("3"));
    a.start();
    b.start();
    children[0]!.emit({ slot: 2, event: "started" });

    const t0 = Date.now();
    await a.stop();

    expect(Date.now() - t0).toBeGreaterThanOrEqual(140);
    expect(a.running).toBe(false);
    expect(children[0]!.killed).toEqual([]);
  });

  it("resolves a stop when the host exits", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    a.start();
    children[0]!.emit({ slot: 2, event: "started" });

    const stopped = a.stop();
    children[0]!.exit("code 1");
    await stopped;

    expect(a.running).toBe(false);
    await wait(120);
    expect(children).toHaveLength(1);
  });

  it("stops the host with the last slot, and the stop returns once the host is gone", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    const b = host.slot(3, command("3"));
    a.start();
    b.start();

    const first = a.stop();
    children[0]!.emit({ slot: 2, event: "stopped" });
    await first;
    expect(children[0]!.killed).toEqual([]);

    const last = b.stop();
    children[0]!.emit({ slot: 3, event: "stopped" });
    await last;

    expect(children[0]!.killed).toEqual(["SIGTERM"]);
    await wait(120);
    expect(children).toHaveLength(1);
    expect(lines).toEqual([]);
  });

  it("kills a host that ignores SIGTERM once the grace is over", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    a.start();
    children[0]!.dies = false;

    const stopped = a.stop();
    children[0]!.emit({ slot: 2, event: "stopped" });
    await stopped;

    expect(children[0]!.killed).toEqual(["SIGTERM", "SIGKILL"]);
  });

  it("starts the host again for a slot wanted after the host stopped", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    a.start();
    const stopped = a.stop();
    children[0]!.emit({ slot: 2, event: "stopped" });
    await stopped;

    a.start();

    expect(children).toHaveLength(2);
    expect(starts(children[1]!)).toHaveLength(1);
  });

  it("does not start the host again when the last slot was stopped while it waited for its delay", async () => {
    const host = make();
    const a = host.slot(2, command("2"));
    a.start();
    children[0]!.exit("code 1");

    await a.stop();
    await wait(150);

    expect(children).toHaveLength(1);
  });

  it("ignores the messages of a slot nobody asked for", () => {
    const host = make();
    host.slot(2, command("2")).start();
    expect(() => children[0]!.emit({ slot: 9, event: "started" })).not.toThrow();
  });
});

describe("forkHost", () => {
  const live: HostChild[] = [];
  afterEach(() => {
    for (const c of live.splice(0)) c.kill("SIGKILL");
  });

  it("forks the file with --host, passes its output lines on, and kills its process group", async () => {
    const out: string[] = [];
    const script = [
      "process.stdout.write(JSON.stringify(process.argv.slice(2)) + '\\n');",
      "process.on('message', (m) => process.send({ slot: m.slot, event: 'started' }));",
      "setInterval(() => {}, 1000);",
    ].join("");
    const file = path.join(tempDir(), "host.cjs");
    fs.writeFileSync(file, script);
    const child = forkHost(file, process.execPath, { PATH: process.env.PATH ?? "" }, (l) => out.push(l));
    live.push(child);
    const got: FromHost[] = [];
    const exits: string[] = [];
    child.on("message", (m) => got.push(m));
    child.on("exit", (how) => exits.push(how));

    child.send({ op: "start", slot: 4, env: {} });
    await until(() => out.length === 1 && got.length === 1);
    expect(out).toEqual(['["--host"]']);
    expect(got).toEqual([{ slot: 4, event: "started" }]);
    expect(child.pid).toBeGreaterThan(0);

    child.kill("SIGTERM");
    await until(() => exits.length === 1);
    expect(exits).toEqual(["signal SIGTERM"]);
  });

  // the Rust host is a binary: spawned, not forked, on the same channel (an executable script stands in for it)
  it.skipIf(process.platform === "win32")("spawns a host binary with --host on the same channel", async () => {
    const out: string[] = [];
    const bin = path.join(tempDir(), "teamsrelay-light");
    const script = [
      `#!${process.execPath}`,
      "process.stdout.write(JSON.stringify([process.argv.slice(2), process.env.NODE_BIN]) + '\\n');",
      "process.on('message', (m) => process.send({ slot: m.slot, event: 'started' }));",
      "setInterval(() => {}, 1000);",
    ].join("\n");
    fs.writeFileSync(bin, script, { mode: 0o755 });
    const child = forkHost("/unused/light.cjs", process.execPath, { PATH: process.env.PATH ?? "", NODE_BIN: process.execPath }, (l) => out.push(l), bin);
    live.push(child);
    const got: FromHost[] = [];
    child.on("message", (m) => got.push(m));
    child.send({ op: "start", slot: 5, env: {} });
    await until(() => out.length === 1 && got.length === 1);
    expect(out).toEqual([JSON.stringify([["--host"], process.execPath])]);
    expect(got).toEqual([{ slot: 5, event: "started" }]);
  });
});
