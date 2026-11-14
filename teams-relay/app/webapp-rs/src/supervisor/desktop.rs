//! The remote desktop of the image (linuxserver Selkies) as s6-rc services, up while a browser needs it
//! (src/supervisor/desktop.ts). svc-de pulls in xorg, nginx and selkies; dbus and xsettingsd hang on xorg without being
//! dependencies of it. Start and stop go one after the other, so a start asked while the desktop goes down waits for it.

use futures_util::future::LocalBoxFuture;
use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

const START: [&str; 3] = ["svc-de", "svc-xsettingsd", "svc-dbus"];
const ALL: [&str; 7] = ["svc-de", "svc-xsettingsd", "svc-dbus", "svc-selkies", "svc-nginx", "svc-xorg", "svc-pulseaudio"];

/// A program run to its end: Err with why it failed.
pub type Run = Rc<dyn Fn(String, Vec<String>) -> LocalBoxFuture<'static, Result<(), String>>>;

/// The program, killed after 60 s; a failure in the words of Node's execFile.
pub fn run_command() -> Run {
    Rc::new(|file: String, args: Vec<String>| {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(&file);
            cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
            let line = format!("{file} {}", args.join(" "));
            let out = match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => return Err(super::spawn_error(&file, &e)),
                Err(_) => return Err(format!("Command failed: {line} (timed out)")),
            };
            if out.status.success() {
                return Ok(());
            }
            let err = String::from_utf8_lossy(&out.stderr);
            Err(format!("Command failed: {}\n{}", line.trim_end(), err.trim()))
        })
    })
}

pub struct Options {
    /// the Wayland socket of the session: the desktop is up when it exists
    pub socket: String,
    /// /command/s6-rc; None where there is no s6: the desktop is taken as running
    pub s6rc: Option<String>,
    /// script that makes the virtual microphone, run once the desktop is up
    pub microphone: Option<String>,
    /// stop after this long without a browser; infinite: never
    pub idle_ms: f64,
    pub log: Rc<dyn Fn(&str)>,
    pub run: Run,
    pub exists: Rc<dyn Fn(&str) -> bool>,
    pub now: Rc<dyn Fn() -> f64>,
    pub poll_ms: u64,
    pub wait_ms: f64,
}

impl Options {
    pub fn real(socket: String, s6rc: Option<String>, microphone: Option<String>, idle_ms: f64, log: Rc<dyn Fn(&str)>) -> Options {
        Options {
            socket,
            s6rc,
            microphone,
            idle_ms,
            log,
            run: run_command(),
            exists: Rc::new(|f: &str| std::path::Path::new(f).exists()),
            now: Rc::new(|| crate::web::now_ms() as f64),
            poll_ms: 250,
            wait_ms: 60_000.0,
        }
    }
}

pub struct Desktop {
    o: Options,
    up: Cell<bool>,
    last_used: Cell<f64>,
    queue: tokio::sync::Mutex<()>,
}

impl Desktop {
    pub fn new(o: Options) -> Rc<Desktop> {
        let now = (o.now)();
        Rc::new(Desktop { o, up: Cell::new(false), last_used: Cell::new(now), queue: tokio::sync::Mutex::new(()) })
    }

    pub fn running(&self) -> bool {
        self.up.get()
    }

    /// Up when this resolves, whoever else asked at the same time. The ask counts from the call.
    pub fn ensure(self: &Rc<Self>) -> impl Future<Output = Result<(), String>> + use<> {
        self.last_used.set((self.o.now)());
        let me = self.clone();
        async move {
            let _turn = me.queue.lock().await;
            me.start().await
        }
    }

    /// Once a minute, with whether a browser runs: the desktop stops after idle_ms without one.
    pub fn tick(self: &Rc<Self>, browser_running: bool) -> impl Future<Output = Result<(), String>> + use<> {
        let due = if browser_running {
            self.last_used.set((self.o.now)());
            false
        } else {
            self.up.get() && self.o.s6rc.is_some() && (self.o.now)() - self.last_used.get() >= self.o.idle_ms
        };
        let me = self.clone();
        async move {
            if !due {
                return Ok(());
            }
            let _turn = me.queue.lock().await;
            me.stop().await
        }
    }

