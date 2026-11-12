import fs from 'node:fs';
import { describe, expect, it } from 'vitest';
import { CallingAudioBridge, JITTER } from '../../scripts/calling-audio-bridge.mjs';
class Codec {
  static Application = { VOIP: 2048 };
  deleted = false;
  setBitrate() {}
  encode(pcm: Buffer) { return Buffer.from(pcm); }
  decode(packet: Buffer) { return Buffer.from(packet); }
  delete() { this.deleted = true; }
}
const setup = () => { const sent: (string | Buffer)[] = []; return { sent, bridge: new CallingAudioBridge({ codec: Codec, send: (v: string | Buffer) => sent.push(v) }) }; };
// one 20 ms microphone packet as the test codec decodes it: 960 samples of the value
const packet = (value: number) => { const pcm = Buffer.alloc(1920); for (let i = 0; i < 960; i++) pcm.writeInt16LE(value, i * 2); return Buffer.concat([Buffer.from([2]), pcm]); };
const ms = (samples: number) => samples / 48;
describe('native calling audio hub adapter', () => {
  it('requests mono audio and stops once', () => {
    const { bridge, sent } = setup(); bridge.start(); bridge.close(); bridge.close();
    expect(sent).toEqual(['CHANNELS 1', 'CAPTURE_DEMAND microphone 1', 'CAPTURE_DEMAND microphone 0']);
    expect(bridge.encoder.deleted && bridge.decoder.deleted).toBe(true);
  });
  it('upsamples Teams 16kHz to 48kHz and emits 20ms byte PCM packets', () => {
    const { bridge, sent } = setup();
    const frame = { samples: new Int16Array(160).fill(1234), sampleRate: 16000, channelCount: 1, numberOfFrames: 160 };
    bridge.speaker(frame); expect(sent).toHaveLength(0); bridge.speaker(frame);
    expect(sent).toHaveLength(1); const b = sent[0] as Buffer;
    expect(b.length).toBe(1920); expect(b.readInt16LE(100)).toBe(1234);
  });
  it('downmixes stereo before encoding', () => {
    const { bridge, sent } = setup(); const samples = new Int16Array(1920);
    for (let i = 0; i < 960; i++) { samples[i * 2] = 4000; samples[i * 2 + 1] = -2000; }
    bridge.speaker({ samples, sampleRate: 48000, channelCount: 2, numberOfFrames: 960 });
    expect((sent[0] as Buffer).readInt16LE(0)).toBe(1000);
  });
  it('waits for 60 ms, fades in, then feeds 10 ms microphone frames', () => {
    const { bridge } = setup();
    for (let i = 0; i < 2; i++) bridge.receive(packet(2345));
    expect(bridge.microphone().every((x: number) => x === 0)).toBe(true);
    bridge.receive(packet(2345));
    const first = bridge.microphone();
    expect(first[0]).toBe(0); expect(first[479]).toBeGreaterThan(2300); expect(first[240]).toBeLessThan(first[479]);
    expect(bridge.microphone().every((x: number) => x === 2345)).toBe(true);
  });
  it('fades the last frame out on an underrun, raises the target by 60 ms and plays again at 60 ms queued', () => {
    const { bridge } = setup();
    for (let i = 0; i < 5; i++) bridge.receive(packet(2345));
    for (let i = 0; i < 10; i++) bridge.microphone();
    const faded = bridge.microphone();
    expect(bridge.stats.underruns).toBe(1); expect(ms(bridge.target)).toBe(160);
    expect(faded[0]).toBe(2345); expect(faded[479]).toBeLessThan(10);
    expect(bridge.microphone().every((x: number) => x === 0)).toBe(true);
    for (let i = 0; i < 2; i++) bridge.receive(packet(2345));
    expect(bridge.microphone().every((x: number) => x === 0)).toBe(true);
    bridge.receive(packet(2345));
    expect(bridge.microphone()[479]).toBeGreaterThan(2300);
  });
  it('never grows the target past 400 ms, and eases it by 20 ms after 20 s without an underrun', () => {
    const { bridge } = setup();
    for (let round = 0; round < 10; round++) {
      for (let i = 0; i * 960 < bridge.target; i++) bridge.receive(packet(2345));
      while (bridge.queued >= 480) bridge.microphone();
      bridge.microphone();
    }
    expect(ms(bridge.target)).toBe(400); expect(ms(bridge.stats.maxTarget)).toBe(400);
    for (let i = 0; i < 25; i++) bridge.receive(packet(2345));
    for (let i = 0; i < JITTER.calmFrames; i++) { bridge.microphone(); if (i % 2) bridge.receive(packet(2345)); }
    expect(ms(bridge.target)).toBe(380);
  });
  it('drops quiet frames, never speech, to win back the delay above the target', () => {
    const { bridge } = setup();
    for (let i = 0; i < 10; i++) bridge.receive(packet(0));
    for (let i = 0; i < 10; i++) bridge.receive(packet(2345));
    bridge.microphone();
    expect(ms(bridge.stats.quietSamples)).toBe(200); expect(ms(bridge.queued)).toBe(190);
    for (let i = 0; i < 19; i++) expect(bridge.microphone().every((x: number) => x === 2345)).toBe(true);
    expect(bridge.stats.droppedSamples).toBe(0);
  });
  it('drops the oldest audio once 500 ms over the target, and rejects malformed input', () => {
    const { bridge } = setup();
    for (let i = 0; i < 31; i++) bridge.receive(Buffer.alloc(1921, 2));
    expect(ms(bridge.queued)).toBe(100); expect(ms(bridge.stats.droppedSamples)).toBe(520);
    bridge.receive(Buffer.from([1, 2])); bridge.receive(Buffer.alloc(4000));
    bridge.speaker({ samples: [], sampleRate: 123, channelCount: 1, numberOfFrames: 1 });
    expect(bridge.stats.invalid).toBe(3);
    bridge.close(); bridge.receive(Buffer.alloc(1921, 2)); expect(bridge.queued).toBe(0);
  });
  it('keeps a microphone from a jittery link flowing: recorded arrivals, 1.5 s talk and 1.5 s pause', () => {
    // 20 ms packets as they reached the server from a laptop on home FWA (test/fixtures). The previous fixed 60 ms
    // prebuffer ran dry 56 times on this trace and dropped 5.5 s.
    const { up } = JSON.parse(fs.readFileSync(new URL('../fixtures/call-audio-arrivals.json', import.meta.url), 'utf8')) as { up: number[] };
    const { bridge } = setup();
    let at = 0, next = 0, sent = 0;
    for (let clock = 0; next < up.length || (bridge.playing && bridge.queued >= 480); clock += 10) {
      while (next < up.length && at + up[next] <= clock) { at += up[next]; bridge.receive(packet(Math.floor(sent / 75) % 2 ? 0 : 2345)); sent++; next++; }
      bridge.microphone();
    }
    expect(bridge.stats.underruns).toBeLessThanOrEqual(15);
    expect(bridge.stats.droppedSamples).toBe(0);
    expect(bridge.stats.quietSamples).toBeGreaterThan(0);
  });
});
