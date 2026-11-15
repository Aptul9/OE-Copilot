//! The audio codecs of a Teams call, on 20 ms frames of 48 kHz mono PCM: Opus (libopus, statically linked), G.722
//! (16 kHz, 64 kbit/s) and G.711 µ-law and A-law (8 kHz), whichever the far end answered with; and the test melody of
//! calling-host-helpers.mjs with the detection of its notes in what comes back.

use std::f64::consts::PI;

pub const RATE: u32 = 48_000;
/// samples of 20 ms at 48 kHz
pub const FRAME_20: usize = 960;
/// samples of 10 ms at 48 kHz
pub const FRAME_10: usize = 480;

/// libopus through its C interface: an encoder or a decoder, 48 kHz mono.
pub struct Opus {
    enc: *mut audiopus_sys::OpusEncoder,
    dec: *mut audiopus_sys::OpusDecoder,
}

// SAFETY: the libopus states are used by one owner at a time (&mut self), never shared
unsafe impl Send for Opus {}

impl Opus {
    fn new() -> Result<Opus, String> {
        let mut err = 0;
        // SAFETY: plain constructors of libopus; the result is checked before use
        let enc = unsafe { audiopus_sys::opus_encoder_create(RATE as i32, 1, audiopus_sys::OPUS_APPLICATION_VOIP, &mut err) };
        if enc.is_null() || err != 0 {
            return Err(format!("opus encoder: {err}"));
        }
        // SAFETY: as above
        let dec = unsafe { audiopus_sys::opus_decoder_create(RATE as i32, 1, &mut err) };
        if dec.is_null() || err != 0 {
            // SAFETY: enc was created above and is not used again
            unsafe { audiopus_sys::opus_encoder_destroy(enc) };
            return Err(format!("opus decoder: {err}"));
        }
        // SAFETY: a valid encoder; OPUS_SET_BITRATE_REQUEST takes one opus_int32
        unsafe { audiopus_sys::opus_encoder_ctl(enc, audiopus_sys::OPUS_SET_BITRATE_REQUEST, 32_000i32) };
        Ok(Opus { enc, dec })
    }

    fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        let mut out = vec![0u8; 1500];
        // SAFETY: pcm holds a whole frame (2.5-60 ms at 48 kHz), out has room for the largest packet asked for
        let n = unsafe { audiopus_sys::opus_encode(self.enc, pcm.as_ptr(), pcm.len() as i32, out.as_mut_ptr(), out.len() as i32) };
        out.truncate(n.max(0) as usize);
        out
    }

    /// PCM of a packet; an empty packet conceals one frame of 20 ms.
    fn decode(&mut self, packet: &[u8]) -> Vec<i16> {
        let mut out = vec![0i16; 5760];
        let (data, len) = if packet.is_empty() { (std::ptr::null(), 0) } else { (packet.as_ptr(), packet.len() as i32) };
        let frames = if packet.is_empty() { FRAME_20 as i32 } else { out.len() as i32 };
        // SAFETY: out has room for 120 ms of mono at 48 kHz, the largest frame of Opus
        let n = unsafe { audiopus_sys::opus_decode(self.dec, data, len, out.as_mut_ptr(), frames, 0) };
        out.truncate(n.max(0) as usize);
        out
    }
}

impl Drop for Opus {
    fn drop(&mut self) {
        // SAFETY: both were created by Opus::new and are destroyed once
        unsafe {
            audiopus_sys::opus_encoder_destroy(self.enc);
            audiopus_sys::opus_decoder_destroy(self.dec);
        }
    }
}

/// Opus of 20 ms frames of 48 kHz mono, as the audio hub of the app speaks it (calling-audio-bridge.mjs).
pub struct OpusCodec(Opus);

impl OpusCodec {
    pub fn new() -> Result<OpusCodec, String> {
        Opus::new().map(OpusCodec)
    }
    pub fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        self.0.encode(pcm)
    }
    pub fn decode(&mut self, packet: &[u8]) -> Vec<i16> {
        self.0.decode(packet)
    }
}

/// 48 kHz down to `rate` (a divisor of it): each output sample the mean of its block, a plain low-pass.
pub fn down(pcm: &[i16], rate: u32) -> Vec<i16> {
    let f = (RATE / rate) as usize;
    pcm.chunks(f).map(|c| (c.iter().map(|&v| i32::from(v)).sum::<i32>() / c.len() as i32) as i16).collect()
}

/// `rate` up to 48 kHz, linearly; `last` is the final sample of the frame before, for a seamless join.
pub fn up(pcm: &[i16], rate: u32, last: &mut i16) -> Vec<i16> {
    let f = (RATE / rate) as usize;
    let mut out = Vec::with_capacity(pcm.len() * f);
    let mut prev = i32::from(*last);
    for &v in pcm {
        let v = i32::from(v);
        for k in 1..=f {
            out.push((prev + (v - prev) * k as i32 / f as i32) as i16);
        }
        prev = v;
    }
    *last = prev as i16;
    out
}

