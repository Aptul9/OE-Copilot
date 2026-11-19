//! The jobs of the browser agent's loop (src/agent/jobs: page-setup, chat-list, conversation, activity, health, identity,
//! read-by, media, desktop, self-check; src/agent/logic: hosts, health, owner, desktop, parking, self-check, read-by).

use super::Agent;
use crate::agent::browser::scripts::{self, s, st};
use crate::agent::browser::{Pick, R, sleep};
use crate::agent::media::image_key;
use crate::agent::run::{in_app, mark_viewing, watch_problem};
use crate::agent::store::{ActivityEntry, CHAT_LIMIT, ChatEntry, SavedMessage};
use crate::agent::{cut, info, now_ms, now_s, warn};
use crate::state::key;
use serde_json::{Map, Value, json};
use std::rc::Rc;
use std::sync::LazyLock;

// ---- hosts (logic/hosts.ts)

static PROXY_SUFFIX: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\.mcas(-gov)?\.(ms|us)$").expect("fixed"));
static BLANK: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^(about:|chrome:|edge:|chrome-error:)").expect("fixed"));

pub fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| PROXY_SUFFIX.replace(h, "").to_string())).unwrap_or_default()
}

pub fn is_teams_url(url: &str) -> bool {
    scripts::list("teams_hosts").contains(&host_of(url)) && !url.contains("serviceworker")
}

pub fn is_login_url(url: &str) -> bool {
    scripts::list("login_hosts").contains(&host_of(url))
}

pub fn is_blank(url: &str) -> bool {
    BLANK.is_match(url)
}

/// Whether the DevTools list of targets (/json/list) holds a page that shows something.
pub fn shows_page(targets: &Value) -> bool {
    targets.as_array().into_iter().flatten().any(|t| t["type"] == "page" && t["url"].as_str().is_some_and(|u| !u.is_empty() && !is_blank(u)))
}

/// The Teams tab; without one, the Microsoft sign-in tab.
pub fn pick_teams_page(tabs: &[(String, String)]) -> Option<String> {
    let mut login = None;
    for (id, url) in tabs {
        if is_teams_url(url) {
            return Some(id.clone());
        }
        if login.is_none() && is_login_url(url) {
            login = Some(id.clone());
        }
    }
    login
}

// ---- owner and desktop (logic/owner.ts, logic/desktop.ts, jobs/page-setup.ts)

/// Seconds the jobs that move Teams wait after the owner's last input, or after the owner opened the remote desktop
pub const OWNER_PAUSE: i64 = 180;
/// Seconds the desktop link of the app counts as the owner on the desktop, before the connection shows
pub const DESKTOP_BRIDGE: i64 = 30;

fn desktop_link_ms(a: &Agent) -> i64 {
    (crate::state::object(Some(&a.store.get_state(key::DESKTOP))).and_then(|o| o.get("ts").and_then(Value::as_f64)).unwrap_or(0.0) * 1000.0) as i64
}

/// Why the owner uses Teams now, None when not: the desktop with this account in front, the desktop link just clicked,
/// or (where the desktop cannot be watched) the owner's own input.
fn owner_why(a: &Agent, now: i64) -> Option<&'static str> {
    let link = desktop_link_ms(a);
    if a.config.desktop.is_some() && !a.desktop_unknown.get() {
        if a.on_desktop.get() {
            return Some("desktop");
        }
        return (now - link < DESKTOP_BRIDGE * 1000 && a.desktop_seen_at.get() < link).then_some("link");
    }
    let input = a.owner_at.get().unwrap_or(0);
    if now - input.max(link) >= OWNER_PAUSE * 1000 {
        return None;
    }
    Some(if now - input < OWNER_PAUSE * 1000 { "input" } else { "link" })
}

pub fn owner_uses(a: &Agent, now: i64) -> bool {
    owner_why(a, now).is_some()
}

/// The pause of the jobs that move Teams, logged when it starts and when it ends.
pub fn note_pause(a: &Agent) {
    let why = owner_why(a, now_ms());
    if why.is_some() == a.owner_paused.get() {
        return;
    }
    a.owner_paused.set(why.is_some());
    match why {
        Some(by) => info("page", "the owner uses Teams: the agent leaves it as it is", &[("by", by.into())]),
        None => info("page", "the owner left Teams: parking and presence keeper again", &[]),
    }
}

/// Viewers of the remote desktop in /proc/net/tcp: connections established to its port, by their server side.
pub fn desktop_viewers(proc_net_tcp: &str, port: u16) -> usize {
    proc_net_tcp
        .lines()
        .skip(1)
        .filter(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            f.len() > 3 && f[3] == "01" && f[1].rsplit(':').next().and_then(|p| u32::from_str_radix(p, 16).ok()) == Some(u32::from(port))
        })
        .count()
}

/// Every round: whether the owner has the remote desktop open with this account's window in front (watchDesktop).
pub async fn watch_desktop(a: &Agent) {
    let Some(d) = &a.config.desktop else { return };
    let tcp = match std::fs::read_to_string("/proc/net/tcp") {
        Ok(t) => t,
        Err(e) => {
            if !a.desktop_unknown.get() {
                warn("desktop", &format!("connections not readable, the owner's input only: {e}"), &[]);
            }
            a.desktop_unknown.set(true);
            note_pause(a);
            return;
        }
    };
    let viewers = desktop_viewers(&tcp, d.port) + desktop_viewers(&std::fs::read_to_string("/proc/net/tcp6").unwrap_or_default(), d.port);
    a.desktop_unknown.set(false);
    let mut front = false;
    if viewers > 0 {
        let mut cmd = tokio::process::Command::new("wlrctl");
        cmd.args(["toplevel", "find", "state:focused", &format!("app_id:{}", d.app_id)])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()))
            .envs(&d.session)
            .uid(d.uid)
            .gid(d.gid)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        // find exits 1 when no window matches; anything else is an error: this account taken as the one in front
        front = match tokio::time::timeout(std::time::Duration::from_secs(3), cmd.status()).await {
            Ok(Ok(s)) if s.success() => true,
            Ok(Ok(s)) if s.code() == Some(1) => false,
            other => {
                if !a.on_desktop.get() {
                    let why = match other {
                        Ok(Ok(s)) => format!("exit {s}"),
                        Ok(Err(e)) => e.to_string(),
                        Err(_) => "timed out".into(),
                    };
                    warn("desktop", &format!("window in front not known, taken as this account: {why}"), &[]);
                }
                true
            }
        };
    }
    if front {
        a.desktop_seen_at.set(now_ms());
    } else if a.on_desktop.get() {
        a.owner_at.set(None);
    }
    a.on_desktop.set(front);
    note_pause(a);
}

