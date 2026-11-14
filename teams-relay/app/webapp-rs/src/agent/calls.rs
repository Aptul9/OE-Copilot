//! Calls of a light account (src/agent/light/native-calls.ts): Microsoft's Teams calling runs in a Node worker of its
//! own (calling-live-host.mjs, WebRTC), started on the IPC channel of fork: a native WebRTC fault must not stop chat
//! polling. The worker recycles itself after 45 idle minutes; it is started again 10 s after any exit.

use super::config::Env;
use super::http::send;
use super::run::Light;
use super::token::SPACES;
use super::{Fail, R, info, now_ms, text_of, warn};
use crate::push::notify::keep_slot;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// A worker started: its channel, the signal of its exit (code, signal).
pub struct Spawned {
    pub worker: Rc<dyn Worker>,
    pub messages: mpsc::UnboundedReceiver<Value>,
    pub exit: oneshot::Receiver<(Option<i32>, Option<i32>)>,
}

pub trait Worker {
    fn send(&self, v: &Value) -> bool;
    fn connected(&self) -> bool;
    fn kill(&self, signal: i32);
    /// it exited, or was never started
    fn gone(&self) -> bool;
}

/// fork(script, args, { env }): the real one, or a fake of the tests.
pub type Spawn = Rc<dyn Fn(&str, &[&str], &Env) -> Result<Spawned, String>>;

/// The Node of the image, which runs the worker.
fn node() -> String {
    std::env::var("NODE_BIN").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "/usr/local/bin/node".into())
}

struct Process {
    pid: i32,
    channel: super::ipc::Sender,
    gone: Rc<std::cell::Cell<bool>>,
}

impl Worker for Process {
    fn send(&self, v: &Value) -> bool {
        self.channel.send(v)
    }
    fn connected(&self) -> bool {
        !self.gone.get() && self.channel.connected()
    }
    fn kill(&self, signal: i32) {
        if !self.gone.get() {
            // SAFETY: a signal to the worker this process started and has not reaped
            unsafe { libc::kill(self.pid, signal) };
        }
    }
    fn gone(&self) -> bool {
        self.gone.get()
    }
}

