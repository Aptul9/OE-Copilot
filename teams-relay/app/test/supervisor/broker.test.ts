import http from "node:http";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { brokerSlots } from "@/lib/control";
import { ConfigError } from "@/shared/env";
import { Accounts, agentCommand, lightCommand, type AccountsConfig } from "@/supervisor/accounts";
import { brokerEnv, parseBrokerSlots } from "@/supervisor/broker";
import { loadConfig } from "@/supervisor/config";
import { controlServer, listen } from "@/supervisor/server";
import { tempDir } from "../helpers";
import { fakeProcess, socketPath } from "./fakes";

const SLOTS = JSON.stringify({ "2": { account: "Mario@Example.com", region: "amer" }, "5": { account: "anna@example.com", region: "emea" } });
const BROKER = { SHARED_BROKER_URL: "http://agente:8765", SHARED_BROKER_KEY: "secret-key", SHARED_BROKER_SLOTS: SLOTS };

function cfg(env: Record<string, string> = BROKER): AccountsConfig {
  return { ...loadConfig({ ...env, NATIVE_CALLS: "1" }).accounts, profilesDir: path.join(tempDir(), "profiles"), node: "node" };
}

describe("shared broker mapping of the supervisor", () => {
  it("maps each slot to its own account and region, none to the others", () => {
    const c = cfg();
    expect(c.sharedBroker).toEqual({
      url: "http://agente:8765",
      key: "secret-key",
      slots: { "2": { account: "mario@example.com", region: "amer" }, "5": { account: "anna@example.com", region: "emea" } },
    });
    expect(lightCommand(2, c).env).toMatchObject({ SHARED_BROKER_URL: "http://agente:8765", SHARED_BROKER_KEY: "secret-key", SHARED_BROKER_ACCOUNT: "mario@example.com", SHARED_BROKER_REGION: "amer" });
    expect(lightCommand(5, c).env).toMatchObject({ SHARED_BROKER_ACCOUNT: "anna@example.com", SHARED_BROKER_REGION: "emea" });
    for (const env of [lightCommand(1, c).env, agentCommand(2, c).env]) {
      expect(Object.keys(env ?? {}).filter((k) => k.startsWith("SHARED_BROKER"))).toEqual([]);
    }
    // the settings passed to every agent never carry the broker
    expect(Object.keys(c.agentEnv).filter((k) => k.startsWith("SHARED_BROKER"))).toEqual([]);
  });

  it("without a mapping nothing changes", () => {
    expect(cfg({}).sharedBroker).toBeUndefined();
    expect(brokerEnv(2, undefined)).toEqual({});
  });

  it.each([
    ["only the URL", { SHARED_BROKER_URL: "http://agente:8765" }],
    ["no key", { SHARED_BROKER_URL: "http://agente:8765", SHARED_BROKER_SLOTS: SLOTS }],
    ["not JSON", { ...BROKER, SHARED_BROKER_SLOTS: "slot2=me" }],
    ["no slot", { ...BROKER, SHARED_BROKER_SLOTS: "{}" }],
    ["slot 0", { ...BROKER, SHARED_BROKER_SLOTS: JSON.stringify({ "0": { account: "a@example.com", region: "emea" } }) }],
    ["no region", { ...BROKER, SHARED_BROKER_SLOTS: JSON.stringify({ "1": { account: "a@example.com" } }) }],
    ["a region that is a path", { ...BROKER, SHARED_BROKER_SLOTS: JSON.stringify({ "1": { account: "a@example.com", region: "emea/../x" } }) }],
    ["not an address", { ...BROKER, SHARED_BROKER_SLOTS: JSON.stringify({ "1": { account: "me", region: "emea" } }) }],
    ["an extra field", { ...BROKER, SHARED_BROKER_SLOTS: JSON.stringify({ "1": { account: "a@example.com", region: "emea", key: "x" } }) }],
    ["one account on two slots", { ...BROKER, SHARED_BROKER_SLOTS: JSON.stringify({ "1": { account: "a@example.com", region: "emea" }, "3": { account: "A@example.com", region: "emea" } }) }],
  ])("refuses %s", (_name, env) => {
    expect(() => loadConfig(env)).toThrow(ConfigError);
  });

  it("parses the mapping on its own", () => {
    expect(parseBrokerSlots('{"7":{"account":"x@example.com","region":"apac"}}')).toEqual({ "7": { account: "x@example.com", region: "apac" } });
  });

  it("never opens a browser for a mapped slot, runs its light agent", async () => {
    const calls: string[] = [];
    const accounts = new Accounts(cfg(), { log: () => undefined, process: (name) => fakeProcess(name, calls), focus: async () => true });
    await expect(accounts.start(2)).rejects.toMatchObject({ status: 409 });
    await accounts.light(2);
    await accounts.start(1);
    expect(calls).toEqual(["start light-2", "start browser-1", "start agent-1"]);
    expect(accounts.broker()).toEqual({ "2": { account: "mario@example.com" }, "5": { account: "anna@example.com" } });
  });
});

describe("GET /broker", () => {
  let server: http.Server | null = null;
  afterEach(async () => {
    const s = server;
    server = null;
    if (s) await new Promise<void>((resolve) => s.close(() => resolve()));
  });

  it("gives the web app the mapped slots and accounts, never URL, key or region", async () => {
    const sock = socketPath(tempDir());
    const accounts = new Accounts(cfg(), { log: () => undefined, process: (name) => fakeProcess(name, []), focus: async () => true });
    server = controlServer(accounts, { desktop: Promise.resolve(), log: () => undefined, broker: () => accounts.broker() });
    await listen(server, sock);
    const slots = await brokerSlots(sock);
    expect(slots).toEqual({ "2": { account: "mario@example.com" }, "5": { account: "anna@example.com" } });
    expect(JSON.stringify(slots)).not.toMatch(/secret-key|agente|amer/);
  });

  it("a supervisor without the route maps nothing", async () => {
    const sock = socketPath(tempDir());
    server = http.createServer((_req, res) => void res.writeHead(404).end());
    await new Promise<void>((resolve) => server!.listen(sock, resolve));
    expect(await brokerSlots(sock)).toEqual({});
  });
});
