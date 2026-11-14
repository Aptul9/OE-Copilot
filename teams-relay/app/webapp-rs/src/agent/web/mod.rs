//! The browser agent of a slot (src/agent/main.ts, loop.ts, cdp.ts, scheduler.ts, context.ts): drives Teams web in the
//! slot's Chromium over DevTools, keeps data/N/messages.db for the web app, sends the push notifications. Today it runs
//! while a light account signs in again (its light-token job hands the sign-in to the light host) and for an account
//! kept in browser mode.

pub mod actions;
pub mod calls;
pub mod commands;
pub mod jobs;
pub mod media;
pub mod signin;

use super::api::TeamsAuth;
use super::browser::{Browser, Page, R, sleep};
use super::chatsvc::ChatSvc;
use super::config::Config;
use super::detector::Detector;
use super::http::Http;
use super::run::Notify;
use super::store::Store;
use super::teams::TeamsPage;
use super::teams::auth::PageAuth;
use super::{info, now_ms, now_s, warn};
use crate::agent::browser::scripts;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What the jobs and the commands work with (Agent of src/agent/context.ts).
pub struct Agent {
    pub config: Config,
    pub store: Rc<Store>,
    pub notifier: Rc<dyn Notify>,
    pub media: media::Media,
    pub detector: RefCell<Detector>,
    pub http: Http,
    pub chatsvc: ChatSvc,
    pub auth: PageAuth,
    tp: RefCell<Option<Rc<TeamsPage>>>,
    /// last health row written, None until the first one
    pub health: RefCell<Option<Value>>,
    /// a muted chat showed something new: the Activity feed is read at the next round it can
    pub mentions_due: Cell<bool>,
    /// the Activity button of the side bar could be clicked at the last health check
    pub rail_ready: Cell<bool>,
    /// the web app starts this account only to check it, every few hours
    pub checked_only: Box<dyn Fn() -> bool>,
    /// a light account: its browser runs only to sign it in again
    pub light_mode: Box<dyn Fn() -> bool>,
    /// a call in progress (the page records from the microphone), a call ringing (its toast shows)
    pub in_call: Cell<bool>,
    pub ringing: Cell<bool>,
    /// when the last call in progress ended (ms)
    pub call_over_at: Cell<Option<i64>>,
    /// the call placed from the app: the chat called and when (ms)
    pub outgoing: RefCell<Option<(String, i64)>>,
    /// since when the side bar shows without the chat list (ms), and the tries to go back to it
    pub loading_since: Cell<Option<i64>>,
    pub back_tries: Cell<u32>,
    pub back_at: Cell<i64>,
    /// the owner's last click, key or wheel turn on the page (ms); whether the pause it gives was logged
    pub owner_at: Cell<Option<i64>>,
    pub owner_paused: Cell<bool>,
    /// a viewer of the remote desktop with this account's window in front; the last time one was seen (ms); the
    /// connections of the desktop could not be read
    pub on_desktop: Cell<bool>,
    pub desktop_seen_at: Cell<i64>,
    pub desktop_unknown: Cell<bool>,
    /// chats read through chatsvc moments ago: the list shows them read (recentlyReadChats)
    pub recently_read: RefCell<HashMap<String, (String, i64)>>,
    /// chats chatsvc saved at the last try: only these can skip Teams (savedByApi)
    pub saved_by_api: RefCell<HashSet<String>>,
    /// the refresh token captured last for the light agent
    pub captured: RefCell<Option<String>>,
    /// pages given their init scripts, the browser given the microphone hook
    pub prepared: RefCell<HashSet<String>>,
    pub mic_hooked: Cell<bool>,
    pub sign_in: RefCell<signin::Told>,
}

pub struct Parts {
    pub config: Config,
    pub store: Rc<Store>,
    pub notifier: Rc<dyn Notify>,
    pub http: Http,
    pub checked_only: Box<dyn Fn() -> bool>,
    pub light_mode: Box<dyn Fn() -> bool>,
}

