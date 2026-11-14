//! The Notifier of src/agent/push/notifier.ts: Web Push to the browsers of the owner, sealed FCM data messages to the
//! phones of the Android app, ntfy when enabled, and the history of the notified messages (messages table).

use super::fcm::{FcmResult, FcmSend, FcmSender};
use super::http::{Http, Request, reqwest_http};
use super::notify::{CALL_TTL, Field, PUSH_TTL, RecentPushes, call_tag, chat_tag, error_text, info, push_retry_delay, push_title, utf16_slice, warn};
use super::seal::seal_for;
use super::vapid::VapidKeys;
use super::webpush::{Options, SendError, send_notification};
use super::{Audience, Devices, PushDevices, PushTarget, device_side, is_phone};
use crate::state::{self, Identity};
use futures_util::future::{LocalBoxFuture, join_all};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

/// webpush.sendNotification, or a fake in the tests.
pub type WebSend = Rc<dyn Fn(Value, String, Options) -> LocalBoxFuture<'static, Result<(), SendError>>>;

/// Runs a retry after the given milliseconds; a timer in production, a list the tests run by hand.
pub type Later = Rc<dyn Fn(u64, LocalBoxFuture<'static, ()>)>;

/// Push service of Safari (web apps on the Home Screen of an iPhone or iPad, Safari on a Mac).
const APPLE_PUSH: &str = "https://web.push.apple.com/";

pub struct NotifierOptions {
    /// data/N/messages.db of the slot (history of the notified messages, known chats, identity)
    pub store_file: PathBuf,
    pub devices: Devices,
    pub vapid: Option<VapidKeys>,
    pub subject: String,
    /// url and topic
    pub ntfy: Option<(String, String)>,
    pub fcm: Option<FcmSender>,
    /// a ringing call can be answered from the app (an account of the browsers container): its pushes offer it
    pub answerable: bool,
}

/// What the network and the clock are, for a notifier: the real ones, or the fakes of the tests.
struct Hooks {
    send: WebSend,
    http: Http,
    later: Later,
    clock: Rc<dyn Fn() -> i64>,
    fcm: Option<Rc<dyn FcmSend>>,
}

/// How a push is delivered.
struct Delivery {
    urgency: String,
    ttl: i64,
    retry: bool,
}

type Skip = Box<dyn Fn(&PushTarget) -> bool>;
type Took = Rc<dyn Fn(&PushTarget)>;
/// when the ringing call started, and the phones it reached
type Ringing = Option<(i64, Rc<RefCell<HashSet<String>>>)>;

struct Inner {
    store_file: PathBuf,
    devices: Box<dyn PushDevices>,
    vapid: Option<VapidKeys>,
    subject: String,
    ntfy: Option<(String, String)>,
    answerable: bool,
    hooks: Hooks,
    recent: RefCell<RecentPushes>,
    /// the phones of the Android app the ringing call (since, when it started) reached: they ring on their own until it
    /// ends, another push would start the ringtone again
    phones_ringing: RefCell<Ringing>,
    /// the ntfy messages of calls, one after the other: the end of a call comes after its start
    ntfy_calls: RefCell<Option<tokio::sync::mpsc::UnboundedSender<Map<String, Value>>>>,
    fcm_missing: Cell<bool>,
}

/// Notifications of the account. Retries run on timers, so a push service that answers slowly or not at all never holds
/// up the caller.
pub struct Notifier(Rc<Inner>);

impl Notifier {
    pub fn new(o: NotifierOptions) -> Self {
        let http = reqwest_http();
        let web = http.clone();
        let send: WebSend = Rc::new(move |sub, payload, options| {
            let web = web.clone();
            Box::pin(async move { send_notification(&web, &sub, &payload, &options).await })
        });
        let later: Later = Rc::new(|ms, run| {
            tokio::task::spawn_local(async move {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                run.await;
            });
        });
        let fcm = o.fcm.map(|f| Rc::new(f) as Rc<dyn FcmSend>);
        let hooks = Hooks { send, http, later, clock: Rc::new(crate::web::now_ms), fcm };
        Notifier::assemble(o.store_file, Box::new(o.devices), o.vapid, o.subject, o.ntfy, o.answerable, hooks)
    }

    fn assemble(
        store_file: PathBuf,
        devices: Box<dyn PushDevices>,
        vapid: Option<VapidKeys>,
        subject: String,
        ntfy: Option<(String, String)>,
        answerable: bool,
        hooks: Hooks,
    ) -> Self {
        Notifier(Rc::new(Inner {
            store_file,
            devices,
            vapid,
            subject,
            ntfy,
            answerable,
            hooks,
            recent: RefCell::new(RecentPushes::new()),
            phones_ringing: RefCell::new(None),
            ntfy_calls: RefCell::new(None),
            fcm_missing: Cell::new(false),
        }))
    }

    /// A new Teams message: the same text within 150 s is notified once. On the device it replaces the notification of
    /// the same chat, unless that one shows a newer message (ts); the notification opens the app on `chat`, when known.
    pub async fn message(&self, title: &str, body: &str, chat: &str) {
        let n = &self.0;
        let now = (n.hooks.clock)();
        if !n.recent.borrow_mut().allow(body, now as f64) {
            return;
        }
        n.add_notification(title, body);
        if n.off() {
            return;
        }
        n.ntfy(title, body).await;
        self.push(title, body, chat, "high", title, now, false).await;
    }

    /// About the relay itself (Teams signed out, outcome of a check): push and ntfy to every device, no history.
    pub async fn alert(&self, title: &str, body: &str, urgency: &str) -> usize {
        self.0.ntfy(title, body).await;
        self.push(title, body, "", urgency, "", 0, true).await
    }

    /// What a job found: like a message, through the choice of the account, and nothing when both sides are off.
    pub async fn notice(&self, title: &str, body: &str, urgency: &str) -> usize {
        if self.0.off() {
            return 0;
        }
        self.0.ntfy(title, body).await;
        self.push(title, body, "", urgency, "", 0, false).await
    }

    pub fn device_count(&self) -> usize {
        self.0.devices.targets().len()
    }

    /// Push to every device; urgency high unless the caller says it can wait (empty: high). Pushes of one `group` (a
    /// chat) share one notification on the device, which keeps the newest (ts, ms). Only the sides the account chose,
    /// unless `everyone`.
    #[allow(clippy::too_many_arguments)]
    pub async fn push(&self, title: &str, body: &str, chat: &str, urgency: &str, group: &str, ts: i64, everyone: bool) -> usize {
        let (title, body, chat, group) = (title.to_string(), body.to_string(), chat.to_string(), group.to_string());
        let content = move |acc: i64| {
            let mut m = Map::new();
            m.insert("title".into(), json!(title));
            m.insert("body".into(), json!(body));
            m.insert("chat".into(), json!(chat));
            if !group.is_empty() {
                m.insert("tag".into(), json!(chat_tag(acc, &group)));
            }
            if ts != 0 {
                m.insert("ts".into(), json!(ts));
            }
            m
        };
        let urgency = if urgency.is_empty() { "high" } else { urgency };
        Inner::deliver(&self.0, content, Delivery { urgency: urgency.into(), ttl: PUSH_TTL, retry: true }, None, None, everyone).await
    }

    /// An incoming Teams call, on a notification of its own per account: "ringing" when it starts, "again" every few
    /// seconds while it rings, "ended" once it stops. While it rings the push service keeps it only while the call could
    /// still be answered, with no retry. Safari devices get the start and the end only (WebKit bug 258922); so do the
    /// phones of the Android app that took a push of this call, which loop the ringtone themselves, and ntfy. Not in the
    /// history of notified messages. answered: the call ended because it was answered from the app.
    pub async fn call(&self, caller: &str, state: &str, since: i64, seconds: i64, answered: bool) -> usize {
        let n = &self.0;
        if n.off() {
            return 0;
        }
        let ringing = state != "ended";
        let title = match (ringing, caller.is_empty()) {
            (true, false) => format!("{caller} is calling"),
            (true, true) => "Incoming call".to_string(),
            (false, false) => format!("Call from {caller}"),
            (false, true) => "Call ended".to_string(),
        };
        let body = if ringing {
            "Teams call, ringing now".to_string()
        } else if answered {
            "Answered in TeamsRelay".to_string()
        } else if seconds != 0 {
            format!("Ended after {seconds} s")
        } else {
            "Ended".to_string()
        };
        let chat = if !caller.is_empty() && n.is_known_chat(caller) { caller.to_string() } else { String::new() };
        if state == "ringing" {
            *n.phones_ringing.borrow_mut() = Some((since, Rc::new(RefCell::new(HashSet::new()))));
        }
        let took_set = n.phones_ringing.borrow().as_ref().filter(|(s, _)| *s == since).map(|(_, t)| t.clone());
        if state != "again" {
            n.ntfy_call(&title, &body, since, ringing);
        }
        let answer = ringing && n.answerable;
        let content = move |acc: i64| {
            let mut m = Map::new();
            m.insert("title".into(), json!(title));
            m.insert("body".into(), json!(body));
            m.insert("chat".into(), json!(chat));
            m.insert("tag".into(), json!(call_tag(acc)));
            m.insert("call".into(), json!(if ringing { "ringing" } else { "ended" }));
            m.insert("ts".into(), json!(since));
            if answer {
                m.insert("answer".into(), json!(true));
            }
            m
        };
        let skip: Option<Skip> = (state == "again").then(|| {
            let took = took_set.clone();
            Box::new(move |t: &PushTarget| {
                t.endpoint.starts_with(APPLE_PUSH) || (is_phone(&t.endpoint) && took.as_ref().is_none_or(|s| s.borrow().contains(&t.endpoint)))
            }) as Skip
        });
        let took: Option<Took> = match (ringing, took_set) {
            (true, Some(set)) => Some(Rc::new(move |t: &PushTarget| {
                if is_phone(&t.endpoint) {
                    set.borrow_mut().insert(t.endpoint.clone());
                }
            })),
            _ => None,
        };
        let o = Delivery { urgency: "high".into(), ttl: if ringing { CALL_TTL } else { PUSH_TTL }, retry: !ringing };
        Inner::deliver(n, content, o, skip, took, false).await
    }

    /// A missed call the check of an account found in its Teams Activity feed: a notification of its own, which alerts,
    /// and ntfy when enabled; time is the one Teams shows.
    pub async fn missed_call(&self, caller: &str, time: &str) -> usize {
        let n = &self.0;
        if n.off() {
            return 0;
        }
        let title = if caller.is_empty() { "Missed call".to_string() } else { format!("Missed call from {caller}") };
        let at = if time.is_empty() { String::new() } else { format!(" at {time}") };
        let body = format!("Teams call{at}, found by the check");
        n.ntfy(&title, &body).await;
        let chat = if !caller.is_empty() && n.is_known_chat(caller) { caller } else { "" };
        self.push(&title, &body, chat, "high", "", 0, false).await
    }
}

impl Inner {
    fn store<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
        let done = crate::slotdb::open_store(&self.store_file).and_then(|c| f(&c).map_err(crate::web::Error::from));
        match done {
            Ok(v) => Some(v),
            Err(e) => {
                warn("store", &error_text(&e.to_string()), &[]);
                None
            }
        }
    }

    fn add_notification(&self, title: &str, body: &str) {
        let ts = crate::web::now_s();
        self.store(|c| c.execute("INSERT INTO messages(ts, source, title, body) VALUES(?, 'teams', ?, ?)", rusqlite::params![ts, title, body]).map(|_| ()));
    }

    fn is_known_chat(&self, name: &str) -> bool {
        self.store(|c| c.query_row("SELECT 1 FROM chats WHERE name=?", [name], |_| Ok(())).optional()).flatten().is_some()
    }

    fn me(&self) -> Identity {
        let raw =
            self.store(|c| c.query_row("SELECT v FROM state WHERE k=?", [state::key::ME], |r| r.get::<_, Option<String>>(0)).optional()).flatten().flatten();
        Identity::parse(raw.as_deref())
    }

    fn audience(&self) -> Audience {
        self.devices.audience().unwrap_or(Audience::ALL)
    }

    fn off(&self) -> bool {
        let a = self.audience();
        !a.mobile && !a.desktop
    }

    /// The account the notifications come from, for an app that shows more than one: slot, label, logo of its company.
    fn account(&self) -> Option<(i64, String, String)> {
        let me = self.me();
        self.devices.account(&me).map(|(acc, label)| (acc, label, me.logo))
    }

    /// Sends to every device, all at once, what `content` makes for the account (its slot), the title naming the account
    /// when the owner has more: Web Push to the browsers, a sealed FCM message to the phones. Returns the devices the
    /// push service took on the first try; the ones it reported as gone are removed, the ones it could not take now are
    /// tried again later when `retry`.
    async fn deliver(
        me: &Rc<Inner>,
        content: impl FnOnce(i64) -> Map<String, Value>,
        o: Delivery,
        skip: Option<Skip>,
        took: Option<Took>,
        everyone: bool,
    ) -> usize {
        let side = if everyone { None } else { Some(me.audience()) };
        let targets: Vec<PushTarget> =
            me.devices.targets().into_iter().filter(|t| !skip.as_ref().is_some_and(|s| s(t)) && side.is_none_or(|a| a.has(device_side(t)))).collect();
        if targets.is_empty() {
            return 0;
        }
        let account = me.account();
        let mut message = content(account.as_ref().map_or(0, |a| a.0));
        let title = crate::js::string(message.get("title"));
        message.insert("title".into(), json!(push_title(&title, account.as_ref().map_or("", |a| a.1.as_str()))));
        if let Some((acc, label, logo)) = &account {
            message.insert("acc".into(), json!(acc));
            if !label.is_empty() {
                message.insert("org".into(), json!(label));
            }
            if !logo.is_empty() {
                message.insert("logo".into(), json!(logo));
            }
        }
        let o = Rc::new(o);
        let message = Rc::new(message);
        let noted = |t: PushTarget, f: LocalBoxFuture<'static, bool>| -> LocalBoxFuture<'static, bool> {
            let took = took.clone();
            Box::pin(async move {
                let ok = f.await;
                if ok && let Some(took) = took {
                    took(&t);
                }
                ok
            })
        };
        let mut sends: Vec<LocalBoxFuture<'static, bool>> = vec![];
        let (phones, browsers): (Vec<PushTarget>, Vec<PushTarget>) = targets.into_iter().partition(|t| is_phone(&t.endpoint));
        if let Some(vapid) = &me.vapid
            && !browsers.is_empty()
        {
            let payload = crate::js::stringify(&Value::Object((*message).clone()));
            let options = Options { subject: me.subject.clone(), vapid: vapid.clone(), ttl: o.ttl, urgency: o.urgency.clone(), timeout_ms: 15_000 };
            for t in browsers {
                sends.push(noted(t.clone(), Inner::send_to(me.clone(), t, payload.clone(), options.clone(), 0, o.retry)));
            }
        }
        if !phones.is_empty() && me.hooks.fcm.is_some() {
            for t in phones {
                sends.push(noted(t.clone(), Inner::send_to_phone(me.clone(), t, message.clone(), o.clone(), 0)));
            }
        } else if !phones.is_empty() && !me.fcm_missing.get() {
            me.fcm_missing.set(true);
            warn("push", "phones of the Android app registered, but no Firebase service account key: FCM off", &[]);
        }
        // one push service that does not answer (15 s) holds no other device
        join_all(sends).await.into_iter().filter(|ok| *ok).count()
    }

    fn send_to(me: Rc<Inner>, t: PushTarget, payload: String, options: Options, attempt: usize, retry: bool) -> LocalBoxFuture<'static, bool> {
        Box::pin(async move {
            let sub: Value = match serde_json::from_str(&t.sub) {
                Ok(v) => v,
                Err(e) => {
                    warn("push", &error_text(&e.to_string()), &[("attempt", attempt.into()), ("retry", "none".into())]);
                    return false;
                }
            };
            let e = match (me.hooks.send)(sub, payload.clone(), options.clone()).await {
                Ok(()) => {
                    if attempt > 0 {
                        info("push", "sent on retry", &[("attempt", attempt.into())]);
                    }
                    return true;
                }
                Err(e) => e,
            };
            let (status, retry_after) = match &e {
                SendError::Status { status, retry_after } => (Some(*status), retry_after.clone()),
                _ => (None, None),
            };
            if let Some(s @ (404 | 410)) = status {
                me.devices.remove(&t.endpoint);
                info("push", "device gone, removed", &[("status", s.into())]);
                return false;
            }
            // without a status, only a push service that did not answer is tried again: a subscription web-push refuses
            // before sending fails the same way every time
            let reached = status.is_some() || matches!(e, SendError::NoAnswer(_));
            let wait = if retry && reached { push_retry_delay(status, retry_after.as_deref(), attempt, (me.hooks.clock)() as f64) } else { None };
            let mut fields: Vec<(&str, Field)> = vec![];
            if let Some(s) = status {
                fields.push(("status", s.into()));
            }
            fields.push(("attempt", attempt.into()));
            fields.push(("retry", wait.map_or(Field::from("none"), Field::from)));
            warn("push", &error_text(&e.text()), &fields);
            if let Some(wait) = wait {
                let again = me.clone();
                Inner::retry_later(
                    &me,
                    wait,
                    t.endpoint.clone(),
                    attempt + 1,
                    Box::new(move |x| Inner::send_to(again, x, payload, options, attempt + 1, true)),
                );
            }
            false
        })
    }

    /// A sealed FCM message to a phone of the Android app: high priority (wakes a phone in Doze) for an urgent push,
    /// normal otherwise. A token FCM no longer knows is removed; 429, 5xx and no answer are tried again as for Web Push.
    fn send_to_phone(me: Rc<Inner>, t: PushTarget, message: Rc<Map<String, Value>>, o: Rc<Delivery>, attempt: usize) -> LocalBoxFuture<'static, bool> {
        Box::pin(async move {
            let phone = serde_json::from_str::<Value>(&t.sub).ok().and_then(|v| {
                let f = v.get("fcm")?;
                Some((f.get("token")?.as_str()?.to_string(), f.get("key")?.as_str()?.to_string()))
            });
            let (Some((token, key)), Some(fcm)) = (phone, me.hooks.fcm.clone()) else { return false };
            let mut status = None;
            let mut retry_after = None;
            let sent = match seal_fitting(&key, &message) {
                Ok(data) => fcm.send(&token, &data, o.ttl, o.urgency == "high").await,
                Err(e) => Err(e),
            };
            match sent {
                Ok(FcmResult::Ok) => {
                    if attempt > 0 {
                        info("push", "sent on retry", &[("attempt", attempt.into()), ("fcm", true.into())]);
                    }
                    return true;
                }
                Ok(FcmResult::Failed { status: s, gone: true, .. }) => {
                    me.devices.remove(&t.endpoint);
                    // 403: a token of another Firebase project, which every phone has when the service account key is
                    // not of the project the app was built with
                    if s == 403 {
                        warn("push", "phone of another Firebase project (SENDER_ID_MISMATCH), removed: key and app build must share one project", &[]);
                    } else {
                        info("push", "phone gone, removed", &[("status", s.into())]);
                    }
                    return false;
                }
                Ok(FcmResult::Failed { status: s, retry_after: r, .. }) => {
                    status = Some(s);
                    retry_after = r;
                }
                // no answer, or the access token refused: the status stays unknown
                Err(e) => warn("push", &format!("FCM: {}", error_text(&e)), &[("attempt", attempt.into())]),
            }
            let wait = if o.retry { push_retry_delay(status, retry_after.as_deref(), attempt, (me.hooks.clock)() as f64) } else { None };
            if let Some(s) = status {
                warn("push", &format!("FCM answered {s}"), &[("attempt", attempt.into()), ("retry", wait.map_or(Field::from("none"), Field::from))]);
            }
            if let Some(wait) = wait {
                let again = me.clone();
                Inner::retry_later(&me, wait, t.endpoint.clone(), attempt + 1, Box::new(move |x| Inner::send_to_phone(again, x, message, o, attempt + 1)));
            }
            false
        })
    }

    /// A retry in `wait` seconds goes to the device only while it is still one of the devices: a subscription can pass
    /// to another user of the same browser, or be removed, while the retry waits.
    fn retry_later(me: &Rc<Inner>, wait: i64, endpoint: String, attempt: usize, send: Box<dyn FnOnce(PushTarget) -> LocalBoxFuture<'static, bool>>) {
        let devices_of = me.clone();
        let run = Box::pin(async move {
            let Some(t) = devices_of.devices.targets().into_iter().find(|x| x.endpoint == endpoint) else {
                let what = if is_phone(&endpoint) { "phone" } else { "device" };
                info("push", &format!("{what} gone before the retry"), &[("attempt", attempt.into())]);
                return;
            };
            send(t).await;
        });
        (me.hooks.later)((wait.max(0) as u64) * 1000, run);
    }

    /// A call on ntfy: priority 5 while it rings, then the same notification (sequence id) at priority 2, quiet. Queued,
    /// so a slow ntfy holds no push of the call and its messages keep their order.
    fn ntfy_call(&self, title: &str, body: &str, since: i64, ringing: bool) {
        let Some(ntfy) = self.ntfy.clone() else { return };
        let account = self.account();
        let label = account.as_ref().map_or("", |a| a.1.as_str());
        let mut m = Map::new();
        m.insert("title".into(), json!(utf16_slice(&push_title(title, label), 0, 100)));
        m.insert("message".into(), json!(body));
        m.insert("priority".into(), json!(if ringing { 5 } else { 2 }));
        m.insert("tags".into(), json!(["telephone_receiver"]));
        m.insert("sequence_id".into(), json!(format!("call-{}-{since}", account.as_ref().map_or(0, |a| a.0))));
        let mut queue = self.ntfy_calls.borrow_mut();
        if queue.is_none() {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Map<String, Value>>();
            let http = self.hooks.http.clone();
            tokio::task::spawn_local(async move {
                while let Some(m) = rx.recv().await {
                    post_ntfy(&http, &ntfy, m).await;
                }
            });
            *queue = Some(tx);
        }
        if let Some(tx) = queue.as_ref() {
            let _ = tx.send(m);
        }
    }

    async fn ntfy(&self, title: &str, body: &str) {
        let Some(ntfy) = &self.ntfy else { return };
        let mut m = Map::new();
        m.insert("title".into(), json!(utf16_slice(if title.is_empty() { "Teams" } else { title }, 0, 100)));
        m.insert("message".into(), json!(utf16_slice(if body.is_empty() { "(new message)" } else { body }, 0, 1000)));
        m.insert("priority".into(), json!(4));
        m.insert("tags".into(), json!(["speech_balloon"]));
        post_ntfy(&self.hooks.http, ntfy, m).await;
    }
}

async fn post_ntfy(http: &Http, (url, topic): &(String, String), message: Map<String, Value>) {
    let mut body = Map::new();
    body.insert("topic".into(), json!(topic));
    body.extend(message);
    let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Err(e) = http(Request::post(url, headers, crate::js::stringify(&Value::Object(body)).into_bytes(), 10_000)).await {
        warn("ntfy", &error_text(&e), &[]);
    }
}

/// An FCM message carries 4096 bytes of data: long texts are cut, as for ntfy, and once more when the sealed message is
/// still too big (text of many bytes per character).
fn seal_fitting(key: &str, content: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let text = |k: &str| crate::js::string(content.get(k).filter(|v| !v.is_null()));
    let cut = |n: usize| {
        let mut m = content.clone();
        m.insert("title".into(), json!(utf16_slice(&text("title"), 0, 100)));
        m.insert("body".into(), json!(utf16_slice(&text("body"), 0, n)));
        Value::Object(m)
    };
    let data = seal_for(key, &cut(1000))?;
    if crate::js::stringify(&Value::Object(data.clone())).encode_utf16().count() < 3800 { Ok(data) } else { seal_for(key, &cut(200)) }
}

#[cfg(test)]
mod tests;
