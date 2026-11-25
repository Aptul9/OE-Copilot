//! The light agent end to end on a fake network (test/agent/light-loop.test.ts, session-renewal.test.ts,
//! light-status.test.ts, native-calls.test.ts).

use super::*;
use crate::agent::calls::fake::{Handle, Started, spawner};
use crate::agent::commands::{Lane, Reads, run_light_commands};
use crate::agent::config::{Env, load};
use crate::agent::devices::StatusPageRow;
use crate::agent::http::Response;
use crate::agent::session::MicrosoftSession;
use crate::agent::testing::{Net, Seen, response, run, temp_dir};
use crate::agent::token::StoredToken;
use std::path::PathBuf;

const ME: &str = "me-oid";

fn mri(oid: &str) -> String {
    format!("8:orgid:{oid}")
}

fn iso(minutes_ago: i64) -> String {
    jiff::Timestamp::from_millisecond(now_ms() - minutes_ago * 60_000).expect("ts").strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn one_to_one(id: &str, oid: &str, text: &str, minutes_ago: i64, read: bool) -> Value {
    json!({ "id": id, "isOneOnOne": true, "isRead": read, "members": [{ "mri": mri(ME) }, { "mri": mri(oid) }], "lastMessage": { "id": minutes_ago.to_string(), "content": format!("<p>{text}</p>"), "originalArrivalTime": iso(minutes_ago) } })
}

/// What the light agent pushed: kind, title, body, chat.
#[derive(Default)]
struct FakeNotify {
    log: RefCell<Vec<(String, String, String, String)>>,
}

impl FakeNotify {
    fn of(&self, kind: &str) -> Vec<(String, String, String)> {
        self.log.borrow().iter().filter(|x| x.0 == kind).map(|x| (x.1.clone(), x.2.clone(), x.3.clone())).collect()
    }
}

impl Notify for FakeNotify {
    fn message<'a>(&'a self, title: &'a str, body: &'a str, chat: &'a str) -> LocalBoxFuture<'a, ()> {
        self.log.borrow_mut().push(("message".into(), title.into(), body.into(), chat.into()));
        Box::pin(async {})
    }
    fn alert<'a>(&'a self, title: &'a str, body: &'a str) -> LocalBoxFuture<'a, usize> {
        self.log.borrow_mut().push(("alert".into(), title.into(), body.into(), String::new()));
        Box::pin(async { 1 })
    }
    fn notice<'a>(&'a self, title: &'a str, body: &'a str) -> LocalBoxFuture<'a, usize> {
        self.log.borrow_mut().push(("notice".into(), title.into(), body.into(), String::new()));
        Box::pin(async { 1 })
    }
    fn call<'a>(&'a self, caller: &'a str, state: &'a str, since: i64, _: i64, _: bool) -> LocalBoxFuture<'a, usize> {
        self.log.borrow_mut().push(("call".into(), caller.into(), state.into(), since.to_string()));
        Box::pin(async { 1 })
    }
    fn device_count(&self) -> usize {
        1
    }
}

type Route = Rc<dyn Fn(&Seen) -> Option<Response>>;

struct World {
    l: Rc<Light>,
    net: Net,
    routes: Rc<RefCell<Vec<Route>>>,
    notify: Rc<FakeNotify>,
    dir: PathBuf,
    chats: Rows,
    feed: Rows,
    workers: Started,
}

type Rows = Rc<RefCell<Vec<Value>>>;

fn ok(body: Value) -> Option<Response> {
    Some(response(200, &[], &body.to_string()))
}

