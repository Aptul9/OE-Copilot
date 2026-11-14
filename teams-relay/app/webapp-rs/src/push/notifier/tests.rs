use super::*;
use crate::push::app_store::{audience_in, owner_has_many_in, seed_app_db, targets_in};
use crate::push::http::Response;
use crate::push::notify::account_label;
use crate::push::seal::{new_device_key, open};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The devices of slot 1 in an app.db of the test, through the queries of AppStore.
struct DbDevices {
    c: Rc<Connection>,
}

impl PushDevices for DbDevices {
    fn targets(&self) -> Vec<PushTarget> {
        targets_in(&self.c, 1, &crate::auth::rows::iso_ms(1_790_000_000_000)).expect("targets")
    }
    fn remove(&self, endpoint: &str) {
        self.c.execute("DELETE FROM push_subscriptions WHERE endpoint=?", [endpoint]).expect("remove");
    }
    fn account(&self, me: &Identity) -> Option<(i64, String)> {
        Some((1, account_label(owner_has_many_in(&self.c, 1).expect("count"), me, 1)))
    }
    fn audience(&self) -> Option<Audience> {
        Some(audience_in(&self.c, 1).expect("read").unwrap_or(Audience::ALL))
    }
}

/// token, data, ttl, high of each FCM message
type FcmLog = Rc<RefCell<Vec<(String, Map<String, Value>, i64, bool)>>>;
type Retries = Rc<RefCell<Vec<(u64, LocalBoxFuture<'static, ()>)>>>;

struct FakeFcm {
    sent: FcmLog,
    answers: Rc<RefCell<Vec<FcmResult>>>,
}

impl FcmSend for FakeFcm {
    fn send<'a>(&'a self, token: &'a str, data: &'a Map<String, Value>, ttl: i64, high: bool) -> LocalBoxFuture<'a, Result<FcmResult, String>> {
        self.sent.borrow_mut().push((token.to_string(), data.clone(), ttl, high));
        let a = if self.answers.borrow().is_empty() { FcmResult::Ok } else { self.answers.borrow_mut().remove(0) };
        Box::pin(async move { Ok(a) })
    }
}

type Sent = (String, String, i64, String);

#[derive(Default)]
struct Opts {
    answerable: bool,
    fcm: bool,
    no_vapid: bool,
    ntfy: bool,
}

struct World {
    db: Rc<Connection>,
    store_file: PathBuf,
    sent: Rc<RefCell<Vec<Sent>>>,
    failures: Rc<RefCell<Vec<SendError>>>,
    retries: Retries,
    fcm_sent: FcmLog,
    fcm_answers: Rc<RefCell<Vec<FcmResult>>>,
    posted: Rc<RefCell<Vec<(String, String)>>>,
    now: Rc<Cell<i64>>,
}

static STORES: AtomicUsize = AtomicUsize::new(0);

fn sub(endpoint: &str) -> String {
    json!({ "endpoint": endpoint, "keys": { "p256dh": "k", "auth": "a" } }).to_string()
}

fn owner_devices() -> Vec<(String, String, String)> {
    ["https://push/u1-phone", "https://push/u1-gone", "https://push/u2-phone"]
        .iter()
        .map(|e| (e.to_string(), if e.contains("u2") { "u2" } else { "u1" }.to_string(), sub(e)))
        .collect()
}

impl World {
    fn new(devices: &[(String, String, String)]) -> World {
        let list: Vec<(&str, &str, &str)> = devices.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
        let n = STORES.fetch_add(1, Ordering::SeqCst);
        let store_file = std::env::temp_dir().join(format!("trweb-notifier-{}-{n}", std::process::id())).join("1").join("messages.db");
        let _ = std::fs::remove_file(&store_file);
        crate::slotdb::open_store(&store_file).expect("store");
        World {
            db: Rc::new(seed_app_db(&list)),
            store_file,
            sent: Rc::default(),
            failures: Rc::default(),
            retries: Rc::default(),
            fcm_sent: Rc::default(),
            fcm_answers: Rc::default(),
            posted: Rc::default(),
            now: Rc::new(Cell::new(1_000_000)),
        }
    }

