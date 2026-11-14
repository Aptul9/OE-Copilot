//! All the light slots in one process (src/supervisor/light-host.ts): the Rust light host (agent/host.rs) or node
//! light.cjs --host, on the IPC channel of fork. Kept running while a slot is wanted: started with the first, started
//! again after an exit, stopped with the last.

use super::config::Env;
use super::process::{Command, Lines, ProcessLike};
use super::{Event, exit_text, signal_group, spawn_error};
use crate::agent::host::{TIMING, Timing, restart_delay};
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{mpsc, oneshot};

/// The channel to one host child; the real one is spawned, tests give a fake.
pub trait HostChild {
    fn pid(&self) -> Option<u32>;
    fn send(&self, m: &Value);
    fn kill(&self, sig: i32);
}

pub enum HostEvent {
    Message(Value),
    Exit(String),
}

pub type SpawnHost = Rc<dyn Fn() -> (Rc<dyn HostChild>, mpsc::UnboundedReceiver<HostEvent>)>;

pub trait LightHostLike {
    fn slot(&self, n: i64, command: Box<dyn Fn() -> Command>) -> Rc<dyn ProcessLike>;
}

#[derive(Default)]
struct SlotState {
    restarts: u32,
    last_exit: Option<String>,
    up: bool,
    wanted: bool,
    /// the next start follows an exit, not a stop: a restart, as Supervised counts them
    failed: bool,
    waiting: Vec<oneshot::Sender<()>>,
}

/// One light slot in the host: what Supervised is to a process of its own.
pub struct HostedLight {
    n: i64,
    command: Box<dyn Fn() -> Command>,
    host: Weak<LightHost>,
    st: RefCell<SlotState>,
}

impl HostedLight {
    fn started(&self) {
        let mut s = self.st.borrow_mut();
        s.up = true;
        if s.failed {
            s.restarts += 1;
        }
        s.failed = false;
    }

    fn exited(&self, how: &str) {
        let mut s = self.st.borrow_mut();
        s.up = false;
        s.last_exit = Some(how.into());
        s.failed = s.wanted;
    }

    fn stopped(&self) {
        let mut s = self.st.borrow_mut();
        s.up = false;
        s.failed = false;
        for w in s.waiting.drain(..) {
            let _ = w.send(());
        }
    }

    /// the host is gone: the slot with it
    fn lost(&self, how: &str) {
        self.exited(how);
        for w in self.st.borrow_mut().waiting.drain(..) {
            let _ = w.send(());
        }
    }

    fn answer(&self) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        self.st.borrow_mut().waiting.push(tx);
        rx
    }

    fn env(&self) -> Env {
        (self.command)().env.unwrap_or_default()
    }
}

impl ProcessLike for HostedLight {
    fn name(&self) -> String {
        format!("light-{}", self.n)
    }

    fn running(&self) -> bool {
        self.st.borrow().up
    }

    fn active(&self) -> bool {
        self.st.borrow().wanted
    }

    fn pid(&self) -> Option<u32> {
        if self.st.borrow().up { self.host.upgrade().and_then(|h| h.pid()) } else { None }
    }

    fn restarts(&self) -> u32 {
        self.st.borrow().restarts
    }

    fn last_exit(&self) -> Option<String> {
        self.st.borrow().last_exit.clone()
    }

    fn start(&self) {
        self.st.borrow_mut().wanted = true;
        if let Some(h) = self.host.upgrade() {
            h.want(self.n);
        }
    }

    fn stop(self: Rc<Self>) -> LocalBoxFuture<'static, ()> {
        Box::pin(async move {
            self.st.borrow_mut().wanted = false;
            if let Some(h) = self.host.upgrade() {
                h.release(&self).await;
            }
        })
    }
}

struct Child {
    id: u64,
    child: Rc<dyn HostChild>,
    gone: Event,
}

#[derive(Default)]
struct HostState {
    child: Option<Child>,
    timer: Option<tokio::task::JoinHandle<()>>,
    failures: u32,
    started_at: Option<Instant>,
    seq: u64,
}

pub struct LightHost {
    spawn: SpawnHost,
    log: Lines,
    timing: Timing,
    slots: RefCell<BTreeMap<i64, Rc<HostedLight>>>,
    st: RefCell<HostState>,
    me: Weak<LightHost>,
}

impl LightHost {
    pub fn new(spawn: SpawnHost, log: Lines, timing: Timing) -> Rc<LightHost> {
        Rc::new_cyclic(|me| LightHost { spawn, log, timing, slots: RefCell::default(), st: RefCell::default(), me: me.clone() })
    }

