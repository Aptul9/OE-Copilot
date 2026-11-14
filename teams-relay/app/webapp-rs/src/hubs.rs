//! The websocket of the sound of a call answered or placed from the app on an account on another computer, or of a
//! light account (src/server/call-audio-hub.ts): the relay of the account and the page of the app each open it; Opus
//! goes down to the page in 0x01 frames, the microphone up as 0x02 frames while the relay asks for it. Who may open
//! which side is GET /api/call/audio's to say. One call per account: a newer socket of a side replaces the older one.

use crate::web::{Req, Resp};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Empty};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{Message, Role, WebSocketConfig};

pub const CALL_AUDIO_PATH: &str = "/api/call/audio/socket";
const RELAY_WAIT: Duration = Duration::from_secs(20);
const MAX_PAYLOAD: usize = 64 * 1024;

struct Side {
    id: u64,
    tx: mpsc::UnboundedSender<Message>,
}

#[derive(Default)]
struct Pair {
    app: Option<Side>,
    relay: Option<Side>,
    demand: bool,
    channels: u8,
    wait: Option<tokio::task::JoinHandle<()>>,
}

thread_local! {
    static PAIRS: RefCell<HashMap<i64, Pair>> = RefCell::new(HashMap::new());
    static IDS: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn settings(channels: u8) -> String {
    format!(r#"{{"type":"server_settings","settings":{{"audio_channels":{{"value":{channels}}}}}}}"#)
}

fn with_pair<T>(slot: i64, f: impl FnOnce(&mut Pair) -> T) -> T {
    PAIRS.with(|p| {
        let mut p = p.borrow_mut();
        let pair = p.entry(slot).or_insert_with(|| Pair { channels: 1, ..Default::default() });
        f(pair)
    })
}

fn to_app(p: &Pair, m: Message) {
    if let Some(a) = &p.app {
        let _ = a.tx.send(m);
    }
}

fn settle(slot: i64) {
    PAIRS.with(|p| {
        let mut p = p.borrow_mut();
        if p.get(&slot).is_some_and(|x| x.app.is_none() && x.relay.is_none())
            && let Some(w) = p.remove(&slot).and_then(|x| x.wait)
        {
            w.abort();
        }
    });
}

/// The page waits for the relay; with none by then it says so, and gives the sound in the app up.
fn wait_for_relay(slot: i64, p: &mut Pair) {
    if p.wait.is_some() || p.app.is_none() {
        return;
    }
    p.wait = Some(tokio::task::spawn_local(async move {
        tokio::time::sleep(RELAY_WAIT).await;
        with_pair(slot, |p| {
            p.wait = None;
            if p.relay.is_some() {
                return;
            }
            if let Some(a) = &p.app {
                let _ = a.tx.send(Message::Text("UNAVAILABLE The relay of the account sends no sound".into()));
                let _ = a.tx.send(Message::Close(None));
            }
        });
    }));
}

fn clear_wait(p: &mut Pair) {
    if let Some(w) = p.wait.take() {
        w.abort();
    }
}

/// The 101 answer and the socket, for an upgrade request the check let in.
async fn accept(
    parts: &hyper::http::request::Parts,
    upgrade: hyper::upgrade::OnUpgrade,
    join: impl FnOnce(WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>) + 'static,
) -> Resp {
    let key = parts.headers.get("sec-websocket-key").map(|k| k.as_bytes().to_vec()).unwrap_or_default();
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(&key);
    tokio::task::spawn_local(async move {
        if let Ok(up) = upgrade.await {
            let mut config = WebSocketConfig::default();
            config.max_message_size = Some(MAX_PAYLOAD);
            config.max_frame_size = Some(MAX_PAYLOAD);
            let ws = WebSocketStream::from_raw_socket(hyper_util::rt::TokioIo::new(up), Role::Server, Some(config)).await;
            join(ws);
        }
    });
    hyper::Response::builder()
        .status(101)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-accept", accept)
        .body(Empty::new().boxed_unsync())
        .expect("response")
}

fn refused() -> Resp {
    hyper::Response::builder()
        .status(403)
        .header("connection", "close")
        .header("content-length", "0")
        .body(Empty::<Bytes>::new().boxed_unsync())
        .expect("response")
}

/// Upgrades of this server, None for every other request. The request object of hyper is rebuilt from its parts.
pub async fn upgrade(parts: &mut hyper::http::request::Parts, path: &str, query: &str) -> Option<Resp> {
    let wants = parts.headers.get("upgrade").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    if !wants || path != CALL_AUDIO_PATH {
        return None;
    }
    let a = form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "a").map(|(_, v)| v.into_owned()).unwrap_or_default();
    let mut headers = hyper::HeaderMap::new();
    for h in ["cookie", "authorization", "origin"] {
        if let Some(v) = parts.headers.get(h) {
            headers.insert(h, v.clone());
        }
    }
    let check = Req {
        method: "GET".into(),
        path: "/api/call/audio".into(),
        query: if a.is_empty() { String::new() } else { format!("a={}", crate::js::encode_uri_component(&a)) },
        headers,
        body: Bytes::new(),
        params: vec![],
        stream_id: None,
    };
    let Ok((slot, side)) = crate::routes::calls::audio_side(&check) else { return Some(refused()) };
    let Some(on) = parts.extensions.remove::<hyper::upgrade::OnUpgrade>() else { return Some(refused()) };
    Some(accept(parts, on, move |ws| if side == "app" { join_app(slot, ws) } else { join_relay(slot, ws) }).await)
}

type Ws = WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>;

/// The writer of a socket: what the channel carries, until a close or the socket goes.
fn split(ws: Ws) -> (u64, mpsc::UnboundedSender<Message>, futures_util::stream::SplitStream<Ws>) {
    let id = IDS.with(|i| {
        let v = i.get();
        i.set(v + 1);
        v
    });
    let (mut sink, stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    tokio::task::spawn_local(async move {
        while let Some(m) = rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
        let _ = sink.close().await;
    });
    (id, tx, stream)
}

fn join_app(slot: i64, ws: Ws) {
    let (id, tx, mut stream) = split(ws);
    with_pair(slot, |p| {
        if let Some(old) = p.app.replace(Side { id, tx }) {
            let _ = old.tx.send(Message::Close(None));
        }
        to_app(p, Message::Text(settings(p.channels).into()));
        if p.relay.is_some() {
            to_app(p, Message::Text("AUDIO_STARTED".into()));
            if p.demand {
                to_app(p, Message::Text("CAPTURE_DEMAND microphone 1".into()));
            }
        } else {
            wait_for_relay(slot, p);
        }
    });
    tokio::task::spawn_local(async move {
        while let Some(Ok(m)) = stream.next().await {
            let current = with_pair(slot, |p| p.app.as_ref().is_some_and(|a| a.id == id));
            if !current {
                continue;
            }
            match m {
                Message::Text(t) => with_pair(slot, |p| {
                    if t.starts_with("START_AUDIO") && p.relay.is_some() {
                        to_app(p, Message::Text("AUDIO_STARTED".into()));
                    }
                }),
                Message::Binary(b) => with_pair(slot, |p| {
                    if b.first() == Some(&0x02)
                        && let Some(r) = &p.relay
                    {
                        let _ = r.tx.send(Message::Binary(b));
                    }
                }),
                Message::Close(_) => break,
                _ => {}
            }
        }
        let gone = with_pair(slot, |p| {
            if p.app.as_ref().is_some_and(|a| a.id == id) {
                p.app = None;
                clear_wait(p);
                true
            } else {
                false
            }
        });
        if gone {
            settle(slot);
        }
    });
}

fn join_relay(slot: i64, ws: Ws) {
    let (id, tx, mut stream) = split(ws);
    with_pair(slot, |p| {
        if let Some(old) = p.relay.replace(Side { id, tx }) {
            let _ = old.tx.send(Message::Close(None));
        }
        clear_wait(p);
        to_app(p, Message::Text("AUDIO_STARTED".into()));
    });
    let demand = regex::Regex::new("^CAPTURE_DEMAND microphone ([01])$").expect("fixed");
    let channels = regex::Regex::new("^CHANNELS ([12])$").expect("fixed");
    tokio::task::spawn_local(async move {
        while let Some(Ok(m)) = stream.next().await {
            let current = with_pair(slot, |p| p.relay.as_ref().is_some_and(|r| r.id == id));
            if !current {
                continue;
            }
            match m {
                Message::Binary(b) => with_pair(slot, |p| {
                    let mut frame = Vec::with_capacity(b.len() + 2);
                    frame.extend_from_slice(&[0x01, 0x00]);
                    frame.extend_from_slice(&b);
                    to_app(p, Message::Binary(frame.into()));
                }),
                Message::Text(t) => with_pair(slot, |p| {
                    if let Some(c) = demand.captures(&t) {
                        p.demand = &c[1] == "1";
                        to_app(p, Message::Text(t.clone()));
                    } else if let Some(c) = channels.captures(&t) {
                        p.channels = c[1].parse().unwrap_or(1);
                        to_app(p, Message::Text(settings(p.channels).into()));
                    }
                }),
                Message::Close(_) => break,
                _ => {}
            }
        }
        let gone = with_pair(slot, |p| {
            if p.relay.as_ref().is_some_and(|r| r.id == id) {
                p.relay = None;
                if p.demand {
                    to_app(p, Message::Text("CAPTURE_DEMAND microphone 0".into()));
                }
                p.demand = false;
                wait_for_relay(slot, p);
                true
            } else {
                false
            }
        });
        if gone {
            settle(slot);
        }
    });
}
