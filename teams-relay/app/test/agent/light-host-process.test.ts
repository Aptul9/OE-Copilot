// The light bundle in host mode as a process: two slots on one fork channel, started and stopped over IPC, then
// SIGTERM. No sign-in is stored, so the slots run their loops without reaching Microsoft.
import { fork, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { build } from "esbuild";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { bundleOptions } from "../../scripts/bundles.mjs";
import type { FromHost, ToHost } from "@/agent/light/host-protocol";
import { tempDir } from "../helpers";

const APP = path.resolve(__dirname, "../..");
let dir = "";
let bundle = "";
let host: ChildProcess | null = null;
const lines: string[] = [];
const messages: FromHost[] = [];

const slotEnv = (slot: number) => ({
  ...(process.env as Record<string, string>),
  NODE_PATH: path.join(APP, "node_modules"),
  ACCOUNT: String(slot),
  DB_PATH: path.join(dir, String(slot), "messages.db"),
  APP_DB: path.join(dir, "app.db"),
  VAPID_PRIVATE: path.join(dir, "missing.pem"),
  VAPID_APPKEY: path.join(dir, "missing.txt"),
  LIGHT_POLL: "15",
});
const send = (m: ToHost) => host?.send(m);
const seen = (m: FromHost) => messages.some((x) => x.slot === m.slot && x.event === m.event);
const logged = (pattern: RegExp) => lines.filter((l) => pattern.test(l)).length;

async function until<T>(check: () => T | null | undefined | false, timeout: number, what: string): Promise<T> {
  const end = Date.now() + timeout;
  for (;;) {
    const v = check();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}; host log:\n${lines.slice(-15).join("\n")}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

beforeAll(async () => {
  dir = tempDir("teamsrelay-light-host-");
  bundle = path.join(dir, "light.cjs");
  await build(bundleOptions("light", bundle));
  host = fork(bundle, ["--host"], { env: { ...process.env, NODE_PATH: path.join(APP, "node_modules") }, stdio: ["ignore", "pipe", "pipe", "ipc"] });
  for (const stream of [host.stdout, host.stderr]) stream?.on("data", (d) => lines.push(...String(d).split("\n").filter(Boolean)));
  host.on("message", (m) => messages.push(m as FromHost));
  host.on("error", (e) => lines.push(`[spawn error] ${e.message}`));
  host.on("exit", (code, signal) => lines.push(`[exit] code=${code} signal=${signal}`));
});

afterAll(() => {
  host?.kill("SIGKILL");
});

describe("light host process", () => {
  it("runs two slots in one process, each with its own log prefix and database", async () => {
    await until(() => logged(/^light: host start pid=\d+$/) === 1, 20_000, "the host start line");
    send({ op: "start", slot: 2, env: slotEnv(2) });
    send({ op: "start", slot: 3, env: slotEnv(3) });
    await until(() => seen({ slot: 2, event: "started" }) && seen({ slot: 3, event: "started" }), 10_000, "both slots started");
    await until(() => logged(/^\[2\] light: start slot=2 /) === 1 && logged(/^\[3\] light: start slot=3 /) === 1, 20_000, "the start line of both slots");
    for (const slot of [2, 3]) await until(() => fs.existsSync(path.join(dir, String(slot), "messages.db")), 10_000, `the database of slot ${slot}`);
  }, 60_000);

  it("stops one slot and leaves the other running", async () => {
    send({ op: "stop", slot: 2 });
    await until(() => seen({ slot: 2, event: "stopped" }), 10_000, "slot 2 stopped");
    expect(seen({ slot: 3, event: "stopped" })).toBe(false);
    expect(messages.some((m) => m.slot === 3 && m.event === "exited")).toBe(false);
    // slot 3 still writes its own lines: a stopped slot 2 must not have taken the host down
    const before = lines.filter((l) => l.startsWith("[3] ")).length;
    send({ op: "start", slot: 3, env: { ...slotEnv(3), LIGHT_POLL: "30" } });
    await until(() => lines.filter((l) => l.startsWith("[3] ")).length > before, 20_000, "a new line of slot 3");
    expect(logged(/^\[3\] light: start slot=3 poll=30/)).toBe(1);
  }, 60_000);

  it("exits within 5 s on SIGTERM", async () => {
    const done = new Promise<number | null>((resolve) => host?.once("exit", (code) => resolve(code)));
    const t0 = Date.now();
    host?.kill("SIGTERM");
    const code = await done;
    expect(Date.now() - t0).toBeLessThan(5000);
    // on Windows kill() terminates the process without a signal handler to run
    if (process.platform !== "win32") expect(code).toBe(0);
  }, 30_000);
});
