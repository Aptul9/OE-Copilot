//! The calling worker of one light account (mode serve of calling-live-host.mjs), on the IPC channel the light host
//! forks it with (agent/calls.rs): registered for its calls, it reports the call of the moment every 500 ms
//! ({call: {caller, since, seen, ringing, active, ended, answered, muted}}), runs the commands of the app ({id, op:
//! call | answer | hangup | mute}, answered {command: id, ok, error}), asks for a fresh Skype token ({refreshToken})
//! when a call outlives it, and recycles itself after 45 idle minutes. The audio goes to the web app's audio hub.

use super::bridge::Hub;
use super::codec::FRAME_20;
use super::media::{Media, relay_servers};
use super::signaling::{Account, Attached, CallLinks, Conversation, Endpoint, Event, Incoming, attach, event_of, relay, uuid};
use super::trouter::Request;
use super::{Fail, R};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Idle this long, the worker exits and the light host starts a fresh one (a fresh token, a fresh registration)
const RECYCLE: Duration = Duration::from_secs(45 * 60);
/// Between two keepalives of a call: the service drops one after 2700 s without
const KEEP_ALIVE: Duration = Duration::from_secs(35 * 60);
/// A call that rings this long without an answer is left to the other devices of the account
const RING_FOR: Duration = Duration::from_secs(60);

fn other(e: impl std::fmt::Display) -> Fail {
    Fail::Other(e.to_string())
}

/// A message to the light host (process.send of the Node worker).
type Out = crate::agent::ipc::Sender;

/// What a call is now, by the numbers of the Teams stack the app knows: 1 ringing, 2 connecting, 3 connected, 7 ended.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Ringing,
    Connecting,
    Connected,
    Ended,
}

enum Side {
    Incoming { inc: Box<Incoming>, attached: Box<Attached> },
    Outgoing { conv: Conversation },
}

struct Call {
    caller: String,
    since: i64,
    state: State,
    answered: bool,
    muted: bool,
    side: Side,
    links: CallLinks,
    media: Option<Arc<Media>>,
    hub: Option<Arc<Hub>>,
    call_leg: Option<String>,
    started: Instant,
    kept: Instant,
}

impl Call {
    fn conversation(&self) -> &Conversation {
        match &self.side {
            Side::Incoming { attached, .. } => &attached.conversation,
            Side::Outgoing { conv } => conv,
        }
    }

    fn report(&self) -> Value {
        json!({ "call": {
            "caller": self.caller,
            "since": self.since,
            "seen": crate::agent::now_ms(),
            "ringing": self.state == State::Ringing,
            "active": matches!(self.state, State::Connecting | State::Connected),
            "ended": self.state == State::Ended,
            "answered": self.answered,
            "muted": self.muted,
        }})
    }

    fn audio_summary(&self) -> Option<Value> {
        let s = self.hub.as_ref()?.stats();
        let hub = self.hub.as_ref()?;
        let target = hub.bridge.lock().map(|b| b.target).unwrap_or(0);
        Some(json!({ "callAudio": {
            "upPackets": s.up, "downPackets": s.down, "underruns": s.underruns, "targetMs": target / 48, "maxTargetMs": s.max_target / 48,
            "quietDroppedMs": s.quiet_samples / 48, "droppedMs": s.dropped_samples / 48, "micSkippedMs": 0, "loopP99Ms": 0, "loopMaxMs": 0,
        }}))
    }
}

struct Worker {
    e: Endpoint,
    out: Out,
    state_dir: PathBuf,
    call: Option<Call>,
    idle_since: Instant,
    token_expiry: Instant,
    /// commands that came while a fresh token was awaited: run right after
    pending: Vec<Map<String, Value>>,
}

fn discovery_of(dir: &Path) -> R<(Value, Instant)> {
    let file = dir.join("discovery.json");
    let meta = std::fs::metadata(&file).map_err(other)?;
    let v: Value = serde_json::from_slice(&std::fs::read(&file).map_err(other)?).map_err(other)?;
    let age = meta.modified().ok().and_then(|m| m.elapsed().ok()).unwrap_or_default();
    let life = Duration::from_secs(v["tokens"]["expiresIn"].as_u64().unwrap_or(3600));
    Ok((v, Instant::now() + life.saturating_sub(age)))
}

impl Worker {
    fn send(&self, v: Value) {
        self.out.send(&v);
    }

