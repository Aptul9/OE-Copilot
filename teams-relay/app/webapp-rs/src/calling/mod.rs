//! Teams calls without a browser and without Microsoft's JavaScript (in place of calling-live-host.mjs, which ran the
//! calling stack of Teams web in Node): the signaling of Teams reimplemented from a trace of that stack, the media over
//! webrtc-rs. One worker per account, run by the light host on the IPC channel of fork as the Node worker was.

pub mod bridge;
pub mod codec;
pub mod media;
pub mod sdp;
pub mod serve;
pub mod signaling;
pub mod trouter;

pub use crate::agent::{Fail, R};

use codec::{FRAME_20, RATE, dominant_notes, melody_frame, melody_heard};
use serde_json::{Value, json};
use signaling::{Account, CallLinks, Conversation, ECHO_BOT, Endpoint, Event, event_of, relay, uuid};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// One call from the account of `discovery` to `peer` (Microsoft's test-call service by default): the melody goes out,
/// what comes back is listened to for it. A line of JSON per step on stdout, the outcome at the end.
pub async fn echo(discovery: &Value, peer: Option<&str>, seconds: u64) -> R<Value> {
    let t0 = Instant::now();
    let step = |name: &str, extra: Value| {
        let mut v = json!({ "t": t0.elapsed().as_millis() as u64, "step": name });
        if let (Value::Object(m), Value::Object(x)) = (&mut v, extra) {
            m.extend(x);
        }
        println!("{v}");
    };
    let account = Account::of(discovery, "TeamsRelay")?;
    let http = signaling::http();
    let r = relay(&http, &account).await?;
    step("relay", json!({ "turn": r.fqdn }));
    let (e, mut requests) = Endpoint::start(account).await?;
    step("registered", json!({ "surl": url::Url::parse(&e.trouter.surl).ok().and_then(|u| u.host_str().map(String::from)) }));
    let (speaker, mut heard) = mpsc::unbounded_channel::<Vec<i16>>();
    let outcome = async {
        let media = media::Media::new(media::relay_servers(&r.fqdn, r.udp, r.tcp, &r.username, &r.credential), speaker).await?;
        let offer = media.offer().await?;
        step("offer", json!({ "bytes": offer.len() }));
        let links = CallLinks::new(&e);
        let conv = Conversation::create(&e, &links, &[json!({ "id": peer.unwrap_or(ECHO_BOT), "participantId": uuid() })], &offer).await?;
        step("created", json!({ "links": conv.links.len() }));
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut accepted = false;
        let mut ended = None;
        let mut muted_at = None;
        let mut sample = 0u64;
        while Instant::now() < deadline && ended.is_none() {
            let Ok(Some(req)) = tokio::time::timeout(Duration::from_millis(500), requests.recv()).await else {
                // 13 s into the call: muted for 4 s, as the Node host tested it
                if accepted && muted_at.is_none() && t0.elapsed() > Duration::from_secs(13) {
                    conv.set_muted(&e, true).await?;
                    media.muted.store(true, Ordering::Relaxed);
                    muted_at = Some(Instant::now());
                    step("muted", json!({}));
                }
                if let Some(at) = muted_at
                    && at.elapsed() > Duration::from_secs(4)
                    && media.muted.load(Ordering::Relaxed)
                {
                    conv.set_muted(&e, false).await?;
                    media.muted.store(false, Ordering::Relaxed);
                    step("unmuted", json!({}));
                }
                continue;
            };
            let ev = event_of(&req);
            let body = if matches!(ev, Event::Accepted { .. }) { CallLinks::acknowledgement(&e).to_string() } else { String::new() };
            e.trouter.answer(&req, 200, &body);
            step("callback", json!({ "name": req.name() }));
            match ev {
                Event::Accepted { by, answer } => {
                    media.answer(&answer).await?;
                    accepted = true;
                    step("accepted", json!({ "by": by, "codec": media.sending() }));
                    media.start_sending(Box::new(move || {
                        let mut f = vec![0i16; FRAME_20];
                        melody_frame(sample, &mut f);
                        sample += FRAME_20 as u64;
                        f
                    }));
                }
                Event::Ended(why) => ended = Some(why),
                _ => {}
            }
        }
        let local_hangup = ended.is_none();
        if local_hangup {
            conv.leave(&e).await?;
            step("left", json!({}));
        }
        let state = format!("{:?}", media.state());
        let codec = media.sending();
        media.close().await;
        Ok::<Value, Fail>(json!({ "accepted": accepted, "ended": ended, "localHangup": local_hangup, "state": state, "codec": codec, "sent": media.counters.sent.load(Ordering::Relaxed), "received": media.counters.received.load(Ordering::Relaxed) }))
    }
    .await;
    e.stop().await;
    let mut pcm = vec![];
    while let Ok(f) = heard.try_recv() {
        pcm.extend(f);
    }
    let values: Vec<f64> = pcm.iter().map(|&v| f64::from(v)).collect();
    let notes = dominant_notes(&values, f64::from(RATE));
    let mut out = outcome?;
    out["heardSeconds"] = json!(pcm.len() as f64 / f64::from(RATE));
    out["melodyHeard"] = json!(melody_heard(&notes, 4));
    out["notesHeard"] = json!(notes.iter().filter(|n| **n != 0.0).count());
    step("done", out.clone());
    Ok(out)
}
