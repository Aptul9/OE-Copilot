//! The call audio of the app (calling-audio-bridge.mjs): its microphone comes over the websocket of the web app's audio
//! hub as Opus, a jittery link that delivers late and in bursts. It plays once 60 ms are queued (again after running
//! dry), and the burst after a stall stays queued as the cushion for the next one. Quiet frames are dropped to win the
//! delay back only above a target that grows 60 ms with each underrun (100 ms to start, at most 400 ms) and eases 20 ms
//! after 20 s without one (at least 60 ms); 500 ms over it, the oldest audio goes. An underrun fades the last frame out.
//! What the call says goes back to the app as 20 ms Opus frames.

use super::codec::{FRAME_10, FRAME_20, OpusCodec};
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const fn ms(n: usize) -> usize {
    n * 48
}
const START: usize = ms(60);
const TARGET: usize = ms(100);
const MIN: usize = ms(60);
const MAX: usize = ms(400);
const UP: usize = ms(60);
const DOWN: usize = ms(20);
const CALM_FRAMES: u32 = 2000;
const MARGIN: usize = ms(60);
const EXCESS: usize = ms(500);
const QUIET: i32 = 650;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub down: u64,
    pub up: u64,
    pub invalid: u64,
    pub dropped_samples: u64,
    pub quiet_samples: u64,
    pub underruns: u64,
    pub max_target: usize,
}

/// The jitter buffer of the app's microphone and the encoder of what goes back to it.
pub struct Bridge {
    codec: OpusCodec,
    ring: Vec<i16>,
    head: usize,
    queued: usize,
    pub target: usize,
    playing: bool,
    calm: u32,
    resumed: bool,
    last: [i16; FRAME_10],
    down: Vec<i16>,
    pub stats: Stats,
    closed: bool,
}

impl Bridge {
    pub fn new() -> Result<Bridge, String> {
        Ok(Bridge {
            codec: OpusCodec::new()?,
            ring: vec![0; MAX + EXCESS + ms(100)],
            head: 0,
            queued: 0,
            target: TARGET,
            playing: false,
            calm: 0,
            resumed: false,
            last: [0; FRAME_10],
            down: vec![],
            stats: Stats { max_target: TARGET, ..Stats::default() },
            closed: false,
        })
    }

    /// A frame of the app: kind 2 then Opus.
    pub fn receive(&mut self, frame: &[u8]) {
        if self.closed {
            return;
        }
        if frame.len() < 2 || frame.len() > 3829 || frame[0] != 2 {
            self.stats.invalid += 1;
            return;
        }
        let pcm = self.codec.decode(&frame[1..]);
        if pcm.is_empty() {
            self.stats.invalid += 1;
            return;
        }
        let n = pcm.len();
        let len = self.ring.len();
        if self.queued + n > len {
            self.discard(self.queued + n - len);
        }
        for (i, v) in pcm.iter().enumerate() {
            self.ring[(self.head + self.queued + i) % len] = *v;
        }
        self.queued += n;
        if self.queued > self.target + EXCESS {
            self.discard(self.queued - self.target);
        }
        self.stats.up += 1;
    }

    /// 10 ms of the app's microphone for the call.
    pub fn microphone(&mut self) -> [i16; FRAME_10] {
        let mut out = [0i16; FRAME_10];
        if self.closed {
            return out;
        }
        if !self.playing {
            if self.queued < START {
                return self.conceal(out);
            }
            self.playing = true;
            self.resumed = true;
        }
        if self.queued < FRAME_10 {
            self.playing = false;
            self.calm = 0;
            self.stats.underruns += 1;
            self.target = MAX.min(self.target + UP);
            self.stats.max_target = self.stats.max_target.max(self.target);
            return self.conceal(out);
        }
        while self.queued - FRAME_10 > self.target + MARGIN && self.peak() < QUIET {
            self.skip(FRAME_10);
            self.stats.quiet_samples += FRAME_10 as u64;
        }
        let len = self.ring.len();
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.ring[(self.head + i) % len];
        }
        self.skip(FRAME_10);
        if self.resumed {
            for (i, o) in out.iter_mut().enumerate() {
                *o = ((i32::from(*o) * i as i32) as f64 / FRAME_10 as f64).round() as i16;
            }
            self.resumed = false;
        }
        self.calm += 1;
        if self.calm >= CALM_FRAMES {
            self.calm = 0;
            self.target = MIN.max(self.target - DOWN);
        }
        self.last = out;
        out
    }

    /// The frame before an underrun once more, fading to silence; silence after it.
    fn conceal(&mut self, mut out: [i16; FRAME_10]) -> [i16; FRAME_10] {
        for (i, o) in out.iter_mut().enumerate() {
            *o = ((i32::from(self.last[i]) * (FRAME_10 - i) as i32) as f64 / FRAME_10 as f64).round() as i16;
        }
        self.last = [0; FRAME_10];
        out
    }

    fn peak(&self) -> i32 {
        let len = self.ring.len();
        (0..FRAME_10).map(|i| i32::from(self.ring[(self.head + i) % len]).abs()).max().unwrap_or(0)
    }

    fn skip(&mut self, n: usize) {
        self.head = (self.head + n) % self.ring.len();
        self.queued -= n;
    }

    fn discard(&mut self, n: usize) {
        self.skip(n);
        self.stats.dropped_samples += n as u64;
    }

    /// What the call says (48 kHz mono): the 20 ms Opus frames ready for the app.
    pub fn speaker(&mut self, pcm: &[i16]) -> Vec<Vec<u8>> {
        if self.closed {
            return vec![];
        }
        self.down.extend_from_slice(pcm);
        let mut out = vec![];
        while self.down.len() >= FRAME_20 {
            let frame: Vec<i16> = self.down.drain(..FRAME_20).collect();
            out.push(self.codec.encode(&frame));
            self.stats.down += 1;
        }
        out
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.queued = 0;
        self.down.clear();
    }
}