impl Agent {
    pub fn new(p: Parts) -> Rc<Agent> {
        Rc::new(Agent {
            media: media::Media::new(p.config.media_dir.clone(), p.config.files_dir.clone(), p.http.clone()),
            config: p.config,
            store: p.store,
            notifier: p.notifier,
            detector: RefCell::new(Detector::default()),
            http: p.http,
            chatsvc: ChatSvc::default(),
            auth: PageAuth::default(),
            tp: RefCell::new(None),
            health: RefCell::new(None),
            mentions_due: Cell::new(false),
            rail_ready: Cell::new(false),
            checked_only: p.checked_only,
            light_mode: p.light_mode,
            in_call: Cell::new(false),
            ringing: Cell::new(false),
            call_over_at: Cell::new(None),
            outgoing: RefCell::new(None),
            loading_since: Cell::new(None),
            back_tries: Cell::new(0),
            back_at: Cell::new(0),
            owner_at: Cell::new(None),
            owner_paused: Cell::new(false),
            on_desktop: Cell::new(false),
            desktop_seen_at: Cell::new(0),
            desktop_unknown: Cell::new(false),
            recently_read: RefCell::default(),
            saved_by_api: RefCell::default(),
            captured: RefCell::new(None),
            prepared: RefCell::default(),
            mic_hooked: Cell::new(false),
            sign_in: RefCell::default(),
        })
    }

    /// The Teams page of the current round.
    pub fn tp(&self) -> Option<Rc<TeamsPage>> {
        self.tp.borrow().clone()
    }

    pub fn page(&self) -> Option<Rc<Page>> {
        self.tp().map(|t| t.page.clone())
    }

    pub fn set_tp(&self, tp: Rc<TeamsPage>) {
        *self.tp.borrow_mut() = Some(tp);
    }

    /// teams of the last health row ("ok", "login", "loading", "err"; "" before the first).
    pub fn teams(&self) -> String {
        self.health.borrow().as_ref().and_then(|h| h["teams"].as_str().map(String::from)).unwrap_or_default()
    }

    pub fn teams_ok(&self) -> bool {
        self.teams() == "ok"
    }

    /// The chatsvc token of the page, None while there is none (getTeamsAuth).
    pub async fn page_auth(&self) -> Option<TeamsAuth> {
        let page = self.page()?;
        self.auth.auth(&page, &self.chatsvc).await
    }

    /// The conversation of a chat (resolveConversationId), with the token of the page.
    pub async fn conversation(&self, auth: &TeamsAuth, chat: &str, mid: Option<&str>) -> Option<String> {
        let page = self.page()?;
        self.auth.conversation(&self.http, &self.chatsvc, auth, &page, chat, mid).await
    }
}

/// When a job runs: every N rounds at an offset, or every N seconds.
#[derive(Clone, Copy, Debug)]
pub enum Every {
    Rounds(u64, u64),
    Seconds(i64),
}

/// The jobs of the loop, in the order they run in a round (agentJobs of src/agent/loop.ts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobName {
    Desktop,
    Page,
    LightToken,
    FreshSignIn,
    Input,
    BackToChats,
    Parking,
    Hook,
    Commands,
    ChatsFull,
    Chats,
    Activity,
    Identity,
    Health,
    SignIn,
    Conversation,
    ReadBy,
    Media,
    SelfCheck,
}

pub struct Job {
    pub name: JobName,
    pub every: Every,
    /// also on the Microsoft sign-in page; every other job needs the Teams page
    pub any_page: bool,
    /// a round it was due on but could not run on: it runs at the first round it can
    pub catch_up: bool,
}

/// Seconds between two inputs on the Teams page
const ACTIVE_EVERY: i64 = 60;
const ACTIVITY_RETRY_MS: i64 = 30_000;
/// A muted chat with something new reads the feed out of its turn, at most once in this many seconds
const MENTION_READ_GAP: i64 = 60;
/// Teams not ready this many seconds into a window of the self-check is the problem it reports
pub const SELF_CHECK_GRACE: i64 = 300;

