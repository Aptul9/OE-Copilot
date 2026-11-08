// The shared Microsoft accounts in the supervisor: the slot binding a light start carries, the light agent environment it
// gives, the browser it refuses, and the list of the OE calendar agent it passes to the web app with its own key
import http from "node:http";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { controlClient, directoryClient } from "@/lib/control";
import { ConfigError } from "@/shared/env";
import { Accounts, agentCommand, lightCommand, type AccountsConfig } from "@/supervisor/accounts";
import { bindingOf, brokerEnv, directory } from "@/supervisor/broker";
import { loadConfig } from "@/supervisor/config";
import { controlServer, listen } from "@/supervisor/server";
import { tempDir } from "../helpers";
import { fakeProcess, socketPath } from "./fakes";

const BROKER = { SHARED_BROKER_URL: "http://oe-copilot-agent:8180", SHARED_BROKER_KEY: "secret-key" };
const MARIO = { account: "mario@example.com", region: "amer" };

function cfg(env: Record<string, string> = BROKER): AccountsConfig {
  return { ...loadConfig({ ...env, NATIVE_CALLS: "1" }).accounts, profilesDir: path.join(tempDir(), "profiles"), node: "node" };
}

describe("broker configuration", () => {
  it("takes the agent address and key, both or none", () => {
    expect(cfg().sharedBroker).toEqual({ url: "http://oe-copilot-agent:8180", key: "secret-key" });
    expect(cfg({}).sharedBroker).toBeUndefined();
    expect(() => loadConfig({ SHARED_BROKER_URL: BROKER.SHARED_BROKER_URL })).toThrow(ConfigError);
    expect(() => loadConfig({ SHARED_BROKER_KEY: "k" })).toThrow(ConfigError);
  });

  it("the agents of the browser and the settings of every agent never carry the broker", () => {
    const c = cfg();
    expect(Object.keys(agentCommand(2, c).env ?? {}).filter((k) => k.startsWith("SHARED_BROKER"))).toEqual([]);
    expect(Object.keys(c.agentEnv).filter((k) => k.startsWith("SHARED_BROKER"))).toEqual([]);
  });
});

describe("binding of a light start", () => {
  it("reads account and region, emea by default, none without an account", () => {
    expect(bindingOf(new URLSearchParams("shared=Mario@Example.com&region=amer"))).toEqual(MARIO);
    expect(bindingOf(new URLSearchParams("shared=mario@example.com"))).toEqual({ account: "mario@example.com", region: "emea" });
    expect(bindingOf(new URLSearchParams(""))).toBeNull();
  });

  it.each([
    ["not an address", "shared=me&region=emea"],
    ["an address with a path", "shared=a@example.com/../x&region=emea"],
    ["a region that is a path", "shared=a@example.com&region=emea/../x"],
    ["a region in capitals", "shared=a@example.com&region=EMEA"],
  ])("refuses %s with 400", (_n, q) => {
    expect(() => bindingOf(new URLSearchParams(q))).toThrow(expect.objectContaining({ status: 400 }));
  });

  it("gives the light agent all four variables of its own account, none without a binding or a broker", () => {
    const c = cfg();
    expect(lightCommand(2, c, MARIO).env).toMatchObject({ SHARED_BROKER_URL: "http://oe-copilot-agent:8180", SHARED_BROKER_KEY: "secret-key", SHARED_BROKER_ACCOUNT: "mario@example.com", SHARED_BROKER_REGION: "amer" });
    expect(Object.keys(lightCommand(2, c).env ?? {}).filter((k) => k.startsWith("SHARED_BROKER"))).toEqual([]);
    expect(brokerEnv(MARIO, undefined)).toEqual({});
    expect(brokerEnv(null, c.sharedBroker)).toEqual({});
  });
});

describe("a bound slot in the supervisor", () => {
  const make = (c = cfg()) => {
    const calls: string[] = [];
    const commands: Record<string, string>[] = [];
    const accounts = new Accounts(c, {
      log: () => undefined,
      process: (name, command) => {
        const p = fakeProcess(name, calls);
        const start = p.start.bind(p);
        p.start = () => {
          if (name.startsWith("light-")) commands.push(command().env ?? {});
          start();
        };
        return p;
      },
      focus: async () => true,
    });
    return { accounts, calls, commands };
  };

  it("runs its light agent on its account and refuses its browser", async () => {
    const { accounts, calls, commands } = make();
    await accounts.light(2, MARIO);
    await expect(accounts.start(2)).rejects.toMatchObject({ status: 409 });
    await accounts.start(1);
    expect(calls).toEqual(["start light-2", "start browser-1", "start agent-1"]);
    expect(commands[0]).toMatchObject({ SHARED_BROKER_ACCOUNT: "mario@example.com", SHARED_BROKER_REGION: "amer" });
  });

  it("the same binding again leaves its light agent alone; another one starts it again on the new account", async () => {
    const { accounts, calls, commands } = make();
    await accounts.light(2, MARIO);
    await accounts.light(2, MARIO);
    await accounts.light(2, { account: "anna@example.com", region: "emea" });
    expect(calls).toEqual(["start light-2", "stop light-2", "start light-2"]);
    expect(commands.map((e) => e.SHARED_BROKER_ACCOUNT)).toEqual(["mario@example.com", "anna@example.com"]);
  });

  it("a light start without a binding ends it: its browser may start again", async () => {
    const { accounts, calls } = make();
    await accounts.light(2, MARIO);
    await accounts.light(2);
    await accounts.stop(2);
    await accounts.start(2);
    expect(calls).toContain("start browser-2");
  });

  it("refuses a binding when this container has no broker", async () => {
    const { accounts, calls } = make(cfg({}));
    await expect(accounts.light(2, MARIO)).rejects.toMatchObject({ status: 409 });
    expect(calls).toEqual([]);
  });
});