    pub fn with_defaults(spawn: SpawnHost, log: Lines) -> Rc<LightHost> {
        Self::new(spawn, log, TIMING)
    }

    pub fn pid(&self) -> Option<u32> {
        self.st.borrow().child.as_ref().and_then(|c| c.child.pid())
    }

    fn current(&self) -> Option<(u64, Rc<dyn HostChild>, Event)> {
        self.st.borrow().child.as_ref().map(|c| (c.id, c.child.clone(), c.gone.clone()))
    }

    fn wanted(&self) -> bool {
        self.slots.borrow().values().any(|s| s.st.borrow().wanted)
    }

    fn start_slot(child: &Rc<dyn HostChild>, s: &HostedLight) {
        child.send(&json!({ "op": "start", "slot": s.n, "env": s.env() }));
    }

    fn want(&self, n: i64) {
        let s = self.slots.borrow().get(&n).cloned();
        match (self.current(), s) {
            (Some((_, child, _)), Some(s)) => Self::start_slot(&child, &s),
            (None, _) if self.st.borrow().timer.is_none() => self.open(),
            _ => {}
        }
    }

    /// Answered when the host says the slot stopped, when the host exits, or after the grace. The last slot to go takes
    /// the host with it.
    async fn release(&self, s: &HostedLight) {
        if let Some((_, child, _)) = self.current() {
            let answered = s.answer();
            child.send(&json!({ "op": "stop", "slot": s.n }));
            let _ = tokio::time::timeout(Duration::from_millis(self.timing.stop_grace_ms), answered).await;
            s.stopped();
        }
        if self.wanted() {
            return;
        }
        if let Some(t) = self.st.borrow_mut().timer.take() {
            t.abort();
        }
        self.close().await;
    }

    fn open(&self) {
        let Some(me) = self.me.upgrade() else { return };
        let (child, mut events) = (self.spawn)();
        let gone = Event::default();
        let id = {
            let mut st = self.st.borrow_mut();
            st.timer = None;
            st.seq += 1;
            st.started_at = Some(Instant::now());
            st.child = Some(Child { id: st.seq, child: child.clone(), gone: gone.clone() });
            st.seq
        };
        tokio::task::spawn_local(async move {
            while let Some(e) = events.recv().await {
                match e {
                    HostEvent::Message(m) => me.on_message(id, &m),
                    HostEvent::Exit(how) => {
                        gone.set();
                        me.exited(id, &how);
                        break;
                    }
                }
            }
        });
        let wanted: Vec<Rc<HostedLight>> = self.slots.borrow().values().filter(|s| s.st.borrow().wanted).cloned().collect();
        for s in wanted {
            Self::start_slot(&child, &s);
        }
    }

    fn on_message(&self, id: u64, m: &Value) {
        if self.current().is_none_or(|c| c.0 != id) {
            return;
        }
        let Some(s) = m.get("slot").and_then(Value::as_i64).and_then(|n| self.slots.borrow().get(&n).cloned()) else { return };
        match m.get("event").and_then(Value::as_str) {
            Some("started") => s.started(),
            Some("exited") => s.exited(m.get("how").and_then(Value::as_str).unwrap_or("")),
            _ => s.stopped(),
        }
    }

    fn exited(&self, id: u64, how: &str) {
        if self.current().is_none_or(|c| c.0 != id) {
            return;
        }
        self.st.borrow_mut().child = None;
        let slots: Vec<Rc<HostedLight>> = self.slots.borrow().values().cloned().collect();
        for s in slots {
            s.lost(how);
        }
        if !self.wanted() {
            return;
        }
        let delay = {
            let mut st = self.st.borrow_mut();
            let up = st.started_at.map_or(0, |t| t.elapsed().as_millis() as u64);
            let (failures, delay) = restart_delay(up, st.failures, self.timing);
            st.failures = failures;
            delay
        };
        (self.log)(&format!("light host exited ({how}), starting again in {} s", crate::js::number_to_string(delay as f64 / 1000.0)));
        let Some(me) = self.me.upgrade() else { return };
        let timer = tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            me.open();
        });
        self.st.borrow_mut().timer = Some(timer);
    }

    async fn close(&self) {
        let Some((_, child, gone)) = self.current() else { return };
        child.kill(libc::SIGTERM);
        let grace = self.timing.stop_grace_ms;
        let (killer, k) = (gone.clone(), child.clone());
        let kill = tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(grace)).await;
            if !killer.is_set() {
                k.kill(libc::SIGKILL);
            }
        });
        gone.wait().await;
        kill.abort();
    }
}