// ---- page setup (jobs/page-setup.ts)

/// Teams keeps the user Available only while its page is visible, focused and in use: the page says so from its first
/// script on; the notification hook captures what Teams shows; the microphone hook tells the call in progress.
pub async fn prepare_page(a: &Agent) -> R<()> {
    let page = a.page().ok_or("no page")?;
    if a.prepared.borrow_mut().insert(page.target().to_string()) {
        if let Err(e) = page.add_init_script("page-state.makeVisible").await {
            warn("page", &format!("init script: {e}"), &[]);
        }
        if let Err(e) = page.add_init_script("page-state.watchInput").await {
            warn("page", &format!("input init script: {e}"), &[]);
        }
    }
    if page.evaluate("page-state.makeVisible", None).await? == "installed" {
        info("page", "visible", &[]);
    }
    if page.evaluate("page-state.installNotificationHook", None).await? == "installed" {
        info("page", "notification hook installed", &[]);
    }
    hook_microphone(a).await;
    note_owner_input(a).await
}

/// In the pages and frames to come (a call window included) before Teams asks for the microphone, and in those open now.
async fn hook_microphone(a: &Agent) {
    let Some(page) = a.page() else { return };
    if !a.mic_hooked.get()
        && let Some(b) = page.browser()
    {
        match b.add_init_script_everywhere(&scripts::call("calls.installMicHook", None)).await {
            Ok(()) => a.mic_hooked.set(true),
            Err(e) => warn("page", &format!("microphone init script: {e}"), &[]),
        }
    }
    let mut installed: i64 = 0;
    for f in page.frames() {
        if page.evaluate_in(&f, "calls.installMicHook", None).await.is_ok_and(|v| v == "installed") {
            installed += 1;
        }
    }
    if installed > 0 {
        info("page", "microphone hook installed", &[("frames", installed.into())]);
    }
}

/// The owner's clicks, keys and wheel turns on the page since the last round, less the agent's own.
pub async fn note_owner_input(a: &Agent) -> R<()> {
    let page = a.page().ok_or("no page")?;
    page.evaluate("page-state.watchInput", None).await?;
    let owner = page.owner_input().await?;
    if let Some(last) = owner.iter().max() {
        a.owner_at.set(Some(a.owner_at.get().unwrap_or(0).max(*last)));
    }
    note_pause(a);
    Ok(())
}

const AFTER_CALL_MS: i64 = 3_000;
const AFTER_CALL_WINDOW_MS: i64 = 120_000;
const AWAY_MS: i64 = 120_000;
const RETRY_MS: i64 = 15_000;
const BACK_TRIES: u32 = 3;

/// The side bar without the chat list (a page a call left): back to the chats after a call, or after AWAY_MS.
pub fn away_from_chats(a: &Agent, now: i64) -> bool {
    let Some(since) = a.loading_since.get() else { return false };
    if a.in_call.get() || a.ringing.get() || a.teams() != "loading" || !a.rail_ready.get() || owner_uses(a, now) {
        return false;
    }
    if a.back_tries.get() > 0 {
        return now - a.back_at.get() >= RETRY_MS;
    }
    let since_call = a.call_over_at.get().map_or(i64::MAX, |t| now - t);
    (AFTER_CALL_MS..=AFTER_CALL_WINDOW_MS).contains(&since_call) || now - since >= AWAY_MS
}

/// The Chat button of the side bar; after BACK_TRIES tries that brought no list back, Teams again.
pub async fn back_to_chats(a: &Agent) {
    let Some(tp) = a.tp() else { return };
    let tries = a.back_tries.get() + 1;
    a.back_at.set(now_ms());
    a.call_over_at.set(None);
    if tries > BACK_TRIES {
        a.back_tries.set(0);
        a.loading_since.set(Some(now_ms()));
        warn("page", "still no chat list after the Chat button: Teams reloaded", &[]);
        if let Err(e) = tp.page.reload(60_000).await {
            warn("page", &format!("reload: {e}"), &[]);
        }
        return;
    }
    a.back_tries.set(tries);
    warn("page", "the side bar shows but no chat list: back to Chat", &[("try", i64::from(tries).into())]);
    if let Err(e) = tp.page.with_input(tp.click_rail(&s("chatView"), 2000)).await {
        warn("page", &format!("back to Chat: {e}"), &[]);
    }
}

/// Real input through CDP, like a person at the desk: Teams keeps its endpoint active and the user Available.
pub async fn keep_active(a: &Agent) {
    let Some(page) = a.page() else { return };
    let r = page
        .with_input(async {
            page.mouse_move(6.0, 6.0, 1).await?;
            page.mouse_move(2.0, 2.0, 1).await?;
            page.press("Shift").await
        })
        .await;
    if let Err(e) = r {
        warn("input", &e, &[]);
    }
}

/// Seconds without a mark of the app before Teams goes back to the self chat
const PARK_AFTER: f64 = 30.0;

/// Chat Teams should show now: the one in use in the app while the app shows it, otherwise the self chat.
pub fn wanted(a: &Agent) -> String {
    let active = a.store.get_state(key::ACTIVE_CHAT);
    let v = crate::state::Viewing::parse(Some(&a.store.get_state(key::VIEWING)));
    if !active.is_empty() && !v.chat.is_empty() && (now_s() as f64) - v.ts < PARK_AFTER {
        return active;
    }
    let me = a.store.self_chat();
    if me.is_empty() { active } else { me }
}

/// The visible page reads what is open: after a reload Teams reopens a chat of its own choosing, put right here. A chat
/// chatsvc keeps fresh needs no navigation.
pub async fn park(a: &Agent, want: &str) {
    let Some(tp) = a.tp() else { return };
    if want.is_empty() || a.saved_by_api.borrow().contains(want) || a.store.has_pending_commands() || tp.is_open(want).await.unwrap_or(false) {
        return;
    }
    tp.note_show(want);
    tp.open_chat(want).await;
}