    fn account_now(&self) -> Account {
        let mut a = self.e.account.clone();
        a.token = self.e.token();
        a
    }

    /// The Skype token, renewed through the light host once it ends within 5 minutes.
    async fn fresh_token(&mut self, inbox: &mut mpsc::UnboundedReceiver<Value>) {
        if Instant::now() + Duration::from_secs(300) < self.token_expiry {
            return;
        }
        self.send(json!({ "refreshToken": true }));
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            match tokio::time::timeout(end - Instant::now(), inbox.recv()).await {
                Ok(Some(m)) if m.get("tokenUpdated").is_some() => {
                    if let Ok((d, expiry)) = discovery_of(&self.state_dir)
                        && let Some(t) = d["tokens"]["skypeToken"].as_str()
                    {
                        self.e.set_token(t);
                        self.token_expiry = expiry;
                    }
                    return;
                }
                Ok(Some(Value::Object(m))) if m.contains_key("op") => self.pending.push(m),
                Ok(Some(_)) => {}
                _ => return,
            }
        }
    }

    /// A media session through the relay of the tenant; the speaker goes to the hub of the call.
    async fn media(&self, hub: Option<Arc<Hub>>) -> R<Arc<Media>> {
        let r = relay(&self.e.http, &self.account_now()).await?;
        let (speaker, mut heard) = mpsc::unbounded_channel::<Vec<i16>>();
        let media = Arc::new(Media::new(relay_servers(&r.fqdn, r.udp, r.tcp, &r.username, &r.credential), speaker).await?);
        tokio::spawn(async move {
            while let Some(pcm) = heard.recv().await {
                if let Some(h) = &hub {
                    h.speaker(&pcm);
                }
            }
        });
        Ok(media)
    }

    async fn hub() -> Option<Arc<Hub>> {
        let url = std::env::var("CALL_AUDIO_URL").ok().filter(|u| !u.is_empty())?;
        let token = std::env::var("CALL_AUDIO_TOKEN_FILE").ok().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| t.trim().to_string());
        match Hub::connect(&url, token.as_deref()).await {
            Ok(h) => Some(Arc::new(h)),
            Err(e) => {
                eprintln!("calling: audio hub: {e}");
                None
            }
        }
    }

    fn start_audio(media: &Media, hub: Option<Arc<Hub>>) {
        media.start_sending(Box::new(move || match &hub {
            Some(h) => h.microphone(),
            None => vec![0; FRAME_20],
        }));
    }

    /// A request of the service on Trouter: answered at once, then acted on.
    async fn on_request(&mut self, req: Request) {
        let body = req.json();
        if let Some(inc) = Incoming::of(&body) {
            self.e.trouter.answer(&req, 200, "");
            if self.call.is_none() {
                self.ring(inc).await;
            }
            return;
        }
        let ev = event_of(&req);
        let ack = if matches!(ev, Event::Accepted { .. }) { CallLinks::acknowledgement(&self.e).to_string() } else { String::new() };
        self.e.trouter.answer(&req, 200, &ack);
        let Some(call) = self.call.as_mut() else { return };
        match ev {
            Event::Accepted { answer, .. } => {
                if let (Side::Outgoing { .. }, Some(media)) = (&call.side, call.media.clone()) {
                    match media.answer(&answer).await {
                        Ok(()) => {
                            call.state = State::Connected;
                            call.answered = true;
                            call.call_leg = req.json()["callAcceptance"]["links"]["callLeg"].as_str().map(String::from);
                        }
                        Err(e) => {
                            eprintln!("calling: answer of the far end: {e}");
                            call.state = State::Ended;
                        }
                    }
                }
            }
            Event::Ended(_) => call.state = State::Ended,
            _ => {}
        }
    }

    /// An incoming call: attached, ringing, reported until answered, declined, or gone.
    async fn ring(&mut self, inc: Incoming) {
        let links = CallLinks::new(&self.e);
        let attached = match attach(&self.e, &inc, &links).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("calling: attach: {e}");
                return;
            }
        };
        if let Err(e) = attached.ringing(&self.e).await {
            eprintln!("calling: ringing: {e}");
        }
        let caller = if inc.caller.is_empty() { "Incoming call".to_string() } else { inc.caller.clone() };
        self.call = Some(Call {
            caller,
            since: crate::agent::now_ms(),
            state: State::Ringing,
            answered: false,
            muted: false,
            side: Side::Incoming { inc: Box::new(inc), attached: Box::new(attached) },
            links,
            media: None,
            hub: None,
            call_leg: None,
            started: Instant::now(),
            kept: Instant::now(),
        });
    }

    async fn command(&mut self, m: &Map<String, Value>) -> R<()> {
        let op = m.get("op").and_then(Value::as_str).unwrap_or("");
        match op {
            "call" => {
                if self.call.is_some() {
                    return Err(other("call already active"));
                }
                let peer = m.get("peer").and_then(Value::as_str).unwrap_or("");
                if !regex::Regex::new(r"(?i)^8:orgid:[a-f0-9-]{36}$").expect("fixed").is_match(peer) {
                    return Err(other("invalid peer"));
                }
                let hub = Self::hub().await;
                let media = self.media(hub.clone()).await?;
                let offer = media.offer().await?;
                let links = CallLinks::new(&self.e);
                let conv = Conversation::create(&self.e, &links, &[json!({ "id": peer, "participantId": uuid() })], &offer).await?;
                Self::start_audio(&media, hub.clone());
                let name = m.get("name").and_then(Value::as_str).unwrap_or(peer).to_string();
                self.call = Some(Call {
                    caller: name,
                    since: crate::agent::now_ms(),
                    state: State::Connecting,
                    answered: false,
                    muted: false,
                    side: Side::Outgoing { conv },
                    links,
                    media: Some(media),
                    hub,
                    call_leg: None,
                    started: Instant::now(),
                    kept: Instant::now(),
                });
                Ok(())
            }
            "answer" => {
                let Some(call) = self.call.as_ref() else { return Err(other("no current call")) };
                if m.get("since").and_then(Value::as_i64) != Some(call.since) {
                    return Err(other("stale call"));
                }
                let Side::Incoming { inc, .. } = &call.side else { return Err(other("not an incoming call")) };
                if call.state != State::Ringing {
                    return Err(other("call not ringing"));
                }
                let (offer, leg) = (inc.offer.clone(), inc.media_leg.clone());
                let hub = Self::hub().await;
                let media = self.media(hub.clone()).await?;
                let answer = media.accept_offer(&offer).await?;
                let call = self.call.as_mut().ok_or_else(|| other("call gone"))?;
                let Side::Incoming { attached, .. } = &call.side else { return Err(other("not an incoming call")) };
                let links = attached.accept(&self.e, &answer, &leg).await?;
                Self::start_audio(&media, hub.clone());
                call.call_leg = links.get("callLeg").and_then(Value::as_str).map(String::from);
                call.media = Some(media);
                call.hub = hub;
                call.state = State::Connected;
                call.answered = true;
                Ok(())
            }
            "hangup" => {
                let Some(call) = self.call.as_mut() else { return Err(other("no current call")) };
                match (&call.side, call.state) {
                    (Side::Incoming { attached, .. }, State::Ringing) => attached.decline(&self.e).await?,
                    _ => call.conversation().leave(&self.e).await?,
                }
                call.state = State::Ended;
                Ok(())
            }
            "mute" => {
                let on = m.get("on").and_then(Value::as_bool).ok_or_else(|| other("mute: on"))?;
                let Some(call) = self.call.as_mut() else { return Err(other("no current call")) };
                if call.state != State::Connected {
                    return Err(other("call not connected"));
                }
                if call.muted != on {
                    call.conversation().set_muted(&self.e, on).await?;
                    if let Some(media) = &call.media {
                        media.muted.store(on, Ordering::Relaxed);
                    }
                    call.muted = on;
                }
                Ok(())
            }
            _ => Err(other("unknown command")),
        }
    }

    /// A command of the app, answered {command: id, ok, error}.
    async fn run(&mut self, m: &Map<String, Value>) {
        let id = m.get("id").cloned().unwrap_or(Value::Null);
        match self.command(m).await {
            Ok(()) => self.send(json!({ "command": id, "ok": true })),
            Err(err) => self.send(json!({ "command": id, "ok": false, "error": crate::agent::text_of(&err) })),
        }
    }

    /// Every 500 ms: the call reported, ended when its media failed or it rang too long, kept alive, cleared once ended.
    async fn tick(&mut self) {
        let Some(call) = self.call.as_mut() else { return };
        if call.state == State::Ringing && call.started.elapsed() > RING_FOR {
            call.state = State::Ended;
        }
        if let Some(media) = &call.media {
            use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState as S;
            if matches!(media.state(), S::Failed | S::Closed) && call.state != State::Ringing {
                call.state = State::Ended;
            }
        }
        if call.state == State::Connected && call.kept.elapsed() > KEEP_ALIVE {
            call.kept = Instant::now();
            call.conversation().keep_alive(&self.e, call.call_leg.as_deref(), &call.links).await;
        }
        let report = call.report();
        self.out.send(&report);
        if call.state == State::Ended {
            if let Some(a) = call.audio_summary() {
                self.out.send(&a);
            }
            if let Some(h) = &call.hub {
                h.close();
            }
            if let Some(m) = &call.media {
                m.close().await;
            }
            self.call = None;
            self.idle_since = Instant::now();
        }
    }

    /// The end of the worker: the call left, the registration removed with its marker.
    async fn stop(&mut self) {
        if let Some(call) = self.call.take() {
            if call.state != State::Ended {
                let _ = match (&call.side, call.state) {
                    (Side::Incoming { attached, .. }, State::Ringing) => attached.decline(&self.e).await,
                    _ => call.conversation().leave(&self.e).await,
                };
            }
            if let Some(m) = &call.media {
                m.close().await;
            }
        }
        self.e.stop().await;
        let _ = std::fs::remove_file(self.state_dir.join("owned-registration.json"));
    }
}