/// G.711 µ-law of one sample.
pub fn ulaw(sample: i16) -> u8 {
    const BIAS: i32 = 0x84;
    const CLIP: i32 = 32635;
    let mut s = i32::from(sample);
    let sign = if s < 0 { 0x80 } else { 0 };
    if s < 0 {
        s = -s;
    }
    s = s.min(CLIP) + BIAS;
    let exponent = (7 - (s.leading_zeros() as i32 - 17)).clamp(0, 7);
    let mantissa = (s >> (exponent + 3)) & 0x0f;
    !(sign | (exponent << 4) | mantissa) as u8
}

pub fn ulaw_decode(byte: u8) -> i16 {
    let u = !byte;
    let sign = u & 0x80;
    let exponent = i32::from((u >> 4) & 0x07);
    let mantissa = i32::from(u & 0x0f);
    let s = (((mantissa << 3) + 0x84) << exponent) - 0x84;
    (if sign != 0 { -s } else { s }) as i16
}

/// G.711 A-law of one sample.
pub fn alaw(sample: i16) -> u8 {
    let mut s = i32::from(sample);
    let sign = if s >= 0 { 0x80 } else { 0 };
    if s < 0 {
        s = -s - 1;
    }
    s = s.min(32767) >> 3;
    let (exponent, mantissa) = if s < 32 {
        (0, s >> 1)
    } else {
        let e = 31 - (s as u32).leading_zeros() as i32 - 4;
        (e, (s >> e) & 0x0f)
    };
    ((sign | (exponent << 4) | mantissa) ^ 0x55) as u8
}

pub fn alaw_decode(byte: u8) -> i16 {
    let a = i32::from(byte ^ 0x55);
    let exponent = (a >> 4) & 0x07;
    let mantissa = a & 0x0f;
    let s = if exponent == 0 { (mantissa << 4) + 8 } else { ((mantissa << 4) + 0x108) << (exponent - 1) };
    (if a & 0x80 != 0 { s } else { -s }) as i16
}

/// The state of G.722 both ways, and the last sample decoded for the join of two frames.
pub struct G722 {
    enc: ezk_g722::libg722::encoder::Encoder,
    dec: ezk_g722::libg722::decoder::Decoder,
    last: i16,
}

/// The codec a call sends and receives with: by its RTP name.
pub enum Codec {
    Opus(Opus),
    G722(Box<G722>),
    Pcmu { last: i16 },
    Pcma { last: i16 },
}

impl Codec {
    /// The codec of an rtpmap name (opus, G722, PCMU, PCMA); None for one this client does not speak.
    pub fn of(name: &str) -> Option<Codec> {
        use ezk_g722::libg722::{Bitrate, decoder::Decoder, encoder::Encoder};
        Some(match name.to_ascii_lowercase().as_str() {
            "opus" => Codec::Opus(Opus::new().ok()?),
            "g722" => Codec::G722(Box::new(G722 {
                enc: Encoder::new(Bitrate::Mode1_64000, false, false),
                dec: Decoder::new(Bitrate::Mode1_64000, false, false),
                last: 0,
            })),
            "pcmu" => Codec::Pcmu { last: 0 },
            "pcma" => Codec::Pcma { last: 0 },
            _ => return None,
        })
    }

    /// The RTP clock rate of the codec (G.722 keeps 8000 by its RFC, while it samples at 16 kHz).
    pub fn clock(&self) -> u32 {
        match self {
            Codec::Opus(_) => 48_000,
            _ => 8_000,
        }
    }

    /// One 20 ms frame of 48 kHz mono, encoded.
    pub fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
        match self {
            Codec::Opus(o) => o.encode(pcm),
            Codec::G722(g) => g.enc.encode(&down(pcm, 16_000)),
            Codec::Pcmu { .. } => down(pcm, 8_000).into_iter().map(ulaw).collect(),
            Codec::Pcma { .. } => down(pcm, 8_000).into_iter().map(alaw).collect(),
        }
    }

    /// A packet as 48 kHz mono.
    pub fn decode(&mut self, packet: &[u8]) -> Vec<i16> {
        match self {
            Codec::Opus(o) => o.decode(packet),
            Codec::G722(g) => {
                let pcm = g.dec.decode(packet);
                up(&pcm, 16_000, &mut g.last)
            }
            Codec::Pcmu { last } => up(&packet.iter().map(|&b| ulaw_decode(b)).collect::<Vec<_>>(), 8_000, last),
            Codec::Pcma { last } => up(&packet.iter().map(|&b| alaw_decode(b)).collect::<Vec<_>>(), 8_000, last),
        }
    }
}

