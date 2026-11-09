import http from "node:http";
import type { AddressInfo } from "node:net";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { traceSlowRequests } from "@/agent/light/slow-requests";
import { inSlot } from "@/agent/slot-scope";

let server: http.Server;
let url = "";
let stop = () => {};

beforeAll(async () => {
  server = http.createServer((req, res) => {
    const wait = Number(new URL(req.url ?? "/", "http://x").searchParams.get("wait") ?? 0);
    setTimeout(() => res.end("ok"), wait);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/chats`;
  stop = traceSlowRequests(150);
});

afterAll(async () => {
  stop();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
});

describe("slow requests", () => {
  it("logs nothing for a request under the threshold", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => undefined);
    await (await fetch(`${url}?wait=0`)).text();
    expect(err).not.toHaveBeenCalled();
    err.mockRestore();
  });

  it("logs a request that timed out, in its slot, with when its headers went out and that no answer came", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => undefined);
    await inSlot(4, () => fetch(`${url}?wait=1000`, { signal: AbortSignal.timeout(300) }).catch(() => undefined));
    await vi.waitFor(() => expect(err).toHaveBeenCalled());
    const line = String(err.mock.calls[0]?.[0]);
    expect(line).toMatch(/^\[4\] fetch: slow GET http:\/\/127\.0\.0\.1:\d+\/chats after \d+ ms sent=\d+ms answer=none error=/);
    err.mockRestore();
  });

  it("logs a request that answered after the threshold", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => undefined);
    await (await fetch(`${url}?wait=300`)).text();
    await vi.waitFor(() => expect(err).toHaveBeenCalled());
    expect(String(err.mock.calls[0]?.[0])).toMatch(/^fetch: slow GET .*\/chats after \d+ ms sent=\d+ms answer=\d+ms$/);
    err.mockRestore();
  });
});