/// The worker until SIGTERM, the end of its channel or 45 idle minutes. Exit code 0 then; 1 when it could not start.
pub async fn serve() -> i32 {
    let state_dir = PathBuf::from(std::env::var("HOST_STATE").unwrap_or_else(|_| "/state".into()));
    let Some(stream) = crate::agent::ipc::from_env() else {
        eprintln!("calling: no IPC channel (NODE_CHANNEL_FD)");
        return 1;
    };
    let (out, mut inbox) = crate::agent::ipc::open(stream);
    let started = async {
        let (discovery, token_expiry) = discovery_of(&state_dir)?;
        let account = Account::of(&discovery, &std::env::var("CALLING_NAME").unwrap_or_else(|_| "TeamsRelay".into()))?;
        let (e, requests) = Endpoint::start(account).await?;
        // the light host removes a registration a dead worker left behind (owned_registration of agent/calls.rs)
        let marker = json!({ "endpoint": e.registration, "registrar": e.registrar() });
        crate::agent::write_private(&state_dir.join("owned-registration.json"), marker.to_string().as_bytes()).map_err(other)?;
        Ok::<_, Fail>((e, requests, token_expiry))
    };
    let (e, mut requests, token_expiry) = match started.await {
        Ok(s) => s,
        Err(err) => {
            out.send(&json!({ "fatal": crate::agent::text_of(&err) }));
            return 1;
        }
    };
    let mut w = Worker { e, out, state_dir, call: None, idle_since: Instant::now(), token_expiry, pending: vec![] };
    w.send(json!({ "stage": "worker-ready" }));
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    loop {
        // the token renewed ahead of its end, idle or in a call: a call that rings needs it at once (attach, progress,
        // accept), and the lifetime of a Skype token follows the access token it came from (1260 s seen, 2026-11-15)
        if Instant::now() + Duration::from_secs(300) >= w.token_expiry {
            w.fresh_token(&mut inbox).await;
            if Instant::now() >= w.token_expiry {
                eprintln!("calling: no fresh Skype token, worker ended");
                break;
            }
        }
        for m in std::mem::take(&mut w.pending) {
            w.run(&m).await;
        }
        tokio::select! {
            req = requests.recv() => match req {
                Some(r) => w.on_request(r).await,
                None => {
                    eprintln!("calling: Trouter connection lost");
                    break;
                }
            },
            msg = inbox.recv() => match msg {
                Some(Value::Object(m)) if m.contains_key("op") => w.run(&m).await,
                Some(_) => {}
                None => break,
            },
            _ = tick.tick() => {
                w.tick().await;
                if w.call.is_none() && w.idle_since.elapsed() > RECYCLE {
                    break;
                }
            }
            _ = async { match term.as_mut() { Some(t) => { t.recv().await; } None => std::future::pending::<()>().await } } => break,
        }
    }
    w.stop().await;
    0
}
