//! The loop of a light slot (src/agent/light/loop.ts): the sign-in kept alive, the chat list and the Activity feed read
//! every poll, new messages, missed calls and mentions pushed, the health row written, the commands of the web app run.

use super::api::{Api, Person, TeamsAuth};
use super::calls::{NativeCalls, Spawn};
use super::chatsvc::ChatSvc;
use super::commands::{Lane, Reads, mark_read, run_light_commands, save_chat};
use super::config::Config;
use super::detector::Detector;
use super::host::Abort;
use super::list::{ListInput, light_list, listed_chats, others_of, presence_word, unnamed_chats, unnamed_people};
use super::mcas::{McasSession, mcas_session_file};
use super::media::Media;
use super::session::{SessionAuth, session_file};
use super::status::{Known, StatusCollector};
use super::store::{CHAT_LIMIT, ChatEntry, Store};
use super::token::{CHATSVCAGG, IC3, PRESENCE, SPACES, Token};
use super::{Fail, R, cut, info, now_ms, now_s, text_of, warn};
use crate::push::notify::keep_slot;
use crate::state::{Identity, LightSignIn, key};
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

/// A sign-in that still needs owner interaction after this long pushes an alert (s).
const SIGN_IN_AFTER: i64 = 30 * 60;
/// The access tokens are redeemed once more this long before the refresh token ends (ms).
const RENEW_BEFORE_MS: i64 = 10 * 60_000;
const HEALTH_EVERY_MS: i64 = 10_000;
const VIEWING_EVERY_MS: i64 = 5_000;
const PRESENCE_EVERY_MS: i64 = 4 * 60_000;
/// Chats whose messages are read for a name, in one read of the list.
const AUTHOR_READS: usize = 3;
/// At its first read the feed's items older than this count as told (ms).
const TOLD_BEFORE_START_MS: i64 = 10 * 60_000;
/// Seconds without a mark of the app before the chat no longer counts as shown (PARK_AFTER).
const PARK_AFTER: f64 = 30.0;
/// The apps whose tokens Teams calling takes: Teams web and Teams desktop/mobile.
const TEAMS_APPS: [&str; 2] = ["5e3ce6c0-2b1f-4285-8d4b-75ee78787346", "1fec8e78-bce4-4aaf-ab1b-5451cc387264"];

/// The notifications a light agent sends (Notify of src/agent/push/notifier.ts).
pub trait Notify {
    fn message<'a>(&'a self, title: &'a str, body: &'a str, chat: &'a str) -> LocalBoxFuture<'a, ()>;
    fn alert<'a>(&'a self, title: &'a str, body: &'a str) -> LocalBoxFuture<'a, usize>;
    fn notice<'a>(&'a self, title: &'a str, body: &'a str) -> LocalBoxFuture<'a, usize>;
    fn call<'a>(&'a self, caller: &'a str, state: &'a str, since: i64, seconds: i64, answered: bool) -> LocalBoxFuture<'a, usize>;
    fn device_count(&self) -> usize;
    /// a missed call the check of an account checked every N hours found (missedCall)
    fn missed_call<'a>(&'a self, _caller: &'a str, _time: &'a str) -> LocalBoxFuture<'a, usize> {
        Box::pin(async { 0 })
    }
}

impl Notify for crate::push::Notifier {
    fn message<'a>(&'a self, title: &'a str, body: &'a str, chat: &'a str) -> LocalBoxFuture<'a, ()> {
        Box::pin(crate::push::Notifier::message(self, title, body, chat))
    }
    fn alert<'a>(&'a self, title: &'a str, body: &'a str) -> LocalBoxFuture<'a, usize> {
        Box::pin(crate::push::Notifier::alert(self, title, body, ""))
    }
    fn notice<'a>(&'a self, title: &'a str, body: &'a str) -> LocalBoxFuture<'a, usize> {
        Box::pin(crate::push::Notifier::notice(self, title, body, ""))
    }
    fn call<'a>(&'a self, caller: &'a str, state: &'a str, since: i64, seconds: i64, answered: bool) -> LocalBoxFuture<'a, usize> {
        Box::pin(crate::push::Notifier::call(self, caller, state, since, seconds, answered))
    }
    fn device_count(&self) -> usize {
        crate::push::Notifier::device_count(self)
    }
    fn missed_call<'a>(&'a self, caller: &'a str, time: &'a str) -> LocalBoxFuture<'a, usize> {
        Box::pin(crate::push::Notifier::missed_call(self, caller, time))
    }
}

