//! A process kept running until stopped (src/supervisor/process.ts), like a container with "restart: unless-stopped".
//! It runs in a session of its own, so the signals of a stop reach what it started too (the zygote and renderers of
//! Chromium).

use super::config::Env;
use super::{Event, exit_text, signal_group, spawn_error};
use crate::agent::host::{TIMING, Timing, restart_delay};
use futures_util::future::LocalBoxFuture;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::io::AsyncBufReadExt;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    pub file: String,
    pub args: Vec<String>,
    /// the whole environment; None: this process's
    pub env: Option<Env>,
    pub cwd: Option<PathBuf>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

/// What the accounts start and stop: a process of its own, or a slot of the light host.
pub trait ProcessLike {
    fn name(&self) -> String;
    fn running(&self) -> bool;
    /// running, or waiting to start again after an exit
    fn active(&self) -> bool;
    fn pid(&self) -> Option<u32>;
    fn restarts(&self) -> u32;
    fn last_exit(&self) -> Option<String>;
    fn start(&self);
    fn stop(self: Rc<Self>) -> LocalBoxFuture<'static, ()>;
}

pub type Lines = Rc<dyn Fn(&str)>;

struct Running {
    id: u64,
    pid: u32,
    gone: Event,
}

#[derive(Default)]
struct State {
    child: Option<Running>,
    wanted: bool,
    timer: Option<tokio::task::JoinHandle<()>>,
    failures: u32,
    started_at: Option<Instant>,
    restarts: u32,
    last_exit: Option<String>,
    seq: u64,
}

pub struct Supervised {
    name: String,
    command: Box<dyn Fn() -> Command>,
    log: Lines,
    output: Option<Lines>,
    timing: Timing,
    st: RefCell<State>,
    me: std::rc::Weak<Supervised>,
}

impl Supervised {
    pub fn new(name: &str, command: Box<dyn Fn() -> Command>, log: Lines, output: Option<Lines>, timing: Timing) -> Rc<Supervised> {
        Rc::new_cyclic(|me| Supervised { name: name.into(), command, log, output, timing, st: RefCell::default(), me: me.clone() })
    }

    pub fn with_defaults(name: &str, command: Box<dyn Fn() -> Command>, log: Lines, output: Option<Lines>) -> Rc<Supervised> {
        Self::new(name, command, log, output, TIMING)
    }

    fn spawn(&self) {
        let Some(me) = self.me.upgrade() else { return };
        let c = (self.command)();
        let id = {
            let mut s = self.st.borrow_mut();
            s.timer = None;
            s.seq += 1;
            s.started_at = Some(Instant::now());
            s.seq
        };
        let mut cmd = tokio::process::Command::new(&c.file);
        cmd.args(&c.args).stdin(std::process::Stdio::null());
        if let Some(env) = &c.env {
            cmd.env_clear().envs(env);
        }
        if let Some(cwd) = &c.cwd {
            cmd.current_dir(cwd);
        }
        if let Some(uid) = c.uid {
            cmd.uid(uid);
        }
        if let Some(gid) = c.gid {
            cmd.gid(gid);
        }
        let piped = self.output.is_some();
        let io = || if piped { std::process::Stdio::piped() } else { std::process::Stdio::null() };
        cmd.stdout(io()).stderr(io());
        // SAFETY: setsid only, between fork and exec
        unsafe {
            cmd.pre_exec(|| if libc::setsid() < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                // as the "error" event of Node: an exit that is started again
                let how = spawn_error(&c.file, &e);
                self.st.borrow_mut().child = Some(Running { id, pid: 0, gone: Event::default() });
                self.exited(id, how);
                return;
            }
        };
        let gone = Event::default();
        self.st.borrow_mut().child = Some(Running { id, pid: child.id().unwrap_or(0), gone: gone.clone() });
        if let Some(output) = &self.output {
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
        }
        tokio::task::spawn_local(async move {
            let status = child.wait().await.ok();
            gone.set();
            me.exited(id, exit_text(status));
        });
    }

    /// The end of run `id`; a later run, or none, leaves it be.
    fn exited(&self, id: u64, how: String) {
        let delay = {
            let mut s = self.st.borrow_mut();
            if s.child.as_ref().is_none_or(|c| c.id != id) {
                return;
            }
            if let Some(c) = s.child.take() {
                c.gone.set();
            }
            s.last_exit = Some(how.clone());
            if !s.wanted {
                return;
            }
            let up = s.started_at.map_or(0, |t| t.elapsed().as_millis() as u64);
            let (failures, delay) = restart_delay(up, s.failures, self.timing);
            s.failures = failures;
            delay
        };
        (self.log)(&format!("{} exited ({how}), starting again in {} s", self.name, crate::js::number_to_string(delay as f64 / 1000.0)));
        let Some(me) = self.me.upgrade() else { return };
        let timer = tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            me.st.borrow_mut().restarts += 1;
            me.spawn();
        });
        self.st.borrow_mut().timer = Some(timer);
    }
}

impl ProcessLike for Supervised {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn running(&self) -> bool {
        self.st.borrow().child.is_some()
    }

    fn active(&self) -> bool {
        self.st.borrow().wanted
    }