    async fn start(&self) -> Result<(), String> {
        if self.up.get() {
            return Ok(());
        }
        // s6-rc takes what is already up as it is, so this is also right when the desktop never went down
        if let Some(s6rc) = &self.o.s6rc {
            (self.o.log)("desktop starting");
            // -b: wait for the lock of the database instead of failing; the boot transition of s6 holds it at first
            let mut args: Vec<String> = ["-b", "-u", "change"].iter().map(|s| s.to_string()).collect();
            args.extend(START.iter().map(|s| s.to_string()));
            (self.o.run)(s6rc.clone(), args).await?;
        }
        self.wait_for_socket().await?;
        if let Some(mic) = &self.o.microphone
            && let Err(e) = (self.o.run)(mic.clone(), vec![]).await
        {
            (self.o.log)(&format!("desktop microphone: {}", crate::push::notify::error_text(&e)));
        }
        self.up.set(true);
        (self.o.log)("desktop up");
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        // a browser, or a start that came while this waited its turn, keeps the desktop
        let Some(s6rc) = &self.o.s6rc else { return Ok(()) };
        let idle = (self.o.now)() - self.last_used.get();
        if !self.up.get() || idle < self.o.idle_ms {
            return Ok(());
        }
        (self.o.log)(&format!("desktop stopping: no browser for {} min", (idle / 60_000.0).round()));
        let mut args: Vec<String> = ["-b", "-d", "change"].iter().map(|s| s.to_string()).collect();
        args.extend(ALL.iter().map(|s| s.to_string()));
        (self.o.run)(s6rc.clone(), args).await?;
        self.up.set(false);
        Ok(())
    }