/// The last chat list read: chat name to conversation id, conversation id to chat name, the other people of each.
#[derive(Clone, Default)]
pub struct ChatList {
    pub ids: IndexMap<String, String>,
    pub names: HashMap<String, String>,
    pub members: IndexMap<String, Vec<String>>,
}

/// What a light agent works with (Light of src/agent/light/context.ts).
pub struct Light {
    pub config: Config,
    pub store: Store,
    pub notifier: Rc<dyn Notify>,
    pub media: Media,
    pub detector: RefCell<Detector>,
    pub api: Api,
    pub token: Rc<Token>,
    pub http: super::http::Http,
    pub chatsvc: ChatSvc,
    /// you: object id and name, from the access token
    pub me: RefCell<(String, String)>,
    pub list: RefCell<ChatList>,
    /// names of people, kept for the life of the slot; the conversations whose messages were read for a name
    pub people: RefCell<IndexMap<String, Person>>,
    pub authors_read: RefCell<HashMap<String, String>>,
    /// conversations with a mention of you the feed shows unread
    pub mentioned: RefCell<HashSet<String>>,
    /// when the next chat list and Activity reads are due (ms), 0: at once
    pub due_list: Cell<i64>,
    pub due_activity: Cell<i64>,
    /// the last chat list read that worked (ms), the last error, Microsoft refused the sign-in
    pub scan_at: Cell<i64>,
    pub error: RefCell<String>,
    pub signed_out: Cell<bool>,
    pub started_at: i64,
    pub feed_read: Cell<bool>,
    pub calls: RefCell<Option<Rc<NativeCalls>>>,
    /// calls switched on for this account in Settings
    pub calls_wanted: Box<dyn Fn() -> bool>,
    pub status: Option<StatusCollector>,
    pub spawn: Spawn,
    /// told once that calls are off on its shared account
    no_calls_told: Cell<bool>,
}

pub struct Parts {
    pub config: Config,
    pub store: Store,
    pub notifier: Rc<dyn Notify>,
    pub token: Rc<Token>,
    pub http: super::http::Http,
    pub calls_wanted: Box<dyn Fn() -> bool>,
    pub status: Option<StatusCollector>,
    pub spawn: Spawn,
}

impl Light {
    pub fn new(p: Parts) -> Rc<Light> {
        Rc::new(Light {
            media: Media::new(p.config.media_dir.clone(), p.http.clone()),
            api: Api::new(p.token.clone(), p.http.clone()),
            config: p.config,
            store: p.store,
            notifier: p.notifier,
            detector: RefCell::new(Detector::default()),
            token: p.token,
            http: p.http,
            chatsvc: ChatSvc::default(),
            me: RefCell::new((String::new(), String::new())),
            list: RefCell::new(ChatList::default()),
            people: RefCell::new(IndexMap::new()),
            authors_read: RefCell::new(HashMap::new()),
            mentioned: RefCell::new(HashSet::new()),
            due_list: Cell::new(0),
            due_activity: Cell::new(0),
            scan_at: Cell::new(0),
            error: RefCell::new(String::new()),
            signed_out: Cell::new(false),
            started_at: now_ms(),
            feed_read: Cell::new(false),
            calls: RefCell::new(None),
            calls_wanted: p.calls_wanted,
            status: p.status,
            spawn: p.spawn,
            no_calls_told: Cell::new(false),
        })
    }

    /// The token for chatsvc, as the agent knows itself.
    pub async fn auth(&self) -> R<TeamsAuth> {
        let (oid, name) = self.me.borrow().clone();
        self.api.auth(Some((&oid, &name))).await
    }

    fn sign_in(&self) -> LightSignIn {
        LightSignIn::parse_value(&serde_json::from_str(&self.store.get_state(key::LIGHT_TOKEN)).unwrap_or(Value::Null))
    }

    fn set_sign_in(&self, s: &LightSignIn) {
        self.store.set_state(key::LIGHT_TOKEN, &crate::js::stringify(&sign_json(s)));
    }
}

/// LightSignIn as JSON: its keys in schema order, the optional ones when set.
pub fn sign_json(s: &LightSignIn) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("at".into(), crate::web::num(s.at));
    m.insert("expiresOn".into(), crate::web::num(s.expires_on));
    m.insert("refused".into(), json!(s.refused));
    for (k, v) in [("session", s.session), ("browser", s.browser), ("mcas", s.mcas), ("broker", s.broker)] {
        if let Some(b) = v {
            m.insert(k.into(), json!(b));
        }
    }
    Value::Object(m)
}

