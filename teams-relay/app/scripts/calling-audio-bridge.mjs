// PCM/Opus boundary for the existing call-audio hub. Inject codec and transport.
// The app's microphone comes over a websocket: a jittery link, or a stall of this host, delivers it late and in
// bursts. It plays once 60 ms are queued (again after running dry), and the burst that follows a stall stays queued as
// the cushion for the next one. Quiet frames are dropped to win the delay back only above a target that grows 60 ms
// with each underrun (100 ms to start, at most 400 ms) and eases 20 ms after 20 s without one (at least 60 ms); 500 ms
// over it, the oldest audio goes. An underrun fades the last frame out instead of cutting it.
const RATE = 48000;
const FRAME = 480;
const ms = (n) => (n * RATE) / 1000;
export const JITTER = { start: ms(60), target: ms(100), min: ms(60), max: ms(400), up: ms(60), down: ms(20), calmFrames: 2000, margin: ms(60), excess: ms(500), quiet: 650 };

export class CallingAudioBridge {
  constructor({ codec, send }) {
    this.encoder = new codec(48000, 1, codec.Application.VOIP);
    this.encoder.setBitrate(32000);
    this.decoder = new codec(48000, 1);
    this.send = send;
    this.down = [];
    this.ring = new Int16Array(JITTER.max + JITTER.excess + ms(100));
    this.head = 0; this.queued = 0;
    this.target = JITTER.target; this.playing = false; this.calm = 0; this.resumed = false;
    this.last = new Int16Array(FRAME);
    this.closed = false;
    this.stats = { down: 0, up: 0, invalid: 0, droppedSamples: 0, quietSamples: 0, underruns: 0, maxTarget: JITTER.target };
  }
  start() { this.send('CHANNELS 1'); this.send('CAPTURE_DEMAND microphone 1'); }
  receive(frame) {
    if (this.closed) return;
    const b = Buffer.from(frame);
    if (b.length < 2 || b.length > 3829 || b[0] !== 2) { this.stats.invalid++; return; }
    let pcm;
    try { pcm = this.decoder.decode(b.subarray(1)); } catch { this.stats.invalid++; return; }
    const n = pcm.length >> 1;
    if (this.queued + n > this.ring.length) this.discard(this.queued + n - this.ring.length);
    for (let i = 0; i < n; i++) this.ring[(this.head + this.queued + i) % this.ring.length] = pcm.readInt16LE(i * 2);
    this.queued += n;
    if (this.queued > this.target + JITTER.excess) this.discard(this.queued - this.target);
    this.stats.up++;
  }
  microphone() {
    const out = new Int16Array(FRAME);
    if (this.closed) return out;
    if (!this.playing) {
      if (this.queued < JITTER.start) return this.conceal(out);
      this.playing = true; this.resumed = true;
    }
    if (this.queued < FRAME) {
      this.playing = false; this.calm = 0;
      this.stats.underruns++;
      this.target = Math.min(JITTER.max, this.target + JITTER.up);
      this.stats.maxTarget = Math.max(this.stats.maxTarget, this.target);
      return this.conceal(out);
    }
    while (this.queued - FRAME > this.target + JITTER.margin && this.peak() < JITTER.quiet) { this.skip(FRAME); this.stats.quietSamples += FRAME; }
    for (let i = 0; i < FRAME; i++) out[i] = this.ring[(this.head + i) % this.ring.length];
    this.skip(FRAME);
    if (this.resumed) { for (let i = 0; i < FRAME; i++) out[i] = Math.round((out[i] * i) / FRAME); this.resumed = false; }
    if (++this.calm >= JITTER.calmFrames) { this.calm = 0; this.target = Math.max(JITTER.min, this.target - JITTER.down); }
    this.last.set(out);
    return out;
  }
  // the frame before an underrun once more, fading to silence; silence after it
  conceal(out) {
    for (let i = 0; i < FRAME; i++) out[i] = Math.round((this.last[i] * (FRAME - i)) / FRAME);
    this.last.fill(0);
    return out;
  }
  peak() {
    let p = 0;
    for (let i = 0; i < FRAME; i++) p = Math.max(p, Math.abs(this.ring[(this.head + i) % this.ring.length]));
    return p;
  }
  skip(n) { this.head = (this.head + n) % this.ring.length; this.queued -= n; }
  discard(n) { this.skip(n); this.stats.droppedSamples += n; }
  speaker(frame) {
    if (this.closed) return;
    const { samples, sampleRate, channelCount, numberOfFrames } = frame;
    if (![16000, 32000, 48000].includes(sampleRate) || ![1, 2].includes(channelCount) || !Number.isInteger(numberOfFrames) || numberOfFrames < 1 || numberOfFrames > 4800 || samples.length < numberOfFrames * channelCount) { this.stats.invalid++; return; }
    const mono = (i) => {
      let sum = 0;
      for (let c = 0; c < channelCount; c++) sum += samples[Math.min(numberOfFrames - 1, i) * channelCount + c];
      return sum / channelCount;
    };
    for (let i = 0; i < Math.round(numberOfFrames * 48000 / sampleRate); i++) {
      const p = i * sampleRate / 48000, lo = Math.floor(p), f = p - lo;
      this.down.push(Math.round(mono(lo) * (1 - f) + mono(lo + 1) * f));
    }
    while (this.down.length >= 960) {
      const pcm = Buffer.allocUnsafe(1920);
      this.down.splice(0, 960).forEach((v, i) => pcm.writeInt16LE(v, i * 2));
      this.send(this.encoder.encode(pcm, 960)); this.stats.down++;
    }
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    try { this.send('CAPTURE_DEMAND microphone 0'); }
    finally { this.encoder.delete(); this.decoder.delete(); this.queued = 0; this.down.length = 0; }
  }
}