    async fn wait_for_socket(&self) -> Result<(), String> {
        let end = (self.o.now)() + self.o.wait_ms;
        while !(self.o.exists)(&self.o.socket) {
            if (self.o.now)() > end {
                return Err(format!("the desktop did not come up within {} s", crate::js::number_to_string(self.o.wait_ms / 1000.0)));
            }
            tokio::time::sleep(Duration::from_millis(self.o.poll_ms)).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::run;
    use std::cell::RefCell;
    use std::collections::HashSet;

    const SOCKET: &str = "/xdg/wayland-0";
    const START_LINE: &str = "/command/s6-rc -b -u change svc-de svc-xsettingsd svc-dbus";
    const STOP_LINE: &str = "/command/s6-rc -b -d change svc-de svc-xsettingsd svc-dbus svc-selkies svc-nginx svc-xorg svc-pulseaudio";

    #[derive(Default)]
    struct Ctx {
        calls: RefCell<Vec<String>>,
        present: RefCell<HashSet<String>>,
        t: Cell<f64>,
        delay: Cell<u64>,
        fail: Cell<bool>,
        fail_file: RefCell<String>,
        hold_stop: RefCell<Option<tokio::sync::oneshot::Receiver<()>>>,
        log: RefCell<Vec<String>>,
    }

    fn setup(tweak: impl FnOnce(&mut Options)) -> (Rc<Desktop>, Rc<Ctx>) {
        let ctx = Rc::new(Ctx::default());
        ctx.t.set(1_000_000.0);
        let c = ctx.clone();
        let run: Run = Rc::new(move |file: String, args: Vec<String>| {
            let c = c.clone();
            Box::pin(async move {
                c.calls.borrow_mut().push(std::iter::once(file.clone()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" "));
                if c.fail.get() {
                    return Err("s6-rc failed".to_string());
                }
                if file == *c.fail_file.borrow() {
                    return Err("no pactl".to_string());
                }
                if args.iter().any(|a| a == "-u") {
                    let (c2, d) = (c.clone(), c.delay.get());
                    tokio::task::spawn_local(async move {
                        tokio::time::sleep(Duration::from_millis(d)).await;
                        c2.present.borrow_mut().insert(SOCKET.into());
                    });
                }
                if args.iter().any(|a| a == "-d") {
                    let hold = c.hold_stop.borrow_mut().take();
                    if let Some(h) = hold {
                        let _ = h.await;
                    }
                    c.present.borrow_mut().remove(SOCKET);
                }
                Ok(())
            })
        });
        let (e, n, l) = (ctx.clone(), ctx.clone(), ctx.clone());
        let mut o = Options {
            socket: SOCKET.into(),
            s6rc: Some("/command/s6-rc".into()),
            microphone: None,
            idle_ms: 600_000.0,
            log: Rc::new(move |s: &str| l.log.borrow_mut().push(s.into())),
            run,
            exists: Rc::new(move |f: &str| e.present.borrow().contains(f)),
            now: Rc::new(move || n.t.get()),
            poll_ms: 1,
            wait_ms: 500.0,
        };
        tweak(&mut o);
        (Desktop::new(o), ctx)
    }

    fn calls(c: &Ctx) -> Vec<String> {
        c.calls.borrow().clone()
    }

    #[test]
    fn starts_once_for_callers_that_ask_together_and_not_again_while_up() {
        run(async {
            let (d, ctx) = setup(|_| {});
            ctx.delay.set(20);
            let (a, b, c) = (d.ensure(), d.ensure(), d.ensure());
            let (a, b, c) = tokio::join!(a, b, c);
            assert!(a.is_ok() && b.is_ok() && c.is_ok());
            d.ensure().await.expect("up");
            assert_eq!(calls(&ctx), [START_LINE]);
            assert!(d.running() && ctx.present.borrow().contains(SOCKET));
        });
    }

    #[test]
    fn runs_the_microphone_once_up_and_carries_on_when_it_fails() {
        run(async {
            let (d, ctx) = setup(|o| o.microphone = Some("/usr/local/bin/desktop-microphone".into()));
            d.ensure().await.expect("up");
            assert_eq!(calls(&ctx), [START_LINE, "/usr/local/bin/desktop-microphone"]);
            let (d, ctx) = setup(|o| o.microphone = Some("/m".into()));
            *ctx.fail_file.borrow_mut() = "/m".into();
            d.ensure().await.expect("up");
            assert!(ctx.log.borrow().contains(&"desktop microphone: no pactl".to_string()));
            assert!(d.running());
        });
    }

    #[test]
    fn stops_after_the_idle_time_without_a_browser_and_starts_again() {
        run(async {
            let (d, ctx) = setup(|_| {});
            d.ensure().await.expect("up");
            ctx.t.set(ctx.t.get() + 9.0 * 60_000.0);
            d.tick(false).await.expect("tick");
            d.tick(true).await.expect("tick");
            ctx.t.set(ctx.t.get() + 9.0 * 60_000.0);
            d.tick(false).await.expect("tick");
            assert_eq!(calls(&ctx), [START_LINE]);
            ctx.t.set(ctx.t.get() + 2.0 * 60_000.0);
            d.tick(false).await.expect("tick");
            assert_eq!(calls(&ctx), [START_LINE, STOP_LINE]);
            assert!(!d.running());
            d.ensure().await.expect("up");
            assert_eq!(calls(&ctx), [START_LINE, STOP_LINE, START_LINE]);
        });
    }

    #[test]
    fn a_start_asked_while_a_stop_waits_keeps_the_desktop_and_one_asked_during_a_stop_brings_it_back() {
        run(async {
            let (d, ctx) = setup(|_| {});
            d.ensure().await.expect("up");
            ctx.t.set(ctx.t.get() + 11.0 * 60_000.0);
            let (stopping, starting) = (d.tick(false), d.ensure());
            let (a, b) = tokio::join!(stopping, starting);
            assert!(a.is_ok() && b.is_ok());
            assert_eq!(calls(&ctx), [START_LINE]);
            assert!(d.running());

            let (d, ctx) = setup(|_| {});
            let (release, hold) = tokio::sync::oneshot::channel();
            *ctx.hold_stop.borrow_mut() = Some(hold);
            d.ensure().await.expect("up");
            ctx.t.set(ctx.t.get() + 11.0 * 60_000.0);
            let stopping = tokio::task::spawn_local(d.tick(false));
            crate::supervisor::testing::until(|| calls(&ctx).len() == 2, 2000).await;
            let starting = d.ensure();
            let _ = release.send(());
            stopping.await.expect("task").expect("stop");
            starting.await.expect("up");
            assert_eq!(calls(&ctx), [START_LINE, STOP_LINE, START_LINE]);
            assert!(d.running());
        });
    }

    #[test]
    fn a_failed_start_rejects_its_callers_and_is_tried_again_at_the_next_ask() {
        run(async {
            let (d, ctx) = setup(|_| {});
            ctx.fail.set(true);
            let (a, b) = tokio::join!(d.ensure(), d.ensure());
            assert_eq!((a.unwrap_err(), b.unwrap_err()), ("s6-rc failed".to_string(), "s6-rc failed".to_string()));
            assert!(!d.running());
            ctx.fail.set(false);
            d.ensure().await.expect("up");
            assert!(d.running());
        });
    }

    #[test]
    fn never_stops_with_an_infinite_idle_time_or_without_s6_and_gives_up_without_a_socket() {
        run(async {
            let (d, ctx) = setup(|o| o.idle_ms = f64::INFINITY);
            d.ensure().await.expect("up");
            ctx.t.set(ctx.t.get() + 24.0 * 3_600_000.0);
            d.tick(false).await.expect("tick");
            assert_eq!(calls(&ctx), [START_LINE]);

            let (d, ctx) = setup(|o| o.s6rc = None);
            ctx.present.borrow_mut().insert(SOCKET.into());
            d.ensure().await.expect("up");
            ctx.t.set(ctx.t.get() + 24.0 * 3_600_000.0);
            d.tick(false).await.expect("tick");
            assert!(calls(&ctx).is_empty() && d.running());

            let (d, _) = setup(|o| {
                o.s6rc = None;
                o.socket = "/never".into();
                o.now = Rc::new(|| crate::web::now_ms() as f64);
                o.wait_ms = 40.0;
            });
            assert!(d.ensure().await.unwrap_err().contains("did not come up"));
            assert!(!d.running());
        });
    }
}