/// The audio hub of the web app for one call: the bridge, fed by the hub's websocket.
pub struct Hub {
    pub bridge: Arc<Mutex<Bridge>>,
    out: mpsc::UnboundedSender<Message>,
}

impl Hub {
    /// Connects to CALL_AUDIO_URL with the bearer of CALL_AUDIO_TOKEN_FILE; the app's microphone starts flowing.
    pub async fn connect(url: &str, token: Option<&str>) -> Result<Hub, String> {
        let u = url::Url::parse(url).map_err(|e| e.to_string())?;
        let internal = std::env::var("CALL_AUDIO_INTERNAL").is_ok_and(|v| v == "1") && u.host_str() == Some("teams-webapp");
        let loopback = matches!(u.host_str(), Some("127.0.0.1") | Some("localhost"));
        if !(u.scheme() == "wss" || (u.scheme() == "ws" && (internal || loopback))) || !u.username().is_empty() || u.path() != "/api/call/audio/socket" {
            return Err("invalid audio hub URL".into());
        }
        let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url).map_err(|e| e.to_string())?;
        if let Some(t) = token {
            req.headers_mut().insert("Authorization", format!("Bearer {t}").parse().map_err(|_| "token")?);
        }
        let (socket, _) = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(req))
            .await
            .map_err(|_| "audio hub timed out".to_string())?
            .map_err(|e| e.to_string())?;
        let (mut write, mut read) = socket.split();
        let bridge = Arc::new(Mutex::new(Bridge::new()?));
        let (out, mut queue) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(m) = queue.recv().await {
                if write.send(m).await.is_err() {
                    break;
                }
            }
            let _ = write.close().await;
        });
        let b = bridge.clone();
        tokio::spawn(async move {
            while let Some(Ok(m)) = read.next().await {
                if let Message::Binary(data) = m
                    && let Ok(mut br) = b.lock()
                {
                    br.receive(&data);
                }
            }
            if let Ok(mut br) = b.lock() {
                br.close();
            }
        });
        let _ = out.send(Message::Text("CHANNELS 1".into()));
        let _ = out.send(Message::Text("CAPTURE_DEMAND microphone 1".into()));
        Ok(Hub { bridge, out })
    }

    /// 20 ms of the app's microphone.
    pub fn microphone(&self) -> Vec<i16> {
        let Ok(mut b) = self.bridge.lock() else { return vec![0; FRAME_20] };
        let mut out = b.microphone().to_vec();
        out.extend_from_slice(&b.microphone());
        out
    }

    /// What the call says, to the app.
    pub fn speaker(&self, pcm: &[i16]) {
        let frames = self.bridge.lock().map(|mut b| b.speaker(pcm)).unwrap_or_default();
        for f in frames {
            let _ = self.out.send(Message::Binary(f.into()));
        }
    }

    pub fn stats(&self) -> Stats {
        self.bridge.lock().map(|b| b.stats.clone()).unwrap_or_default()
    }

    pub fn close(&self) {
        let _ = self.out.send(Message::Text("CAPTURE_DEMAND microphone 0".into()));
        let _ = self.out.send(Message::Close(None));
        if let Ok(mut b) = self.bridge.lock() {
            b.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(codec: &mut OpusCodec, level: i16) -> Vec<u8> {
        let pcm: Vec<i16> = (0..FRAME_20).map(|i| if i % 2 == 0 { level } else { -level }).collect();
        let mut p = vec![2u8];
        p.extend(codec.encode(&pcm));
        p
    }

    #[test]
    fn plays_once_60_ms_are_queued_and_grows_its_target_on_an_underrun() {
        let mut b = Bridge::new().expect("bridge");
        let mut enc = OpusCodec::new().expect("opus");
        b.receive(&[1, 2, 3]);
        assert_eq!(b.stats.invalid, 1);
        b.receive(&packet(&mut enc, 8000));
        b.receive(&packet(&mut enc, 8000));
        assert_eq!(b.microphone(), [0; FRAME_10], "40 ms queued: not yet");
        b.receive(&packet(&mut enc, 8000));
        let first = b.microphone();
        assert!(first[FRAME_10 - 1] != 0 && first[0] == 0, "fades in");
        // the 50 ms left go out in 5 frames, the sixth finds the buffer dry
        for _ in 0..6 {
            b.microphone();
        }
        assert_eq!(b.stats.underruns, 1);
        assert_eq!(b.target, TARGET + UP);
        let frames = b.speaker(&vec![100; FRAME_20 + 10]);
        assert_eq!((frames.len(), b.stats.down), (1, 1));
        b.close();
        assert!(b.speaker(&vec![0; FRAME_20]).is_empty());
    }
}