impl LightHostLike for LightHost {
    fn slot(&self, n: i64, command: Box<dyn Fn() -> Command>) -> Rc<dyn ProcessLike> {
        let mut slots = self.slots.borrow_mut();
        let s = slots.entry(n).or_insert_with(|| Rc::new(HostedLight { n, command, host: self.me.clone(), st: RefCell::default() }));
        s.clone()
    }
}

struct Spawned {
    pid: Option<u32>,
    channel: Option<crate::agent::ipc::Sender>,
}

impl HostChild for Spawned {
    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn send(&self, m: &Value) {
        if let Some(c) = &self.channel {
            c.send(m);
        }
    }

    fn kill(&self, sig: i32) {
        if let Some(pid) = self.pid {
            signal_group(pid, sig);
        }
    }
}

/// The host as a child of this process with an IPC channel, in a session of its own, so that a stop reaches the calling
/// workers it starts too: the Rust binary `bin`, else node `file`, with --host; its output lines to `output`.
pub fn fork_host(file: &str, node: &str, env: &Env, output: Lines, bin: Option<&str>) -> (Rc<dyn HostChild>, mpsc::UnboundedReceiver<HostEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let program = bin.unwrap_or(node);
    let mut cmd = tokio::process::Command::new(program);
    if bin.is_none() {
        cmd.arg(file);
    }
    cmd.arg("--host").env_clear().envs(env).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let (mut child, ours) = match crate::agent::ipc::spawn_with_channel(&mut cmd, true) {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(HostEvent::Exit(spawn_error(program, &e)));
            return (Rc::new(Spawned { pid: None, channel: None }), rx);
        }
    };
    let pid = child.id();
    let (channel, mut messages) = crate::agent::ipc::open(ours);
    let out = child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin>);
    let err = child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin>);
    for stream in [out, err].into_iter().flatten() {
        let output = output.clone();
        tokio::task::spawn_local(async move {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                output(&line);
            }
        });
    }
    let to = tx.clone();
    tokio::task::spawn_local(async move {
        while let Some(m) = messages.recv().await {
            if to.send(HostEvent::Message(m)).is_err() {
                break;
            }
        }
    });
    tokio::task::spawn_local(async move {
        let status = child.wait().await.ok();
        let _ = tx.send(HostEvent::Exit(exit_text(status)));
    });
    (Rc::new(Spawned { pid, channel: Some(channel) }), rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{run, temp_dir};
    use crate::supervisor::testing::until;
    use std::cell::Cell;

    const FAST: Timing = Timing { min_delay_ms: 40, max_delay_ms: 160, stable_ms: 60_000, stop_grace_ms: 150 };

    /// A host of a test: what it was sent and the signals, a handle to make it talk and exit.
    struct Fake {
        pid: u32,
        sent: RefCell<Vec<Value>>,
        killed: RefCell<Vec<i32>>,
        /// it dies on SIGTERM, as the host does
        dies: Cell<bool>,
        events: mpsc::UnboundedSender<HostEvent>,
    }

    impl HostChild for Fake {
        fn pid(&self) -> Option<u32> {
            Some(self.pid)
        }
        fn send(&self, m: &Value) {
            self.sent.borrow_mut().push(m.clone());
        }
        fn kill(&self, sig: i32) {
            self.killed.borrow_mut().push(sig);
            if self.dies.get() || sig == libc::SIGKILL {
                self.exit(&format!("signal {}", crate::agent::calls::signal_name(sig)));
            }
        }
    }

    impl Fake {
        fn emit(&self, m: Value) {
            let _ = self.events.send(HostEvent::Message(m));
        }
        fn exit(&self, how: &str) {
            let _ = self.events.send(HostEvent::Exit(how.into()));
        }
        fn starts(&self) -> Vec<i64> {
            self.sent.borrow().iter().filter(|m| m["op"] == "start").map(|m| m["slot"].as_i64().unwrap_or(0)).collect()
        }
    }

    type Children = Rc<RefCell<Vec<Rc<Fake>>>>;

    fn make() -> (Rc<LightHost>, Children, Rc<RefCell<Vec<String>>>) {
        let children: Children = Rc::default();
        let lines: Rc<RefCell<Vec<String>>> = Rc::default();
        let c = children.clone();
        let spawn: SpawnHost = Rc::new(move || {
            let (events, rx) = mpsc::unbounded_channel();
            let f = Rc::new(Fake { pid: 5000 + c.borrow().len() as u32, sent: RefCell::default(), killed: RefCell::default(), dies: Cell::new(true), events });
            c.borrow_mut().push(f.clone());
            (f as Rc<dyn HostChild>, rx)
        });
        let l = lines.clone();
        (LightHost::new(spawn, Rc::new(move |s: &str| l.borrow_mut().push(s.into())), FAST), children, lines)
    }

    fn command(tag: &'static str) -> Box<dyn Fn() -> Command> {
        Box::new(move || Command { file: "node".into(), env: Some([("ACCOUNT".to_string(), tag.to_string())].into_iter().collect()), ..Default::default() })
    }

    fn child(c: &Children, i: usize) -> Rc<Fake> {
        c.borrow()[i].clone()
    }

    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn one_host_for_all_the_slots_running_after_started_with_its_pid() {
        run(async {
            let (host, children, _) = make();
            let a = host.slot(2, command("2"));
            assert_eq!((a.name(), a.running(), a.active(), a.pid(), a.restarts(), a.last_exit()), ("light-2".into(), false, false, None, 0, None));
            a.start();
            host.slot(3, command("3")).start();
            assert_eq!(children.borrow().len(), 1);
            assert_eq!(
                *child(&children, 0).sent.borrow(),
                [json!({"op": "start", "slot": 2, "env": {"ACCOUNT": "2"}}), json!({"op": "start", "slot": 3, "env": {"ACCOUNT": "3"}})]
            );
            assert!(a.active() && !a.running());
            child(&children, 0).emit(json!({"slot": 2, "event": "started"}));
            settle().await;
            assert_eq!((a.running(), a.pid()), (true, Some(5000)));
            child(&children, 0).emit(json!({"slot": 9, "event": "started"}));
            settle().await;
        });
    }

    #[test]
    fn a_slot_exits_with_its_reason_and_restarts_count_only_after_an_exit() {
        run(async {
            let (host, children, _) = make();
            let (a, b) = (host.slot(2, command("2")), host.slot(3, command("3")));
            a.start();
            b.start();
            let c = child(&children, 0);
            c.emit(json!({"slot": 2, "event": "started"}));
            c.emit(json!({"slot": 3, "event": "started"}));
            c.emit(json!({"slot": 2, "event": "exited", "how": "Error: boom"}));
            settle().await;
            assert_eq!((a.running(), a.active(), a.last_exit().as_deref(), a.pid(), b.running()), (false, true, Some("Error: boom"), None, true));
            c.emit(json!({"slot": 2, "event": "started"}));
            settle().await;
            assert_eq!((a.running(), a.restarts()), (true, 1));
            let stopping = a.clone().stop();
            c.emit(json!({"slot": 2, "event": "stopped"}));
            stopping.await;
            a.start();
            c.emit(json!({"slot": 2, "event": "started"}));
            settle().await;
            assert_eq!((a.running(), a.restarts()), (true, 1));
        });
    }

    #[test]
    fn starts_the_host_again_after_it_exits_with_a_fresh_command_and_a_doubling_delay() {
        run(async {
            let (host, children, lines) = make();
            let n = Rc::new(Cell::new(0));
            let m = n.clone();
            let a = host.slot(
                2,
                Box::new(move || {
                    m.set(m.get() + 1);
                    let env = [("ACCOUNT".to_string(), "2".to_string()), ("RUN".to_string(), m.get().to_string())].into_iter().collect();
                    Command { env: Some(env), ..Default::default() }
                }),
            );
            let b = host.slot(3, command("3"));
            a.start();
            b.start();
            child(&children, 0).emit(json!({"slot": 2, "event": "started"}));
            child(&children, 0).exit("code 1");
            settle().await;
            assert_eq!((a.running(), a.active(), a.last_exit().as_deref(), b.last_exit().as_deref()), (false, true, Some("code 1"), Some("code 1")));
            assert_eq!(*lines.borrow(), ["light host exited (code 1), starting again in 0.04 s"]);
            until(|| children.borrow().len() == 2, 5000).await;
            assert_eq!(
                *child(&children, 1).sent.borrow(),
                [json!({"op": "start", "slot": 2, "env": {"ACCOUNT": "2", "RUN": "2"}}), json!({"op": "start", "slot": 3, "env": {"ACCOUNT": "3"}})]
            );
            child(&children, 1).emit(json!({"slot": 2, "event": "started"}));
            settle().await;
            assert_eq!((a.running(), a.restarts(), a.pid()), (true, 1, Some(5001)));
            child(&children, 1).exit("code 1");
            settle().await;
            assert_eq!(lines.borrow()[1], "light host exited (code 1), starting again in 0.08 s");
            until(|| children.borrow().len() == 3, 5000).await;
        });
    }

    #[test]
    fn a_stop_is_answered_by_stopped_by_the_grace_or_by_the_host_exit() {
        run(async {
            let (host, children, _) = make();
            let (a, b) = (host.slot(2, command("2")), host.slot(3, command("3")));
            a.start();
            b.start();
            let c = child(&children, 0);
            let stopping = a.clone().stop();
            let answer = c.clone();
            tokio::task::spawn_local(async move {
                settle().await;
                answer.emit(json!({"slot": 2, "event": "stopped"}));
            });
            stopping.await;
            assert_eq!(c.sent.borrow().last(), Some(&json!({"op": "stop", "slot": 2})));
            assert!(!a.active() && !a.running() && c.killed.borrow().is_empty());
            a.start();
            let t0 = Instant::now();
            a.clone().stop().await;
            assert!(t0.elapsed() >= Duration::from_millis(140));
            assert!(c.killed.borrow().is_empty());
        });
    }

    #[test]
    fn the_last_slot_takes_the_host_with_it_killed_after_the_grace_and_a_new_slot_starts_another() {
        run(async {
            let (host, children, lines) = make();
            let a = host.slot(2, command("2"));
            a.start();
            let c = child(&children, 0);
            c.dies.set(false);
            let stopping = a.clone().stop();
            let answer = c.clone();
            tokio::task::spawn_local(async move {
                settle().await;
                answer.emit(json!({"slot": 2, "event": "stopped"}));
            });
            stopping.await;
            assert_eq!(*c.killed.borrow(), [libc::SIGTERM, libc::SIGKILL]);
            tokio::time::sleep(Duration::from_millis(120)).await;
            assert_eq!(children.borrow().len(), 1);
            assert!(lines.borrow().is_empty());
            a.start();
            assert_eq!((children.borrow().len(), child(&children, 1).starts()), (2, vec![2]));
        });
    }

    #[test]
    fn no_new_host_when_the_last_slot_stopped_during_the_delay() {
        run(async {
            let (host, children, _) = make();
            let a = host.slot(2, command("2"));
            a.start();
            child(&children, 0).exit("code 1");
            settle().await;
            a.clone().stop().await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(children.borrow().len(), 1);
        });
    }

    /// The real spawn, with a shell script for a host: --host, the channel on fd 3, its lines, the group killed.
    #[test]
    fn fork_host_spawns_the_binary_with_host_on_the_channel_and_a_missing_one_is_an_exit() {
        let Some(node) = std::env::var("NODE_BIN").ok().filter(|n| !n.is_empty()) else {
            eprintln!("NODE_BIN not set: the host process test is skipped");
            return;
        };
        run(async {
            let dir = temp_dir();
            let bin = dir.join("teamsrelay-light");
            let script = format!(
                "#!{node}\nprocess.stdout.write(JSON.stringify([process.argv.slice(2), process.env.NODE_BIN]) + '\\n');\nprocess.on('message', (m) => process.send({{ slot: m.slot, event: 'started' }}));\nsetInterval(() => {{}}, 1000);\n"
            );
            std::fs::write(&bin, script).expect("script");
            std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("chmod");
            let out: Rc<RefCell<Vec<String>>> = Rc::default();
            let o = out.clone();
            let env: Env = [("PATH".to_string(), std::env::var("PATH").unwrap_or_default()), ("NODE_BIN".to_string(), node.clone())].into_iter().collect();
            let (child, mut events) =
                fork_host("/unused/light.cjs", &node, &env, Rc::new(move |l: &str| o.borrow_mut().push(l.into())), Some(bin.to_str().expect("path")));
            child.send(&json!({"op": "start", "slot": 5, "env": {}}));
            let first = tokio::time::timeout(Duration::from_secs(10), events.recv()).await.expect("in time").expect("event");
            assert!(matches!(&first, HostEvent::Message(m) if *m == json!({"slot": 5, "event": "started"})));
            until(|| out.borrow().len() == 1, 5000).await;
            assert_eq!(out.borrow()[0], json!([["--host"], node]).to_string());
            child.kill(libc::SIGTERM);
            let exit = tokio::time::timeout(Duration::from_secs(10), events.recv()).await.expect("in time").expect("event");
            assert!(matches!(&exit, HostEvent::Exit(how) if how == "signal SIGTERM"));

            let (gone, mut events) = fork_host("x", &node, &env, Rc::new(|_: &str| {}), Some("/nonexistent/teamsrelay-light"));
            assert_eq!(gone.pid(), None);
            assert!(matches!(events.recv().await, Some(HostEvent::Exit(how)) if how == "spawn /nonexistent/teamsrelay-light ENOENT"));
        });
    }
}
