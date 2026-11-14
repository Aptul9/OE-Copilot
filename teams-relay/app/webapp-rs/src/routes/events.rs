//! GET /api/events: one stream per open app. Every second health, chats, activity, the call log and the messages of the
//! open chat of the selected account with its last open (the account list every 5 s); every 250 ms the calls ringing
//! and in progress in every account of the user. An event goes out only for the parts whose content changed.

use crate::accounts::{accounts_of, no_health, slot_health, up_since};
use crate::appdb::{self, Slot};
use crate::auth::{current_user, pick_slot, require_user};
use crate::route;
use crate::server::Route;
use crate::slotdb::SlotReader;
use crate::state::{in_call_of, ringing_call};
use crate::web::{Req, Res, now_ms};
use bytes::Bytes;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::mpsc;

pub fn routes() -> Vec<Route> {
    vec![route!("GET", "/api/events", events)]
}

/// An account whose browser runs now, where a call can ring.
fn runs_browser(s: &Slot) -> bool {
    s.stopped == 0 && (s.check_every == 0 || s.checking > 0) && (s.light == 0 || s.refreshing > 0)
}

/// The calls ringing and in progress in the accounts of a user, one connection per account database for the stream.
#[derive(Default)]
pub struct CallReaders {
    readers: HashMap<i64, SlotReader>,
}

