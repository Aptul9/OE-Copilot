//! The light host (src/agent/light/host.ts): every light slot of the container in this one process, driven by the
//! supervisor over the IPC channel of fork (src/supervisor/light-host.ts). Each slot runs as work of its own slot (its
//! log lines start with "[slot] "); one that ends while it is wanted is reported and started again after the delay of
//! the supervisor's processes.

use super::config::Env;
use super::{error_text, info, warn};
use crate::push::notify::in_slot;
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// A stop of a slot: set once, seen by the loop at its next round, awaited by what must end with it.
#[derive(Clone)]
pub struct Abort(Rc<(Cell<bool>, Notify)>);

impl Abort {
    pub fn new() -> Abort {
        Abort(Rc::new((Cell::new(false), Notify::new())))
    }

    pub fn abort(&self) {
        self.0.0.set(true);
        self.0.1.notify_waiters();
    }

    pub fn aborted(&self) -> bool {
        self.0.0.get()
    }

    pub async fn wait(&self) {
        while !self.aborted() {
            let n = self.0.1.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            if self.aborted() {
                return;
            }
            n.await;
        }
    }
}

impl Default for Abort {
    fn default() -> Self {
        Abort::new()
    }
}

/// The delays of the supervisor's processes (TIMING of src/supervisor/process.ts).
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub min_delay_ms: u64,
    pub max_delay_ms: u64,
    pub stable_ms: u64,
    pub stop_grace_ms: u64,
}

pub const TIMING: Timing = Timing { min_delay_ms: 1000, max_delay_ms: 60_000, stable_ms: 300_000, stop_grace_ms: 30_000 };

/// Delay before starting again a run that lasted `up_for_ms`: doubles with every exit that follows a short run, back to
/// the minimum after a run of stable_ms or longer.
pub fn restart_delay(up_for_ms: u64, failures: u32, t: Timing) -> (u32, u64) {
    let n = if up_for_ms >= t.stable_ms { 1 } else { failures + 1 };
    (n, t.max_delay_ms.min(t.min_delay_ms.saturating_mul(1u64 << (n - 1).min(40))))
}

/// The run of one slot: until its abort, Err with the reason it ended on its own.
pub type SlotRunner = Rc<dyn Fn(i64, Env, Abort) -> LocalBoxFuture<'static, Result<(), String>>>;

/// When a run ended.
#[derive(Clone)]
struct Done(Rc<(Cell<bool>, Notify)>);

impl Done {
    fn new(done: bool) -> Done {
        Done(Rc::new((Cell::new(done), Notify::new())))
    }

    fn set(&self) {
        self.0.0.set(true);
        self.0.1.notify_waiters();
    }

    async fn wait(&self) {
        while !self.0.0.get() {
            let n = self.0.1.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            if self.0.0.get() {
                return;
            }
            n.await;
        }
    }
}

struct Entry {
    key: String,
    env: Env,
    abort: Abort,
    current: RefCell<Done>,
    timer: RefCell<Option<tokio::task::JoinHandle<()>>>,
    failures: Cell<u32>,
}

pub struct SlotHost {
    run: SlotRunner,
    send: Box<dyn Fn(Value)>,
    timing: Timing,
    slots: RefCell<HashMap<i64, Rc<Entry>>>,
    /// messages of one slot are handled in order: a stop must not overtake the start before it
    queue: RefCell<HashMap<i64, Rc<tokio::sync::Mutex<()>>>>,
    me: std::rc::Weak<SlotHost>,
}

/// The environment as a key: the same variables in any order are the same environment.
fn key_of(env: &Env) -> String {
    crate::js::stringify(&json!(env))
}

impl SlotHost {
    pub fn new(run: SlotRunner, send: Box<dyn Fn(Value)>, timing: Timing) -> Rc<SlotHost> {
        Rc::new_cyclic(|me| SlotHost { run, send, timing, slots: RefCell::default(), queue: RefCell::default(), me: me.clone() })
    }

