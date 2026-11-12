// The worklet that plays a call answered in the app, run here on a fake AudioWorklet scope: the chunks the page posts
// (decoded Opus, one Float32Array per channel) and the quanta of 128 samples the audio thread asks for.
import fs from "node:fs";
import { describe, expect, it } from "vitest";
import { PLAYER } from "@/lib/call-audio/player";

type Player = {
  port: { onmessage: ((e: { data: Float32Array[] }) => void) | null };
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean;
  target: number;
  queued: number;
  underruns: number;
};

function player(): Player {
  let made: (new () => Player) | null = null;
  class Base {
    port = { onmessage: null, postMessage() {} };
  }
  new Function("AudioWorkletProcessor", "registerProcessor", "sampleRate", PLAYER)(Base, (_: string, c: new () => Player) => (made = c), 48000);
  return new made!();
}

// one decoded 20 ms packet, mono
const push = (p: Player, value: number) => p.port.onmessage!({ data: [new Float32Array(960).fill(value)] });
const render = (p: Player) => {
  const out = [new Float32Array(128), new Float32Array(128)];
  p.process([], [out]);
  return out;
};
const ms = (samples: number) => samples / 48;

describe("call audio player worklet", () => {
  it("waits for 60 ms, fades in, then plays a mono stream on both channels", () => {
    const p = player();
    for (let i = 0; i < 2; i++) push(p, 0.5);
    expect(render(p)[0].every((v) => v === 0)).toBe(true);
    push(p, 0.5);
    const [left] = render(p);
    expect(left[0]).toBe(0);
    expect(left[127]).toBeGreaterThan(0.49);
    const [l, r] = render(p);
    expect(l.every((v) => v === 0.5) && r.every((v) => v === 0.5)).toBe(true);
  });

  it("fades out when it runs dry, raises the target by 60 ms and plays again at 60 ms queued", () => {
    const p = player();
    for (let i = 0; i < 5; i++) push(p, 0.5);
    for (let i = 0; i < 37; i++) render(p);
    const [dry] = render(p);
    expect(p.underruns).toBe(1);
    expect(ms(p.target)).toBe(160);
    expect(dry[63]).toBe(0.5);
    expect(dry[64]).toBeGreaterThan(0);
    expect(dry[127]).toBeLessThan(0.01);
    expect(render(p)[0].every((v) => v === 0)).toBe(true);
    for (let i = 0; i < 2; i++) push(p, 0.5);
    expect(render(p)[0].every((v) => v === 0)).toBe(true);
    push(p, 0.5);
    expect(render(p)[0][127]).toBeGreaterThan(0.49);
  });

  it("eases the target by 20 ms after 20 s without running dry", () => {
    const p = player();
    for (let i = 0; i < 5; i++) push(p, 0.5);
    for (let i = 0; i < 38; i++) render(p);
    expect(ms(p.target)).toBe(160);
    for (let i = 0; i < 8; i++) push(p, 0.5);
    for (let s = 0; s < 500; s++) {
      for (let i = 0; i < 15; i++) render(p);
      push(p, 0.5);
      push(p, 0.5);
    }
    expect(p.underruns).toBe(1);
    expect(ms(p.target)).toBe(140);
  });

  it("skips quiet packets, never speech, to win back the delay above the target", () => {
    const p = player();
    for (let i = 0; i < 10; i++) push(p, 0.001);
    for (let i = 0; i < 10; i++) push(p, 0.5);
    render(p);
    expect(p.queued).toBe(9600 - 128);
    for (let i = 0; i < 74; i++) expect(render(p)[0].every((v) => v === 0.5)).toBe(true);
  });

  it("drops the oldest audio once 500 ms over the target", () => {
    const p = player();
    for (let i = 0; i < 30; i++) push(p, 0.5);
    expect(ms(p.queued)).toBe(600);
    push(p, 0.5);
    expect(ms(p.queued)).toBe(100);
  });

  it("keeps a call from a jittery link playing: recorded arrivals, 1.5 s talk and 1.5 s pause", () => {
    // 20 ms packets as they reached a laptop on home FWA from the server (test/fixtures). The previous fixed 60 ms
    // prebuffer ran dry 20 times on this trace and dropped 1.5 s.
    const { down } = JSON.parse(fs.readFileSync(new URL("./fixtures/call-audio-arrivals.json", import.meta.url), "utf8")) as { down: number[] };
    const p = player();
    let at = 0;
    let next = 0;
    for (let quantum = 0; next < down.length; quantum++) {
      const clock = (quantum * 128) / 48;
      while (next < down.length && at + down[next] <= clock) {
        at += down[next];
        push(p, Math.floor(next / 75) % 2 ? 0.001 : 0.5);
        next++;
      }
      render(p);
    }
    expect(p.underruns).toBeLessThanOrEqual(10);
  });
});