/// The chat the app shows now, "" when none: marked less than PARK_AFTER seconds ago, not left since.
fn shown_in_app(store: &Store, now: i64) -> String {
    let v = crate::state::Viewing::parse(Some(&store.get_state(key::VIEWING)));
    if !v.chat.is_empty() && (now as f64) - v.ts < PARK_AFTER { v.chat } else { String::new() }
}

/// The app shows this chat now (inApp).
pub fn in_app(store: &Store, chat: &str) -> bool {
    !chat.is_empty() && store.get_state(key::ACTIVE_CHAT) == chat && shown_in_app(store, now_s()) == chat
}

/// The chat is on screen in the app, unless the app stopped showing a chat after the command was queued (since).
pub fn mark_viewing(store: &Store, chat: &str, since: i64) {
    let shown = crate::state::Viewing::parse(Some(&store.get_state(key::VIEWING)));
    if since != 0 && shown.chat.is_empty() && shown.ts >= since as f64 {
        return;
    }
    store.set_state(key::VIEWING, &crate::js::stringify(&json!({ "chat": chat, "ts": now_s() })));
}

/// The sign-in Microsoft refused, for the web app's needs-sign-in state.
fn refused(l: &Light, e: &Fail) -> bool {
    let Fail::SignedOut(m) = e else { return false };
    if !l.signed_out.get() {
        warn("light", &format!("sign-in refused: {m}"), &[]);
    }
    l.signed_out.set(true);
    let s = l.sign_in();
    if !s.refused {
        l.set_sign_in(&LightSignIn { refused: true, ..s });
    }
    true
}

/// When renewSignIn next tries, and the expiry it last renewed the access tokens for.
#[derive(Default)]
pub struct Renewed {
    pub for_expiry: i64,
    pub retry_at: i64,
}