    /// One message of the supervisor: {op: "start", slot, env} or {op: "stop", slot}.
    pub async fn handle(&self, m: Value) {
        let Some(slot) = m.get("slot").and_then(Value::as_i64) else { return };
        let lock = self.queue.borrow_mut().entry(slot).or_default().clone();
        let _turn = lock.lock().await;
        match m.get("op").and_then(Value::as_str) {
            Some("stop") => {
                self.end(slot).await;
                (self.send)(json!({ "slot": slot, "event": "stopped" }));
            }
            Some("start") => {
                let env: Env = m
                    .get("env")
                    .and_then(Value::as_object)
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), crate::js::string(Some(v)))).collect())
                    .unwrap_or_default();
                let key = key_of(&env);
                if self.slots.borrow().get(&slot).is_some_and(|e| e.key == key) {
                    return;
                }
                self.end(slot).await;
                let e =
                    Rc::new(Entry { key, env, abort: Abort::new(), current: RefCell::new(Done::new(true)), timer: RefCell::new(None), failures: Cell::new(0) });
                self.slots.borrow_mut().insert(slot, e.clone());
                self.begin(slot, e);
            }
            _ => {}
        }
    }

    pub fn running(&self) -> Vec<i64> {
        let mut s: Vec<i64> = self.slots.borrow().keys().copied().collect();
        s.sort();
        s
    }

    /// Every slot aborted; their runs awaited for the grace time.
    pub async fn shutdown(&self, grace: Duration) {
        let all: Vec<Rc<Entry>> = self.slots.borrow_mut().drain().map(|(_, e)| e).collect();
        for e in &all {
            Self::abort(e);
        }
        let waits = all.iter().map(|e| {
            let d = e.current.borrow().clone();
            async move { d.wait().await }
        });
        let _ = tokio::time::timeout(grace, futures_util::future::join_all(waits)).await;
    }

    fn abort(e: &Entry) {
        e.abort.abort();
        if let Some(t) = e.timer.borrow_mut().take() {
            t.abort();
        }
    }

    async fn end(&self, slot: i64) {
        let Some(e) = self.slots.borrow_mut().remove(&slot) else { return };
        Self::abort(&e);
        let done = e.current.borrow().clone();
        let _ = tokio::time::timeout(Duration::from_millis(self.timing.stop_grace_ms), done.wait()).await;
    }

    /// A run that ends while its slot is wanted is reported and started again, like a process of the supervisor.
    fn begin(&self, slot: i64, e: Rc<Entry>) {
        let Some(host) = self.me.upgrade() else { return };
        *e.timer.borrow_mut() = None;
        let done = Done::new(false);
        *e.current.borrow_mut() = done.clone();
        let started = Instant::now();
        let run = (self.run)(slot, e.env.clone(), e.abort.clone());
        tokio::task::spawn_local(async move {
            let how = match in_slot(slot, run).await {
                Ok(()) => "returned".to_string(),
                Err(why) => error_text(&why),
            };
            done.set();
            if e.abort.aborted() {
                return;
            }
            let (failures, delay) = restart_delay(started.elapsed().as_millis() as u64, e.failures.get(), host.timing);
            e.failures.set(failures);
            (host.send)(json!({ "slot": slot, "event": "exited", "how": how }));
            let seconds = crate::js::number_to_string(delay as f64 / 1000.0);
            in_slot(slot, async { warn("light", &format!("slot exited ({how}), starting again in {seconds} s"), &[]) }).await;
            let (h, again) = (host.clone(), e.clone());
            let timer = tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                h.begin(slot, again);
            });
            *e.timer.borrow_mut() = Some(timer);
        });
        (self.send)(json!({ "slot": slot, "event": "started" }));
    }
}