/// The jobs of an agent, desktop first where the agent watches one.
pub fn agent_jobs(desktop: bool) -> Vec<Job> {
    use Every::*;
    use JobName::*;
    let j = |name, every, any_page, catch_up| Job { name, every, any_page, catch_up };
    let mut jobs = vec![];
    if desktop {
        jobs.push(j(Desktop, Rounds(1, 0), true, false));
    }
    jobs.extend([
        j(Page, Rounds(1, 0), false, false),
        j(LightToken, Rounds(1, 0), true, false),
        j(FreshSignIn, Rounds(1, 0), false, false),
        j(Input, Seconds(ACTIVE_EVERY), false, false),
        j(BackToChats, Rounds(1, 0), false, false),
        j(Parking, Rounds(5, 2), false, false),
        j(Hook, Rounds(1, 0), false, false),
        j(Commands, Rounds(1, 0), false, false),
        j(ChatsFull, Rounds(300, 1), false, false),
        j(Chats, Rounds(3, 0), false, false),
        j(Activity, Rounds(150, 5), false, true),
        j(Identity, Rounds(300, 7), false, false),
        j(Health, Rounds(5, 0), true, false),
        j(SignIn, Rounds(1, 0), true, false),
        j(Conversation, Rounds(1, 0), false, false),
        j(ReadBy, Rounds(2, 0), false, false),
        j(Media, Rounds(300, 31), false, false),
        j(SelfCheck, Rounds(1, 0), false, false),
    ]);
    jobs
}

/// When the feed of an account always on is read out of its turn: a few seconds after each call that ended, and once
/// more later (FeedAfterCalls).
#[derive(Default)]
pub struct FeedAfterCalls {
    at: RefCell<Vec<i64>>,
}

/// Seconds after a call ends to read the Activity feed, where Teams lists it once missed
pub const FEED_AFTER_CALL: [i64; 2] = [8, 40];

impl FeedAfterCalls {
    pub fn call_ended(&self) {
        let t = now_ms();
        let mut at = self.at.borrow_mut();
        at.extend(FEED_AFTER_CALL.iter().map(|s| t + s * 1000));
        at.sort();
    }

    pub fn due(&self) -> bool {
        self.at.borrow().first().is_some_and(|t| *t <= now_ms())
    }

    pub fn ran(&self) {
        let t = now_ms();
        self.at.borrow_mut().retain(|x| *x > t);
    }
}

/// The scheduler of the loop and what the jobs remember between rounds.
pub struct Scheduler {
    jobs: Vec<Job>,
    rounds: u64,
    last_run: HashMap<JobName, i64>,
    late: HashSet<JobName>,
    /// the chat parking last ran for
    parked_for: Option<String>,
    /// a feed read that failed is tried once more ACTIVITY_RETRY_MS later
    retry_at: i64,
    /// since when Teams was not ready while the self-check was due (ms), 0 while it is
    unready_since: i64,
    pub after_calls: Rc<FeedAfterCalls>,
}

impl Scheduler {
    pub fn new(jobs: Vec<Job>, after_calls: Rc<FeedAfterCalls>) -> Scheduler {
        Scheduler { jobs, rounds: 0, last_run: HashMap::new(), late: HashSet::new(), parked_for: None, retry_at: 0, unready_since: 0, after_calls }
    }

    fn idle(a: &Agent) -> bool {
        !a.in_call.get() && !a.ringing.get() && !jobs::owner_uses(a, now_ms())
    }

    fn free(a: &Agent) -> bool {
        a.teams_ok() && Self::idle(a)
    }

    fn mention_read_due(a: &Agent) -> bool {
        a.mentions_due.get() && now_s() - a.store.get_state("activity_ts").parse::<i64>().unwrap_or(0) >= MENTION_READ_GAP
    }

    fn force(&self, j: JobName, a: &Agent, want: &str) -> bool {
        match j {
            JobName::Parking => self.parked_for.as_deref() != Some(want),
            JobName::Activity => (self.retry_at > 0 && now_ms() >= self.retry_at) || self.after_calls.due() || Self::mention_read_due(a),
            JobName::Identity => a.store.get_state(crate::state::key::ME).is_empty(),
            _ => false,
        }
    }