/// Notifications captured by the hook, a second source of new messages.
pub async fn drain_hook(a: &Agent) -> R<()> {
    let page = a.page().ok_or("no page")?;
    let own = scripts::regex("texts", "ownNotification");
    let tag = scripts::text("healthTag");
    for n in page.evaluate("page-state.drainNotifications", None).await?.as_array().cloned().unwrap_or_default() {
        let (title, body) = (n["title"].as_str().unwrap_or(""), n["body"].as_str().unwrap_or(""));
        if title == tag || own.is_match(title) {
            continue;
        }
        let chat = if a.store.is_known_chat(title) { title } else { "" };
        let shown = in_app(&a.store, chat);
        let mut fields = vec![("body", body.into())];
        if shown {
            fields.push(("inApp", true.into()));
        }
        info("MSG", title, &fields);
        if !shown {
            a.notifier.message(title, body, chat).await;
        }
    }
    Ok(())
}

// ---- presence (shared/presence.ts)

/// The presence badge of a person ("Available, Out of office") as one of the words of the app, "" for none.
pub fn presence_of(label: &str) -> &'static str {
    static LABELS: LazyLock<Vec<(&'static str, regex::Regex)>> = LazyLock::new(|| {
        [
            ("ooo", r"(?i)out of (the )?office"),
            ("dnd", r"(?i)do not disturb|presenting|focusing"),
            ("busy", r"(?i)busy|in a (call|meeting|conference)|on the phone"),
            ("away", r"(?i)away|be right back|inactive"),
            ("offline", r"(?i)offline"),
            ("available", r"(?i)available"),
        ]
        .into_iter()
        .map(|(p, r)| (p, regex::Regex::new(r).expect("fixed")))
        .collect()
    });
    LABELS.iter().find(|(_, re)| re.is_match(label)).map_or("", |(p, _)| p)
}

// ---- chat list (jobs/chat-list.ts)

/// A row of the list as the page reads it (ListRow), its presence a word of the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListRow {
    pub entry: ChatEntry,
    pub avsrc: String,
}

/// The rows as Teams shows them; a chat read through chatsvc moments ago shows read.
pub async fn read_list(a: &Agent) -> R<Vec<ListRow>> {
    let page = a.page().ok_or("no page")?;
    let rows = page.evaluate("chat-list.readChatList", Some(&st(json!({})))).await?;
    Ok(rows
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
            let b = |k: &str| r[k] == Value::Bool(true);
            let read_now = is_recently_read(a, &s("name"), &s("preview"));
            ListRow {
                entry: ChatEntry {
                    name: s("name"),
                    preview: s("preview"),
                    time: s("time"),
                    unread: !read_now && b("unread"),
                    mention: !read_now && b("mention"),
                    muted: b("muted"),
                    av: String::new(),
                    presence: presence_of(&s("presence")).into(),
                    kind: s("kind"),
                },
                avsrc: s("avsrc"),
            }
        })
        .collect())
}

/// New messages found in a list read: pushed, none for the chat the app shows now.
pub async fn notify_new(a: &Rc<Agent>, rows: &[ChatEntry]) {
    let news = a.detector.borrow_mut().scan(rows);
    for (chat, body) in news {
        let shown = in_app(&a.store, &chat);
        let mut fields = vec![("preview", cut(&body, 50).into())];
        if shown {
            fields.push(("inApp", true.into()));
        }
        info("NEWMSG", &chat, &fields);
        let (a2, c2) = (a.clone(), chat.clone());
        tokio::task::spawn_local(async move {
            save_chat_via_api(&a2, &c2).await;
        });
        if shown {
            let (a2, c2) = (a.clone(), chat.clone());
            tokio::task::spawn_local(async move { mark_read_via_api(&a2, &c2).await });
        } else {
            a.notifier.message(&chat, &body, &chat).await;
        }
    }
    let own = a.detector.borrow_mut().take_own();
    for chat in own {
        let a2 = a.clone();
        tokio::task::spawn_local(async move {
            save_chat_via_api(&a2, &chat).await;
        });
    }
    let muted = a.detector.borrow_mut().take_muted();
    if !muted.is_empty() {
        info("chats", "muted chat with something new: reading Activity for a mention", &[("chats", muted.join(", ").into())]);
        a.mentions_due.set(true);
    }
}

/// The chats Teams has in the page saved on top of the known ones, pictures copied or not, then their new messages.
pub async fn read_chats(a: &Rc<Agent>, pictures: bool) -> R<bool> {
    let rows = read_list(a).await?;
    if rows.is_empty() {
        return Ok(false);
    }
    let entries = if pictures { with_pictures(a, rows, 8).await } else { rows.into_iter().map(|r| r.entry).collect() };
    a.store.save_chats(&entries, false).map_err(|e| e.to_string())?;
    a.store.set_state("last_scan_ts", &now_s().to_string());
    notify_new(a, &entries).await;
    Ok(true)
}

async fn with_pictures(a: &Agent, rows: Vec<ListRow>, limit: usize) -> Vec<ChatEntry> {
    let Some(page) = a.page() else { return rows.into_iter().map(|r| r.entry).collect() };
    let mut budget = limit;
    let mut out = vec![];
    for r in rows {
        let av = a.media.avatar(&page, &r.avsrc, &mut budget).await;
        out.push(ChatEntry { av, ..r.entry });
    }
    out
}

pub async fn scan_chats(a: &Rc<Agent>) {
    if let Err(e) = read_chats(a, true).await {
        warn("chats", &e, &[]);
    }
}

/// Two chats of the first 20 without messages saved read through chatsvc, in the background.
pub async fn prefetch_chats(a: &Rc<Agent>) {
    let mut tried = 0;
    for c in a.store.chats().into_iter().take(20) {
        if tried >= 2 || a.store.has_pending_commands() {
            break;
        }
        if a.store.messages(&c.name).is_empty() {
            tried += 1;
            save_chat_via_api(a, &c.name).await;
        }
    }
}

