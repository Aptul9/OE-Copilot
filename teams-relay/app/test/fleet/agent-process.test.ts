// The fleet agent bundle as a real process: build dist/fleet-agent.cjs, run it with a config that enables cmdapi, and
// drive that cmdapi over HTTP. Proves the agent bundle builds (ssh2 external) and boots its components. The reverse
// tunnel targets an unreachable VM on purpose; the agent keeps running and serving cmdapi while it redials.
import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { build } from "esbuild";
import { bundleOptions } from "../../scripts/bundles.mjs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { tempDir } from "../helpers";

const APP = path.resolve(__dirname, "../..");
const TOKEN = "a".repeat(32);
let proc: ChildProcess | null = null;
let url = "";

async function freePort(): Promise<number> {
  const s = net.createServer();
  await new Promise<void>((r) => s.listen(0, "127.0.0.1", r));
  const p = (s.address() as net.AddressInfo).port;
  await new Promise((r) => s.close(r));
  return p;
}

beforeAll(async () => {
  const dir = tempDir();
  const bundle = path.join(dir, "fleet-agent.cjs");
  await build(bundleOptions("fleet-agent", bundle));

  const port = await freePort();
  const configFile = path.join(dir, "fleet.config.json");
  fs.writeFileSync(configFile, JSON.stringify({ vm: "localhost", cmdapi: { enabled: true, vmPort: await freePort(), localPort: port, token: TOKEN } }));

  // run from APP so `require("ssh2")` (external) resolves; NODE_PATH makes it resolve from the temp bundle too
  proc = spawn(process.execPath, [bundle], {
    cwd: APP,
    env: { ...process.env, FLEET_CONFIG: configFile, NODE_PATH: path.join(APP, "node_modules") },
    stdio: ["ignore", "pipe", "pipe"],
  });
  url = await new Promise<string>((resolve, reject) => {
    let buf = "";
    const timer = setTimeout(() => reject(new Error(`agent did not start cmdapi; saw: ${buf}`)), 15000);
    const onData = (c: Buffer) => {
      buf += c;
      const m = buf.match(/listening on (http:\/\/\S+)/);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    };
    proc!.stdout!.on("data", onData);
    proc!.stderr!.on("data", onData);
    proc!.once("error", reject);
  });
}, 30000);

afterAll(() => {
  proc?.kill();
});

describe("fleet agent process", () => {
  it("serves cmdapi from the enabled component", async () => {
    expect((await (await fetch(`${url}/health`)).json()).ok).toBe(true);
    const res = await fetch(`${url}/command`, { method: "POST", headers: { authorization: `Bearer ${TOKEN}` }, body: "echo agenttest" });
    expect(res.status).toBe(200);
    expect((await res.json()).stdout).toContain("agenttest");
  });

  it("boots from a fleet.env file without fleet.config.json", async () => {
    const dir = tempDir();
    const bundle = path.join(dir, "fleet-agent.cjs");
    await build(bundleOptions("fleet-agent", bundle));
    const port = await freePort();
    const vmPort = await freePort();
    fs.writeFileSync(
      path.join(dir, "fleet.env"),
      `FLEET_VM=localhost\nFLEET_CMDAPI=1\nFLEET_CMDAPI_VM_PORT=${vmPort}\nFLEET_CMDAPI_LOCAL_PORT=${port}\nFLEET_CMDAPI_TOKEN=${TOKEN}\n`,
    );
    const envOnly: Record<string, string | undefined> = { ...process.env, NODE_PATH: path.join(APP, "node_modules") };
    delete envOnly.FLEET_CONFIG;
    delete envOnly.FLEET_VM;
    const child = spawn(process.execPath, [bundle], { cwd: dir, env: envOnly as NodeJS.ProcessEnv, stdio: ["ignore", "pipe", "pipe"] });
    try {
      const envUrl = await new Promise<string>((resolve, reject) => {
        let buf = "";
        const timer = setTimeout(() => reject(new Error(`env agent did not start; saw: ${buf}`)), 15000);
        const onData = (c: Buffer) => {
          buf += c;
          const m = buf.match(/listening on (http:\/\/\S+)/);
          if (m) {
            clearTimeout(timer);
            resolve(m[1]);
          }
        };
        child.stdout!.on("data", onData);
        child.stderr!.on("data", onData);
        child.once("error", reject);
      });
      expect((await (await fetch(`${envUrl}/health`)).json()).ok).toBe(true);
    } finally {
      child.kill();
    }
  });
});