/// The worker as a child of this process: stdin and stdout ignored, stderr to the host's, the channel on fd 3, in the
/// process group of the host (a stop of the host reaches it).
pub fn fork_worker() -> Spawn {
    Rc::new(|script: &str, args: &[&str], env: &Env| {
        use std::os::fd::AsRawFd;
        let (ours, theirs) = super::ipc::pair().map_err(|e| e.to_string())?;
        let fd = theirs.as_raw_fd();
        let mut cmd = tokio::process::Command::new(node());
        cmd.arg(script)
            .args(args)
            .env_clear()
            .envs(env)
            .env("NODE_CHANNEL_FD", super::ipc::CHILD_FD.to_string())
            .env("NODE_CHANNEL_SERIALIZATION_MODE", "json");
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit());
        // SAFETY: only async-signal-safe calls between fork and exec
        unsafe {
            cmd.pre_exec(move || {
                if fd == super::ipc::CHILD_FD {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                } else if libc::dup2(fd, super::ipc::CHILD_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        drop(theirs);
        let pid = child.id().unwrap_or(0) as i32;
        let (channel, messages) = super::ipc::open(ours);
        let gone = Rc::new(std::cell::Cell::new(false));
        let (tx, exit) = oneshot::channel();
        let done = gone.clone();
        tokio::task::spawn_local(async move {
            use std::os::unix::process::ExitStatusExt;
            let status = child.wait().await.ok();
            done.set(true);
            let _ = tx.send((status.and_then(|s| s.code()), status.and_then(|s| s.signal())));
        });
        Ok(Spawned { worker: Rc::new(Process { pid, channel, gone }), messages, exit })
    })
}

/// The registration a worker that died left behind: its endpoint and the registrar it was made at.
fn owned_registration(file: &std::path::Path) -> R<Option<(String, String)>> {
    if !file.exists() {
        return Ok(None);
    }
    let v: Value = serde_json::from_slice(&std::fs::read(file).map_err(|e| Fail::Other(e.to_string()))?).map_err(|e| Fail::Other(e.to_string()))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (endpoint, registrar) = (s("endpoint"), s("registrar"));
    let id = regex::Regex::new("(?i)^[a-f0-9-]{36}$").expect("fixed");
    let reg = regex::Regex::new(r"(?i)^https://teams\.microsoft\.com/registrar/prod/v2/registrations$").expect("fixed");
    if !id.is_match(&endpoint) || !reg.is_match(&registrar) {
        return Err(Fail::Other("invalid calling ownership".into()));
    }
    Ok(Some((endpoint, registrar)))
}

#[derive(Default)]
struct State {
    child: Option<Rc<dyn Worker>>,
    stopped: bool,
    notified: i64,
    logged_call: i64,
    sequence: i64,
    pending: HashMap<i64, oneshot::Sender<bool>>,
}

pub struct NativeCalls {
    l: Weak<Light>,
    spawn: Spawn,
    me: Weak<NativeCalls>,
    state: RefCell<State>,
}

const SIGTERM: i32 = libc::SIGTERM;
const SIGKILL: i32 = libc::SIGKILL;

impl NativeCalls {
    pub fn new(l: &Rc<Light>, spawn: Spawn) -> Rc<NativeCalls> {
        Rc::new_cyclic(|me| NativeCalls { l: Rc::downgrade(l), spawn, me: me.clone(), state: RefCell::new(State::default()) })
    }

    fn rc(&self) -> Option<Rc<NativeCalls>> {
        self.me.upgrade()
    }

    fn stopped(&self) -> bool {
        self.state.borrow().stopped
    }

    async fn authz(l: &Light, timeout_ms: u64) -> R<super::http::Response> {
        let access = l.token.access_token(SPACES).await?;
        let headers = vec![("Authorization", format!("Bearer {access}")), ("x-ms-client-type", "web".into())];
        send(&l.http, "POST", "https://teams.microsoft.com/api/authsvc/v1.0/authz", headers, vec![], timeout_ms).await
    }

    pub async fn start(&self) -> R<()> {
        if self.stopped() || self.state.borrow().child.is_some() {
            return Ok(());
        }
        let Some(l) = self.l.upgrade() else { return Ok(()) };
        let dir = l.config.light_token_file.parent().map(std::path::PathBuf::from).unwrap_or_default();
        let r = Self::authz(&l, 20_000).await?;
        if !r.ok() {
            return Err(Fail::Other(format!("calling discovery {}", r.status)));
        }
        let discovery = super::http::json(&r)?;
        if self.stopped() {
            return Ok(());
        }
        let io = |e: std::io::Error| Fail::Other(e.to_string());
        super::write_private(&dir.join("discovery.json"), crate::js::stringify(&discovery).as_bytes()).map_err(io)?;
        let audio_token = dir.join("calling-audio-token");
        let secret: String = super::random_bytes(32).iter().map(|b| format!("{b:02x}")).collect();
        super::write_private(&audio_token, secret.as_bytes()).map_err(io)?;
        // a previous worker may have died before its terminal DELETE: remove only its saved registration
        let marker = dir.join("owned-registration.json");
        if let Some((endpoint, registrar)) = owned_registration(&marker)? {
            let skype = discovery.get("tokens").and_then(|t| t.get("skypeToken")).map(|t| crate::js::string(Some(t))).unwrap_or_default();
            let headers = vec![("X-Skypetoken", skype), ("X-MS-Migration", "True".into())];
            let r = send(&l.http, "DELETE", &format!("{registrar}/{endpoint}"), headers, vec![], 15_000).await?;
            if !r.ok() && r.status != 404 {
                return Err(Fail::Other(format!("calling cleanup {}", r.status)));
            }
            std::fs::remove_file(&marker).map_err(io)?;
        }
        // stopped during the cleanup: a worker started now would outlive its slot in the light host
        if self.stopped() {
            return Ok(());
        }
        let (oid, name) = l.me.borrow().clone();
        let mut env = l.config.env.clone();
        for (k, v) in [
            ("HOST_STATE", dir.display().to_string()),
            ("HOST_BUNDLES", "/app/calling".into()),
            ("WRTC_DIR", "/app".into()),
            ("CALLING_OID", oid),
            ("CALLING_NAME", name),
            ("CALL_AUDIO_TOKEN_FILE", audio_token.display().to_string()),
            ("CALL_AUDIO_URL", l.config.call_audio_url.clone()),
            ("CALL_AUDIO_INTERNAL", "1".into()),
        ] {
            env.insert(k.into(), v);
        }
        let spawned = (self.spawn)(&l.config.calling_host, &["--mode=serve"], &env).map_err(Fail::Other)?;
        let worker = spawned.worker.clone();
        self.state.borrow_mut().child = Some(worker.clone());
        let Some(me) = self.rc() else { return Ok(()) };
        let mut messages = spawned.messages;
        let on = me.clone();
        let w = worker.clone();
        tokio::task::spawn_local(keep_slot(async move {
            while let Some(m) = messages.recv().await {
                on.on_message(&w, m);
            }
        }));
        let exit = spawned.exit;
        tokio::task::spawn_local(keep_slot(async move {
            let (code, signal) = exit.await.unwrap_or((None, None));
            me.exited(&worker, code, signal);
        }));
        Ok(())
    }

    fn on_message(&self, child: &Rc<dyn Worker>, m: Value) {
        let Some(l) = self.l.upgrade() else { return };
        if m.get("refreshToken").is_some_and(crate::js::truthy) {
            let (l2, child) = (l.clone(), child.clone());
            tokio::task::spawn_local(keep_slot(async move {
                let run = async {
                    l2.token.renew_all();
                    let r = Self::authz(&l2, 15_000).await?;
                    if !r.ok() {
                        return Err(Fail::Other(format!("calling token refused {}", r.status)));
                    }
                    let dir = l2.config.light_token_file.parent().map(std::path::PathBuf::from).unwrap_or_default();
                    super::write_private(&dir.join("discovery.json"), crate::js::stringify(&super::http::json(&r)?).as_bytes())
                        .map_err(|e| Fail::Other(e.to_string()))?;
                    child.send(&json!({ "tokenUpdated": true }));
                    Ok::<(), Fail>(())
                };
                if let Err(e) = run.await {
                    warn("calling", &format!("token refresh failed: {}, worker ended", text_of(&e)), &[]);
                    child.kill(SIGTERM);
                }
            }));
        }
        if m.get("stage") == Some(&json!("worker-ready")) {
            l.store.set_state(crate::state::key::CALL_AUDIO, "1");
        }
        if let Some(e) = m.get("error").filter(|e| crate::js::truthy(e)) {
            warn("calling", &crate::js::string(Some(e)), &[]);
        }
        if let Some(Value::Object(audio)) = m.get("callAudio") {
            let fields: Vec<(&str, super::Field)> = audio.iter().map(|(k, v)| (k.as_str(), super::Field::Num(crate::js::number(Some(v))))).collect();
            info("calling", "call audio", &fields);
        }
        if let Some(id) = m.get("command").and_then(Value::as_f64) {
            let waiting = self.state.borrow_mut().pending.remove(&(id as i64));
            if let Some(tx) = waiting {
                let _ = tx.send(m.get("ok").is_some_and(crate::js::truthy));
            }
        }
        if let Some(Value::Object(c)) = m.get("call") {
            self.on_call(&l, c.clone());
        }
    }

    fn on_call(&self, l: &Light, mut c: Map<String, Value>) {
        let caller = c.get("caller").and_then(Value::as_str).unwrap_or("").to_string();
        let bare = caller.trim_start_matches("8:orgid:").to_string();
        let named = {
            let list = l.list.borrow();
            list.members
                .iter()
                .find(|(_, members)| members.iter().any(|m| *m == caller || m.ends_with(&bare)))
                .and_then(|(thread, _)| list.names.get(thread).cloned())
        };
        let caller = named.filter(|n| !n.is_empty()).unwrap_or(caller);
        c.insert("caller".into(), json!(caller));
        let num = |k: &str| crate::js::number(c.get(k));
        let since = num("since");
        let since_i = if since.is_finite() { since as i64 } else { 0 };
        let flag = |k: &str| c.get(k).is_some_and(crate::js::truthy);
        if flag("ended") && since_i != 0 && self.state.borrow().logged_call != since_i {
            self.state.borrow_mut().logged_call = since_i;
            let seconds = if flag("answered") { ((now_ms() - since_i) / 1000).max(0) } else { 0 };
            l.store.add_call(&caller, since_i, seconds);
        }
        let text = crate::js::stringify(&Value::Object(c.clone()));
        l.store.set_state(crate::state::key::CALL, &text);
        l.store.set_state(crate::state::key::IN_CALL, &text);
        let notifier = l.notifier.clone();
        let notified = self.state.borrow().notified;
        if flag("ringing") && notified != since_i {
            self.state.borrow_mut().notified = since_i;
            tokio::task::spawn_local(keep_slot(async move {
                notifier.call(&caller, "ringing", since_i, 0, false).await;
            }));
        } else if !flag("ringing") && notified == since_i {
            self.state.borrow_mut().notified = 0;
            let active = flag("active");
            tokio::task::spawn_local(keep_slot(async move {
                notifier.call(&caller, "ended", since_i, 0, active).await;
            }));
        }
    }

    fn exited(&self, worker: &Rc<dyn Worker>, code: Option<i32>, signal: Option<i32>) {
        let stopped = self.stopped();
        // exit 0: the worker recycled itself (idle 45 min) or ended on a SIGTERM logged where it was sent
        if !stopped && (signal.is_some() || code != Some(0)) {
            let how = match signal {
                Some(s) => format!("signal {}", signal_name(s)),
                None => format!("code {}", code.map_or("null".into(), |c| c.to_string())),
            };
            warn("calling", &format!("worker exited ({how}), starting again in 10 s"), &[]);
        }
        {
            let mut s = self.state.borrow_mut();
            if s.child.as_ref().is_some_and(|c| Rc::ptr_eq(c, worker)) {
                s.child = None;
            }
            s.pending.clear();
        }
        if let Some(l) = self.l.upgrade() {
            l.store.set_state(crate::state::key::CALL_AUDIO, "0");
            let ended = crate::js::stringify(&json!({ "caller": "", "since": 0, "seen": now_ms(), "ringing": false, "active": false }));
            l.store.set_state(crate::state::key::CALL, &ended);
            l.store.set_state(crate::state::key::IN_CALL, &ended);
        }
        if !stopped {
            self.retry();
        }
    }

    pub fn schedule(&self) {
        self.retry();
    }

    fn retry(&self) {
        if self.stopped() {
            return;
        }
        let Some(me) = self.rc() else { return };
        tokio::task::spawn_local(keep_slot(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if !me.stopped()
                && let Err(e) = me.start().await
            {
                let _ = e;
                me.retry();
            }
        }));
    }

    /// A command to the worker; false when it is not connected, refused it, or did not answer in 30 s (then it is
    /// ended).
    pub async fn command(&self, op: &str, args: Map<String, Value>) -> bool {
        let child = self.state.borrow().child.clone();
        let Some(child) = child.filter(|c| c.connected()) else { return false };
        let id = {
            let mut s = self.state.borrow_mut();
            s.sequence += 1;
            s.sequence
        };
        let (tx, rx) = oneshot::channel();
        self.state.borrow_mut().pending.insert(id, tx);
        let mut msg = Map::new();
        msg.insert("id".into(), json!(id));
        msg.insert("op".into(), json!(op));
        msg.extend(args);
        if !child.send(&Value::Object(msg)) {
            self.state.borrow_mut().pending.remove(&id);
            return false;
        }
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(ok)) => ok,
            Ok(Err(_)) => false,
            Err(_) => {
                self.state.borrow_mut().pending.remove(&id);
                warn("calling", &format!("worker did not answer {op} in 30 s, worker ended"), &[]);
                let child = self.state.borrow().child.clone();
                if let Some(c) = child {
                    c.kill(SIGTERM);
                }
                false
            }
        }
    }

    /// Calls end: the worker is ended, killed after 25 s if it is still there.
    pub fn stop(&self) {
        self.state.borrow_mut().stopped = true;
        if let Some(l) = self.l.upgrade() {
            l.store.set_state(crate::state::key::CALL_AUDIO, "0");
        }
        let child = self.state.borrow().child.clone();
        if let Some(c) = child {
            c.kill(SIGTERM);
            tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_secs(25)).await;
                if !c.gone() {
                    c.kill(SIGKILL);
                }
            });
        }
    }
}