/// The host over the channel of its parent: each message handled, every slot stopped on a disconnect or a signal.
pub async fn run_host() -> Result<(), String> {
    let stream = super::ipc::from_env().ok_or("no IPC channel: the light host runs under the supervisor")?;
    let (to_parent, mut messages) = super::ipc::open(stream);
    let runner: SlotRunner = Rc::new(|slot, env, abort| Box::pin(super::slot::start_slot(slot, env, abort)));
    let host = SlotHost::new(
        runner,
        Box::new(move |m| {
            to_parent.send(&m);
        }),
        TIMING,
    );
    info("light", "host start", &[("pid", (std::process::id() as i64).into())]);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).map_err(|e| e.to_string())?;
    loop {
        tokio::select! {
            m = messages.recv() => match m {
                Some(m) => {
                    let h = host.clone();
                    tokio::task::spawn_local(async move { h.handle(m).await });
                }
                None => break,
            },
            _ = term.recv() => break,
            _ = int.recv() => break,
        }
    }
    host.shutdown(Duration::from_millis(TIMING.stop_grace_ms)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::run;

    const FAST: Timing = Timing { min_delay_ms: 10, max_delay_ms: 40, stable_ms: 1000, stop_grace_ms: 200 };

    type Calls = Rc<RefCell<Vec<(i64, Env, Abort, Option<i64>)>>>;
    type Behave = Rc<dyn Fn(i64, usize, Abort) -> LocalBoxFuture<'static, Result<(), String>>>;

    fn until_abort(a: Abort) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async move {
            a.wait().await;
            Ok(())
        })
    }

    fn setup(behave: Behave) -> (Rc<SlotHost>, Calls, Rc<RefCell<Vec<Value>>>) {
        let calls: Calls = Rc::default();
        let sent: Rc<RefCell<Vec<Value>>> = Rc::default();
        let c = calls.clone();
        let runner: SlotRunner = Rc::new(move |slot, env, abort| {
            c.borrow_mut().push((slot, env, abort.clone(), crate::push::notify::current_slot()));
            let n = c.borrow().iter().filter(|x| x.0 == slot).count();
            behave(slot, n, abort)
        });
        let s = sent.clone();
        (SlotHost::new(runner, Box::new(move |m| s.borrow_mut().push(m)), FAST), calls, sent)
    }

    fn env(pairs: &[(&str, &str)]) -> Value {
        Value::Object(pairs.iter().map(|(k, v)| (k.to_string(), json!(v))).collect())
    }

    fn of(calls: &Calls, slot: i64) -> usize {
        calls.borrow().iter().filter(|c| c.0 == slot).count()
    }

    async fn until(check: impl Fn() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out");
    }

    #[test]
    fn runs_each_slot_with_its_own_environment_and_reports_it_started() {
        run(async {
            let (host, calls, sent) = setup(Rc::new(|_, _, a| until_abort(a)));
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[("ACCOUNT", "2")]) })).await;
            host.handle(json!({ "op": "start", "slot": 3, "env": env(&[("ACCOUNT", "3")]) })).await;
            tokio::task::yield_now().await;
            assert_eq!(calls.borrow().iter().map(|c| (c.0, c.1["ACCOUNT"].clone(), c.3)).collect::<Vec<_>>(), [(2, "2".into(), None), (3, "3".into(), None)]);
            assert_eq!(*sent.borrow(), [json!({"slot": 2, "event": "started"}), json!({"slot": 3, "event": "started"})]);
            assert_eq!(host.running(), [2, 3]);
            host.shutdown(Duration::from_millis(50)).await;
        });
    }

    #[test]
    fn starts_a_failed_slot_again_after_a_doubling_delay_and_leaves_the_others_alone() {
        run(async {
            let (host, calls, sent) =
                setup(Rc::new(|slot, n, a| if slot == 2 && n < 3 { Box::pin(async { Err("again\nsecond line".to_string()) }) } else { until_abort(a) }));
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[]) })).await;
            host.handle(json!({ "op": "start", "slot": 3, "env": env(&[]) })).await;
            until(|| of(&calls, 2) == 3).await;
            assert!(sent.borrow().contains(&json!({ "slot": 2, "event": "exited", "how": "again" })));
            assert_eq!(sent.borrow().iter().filter(|m| m["slot"] == 2 && m["event"] == "started").count(), 3);
            assert_eq!(of(&calls, 3), 1);
            assert!(!sent.borrow().iter().any(|m| m["slot"] == 3 && m["event"] != "started"));
            assert_eq!(restart_delay(5, 1, FAST), (2, 20));
            assert_eq!(restart_delay(2000, 5, FAST), (1, 10));
            host.shutdown(Duration::from_millis(50)).await;
        });
    }

    #[test]
    fn a_start_with_the_same_environment_does_nothing_another_rebinds() {
        run(async {
            let (host, calls, sent) = setup(Rc::new(|_, _, a| until_abort(a)));
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[("A", "1"), ("B", "2")]) })).await;
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[("B", "2"), ("A", "1")]) })).await;
            assert_eq!((of(&calls, 2), sent.borrow().len()), (1, 1));
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[("A", "1"), ("B", "3")]) })).await;
            assert_eq!(of(&calls, 2), 2);
            assert!(calls.borrow()[0].2.aborted() && !calls.borrow()[1].2.aborted());
            assert_eq!(calls.borrow()[1].1["B"], "3");
            host.shutdown(Duration::from_millis(50)).await;
        });
    }

    #[test]
    fn stops_a_slot_without_starting_it_again() {
        run(async {
            let (host, calls, sent) = setup(Rc::new(|_, _, a| {
                Box::pin(async move {
                    a.wait().await;
                    Err("aborted".to_string())
                })
            }));
            host.handle(json!({ "op": "start", "slot": 3, "env": env(&[]) })).await;
            host.handle(json!({ "op": "stop", "slot": 3 })).await;
            assert!(calls.borrow()[0].2.aborted());
            assert_eq!(*sent.borrow(), [json!({"slot": 3, "event": "started"}), json!({"slot": 3, "event": "stopped"})]);
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!((of(&calls, 3), sent.borrow().len()), (1, 2));
            assert!(host.running().is_empty());
            // a stop of an unknown slot is answered at once
            host.handle(json!({ "op": "stop", "slot": 9 })).await;
            assert_eq!(sent.borrow().last(), Some(&json!({"slot": 9, "event": "stopped"})));
        });
    }

    #[test]
    fn a_slot_stopped_while_it_waits_for_the_delay_does_not_start_again() {
        run(async {
            let (host, calls, sent) = setup(Rc::new(|_, _, _| Box::pin(async { Err("down".to_string()) })));
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[]) })).await;
            until(|| sent.borrow().contains(&json!({ "slot": 2, "event": "exited", "how": "down" }))).await;
            host.handle(json!({ "op": "stop", "slot": 2 })).await;
            let n = of(&calls, 2);
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(of(&calls, 2), n);
            assert_eq!(sent.borrow().last(), Some(&json!({"slot": 2, "event": "stopped"})));
        });
    }

    #[test]
    fn a_stop_and_a_shutdown_give_up_waiting_after_the_grace() {
        run(async {
            let (host, _, sent) = setup(Rc::new(|_, _, _| Box::pin(futures_util::future::pending())));
            host.handle(json!({ "op": "start", "slot": 2, "env": env(&[]) })).await;
            let t0 = Instant::now();
            host.handle(json!({ "op": "stop", "slot": 2 })).await;
            assert!(t0.elapsed() >= Duration::from_millis(150));
            assert_eq!(sent.borrow().last(), Some(&json!({"slot": 2, "event": "stopped"})));
            host.handle(json!({ "op": "start", "slot": 3, "env": env(&[]) })).await;
            let t0 = Instant::now();
            host.shutdown(Duration::from_millis(100)).await;
            assert!(t0.elapsed() >= Duration::from_millis(80) && t0.elapsed() < Duration::from_millis(1000));
        });
    }

    #[test]
    fn the_runs_are_work_of_their_slot() {
        run(async {
            let seen: Rc<RefCell<Vec<Option<i64>>>> = Rc::default();
            let s = seen.clone();
            let runner: SlotRunner = Rc::new(move |_, _, a| {
                let s = s.clone();
                Box::pin(async move {
                    s.borrow_mut().push(crate::push::notify::current_slot());
                    a.wait().await;
                    Ok(())
                })
            });
            let host = SlotHost::new(runner, Box::new(|_| {}), FAST);
            host.handle(json!({ "op": "start", "slot": 6, "env": {} })).await;
            tokio::task::yield_now().await;
            assert_eq!(*seen.borrow(), [Some(6)]);
            host.shutdown(Duration::from_millis(50)).await;
        });
    }
}
