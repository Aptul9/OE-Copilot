import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FromHost } from "@/agent/light/host-protocol";
import { SlotHost, type SlotRunner } from "@/agent/light/host";
import { currentSlot } from "@/agent/slot-scope";

const TIMING = { minDelayMs: 10, maxDelayMs: 40, stableMs: 1000, stopGraceMs: 200 };

type Call = { slot: number; env: Record<string, string>; signal: AbortSignal };

const untilAbort = ({ signal }: Call) => new Promise<void>((resolve) => signal.addEventListener("abort", () => resolve()));

function setup(behave: (call: Call, n: number) => Promise<void> = untilAbort) {
  const calls: Call[] = [];
  const sent: FromHost[] = [];
  const run: SlotRunner = (env, signal) => {
    const call = { slot: currentSlot(), env, signal };
    calls.push(call);
    return behave(call, calls.filter((c) => c.slot === call.slot).length);
  };
  const host = new SlotHost(run, (m) => sent.push(m), TIMING);
  hosts.push(host);
  return { host, calls, sent, of: (slot: number) => calls.filter((c) => c.slot === slot) };
}

const hosts: SlotHost[] = [];
afterEach(async () => {
  await Promise.all(hosts.splice(0).map((h) => h.shutdown(50)));
  vi.restoreAllMocks();
});

describe("light host", () => {
  it("runs each slot in its own scope with its own environment and reports it started", async () => {
    const { host, calls, sent } = setup();
    await host.handle({ op: "start", slot: 2, env: { ACCOUNT: "2" } });
    await host.handle({ op: "start", slot: 3, env: { ACCOUNT: "3" } });
    expect(calls.map((c) => [c.slot, c.env])).toEqual([[2, { ACCOUNT: "2" }], [3, { ACCOUNT: "3" }]]);
    expect(sent).toEqual([{ slot: 2, event: "started" }, { slot: 3, event: "started" }]);
    expect(host.running()).toEqual([2, 3]);
  });

  it("starts a slot that failed again after the delay and leaves the others alone", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { host, sent, of } = setup(async (call, n) => {
      if (call.slot === 2 && n === 1) throw new Error("boom\nsecond line");
      return untilAbort(call);
    });
    await host.handle({ op: "start", slot: 2, env: { ACCOUNT: "2" } });
    await host.handle({ op: "start", slot: 3, env: { ACCOUNT: "3" } });
    await vi.waitFor(() => expect(of(2)).toHaveLength(2));
    expect(sent).toContainEqual({ slot: 2, event: "exited", how: "boom" });
    expect(sent.filter((m) => m.slot === 2 && m.event === "started")).toHaveLength(2);
    expect(err).toHaveBeenCalledWith("[2] light: slot exited (boom), starting again in 0.01 s");
    expect(of(3)).toHaveLength(1);
    expect(sent.some((m) => m.slot === 3 && m.event !== "started")).toBe(false);
  });

  it("reports a run that returned, and starts it again", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { host, sent, of } = setup(async (call, n) => (n === 1 ? undefined : untilAbort(call)));
    await host.handle({ op: "start", slot: 2, env: {} });
    await vi.waitFor(() => expect(of(2)).toHaveLength(2));
    expect(sent).toContainEqual({ slot: 2, event: "exited", how: "returned" });
  });

  it("doubles the delay of a slot that keeps failing", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { host, of } = setup(async (call, n) => {
      if (n < 3) throw new Error("again");
      return untilAbort(call);
    });
    await host.handle({ op: "start", slot: 2, env: {} });
    await vi.waitFor(() => expect(of(2)).toHaveLength(3));
    expect(err.mock.calls.map((c) => c[0])).toEqual(["[2] light: slot exited (again), starting again in 0.01 s", "[2] light: slot exited (again), starting again in 0.02 s"]);
  });

  it("does nothing for a start with the environment it already runs, and rebinds on another", async () => {
    const { host, of, sent } = setup();
    await host.handle({ op: "start", slot: 2, env: { A: "1", B: "2" } });
    await host.handle({ op: "start", slot: 2, env: { B: "2", A: "1" } });
    expect(of(2)).toHaveLength(1);
    expect(sent).toHaveLength(1);
    await host.handle({ op: "start", slot: 2, env: { A: "1", B: "3" } });
    expect(of(2)).toHaveLength(2);
    expect(of(2)[0]?.signal.aborted).toBe(true);
    expect(of(2)[1]?.signal.aborted).toBe(false);
    expect(of(2)[1]?.env).toEqual({ A: "1", B: "3" });
  });

  it("stops a slot without starting it again, even when its run rejects on abort", async () => {
    const { host, of, sent } = setup((call) => new Promise<void>((_, reject) => call.signal.addEventListener("abort", () => reject(new Error("aborted")))));
    await host.handle({ op: "start", slot: 3, env: {} });
    await host.handle({ op: "stop", slot: 3 });
    expect(of(3)[0]?.signal.aborted).toBe(true);
    expect(sent).toEqual([{ slot: 3, event: "started" }, { slot: 3, event: "stopped" }]);
    await sleep(100);
    expect(of(3)).toHaveLength(1);
    expect(sent).toHaveLength(2);
    expect(host.running()).toEqual([]);
  });

  it("does not start again a slot stopped while it waited for the delay", async () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { host, of, sent } = setup(async () => {
      throw new Error("down");
    });
    await host.handle({ op: "start", slot: 2, env: {} });
    await vi.waitFor(() => expect(sent).toContainEqual({ slot: 2, event: "exited", how: "down" }));
    await host.handle({ op: "stop", slot: 2 });
    const n = of(2).length;
    await sleep(100);
    expect(of(2)).toHaveLength(n);
    expect(sent.at(-1)).toEqual({ slot: 2, event: "stopped" });
  });

  it("answers a stop of an unknown slot at once", async () => {
    const { host, sent } = setup();
    await host.handle({ op: "stop", slot: 9 });
    expect(sent).toEqual([{ slot: 9, event: "stopped" }]);
  });

  it("answers a stop after the grace time when the run does not end", async () => {
    const { host, sent } = setup(() => new Promise<void>(() => undefined));
    await host.handle({ op: "start", slot: 2, env: {} });
    const t0 = Date.now();
    await host.handle({ op: "stop", slot: 2 });
    expect(Date.now() - t0).toBeGreaterThanOrEqual(150);
    expect(sent.at(-1)).toEqual({ slot: 2, event: "stopped" });
  });

  it("aborts every slot on shutdown and waits for their runs", async () => {
    let ended = 0;
    const { host, of } = setup(async (call) => {
      await untilAbort(call);
      await sleep(20);
      ended++;
    });
    await host.handle({ op: "start", slot: 2, env: {} });
    await host.handle({ op: "start", slot: 3, env: {} });
    await host.shutdown(1000);
    expect([of(2)[0]?.signal.aborted, of(3)[0]?.signal.aborted]).toEqual([true, true]);
    expect(ended).toBe(2);
    expect(host.running()).toEqual([]);
  });

  it("gives up waiting on shutdown after the grace time", async () => {
    const { host } = setup(() => new Promise<void>(() => undefined));
    await host.handle({ op: "start", slot: 2, env: {} });
    const t0 = Date.now();
    await host.shutdown(100);
    expect(Date.now() - t0).toBeGreaterThanOrEqual(80);
    expect(Date.now() - t0).toBeLessThan(1000);
  });
});