    fn pid(&self) -> Option<u32> {
        self.st.borrow().child.as_ref().map(|c| c.pid).filter(|p| *p > 0)
    }

    fn restarts(&self) -> u32 {
        self.st.borrow().restarts
    }

    fn last_exit(&self) -> Option<String> {
        self.st.borrow().last_exit.clone()
    }

    fn start(&self) {
        let idle = {
            let mut s = self.st.borrow_mut();
            s.wanted = true;
            s.child.is_none() && s.timer.is_none()
        };
        if idle {
            self.spawn();
        }
    }

    fn stop(self: Rc<Self>) -> LocalBoxFuture<'static, ()> {
        Box::pin(async move {
            let running = {
                let mut s = self.st.borrow_mut();
                s.wanted = false;
                if let Some(t) = s.timer.take() {
                    t.abort();
                }
                s.child.as_ref().map(|c| (c.pid, c.gone.clone()))
            };
            let Some((pid, gone)) = running else { return };
            signal_group(pid, libc::SIGTERM);
            let grace = self.timing.stop_grace_ms;
            let killer = gone.clone();
            let kill = tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_millis(grace)).await;
                if !killer.is_set() {
                    signal_group(pid, libc::SIGKILL);
                }
            });
            gone.wait().await;
            kill.abort();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::run;
    use crate::supervisor::testing::until;

    const FAST: Timing = Timing { min_delay_ms: 50, max_delay_ms: 400, stable_ms: 60_000, stop_grace_ms: 500 };

    fn sh(code: &str) -> Command {
        Command { file: "/bin/sh".into(), args: vec!["-c".into(), code.into()], ..Default::default() }
    }

    fn supervised(c: Command, output: Option<Lines>, timing: Timing) -> Rc<Supervised> {
        Supervised::new("test", Box::new(move || c.clone()), Rc::new(|_| {}), output, timing)
    }

    #[test]
    fn restart_delays_double_up_to_the_maximum_and_reset_after_a_stable_run() {
        let t = Timing { min_delay_ms: 1000, max_delay_ms: 60_000, stable_ms: 300_000, stop_grace_ms: 10_000 };
        let mut failures = 0;
        let mut delays = vec![];
        for _ in 0..8 {
            let (f, d) = restart_delay(5_000, failures, t);
            failures = f;
            delays.push(d);
        }
        assert_eq!(delays, [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000]);
        assert_eq!(restart_delay(300_000, 6, t), (1, 1000));
    }

    #[test]
    fn starts_the_process_again_after_it_exits_and_stops_it_for_good() {
        run(async {
            let p = supervised(sh("exit 3"), None, FAST);
            p.start();
            until(|| p.restarts() >= 2, 10_000).await;
            assert_eq!(p.last_exit().as_deref(), Some("code 3"));
            p.clone().stop().await;

            let p = supervised(sh("sleep 60"), None, FAST);
            p.start();
            assert!(p.running() && p.pid().is_some());
            p.clone().stop().await;
            assert!(!p.running());
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!((p.running(), p.restarts(), p.last_exit().as_deref()), (false, 0, Some("signal SIGTERM")));
        });
    }

    #[test]
    fn cancels_a_restart_still_waiting_and_retries_a_program_that_cannot_start() {
        run(async {
            let p = supervised(sh("exit 1"), None, Timing { min_delay_ms: 300, ..FAST });
            p.start();
            until(|| p.last_exit().is_some(), 10_000).await;
            p.clone().stop().await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            assert_eq!((p.running(), p.restarts()), (false, 0));

            let p = supervised(Command { file: "/nonexistent/teamsrelay-test-binary".into(), ..Default::default() }, None, FAST);
            p.start();
            until(|| p.restarts() >= 1, 10_000).await;
            assert_eq!(p.last_exit().as_deref(), Some("spawn /nonexistent/teamsrelay-test-binary ENOENT"));
            p.clone().stop().await;
        });
    }

    #[test]
    fn passes_the_output_lines_on_and_kills_what_ignores_sigterm_with_its_children() {
        run(async {
            let lines: Rc<RefCell<Vec<String>>> = Rc::default();
            let l = lines.clone();
            let code = "echo one; echo two >&2; trap '' TERM; sleep 60 & echo child $!; while true; do sleep 1; done";
            let p = supervised(sh(code), Some(Rc::new(move |s: &str| l.borrow_mut().push(s.to_string()))), FAST);
            p.start();
            until(|| lines.borrow().iter().any(|l| l.starts_with("child ")), 10_000).await;
            let mut got: Vec<String> = lines.borrow().iter().filter(|l| !l.starts_with("child ")).cloned().collect();
            got.sort();
            assert_eq!(got, ["one", "two"]);
            let child: i32 = lines.borrow().iter().find_map(|l| l.strip_prefix("child ").map(|p| p.parse().expect("pid"))).expect("child");
            let t0 = Instant::now();
            p.clone().stop().await;
            assert!(t0.elapsed() >= Duration::from_millis(400));
            assert_eq!(p.last_exit().as_deref(), Some("signal SIGKILL"));
            // SAFETY: signal 0 only asks whether the process exists
            until(|| unsafe { libc::kill(child, 0) } != 0, 3000).await;
        });
    }
}
