import http from "node:http";
import type { AddressInfo } from "node:net";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { Agent, getGlobalDispatcher, setGlobalDispatcher, type Dispatcher } from "undici";
import { slotConnections } from "@/agent/light/connections";
import { inSlot } from "@/agent/slot-scope";

let server: http.Server;
let url = "";
let before: Dispatcher;

beforeAll(async () => {
  server = http.createServer((req, res) => res.end("ok"));
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
  before = getGlobalDispatcher();
});

afterAll(async () => {
  setGlobalDispatcher(before);
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
});

describe("slot connections", () => {
  it("sends the requests of each slot of the light host to a pool of its own, as a process of its own had", async () => {
    const pools: string[][] = [];
    const routed = slotConnections(() => {
      const agent = new Agent();
      const requests: string[] = [];
      pools.push(requests);
      const dispatch = agent.dispatch.bind(agent);
      agent.dispatch = (options, handler) => {
        requests.push(String(options.path));
        return dispatch(options, handler);
      };
      return agent;
    });
    setGlobalDispatcher(routed);
    for (const slot of [2, 3, 2, 3]) expect(await inSlot(slot, async () => (await fetch(`${url}${slot}`)).text())).toBe("ok");
    expect(pools).toEqual([
      ["/2", "/2"],
      ["/3", "/3"],
    ]);
  });
});
