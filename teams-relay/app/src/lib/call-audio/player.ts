import { OPUS_RATE } from "./frames";

// The worklet that plays the decoded call. The packets come over a websocket, late and in bursts on a jittery link or
// when the server stalls. Playing starts once 60 ms are queued (again after running dry), and the burst that follows a
// stall stays queued as the cushion for the next one. Quiet packets are skipped to win the delay back only above a
// target that grows 60 ms each time the queue runs dry (100 ms to start, at most 400 ms) and eases 20 ms after 20 s
// without that (at least 60 ms); 500 ms over it, the oldest go. Running dry fades out instead of cutting off. A mono
// stream plays on both channels.
export const PLAYER = `
class CallAudioPlayer extends AudioWorkletProcessor {
  constructor() {
    super();
    const ms = (n) => Math.round((sampleRate * n) / 1000);
    this.start = ms(60);
    this.min = ms(60);
    this.max = ms(400);
    this.up = ms(60);
    this.down = ms(20);
    this.margin = ms(60);
    this.excess = ms(500);
    this.calmFor = ms(20000);
    this.target = ms(100);
    this.chunks = [];
    this.offset = 0;
    this.queued = 0;
    this.playing = false;
    this.resumed = false;
    this.calm = 0;
    this.underruns = 0;
    this.last = [];
    this.fresh = false;
    this.port.onmessage = (e) => {
      let peak = 0;
      for (const v of e.data[0]) peak = Math.max(peak, Math.abs(v));
      this.chunks.push({ planes: e.data, peak });
      this.queued += e.data[0].length;
      if (this.queued <= this.target + this.excess) return;
      while (this.chunks.length > 1 && this.queued > this.target) {
        this.queued -= this.chunks.shift().planes[0].length - this.offset;
        this.offset = 0;
      }
    };
  }
  process(inputs, outputs) {
    const out = outputs[0];
    const n = out[0].length;
    if (!this.playing) {
      if (this.queued < this.start) {
        this.conceal(out, 0);
        return true;
      }
      this.playing = true;
      this.resumed = true;
    }
    while (this.offset === 0 && this.chunks.length > 1 && this.chunks[0].peak < 0.02 && this.queued - this.chunks[0].planes[0].length > this.target + this.margin) {
      this.queued -= this.chunks.shift().planes[0].length;
    }
    let done = 0;
    while (done < n && this.chunks.length) {
      const chunk = this.chunks[0].planes;
      const take = Math.min(n - done, chunk[0].length - this.offset);
      for (let c = 0; c < out.length; c++) out[c].set(chunk[Math.min(c, chunk.length - 1)].subarray(this.offset, this.offset + take), done);
      done += take;
      this.offset += take;
      this.queued -= take;
      if (this.offset >= chunk[0].length) {
        this.chunks.shift();
        this.offset = 0;
      }
    }
    if (this.resumed) {
      for (const channel of out) for (let i = 0; i < done; i++) channel[i] *= i / n;
      this.resumed = false;
    }
    if (done < n) {
      this.playing = false;
      this.calm = 0;
      this.underruns++;
      this.target = Math.min(this.max, this.target + this.up);
      this.conceal(out, done);
      return true;
    }
    if ((this.calm += n) >= this.calmFor) {
      this.calm = 0;
      this.target = Math.max(this.min, this.target - this.down);
    }
    if (this.last.length !== out.length || this.last[0].length !== n) this.last = out.map(() => new Float32Array(n));
    for (let c = 0; c < out.length; c++) this.last[c].set(out[c]);
    this.fresh = true;
    return true;
  }
  // from sample from on: the quantum before once more, fading to silence; silence after it
  conceal(out, from) {
    const n = out[0].length;
    for (let c = 0; c < out.length; c++) {
      for (let i = from; i < n; i++) out[c][i] = this.fresh && this.last[c] ? (this.last[c][i] * (n - i)) / n : 0;
    }
    this.fresh = false;
  }
}
registerProcessor("call-audio-player", CallAudioPlayer);
`;

// A worklet module from its source, without a file of its own on the server
export async function addWorklet(ctx: BaseAudioContext, source: string) {
  const url = URL.createObjectURL(new Blob([source], { type: "application/javascript" }));
  try {
    await ctx.audioWorklet.addModule(url);
  } finally {
    URL.revokeObjectURL(url);
  }
}

// Opus packets of the call, decoded by WebCodecs and played on the given output of an AudioContext of 48 kHz
export class OpusPlayer {
  private decoder: AudioDecoder | null = null;
  private node: AudioWorkletNode | null = null;
  private count = 2;
  private timestamp = 0;

  constructor(
    private readonly ctx: AudioContext,
    private readonly output: AudioNode,
  ) {}

  async ready() {
    await addWorklet(this.ctx, PLAYER);
    this.node = new AudioWorkletNode(this.ctx, "call-audio-player", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
    this.node.connect(this.output);
    this.configure();
  }

  channels(n: number) {
    if (n === this.count) return;
    this.count = n;
    if (this.node) this.configure();
  }

  play(packet: Uint8Array) {
    const d = this.decoder;
    // A burst after a stall arrives in one go and is the worklet's to absorb; only a decoder seconds behind drops packets.
    if (!d || d.state !== "configured" || d.decodeQueueSize > 100) return;
    try {
      d.decode(new EncodedAudioChunk({ type: "key", timestamp: this.timestamp, data: packet }));
      this.timestamp += 20_000;
    } catch {
      // a packet the decoder refuses is skipped
    }
  }

  close() {
    if (this.decoder && this.decoder.state !== "closed") this.decoder.close();
    this.decoder = null;
    this.node?.disconnect();
    this.node = null;
  }

  private configure() {
    if (this.decoder && this.decoder.state !== "closed") this.decoder.close();
    this.decoder = new AudioDecoder({ output: (frame) => this.frame(frame), error: () => undefined });
    this.decoder.configure({ codec: "opus", sampleRate: OPUS_RATE, numberOfChannels: this.count });
  }

  private frame(frame: AudioData) {
    try {
      const planes: Float32Array[] = [];
      for (let c = 0; c < frame.numberOfChannels; c++) {
        const plane = new Float32Array(frame.numberOfFrames);
        frame.copyTo(plane, { planeIndex: c, format: "f32-planar" });
        planes.push(plane);
      }
      this.node?.port.postMessage(
        planes,
        planes.map((p) => p.buffer),
      );
    } finally {
      frame.close();
    }
  }
}