    fn when(&mut self, j: JobName, a: &Agent, want: &str) -> bool {
        match j {
            JobName::LightToken | JobName::FreshSignIn => (a.light_mode)(),
            JobName::Input => !jobs::owner_uses(a, now_ms()),
            JobName::BackToChats => jobs::away_from_chats(a, now_ms()),
            JobName::Parking | JobName::ChatsFull => Self::free(a),
            JobName::Activity => Self::free(a) && a.rail_ready.get(),
            JobName::Identity => a.teams_ok(),
            JobName::SignIn => a.teams() == "login" && Self::idle(a),
            JobName::ReadBy => {
                !want.is_empty() && a.store.get_state(crate::state::key::ACTIVE_CHAT) == want && Self::free(a) && !a.store.has_pending_commands()
            }
            JobName::SelfCheck => self.self_check_now(a),
            _ => true,
        }
    }

    /// The automatic check waits for Teams, SELF_CHECK_GRACE at most; an account started only to be checked has its own.
    fn self_check_now(&mut self, a: &Agent) -> bool {
        if jobs::self_check_due(a).is_none() || (a.checked_only)() {
            self.unready_since = 0;
            return false;
        }
        if a.in_call.get() || a.ringing.get() {
            return false;
        }
        if a.teams_ok() {
            self.unready_since = 0;
            return true;
        }
        if self.unready_since == 0 {
            self.unready_since = now_ms();
        }
        now_ms() - self.unready_since >= SELF_CHECK_GRACE * 1000
    }

    fn due(&self, job: &Job, a: &Agent, want: &str) -> bool {
        if self.force(job.name, a, want) || self.late.contains(&job.name) {
            return true;
        }
        match job.every {
            Every::Seconds(s) => now_ms() - self.last_run.get(&job.name).copied().unwrap_or(i64::MIN / 2) >= s * 1000,
            Every::Rounds(n, offset) => self.rounds % n == offset,
        }
    }

    /// The due jobs in their order, then the next round; a failing job is reported and the round goes on.
    pub async fn run_round(&mut self, a: &Rc<Agent>, on_teams: bool, want: &str) {
        let jobs: Vec<(JobName, bool, bool)> = self.jobs.iter().filter(|j| self.due(j, a, want)).map(|j| (j.name, j.any_page, j.catch_up)).collect();
        for (name, any_page, catch_up) in jobs {
            if (!on_teams && !any_page) || !self.when(name, a, want) {
                if catch_up {
                    self.late.insert(name);
                }
                continue;
            }
            self.late.remove(&name);
            self.last_run.insert(name, now_ms());
            if let Err(e) = self.run(name, a, want).await {
                warn("job", &e, &[("job", format!("{name:?}").to_lowercase().into())]);
            }
        }
        self.rounds += 1;
    }

    async fn run(&mut self, name: JobName, a: &Rc<Agent>, want: &str) -> R<()> {
        let page = a.page().ok_or("no page")?;
        match name {
            JobName::Desktop => jobs::watch_desktop(a).await,
            JobName::Page => jobs::prepare_page(a).await?,
            JobName::LightToken => {
                signin::capture_light_token(a).await;
            }
            JobName::FreshSignIn => {
                signin::fresh_sign_in(a).await;
            }
            JobName::Input => jobs::keep_active(a).await,
            JobName::BackToChats => jobs::back_to_chats(a).await,
            JobName::Parking => {
                self.parked_for = Some(want.to_string());
                page.as_agent(jobs::park(a, want)).await;
            }
            JobName::Hook => jobs::drain_hook(a).await?,
            JobName::Commands => commands::run_pending_commands(a).await,
            JobName::ChatsFull => {
                jobs::scan_chats_full(a).await;
            }
            JobName::Chats => {
                jobs::scan_chats(a).await;
                jobs::prefetch_chats(a).await;
            }
            JobName::Activity => {
                let retry = self.retry_at > 0;
                self.retry_at = 0;
                self.after_calls.ran();
                a.mentions_due.set(false);
                if page.as_agent(jobs::read_activity(a, 0)).await.is_none() {
                    if !retry {
                        self.retry_at = now_ms() + ACTIVITY_RETRY_MS;
                    }
                    return Ok(());
                }
                if !(a.checked_only)() {
                    super::run::push_missed_calls(&a.store, &a.notifier).await;
                    super::run::push_mentions(&a.store, &a.notifier).await;
                }
            }
            JobName::Identity => jobs::save_identity(a).await,
            JobName::Health => {
                jobs::update_health(a).await;
            }
            JobName::SignIn => signin::try_sign_in(a).await,
            JobName::Conversation => {
                let chat = a.store.get_state(crate::state::key::ACTIVE_CHAT);
                if !chat.is_empty() {
                    jobs::save_open_chat(a, &chat).await;
                }
            }
            JobName::ReadBy => {
                page.as_agent(jobs::prefetch_read_by(a, want)).await;
            }
            JobName::Media => jobs::prune_media(a),
            JobName::SelfCheck => jobs::scheduled_self_check(a).await,
        }
        Ok(())
    }
}

