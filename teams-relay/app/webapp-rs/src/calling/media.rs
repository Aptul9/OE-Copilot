//! The audio of a call over WebRTC (webrtc-rs): one audio stream, offered with Opus, G.722 and G.711, sent with the
//! codec the far end prefers among those it answered with, through the media relays of Teams. The microphone is pulled
//! every 20 ms, what comes back is decoded to 48 kHz mono for the speaker.

use super::codec::{Codec, FRAME_20};
use super::sdp;
use super::{Fail, R};
use async_trait::async_trait;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::{TrackLocal, TrackLocalContext, TrackLocalWriter};

fn webrtc_err(e: impl std::fmt::Display) -> Fail {
    Fail::Other(format!("webrtc: {e}"))
}

/// The codecs offered, in the order of the Teams web client: payload type, name, clock, channels, fmtp.
const OFFERED: [(u8, &str, u32, u16, &str); 4] =
    [(111, "opus", 48_000, 2, "minptime=10;useinbandfec=1"), (9, "G722", 8_000, 1, ""), (0, "PCMU", 8_000, 1, ""), (8, "PCMA", 8_000, 1, "")];

fn capability(name: &str, clock: u32, channels: u16, fmtp: &str) -> RTCRtpCodecCapability {
    RTCRtpCodecCapability { mime_type: format!("audio/{name}"), clock_rate: clock, channels, sdp_fmtp_line: fmtp.into(), rtcp_feedback: vec![] }
}

/// What the sender was bound to once negotiated: SSRC, payload type, codec name, where packets go.
struct Binding {
    ssrc: u32,
    payload_type: u8,
    name: String,
    write: Arc<dyn TrackLocalWriter + Send + Sync>,
}

/// An audio track that takes whichever of the offered codecs the far end prefers (the order of its answer).
#[derive(Default)]
struct AnyAudioTrack {
    preferred: std::sync::Mutex<Vec<u8>>,
    binding: std::sync::Mutex<Option<Arc<Binding>>>,
}

impl std::fmt::Debug for AnyAudioTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnyAudioTrack")
    }
}

#[async_trait]
impl TrackLocal for AnyAudioTrack {
    async fn bind(&self, t: &TrackLocalContext) -> webrtc::error::Result<RTCRtpCodecParameters> {
        let params = t.codec_parameters();
        let preferred = self.preferred.lock().map(|p| p.clone()).unwrap_or_default();
        let ours = |p: &&RTCRtpCodecParameters| Codec::of(p.capability.mime_type.trim_start_matches("audio/")).is_some();
        let pick = preferred.iter().find_map(|pt| params.iter().filter(ours).find(|p| p.payload_type == *pt)).or_else(|| params.iter().find(ours));
        let Some(codec) = pick.cloned() else { return Err(webrtc::Error::ErrUnsupportedCodec) };
        let binding = Binding {
            ssrc: t.ssrc(),
            payload_type: codec.payload_type,
            name: codec.capability.mime_type.trim_start_matches("audio/").to_string(),
            write: t.write_stream(),
        };
        if let Ok(mut b) = self.binding.lock() {
            *b = Some(Arc::new(binding));
        }
        Ok(codec)
    }

    async fn unbind(&self, _t: &TrackLocalContext) -> webrtc::error::Result<()> {
        if let Ok(mut b) = self.binding.lock() {
            *b = None;
        }
        Ok(())
    }

    fn id(&self) -> &str {
        "main-audio"
    }

    fn rid(&self) -> Option<&str> {
        None
    }

    fn stream_id(&self) -> &str {
        "teamsrelay"
    }