impl CallReaders {
    pub fn ringing(&mut self, slots: &[Slot]) -> Value {
        let now = now_ms() as f64;
        let mut calls = vec![];
        for s in slots {
            if !(runs_browser(s) || (s.light != 0 && s.stopped == 0)) {
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(e) = self.readers.entry(s.slot) {
                match SlotReader::try_for_slot(s.slot) {
                    Ok(Some(r)) => {
                        e.insert(r);
                    }
                    _ => continue,
                }
            }
            let r = &self.readers[&s.slot];
            if let Some((caller, since)) = ringing_call(r.call().as_ref(), now) {
                calls.push(json!({ "acc": s.slot, "caller": caller, "since": since }));
            }
            if let Some((caller, since, muted)) = in_call_of(r.in_call().as_ref(), now) {
                let mut o = serde_json::Map::new();
                o.insert("acc".into(), json!(s.slot));
                o.insert("caller".into(), json!(caller));
                o.insert("since".into(), since);
                if let Some(m) = muted {
                    o.insert("muted".into(), json!(m));
                }
                o.insert("active".into(), json!(true));
                calls.push(Value::Object(o));
            }
        }
        self.readers.retain(|slot, _| slots.iter().any(|s| s.slot == *slot));
        Value::Array(calls)
    }
}

struct Stream {
    tx: mpsc::Sender<Bytes>,
    last: HashMap<&'static str, String>,
}

impl Stream {
    /// false once the client is gone
    async fn send(&mut self, event: &'static str, data: &Value) -> bool {
        let s = crate::js::stringify(data);
        if self.last.get(event) == Some(&s) {
            return true;
        }
        let chunk = format!("event: {event}\ndata: {s}\n\n");
        self.last.insert(event, s);
        self.tx.send(Bytes::from(chunk)).await.is_ok()
    }
}

async fn events(req: Req) -> Res {
    let user = require_user(&req)?;
    let owned = appdb::slots_of(&user.id)?;
    let slot = if owned.is_empty() { 0 } else { pick_slot(&owned.iter().map(|s| s.slot).collect::<Vec<_>>(), req.query_param("a").as_deref())? };
    let chat = req.query_param("chat").unwrap_or_default();
    let told = req.query_param("told").as_deref() == Some("1");
    let cookie = req.header("cookie");
    let (tx, mut rx) = mpsc::channel::<Bytes>(64);
    tokio::task::spawn_local(async move {
        let mut st = Stream { tx, last: HashMap::new() };
        let mut reader: Option<SlotReader> = None;
        let mut calls = CallReaders::default();
        let mut mine = owned;
        let mut ticks: u64 = 0;
        let mut beat = tokio::time::interval(Duration::from_millis(250));
        let mut quarter: u64 = 0;
        loop {
            beat.tick().await;
            if st.tx.is_closed() {
                return;
            }
            let full = quarter.is_multiple_of(4);
            quarter += 1;
            if full {
                ticks += 1;
                let r: Result<bool, crate::web::Error> = async {
                    mine = appdb::slots_of(&user.id)?;
                    let own = if slot != 0 { mine.iter().find(|s| s.slot == slot).cloned() } else { None };
                    if ticks.is_multiple_of(60) {
                        let check = Req {
                            method: "GET".into(),
                            path: "/api/events".into(),
                            query: String::new(),
                            headers: cookie_headers(cookie.as_deref()),
                            body: Bytes::new(),
                            params: vec![],
                            stream_id: None,
                        };
                        let still = current_user(&check)?;
                        if still.is_none() || (slot != 0 && own.is_none()) {
                            return Ok(false);
                        }
                    }
                    if ticks % 5 == 1 && !st.send("accounts", &accounts_of(&user.id)?).await {
                        return Ok(false);
                    }
                    if !st.send("calls", &calls.ringing(&mine)).await {
                        return Ok(false);
                    }
                    let Some(own) = own else { return Ok(true) };
                    if reader.is_none() {
                        reader = SlotReader::try_for_slot(slot)?;
                    }
                    let push_subs = appdb::count_push_subscriptions(&user.id)?;
                    let h = reader.as_ref().map(|r| r.health(up_since(&own))).unwrap_or_else(|| no_health(&own));
                    let mut h = slot_health(h, &own);
                    crate::js::set(&mut h, "push_subs", Some(json!(push_subs)));
                    if !st.send("health", &Value::Object(h)).await {
                        return Ok(false);
                    }
                    if let Some(r) = &reader {
                        let (chats, activity, log) =
                            (Value::Array(r.chats()), r.activity(), Value::Array(r.call_log().into_iter().map(Value::Object).collect()));
                        let open = if chat.is_empty() { None } else { Some(r.open_of(&chat).unwrap_or(Value::Null)) };
                        let msgs = if chat.is_empty() { None } else { Some(json!({ "chat": chat, "rows": r.messages(&chat), "open": open })) };
                        if !chat.is_empty() && !told && ticks % 10 == 1 {
                            r.mark_viewing(&chat)?;
                        }
                        if !st.send("chats", &chats).await || !st.send("activity", &activity).await || !st.send("calllog", &log).await {
                            return Ok(false);
                        }
                        if let Some(m) = msgs
                            && !st.send("messages", &m).await
                        {
                            return Ok(false);
                        }
                    }
                    if ticks.is_multiple_of(20) && st.tx.send(Bytes::from_static(b": ping\n\n")).await.is_err() {
                        return Ok(false);
                    }
                    Ok(true)
                }
                .await;
                match r {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => eprintln!("events: {e}"),
                }
            } else {
                let r: Result<bool, crate::web::Error> = async {
                    if !st.send("calls", &calls.ringing(&mine)).await {
                        return Ok(false);
                    }
                    if let Some(r) = &reader {
                        if !st.send("chats", &Value::Array(r.chats())).await {
                            return Ok(false);
                        }
                        if !chat.is_empty() {
                            let open = r.open_of(&chat).unwrap_or(Value::Null);
                            let m = json!({ "chat": chat, "rows": r.messages(&chat), "open": open });
                            if !st.send("messages", &m).await {
                                return Ok(false);
                            }
                        }
                    }
                    Ok(true)
                }
                .await;
                match r {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => eprintln!("events: {e}"),
                }
            }
        }
    });
    let stream = futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx).map(|o| o.map(|b| Ok::<_, Infallible>(Frame::data(b)))));
    Ok(hyper::Response::builder()
        .status(200)
        .header("content-type", "text/event-stream; charset=utf-8")
        .header("cache-control", "no-cache, no-transform")
        .header("x-accel-buffering", "no")
        .body(StreamBody::new(stream).boxed_unsync())
        .expect("response"))
}

fn cookie_headers(cookie: Option<&str>) -> hyper::HeaderMap {
    let mut h = hyper::HeaderMap::new();
    if let Some(v) = cookie.and_then(|c| hyper::header::HeaderValue::from_str(c).ok()) {
        h.insert("cookie", v);
    }
    h
}