/// The whole list, scrolled from the top (it is virtualized), saved complete and in order, then back to the top. Number
/// of chats read, None when the read failed.
pub async fn scan_chats_full(a: &Rc<Agent>) -> Option<usize> {
    let page = a.page()?;
    let scroll = |to: &'static str| {
        let page = page.clone();
        async move { page.evaluate("chat-list.scrollList", Some(&json!({ "item": s("anyChatRow"), "to": to }))).await }
    };
    let run = async {
        scroll("top").await?;
        sleep(400).await;
        let mut seen: indexmap::IndexMap<String, ChatEntry> = indexmap::IndexMap::new();
        for _ in 0..8 {
            for c in with_pictures(a, read_list(a).await?, 20).await {
                seen.insert(c.name.clone(), c);
            }
            if seen.len() >= CHAT_LIMIT || scroll("down").await? != Value::Bool(true) {
                break;
            }
            sleep(500).await;
        }
        scroll("top").await?;
        if seen.is_empty() {
            return Ok::<usize, String>(0);
        }
        let all: Vec<ChatEntry> = seen.into_values().take(CHAT_LIMIT).collect();
        a.store.save_chats(&all, true).map_err(|e| e.to_string())?;
        a.store.set_state("last_scan_ts", &now_s().to_string());
        Ok(all.len())
    };
    match run.await {
        Ok(n) => Some(n),
        Err(e) => {
            warn("chats", &format!("full list: {e}"), &[]);
            None
        }
    }
}

// ---- conversation (jobs/conversation.ts)

/// A chat read through chatsvc moments ago shows read in the list until its preview changes (recentlyReadChats).
pub fn is_recently_read(a: &Agent, chat: &str, preview: &str) -> bool {
    let entry = a.recently_read.borrow().get(chat).cloned();
    let Some((was, until)) = entry else { return false };
    if now_ms() > until || (!preview.is_empty() && !was.is_empty() && preview != was) {
        a.recently_read.borrow_mut().remove(chat);
        return false;
    }
    true
}

/// The chat read: in the app at once, on Teams at its last message.
pub async fn mark_read_via_api(a: &Agent, chat: &str) {
    let preview = a.store.chats().into_iter().find(|c| c.name == chat).map(|c| c.preview).unwrap_or_default();
    a.recently_read.borrow_mut().insert(chat.to_string(), (preview, now_ms() + 20_000));
    a.store.mark_chat_read(chat);
    let Some(auth) = a.page_auth().await else { return };
    let Some(conv) = a.conversation(&auth, chat, None).await else { return };
    let last = a.store.messages(chat).last().and_then(|m| m.get("mid").and_then(Value::as_str).map(String::from));
    a.chatsvc.mark_read(&a.http, &auth, &conv, last.as_deref()).await;
}

/// Messages of the chat through chatsvc, the pictures and images saved before kept; true once saved.
pub async fn save_chat_via_api(a: &Agent, chat: &str) -> bool {
    let saved = save_through_api(a, chat).await;
    if saved {
        a.saved_by_api.borrow_mut().insert(chat.to_string());
    } else {
        a.saved_by_api.borrow_mut().remove(chat);
    }
    saved
}

async fn save_through_api(a: &Agent, chat: &str) -> bool {
    let Some(page) = a.page() else { return false };
    let Some(auth) = a.page_auth().await else { return false };
    let Some(conv) = a.conversation(&auth, chat, None).await else { return false };
    let read_by = |mids: &[String]| a.store.read_by_cache(mids);
    let Some(messages) = a.chatsvc.fetch_chat_messages(&a.http, &auth, &conv, &read_by).await else { return false };
    let existing = a.store.messages(chat);
    let prev = |mid: &str| existing.iter().find(|m| m.get("mid").and_then(Value::as_str) == Some(mid));
    let author_av = |author: &str| {
        existing
            .iter()
            .find(|m| m.get("author").and_then(Value::as_str) == Some(author) && m.get("av").and_then(Value::as_str).is_some_and(|v| !v.is_empty()))
            .and_then(|m| m["av"].as_str().map(String::from))
    };
    let entry = a.store.chats().into_iter().find(|c| c.name == chat);
    let default_av = entry.filter(|c| c.kind == "one" && !c.av.is_empty()).map(|c| c.av).unwrap_or_default();
    let mut out = vec![];
    for mut m in messages {
        let before = prev(&m.mid);
        let mut extra = m.extra.clone().unwrap_or_default();
        let av = before
            .and_then(|b| b.get("av").and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from))
            .or_else(|| (!m.author.is_empty()).then(|| author_av(&m.author)).flatten())
            .or_else(|| (!m.mine && !default_av.is_empty()).then(|| default_av.clone()))
            .unwrap_or_default();
        let status = before
            .and_then(|b| b.get("status").and_then(Value::as_str))
            .filter(|s| s.to_lowercase().contains("seen"))
            .map(|s| json!(s))
            .or_else(|| extra.get("status").cloned());
        if let Some(Value::Array(images)) = extra.get("images").cloned()
            && !images.is_empty()
        {
            let mut resolved = vec![];
            for (i, im) in images.iter().enumerate() {
                let w = im.get("w").cloned().unwrap_or(json!(0));
                let h = im.get("h").cloned().unwrap_or(json!(0));
                if let Some(f) =
                    before.and_then(|b| b.get("images")).and_then(|x| x.get(i)).and_then(|x| x.get("f")).and_then(Value::as_str).filter(|f| !f.is_empty())
                {
                    let pw = before.and_then(|b| b["images"][i].get("w").cloned()).unwrap_or(w);
                    let ph = before.and_then(|b| b["images"][i].get("h").cloned()).unwrap_or(h);
                    resolved.push(json!({ "f": f, "w": pw, "h": ph }));
                    continue;
                }
                let src = im.get("url").and_then(Value::as_str).unwrap_or("");
                let f = if src.is_empty() { None } else { a.media.image(Some(&page), &image_key(chat, &m.mid, i), src, &auth.token).await };
                match f {
                    Some(f) => resolved.push(json!({ "f": f, "w": w, "h": h })),
                    None if src.starts_with("https://") => resolved.push(json!({ "url": src, "w": w, "h": h })),
                    None => {}
                }
            }
            if resolved.is_empty() {
                extra.shift_remove("images");
            } else {
                extra.insert("images".into(), Value::Array(resolved));
            }
        }
        crate::js::set(&mut extra, "status", status);
        crate::js::set(&mut extra, "av", Some(json!(av)));
        m.extra = crate::agent::chatsvc::extra_of(&extra);
        out.push(m);
    }
    match a.store.save_chat_messages(chat, &out) {
        Ok(()) => true,
        Err(e) => {
            warn("messages", &format!("chatsvc save failed: {e}"), &[("chat", chat.into())]);
            false
        }
    }
}