    fn kind(&self) -> RTPCodecType {
        RTPCodecType::Audio
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// 20 ms of the microphone, 48 kHz mono, asked for each frame sent.
pub type Microphone = Box<dyn FnMut() -> Vec<i16> + Send>;

/// Statistics of the audio of a call.
#[derive(Debug, Default)]
pub struct Counters {
    pub sent: AtomicU64,
    pub received: AtomicU64,
    pub decoded_samples: AtomicU64,
}

pub struct Media {
    pc: Arc<RTCPeerConnection>,
    track: Arc<AnyAudioTrack>,
    ssrc: u32,
    pub muted: Arc<AtomicBool>,
    pub counters: Arc<Counters>,
    state: Arc<std::sync::Mutex<RTCPeerConnectionState>>,
    speaker: mpsc::UnboundedSender<Vec<i16>>,
    names: Arc<Mutex<std::collections::HashMap<u8, String>>>,
}

impl Media {
    /// A peer connection through the relays of `ice` (TURN of Teams); what is decoded goes to `speaker`.
    pub async fn new(ice: Vec<RTCIceServer>, speaker: mpsc::UnboundedSender<Vec<i16>>) -> R<Media> {
        let mut m = MediaEngine::default();
        for (pt, name, clock, channels, fmtp) in OFFERED {
            let p = RTCRtpCodecParameters { capability: capability(name, clock, channels, fmtp), payload_type: pt, ..Default::default() };
            m.register_codec(p, RTPCodecType::Audio).map_err(webrtc_err)?;
        }
        let registry = register_default_interceptors(Registry::new(), &mut m).map_err(webrtc_err)?;
        let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(registry).build();
        let pc = Arc::new(api.new_peer_connection(RTCConfiguration { ice_servers: ice, ..Default::default() }).await.map_err(webrtc_err)?);
        let track = Arc::new(AnyAudioTrack::default());
        let init = RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Sendrecv, send_encodings: vec![] };
        let transceiver = pc.add_transceiver_from_track(track.clone() as Arc<dyn TrackLocal + Send + Sync>, Some(init)).await.map_err(webrtc_err)?;
        let ssrc = transceiver.sender().await.get_parameters().await.encodings.first().map(|e| e.ssrc).unwrap_or(0);
        if std::env::var("CALLING_DEBUG").is_ok() {
            let t0 = std::time::Instant::now();
            pc.on_ice_candidate(Box::new(move |c| {
                if let Some(c) = c {
                    eprintln!("{}", serde_json::json!({ "candidate": format!("{:?} {} {}:{}", c.typ, c.protocol, c.address, c.port), "ms": t0.elapsed().as_millis() as u64 }));
                }
                Box::pin(async {})
            }));
        }
        let state = Arc::new(std::sync::Mutex::new(RTCPeerConnectionState::New));
        let s = state.clone();
        pc.on_peer_connection_state_change(Box::new(move |st| {
            if let Ok(mut x) = s.lock() {
                *x = st;
            }
            Box::pin(async {})
        }));
        let counters = Arc::new(Counters::default());
        let names: Arc<Mutex<std::collections::HashMap<u8, String>>> = Arc::default();
        let (c, sp, nm) = (counters.clone(), speaker.clone(), names.clone());
        pc.on_track(Box::new(move |remote, _, _| {
            let (c, sp, nm) = (c.clone(), sp.clone(), nm.clone());
            Box::pin(async move {
                tokio::spawn(async move {
                    let mut codecs: std::collections::HashMap<u8, Codec> = std::collections::HashMap::new();
                    while let Ok((packet, _)) = remote.read_rtp().await {
                        c.received.fetch_add(1, Ordering::Relaxed);
                        let pt = packet.header.payload_type;
                        let codec = match codecs.entry(pt) {
                            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                            std::collections::hash_map::Entry::Vacant(v) => {
                                let name = nm.lock().await.get(&pt).cloned().unwrap_or_default();
                                match Codec::of(&name) {
                                    Some(c) => v.insert(c),
                                    None => continue,
                                }
                            }
                        };
                        let pcm = codec.decode(&packet.payload);
                        c.decoded_samples.fetch_add(pcm.len() as u64, Ordering::Relaxed);
                        if sp.send(pcm).is_err() {
                            break;
                        }
                    }
                });
            })
        }));
        Ok(Media { pc, track, ssrc, muted: Arc::default(), counters, state, speaker, names })
    }

    pub fn state(&self) -> RTCPeerConnectionState {
        self.state.lock().map(|s| *s).unwrap_or(RTCPeerConnectionState::Unspecified)
    }

    /// The offer in the dialect of Teams, once ICE has a relay candidate over UDP, the address Teams announces (or all
    /// its candidates; 5 s at most: TCP and IPv6 relays can take longer and are not needed).
    pub async fn offer(&self) -> R<String> {
        let offer = self.pc.create_offer(None).await.map_err(webrtc_err)?;
        let mut gathered = self.pc.gathering_complete_promise().await;
        self.pc.set_local_description(offer).await.map_err(webrtc_err)?;
        self.wait_relay(&mut gathered).await;
        let local = self.pc.local_description().await.ok_or_else(|| Fail::Other("no local description".into()))?;
        sdp::to_ngc(&local.sdp, &sdp::OfferExtras { ssrc: self.ssrc, cname: format!("teamsrelay{}", self.ssrc) })
            .ok_or_else(|| Fail::Other("offer without audio or candidates".into()))
    }

    /// The answer of the far end, in the dialect of Teams; the codec it prefers among ours is the one sent.
    pub async fn answer(&self, ngc: &str) -> R<()> {
        let answered = sdp::from_ngc(ngc).ok_or_else(|| Fail::Other("answer without audio".into()))?;
        self.remember_names(ngc).await;
        if let Ok(mut p) = self.track.preferred.lock() {
            *p = answered.payload_types.clone();
        }
        let desc = RTCSessionDescription::answer(answered.sdp).map_err(webrtc_err)?;
        self.pc.set_remote_description(desc).await.map_err(webrtc_err)
    }

    /// The offer of an incoming call (dialect of Teams) answered: this client's answer in the dialect, once ICE has a
    /// relay candidate. The codec sent is the one the caller prefers among ours.
    pub async fn accept_offer(&self, ngc: &str) -> R<String> {
        let offered = sdp::from_ngc(ngc).ok_or_else(|| Fail::Other("offer without audio".into()))?;
        self.remember_names(ngc).await;
        if let Ok(mut p) = self.track.preferred.lock() {
            *p = offered.payload_types.clone();
        }
        let desc = RTCSessionDescription::offer(offered.sdp).map_err(webrtc_err)?;
        self.pc.set_remote_description(desc).await.map_err(webrtc_err)?;
        let answer = self.pc.create_answer(None).await.map_err(webrtc_err)?;
        let mut gathered = self.pc.gathering_complete_promise().await;
        self.pc.set_local_description(answer).await.map_err(webrtc_err)?;
        self.wait_relay(&mut gathered).await;
        let local = self.pc.local_description().await.ok_or_else(|| Fail::Other("no local description".into()))?;
        sdp::to_ngc(&local.sdp, &sdp::OfferExtras { ssrc: self.ssrc, cname: format!("teamsrelay{}", self.ssrc) })
            .ok_or_else(|| Fail::Other("answer without audio or candidates".into()))
    }

    async fn remember_names(&self, ngc: &str) {
        let mut names = self.names.lock().await;
        for l in ngc.lines() {
            if let Some(map) = l.trim().strip_prefix("a=rtpmap:") {
                let mut f = map.split_whitespace();
                if let (Some(pt), Some(codec)) = (f.next().and_then(|p| p.parse::<u8>().ok()), f.next()) {
                    names.insert(pt, codec.split('/').next().unwrap_or("").to_string());
                }
            }
        }
    }

    /// Until ICE has the address Teams announces: a relay candidate over UDP, else a reflexive one and 300 ms more for a
    /// relay, else all it gathered; 5 s at most. The TURN allocation of webrtc-rs with the relays of Teams has not
    /// produced a relay candidate on oe-vm (2026-11-15), its reflexive address carries the calls: the gathering waited
    /// for its full 5 s there before every call.
    async fn wait_relay(&self, gathered: &mut tokio::sync::mpsc::Receiver<()>) {
        let started = tokio::time::Instant::now();
        let end = started + Duration::from_secs(5);
        let relay = regex::Regex::new(r"(?m)^a=candidate:\S+ \d+ udp \d+ \d+\.\d+\.\d+\.\d+ \d+ typ relay").expect("fixed");
        let reflexive = regex::Regex::new(r"(?m)^a=candidate:\S+ \d+ udp \d+ \d+\.\d+\.\d+\.\d+ \d+ typ srflx").expect("fixed");
        let mut why = "timeout";
        let mut grace: Option<tokio::time::Instant> = None;
        while tokio::time::Instant::now() < end {
            if tokio::time::timeout(Duration::from_millis(50), gathered.recv()).await.is_ok() {
                why = "gathered";
                break;
            }
            let sdp = self.pc.local_description().await.map(|d| d.sdp).unwrap_or_default();
            if relay.is_match(&sdp) {
                why = "relay";
                break;
            }
            if reflexive.is_match(&sdp) {
                let until = *grace.get_or_insert_with(|| tokio::time::Instant::now() + Duration::from_millis(300));
                if tokio::time::Instant::now() >= until {
                    why = "reflexive";
                    break;
                }
            }
        }
        if std::env::var("CALLING_DEBUG").is_ok() {
            let sdp = self.pc.local_description().await.map(|d| d.sdp).unwrap_or_default();
            let candidates: Vec<&str> = sdp.lines().filter(|l| l.starts_with("a=candidate")).collect();
            eprintln!("{}", serde_json::json!({ "ice": why, "ms": started.elapsed().as_millis() as u64, "candidates": candidates }));
        }
    }

    /// The codec the stream sends with, once bound.
    pub fn sending(&self) -> Option<String> {
        self.track.binding.lock().ok().and_then(|b| b.as_ref().map(|b| b.name.clone()))
    }

    /// Sends the microphone every 20 ms until the peer connection closes: silence while muted.
    pub fn start_sending(&self, mut mic: Microphone) {
        let (track, muted, counters, state) = (self.track.clone(), self.muted.clone(), self.counters.clone(), self.state.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
            let (mut seq, mut ts) = (rand_u16(), rand_u32());
            let mut codec: Option<(u8, Codec)> = None;
            let mut first = true;
            loop {
                tick.tick().await;
                let st = state.lock().map(|s| *s).unwrap_or(RTCPeerConnectionState::Closed);
                if matches!(st, RTCPeerConnectionState::Closed | RTCPeerConnectionState::Failed) {
                    break;
                }
                let mut pcm = mic();
                pcm.resize(FRAME_20, 0);
                let binding = track.binding.lock().ok().and_then(|b| b.clone());
                let Some(b) = binding else { continue };
                if codec.as_ref().is_none_or(|(pt, _)| *pt != b.payload_type) {
                    codec = Codec::of(&b.name).map(|c| (b.payload_type, c));
                }
                let Some((_, c)) = codec.as_mut() else { continue };
                if muted.load(Ordering::Relaxed) {
                    pcm.fill(0);
                }
                let payload = c.encode(&pcm);
                let packet = webrtc::rtp::packet::Packet {
                    header: webrtc::rtp::header::Header {
                        version: 2,
                        marker: first,
                        payload_type: b.payload_type,
                        sequence_number: seq,
                        timestamp: ts,
                        ssrc: b.ssrc,
                        ..Default::default()
                    },
                    payload: payload.into(),
                };
                first = false;
                seq = seq.wrapping_add(1);
                ts = ts.wrapping_add(c.clock() / 50);
                if b.write.write_rtp(&packet).await.is_ok() {
                    counters.sent.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }

    pub async fn close(&self) {
        let _ = self.pc.close().await;
        let _ = &self.speaker;
    }
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_le_bytes(b)
}

fn rand_u16() -> u16 {
    rand_u32() as u16
}

/// The ICE servers of Teams: its TURN relay (fqdn of the ECS settings, UDP and TCP ports) with the credentials of
/// the relay token service.
pub fn relay_servers(fqdn: &str, udp: u16, tcp: u16, username: &str, credential: &str) -> Vec<RTCIceServer> {
    vec![RTCIceServer {
        urls: vec![format!("turn:{fqdn}:{udp}?transport=udp"), format!("turn:{fqdn}:{tcp}?transport=tcp")],
        username: username.into(),
        credential: credential.into(),
    }]
}