    fn notifier(&self, o: Opts) -> Notifier {
        let (sent, failures) = (self.sent.clone(), self.failures.clone());
        let send: WebSend = Rc::new(move |s, payload, options| {
            let endpoint = s["endpoint"].as_str().unwrap_or("").to_string();
            let r = if endpoint.ends_with("gone") {
                Err(SendError::Status { status: 410, retry_after: None })
            } else if !failures.borrow().is_empty() {
                Err(failures.borrow_mut().remove(0))
            } else {
                sent.borrow_mut().push((endpoint, payload, options.ttl, options.urgency));
                Ok(())
            };
            Box::pin(async move { r })
        });
        let posted = self.posted.clone();
        let http: Http = Rc::new(move |r: Request| {
            posted.borrow_mut().push((r.url.clone(), String::from_utf8(r.body).expect("utf8")));
            Box::pin(async { Ok(Response { status: 200, ..Default::default() }) })
        });
        let retries = self.retries.clone();
        let later: Later = Rc::new(move |ms, run| retries.borrow_mut().push((ms, run)));
        let now = self.now.clone();
        let fcm = o.fcm.then(|| Rc::new(FakeFcm { sent: self.fcm_sent.clone(), answers: self.fcm_answers.clone() }) as Rc<dyn FcmSend>);
        let hooks = Hooks { send, http, later, clock: Rc::new(move || now.get()), fcm };
        let vapid = (!o.no_vapid).then(|| VapidKeys { public_key: "BPublic".into(), private_key: "private".into() });
        let ntfy = o.ntfy.then(|| ("https://ntfy.example".to_string(), "relay-test".to_string()));
        Notifier::assemble(
            self.store_file.clone(),
            Box::new(DbDevices { c: self.db.clone() }),
            vapid,
            "mailto:admin@example.com".into(),
            ntfy,
            o.answerable,
            hooks,
        )
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.borrow().clone()
    }

    fn payloads(&self) -> Vec<Value> {
        self.sent.borrow().iter().map(|s| serde_json::from_str(&s.1).expect("json")).collect()
    }

    fn waits(&self) -> Vec<u64> {
        self.retries.borrow().iter().map(|r| r.0).collect()
    }

    /// Runs the first retry waiting, as its timer would.
    async fn retry_now(&self) {
        let waiting = !self.retries.borrow().is_empty();
        if waiting {
            let (_, run) = self.retries.borrow_mut().remove(0);
            run.await;
        }
    }

    fn endpoints(&self) -> Vec<String> {
        self.db
            .prepare("SELECT endpoint FROM push_subscriptions ORDER BY endpoint")
            .expect("sql")
            .query_map([], |r| r.get(0))
            .expect("rows")
            .map(Result::unwrap)
            .collect()
    }

    fn store(&self, sql: &str) {
        crate::slotdb::open_store(&self.store_file).expect("store").execute_batch(sql).expect("sql");
    }

    fn history(&self) -> Vec<String> {
        let c = crate::slotdb::open_store(&self.store_file).expect("store");
        c.prepare("SELECT title FROM messages ORDER BY id").expect("sql").query_map([], |r| r.get(0)).expect("rows").map(Result::unwrap).collect()
    }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
    tokio::task::LocalSet::new().block_on(&rt, f)
}

/// The JSON a payload must be, byte for byte (key order included).
fn text(v: Value) -> String {
    crate::js::stringify(&v)
}

const SINCE: i64 = 1_790_000_000_000;

#[test]
fn pushes_to_the_devices_of_the_owner_urgent_kept_a_day_and_forgets_the_gone_ones() {
    let w = World::new(&owner_devices());
    let n = w.notifier(Opts::default());
    assert_eq!(run(n.push("Anna Rossi", "ciao", "", "high", "", 0, false)), 1);
    assert_eq!(
        w.sent(),
        [("https://push/u1-phone".into(), text(json!({"title": "Anna Rossi", "body": "ciao", "chat": "", "acc": 1})), PUSH_TTL, "high".into())]
    );
    assert!(w.waits().is_empty());
    assert_eq!(w.endpoints(), ["https://push/u1-phone", "https://push/u2-phone"]);
    assert_eq!(n.device_count(), 1);
}