/// The name Node gives a signal in an exit.
pub fn signal_name(s: i32) -> String {
    match s {
        libc::SIGTERM => "SIGTERM".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGINT => "SIGINT".into(),
        libc::SIGABRT => "SIGABRT".into(),
        libc::SIGSEGV => "SIGSEGV".into(),
        libc::SIGHUP => "SIGHUP".into(),
        libc::SIGPIPE => "SIGPIPE".into(),
        n => format!("{n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Node worker on the channel of fork_worker: it says it is ready, answers a command, ends on SIGTERM. Runs
    /// where NODE_BIN names a Node (scripts/check.sh sets it).
    #[test]
    fn a_node_worker_talks_over_the_channel_of_fork() {
        if std::env::var("NODE_BIN").map_or(true, |n| n.is_empty()) {
            eprintln!("NODE_BIN not set: the Node worker test is skipped");
            return;
        }
        crate::agent::testing::run(async {
            let dir = crate::agent::testing::temp_dir();
            let script = dir.join("worker.mjs");
            std::fs::write(
                &script,
                r#"process.send({ stage: "worker-ready", argv: process.argv.slice(2), slot: process.env.ACCOUNT });
process.on("message", (m) => process.send({ command: m.id, ok: m.op === "answer" && m.since === 123 }));
process.once("SIGTERM", () => process.exit(0));
setInterval(() => {}, 1000);"#,
            )
            .expect("script");
            let env: Env = [("ACCOUNT".to_string(), "4".to_string()), ("PATH".to_string(), std::env::var("PATH").unwrap_or_default())].into_iter().collect();
            let mut s = fork_worker()(script.to_str().expect("path"), &["--mode=serve"], &env).expect("spawned");
            let ready = tokio::time::timeout(Duration::from_secs(10), s.messages.recv()).await.expect("in time").expect("message");
            assert_eq!(ready, json!({ "stage": "worker-ready", "argv": ["--mode=serve"], "slot": "4" }));
            assert!(s.worker.connected());
            assert!(s.worker.send(&json!({ "id": 1, "op": "answer", "since": 123 })));
            let answer = tokio::time::timeout(Duration::from_secs(10), s.messages.recv()).await.expect("in time").expect("message");
            assert_eq!(answer, json!({ "command": 1, "ok": true }));
            s.worker.kill(SIGTERM);
            let (code, signal) = tokio::time::timeout(Duration::from_secs(10), s.exit).await.expect("in time").expect("exit");
            assert_eq!((code, signal), (Some(0), None));
            assert!(s.worker.gone() && !s.worker.connected());
        });
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::cell::Cell;

    /// A worker of a test: what was sent, the signals, and handles to make it talk and exit.
    pub struct FakeWorker {
        pub sent: RefCell<Vec<Value>>,
        pub kills: RefCell<Vec<i32>>,
        pub exited: Cell<bool>,
    }

    impl Worker for FakeWorker {
        fn send(&self, v: &Value) -> bool {
            self.sent.borrow_mut().push(v.clone());
            true
        }
        fn connected(&self) -> bool {
            !self.exited.get()
        }
        fn kill(&self, signal: i32) {
            self.kills.borrow_mut().push(signal);
        }
        fn gone(&self) -> bool {
            self.exited.get()
        }
    }

    /// exit code and signal of a worker
    type Exit = oneshot::Sender<(Option<i32>, Option<i32>)>;
    /// what the spawn of a test started: script, environment, the handle on the worker
    pub type Started = Rc<RefCell<Vec<(String, Env, Rc<Handle>)>>>;

    pub struct Handle {
        pub worker: Rc<FakeWorker>,
        pub say: mpsc::UnboundedSender<Value>,
        pub exit: RefCell<Option<Exit>>,
    }

    impl Handle {
        pub fn exit(&self, code: Option<i32>, signal: Option<i32>) {
            self.worker.exited.set(true);
            if let Some(tx) = self.exit.borrow_mut().take() {
                let _ = tx.send((code, signal));
            }
        }
    }

    /// A spawn that records what it started and hands the test the worker.
    pub fn spawner() -> (Spawn, Started) {
        let started: Started = Rc::default();
        let log = started.clone();
        let spawn: Spawn = Rc::new(move |script: &str, args: &[&str], env: &Env| {
            assert_eq!(args, ["--mode=serve"]);
            let worker = Rc::new(FakeWorker { sent: RefCell::default(), kills: RefCell::default(), exited: Cell::new(false) });
            let (say, messages) = mpsc::unbounded_channel();
            let (tx, exit) = oneshot::channel();
            let handle = Rc::new(Handle { worker: worker.clone(), say, exit: RefCell::new(Some(tx)) });
            log.borrow_mut().push((script.to_string(), env.clone(), handle));
            Ok(Spawned { worker, messages, exit })
        });
        (spawn, started)
    }
}
