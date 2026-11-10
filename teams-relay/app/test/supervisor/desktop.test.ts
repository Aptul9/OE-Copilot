import { describe, expect, it, vi } from "vitest";
import { Desktop, type DesktopOptions } from "@/supervisor/desktop";

const SOCKET = "/xdg/wayland-0";
// -b: wait for the lock of the s6-rc database, which the boot transition holds while the supervisor already runs
const START = "/command/s6-rc -b -u change svc-de svc-xsettingsd svc-dbus";
const STOP = "/command/s6-rc -b -d change svc-de svc-xsettingsd svc-dbus svc-selkies svc-nginx svc-xorg svc-pulseaudio";

function setup(over: Partial<DesktopOptions> = {}) {
  const ctx = { calls: [] as string[], present: new Set<string>(), t: 1_000_000, delay: 0, fail: false, failFile: "", holdStop: null as Promise<void> | null };
  const d = new Desktop({
    socket: SOCKET,
    s6rc: "/command/s6-rc",
    microphone: null,
    idleMs: 600_000,
    log: () => undefined,
    run: async (file, args) => {
      ctx.calls.push([file, ...args].join(" "));
      if (ctx.fail) throw new Error("s6-rc failed");
      if (file === ctx.failFile) throw new Error("no pactl");
      if (args.includes("-u")) setTimeout(() => ctx.present.add(SOCKET), ctx.delay);
      if (args.includes("-d")) {
        await ctx.holdStop;
        ctx.present.delete(SOCKET);
      }
    },
    exists: (f) => ctx.present.has(f),
    now: () => ctx.t,
    pollMs: 1,
    waitMs: 500,
    ...over,
  });
  return { d, ctx };
}

describe("desktop", () => {
  it("starts the desktop once for callers that ask together, and answers when the socket is there", async () => {
    const { d, ctx } = setup();
    ctx.delay = 20;
    await Promise.all([d.ensure(), d.ensure(), d.ensure()]);
    expect(ctx.calls).toEqual([START]);
    expect(ctx.present.has(SOCKET)).toBe(true);
    expect(d.running).toBe(true);
  });

  it("does not start it again while it is up", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    await d.ensure();
    expect(ctx.calls).toEqual([START]);
  });

  it("runs the microphone script once the desktop is up, and carries on when it fails", async () => {
    const log = vi.fn();
    const first = setup({ microphone: "/usr/local/bin/desktop-microphone" });
    await first.d.ensure();
    expect(first.ctx.calls).toEqual([START, "/usr/local/bin/desktop-microphone"]);

    const second = setup({ microphone: "/m", log });
    second.ctx.failFile = "/m";
    await second.d.ensure();
    expect(log).toHaveBeenCalledWith("desktop microphone: no pactl");
    expect(second.d.running).toBe(true);
  });

  it("stops it after the idle time without a browser, not before, and a browser resets the clock", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    ctx.t += 9 * 60_000;
    await d.tick(false);
    expect(ctx.calls).toEqual([START]);
    await d.tick(true);
    ctx.t += 9 * 60_000;
    await d.tick(false);
    expect(ctx.calls).toEqual([START]);
    ctx.t += 2 * 60_000;
    await d.tick(false);
    expect(ctx.calls).toEqual([START, STOP]);
    expect(d.running).toBe(false);
  });

  it("starts it again after it was stopped", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    ctx.t += 11 * 60_000;
    await d.tick(false);
    await d.ensure();
    expect(ctx.calls).toEqual([START, STOP, START]);
    expect(d.running).toBe(true);
  });

  it("keeps the desktop when a start is asked while a stop waits its turn", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    ctx.t += 11 * 60_000;
    const stopping = d.tick(false);
    const starting = d.ensure();
    await Promise.all([stopping, starting]);
    expect(ctx.calls).toEqual([START]);
    expect(d.running).toBe(true);
  });

  it("starts the desktop again after a stop in progress ends, for a start asked meanwhile", async () => {
    const { d, ctx } = setup();
    let release = () => {};
    ctx.holdStop = new Promise<void>((resolve) => (release = resolve));
    await d.ensure();
    ctx.t += 11 * 60_000;
    const stopping = d.tick(false);
    await vi.waitFor(() => expect(ctx.calls).toEqual([START, STOP]));
    const starting = d.ensure();
    release();
    await Promise.all([stopping, starting]);
    expect(ctx.calls).toEqual([START, STOP, START]);
    expect(d.running).toBe(true);
  });

  it("stays down and rejects the callers when the start fails, and tries again at the next ask", async () => {
    const { d, ctx } = setup();
    ctx.fail = true;
    await expect(Promise.all([d.ensure(), d.ensure()])).rejects.toThrow("s6-rc failed");
    expect(d.running).toBe(false);
    ctx.fail = false;
    await d.ensure();
    expect(d.running).toBe(true);
  });

  it("never stops with an infinite idle time, or without s6", async () => {
    const keep = setup({ idleMs: Infinity });
    await keep.d.ensure();
    keep.ctx.t += 24 * 3600_000;
    await keep.d.tick(false);
    expect(keep.ctx.calls).toEqual([START]);

    const bare = setup({ s6rc: null });
    bare.ctx.present.add(SOCKET);
    await bare.d.ensure();
    bare.ctx.t += 24 * 3600_000;
    await bare.d.tick(false);
    expect(bare.ctx.calls).toEqual([]);
    expect(bare.d.running).toBe(true);
  });

  it("gives up when the Wayland socket does not come", async () => {
    const d = new Desktop({ socket: "/never", s6rc: null, microphone: null, idleMs: 1, log: () => undefined, exists: () => false, now: Date.now, pollMs: 2, waitMs: 40 });
    await expect(d.ensure()).rejects.toThrow(/did not come up/);
    expect(d.running).toBe(false);
  });
});