/// Captured sessions renew over HTTP; the others keep their access tokens until the refresh token ends.
pub async fn renew_sign_in(l: &Light, renewed: &mut Renewed) -> R<()> {
    let previous = l.sign_in();
    // a shared Microsoft account: the broker renews, nothing is stored here
    if l.token.broker_backed() {
        if previous.broker != Some(true) {
            l.set_sign_in(&LightSignIn { at: now_s() as f64, expires_on: 0.0, refused: false, broker: Some(true), ..Default::default() });
        } else if previous.refused && !l.signed_out.get() {
            l.set_sign_in(&LightSignIn { refused: false, ..previous });
        }
        return Ok(());
    }
    let file = l.config.light_token_file.clone();
    if previous.browser == Some(true) {
        return Ok(());
    }
    if previous.session == Some(true) || previous.mcas == Some(true) || session_file(&file).exists() {
        if previous.session != Some(true) {
            l.set_sign_in(&LightSignIn { session: Some(true), ..previous.clone() });
        }
        let stored = l.token.stored();
        if now_ms() < renewed.retry_at {
            return Ok(());
        }
        let run = async {
            let Some(stored) = stored else { return Err(Fail::NeedsSignIn("no refresh token stored".into())) };
            if !previous.refused && stored.expires_on * 1000 - now_ms() > 4 * 3_600_000 {
                return Ok(());
            }
            let fresh = if previous.mcas == Some(true) {
                McasSession::new(mcas_session_file(&file), l.http.clone()).renew(&stored).await?
            } else {
                SessionAuth::new(session_file(&file), l.http.clone()).renew(&stored).await?
            };
            fresh.save(&file)?;
            l.token.renew_all();
            l.signed_out.set(false);
            renewed.retry_at = 0;
            let at = if previous.at != 0.0 { previous.at } else { now_s() as f64 };
            let mcas = (previous.mcas == Some(true)).then_some(true);
            l.set_sign_in(&LightSignIn { at, expires_on: fresh.expires_on as f64, refused: false, session: Some(true), browser: None, mcas, broker: None });
            if previous.expires_on != fresh.expires_on as f64 {
                let until = jiff::Timestamp::from_second(fresh.expires_on).map(|t| t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default();
                info("light", "Microsoft session renewed without browser", &[("until", until.into())]);
            }
            Ok(())
        };
        if let Err(e) = run.await {
            let needs = matches!(e, Fail::NeedsSignIn(_));
            renewed.retry_at = now_ms() + if needs { 5 * 60_000 } else { 60_000 };
            if needs {
                l.set_sign_in(&LightSignIn { browser: Some(true), ..previous });
                info("light", "Microsoft session needs browser renewal", &[]);
            } else {
                warn("light", &text_of(&e), &[]);
            }
        }
        return Ok(());
    }
    let Some(s) = l.token.stored() else { return Ok(()) };
    if s.expires_on == 0 || renewed.for_expiry == s.expires_on || s.expires_on * 1000 - now_ms() > RENEW_BEFORE_MS {
        return Ok(());
    }
    renewed.for_expiry = s.expires_on;
    l.token.renew_all();
    for scope in [IC3, CHATSVCAGG, SPACES, PRESENCE] {
        let _ = l.token.access_token(scope).await;
    }
    info("light", "access tokens renewed ahead of the end of the sign-in", &[]);
    Ok(())
}

/// Who you are, from the access token; the identity row of the account when the browser never wrote one.
async fn identify(l: &Light) -> R<()> {
    let auth = l.api.auth(None).await?;
    let Some(oid) = auth.oid.clone().filter(|o| !o.is_empty()) else { return Err(Fail::Other("no object id in the access token".into())) };
    let saved = Identity::parse(Some(&l.store.get_state(key::ME)));
    // a slot mapped to a shared account reads that account only, and never the chats of a slot signed in as someone else
    if let Some(b) = &l.config.shared_broker {
        let broker = b.account.to_lowercase();
        let upn = auth.upn.clone().unwrap_or_default().to_lowercase();
        if upn != broker {
            let who = if upn.is_empty() { "an unknown account".to_string() } else { upn };
            return Err(Fail::SignedOut(format!("the shared broker gave a token of {who}, not of {broker}")));
        }
        if !saved.email.is_empty() && saved.email.to_lowercase() != broker {
            return Err(Fail::SignedOut(format!(
                "slot signed in as {}, not the shared account {broker}: remove this Teams account, the shared account gets a new one",
                saved.email
            )));
        }
    }
    let name = [saved.name.clone(), auth.name.clone().unwrap_or_default()].into_iter().find(|n| !n.is_empty()).unwrap_or_default();
    *l.me.borrow_mut() = (oid, name);
    if l.store.get_state(key::ME).is_empty() {
        let me = Identity { name: auth.name.unwrap_or_default(), email: auth.upn.unwrap_or_default(), ..saved };
        l.store.set_state(key::ME, &crate::js::stringify(&me.json()));
    }
    Ok(())
}

/// A chat with a person neither look-up names: the names the messages of the chat carry, read once per last message.
async fn name_from_messages(l: &Light, listed: &[Value]) {
    let oid = l.me.borrow().0.clone();
    let unnamed = unnamed_chats(listed, &oid, &l.people.borrow());
    let mut reads = 0;
    for c in unnamed {
        let id = c.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let last = c.get("lastMessage").and_then(|m| m.get("id")).map(|v| crate::js::string(Some(v))).unwrap_or_default();
        if reads >= AUTHOR_READS || l.authors_read.borrow().get(&id) == Some(&last) {
            continue;
        }
        reads += 1;
        l.authors_read.borrow_mut().insert(id.clone(), last);
        match l.api.authors(&id).await {
            Ok(found) => {
                let mut people = l.people.borrow_mut();
                for (mri, p) in found {
                    people.entry(mri).or_insert(p);
                }
            }
            Err(e) => {
                let kind = if c.get("isOneOnOne") == Some(&Value::Bool(true)) { "a 1:1 chat" } else { "a group chat" };
                warn("chats", &format!("names of {kind}: {}", text_of(&e)), &[]);
            }
        }
    }
}

/// The chat list: saved complete, new messages notified as the browser agent does.
pub async fn read_list(l: &Rc<Light>) -> bool {
    match list_once(l).await {
        Ok(()) => true,
        Err(e) => {
            if !refused(l, &e) {
                *l.error.borrow_mut() = text_of(&e);
                warn("chats", &text_of(&e), &[]);
            }
            false
        }
    }
}

async fn list_once(l: &Rc<Light>) -> R<()> {
    if l.me.borrow().0.is_empty() {
        identify(l).await?;
    }
    let chats = l.api.chats().await?;
    let listed = listed_chats(&chats, CHAT_LIMIT);
    let oid = l.me.borrow().0.clone();
    let missing = unnamed_people(&listed, &oid, &l.people.borrow());
    if !missing.is_empty() {
        let found = l.api.people(&missing).await.unwrap_or_default();
        l.people.borrow_mut().extend(found);
    }
    name_from_messages(l, &listed).await;
    let mine = format!("8:orgid:{oid}");
    let mut watched = vec![mine.clone()];
    for c in listed.iter().filter(|c| c.get("isOneOnOne") == Some(&Value::Bool(true))) {
        if let Some(o) = others_of(c, &oid).into_iter().next()
            && !watched.contains(&o)
        {
            watched.push(o);
        }
    }
    let presence = l.api.presence(&watched).await.unwrap_or_default();
    let me = Identity::parse(Some(&l.store.get_state(key::ME)));
    let notes = l.api.notes_last().await.unwrap_or(None);
    let my_name = if me.name.is_empty() { l.me.borrow().1.clone() } else { me.name };
    let (rows, ids) = {
        let people = l.people.borrow();
        let mentioned = l.mentioned.borrow();
        light_list(
            &chats,
            &ListInput {
                me: &oid,
                names: &people,
                presence: &presence,
                mentioned: &mentioned,
                now_ms: now_ms() as f64,
                notes: notes.map(|(_, content, time)| (format!("{my_name} (You)"), content, time)),
            },
        )
    };
    *l.list.borrow_mut() = ChatList {
        names: ids.iter().map(|(name, id)| (id.clone(), name.clone())).collect(),
        members: chats.iter().map(|c| (c.get("id").and_then(Value::as_str).unwrap_or("").to_string(), others_of(c, &oid))).collect(),
        ids,
    };
    l.store.save_chats(&rows, true).map_err(|e| Fail::Other(e.to_string()))?;
    l.store.set_state("last_scan_ts", &now_s().to_string());
    let own = presence_word(presence.get(&mine));
    if !own.is_empty() {
        l.store.set_state(key::PRESENCE, own);
    }
    notify_new(l, &rows).await;
    l.scan_at.set(now_ms());
    l.error.borrow_mut().clear();
    l.signed_out.set(false);
    Ok(())
}

async fn notify_new(l: &Rc<Light>, rows: &[ChatEntry]) {
    let news = l.detector.borrow_mut().scan(rows);
    for (chat, body) in news {
        let shown = in_app(&l.store, &chat);
        let mut fields = vec![("preview", cut(&body, 50).into())];
        if shown {
            fields.push(("inApp", true.into()));
        }
        info("NEWMSG", &chat, &fields);
        let (l2, c2) = (l.clone(), chat.clone());
        tokio::task::spawn_local(keep_slot(async move {
            let _ = save_chat(&l2, &c2).await;
        }));
        if shown {
            let (l2, c2) = (l.clone(), chat.clone());
            tokio::task::spawn_local(keep_slot(async move { mark_read(&l2, &c2).await }));
        } else {
            l.notifier.message(&chat, &body, &chat).await;
        }
    }
    let own = l.detector.borrow_mut().take_own();
    for chat in own {
        let l2 = l.clone();
        tokio::task::spawn_local(keep_slot(async move {
            let _ = save_chat(&l2, &chat).await;
        }));
    }
    // a muted chat with something new: the feed tells whether it is a mention of you
    if !l.detector.borrow_mut().take_muted().is_empty() {
        l.due_activity.set(0);
    }
}

/// ids of a state row, newest first: an array of strings, else none (z.array(z.string()).catch([])).
pub fn ids_of(raw: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(a)) if a.iter().all(Value::is_string) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

/// ids newest first: the ones of this read, then the ones of the earlier reads, up to 200.
pub fn union(now: Vec<String>, before: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for id in now.into_iter().chain(before) {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out.truncate(200);
    out
}

fn tell_old(l: &Light, items: &[super::api::Notification], since: i64) {
    let old: Vec<String> = items.iter().filter(|n| super::activity::at_of(n) < since as f64).map(|n| crate::js::string(n.activity.get("activityId"))).collect();
    for k in ["calls_told", "mentions_told"] {
        let raw = l.store.get_state(k);
        let told = if raw.is_empty() { vec![] } else { ids_of(&raw) };
        l.store.set_state(k, &crate::js::stringify(&json!(union(told, old.clone()))));
    }
}

/// Each missed call of the feed that no push told yet alerts once; the first feed only records what it has.
pub async fn push_missed_calls(store: &Store, notifier: &Rc<dyn Notify>) -> usize {
    let calls = store.missed_calls();
    let raw = store.get_state("calls_told");
    let told = (!raw.is_empty()).then(|| ids_of(&raw));
    let checked: Vec<String> =
        crate::state::object(Some(&store.get_state(key::CHECK_SEEN))).and_then(|o| o.get("calls").cloned()).map(|c| ids_of(&c.to_string())).unwrap_or_default();
    let known: HashSet<&String> = told.iter().flatten().chain(checked.iter()).collect();
    let fresh: Vec<&(String, String, String)> = if told.is_some() { calls.iter().filter(|c| !known.contains(&c.0)).collect() } else { vec![] };
    store.set_state("calls_told", &crate::js::stringify(&json!(union(calls.iter().map(|c| c.0.clone()).collect(), told.clone().unwrap_or_default()))));
    let pushes = fresh.iter().map(|(_, caller, time)| {
        let title = if caller.is_empty() { "Missed call".to_string() } else { format!("Missed call from {caller}") };
        let body = format!("Teams call{}, not answered", if time.is_empty() { String::new() } else { format!(" at {time}") });
        let n = notifier.clone();
        async move { n.notice(&title, &body).await }
    });
    futures_util::future::join_all(pushes).await;
    if !fresh.is_empty() {
        info("calls", "missed", &[("calls", fresh.len().into())]);
    }
    fresh.len()
}

/// Each mention the chat list does not notify (a muted chat, a channel) notifies once, as Teams does.
pub async fn push_mentions(store: &Store, notifier: &Rc<dyn Notify>) {
    let mentions = store.mentions();
    let raw = store.get_state("mentions_told");
    let told = (!raw.is_empty()).then(|| ids_of(&raw));
    let known: HashSet<&String> = told.iter().flatten().collect();
    let fresh: Vec<_> = if told.is_some() {
        mentions
            .iter()
            .filter(|m| !known.contains(&m.id) && m.unread && (m.channel || store.is_muted_chat(&m.chat)) && !in_app(store, &m.chat))
            .cloned()
            .collect()
    } else {
        vec![]
    };
    store.set_state("mentions_told", &crate::js::stringify(&json!(union(mentions.iter().map(|m| m.id.clone()).collect(), told.clone().unwrap_or_default()))));
    for m in fresh {
        let chat = if store.is_known_chat(&m.chat) { m.chat.clone() } else { String::new() };
        info("MENTION", &m.chat, &[("by", m.actor.as_str().into()), ("preview", cut(&m.preview, 50).into())]);
        let who = if m.actor.is_empty() { "You were mentioned".to_string() } else { format!("{} mentioned you", m.actor) };
        let body = format!("{who}{}", if m.preview.is_empty() { String::new() } else { format!(": {}", m.preview) });
        let title = if m.chat.is_empty() { m.actor.clone() } else { m.chat.clone() };
        notifier.message(&title, &body, &chat).await;
    }
}

/// The Activity feed: saved, its missed calls and the mentions the list does not tell pushed.
pub async fn read_feed(l: &Rc<Light>) -> bool {
    if l.me.borrow().0.is_empty() {
        return false;
    }
    let items = match l.api.notifications().await {
        Ok(i) => i,
        Err(e) => {
            if !refused(l, &e) {
                warn("activity", &text_of(&e), &[]);
            }
            return false;
        }
    };
    if !l.feed_read.get() {
        tell_old(l, &items, l.started_at - TOLD_BEFORE_START_MS);
    }
    l.feed_read.set(true);
    let feed = {
        let list = l.list.borrow();
        super::activity::light_feed(&items, &|id| list.names.get(id).cloned().unwrap_or_default(), now_ms() as f64)
    };
    let mention = regex::Regex::new("(?i)mention").expect("fixed");
    *l.mentioned.borrow_mut() = items
        .iter()
        .filter(|n| !n.read && mention.is_match(n.activity.get("activityType").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|n| n.activity.get("sourceThreadId").and_then(Value::as_str).filter(|t| !t.is_empty()).map(String::from))
        .collect();
    if !feed.is_empty() {
        let _ = l.store.save_activity(&feed);
    }
    l.store.set_state(key::ACTIVITY_TS, &now_s().to_string());
    push_missed_calls(&l.store, &l.notifier).await;
    push_mentions(&l.store, &l.notifier).await;
    true
}

/// A problem that lasts `after` seconds is pushed once, and its end once more (watchProblem).
pub fn watch_problem(state: &str, since: i64, alerted: bool, armed: bool, after: i64, now: i64) -> ((i64, bool), Option<&'static str>) {
    match state {
        "problem" => {
            let since = if since != 0 { since } else { now };
            let due = armed && !alerted && now - since >= after;
            ((since, alerted || due), due.then_some("problem"))
        }
        "fine" => ((0, false), alerted.then_some("fine")),
        _ => ((since, alerted), None),
    }
}

/// The health row of the web app, as the browser agent writes it.
async fn write_health(l: &Light) {
    let now = now_s();
    let poll = l.config.light_poll;
    let fresh = l.scan_at.get() != 0 && now_ms() - l.scan_at.get() < (2 * poll + 15) * 1000;
    let teams = if l.signed_out.get() {
        "login"
    } else if fresh {
        "ok"
    } else if l.scan_at.get() != 0 || l.error.borrow().is_empty() {
        "loading"
    } else {
        "err"
    };
    let last_scan = l.store.get_state("last_scan_ts").parse::<f64>().unwrap_or(0.0);
    let overall = if teams == "login" || teams == "err" {
        "red"
    } else if fresh {
        "green"
    } else {
        "yellow"
    };
    let h = json!({
        "cdp": "ok",
        "ts": now,
        "teams": teams,
        "push_subs": l.notifier.device_count(),
        "last_msg_ts": l.store.last_notification_ts(),
        "last_scan_ts": crate::web::num(last_scan),
        "watcher": if fresh { "ok" } else { "stale" },
        "overall": overall,
    });
    let w = crate::state::object(Some(&l.store.get_state(key::LOGIN_WATCH))).unwrap_or_default();
    let since = w.get("since").and_then(Value::as_f64).unwrap_or(0.0) as i64;
    let alerted = w.get("alerted").and_then(Value::as_bool).unwrap_or(false);
    let state = match teams {
        "login" => "problem",
        "ok" => "fine",
        _ => "unknown",
    };
    let armed = !l.store.get_state(key::ME).is_empty();
    let ((since, alerted), push) = watch_problem(state, since, alerted, armed, SIGN_IN_AFTER, now);
    l.store.set_state(key::LOGIN_WATCH, &crate::js::stringify(&json!({ "since": since, "alerted": alerted })));
    match push {
        Some("problem") => {
            warn("SESSION", "light account signed out: alert pushed", &[]);
            l.notifier.alert("Teams signed out", "Microsoft requires sign-in: open this account's remote desktop and sign in again.").await;
        }
        Some(_) => {
            l.notifier.alert("Teams back", "Signed in again: messages are relayed.").await;
        }
        None => {}
    }
    l.store.set_state("teams_status_prev", teams);
    l.store.set_state(key::HEALTH, &crate::js::stringify(&h));
}

/// The chat the app shows now, read again every few seconds: its new messages show as they come.
async fn read_viewing(l: &Rc<Light>) {
    let chat = l.store.get_state(key::ACTIVE_CHAT);
    if !chat.is_empty()
        && in_app(&l.store, &chat)
        && let Err(e) = save_chat(l, &chat).await
    {
        refused(l, &e);
    }
}

fn app_of(token: &str) -> String {
    super::claims(token).get("appid").map(|v| crate::js::string(Some(v))).unwrap_or_default()
}

/// The calling worker of this account follows its Calls switch. On a shared Microsoft account only with a token of a
/// Teams app: Teams calling refuses any other, and the worker would ask again every 10 s.
pub async fn sync_native_calls(l: &Rc<Light>, mut want: bool) -> R<()> {
    let has = l.calls.borrow().is_some();
    if want && !has && l.token.broker_backed() && !TEAMS_APPS.contains(&app_of(&l.token.access_token(SPACES).await?).as_str()) {
        if !l.no_calls_told.get() {
            info("calling", "not available yet on this shared Microsoft account: its token is not of a Teams app; chats and notifications only", &[]);
        }
        l.no_calls_told.set(true);
        want = false;
    }
    if want && !has {
        if l.me.borrow().0.is_empty() {
            identify(l).await?;
        }
        let calls = NativeCalls::new(l, l.spawn.clone());
        *l.calls.borrow_mut() = Some(calls.clone());
        if let Err(e) = calls.start().await {
            warn("calling", &text_of(&e), &[]);
            calls.schedule();
        }
    } else if !want && has {
        info("calling", "switched off for this account", &[]);
        let calls = l.calls.borrow_mut().take();
        if let Some(c) = calls {
            c.stop();
        }
    }
    Ok(())
}

fn reads(l: &Rc<Light>) -> Reads {
    let (a, b) = (Rc::downgrade(l), Rc::downgrade(l));
    Reads {
        list: Box::new(move || {
            let l = a.clone();
            Box::pin(async move {
                match l.upgrade() {
                    Some(l) => read_list(&l).await,
                    None => false,
                }
            })
        }),
        activity: Box::new(move || {
            let l = b.clone();
            Box::pin(async move {
                match l.upgrade() {
                    Some(l) => read_feed(&l).await,
                    None => false,
                }
            })
        }),
    }
}

/// One round of the loop, steps due now; an error ends the round, not the slot.
async fn round(l: &Rc<Light>, t: &mut Timers, renewed: &mut Renewed, reads: &Reads, lane: Lane) -> R<()> {
    let now = now_ms();
    let native = l.config.native_calls;
    if native && now >= t.calls {
        t.calls = now + 5_000;
        sync_native_calls(l, native && (l.calls_wanted)()).await?;
    }
    if now >= t.renewal {
        t.renewal = now + 60_000;
        renew_sign_in(l, renewed).await?;
    }
    if now >= t.presence {
        t.presence = now + PRESENCE_EVERY_MS;
        if let Err(e) = l.api.keep_active().await
            && !refused(l, &e)
        {
            warn("presence", &text_of(&e), &[]);
        }
    }
    if now >= l.due_list.get() {
        l.due_list.set(now + l.config.light_poll * 1000);
        read_list(l).await;
        if let Some(status) = &l.status {
            let people = l.people.borrow().clone();
            let list = l.list.borrow().clone();
            status.poll(Known { people: &people, ids: &list.ids, members: &list.members }, &l.api, &l.store, now_ms()).await;
        }
    }
    if now >= l.due_activity.get() {
        l.due_activity.set(now + l.config.light_poll * 1000);
        read_feed(l).await;
    }
    if now >= t.viewing {
        t.viewing = now + VIEWING_EVERY_MS;
        read_viewing(l).await;
    }
    if now >= t.health {
        t.health = now + HEALTH_EVERY_MS;
        write_health(l).await;
    }
    run_light_commands(l, reads, lane).await;
    Ok(())
}

struct Timers {
    health: i64,
    viewing: i64,
    renewal: i64,
    presence: i64,
    calls: i64,
}

/// The slot until `abort` (none: for good).
pub async fn run_light(l: Rc<Light>, abort: Abort) {
    let native = l.config.native_calls;
    if native {
        // the light host aborts the slot and owns the process signals: a call ends at once, not after the read in progress
        let w = Rc::downgrade(&l);
        let a = abort.clone();
        tokio::task::spawn_local(async move {
            a.wait().await;
            let calls = w.upgrade().and_then(|l| l.calls.borrow().clone());
            if let Some(c) = calls {
                c.stop();
            }
        });
        if let Err(e) = sync_native_calls(&l, (l.calls_wanted)()).await {
            warn("calling", &text_of(&e), &[]);
        }
    }
    let reads = Rc::new(reads(&l));
    let mut renewed = Renewed::default();
    let mut t = Timers { health: 0, viewing: 0, renewal: 0, presence: 0, calls: now_ms() + 5_000 };
    // call controls have their own lane: a chat-list read or message command must not hold an answer or hang-up
    if native {
        l.store.interrupted_commands();
    }
    let lane_done = Rc::new(Cell::new(false));
    if native {
        let (l2, r2, done) = (l.clone(), reads.clone(), lane_done.clone());
        tokio::task::spawn_local(keep_slot(async move {
            while !done.get() {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if done.get() {
                    break;
                }
                run_light_commands(&l2, &r2, Lane::Calls).await;
            }
        }));
    }
    let lane = if native { Lane::Other } else { Lane::All };
    while !abort.aborted() {
        if let Err(e) = round(&l, &mut t, &mut renewed, &reads, lane).await {
            warn("loop", &text_of(&e), &[]);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    lane_done.set(true);
    let calls = l.calls.borrow().clone();
    if let Some(c) = calls {
        c.stop();
    }
}

#[cfg(test)]
mod tests;