/// The Chromium of a slot, reached over its DevTools port (CdpBrowser). The supervisor owns it: the agent never launches
/// it, and closes it only when its own view of it is stuck. A lost connection is made again.
pub struct CdpBrowser {
    url: String,
    current: RefCell<Option<Rc<Browser>>>,
}

/// Without a Teams tab for this long the agent exits and the supervisor starts it again
const NO_TAB_EXIT_MS: i64 = 60_000;

impl CdpBrowser {
    pub fn new(url: &str) -> CdpBrowser {
        CdpBrowser { url: url.to_string(), current: RefCell::new(None) }
    }

    /// The browser of the moment, connected again (every 3 s) while it does not answer.
    pub async fn browser(&self) -> Rc<Browser> {
        if let Some(b) = self.current.borrow().clone() {
            if b.connected() {
                return b;
            }
            warn("cdp", "connection lost", &[]);
        }
        loop {
            match Browser::connect(&self.url).await {
                Ok(b) => {
                    let _ = b.grant_notifications().await;
                    info("cdp", "connected", &[("cdp", self.url.as_str().into())]);
                    *self.current.borrow_mut() = Some(b.clone());
                    return b;
                }
                Err(e) => {
                    warn("cdp", &format!("waiting: {e}"), &[]);
                    sleep(3000).await;
                }
            }
        }
    }

    /// No Teams tab for `ms`: past NO_TAB_EXIT_MS the agent exits and the supervisor starts it again; a browser that shows
    /// a page the agent cannot see is closed first.
    pub async fn no_teams_tab(&self, ms: i64, url: &str) {
        if ms <= NO_TAB_EXIT_MS {
            return;
        }
        if url.is_empty() && self.chromium_shows_page().await {
            warn("agent", "Chromium shows a page the agent cannot see: closing the browser, the supervisor starts it again", &[]);
            let b = self.current.borrow().clone();
            if let Some(b) = b
                && let Err(e) = b.close_browser().await
            {
                warn("cdp", &format!("browser not closed: {e}"), &[]);
            }
        }
        warn("agent", "no Teams tab for 60 s: exiting, the supervisor starts the agent again", &[]);
        std::process::exit(1);
    }

    async fn chromium_shows_page(&self) -> bool {
        let Ok(url) = url::Url::parse(&self.url).and_then(|u| u.join("/json/list")) else { return false };
        let Ok(r) = reqwest::Client::new().get(url).timeout(std::time::Duration::from_secs(5)).send().await else { return false };
        let Ok(body) = r.bytes().await else { return false };
        let targets: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        jobs::shows_page(&targets)
    }
}

/// A tab on a gateway of the proxy of the tenant (its monitored-access notice).
static MCAS_GATEWAY: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^https://[a-z0-9-]+\.access\.mcas\.ms/").expect("fixed"));