/// Messages of the chat open in Teams; None when Teams shows another chat or the page could not be read.
pub async fn read_open_messages(a: &Agent, chat: &str) -> Option<Vec<SavedMessage>> {
    let tp = a.tp()?;
    let rows = async {
        if !tp.is_open(chat).await? {
            return Ok(None);
        }
        tp.page.evaluate("conversation.readMessages", Some(&st(json!({})))).await.map(Some)
    };
    let rows = match rows.await {
        Ok(Some(r)) => r.as_array().cloned().unwrap_or_default(),
        Ok(None) => return None,
        Err(e) => {
            warn("messages", &e, &[("chat", chat.into())]);
            return None;
        }
    };
    let mine: Vec<String> =
        rows.iter().filter(|m| m["mine"] == Value::Bool(true)).filter_map(|m| m["mid"].as_str().filter(|x| !x.is_empty()).map(String::from)).collect();
    let read_by = a.store.read_by_cache(&mine);
    let mut budget = 8;
    let mut out = vec![];
    for m in rows {
        let s = |k: &str| m[k].as_str().unwrap_or("").to_string();
        let mid = s("mid");
        let mut images = vec![];
        for (i, im) in m["images"].as_array().into_iter().flatten().enumerate() {
            let src = im["src"].as_str().unwrap_or("");
            let (w, h) = (im["w"].clone(), im["h"].clone());
            match a.media.image(Some(&tp.page), &image_key(chat, &mid, i), src, "").await {
                Some(f) => images.push(json!({ "f": f, "w": w, "h": h })),
                // the address of an image Teams has not loaded yet works only inside the Teams session
                None if im["loaded"] == Value::Bool(true) && src.starts_with("https://") => images.push(json!({ "url": src, "w": w, "h": h })),
                None => {}
            }
        }
        let av = a.media.avatar(&tp.page, &s("avsrc"), &mut budget).await;
        let mut fields = Map::new();
        for k in ["quote", "files", "reactions", "status", "edited", "html", "mentionsMe", "deleted"] {
            if let Some(v) = m.get(k).filter(|v| !v.is_null()) {
                fields.insert(k.into(), v.clone());
            }
        }
        fields.insert("images".into(), Value::Array(images));
        if let Some(rb) = read_by.get(&mid) {
            fields.insert("readby".into(), rb.clone());
        }
        fields.insert("av".into(), json!(av));
        let ordered: Map<String, Value> = ["quote", "images", "files", "reactions", "status", "edited", "readby", "html", "mentionsMe", "av", "deleted"]
            .iter()
            .filter_map(|k| fields.get(*k).map(|v| ((*k).to_string(), v.clone())))
            .collect();
        out.push(SavedMessage {
            mid,
            author: s("author"),
            text: s("text"),
            mine: m["mine"] == Value::Bool(true),
            reacts: s("reacts"),
            extra: crate::agent::chatsvc::extra_of(&ordered),
        });
    }
    Some(out)
}

/// True once the messages of the chat are saved: through chatsvc, else from the page.
pub async fn save_open_chat(a: &Agent, chat: &str) -> bool {
    if save_chat_via_api(a, chat).await {
        return true;
    }
    match read_open_messages(a, chat).await {
        Some(messages) => a.store.save_chat_messages(chat, &messages).is_ok(),
        None => false,
    }
}

pub fn mark_viewing_of(a: &Agent, chat: &str, since: i64) {
    mark_viewing(&a.store, chat, since);
}

// ---- Activity feed (jobs/activity.ts)

const FEED_LIMIT: usize = 40;

/// The feed through chatsvc with the token of the page; pictures the page copied stay. Number of items saved, None
/// when chatsvc does not serve it.
pub async fn read_activity_api(a: &Agent) -> Option<usize> {
    let page = a.page()?;
    let auth = a.page_auth().await?;
    let headers = vec![("Accept", "application/json".to_string()), ("Authorization", format!("Bearer {}", auth.token))];
    let r = match crate::agent::http::send(&a.http, "GET", &crate::agent::api::notifications_url(&auth.region), headers, vec![], 15_000).await {
        Ok(r) if r.ok() => r,
        Ok(r) => {
            warn("activity", &format!("chatsvc: HTTP {}", r.status), &[]);
            return None;
        }
        Err(e) => {
            warn("activity", &format!("chatsvc: {}", crate::agent::text_of(&e)), &[]);
            return None;
        }
    };
    let notes = crate::agent::api::notifications_of(&crate::agent::http::json_or_empty(&r));
    a.auth.sync(&page, false).await;
    let chats = a.store.chats();
    let pictures = a.store.activity_avatars();
    let items: Vec<ActivityEntry> = crate::agent::activity::light_feed(
        &notes,
        &|conv| chats.iter().find(|c| a.auth.cached(&c.name).as_deref() == Some(conv)).map(|c| c.name.clone()).unwrap_or_default(),
        now_ms() as f64,
    )
    .into_iter()
    .map(|e| {
        let av = pictures.get(&e.id).cloned().or_else(|| chats.iter().find(|c| c.name == e.actor).map(|c| c.av.clone())).unwrap_or_default();
        ActivityEntry { av, ..e }
    })
    .collect();
    if items.is_empty() {
        return None;
    }
    let _ = a.store.save_activity(&items);
    a.store.set_state(key::ACTIVITY_TS, &now_s().to_string());
    Some(items.len())
}