impl World {
    fn new(extra: &[(&str, &str)]) -> World {
        let dir = temp_dir();
        let mut env: Env = [("ACCOUNT", "7"), ("DOMAIN", "relay.example"), ("LIGHT_POLL", "60")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        env.insert("DB_PATH".into(), dir.join("messages.db").display().to_string());
        env.insert("APP_DB".into(), dir.join("app.db").display().to_string());
        for (k, v) in extra {
            env.insert(k.to_string(), v.to_string());
        }
        let config = load(&env).expect("config");
        let store = Store::open(&config.db_path).expect("store");
        store.set_state(key::ME, r#"{"name":"Me Myself","email":"","tenant":"","av":"","logo":""}"#);
        StoredToken {
            refresh_token: "rt".into(),
            client_id: "client".into(),
            tenant_id: "tenant".into(),
            region: "emea".into(),
            expires_on: now_s() + 20 * 3600,
        }
        .save(&config.light_token_file)
        .expect("token");
        let net = Net::new();
        let routes: Rc<RefCell<Vec<Route>>> = Rc::default();
        let (chats, feed): (Rows, Rows) = Default::default();
        let (r, c, f) = (routes.clone(), chats.clone(), feed.clone());
        net.respond(move |s| {
            let found = r.borrow().iter().rev().find_map(|route| route(s));
            if let Some(found) = found {
                return found;
            }
            let u = &s.url;
            if u.contains("/oauth2/v2.0/token") && s.text().contains("grant_type=refresh_token") {
                return response(200, &[], r#"{"access_token":"t","expires_in":3600}"#);
            }
            if u.contains("/api/csa/") {
                return response(200, &[], &json!({ "chats": *c.borrow() }).to_string());
            }
            if u.contains("/beta/users/fetch") {
                let people: Vec<Value> = s.json().as_array().cloned().unwrap_or_default().iter().map(|m| {
                    let m = m.as_str().unwrap_or("");
                    let last = m.split(':').next_back().unwrap_or("");
                    json!({ "mri": m, "displayName": format!("Person {last}"), "givenName": last })
                }).collect();
                return response(200, &[], &json!({ "value": people }).to_string());
            }
            if u.contains("getpresence") {
                let p: Vec<Value> = s.json().as_array().cloned().unwrap_or_default().iter().map(|x| json!({ "mri": x["mri"], "presence": { "availability": "Away", "activity": "Away" } })).collect();
                return response(200, &[], &Value::Array(p).to_string());
            }
            if u.contains("48%3Anotifications") {
                let messages: Vec<Value> = f.borrow().iter().map(|n| json!({ "id": n["mid"], "properties": { "activity": n["activity"].to_string(), "isread": if n["read"] == json!(false) { "false" } else { "true" } } })).collect();
                return response(200, &[], &json!({ "messages": messages }).to_string());
            }
            if u.contains("48%3Anotes") || u.contains("pageSize=50") {
                return response(200, &[], r#"{"messages":[]}"#);
            }
            if u.contains("presence.teams.microsoft.com/v1/me/") {
                return response(200, &[], "");
            }
            response(404, &[], "{}")
        });
        let notify = Rc::new(FakeNotify::default());
        let (spawn, workers) = spawner();
        let l = Light::new(Parts {
            token: Rc::new(Token::new(config.light_token_file.clone(), config.shared_broker.clone(), net.http())),
            config,
            store,
            notifier: notify.clone(),
            http: net.http(),
            calls_wanted: Box::new(|| true),
            status: None,
            spawn,
        });
        *l.me.borrow_mut() = (ME.into(), "Me Myself".into());
        World { l, net, routes, notify, dir, chats, feed, workers }
    }

    /// A route of the test, before the ones given earlier.
    fn on(&self, f: impl Fn(&Seen) -> Option<Response> + 'static) {
        self.routes.borrow_mut().push(Rc::new(f));
    }

    fn reads(&self) -> Reads {
        super::reads(&self.l)
    }

    fn requests(&self, part: &str) -> Vec<Seen> {
        self.net.requests().into_iter().filter(|s| s.url.contains(part)).collect()
    }

    fn chat_names(&self) -> Vec<(String, String)> {
        self.l.store.chats().into_iter().map(|c| (c.name, c.presence)).collect()
    }

    async fn commands(&self) {
        run_light_commands(&self.l, &self.reads(), Lane::All).await;
    }
}

/// Messages of a conversation, and 201 for whatever is sent.
fn chatsvc(messages: Value) -> impl Fn(&Seen) -> Option<Response> {
    move |s| {
        if !s.url.contains("/api/chatsvc/") || s.url.contains("48%3A") {
            return None;
        }
        if s.method == "GET" && s.url.contains("/messages?") {
            return ok(json!({ "messages": messages }));
        }
        Some(response(201, &[], r#"{"OriginalArrivalTime":1793700000000}"#))
    }
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

#[test]
fn keeps_its_own_presence_active_from_the_loop() {
    run(async {
        let w = World::new(&[]);
        let abort = Abort::new();
        let a = abort.clone();
        w.on(move |s| {
            if s.url.contains("/api/csa/") {
                a.abort();
            }
            None
        });
        run_light(w.l.clone(), abort).await;
        assert_eq!(w.requests("/v1/me/reportmyactivity/").len(), 1);
        assert_eq!(w.requests("/v1/me/endpoints/").len(), 1);
    });
}

#[test]
fn saves_the_list_with_names_presence_and_the_chat_with_yourself_and_pushes_only_after_the_first_read() {
    run(async {
        let w = World::new(&[]);
        *w.chats.borrow_mut() = vec![one_to_one("19:anna@unq.gbl.spaces", "anna", "hello", 30, true)];
        let time = iso(60);
        w.on(move |s| {
            if s.url.contains("48%3Anotes") { ok(json!({ "messages": [{ "id": "5", "content": "<p>todo</p>", "originalarrivaltime": time }] })) } else { None }
        });
        assert!(read_list(&w.l).await);
        assert_eq!(w.chat_names(), [("Person anna".into(), "away".into()), ("Me Myself (You)".into(), String::new())]);
        assert_eq!(w.l.store.get_state(key::PRESENCE), "away");
        assert!(w.notify.of("message").is_empty());
        *w.chats.borrow_mut() = vec![one_to_one("19:anna@unq.gbl.spaces", "anna", "are you there?", 0, false)];
        w.on(chatsvc(json!([])));
        assert!(read_list(&w.l).await);
        assert_eq!(w.notify.of("message"), [("Person anna".into(), "are you there?".into(), "Person anna".into())]);
        assert_eq!(w.requests("fetchShortProfile").len(), 1);
        settle().await;
    });
}

#[test]
fn names_a_chat_neither_look_up_knows_from_its_messages_read_once_per_last_message() {
    run(async {
        let w = World::new(&[]);
        let mut chat = one_to_one("19:ext@unq.gbl.spaces", "ext", "mine", 5, true);
        chat["isLastMessageFromMe"] = json!(true);
        *w.chats.borrow_mut() = vec![chat.clone()];
        let named = Rc::new(Cell::new(false));
        let n = named.clone();
        w.on(move |s| {
            if s.url.contains("/beta/users/fetch") {
                return ok(json!({ "value": [] }));
            }
            if s.url.contains("19%3Aext%40unq.gbl.spaces") && s.url.contains("pageSize=50") {
                let messages = if n.get() {
                    json!([{ "from": format!("https://x/v1/users/ME/contacts/{}", mri("ext")), "imdisplayname": "Esterno Persona" }])
                } else {
                    json!([])
                };
                return ok(json!({ "messages": messages }));
            }
            None
        });
        read_list(&w.l).await;
        read_list(&w.l).await;
        assert_eq!(w.requests("19%3Aext%40unq.gbl.spaces").len(), 1);
        assert!(w.chat_names().is_empty());
        named.set(true);
        chat["lastMessage"]["id"] = json!("1");
        *w.chats.borrow_mut() = vec![chat];
        read_list(&w.l).await;
        assert_eq!(w.requests("19%3Aext%40unq.gbl.spaces").len(), 2);
        assert_eq!(w.chat_names().iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Esterno Persona"]);
    });
}

#[test]
fn opens_a_chat_through_chatsvc_and_marks_it_read() {
    run(async {
        let w = World::new(&[]);
        *w.chats.borrow_mut() = vec![one_to_one("19:anna@unq.gbl.spaces", "anna", "hello", 1, true)];
        read_list(&w.l).await;
        w.on(chatsvc(json!([{ "id": "1793690000000", "messagetype": "RichText/Html", "content": "<p>hello</p>", "imdisplayname": "Person anna", "from": format!("https://x/contacts/{}", mri("anna")) }])));
        let id = w.l.store.enqueue("open", "Person anna", "");
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("done"));
        assert_eq!(w.l.store.messages("Person anna").iter().map(|m| m["text"].clone()).collect::<Vec<_>>(), [json!("hello")]);
        assert_eq!(w.l.store.get_state(key::ACTIVE_CHAT), "Person anna");
        settle().await;
        assert!(w.net.requests().iter().any(|s| s.url.contains("consumptionhorizon")));
        // a chat the list does not have, with its reason
        let id = w.l.store.enqueue("open", "Nobody", "");
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("failed"));
        assert_eq!(w.l.store.get_state(&crate::state::cmd_result_key(id)), r#"{"reason":"not-listed"}"#);
        // what needs the Teams page is refused
        let id = w.l.store.enqueue("download", "x", "{}");
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("failed"));
    });
}

#[test]
fn an_open_chat_marks_its_unread_activity_items_read_on_teams() {
    run(async {
        let w = World::new(&[]);
        *w.chats.borrow_mut() = vec![one_to_one("19:anna@unq.gbl.spaces", "anna", "hello", 1, true)];
        read_list(&w.l).await;
        let item = |id: i64, mid: &str, thread: &str, read: bool| json!({ "mid": mid, "read": read, "activity": { "activityId": id, "activityType": "mentionInChat", "activityTimestamp": iso(1), "sourceThreadId": thread, "sourceUserImDisplayName": "Person anna" } });
        *w.feed.borrow_mut() =
            vec![item(1, "m1", "19:anna@unq.gbl.spaces", false), item(2, "m2", "19:anna@unq.gbl.spaces", true), item(3, "m3", "19:other@thread.v2", false)];
        read_feed(&w.l).await;
        w.on(chatsvc(json!([{ "id": "1793690000000", "messagetype": "RichText/Html", "content": "<p>hello</p>", "imdisplayname": "Person anna", "from": format!("https://x/contacts/{}", mri("anna")) }])));
        let id = w.l.store.enqueue("open", "Person anna", "");
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("done"));
        settle().await;
        let marks: Vec<(String, Value)> =
            w.net.requests().into_iter().filter(|s| s.method == "PUT" && s.url.contains("name=isread")).map(|s| (s.url.clone(), s.json())).collect();
        assert_eq!(marks.len(), 1);
        assert!(marks[0].0.contains("/conversations/48%3Anotifications/messages/m1/properties?name=isread"), "{}", marks[0].0);
        assert_eq!(marks[0].1, json!({ "isread": "true" }));
        assert_eq!(w.l.store.unread_activity(), ["3"]);
    });
}

#[test]
fn saves_the_image_of_a_message_with_the_chatsvc_token_once() {
    // AMS, and its Teams gateway
    for ams in
        ["https://eu-api.asm.skype.com/v1/objects/0-weu-d2-abc/views/imgo", "https://it-prod.asyncgw.teams.microsoft.com/v1/objects/0-itn-d3-abc/views/imgo"]
    {
        saves_the_image_from(ams);
    }
}

fn saves_the_image_from(ams: &'static str) {
    run(async {
        let w = World::new(&[]);
        *w.chats.borrow_mut() = vec![one_to_one("19:anna@unq.gbl.spaces", "anna", "hello", 1, true)];
        read_list(&w.l).await;
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0];
        jpeg.extend(std::iter::repeat_n(1u8, 2000));
        let image = jpeg.clone();
        w.on(move |s| {
            (s.url == ams).then(|| {
                if s.header("Authorization") == Some("Bearer t") {
                    Response { status: 200, headers: vec![("Content-Type".into(), "image/jpeg".into())], body: image.clone() }
                } else {
                    response(401, &[], "")
                }
            })
        });
        let content = format!(r#"<p><img src="{ams}" width="833" height="250" itemtype="http://schema.skype.com/AMSImage"></p>"#);
        w.on(chatsvc(json!([{ "id": "1793690000000", "messagetype": "RichText/Html", "content": content, "imdisplayname": "Person anna", "from": format!("https://x/contacts/{}", mri("anna")) }])));
        for _ in 0..2 {
            let id = w.l.store.enqueue("open", "Person anna", "");
            w.commands().await;
            assert_eq!(w.l.store.command_status(id).as_deref(), Some("done"));
        }
        let im = w.l.store.messages("Person anna")[0]["images"][0].clone();
        assert_eq!((im["w"].clone(), im["h"].clone(), im.get("url").cloned()), (json!(833), json!(250), None));
        let f = im["f"].as_str().expect("file").to_string();
        assert!(regex::Regex::new(r"^[0-9a-f]{16}\.jpg$").expect("fixed").is_match(&f), "{f}");
        assert_eq!(std::fs::read(w.dir.join("media").join(&f)).expect("image"), jpeg);
        assert_eq!(w.net.requests().iter().filter(|s| s.url == ams).count(), 1);
        settle().await;
    });
}

#[test]
fn sends_through_chatsvc_to_the_conversation_of_the_chat_and_tags_people_with_their_mri() {
    run(async {
        let w = World::new(&[]);
        let group = json!({ "id": "19:g@thread.v2", "title": "Team", "members": [{ "mri": mri(ME) }, { "mri": mri("anna") }, { "mri": mri("marco") }], "lastMessage": { "id": "1", "content": "x", "originalArrivalTime": iso(1) } });
        *w.chats.borrow_mut() = vec![one_to_one("19:anna@unq.gbl.spaces", "anna", "hello", 1, true), group];
        read_list(&w.l).await;
        w.on(chatsvc(json!([])));
        let id = w.l.store.enqueue("send", "Person anna", "see you");
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("done"));
        let post = w.net.requests().into_iter().find(|s| s.method == "POST" && s.url.contains("/api/chatsvc/")).expect("post");
        assert!(post.url.contains(&crate::js::encode_uri_component("19:anna@unq.gbl.spaces")));
        assert_eq!(post.json()["content"], "<p>see you</p>");
        w.net.clear();
        let parts = json!({ "parts": [{ "text": "hi " }, { "mention": "Person anna" }, { "text": " ok?" }] }).to_string();
        let id = w.l.store.enqueue("sendmentions", "Team", &parts);
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("done"));
        let body = w.net.requests().into_iter().find(|s| s.method == "POST" && s.url.contains("/api/chatsvc/")).expect("post").json();
        assert_eq!(body["content"], r#"<p>hi <span itemtype="http://schema.skype.com/Mention" itemscope="" itemid="0">Person anna</span> ok?</p>"#);
        let mentions: Value = serde_json::from_str(body["properties"]["mentions"].as_str().expect("mentions")).expect("json");
        assert_eq!(mentions, json!([{ "itemid": 0, "mri": mri("anna"), "mentionType": "person", "displayName": "Person anna" }]));
        settle().await;
    });
}

fn call(id: i64, minutes_ago: i64, caller: &str, read: bool) -> Value {
    json!({ "activity": { "activityId": id, "activityType": "call", "activitySubtype": "missedCall", "activityTimestamp": iso(minutes_ago), "sourceUserImDisplayName": caller }, "read": read })
}

#[test]
fn pushes_a_missed_call_once_after_the_first_feed_read() {
    run(async {
        let w = World::new(&[]);
        *w.feed.borrow_mut() = vec![call(1, 90, "Old Caller", true)];
        read_feed(&w.l).await;
        assert!(w.notify.of("notice").is_empty());
        w.feed.borrow_mut().insert(0, call(2, 0, "New Caller", false));
        read_feed(&w.l).await;
        read_feed(&w.l).await;
        assert_eq!(w.notify.of("notice").iter().map(|n| n.0.as_str()).collect::<Vec<_>>(), ["Missed call from New Caller"]);
        assert_eq!(w.l.store.missed_calls().iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["2", "1"]);
    });
}

#[test]
fn at_its_start_pushes_the_calls_that_came_while_the_account_changed_mode_not_those_of_weeks_ago() {
    run(async {
        let w = World::new(&[]);
        *w.feed.borrow_mut() = vec![call(3, 2, "Caller 3", true), call(2, 30 * 24 * 60, "Caller 2", true), call(1, 60, "Caller 1", true)];
        w.l.store.set_state("calls_told", r#"["1"]"#);
        read_feed(&w.l).await;
        assert_eq!(w.notify.of("notice").iter().map(|n| n.0.as_str()).collect::<Vec<_>>(), ["Missed call from Caller 3"]);
    });
}

#[test]
fn tells_the_web_app_when_microsoft_refused_the_sign_in() {
    run(async {
        let w = World::new(&[]);
        w.l.store.set_state(key::LIGHT_TOKEN, r#"{"at":1,"expiresOn":2,"refused":false}"#);
        w.on(|s| s.url.contains("/oauth2/v2.0/token").then(|| response(400, &[], r#"{"error":"invalid_grant"}"#)));
        assert!(!read_list(&w.l).await);
        assert!(w.l.signed_out.get());
        assert!(w.l.sign_in().refused);
    });
}

/// Starts the calling worker of the slot on a fake spawn: discovery answered, the worker handle returned.
async fn worker(w: &World) -> Rc<Handle> {
    w.on(|s| s.url.contains("/authsvc/").then(|| response(200, &[], r#"{"tokens":{"skypeToken":"test"},"regionGtms":{}}"#)));
    sync_native_calls(&w.l, true).await.expect("calls");
    w.workers.borrow().last().expect("worker").2.clone()
}

#[test]
fn the_calling_worker_starts_isolated_records_the_call_and_answers_commands() {
    run(async {
        let w = World::new(&[("NATIVE_CALLS", "1"), ("CALLING_HOST", "/opt/calling-host.mjs")]);
        w.l.list.borrow_mut().members.insert("thread".into(), vec![mri("peer")]);
        w.l.list.borrow_mut().names.insert("thread".into(), "Peer".into());
        let h = worker(&w).await;
        let (script, env) = (w.workers.borrow()[0].0.clone(), w.workers.borrow()[0].1.clone());
        assert_eq!(script, "/opt/calling-host.mjs");
        assert_eq!(
            (env["ACCOUNT"].as_str(), env["DOMAIN"].as_str(), env["CALL_AUDIO_URL"].as_str(), env["CALLING_OID"].as_str()),
            ("7", "relay.example", "wss://relay.example/api/call/audio/socket?a=7", ME)
        );
        h.say.send(json!({ "stage": "worker-ready" })).expect("say");
        h.say.send(json!({ "call": { "caller": mri("peer"), "since": 123, "seen": 124, "ringing": true, "active": false } })).expect("say");
        settle().await;
        assert_eq!(w.l.store.get_state(key::CALL_AUDIO), "1");
        assert_eq!(serde_json::from_str::<Value>(&w.l.store.get_state(key::CALL)).expect("call")["caller"], "Peer");
        assert_eq!(w.notify.of("call"), [("Peer".into(), "ringing".into(), "123".into())]);
        let calls = w.l.calls.borrow().clone().expect("calls");
        let reply = h.clone();
        tokio::task::spawn_local(async move {
            settle().await;
            let id = reply.worker.sent.borrow()[0]["id"].clone();
            reply.say.send(json!({ "command": id, "ok": true })).expect("say");
        });
        let mut args = serde_json::Map::new();
        args.insert("since".into(), json!(123));
        assert!(calls.command("answer", args).await);
        assert_eq!(h.worker.sent.borrow()[0], json!({ "id": 1, "op": "answer", "since": 123 }));
        h.say
            .send(json!({ "call": { "caller": mri("peer"), "since": 123, "seen": 130, "ringing": false, "active": false, "ended": true, "answered": true } }))
            .expect("say");
        h.say
            .send(json!({ "call": { "caller": mri("peer"), "since": 123, "seen": 131, "ringing": false, "active": false, "ended": true, "answered": true } }))
            .expect("say");
        settle().await;
        assert_eq!(crate::slotdb::SlotReader::open(&w.l.config.db_path).expect("db").call_log().len(), 1);
        assert_eq!(w.notify.of("call").len(), 2);
        let token = std::fs::read_to_string(w.dir.join("calling-audio-token")).expect("token");
        assert_eq!(token.len(), 64);
        calls.stop();
        assert_eq!(*h.worker.kills.borrow(), [libc::SIGTERM]);
        h.exit(Some(0), None);
        settle().await;
        assert_eq!(w.l.store.get_state(key::CALL_AUDIO), "0");
    });
}

#[test]
fn the_call_command_goes_to_the_one_other_person_of_the_chat() {
    run(async {
        let w = World::new(&[("NATIVE_CALLS", "1")]);
        w.l.list.borrow_mut().ids.insert("Other".into(), "thread".into());
        w.l.list.borrow_mut().members.insert("thread".into(), vec![mri("other")]);
        let h = worker(&w).await;
        let reply = h.clone();
        tokio::task::spawn_local(async move {
            settle().await;
            let id = reply.worker.sent.borrow()[0]["id"].clone();
            reply.say.send(json!({ "command": id, "ok": true })).expect("say");
        });
        let id = w.l.store.enqueue("call", "Other", "");
        w.commands().await;
        assert_eq!(w.l.store.command_status(id).as_deref(), Some("done"));
        assert_eq!(h.worker.sent.borrow()[0], json!({ "id": 1, "op": "call", "thread": "thread", "peer": mri("other"), "name": "Other" }));
        // a call control only in its own lane, an open only in the other
        let answer = w.l.store.enqueue("answer", "Peer", r#"{"since":1}"#);
        let open = w.l.store.enqueue("open", "Nobody", "");
        run_light_commands(&w.l, &w.reads(), Lane::Other).await;
        assert_eq!((w.l.store.command_status(answer).as_deref(), w.l.store.command_status(open).as_deref()), (Some("pending"), Some("failed")));
    });
}

#[test]
fn starts_the_worker_when_calls_are_on_and_stops_it_when_switched_off() {
    run(async {
        let w = World::new(&[("NATIVE_CALLS", "1")]);
        let h = worker(&w).await;
        sync_native_calls(&w.l, true).await.expect("calls");
        assert_eq!(w.workers.borrow().len(), 1);
        sync_native_calls(&w.l, false).await.expect("calls");
        assert_eq!(*h.worker.kills.borrow(), [libc::SIGTERM]);
        assert!(w.l.calls.borrow().is_none());
    });
}

#[test]
fn on_a_shared_account_the_worker_starts_only_with_a_token_of_a_teams_app() {
    run(async {
        let w = World::new(&[
            ("NATIVE_CALLS", "1"),
            ("SHARED_BROKER_URL", "http://agent"),
            ("SHARED_BROKER_KEY", "k"),
            ("SHARED_BROKER_ACCOUNT", "me@example.com"),
            ("SHARED_BROKER_REGION", "emea"),
        ]);
        let app = Rc::new(RefCell::new("27922004-5251-4030-b22d-91ecd9a37ea4".to_string()));
        let a = app.clone();
        w.on(move |s| {
            s.url.contains("/api/token-resource").then(|| {
                let token = format!("x.{}.x", crate::js::base64_url(json!({ "appid": *a.borrow(), "exp": 9_999_999_999i64 }).to_string().as_bytes()));
                response(200, &[], &json!({ "token": token }).to_string())
            })
        });
        w.on(|s| s.url.contains("/authsvc/").then(|| response(200, &[], r#"{"tokens":{"skypeToken":"test"},"regionGtms":{}}"#)));
        sync_native_calls(&w.l, true).await.expect("calls");
        sync_native_calls(&w.l, true).await.expect("calls");
        assert!(w.workers.borrow().is_empty());
        *app.borrow_mut() = "1fec8e78-bce4-4aaf-ab1b-5451cc387264".into();
        w.l.token.renew_all();
        sync_native_calls(&w.l, true).await.expect("calls");
        assert_eq!(w.workers.borrow().len(), 1);
        assert!(w.l.calls.borrow().is_some());
    });
}

#[test]
fn a_token_refresh_the_worker_asks_for_that_fails_ends_it() {
    run(async {
        let w = World::new(&[("NATIVE_CALLS", "1")]);
        let h = worker(&w).await;
        w.on(|s| s.url.contains("/authsvc/").then(|| response(403, &[], "no")));
        h.say.send(json!({ "refreshToken": true })).expect("say");
        for _ in 0..50 {
            if !h.worker.kills.borrow().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(*h.worker.kills.borrow(), [libc::SIGTERM]);
    });
}

#[test]
fn a_worker_that_does_not_answer_a_command_in_30_s_is_ended() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().expect("runtime");
    tokio::task::LocalSet::new().block_on(&rt, async {
        let w = World::new(&[("NATIVE_CALLS", "1")]);
        let h = worker(&w).await;
        let calls = w.l.calls.borrow().clone().expect("calls");
        assert!(!calls.command("answer", serde_json::Map::new()).await);
        assert_eq!(*h.worker.kills.borrow(), [libc::SIGTERM]);
    });
}

#[test]
fn starts_no_worker_when_stopped_while_it_cleaned_up_the_last_registration() {
    run(async {
        let w = World::new(&[("NATIVE_CALLS", "1")]);
        std::fs::write(
            w.dir.join("owned-registration.json"),
            r#"{"endpoint":"0f8fad5b-d9cb-469f-a165-70867728950e","registrar":"https://teams.microsoft.com/registrar/prod/v2/registrations"}"#,
        )
        .expect("marker");
        let calls = NativeCalls::new(&w.l, w.l.spawn.clone());
        let c = calls.clone();
        w.on(move |s| {
            if s.url.contains("/registrar/") {
                c.stop();
                return Some(response(204, &[], ""));
            }
            s.url.contains("/authsvc/").then(|| response(200, &[], r#"{"tokens":{"skypeToken":"test"},"regionGtms":{}}"#))
        });
        calls.start().await.expect("started");
        assert!(w.workers.borrow().is_empty());
    });
}

// The sign-in renewed over HTTP (test/agent/session-renewal.test.ts)

fn session_world(expires_on: i64, refused: bool) -> World {
    let w = World::new(&[]);
    let file = w.l.config.light_token_file.clone();
    StoredToken { refresh_token: "old".into(), client_id: "client".into(), tenant_id: "tenant".into(), region: "emea".into(), expires_on }
        .save(&file)
        .expect("token");
    let cookie = json!({ "name": "ESTSAUTHPERSISTENT", "value": "cookie", "domain": ".login.microsoftonline.com", "path": "/", "expires": now_s() + 100_000 });
    MicrosoftSession::save(
        &w.dir.join("session.json"),
        &MicrosoftSession { user_agent: "ua".into(), login_hint: "me@example.com".into(), cookies: vec![cookie.as_object().cloned().expect("o")] },
    )
    .expect("session");
    w.l.store.set_state(key::LIGHT_TOKEN, &json!({ "at": 1, "expiresOn": expires_on, "refused": refused, "session": true }).to_string());
    w.l.signed_out.set(refused);
    w
}

fn granted(w: &World) {
    w.on(|s| {
        if s.url.contains("/authorize") {
            let state = url::Url::parse(&s.url).ok()?.query_pairs().find(|(k, _)| k == "state")?.1.to_string();
            return Some(response(302, &[("Location", &format!("https://teams.cloud.microsoft/v2/authv2?code=c&state={state}"))], ""));
        }
        (s.url.contains("/oauth2/v2.0/token") && s.text().contains("authorization_code")).then(|| response(200, &[], r#"{"refresh_token":"new"}"#))
    });
}

fn login_requests(w: &World) -> usize {
    w.net.requests().iter().filter(|s| s.url.contains("/authorize") || (s.url.contains("/token") && s.text().contains("authorization_code"))).count()
}

#[test]
fn renews_the_stored_token_of_a_session_over_http() {
    run(async {
        let w = session_world(2000, false);
        granted(&w);
        let mut renewed = Renewed::default();
        renew_sign_in(&w.l, &mut renewed).await.expect("renewed");
        let stored = StoredToken::load(&w.l.config.light_token_file).expect("token");
        assert_eq!(stored.refresh_token, "new");
        assert!((stored.expires_on - (now_s() + 86400)).abs() <= 2);
        let s = w.l.sign_in();
        assert_eq!((s.session, s.refused, s.expires_on as i64), (Some(true), false, stored.expires_on));
        renew_sign_in(&w.l, &mut renewed).await.expect("renewed");
        assert_eq!(login_requests(&w), 2);
    });
}

#[test]
fn replays_authorize_when_a_still_young_token_was_marked_refused() {
    run(async {
        let w = session_world(now_s() + 80_000, true);
        granted(&w);
        renew_sign_in(&w.l, &mut Renewed::default()).await.expect("renewed");
        assert_eq!(login_requests(&w), 2);
        assert!(!w.l.signed_out.get());
    });
}

#[test]
fn backs_off_a_transient_failure_without_marking_the_sign_in_refused() {
    run(async {
        let w = session_world(2000, false);
        w.on(|s| s.url.contains("/authorize").then(|| response(503, &[], "")));
        let mut renewed = Renewed::default();
        renew_sign_in(&w.l, &mut renewed).await.expect("renewed");
        renew_sign_in(&w.l, &mut renewed).await.expect("renewed");
        assert_eq!(w.requests("/authorize").len(), 1);
        assert!(!w.l.sign_in().refused);
    });
}

#[test]
fn asks_for_the_browser_for_a_missing_session_or_an_mcas_form_keeping_the_token() {
    run(async {
        let w = session_world(2000, false);
        std::fs::remove_file(w.dir.join("session.json")).expect("removed");
        renew_sign_in(&w.l, &mut Renewed::default()).await.expect("renewed");
        let s = w.l.sign_in();
        assert_eq!((s.refused, s.session, s.browser, w.l.signed_out.get()), (false, Some(true), Some(true), false));

        let w = session_world(2000, false);
        w.on(|s| {
            s.url.contains("/authorize").then(|| {
                response(200, &[], r#"<form method="post" action="https://tenant.access.mcas.ms/aad_login"><input name="id_token" value="private"></form>"#)
            })
        });
        let mut renewed = Renewed::default();
        renew_sign_in(&w.l, &mut renewed).await.expect("renewed");
        renew_sign_in(&w.l, &mut renewed).await.expect("renewed");
        assert_eq!(w.requests("/authorize").len(), 1);
        assert_eq!((w.l.sign_in().browser, w.l.sign_in().refused), (Some(true), false));
        assert_eq!(StoredToken::load(&w.l.config.light_token_file).expect("token").refresh_token, "old");

        // an MCAS session goes through its own client, never the direct authorize
        let w = session_world(2000, false);
        w.l.store.set_state(key::LIGHT_TOKEN, r#"{"at":1,"expiresOn":2000,"refused":false,"session":true,"mcas":true}"#);
        renew_sign_in(&w.l, &mut Renewed::default()).await.expect("renewed");
        assert!(w.requests("/authorize").is_empty());
        assert_eq!(w.l.sign_in().browser, Some(true));
    });
}

// The status of the people of the status pages (test/agent/light-status.test.ts)

const T0: i64 = 1_793_700_000;

fn at(s: i64) -> i64 {
    (T0 + s) * 1000
}

fn collector(w: &World, pages: Rc<RefCell<Option<Vec<StatusPageRow>>>>) -> (StatusCollector, PathBuf) {
    let file = w.dir.join("status.db");
    let p = pages.clone();
    (StatusCollector::new(Box::new(move || p.borrow().clone()), file.clone(), 7, 180), file)
}

fn page(slug: &str, slot: i64, name: &str, ends: i64) -> StatusPageRow {
    StatusPageRow { slug: slug.into(), slot, name: name.into(), ends }
}

async fn poll(w: &World, c: &StatusCollector, now_ms: i64) {
    let people = w.l.people.borrow().clone();
    let list = w.l.list.borrow().clone();
    c.poll(Known { people: &people, ids: &list.ids, members: &list.members }, &w.l.api, &w.l.store, now_ms).await;
}

fn stretches(file: &std::path::Path, name: &str) -> Vec<(i64, i64, String)> {
    crate::status::db::stretches_of(&crate::status::db::open_status_db(file).expect("db"), name, 0, T0 + 99_999).expect("rows")
}

#[test]
fn extends_the_stretch_on_the_same_status_and_opens_one_on_a_change_and_after_a_gap() {
    run(async {
        let w = World::new(&[]);
        w.l.people.borrow_mut().insert(mri("cam"), Person { display_name: "camilla marini".into(), given_name: "Camilla".into() });
        let availability = Rc::new(RefCell::new("Busy".to_string()));
        let a = availability.clone();
        w.on(move |s| {
            s.url
                .contains("getpresence")
                .then(|| response(200, &[], &json!([{ "mri": mri("cam"), "presence": { "availability": *a.borrow(), "activity": "x" } }]).to_string()))
        });
        let (c, file) = collector(&w, Rc::new(RefCell::new(Some(vec![page("kamila", 7, "Camilla Marini", 0)]))));
        poll(&w, &c, at(0)).await;
        poll(&w, &c, at(60)).await;
        *availability.borrow_mut() = "Available".into();
        poll(&w, &c, at(120)).await;
        poll(&w, &c, at(500)).await;
        assert_eq!(
            stretches(&file, "Camilla Marini"),
            [(T0, T0 + 60, "busy".into()), (T0 + 120, T0 + 120, "available".into()), (T0 + 500, T0 + 500, "available".into())]
        );
        assert_eq!(w.l.store.get_state(key::STATUS_NAMES), json!({ "Camilla Marini": mri("cam") }).to_string());
    });
}

#[test]
fn finds_a_person_as_the_other_member_of_the_1_1_chat_of_that_name_and_polls_only_its_pages_not_ended() {
    run(async {
        let w = World::new(&[]);
        w.l.list.borrow_mut().ids.insert("Damiano Bogni".into(), "19:dam@unq.gbl.spaces".into());
        w.l.list.borrow_mut().members.insert("19:dam@unq.gbl.spaces".into(), vec![mri("dam")]);
        let pages =
            vec![page("damian", 7, "Damiano Bogni", 0), page("ghost", 7, "Nobody Here", 0), page("old", 7, "Old One", T0 - 1), page("other", 2, "Marco", 0)];
        let (c, _) = collector(&w, Rc::new(RefCell::new(Some(pages))));
        poll(&w, &c, at(0)).await;
        let asked = w.requests("getpresence");
        assert_eq!(asked.last().expect("asked").json(), json!([{ "mri": mri("dam") }]));
        assert_eq!(w.l.store.get_state(key::STATUS_NAMES), json!({ "Damiano Bogni": mri("dam"), "Nobody Here": "" }).to_string());
    });
}

#[test]
fn reads_the_pages_once_a_minute_and_deletes_the_rows_no_page_keeps_once_an_hour() {
    run(async {
        let w = World::new(&[]);
        let day = 86400;
        let pages = Rc::new(RefCell::new(Some(vec![
            page("kamila", 7, "Camilla Marini", 0),
            page("old", 7, "Ended Long Ago", T0 - 8 * day),
            page("recent", 2, "Ended Recently", T0 - 2 * day),
        ])));
        let (c, file) = collector(&w, pages.clone());
        let db = crate::status::db::open_status_db(&file).expect("db");
        for name in ["Ended Long Ago", "Ended Recently", "Gone Page"] {
            crate::status::db::record_statuses(&db, &[(name.into(), "x".into(), "away".into())], T0 - 9 * day, 180).expect("rows");
        }
        let names = || -> Vec<String> {
            db.prepare("SELECT DISTINCT name FROM presence_log ORDER BY name")
                .expect("sql")
                .query_map([], |r| r.get(0))
                .expect("rows")
                .map(Result::unwrap)
                .collect()
        };
        poll(&w, &c, at(0)).await;
        assert_eq!(names(), ["Ended Recently"]);
        *pages.borrow_mut() = Some(vec![]);
        poll(&w, &c, at(30)).await;
        poll(&w, &c, at(120)).await;
        assert_eq!(names(), ["Ended Recently"]);
        poll(&w, &c, at(3700)).await;
        assert!(names().is_empty());
        // nothing deleted while app.db cannot be read
        let (c, file) = collector(&w, Rc::new(RefCell::new(None)));
        crate::status::db::record_statuses(
            &crate::status::db::open_status_db(&file).expect("db"),
            &[("Camilla".into(), "x".into(), "away".into())],
            T0 - 100,
            180,
        )
        .expect("row");
        poll(&w, &c, at(0)).await;
        assert_eq!(stretches(&file, "Camilla").len(), 1);
    });
}
