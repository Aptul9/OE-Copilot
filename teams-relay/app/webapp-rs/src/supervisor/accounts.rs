//! Browser and agent of every Teams account, started, stopped and wiped on request of the web app; the light agent alone
//! for a light account, between the runs of its browser (src/supervisor/accounts.ts). The two never run together: both
//! would push the same messages.

use super::broker::{Binding, broker_env};
use super::config::{AccountsConfig, Env};
use super::light_host::{LightHost, LightHostLike, fork_host};
use super::process::{Command, Lines, ProcessLike, Supervised};
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct AccountError {
    /// the HTTP status of the control API
    pub status: u16,
    pub message: String,
}

impl AccountError {
    pub fn new(status: u16, message: &str) -> AccountError {
        AccountError { status, message: message.into() }
    }
}

const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Stale locks of a Chromium that ran elsewhere: Chromium refuses a profile whose lock names another host.
const LOCKS: [&str; 3] = ["SingletonLock", "SingletonSocket", "SingletonCookie"];

/// The Windows identity verified with the BIP sign-in flow (src/local/browser-identity.ts).
const USER_AGENT: &str = "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";

pub fn profile_dir(n: i64, cfg: &AccountsConfig) -> PathBuf {
    cfg.profiles_dir.join(n.to_string())
}