/// The test melody: 200 ms notes with 50 ms of silence after each. Noise suppression keeps it, a steady tone it removes.
pub const MELODY: [f64; 6] = [440.0, 554.0, 659.0, 880.0, 659.0, 554.0];
const NOTE: u64 = 12_000;
const SOUND: u64 = 9_600;
/// The peer's test tone in an answered call: far from every melody note
pub const PEER_TONE: f64 = 1000.0;

/// One frame of the melody from sample `start` on (48 kHz mono).
pub fn melody_frame(start: u64, out: &mut [i16]) {
    for (i, o) in out.iter_mut().enumerate() {
        let n = start + i as u64;
        let hz = MELODY[((n / NOTE) % MELODY.len() as u64) as usize];
        *o = if n % NOTE < SOUND { (9000.0 * (2.0 * PI * hz * n as f64 / f64::from(RATE)).sin()).round() as i16 } else { 0 };
    }
}

/// Share of the signal's energy at one frequency, 0 to 1 (Goertzel).
pub fn goertzel_fraction(values: &[f64], rate: f64, hz: f64) -> f64 {
    let k = 2.0 * (2.0 * PI * hz / rate).cos();
    let (mut s1, mut s2, mut energy) = (0.0, 0.0, 0.0);
    for &v in values {
        let s = v + k * s1 - s2;
        s2 = s1;
        s1 = s;
        energy += v * v;
    }
    let power = s1 * s1 + s2 * s2 - k * s1 * s2;
    if energy > 0.0 { 2.0 * power / (energy * values.len() as f64) } else { 0.0 }
}

/// Per 100 ms of the signal: the melody note or the peer's tone holding over half the energy, 0 for none.
pub fn dominant_notes(values: &[f64], rate: f64) -> Vec<f64> {
    let step = (rate / 10.0).round() as usize;
    let mut candidates: Vec<f64> = MELODY.to_vec();
    candidates.push(PEER_TONE);
    candidates.dedup();
    values
        .chunks_exact(step)
        .map(|part| {
            let (best, hz) = candidates.iter().map(|&hz| (goertzel_fraction(part, rate, hz), hz)).fold((0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a });
            if best > 0.5 { hz } else { 0.0 }
        })
        .collect()
}

/// Whether a run of detected notes carries the melody: at least `need` note changes in its order.
pub fn melody_heard(notes: &[f64], need: usize) -> bool {
    let changes: Vec<f64> = notes.iter().enumerate().filter(|(i, n)| **n != 0.0 && (*i == 0 || notes[i - 1] != **n)).map(|(_, n)| *n).collect();
    let in_order = changes.windows(2).filter(|w| (0..MELODY.len()).any(|j| MELODY[j] == w[0] && MELODY[(j + 1) % MELODY.len()] == w[1])).count();
    in_order >= need
}

#[cfg(test)]
mod tests {
    use super::*;

    fn melody(seconds: u64) -> Vec<i16> {
        let mut out = vec![0i16; (seconds * u64::from(RATE)) as usize];
        melody_frame(0, &mut out);
        out
    }

    /// The melody through a codec and back is still heard as the melody.
    fn through(mut c: Codec) -> bool {
        let pcm = melody(3);
        let mut back = vec![];
        for f in pcm.as_chunks::<FRAME_20>().0 {
            let p = c.encode(f);
            assert!(!p.is_empty());
            back.extend(c.decode(&p));
        }
        let values: Vec<f64> = back.iter().map(|&v| f64::from(v)).collect();
        melody_heard(&dominant_notes(&values, f64::from(RATE)), 4)
    }

    #[test]
    fn the_melody_survives_every_codec() {
        let values: Vec<f64> = melody(3).iter().map(|&v| f64::from(v)).collect();
        assert!(melody_heard(&dominant_notes(&values, 48_000.0), 4));
        assert!(!melody_heard(&[0.0; 30], 1));
        for name in ["opus", "G722", "PCMU", "PCMA"] {
            assert!(through(Codec::of(name).expect(name)), "{name}");
        }
        assert!(Codec::of("ISAC").is_none());
        assert_eq!(Codec::of("G722").expect("g722").encode(&vec![0; FRAME_20]).len(), 160);
        assert_eq!(Codec::of("PCMU").expect("pcmu").encode(&vec![0; FRAME_20]).len(), 160);
    }

    #[test]
    fn g711_round_trips_within_its_step() {
        for s in [-32768i16, -1000, -1, 0, 1, 1000, 32767] {
            assert!((i32::from(ulaw_decode(ulaw(s))) - i32::from(s)).abs() <= (i32::from(s).abs() / 16).max(8), "ulaw {s}");
            assert!((i32::from(alaw_decode(alaw(s))) - i32::from(s)).abs() <= (i32::from(s).abs() / 16).max(16), "alaw {s}");
        }
    }
}