#[test]
fn names_the_account_when_the_owner_has_more_and_sends_the_logo() {
    let w = World::new(&owner_devices());
    w.db.execute_batch("INSERT INTO teams_accounts(slot, owner_id, added) VALUES(3, 'u1', 0)").expect("account");
    w.store(r#"INSERT INTO state VALUES('me', '{"name":"Anna","email":"anna@contoso.example","tenant":"Contoso","av":""}')"#);
    let n = w.notifier(Opts::default());
    run(n.push("Anna Rossi", "ciao", "", "high", "", 0, false));
    assert_eq!(w.sent()[0].1, text(json!({"title": "Anna Rossi \u{b7} Contoso", "body": "ciao", "chat": "", "acc": 1, "org": "Contoso"})));

    let w = World::new(&owner_devices());
    w.store(r#"INSERT INTO state VALUES('me', '{"name":"Anna","tenant":"Contoso","logo":"0123456789abcdef.png"}')"#);
    run(w.notifier(Opts::default()).alert("Teams signed out", "Sign in again", ""));
    assert_eq!(w.sent()[0].1, text(json!({"title": "Teams signed out", "body": "Sign in again", "chat": "", "acc": 1, "logo": "0123456789abcdef.png"})));
}

#[test]
fn a_message_is_tagged_with_its_chat_recorded_and_notified_once_within_150_s() {
    let w = World::new(&owner_devices());
    let n = w.notifier(Opts::default());
    run(async {
        n.message("Anna Rossi", "are you there?", "Anna Rossi").await;
        w.now.set(w.now.get() + 60_000);
        n.message("Anna Rossi", "Are you there?", "Anna Rossi").await;
    });
    assert_eq!(
        w.sent(),
        [(
            "https://push/u1-phone".into(),
            text(json!({"title": "Anna Rossi", "body": "are you there?", "chat": "Anna Rossi", "tag": "chat-1-Anna Rossi", "ts": 1_000_000, "acc": 1})),
            PUSH_TTL,
            "high".into()
        )]
    );
    assert_eq!(w.history(), ["Anna Rossi"]);
}

#[test]
fn messages_and_alerts_high_a_passed_check_normal_alerts_not_in_the_history() {
    let w = World::new(&owner_devices());
    let n = w.notifier(Opts::default());
    run(async {
        n.message("Anna Rossi", "urgent?", "Anna Rossi").await;
        n.alert("Teams signed out", "Sign in again", "").await;
        n.alert("Teams OK", "Automatic check: the whole chain works.", "normal").await;
    });
    let got: Vec<(String, String)> = w.sent().into_iter().zip(w.payloads()).map(|(s, p)| (p["title"].as_str().expect("title").to_string(), s.3)).collect();
    assert_eq!(got, [("Anna Rossi".into(), "high".into()), ("Teams signed out".into(), "high".into()), ("Teams OK".into(), "normal".into())]);
    assert_eq!(w.history(), ["Anna Rossi"]);
}

#[test]
fn calls_ring_on_a_notification_of_their_own_and_turn_quiet_when_they_end() {
    let w = World::new(&owner_devices());
    let n = w.notifier(Opts::default());
    assert_eq!(run(n.call("Anna Rossi", "ringing", SINCE, 0, false)), 1);
    assert_eq!(
        w.sent(),
        [(
            "https://push/u1-phone".into(),
            text(
                json!({"title": "Anna Rossi is calling", "body": "Teams call, ringing now", "chat": "", "tag": "call-1", "call": "ringing", "ts": SINCE, "acc": 1})
            ),
            CALL_TTL,
            "high".into()
        )]
    );
    assert!(w.history().is_empty());

    // the chat of the caller when known; kept a day
    let w = World::new(&owner_devices());
    w.store("INSERT INTO chats(name) VALUES('Anna Rossi')");
    run(w.notifier(Opts::default()).call("Anna Rossi", "ended", SINCE, 9, false));
    assert_eq!(
        w.sent(),
        [(
            "https://push/u1-phone".into(),
            text(
                json!({"title": "Call from Anna Rossi", "body": "Ended after 9 s", "chat": "Anna Rossi", "tag": "call-1", "call": "ended", "ts": SINCE, "acc": 1})
            ),
            PUSH_TTL,
            "high".into()
        )]
    );

    // no caller read, the account named when the owner has more
    let w = World::new(&owner_devices());
    w.db.execute_batch("INSERT INTO teams_accounts(slot, owner_id, added) VALUES(3, 'u1', 0)").expect("account");
    w.store(r#"INSERT INTO state VALUES('me', '{"tenant":"Contoso"}')"#);
    let n = w.notifier(Opts::default());
    run(async {
        n.call("", "ringing", SINCE, 0, false).await;
        n.call("", "ended", SINCE, 0, false).await;
    });
    let got: Vec<(String, String)> = w.payloads().iter().map(|p| (p["title"].as_str().expect("t").into(), p["body"].as_str().expect("b").into())).collect();
    assert_eq!(got, [("Incoming call \u{b7} Contoso".into(), "Teams call, ringing now".into()), ("Call ended \u{b7} Contoso".into(), "Ended".into())]);
}

#[test]
fn the_answer_is_offered_while_ringing_by_an_account_that_can_answer_only() {
    let w = World::new(&owner_devices());
    let n = w.notifier(Opts { answerable: true, ..Default::default() });
    run(async {
        n.call("Anna Rossi", "ringing", SINCE, 0, false).await;
        n.call("Anna Rossi", "again", SINCE, 0, false).await;
        n.call("Anna Rossi", "ended", SINCE, 9, true).await;
    });
    let p = w.payloads();
    assert_eq!(p.iter().map(|p| p.get("answer").cloned()).collect::<Vec<_>>(), [Some(json!(true)), Some(json!(true)), None]);
    assert_eq!((p[2]["body"].as_str(), p[2]["call"].as_str()), (Some("Answered in TeamsRelay"), Some("ended")));

    let w = World::new(&owner_devices());
    run(w.notifier(Opts::default()).call("Anna Rossi", "ringing", SINCE, 0, false));
    assert!(w.payloads()[0].get("answer").is_none());
}

#[test]
fn a_ringing_push_is_not_tried_again_an_ended_one_is() {
    let w = World::new(&owner_devices());
    w.failures.borrow_mut().extend([SendError::Status { status: 503, retry_after: None }, SendError::Status { status: 503, retry_after: None }]);
    let n = w.notifier(Opts::default());
    run(n.call("Anna Rossi", "ringing", SINCE, 0, false));
    assert!(w.waits().is_empty());
    run(n.call("Anna Rossi", "ended", SINCE, 9, false));
    assert_eq!(w.waits(), [5000]);
}

#[test]
fn an_iphone_rings_at_the_start_and_the_end_only() {
    let apple = "https://web.push.apple.com/QK1-iphone";
    let w = World::new(&[("https://push/u1-phone".into(), "u1".into(), sub("https://push/u1-phone")), (apple.into(), "u1".into(), sub(apple))]);
    let n = w.notifier(Opts::default());
    run(async {
        n.call("Anna Rossi", "ringing", SINCE, 0, false).await;
        n.call("Anna Rossi", "again", SINCE, 0, false).await;
        n.call("Anna Rossi", "ended", SINCE, 9, false).await;
    });
    let calls = |e: &str| -> Vec<String> {
        w.sent().iter().zip(w.payloads()).filter(|(s, _)| s.0 == e).map(|(_, p)| p["call"].as_str().expect("call").to_string()).collect()
    };
    assert_eq!(calls("https://push/u1-phone"), ["ringing", "ringing", "ended"]);
    assert_eq!(calls(apple), ["ringing", "ended"]);
}

#[test]
fn a_missed_call_found_by_a_check() {
    let w = World::new(&owner_devices());
    w.store("INSERT INTO chats(name) VALUES('Anna Rossi')");
    let n = w.notifier(Opts::default());
    run(async {
        assert_eq!(n.missed_call("Anna Rossi", "1:15 PM").await, 1);
        n.missed_call("", "").await;
    });
    assert_eq!(
        w.sent().into_iter().map(|s| s.1).collect::<Vec<_>>(),
        [
            text(json!({"title": "Missed call from Anna Rossi", "body": "Teams call at 1:15 PM, found by the check", "chat": "Anna Rossi", "acc": 1})),
            text(json!({"title": "Missed call", "body": "Teams call, found by the check", "chat": "", "acc": 1})),
        ]
    );
}

#[test]
fn tries_again_when_the_push_service_fails_or_does_not_answer() {
    let w = World::new(&owner_devices());
    w.failures.borrow_mut().extend([
        SendError::Status { status: 503, retry_after: None },
        SendError::NoAnswer("read ECONNRESET".into()),
        SendError::NoAnswer("Socket timeout".into()),
    ]);
    let n = w.notifier(Opts::default());
    run(async {
        assert_eq!(n.push("Anna Rossi", "ciao", "", "high", "", 0, false).await, 0);
        assert_eq!(w.waits(), [5000]);
        w.retry_now().await;
        assert_eq!(w.waits(), [30_000]);
        w.retry_now().await;
        assert_eq!(w.waits(), [120_000]);
        w.retry_now().await;
    });
    assert_eq!(
        w.sent(),
        [("https://push/u1-phone".into(), text(json!({"title": "Anna Rossi", "body": "ciao", "chat": "", "acc": 1})), PUSH_TTL, "high".into())]
    );
    assert!(w.waits().is_empty());
}

#[test]
fn retry_rules() {
    let push = |failures: Vec<SendError>| {
        let w = World::new(&owner_devices());
        w.failures.borrow_mut().extend(failures);
        let n = w.notifier(Opts::default());
        assert_eq!(run(n.push("Anna Rossi", "ciao", "", "high", "", 0, false)), 0);
        (w, n)
    };
    // refused by web-push before sending
    let (w, _) = push(vec![SendError::Refused("You must pass in a subscription with at least an endpoint.".into())]);
    assert!(w.waits().is_empty());
    for status in [400, 401, 403, 413] {
        let (w, _) = push(vec![SendError::Status { status, retry_after: None }]);
        assert!(w.waits().is_empty(), "{status}");
    }
    // as long as a 429 asks, within 15 minutes
    let (w, _n) =
        push(vec![SendError::Status { status: 429, retry_after: Some("42".into()) }, SendError::Status { status: 429, retry_after: Some("86400".into()) }]);
    assert_eq!(w.waits(), [42_000]);
    run(w.retry_now());
    assert_eq!(w.waits(), [900_000]);
    // three retries, then none
    let statuses = [500, 502, 503, 504];
    let (w, _n) = push(statuses.iter().map(|&status| SendError::Status { status, retry_after: None }).collect());
    let mut waits = vec![];
    while let Some(ms) = w.waits().first().copied() {
        waits.push(ms);
        run(w.retry_now());
    }
    assert_eq!(waits, [5000, 30_000, 120_000]);
    assert!(w.failures.borrow().is_empty() && w.sent().is_empty());
}

#[test]
fn a_retry_is_dropped_when_the_device_is_no_longer_the_owners() {
    let w = World::new(&owner_devices());
    w.failures.borrow_mut().extend([SendError::Status { status: 503, retry_after: None }, SendError::Status { status: 503, retry_after: None }]);
    let n = w.notifier(Opts::default());
    run(n.push("Anna Rossi", "ciao", "", "high", "", 0, false));
    w.db.execute_batch("UPDATE push_subscriptions SET user_id='u2' WHERE endpoint='https://push/u1-phone'").expect("update");
    run(w.retry_now());
    assert_eq!(w.failures.borrow().len(), 1);
    assert!(w.sent().is_empty() && w.waits().is_empty());
}

#[test]
fn nothing_without_keys() {
    let w = World::new(&owner_devices());
    assert_eq!(run(w.notifier(Opts { no_vapid: true, ..Default::default() }).push("x", "y", "", "high", "", 0, false)), 0);
}

const TOKEN: &str = "android-token-1234567890";

fn with_phones(key: &str) -> World {
    let phone = |token: &str, name: &str, session: &str| json!({ "fcm": { "token": token, "key": key, "name": name, "session": session } }).to_string();
    World::new(&[
        ("https://push/u1-phone".into(), "u1".into(), sub("https://push/u1-phone")),
        (format!("fcm:{TOKEN}"), "u1".into(), phone(TOKEN, "Google Pixel 9", "s-live")),
        ("fcm:token-of-a-session-run-out-12345".into(), "u1".into(), phone("token-of-a-session-run-out-12345", "Old phone", "s-over")),
        ("fcm:token-of-a-session-signed-out-1".into(), "u1".into(), phone("token-of-a-session-signed-out-1", "Lost phone", "s-gone")),
    ])
}

fn fcm() -> Opts {
    Opts { fcm: true, ..Default::default() }
}

#[test]
fn a_phone_gets_the_content_of_the_browsers_sealed_with_its_key_only_while_its_session_runs() {
    let key = new_device_key();
    let w = with_phones(&key);
    run(w.notifier(fcm()).message("Anna Rossi", "are you there?", "Anna Rossi"));
    let sent = w.fcm_sent.borrow().clone();
    assert_eq!(sent.iter().map(|s| (s.0.as_str(), s.2, s.3)).collect::<Vec<_>>(), [(TOKEN, PUSH_TTL, true)]);
    assert_eq!(sent[0].1["v"], "1");
    let opened = open(&key, &sent[0].1).expect("opens");
    assert_eq!(text(opened), w.sent()[0].1);
}

#[test]
fn a_phone_rings_at_the_start_and_turns_quiet_at_the_end_and_a_missed_first_push_rings_with_the_next() {
    let key = new_device_key();
    let w = with_phones(&key);
    let n = w.notifier(fcm());
    run(async {
        n.call("Anna Rossi", "ringing", SINCE, 0, false).await;
        n.call("Anna Rossi", "again", SINCE, 0, false).await;
        n.call("Anna Rossi", "ended", SINCE, 7, false).await;
    });
    let sent = w.fcm_sent.borrow().clone();
    assert_eq!(sent.iter().map(|s| (s.2, s.3)).collect::<Vec<_>>(), [(CALL_TTL, true), (PUSH_TTL, true)]);
    let first = open(&key, &sent[0].1).expect("opens");
    assert_eq!((first["title"].as_str(), first["call"].as_str(), first["tag"].as_str()), (Some("Anna Rossi is calling"), Some("ringing"), Some("call-1")));
    assert_eq!(open(&key, &sent[1].1).expect("opens")["body"], "Ended after 7 s");

    let w = with_phones(&key);
    w.fcm_answers.borrow_mut().push(FcmResult::Failed { status: 503, gone: false, retry_after: None });
    let n = w.notifier(fcm());
    run(async {
        n.call("Anna Rossi", "ringing", SINCE, 0, false).await;
        n.call("Anna Rossi", "again", SINCE, 0, false).await;
        n.call("Anna Rossi", "again", SINCE, 0, false).await;
    });
    assert_eq!(w.fcm_sent.borrow().len(), 2);
    assert_eq!(open(&key, &w.fcm_sent.borrow()[1].1).expect("opens")["call"], "ringing");
    assert!(w.waits().is_empty());
}

#[test]
fn a_passed_check_is_normal_priority_and_a_long_message_is_cut_to_fit_fcm() {
    let key = new_device_key();
    let w = with_phones(&key);
    let n = w.notifier(fcm());
    run(async {
        n.alert("Teams OK", "Automatic check: the whole chain works.", "normal").await;
        n.message("Anna Rossi", &"\u{e9}".repeat(5000), "Anna Rossi").await;
    });
    let sent = w.fcm_sent.borrow().clone();
    assert!(!sent[0].3);
    assert_eq!(open(&key, &sent[1].1).expect("opens")["body"], "\u{e9}".repeat(1000));
    assert!(crate::js::stringify(&Value::Object(sent[1].1.clone())).len() < 4000);
    // many bytes per character: still too big at 1000, cut again to 200
    let long = "\u{4e2d}".repeat(2000);
    run(n.message("Anna Rossi", &long, "Anna Rossi"));
    assert_eq!(open(&key, &w.fcm_sent.borrow()[2].1).expect("opens")["body"].as_str().expect("body").encode_utf16().count(), 200);
}

#[test]
fn a_gone_phone_is_removed_and_one_the_service_could_not_take_is_tried_later() {
    let key = new_device_key();
    let w = with_phones(&key);
    w.fcm_answers.borrow_mut().extend([
        FcmResult::Failed { status: 503, gone: false, retry_after: None },
        FcmResult::Failed { status: 429, gone: false, retry_after: Some("90".into()) },
    ]);
    let n = w.notifier(fcm());
    run(async {
        n.message("Anna Rossi", "one", "Anna Rossi").await;
        assert_eq!(w.waits(), [5000]);
        w.retry_now().await;
        assert_eq!(w.fcm_sent.borrow().len(), 2);
        assert_eq!(w.waits(), [90_000]);
        w.fcm_answers.borrow_mut().push(FcmResult::Failed { status: 404, gone: true, retry_after: None });
        n.message("Anna Rossi", "two", "Anna Rossi").await;
    });
    assert_eq!(w.endpoints(), ["fcm:token-of-a-session-run-out-12345", "fcm:token-of-a-session-signed-out-1", "https://push/u1-phone"]);
}

#[test]
fn without_the_service_account_key_the_browsers_are_still_reached() {
    let w = with_phones(&new_device_key());
    assert_eq!(run(w.notifier(Opts::default()).push("Anna Rossi", "ciao", "", "high", "", 0, false)), 1);
    assert!(w.fcm_sent.borrow().is_empty());
    assert_eq!(w.sent().len(), 1);
}

#[test]
fn the_sides_the_account_chose() {
    let mobile = |e: &str| json!({ "endpoint": e, "keys": { "p256dh": "k", "auth": "a" }, "kind": "mobile" }).to_string();
    let devices = || {
        vec![
            ("https://push/desk".to_string(), "u1".to_string(), sub("https://push/desk")),
            ("https://push/mob".to_string(), "u1".to_string(), mobile("https://push/mob")),
        ]
    };
    let w = World::new(&devices());
    w.db.execute_batch("UPDATE teams_accounts SET notify_mobile=0 WHERE slot=1").expect("update");
    assert_eq!(run(w.notifier(Opts::default()).notice("Found", "x", "")), 1);
    assert_eq!(w.sent()[0].0, "https://push/desk");

    let w = World::new(&devices());
    w.db.execute_batch("UPDATE teams_accounts SET notify_mobile=0, notify_desktop=0 WHERE slot=1").expect("update");
    let n = w.notifier(Opts::default());
    run(async {
        n.message("Anna Rossi", "hi", "Anna Rossi").await;
        assert_eq!(n.notice("Found", "x", "").await, 0);
        assert_eq!(n.call("Anna Rossi", "ringing", SINCE, 0, false).await, 0);
        assert_eq!(n.missed_call("Anna Rossi", "").await, 0);
    });
    assert!(w.sent().is_empty());
    assert_eq!(w.history(), ["Anna Rossi"]);
    // alerts are about the relay: every device
    assert_eq!(run(n.alert("Teams signed out", "Sign in again", "")), 2);
}

#[test]
fn ntfy_for_messages_and_for_calls_in_order() {
    let w = World::new(&[]);
    let n = w.notifier(Opts { ntfy: true, no_vapid: true, ..Default::default() });
    run(async {
        n.message("", "", "").await;
        n.call("Anna Rossi", "ringing", SINCE, 0, false).await;
        n.call("Anna Rossi", "again", SINCE, 0, false).await;
        n.call("Anna Rossi", "ended", SINCE, 7, false).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    });
    let posted = w.posted.borrow().clone();
    assert!(posted.iter().all(|p| p.0 == "https://ntfy.example"));
    assert_eq!(
        posted.into_iter().map(|p| p.1).collect::<Vec<_>>(),
        [
            text(json!({"topic": "relay-test", "title": "Teams", "message": "(new message)", "priority": 4, "tags": ["speech_balloon"]})),
            text(
                json!({"topic": "relay-test", "title": "Anna Rossi is calling", "message": "Teams call, ringing now", "priority": 5, "tags": ["telephone_receiver"], "sequence_id": "call-1-1790000000000"})
            ),
            text(
                json!({"topic": "relay-test", "title": "Call from Anna Rossi", "message": "Ended after 7 s", "priority": 2, "tags": ["telephone_receiver"], "sequence_id": "call-1-1790000000000"})
            ),
        ]
    );
}