/// The Activity feed, through chatsvc, else in the page and back to the chat view. Number of items read, or None.
pub async fn read_activity(a: &Agent, rail_wait: u64) -> Option<usize> {
    if let Some(n) = read_activity_api(a).await {
        return Some(n);
    }
    let tp = a.tp()?;
    let page = tp.page.clone();
    let read = async {
        tp.clear_overlays().await;
        tp.click_rail(&s("activityView"), rail_wait).await?;
        page.wait_for(&s("feedItem"), Pick::First, 8000).await?;
        sleep(500).await;
        let mut seen: indexmap::IndexMap<String, Value> = indexmap::IndexMap::new();
        for _ in 0..6 {
            for item in page.evaluate("activity.readActivityFeed", Some(&st(json!({})))).await?.as_array().cloned().unwrap_or_default() {
                let id = item["id"]
                    .as_str()
                    .filter(|i| !i.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("{}{}", item["title"].as_str().unwrap_or(""), item["tm"].as_str().unwrap_or("")));
                seen.insert(id, item);
            }
            if seen.len() >= FEED_LIMIT
                || page.evaluate("chat-list.scrollList", Some(&json!({ "item": s("feedItem"), "to": "down" }))).await? != Value::Bool(true)
            {
                break;
            }
            sleep(600).await;
        }
        let mut budget = FEED_LIMIT;
        let mut items = vec![];
        for it in seen.into_values().take(FEED_LIMIT) {
            let st = |k: &str| it[k].as_str().unwrap_or("").to_string();
            let av = a.media.avatar(&page, &st("avsrc"), &mut budget).await;
            items.push(ActivityEntry {
                id: st("id"),
                kind: st("kind"),
                actor: st("actor"),
                title: st("title"),
                emoji: st("emoji"),
                preview: st("preview"),
                tm: st("tm"),
                chat: st("chat"),
                channel: it["channel"] == Value::Bool(true),
                unread: it["unread"] == Value::Bool(true),
                av,
            });
        }
        Ok::<Vec<ActivityEntry>, String>(items)
    };
    let items = match read.await {
        Ok(i) => Some(i),
        Err(e) => {
            warn("activity", &e, &[]);
            None
        }
    };
    // always back to Chat: the rest of the agent works on the chat view
    let back = async {
        tp.click_rail(&s("chatView"), 0).await?;
        page.wait_for(&s("anyChatRow"), Pick::First, 8000).await?;
        let want = wanted(a);
        if !want.is_empty() {
            tp.open_chat(&want).await;
        }
        Ok::<(), String>(())
    };
    if let Err(e) = back.await {
        warn("activity", &format!("back to chat: {e}"), &[]);
    }
    let items = items.filter(|i| !i.is_empty())?;
    let _ = a.store.save_activity(&items);
    a.store.set_state(key::ACTIVITY_TS, &now_s().to_string());
    Some(items.len())
}

// ---- health (jobs/health.ts, logic/health.ts)

/// seconds Teams stays signed out, or the browser does not start, before the push
const SIGN_IN_AFTER: i64 = 60;
const BROWSER_AFTER: i64 = 300;
/// A chat list read in the last minute means new message detection is running
const SCAN_FRESH: i64 = 60;
/// seconds signed out before the one press of Sign in, seconds it has to bring Teams back, seconds between two attempts
pub const SIGN_IN_TRY_AFTER: i64 = 15;
pub const SIGN_IN_TRY_WAIT: i64 = 60;
pub const SIGN_IN_TRY_EVERY: i64 = 1800;

/// The health row from what the page shows (computeHealth): probe None when the page could not be read.
pub fn compute_health(probe: Option<&Value>, url: &str, push_subs: usize, last_msg_ts: i64, last_scan_ts: i64, now: i64) -> Value {
    let mut h = Map::new();
    h.insert("cdp".into(), json!("ok"));
    h.insert("ts".into(), json!(now));
    let mut teams = "err";
    let mut page = Map::new();
    match probe {
        Some(p) => {
            let low = url.to_lowercase();
            let signed_out = low.contains("login") || low.contains("signin");
            let reduced = p["reduced"] == Value::Bool(true);
            teams = if signed_out || reduced {
                "login"
            } else if p["domReady"] == Value::Bool(true) {
                "ok"
            } else {
                "loading"
            };
            page.insert("reduced".into(), json!(reduced));
            page.insert("hook".into(), json!(if p["hookInstalled"] == Value::Bool(true) { "ok" } else { "no" }));
            if let Some(pr) = p.get("presence").filter(|v| !v.is_null()) {
                page.insert("presence".into(), pr.clone());
            }
        }
        None => {
            page.insert("hook".into(), json!("no"));
        }
    }
    h.insert("teams".into(), json!(teams));
    h.extend(page);
    let fresh = last_scan_ts != 0 && now - last_scan_ts < SCAN_FRESH;
    h.insert("push_subs".into(), json!(push_subs));
    h.insert("last_msg_ts".into(), json!(last_msg_ts));
    h.insert("last_scan_ts".into(), json!(last_scan_ts));
    h.insert("watcher".into(), json!(if fresh { "ok" } else { "stale" }));
    h.insert(
        "overall".into(),
        json!(if teams == "login" || teams == "err" {
            "red"
        } else if fresh {
            "green"
        } else {
            "yellow"
        }),
    );
    Value::Object(h)
}

/// The health row read by the app, about every 5 s.
pub async fn update_health(a: &Agent) -> Value {
    let mut probe = None;
    let mut rail = false;
    let mut url = String::new();
    if let Some(page) = a.page() {
        url = page.url();
        let on_teams = is_teams_url(&url);
        let read = async {
            let p = page.evaluate("page-state.probePage", Some(&st(json!({ "withPresence": on_teams })))).await?;
            if on_teams {
                rail = page.evaluate("page-state.uncoveredPoint", Some(&json!(s("activityView")))).await?.is_object()
                    || !page.evaluate("message-actions.openOverlayNames", Some(&scripts::sel())).await?.as_array().is_none_or(Vec::is_empty);
                if let Some(presence) = p["presence"].as_str().filter(|x| !x.is_empty()) {
                    a.store.set_state(key::PRESENCE, presence_of(presence));
                    let before = a.store.get_state("presence_prev");
                    if presence != before {
                        info("presence", &format!("{} to {presence}", if before.is_empty() { "-" } else { &before }), &[]);
                        a.store.set_state("presence_prev", presence);
                    }
                }
            }
            Ok::<Value, String>(p)
        };
        match read.await {
            Ok(p) => probe = Some(p),
            Err(e) => warn("health", &e, &[]),
        }
    }
    a.rail_ready.set(rail);
    let last_scan = a.store.get_state("last_scan_ts").parse::<i64>().unwrap_or(0);
    let mut h = compute_health(probe.as_ref(), &url, a.notifier.device_count(), a.store.last_notification_ts(), last_scan, now_s());
    if h["teams"] == "loading" && rail {
        if a.loading_since.get().is_none() {
            a.loading_since.set(Some(now_ms()));
        }
    } else {
        a.loading_since.set(None);
    }
    if h["teams"] == "ok" {
        a.back_tries.set(0);
    }
    if owner_uses(a, now_ms()) {
        h["desktop"] = json!("in-use");
    }
    save_health(a, h).await
}