describe("control API of the shared accounts", () => {
  let server: http.Server | null = null;
  afterEach(async () => {
    const s = server;
    server = null;
    if (s) await new Promise<void>((resolve) => s.close(() => resolve()));
  });

  it("the web app's light start carries the binding of the slot, read from its database", async () => {
    const sock = socketPath(tempDir());
    const seen: unknown[] = [];
    const accounts = { start: vi.fn(), light: vi.fn(async (_n: number, b: unknown) => void seen.push(b)), stop: vi.fn(), wipe: vi.fn(), show: vi.fn(), status: () => [] };
    server = controlServer(accounts as unknown as Accounts, { desktop: Promise.resolve(), log: () => undefined });
    await listen(server, sock);
    const ctl = controlClient(sock, (n) => (n === 2 ? MARIO : null));
    await ctl.light(2);
    await ctl.light(3);
    expect(seen).toEqual([MARIO, null]);
  });

  it("without a broker the list is off for the web app", async () => {
    const sock = socketPath(tempDir());
    server = controlServer(new Accounts(cfg({}), { log: () => undefined, process: (n) => fakeProcess(n, []) }), { desktop: Promise.resolve(), log: () => undefined });
    await listen(server, sock);
    expect(await directoryClient(sock).list()).toBeNull();
  });

  it("passes list, change and report to the agent with the key of this container, only the fields its routes take", async () => {
    const sock = socketPath(tempDir());
    const requests: { url: string; auth: string; body: unknown }[] = [];
    const fetchFn = vi.fn(async (url: URL, init: RequestInit) => {
      requests.push({ url: String(url), auth: String((init.headers as Record<string, string>).Authorization), body: init.body ? JSON.parse(String(init.body)) : undefined });
      expect(init.redirect).toBe("error");
      return Response.json({ ok: true, accounts: [] });
    });
    server = controlServer(new Accounts(cfg({}), { log: () => undefined, process: (n) => fakeProcess(n, []) }), {
      desktop: Promise.resolve(),
      log: () => undefined,
      directory: directory({ url: BROKER.SHARED_BROKER_URL, key: "secret-key" }, fetchFn as unknown as typeof fetch),
    });
    await listen(server, sock);
    const dir = directoryClient(sock);
    expect(await dir.list()).toEqual({ ok: true, accounts: [] });
    await dir.set({ account: "mario@example.com", capabilities: ["chat"], ...({ tenantId: "evil" } as object) } as { account: string; capabilities: string[] });
    await dir.set({ account: "mario@example.com", enabled: false });
    await dir.report({ accounts: [{ account: "mario@example.com", slot: 2, state: "ok" }], others: [], ...({ key: "x" } as object) } as { accounts: unknown[]; others: unknown[] });
    expect(requests).toEqual([
      { url: "http://oe-copilot-agent:8180/api/auth/shared", auth: "Bearer secret-key", body: undefined },
      { url: "http://oe-copilot-agent:8180/api/auth/shared", auth: "Bearer secret-key", body: { account: "mario@example.com", capabilities: ["chat"] } },
      { url: "http://oe-copilot-agent:8180/api/auth/shared", auth: "Bearer secret-key", body: { account: "mario@example.com", enabled: false } },
      { url: "http://oe-copilot-agent:8180/api/auth/shared/teams", auth: "Bearer secret-key", body: { accounts: [{ account: "mario@example.com", slot: 2, state: "ok" }], others: [] } },
    ]);
  });

  it("an agent that refuses the key or does not answer is a 502 with the reason, never the key", async () => {
    const sock = socketPath(tempDir());
    let answer: () => Promise<Response> = async () => Response.json({ errore: "non autorizzato" }, { status: 401 });
    server = controlServer(new Accounts(cfg({}), { log: () => undefined, process: (n) => fakeProcess(n, []) }), {
      desktop: Promise.resolve(),
      log: () => undefined,
      directory: directory({ url: BROKER.SHARED_BROKER_URL, key: "secret-key" }, (() => answer()) as unknown as typeof fetch),
    });
    await listen(server, sock);
    await expect(directoryClient(sock).list()).rejects.toThrow(/TEAMS_BROKER_KEY/);
    answer = async () => {
      throw new TypeError("fetch failed");
    };
    const e = await directoryClient(sock).list().catch((x: Error) => x);
    expect(String(e)).toMatch(/not reachable/);
    expect(String(e)).not.toMatch(/secret-key/);
  });
});