fn env_of(pairs: &[(&str, String)]) -> Env {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// HOME is the profile directory, as /config was in the chromium-N container of earlier releases: the Chromium profile
/// stays at $HOME/.config/chromium and the accounts signed in there stay signed in.
pub fn browser_command(n: i64, cfg: &AccountsConfig) -> Command {
    let home = profile_dir(n, cfg);
    let mut env = env_of(&[("PATH", PATH.into()), ("HOME", home.display().to_string())]);
    env.extend(cfg.session.clone());
    env.extend(cfg.browser_env.clone());
    let args = [
        USER_AGENT.to_string(),
        "--ozone-platform=wayland".into(),
        format!("--class=teamsrelay-{n}"),
        format!("--remote-debugging-port={}", i64::from(cfg.cdp_base_port) + n),
        "--password-store=basic".into(),
        "--simulate-outdated-no-au=Tue, 31 Dec 2099 23:59:59 GMT".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--start-maximized".into(),
        // the windows cover each other on the one desktop: a covered window keeps running at full speed
        "--disable-backgrounding-occluded-windows".into(),
        "--disable-renderer-backgrounding".into(),
        "--disable-background-timer-throttling".into(),
        "https://teams.microsoft.com/".into(),
    ];
    Command { file: cfg.chromium.clone(), args: args.to_vec(), env: Some(env), cwd: Some(home), uid: Some(cfg.uid), gid: Some(cfg.gid) }
}

/// The agent runs as root: data/, vapid/ and fcm/ are under /root, out of reach of the browsers.
pub fn agent_command(n: i64, cfg: &AccountsConfig) -> Command {
    let p = |dir: &Path, rest: &str| dir.join(rest).display().to_string();
    let mut env = env_of(&[("PATH", PATH.into()), ("HOME", "/root".into()), ("NODE_ENV", "production".into())]);
    env.extend(cfg.agent_env.clone());
    env.extend(env_of(&[
        ("ACCOUNT", n.to_string()),
        ("CDP", format!("http://127.0.0.1:{}", i64::from(cfg.cdp_base_port) + n)),
        ("DB_PATH", p(&cfg.data_dir, &format!("{n}/messages.db"))),
        ("APP_DB", p(&cfg.data_dir, "app.db")),
        ("VAPID_PRIVATE", p(&cfg.vapid_dir, "private_key.pem")),
        ("VAPID_APPKEY", p(&cfg.vapid_dir, "appkey.txt")),
        ("FCM_CREDENTIALS", p(&cfg.fcm_dir, "service-account.json")),
        // the owner on the remote desktop with the window of this account in front
        ("DESKTOP_PORT", cfg.desktop_port.to_string()),
        ("DESKTOP_UID", cfg.uid.to_string()),
        ("DESKTOP_GID", cfg.gid.to_string()),
        ("DESKTOP_APP_ID", format!("teamsrelay-{n}")),
        ("XDG_RUNTIME_DIR", cfg.session.get("XDG_RUNTIME_DIR").cloned().unwrap_or_default()),
        ("WAYLAND_DISPLAY", cfg.session.get("WAYLAND_DISPLAY").cloned().unwrap_or_default()),
    ]));
    Command { file: cfg.node.clone(), args: vec![cfg.agent_script.clone()], env: Some(env), ..Default::default() }
}

/// The light agent of a light account: the environment of the agent, without its browser and its desktop; bound to a
/// shared Microsoft account, the broker of that account.
pub fn light_command(n: i64, cfg: &AccountsConfig, binding: Option<&Binding>) -> Command {
    let gone = ["CDP", "DESKTOP_PORT", "DESKTOP_UID", "DESKTOP_GID", "DESKTOP_APP_ID", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"];
    let mut env: Env = agent_command(n, cfg).env.unwrap_or_default().into_iter().filter(|(k, _)| !gone.contains(&k.as_str())).collect();
    env.extend(broker_env(binding, cfg.shared_broker.as_ref()));
    // NATIVE_CALLS: 1 for every account, else the list of the slots that have calls
    if let Some(calls) = env.get("NATIVE_CALLS").filter(|v| *v != "1").cloned() {
        let on = calls.split(',').any(|s| s == n.to_string());
        env.insert("NATIVE_CALLS".into(), if on { "1" } else { "0" }.into());
    }
    Command { file: cfg.node.clone(), args: vec![cfg.light_script.clone()], env: Some(env), ..Default::default() }
}

/// Makes the process `name` of `command`, its output lines to the given function (none: no output kept).
pub type MakeProcess = Rc<dyn Fn(&str, Box<dyn Fn() -> Command>, Option<Lines>) -> Rc<dyn ProcessLike>>;
/// Brings the window of account N to the front of the desktop.
pub type Focus = Rc<dyn Fn(i64) -> LocalBoxFuture<'static, bool>>;

pub struct Deps {
    pub log: Lines,
    pub process: Option<MakeProcess>,
    pub focus: Option<Focus>,
    pub light_host: Option<Rc<dyn LightHostLike>>,
}

struct Account {
    browser: Rc<dyn ProcessLike>,
    agent: Rc<dyn ProcessLike>,
    light: Rc<dyn ProcessLike>,
}

pub struct Accounts {
    cfg: Rc<AccountsConfig>,
    log: Lines,
    accounts: RefCell<BTreeMap<i64, Rc<Account>>>,
    /// the shared account each bound slot runs on, as the web app sent it with its last light start
    bindings: Rc<RefCell<HashMap<i64, Binding>>>,
    queues: RefCell<HashMap<i64, Rc<tokio::sync::Mutex<()>>>>,
    make: MakeProcess,
    focus: Focus,
    host: Option<Rc<dyn LightHostLike>>,
}

/// A number of the URL as an account: positive integers only.
fn check(n: Option<i64>) -> Result<i64, AccountError> {
    match n {
        Some(n) if n >= 1 => Ok(n),
        Some(n) => Err(AccountError::new(404, &format!("No account {n}: accounts are numbered from 1"))),
        None => Err(AccountError::new(404, "No account NaN: accounts are numbered from 1")),
    }
}

impl Accounts {
    pub fn new(cfg: AccountsConfig, deps: Deps) -> Rc<Accounts> {
        let cfg = Rc::new(cfg);
        let log = deps.log.clone();
        let say: Lines = {
            let log = log.clone();
            Rc::new(move |m: &str| log(&crate::push::notify::format("supervisor", m, &[])))
        };
        let make = deps.process.unwrap_or_else(|| {
            let say = say.clone();
            Rc::new(move |name: &str, command, output| Supervised::with_defaults(name, command, say.clone(), output) as Rc<dyn ProcessLike>)
        });
        let focus = deps.focus.unwrap_or_else(|| {
            let cfg = cfg.clone();
            Rc::new(move |n| Box::pin(focus_window(n, cfg.clone())))
        });
        let host = match deps.light_host {
            Some(h) => Some(h),
            None if cfg.light_host => {
                // the slots bring their own environment; the host only needs what any agent process gets, and the Rust
                // one the Node its calling workers run on
                let mut env = env_of(&[("PATH", PATH.into()), ("HOME", "/root".into()), ("NODE_ENV", "production".into())]);
                env.extend(cfg.agent_env.clone());
                if cfg.light_bin.is_some() {
                    env.insert("NODE_BIN".into(), cfg.node.clone());
                }
                let (c, out) = (cfg.clone(), log.clone());
                let spawn: super::light_host::SpawnHost = Rc::new(move || fork_host(&c.light_script, &c.node, &env, out.clone(), c.light_bin.as_deref()));
                Some(LightHost::with_defaults(spawn, say.clone()) as Rc<dyn LightHostLike>)
            }
            None => None,
        };
        Rc::new(Accounts { cfg, log, accounts: RefCell::default(), bindings: Rc::default(), queues: RefCell::default(), make, focus, host })
    }

    fn say(&self, m: &str) {
        (self.log)(&crate::push::notify::format("supervisor", m, &[]));
    }

    /// The requests of one account run one at a time, in the order they came.
    fn queue(&self, n: i64) -> Rc<tokio::sync::Mutex<()>> {
        self.queues.borrow_mut().entry(n).or_default().clone()
    }

    fn account(&self, n: i64) -> Rc<Account> {
        if let Some(a) = self.accounts.borrow().get(&n) {
            return a.clone();
        }
        let (cfg, log) = (self.cfg.clone(), self.log.clone());
        let c = cfg.clone();
        let browser = (self.make)(&format!("browser-{n}"), Box::new(move || browser_command(n, &c)), None);
        let c = cfg.clone();
        let prefixed = |log: Lines| -> Lines { Rc::new(move |line: &str| log(&format!("[{n}] {line}"))) };
        let agent = (self.make)(&format!("agent-{n}"), Box::new(move || agent_command(n, &c)), Some(prefixed(log.clone())));
        let bindings = self.bindings.clone();
        let light_of: Box<dyn Fn() -> Command> = Box::new(move || light_command(n, &cfg, bindings.borrow().get(&n)));
        let light = match &self.host {
            Some(h) => h.slot(n, light_of),
            None => (self.make)(&format!("light-{n}"), light_of, Some(prefixed(log))),
        };
        let a = Rc::new(Account { browser, agent, light });
        self.accounts.borrow_mut().insert(n, a.clone());
        a
    }

    pub async fn start(&self, n: Option<i64>) -> Result<(), AccountError> {
        let n = check(n)?;
        let q = self.queue(n);
        let _turn = q.lock().await;
        // a slot bound to a shared account has no browser sign-in: its session lives in the OE calendar agent
        if self.bindings.borrow().contains_key(&n) {
            return Err(AccountError::new(409, &format!("Account {n} uses a shared Microsoft account: it runs without a browser")));
        }
        let a = self.account(n);
        if a.light.active() {
            a.light.clone().stop().await;
        }
        let running = a.browser.active() && a.agent.active();
        if !a.browser.active() {
            self.prepare_profile(n);
        }
        a.browser.start();
        a.agent.start();
        if !running {
            self.say(&format!("account {n} started"));
        }
        Ok(())
    }

    pub async fn signin(&self, n: Option<i64>) -> Result<(), AccountError> {
        let n = check(n)?;
        let q = self.queue(n);
        let _turn = q.lock().await;
        if self.bindings.borrow().contains_key(&n) {
            return Err(AccountError::new(409, "Shared accounts have no local browser profile"));
        }
        let a = self.account(n);
        if !a.browser.active() {
            self.prepare_profile(n);
        }
        a.browser.start();
        Ok(())
    }

    /// The light agent alone: browser and agent stop first. binding: the shared account of a bound slot (every light
    /// start of the web app sends it again); a light agent already running on another binding starts again on it.
    pub async fn light(&self, n: Option<i64>, binding: Option<Binding>) -> Result<(), AccountError> {
        let n = check(n)?;
        let q = self.queue(n);
        let _turn = q.lock().await;
        if binding.is_some() && self.cfg.shared_broker.is_none() {
            return Err(AccountError::new(
                409,
                &format!("Account {n} uses a shared Microsoft account, but this container has no SHARED_BROKER_URL and SHARED_BROKER_KEY"),
            ));
        }
        let a = self.account(n);
        let same = self.bindings.borrow().get(&n) == binding.as_ref();
        match &binding {
            Some(b) => self.bindings.borrow_mut().insert(n, b.clone()),
            None => self.bindings.borrow_mut().remove(&n),
        };
        if a.light.active() && same {
            return Ok(());
        }
        if a.light.active() {
            a.light.clone().stop().await;
        }
        if a.agent.active() {
            a.agent.clone().stop().await;
        }
        if a.browser.active() {
            a.browser.clone().stop().await;
        }
        a.light.start();
        let on = binding.map(|b| format!(" on the shared account {}", b.account)).unwrap_or_default();
        self.say(&format!("account {n} started without its browser{on}"));
        Ok(())
    }

    /// Agent first: it would log the browser as gone.
    pub async fn stop(&self, n: Option<i64>) -> Result<(), AccountError> {
        let n = check(n)?;
        let q = self.queue(n);
        let _turn = q.lock().await;
        let Some(a) = self.accounts.borrow().get(&n).cloned() else { return Ok(()) };
        if a.light.active() {
            a.light.clone().stop().await;
        }
        a.agent.clone().stop().await;
        a.browser.clone().stop().await;
        self.say(&format!("account {n} stopped"));
        Ok(())
    }

    /// Deletes the Microsoft session of the account: its profile directory, every entry first, then the directory
    /// itself. A directory that is a mount point (one volume per account) stays, empty.
    pub async fn wipe(&self, n: Option<i64>) -> Result<(), AccountError> {
        let n = check(n)?;
        let q = self.queue(n);
        let _turn = q.lock().await;
        if let Some(a) = self.accounts.borrow().get(&n)
            && (a.browser.active() || a.agent.active() || a.light.active())
        {
            return Err(AccountError::new(409, &format!("Account {n} is running: stop it first")));
        }
        // the slot is free for anyone: a shared account it ran on goes with its data
        self.bindings.borrow_mut().remove(&n);
        let dir = profile_dir(n, &self.cfg);
        let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(()) };
        for e in entries.flatten() {
            let p = e.path();
            let _ = if p.is_dir() && !p.is_symlink() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
        let mut left: Vec<String> =
            std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        if !left.is_empty() {
            left.sort();
            return Err(AccountError::new(500, &format!("account {n}: not wiped, left: {}", left.join(" "))));
        }
        // a mount point: nothing of the session is left in it
        let _ = std::fs::remove_dir(&dir);
        self.say(&format!("account {n} wiped"));
        Ok(())
    }

    pub async fn show(&self, n: Option<i64>) -> Result<bool, AccountError> {
        let n = check(n)?;
        let running = self.accounts.borrow().get(&n).is_some_and(|a| a.browser.running());
        if !running {
            return Ok(false);
        }
        Ok((self.focus)(n).await)
    }

    pub fn status(&self) -> Value {
        let state = |p: &Rc<dyn ProcessLike>| json!({ "running": p.running(), "pid": p.pid(), "restarts": p.restarts(), "lastExit": p.last_exit() });
        Value::Array(
            self.accounts
                .borrow()
                .iter()
                .map(|(n, a)| json!({ "account": n, "browser": state(&a.browser), "agent": state(&a.agent), "light": state(&a.light) }))
                .collect(),
        )
    }

    /// Whether any browser runs: the desktop stays up while one does.
    pub fn browser_running(&self) -> bool {
        self.accounts.borrow().values().any(|a| a.browser.running())
    }

    pub async fn stop_all(&self) {
        let all: Vec<i64> = self.accounts.borrow().keys().copied().collect();
        futures_util::future::join_all(all.into_iter().map(|n| self.stop(Some(n)))).await;
    }

    fn prepare_profile(&self, n: i64) {
        let dir = profile_dir(n, &self.cfg);
        let _ = std::fs::create_dir_all(&dir);
        // A directory made by Docker belongs to root: the browser writes its profile there. What is inside stays.
        // SAFETY: geteuid has no preconditions
        if unsafe { libc::geteuid() } == 0
            && let Ok(m) = std::fs::metadata(&dir)
        {
            use std::os::unix::fs::MetadataExt;
            if m.uid() != self.cfg.uid || m.gid() != self.cfg.gid {
                let _ = std::os::unix::fs::chown(&dir, Some(self.cfg.uid), Some(self.cfg.gid));
            }
        }
        for lock in LOCKS {
            let p = dir.join(".config").join("chromium").join(lock);
            if std::fs::symlink_metadata(&p).is_ok() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// wlrctl talks to the compositor of the desktop, which accepts only its own user.
async fn focus_window(n: i64, cfg: Rc<AccountsConfig>) -> bool {
    let mut cmd = tokio::process::Command::new(&cfg.wlrctl);
    cmd.args(["toplevel", "focus", &format!("app_id:teamsrelay-{n}")]).env_clear().env("PATH", PATH).envs(&cfg.session).uid(cfg.uid).gid(cfg.gid);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    matches!(tokio::time::timeout(std::time::Duration::from_secs(5), cmd.status()).await, Ok(Ok(s)) if s.success())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agent::testing::{run, temp_dir};
    use std::cell::Cell;
    use std::time::Duration;

    /// Stand-in for a supervised process: records the calls, a stop can take its time.
    pub struct FakeProcess {
        name: String,
        calls: Rc<RefCell<Vec<String>>>,
        stop_ms: u64,
        wanted: Cell<bool>,
        pub command: Box<dyn Fn() -> Command>,
        pub commands: Rc<RefCell<Vec<Command>>>,
    }

    impl ProcessLike for FakeProcess {
        fn name(&self) -> String {
            self.name.clone()
        }
        fn running(&self) -> bool {
            self.wanted.get()
        }
        fn active(&self) -> bool {
            self.wanted.get()
        }
        fn pid(&self) -> Option<u32> {
            self.wanted.get().then_some(4242)
        }
        fn restarts(&self) -> u32 {
            0
        }
        fn last_exit(&self) -> Option<String> {
            None
        }
        fn start(&self) {
            self.calls.borrow_mut().push(format!("start {}", self.name));
            self.commands.borrow_mut().push((self.command)());
            self.wanted.set(true);
        }
        fn stop(self: Rc<Self>) -> LocalBoxFuture<'static, ()> {
            Box::pin(async move {
                self.calls.borrow_mut().push(format!("stop {}", self.name));
                tokio::time::sleep(Duration::from_millis(self.stop_ms)).await;
                self.wanted.set(false);
            })
        }
    }

    pub struct World {
        pub calls: Rc<RefCell<Vec<String>>>,
        pub commands: Rc<RefCell<Vec<Command>>>,
        pub lines: Rc<RefCell<Vec<String>>>,
        pub focused: Rc<RefCell<Vec<i64>>>,
    }

    pub fn cfg() -> AccountsConfig {
        let root = temp_dir();
        let mut c = crate::supervisor::config::tests::of(&[
            ("TZ", "Europe/Rome"),
            ("LANG", "en_US.UTF-8"),
            ("VAPID_SUBJECT", "mailto:me@example.com"),
            ("NTFY_ENABLED", "0"),
        ])
        .expect("config")
        .accounts;
        c.profiles_dir = root.join("profiles");
        c.light_host = false;
        std::fs::create_dir_all(&c.profiles_dir).expect("profiles");
        c
    }

    pub fn accounts_with(cfg: AccountsConfig, stop_ms: u64, light_host: Option<Rc<dyn LightHostLike>>) -> (Rc<Accounts>, World) {
        let w = World { calls: Rc::default(), commands: Rc::default(), lines: Rc::default(), focused: Rc::default() };
        let (calls, commands, lines, focused) = (w.calls.clone(), w.commands.clone(), w.lines.clone(), w.focused.clone());
        let make: MakeProcess = Rc::new(move |name: &str, command, _| {
            Rc::new(FakeProcess { name: name.into(), calls: calls.clone(), stop_ms, wanted: Cell::new(false), command, commands: commands.clone() })
                as Rc<dyn ProcessLike>
        });
        let deps = Deps {
            log: Rc::new(move |l: &str| lines.borrow_mut().push(l.into())),
            process: Some(make),
            focus: Some(Rc::new(move |n| {
                focused.borrow_mut().push(n);
                Box::pin(async { true })
            })),
            light_host,
        };
        (Accounts::new(cfg, deps), w)
    }

    fn calls(w: &World) -> Vec<String> {
        w.calls.borrow().clone()
    }

    fn profile(c: &AccountsConfig, n: i64, files: &[&str]) -> PathBuf {
        let dir = profile_dir(n, c);
        for f in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
            std::fs::write(p, "x").expect("file");
        }
        dir
    }

    #[test]
    fn the_commands_of_browser_agent_and_light_agent() {
        let c = cfg();
        let b = browser_command(2, &c);
        let home = c.profiles_dir.join("2");
        assert_eq!((b.file.as_str(), b.uid, b.gid, b.cwd.as_ref()), ("/usr/bin/chromium", Some(1000), Some(1000), Some(&home)));
        let env = b.env.clone().expect("env");
        assert_eq!(
            (
                env["HOME"].as_str(),
                env["XDG_RUNTIME_DIR"].as_str(),
                env["WAYLAND_DISPLAY"].as_str(),
                env["DISPLAY"].as_str(),
                env["TZ"].as_str(),
                env["LANG"].as_str()
            ),
            (home.to_str().expect("p"), "/config/.XDG", "wayland-0", ":0", "Europe/Rome", "en_US.UTF-8")
        );
        for a in ["--remote-debugging-port=9223", "--class=teamsrelay-2", "--ozone-platform=wayland", "--disable-backgrounding-occluded-windows"] {
            assert!(b.args.contains(&a.to_string()), "{a}");
        }
        assert_eq!(b.args.last().map(String::as_str), Some("https://teams.microsoft.com/"));
        let joined = b.args.join(" ");
        assert!(!joined.contains("--no-sandbox") && !joined.contains("--remote-allow-origins"));

        let a = agent_command(2, &c);
        assert_eq!((a.file.as_str(), a.args.clone(), a.uid), ("/usr/local/bin/node", vec!["/app/agent.cjs".to_string()], None));
        let e = a.env.expect("env");
        for (k, v) in [
            ("ACCOUNT", "2"),
            ("CDP", "http://127.0.0.1:9223"),
            ("DB_PATH", "/root/data/2/messages.db"),
            ("APP_DB", "/root/data/app.db"),
            ("VAPID_PRIVATE", "/root/vapid/private_key.pem"),
            ("VAPID_APPKEY", "/root/vapid/appkey.txt"),
            ("FCM_CREDENTIALS", "/root/fcm/service-account.json"),
            ("VAPID_SUBJECT", "mailto:me@example.com"),
            ("TZ", "Europe/Rome"),
            ("DESKTOP_PORT", "8082"),
            ("DESKTOP_UID", "1000"),
            ("DESKTOP_APP_ID", "teamsrelay-2"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ] {
            assert_eq!(e.get(k).map(String::as_str), Some(v), "{k}");
        }

        let l = light_command(2, &c, None);
        assert_eq!(l.args, ["/app/light.cjs"]);
        let e = l.env.expect("env");
        assert_eq!((e["ACCOUNT"].as_str(), e["DB_PATH"].as_str(), e["APP_DB"].as_str()), ("2", "/root/data/2/messages.db", "/root/data/app.db"));
        for k in ["CDP", "DESKTOP_PORT", "DESKTOP_APP_ID", "WAYLAND_DISPLAY"] {
            assert!(!e.contains_key(k), "{k}");
        }
        let mut c = cfg();
        c.agent_env.insert("NATIVE_CALLS".into(), "7".into());
        assert_eq!(
            (light_command(7, &c, None).env.expect("e")["NATIVE_CALLS"].as_str(), light_command(4, &c, None).env.expect("e")["NATIVE_CALLS"].as_str()),
            ("1", "0")
        );
        c.agent_env.insert("NATIVE_CALLS".into(), "1".into());
        assert_eq!(light_command(4, &c, None).env.expect("e")["NATIVE_CALLS"], "1");
    }

    #[test]
    fn the_light_agent_runs_alone_and_a_start_stops_it() {
        run(async {
            let (a, w) = accounts_with(cfg(), 0, None);
            a.signin(Some(1)).await.expect("signin");
            assert_eq!(calls(&w), ["start browser-1"]);
            assert!(a.show(Some(1)).await.expect("show"));
            a.stop(Some(1)).await.expect("stop");
            w.calls.borrow_mut().clear();
            a.start(Some(1)).await.expect("start");
            a.light(Some(1), None).await.expect("light");
            a.light(Some(1), None).await.expect("light");
            assert_eq!(calls(&w), ["start browser-1", "start agent-1", "stop agent-1", "stop browser-1", "start light-1"]);
            w.calls.borrow_mut().clear();
            a.start(Some(1)).await.expect("start");
            assert_eq!(calls(&w), ["stop light-1", "start browser-1", "start agent-1"]);
            a.light(Some(1), None).await.expect("light");
            w.calls.borrow_mut().clear();
            a.stop(Some(1)).await.expect("stop");
            assert_eq!(calls(&w), ["stop light-1", "stop agent-1", "stop browser-1"]);
            a.light(Some(3), None).await.expect("light");
            assert_eq!(a.wipe(Some(3)).await.unwrap_err().status, 409);
            assert_eq!(a.status()[1]["light"]["running"], true);
        });
    }

    #[test]
    fn starts_logs_queues_and_numbers() {
        run(async {
            let c = cfg();
            let dir = profile(
                &c,
                2,
                &[".config/chromium/SingletonLock", ".config/chromium/SingletonSocket", ".config/chromium/SingletonCookie", ".config/chromium/Default/Cookies"],
            );
            let (a, w) = accounts_with(c.clone(), 100, None);
            a.start(Some(2)).await.expect("start");
            assert_eq!(
                std::fs::read_dir(dir.join(".config/chromium")).expect("dir").flatten().map(|e| e.file_name().into_string().expect("name")).collect::<Vec<_>>(),
                ["Default"]
            );
            a.start(Some(2)).await.expect("start");
            a.stop(Some(2)).await.expect("stop");
            a.start(Some(2)).await.expect("start");
            let log: Vec<String> = w.lines.borrow().iter().filter(|l| l.contains("account 2")).cloned().collect();
            assert_eq!(log, ["supervisor: account 2 started", "supervisor: account 2 stopped", "supervisor: account 2 started"]);
            w.calls.borrow_mut().clear();
            let (x, y) = tokio::join!(a.stop(Some(2)), a.start(Some(2)));
            assert!(x.is_ok() && y.is_ok());
            assert_eq!(calls(&w), ["stop agent-2", "stop browser-2", "start browser-2", "start agent-2"]);
            w.calls.borrow_mut().clear();
            for n in [Some(0), Some(-1), None] {
                assert_eq!(a.start(n).await.unwrap_err().status, 404);
            }
            assert_eq!(a.start(Some(0)).await.unwrap_err().message, "No account 0: accounts are numbered from 1");
            a.start(Some(40)).await.expect("start");
            assert_eq!(calls(&w), ["start browser-40", "start agent-40"]);
            assert_eq!(
                a.status()[0],
                json!({"account": 2, "browser": {"running": true, "pid": 4242, "restarts": 0, "lastExit": null}, "agent": {"running": true, "pid": 4242, "restarts": 0, "lastExit": null}, "light": {"running": false, "pid": null, "restarts": 0, "lastExit": null}})
            );
            w.calls.borrow_mut().clear();
            a.stop_all().await;
            let mut got = calls(&w);
            got.sort();
            assert_eq!(got, ["stop agent-2", "stop agent-40", "stop browser-2", "stop browser-40"]);
            assert!(!a.browser_running());
        });
    }

    #[test]
    fn wipes_a_stopped_account_only_and_shows_a_running_one() {
        run(async {
            let c = cfg();
            let dir = profile(&c, 3, &[".config/chromium/Default/Cookies", ".cache/chromium/x", "top.txt"]);
            let (a, w) = accounts_with(c.clone(), 0, None);
            a.wipe(Some(3)).await.expect("wipe");
            assert!(!dir.exists());
            a.wipe(Some(4)).await.expect("never had one");
            a.start(Some(3)).await.expect("start");
            assert!(std::fs::read_dir(&dir).expect("dir").next().is_none());
            assert_eq!(a.wipe(Some(3)).await.unwrap_err(), AccountError::new(409, "Account 3 is running: stop it first"));
            assert!(!a.show(Some(1)).await.expect("show"));
            assert!(a.show(Some(3)).await.expect("show"));
            assert_eq!(*w.focused.borrow(), [3]);
        });
    }

    #[test]
    fn the_light_slots_on_the_host_get_the_command_of_a_single_light_process() {
        run(async {
            struct Host(Rc<RefCell<Vec<String>>>, Rc<RefCell<Vec<Command>>>);
            impl LightHostLike for Host {
                fn slot(&self, n: i64, command: Box<dyn Fn() -> Command>) -> Rc<dyn ProcessLike> {
                    Rc::new(FakeProcess {
                        name: format!("light-{n}"),
                        calls: self.0.clone(),
                        stop_ms: 0,
                        wanted: Cell::new(false),
                        command,
                        commands: self.1.clone(),
                    })
                }
            }
            let c = cfg();
            let calls_: Rc<RefCell<Vec<String>>> = Rc::default();
            let cmds: Rc<RefCell<Vec<Command>>> = Rc::default();
            let mut hc = c.clone();
            hc.light_host = true;
            let (a, w) = accounts_with(hc, 0, Some(Rc::new(Host(calls_.clone(), cmds.clone()))));
            a.light(Some(2), None).await.expect("light");
            assert_eq!(*calls_.borrow(), ["start light-2"]);
            assert_eq!(*cmds.borrow(), [light_command(2, &c, None)]);
            a.stop(Some(2)).await.expect("stop");
            assert_eq!(*calls_.borrow(), ["start light-2", "stop light-2"]);
            assert_eq!(calls(&w), ["stop agent-2", "stop browser-2"]);
        });
    }

    #[test]
    fn a_bound_slot_runs_on_its_account_refuses_its_browser_and_a_wipe_drops_the_binding() {
        run(async {
            let mut c = cfg();
            c.shared_broker = Some(crate::supervisor::broker::SharedBroker { url: "http://oe-copilot-agent:8180".into(), key: "secret-key".into() });
            let mario = Binding { account: "mario@example.com".into(), region: "amer".into() };
            let (a, w) = accounts_with(c.clone(), 0, None);
            a.light(Some(2), Some(mario.clone())).await.expect("light");
            assert_eq!(a.start(Some(2)).await.unwrap_err().status, 409);
            a.start(Some(1)).await.expect("start");
            assert_eq!(calls(&w), ["start light-2", "start browser-1", "start agent-1"]);
            assert_eq!(w.commands.borrow()[0].env.as_ref().expect("env")["SHARED_BROKER_ACCOUNT"], "mario@example.com");
            w.calls.borrow_mut().clear();
            a.light(Some(2), Some(mario.clone())).await.expect("same");
            a.light(Some(2), Some(Binding { account: "anna@example.com".into(), region: "emea".into() })).await.expect("other");
            assert_eq!(calls(&w), ["stop light-2", "start light-2"]);
            assert_eq!(w.commands.borrow().last().and_then(|c| c.env.as_ref()).expect("env")["SHARED_BROKER_ACCOUNT"], "anna@example.com");
            assert_eq!(a.wipe(Some(2)).await.unwrap_err().status, 409);
            a.stop(Some(2)).await.expect("stop");
            assert_eq!(a.start(Some(2)).await.unwrap_err().status, 409);
            a.wipe(Some(2)).await.expect("wipe");
            a.start(Some(2)).await.expect("start");
            assert_eq!(calls(&w)[calls(&w).len() - 2..], ["start browser-2", "start agent-2"]);
            a.light(Some(5), Some(mario.clone())).await.expect("light");
            a.light(Some(5), None).await.expect("unbound");
            a.stop(Some(5)).await.expect("stop");
            a.start(Some(5)).await.expect("its browser again");

            let (b, w) = accounts_with(cfg(), 0, None);
            assert_eq!(b.light(Some(2), Some(mario)).await.unwrap_err().status, 409);
            assert!(calls(&w).is_empty());
        });
    }
}