/// Health while the browser shows no Teams page: a blank tab is Teams starting, any other page a sign-in waiting.
pub async fn no_tab_health(a: &Agent, url: &str) -> Value {
    let sign_in = url.starts_with("http:") || url.starts_with("https:");
    let h = json!({ "cdp": "ok", "teams": if sign_in { "login" } else { "loading" }, "overall": if sign_in { "red" } else { "yellow" }, "ts": now_s() });
    save_health(a, h).await
}

async fn save_health(a: &Agent, h: Value) -> Value {
    watch_sign_in(a, h["teams"].as_str().unwrap_or("")).await;
    watch_browser(a, h["browser"] == "down").await;
    a.store.set_state("teams_status_prev", h["teams"].as_str().unwrap_or(""));
    a.store.set_state(key::HEALTH, &crate::js::stringify(&h));
    *a.health.borrow_mut() = Some(h.clone());
    h
}

pub fn watch_of(raw: &str) -> (i64, bool) {
    let o = crate::state::object(Some(raw)).unwrap_or_default();
    (o.get("since").and_then(Value::as_f64).unwrap_or(0.0) as i64, o.get("alerted").and_then(Value::as_bool).unwrap_or(false))
}

/// The one press of Sign in of the last sign-out: when, what it pressed, whether Microsoft's page was pressed.
pub fn sign_in_try(a: &Agent) -> (i64, Vec<String>, bool) {
    let o = crate::state::object(Some(&a.store.get_state("sign_in_try"))).unwrap_or_default();
    let pressed = o.get("pressed").and_then(Value::as_array).map(|p| p.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
    (o.get("at").and_then(Value::as_f64).unwrap_or(0.0) as i64, pressed, o.get("microsoft").and_then(Value::as_bool).unwrap_or(false))
}

/// One push when Teams has been signed out for a minute, one more when it is back; after a press of Sign in, the push
/// waits for the minute of that press.
async fn watch_sign_in(a: &Agent, teams: &str) {
    let state = match teams {
        "login" => "problem",
        "ok" => "fine",
        _ => "unknown",
    };
    let (since, alerted) = watch_of(&a.store.get_state(key::LOGIN_WATCH));
    let (at, pressed_list, _) = sign_in_try(a);
    let pressed = since > 0 && at >= since && !pressed_list.is_empty();
    let after = if pressed { SIGN_IN_AFTER.max(at - since + SIGN_IN_TRY_WAIT) } else { SIGN_IN_AFTER };
    let armed = !a.store.get_state(key::ME).is_empty();
    let ((since, alerted), push) = watch_problem(state, since, alerted, armed, after, now_s());
    a.store.set_state(key::LOGIN_WATCH, &crate::js::stringify(&json!({ "since": since, "alerted": alerted })));
    match push {
        Some("problem") => {
            warn("SESSION", if pressed { "Teams still signed out after the Sign in button: alert pushed" } else { "Teams signed out: alert pushed" }, &[]);
            let lead = if pressed { "The Sign in button did not help. " } else { "" };
            a.notifier.alert("Teams signed out", &format!("{lead}{}: no messages until then.", a.config.alert_sign_in)).await;
        }
        Some(_) => {
            info("SESSION", "Teams signed in again", &[]);
            a.notifier.alert("Teams back", "Signed in again: messages are relayed.").await;
        }
        None if state == "fine" && pressed => {
            info("SESSION", "Teams signed in again after the Sign in button: nothing pushed", &[("pressed", pressed_list.join(",").into())])
        }
        None => {}
    }
}

async fn watch_browser(a: &Agent, down: bool) {
    let (since, alerted) = watch_of(&a.store.get_state("browser_watch"));
    let ((since, alerted), push) = watch_problem(if down { "problem" } else { "fine" }, since, alerted, true, BROWSER_AFTER, now_s());
    a.store.set_state("browser_watch", &crate::js::stringify(&json!({ "since": since, "alerted": alerted })));
    match push {
        Some("problem") => {
            a.notifier.alert("Relay browser down", &format!("{}: see the log.", a.config.alert_browser_down)).await;
        }
        Some(_) => {
            a.notifier.alert("Relay browser back", "The browser runs again.").await;
        }
        None => {}
    }
}

// ---- identity (jobs/identity.ts)

/// Who is signed in: name, email and organization, with the profile picture and the logo of the company.
pub async fn save_identity(a: &Agent) {
    let Some(page) = a.page() else { return };
    let read = match page.evaluate("page-state.readIdentity", Some(&scripts::sel())).await {
        Ok(r) => r,
        Err(e) => {
            warn("identity", &e, &[]);
            return;
        }
    };
    let s = |k: &str| read[k].as_str().unwrap_or("").to_string();
    if s("email").is_empty() && s("name").is_empty() {
        return;
    }
    let old = crate::state::object(Some(&a.store.get_state(key::ME)));
    let o = |k: &str| old.as_ref().and_then(|m| m.get(k).and_then(Value::as_str)).unwrap_or("").to_string();
    let mut budget = 1;
    let av = a.media.avatar(&page, &s("avsrc"), &mut budget).await;
    let av = if av.is_empty() { o("av") } else { av };
    let saved_logo = if old.is_some() && o("tenant") == s("tenant") { o("logo") } else { String::new() };
    let logo = a.media.company_logo(&s("email"), &s("tenant"), &saved_logo, Some(&page)).await;
    let me = json!({ "name": s("name"), "email": s("email"), "tenant": s("tenant"), "av": av, "logo": logo });
    if old.is_none() || o("name") != s("name") || o("email") != s("email") || o("tenant") != s("tenant") || o("av") != me["av"] || o("logo") != logo {
        info("identity", "account", &[("email", s("email").into()), ("tenant", s("tenant").into()), ("logo", (!logo.is_empty()).into())]);
    }
    a.store.set_state(key::ME, &crate::js::stringify(&me));
}

// ---- Read by (jobs/read-by.ts, logic/read-by.ts)

pub const READBY_RECENT: usize = 5;
pub const READBY_EVERY: i64 = 60;

pub fn read_by_done(label: &str) -> bool {
    scripts::regex("texts", "readBy").captures(label).is_some_and(|m| m[1] == m[2])
}

/// "Read by" of one of your recent messages of the open chat, one per call.
pub async fn prefetch_read_by(a: &Agent, chat: &str) -> bool {
    let one = format!("chat_1to1:{chat}");
    if chat.is_empty() || a.store.get_state(&one) == "1" {
        return false;
    }
    let known = a.store.read_by_of(chat);
    let now = now_s();
    let Some(mid) = a
        .store
        .own_recent_message_ids(chat, READBY_RECENT)
        .into_iter()
        .find(|mid| known.get(mid).is_none_or(|(label, ts)| !read_by_done(label) && now - ts > READBY_EVERY))
    else {
        return false;
    };
    let Some((label, names)) = super::actions::read_receipts(a, chat, &mid).await else { return true };
    // 1:1 chat: the Seen status is enough
    if label.is_empty() {
        a.store.set_state(&one, "1");
        return true;
    }
    a.store.save_read_by(&mid, chat, &label, &names);
    save_open_chat(a, chat).await;
    true
}

// ---- media and self-check

pub fn prune_media(a: &Agent) {
    let removed = a.media.prune(&a.store.media_files());
    if removed > 0 {
        info("media", "removed files no row names", &[("files", removed.into())]);
    }
}

/// The state key of the window of the automatic check now (8-11 and 17-20 local time), None outside both.
pub fn self_check_window(now_ms: i64) -> Option<String> {
    let t = jiff::Timestamp::from_millisecond(now_ms).ok()?.to_zoned(crate::agent::local_zone());
    let h = t.hour();
    let half = if (8..11).contains(&h) {
        "am"
    } else if (17..20).contains(&h) {
        "pm"
    } else {
        return None;
    };
    Some(format!("hc_{:04}{:02}{:02}_{half}", t.year(), t.month(), t.day()))
}

pub fn self_check_due(a: &Agent) -> Option<String> {
    self_check_window(now_ms()).filter(|k| a.store.get_state(k) != "1")
}

/// Real check: Teams connected and the new message detection running.
pub async fn self_check(a: &Rc<Agent>) -> (bool, String) {
    let h = update_health(a).await;
    match h["teams"].as_str().unwrap_or("") {
        "login" => return (false, "Teams signed out: sign in again".into()),
        "ok" => {}
        _ => return (false, "Teams not fully loaded".into()),
    }
    match read_chats(a, false).await {
        Ok(true) => (true, String::new()),
        Ok(false) => (false, "Chat list not readable".into()),
        Err(e) => (false, format!("New message detection failed: {e}")),
    }
}

pub async fn scheduled_self_check(a: &Rc<Agent>) {
    let Some(key) = self_check_due(a) else { return };
    let (ok, why) = self_check(a).await;
    a.store.set_state(&key, "1");
    if !ok {
        a.notifier.alert("Teams: problem", &why).await;
    }
    let mut fields = vec![];
    if !why.is_empty() {
        fields.push(("why", why.as_str().into()));
    }
    info("SELFCHECK", if ok { "ok" } else { "problem" }, &fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_presence_health_and_viewers() {
        assert!(is_teams_url("https://teams.cloud.microsoft/v2/"));
        assert!(is_teams_url("https://teams.microsoft.com.mcas.ms/"));
        assert!(!is_teams_url("https://teams.microsoft.com/serviceworker.js"));
        assert!(is_login_url("https://login.microsoftonline.com/common"));
        let tabs = vec![("a".to_string(), "https://login.microsoftonline.com/x".to_string()), ("b".to_string(), "https://teams.cloud.microsoft/".to_string())];
        assert_eq!(pick_teams_page(&tabs).as_deref(), Some("b"));
        assert_eq!(pick_teams_page(&tabs[..1]).as_deref(), Some("a"));
        assert!(shows_page(&json!([{ "type": "page", "url": "https://x" }])) && !shows_page(&json!([{ "type": "page", "url": "about:blank" }])));
        assert_eq!(presence_of("Available, Out of office"), "ooo");
        assert_eq!(presence_of("In a call"), "busy");
        assert_eq!(presence_of("Presenting"), "dnd");
        assert_eq!(presence_of("Be right back"), "away");
        let ok = json!({ "reduced": false, "domReady": true, "hookInstalled": true, "presence": "available" });
        let h = compute_health(Some(&ok), "https://teams.cloud.microsoft/", 2, 10, 100, 120);
        assert_eq!(
            h,
            json!({ "cdp": "ok", "ts": 120, "teams": "ok", "reduced": false, "hook": "ok", "presence": "available", "push_subs": 2, "last_msg_ts": 10, "last_scan_ts": 100, "watcher": "ok", "overall": "green" })
        );
        assert_eq!(compute_health(Some(&ok), "https://login.microsoftonline.com/", 0, 0, 0, 120)["teams"], "login");
        assert_eq!(compute_health(None, "", 0, 0, 0, 120)["overall"], "red");
        let tcp = "  sl  local_address rem_address   st\n   0: 0100007F:1F92 0100007F:C350 01 x\n   1: 0100007F:1F92 00000000:0000 0A x\n   2: 0100007F:C350 0100007F:1F92 01 x\n";
        assert_eq!(desktop_viewers(tcp, 8082), 1);
        assert!(read_by_done("Read by 3 of 3") && !read_by_done("Read by 2 of 3"));
    }
}