/// The agent until it exits: about one round a second, the call watch on a timer of its own (runAgent).
pub async fn run_agent(a: Rc<Agent>, source: CdpBrowser) {
    let after_calls = Rc::new(FeedAfterCalls::default());
    let mut scheduler = Scheduler::new(agent_jobs(a.config.desktop.is_some()), after_calls.clone());
    calls::CallWatch::start(a.clone(), after_calls);
    let mut away: Option<(i64, String)> = None;
    let mut current: Option<String> = None;
    loop {
        let browser = source.browser().await;
        let tabs = browser.tabs();
        match jobs::pick_teams_page(&tabs) {
            None => {
                let mut continued = false;
                for (id, url) in &tabs {
                    if !MCAS_GATEWAY.is_match(url) {
                        continue;
                    }
                    if let Ok(p) = browser.page(id).await
                        && signin::continue_mcas(&p).await
                    {
                        info("SESSION", "MCAS monitored-access notice continued", &[]);
                        continued = true;
                        break;
                    }
                }
                if continued {
                    away = None;
                    sleep(1000).await;
                    continue;
                }
                // the page that is not blank, if any: a sign-in on another host looks like this
                let url = tabs.iter().map(|t| t.1.clone()).find(|u| !jobs::is_blank(u)).unwrap_or_default();
                jobs::no_tab_health(&a, &url).await;
                if away.as_ref().is_none_or(|(_, u)| u.is_empty() != url.is_empty()) {
                    let host = jobs::host_of(&url);
                    if url.is_empty() {
                        info("agent", "blank tab", &[]);
                    } else {
                        info("agent", "not on Teams", &[("host", host.into())]);
                    }
                    away = Some((now_ms(), url.clone()));
                }
                let since = away.as_ref().map_or(now_ms(), |(s, _)| *s);
                source.no_teams_tab(now_ms() - since, &url).await;
                sleep(3000).await;
                continue;
            }
            Some(id) => {
                away = None;
                match browser.page(&id).await {
                    Ok(page) => {
                        if current.as_deref() != Some(id.as_str()) || a.page().is_none_or(|p| !Rc::ptr_eq(&p, &page)) {
                            a.set_tp(Rc::new(TeamsPage::new(page.clone(), a.store.clone())));
                            // a new tab: its side bar is not known clickable until the next health check
                            a.rail_ready.set(false);
                            current = Some(id.clone());
                        }
                        let on_teams = jobs::is_teams_url(&page.url());
                        let want = if on_teams { jobs::wanted(&a) } else { String::new() };
                        scheduler.run_round(&a, on_teams, &want).await;
                    }
                    Err(e) => {
                        warn("loop", &e, &[]);
                        sleep(2000).await;
                    }
                }
            }
        }
        for _ in 0..10 {
            if a.tp().is_some() && a.store.has_pending_commands() {
                commands::run_pending_commands(&a).await;
            }
            sleep(100).await;
        }
    }
}

/// The agent of the slot of `env` until it exits (main of src/agent/main.ts).
pub async fn start_agent(env: super::config::Env) -> Result<(), String> {
    use crate::push::{FcmSender, Notifier, NotifierOptions, load_service_account, load_vapid_keys};
    let config = super::config::load(&env).map_err(|e| format!("configuration: {e}"))?;
    let store = Rc::new(Store::open(&config.db_path).map_err(|e| e.to_string())?);
    let push = &config.push;
    let vapid = load_vapid_keys(&push.vapid_private_key_file, &push.vapid_app_key_file)?;
    if vapid.is_none() {
        warn("push", "no VAPID private key: push notifications off", &[("file", push.vapid_private_key_file.as_str().into())]);
    }
    let sa = load_service_account(&push.fcm_credentials)?;
    let fcm = sa.is_some();
    let notifier = Notifier::new(NotifierOptions {
        store_file: config.db_path.clone(),
        devices: Box::new(super::devices::AppDb::new(config.app_db.clone(), config.slot)),
        vapid: vapid.clone(),
        subject: push.vapid_subject.clone(),
        ntfy: push.ntfy.clone(),
        fcm: sa.map(FcmSender::new),
        answerable: config.native_calls,
    });
    info(
        "agent",
        "start",
        &[
            ("slot", config.slot.into()),
            ("cdp", config.cdp.as_str().into()),
            ("push", vapid.is_some().into()),
            ("fcm", fcm.into()),
            ("ntfy", push.ntfy.is_some().into()),
        ],
    );
    let app = Rc::new(super::devices::AppDb::new(config.app_db.clone(), config.slot));
    let light = app.clone();
    let source = CdpBrowser::new(&config.cdp);
    let a = Agent::new(Parts {
        config,
        store,
        notifier: Rc::new(notifier),
        http: super::http::slot_http(),
        checked_only: Box::new(move || app.checked_only()),
        light_mode: Box::new(move || light.light_mode()),
    });
    run_agent(a, source).await;
    Ok(())
}

/// The page scripts a Rust release carries, checked at build (--check of the image): every one a function.
pub fn check_scripts() -> usize {
    scripts::count()
}

#[cfg(test)]
mod tests;
