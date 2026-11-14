//! The supervisor of TeamsRelay in Rust (src/supervisor): the browser and the agent of every Teams account, in the
//! browsers container, started, stopped and wiped on request of the web app over a unix socket; the light slots in the
//! light host (agent/host.rs, or node light.cjs --host); the remote desktop on demand; the shared Microsoft accounts of
//! the OE calendar agent. Runs as root, an s6 service of the image.

pub mod accounts;
pub mod broker;
pub mod cdp;
pub mod config;
pub mod desktop;
pub mod light_host;
pub mod process;
pub mod server;

use std::cell::Cell;
use std::rc::Rc;
use tokio::sync::Notify;

/// A line of the supervisor: "supervisor: <message>".
pub fn say(message: &str) {
    println!("{}", crate::push::notify::format("supervisor", message, &[]));
}

/// Something that happens once (a process gone, a slot answered): set, and awaited by any number of waiters.
#[derive(Clone, Default)]
pub struct Event(Rc<(Cell<bool>, Notify)>);

impl Event {
    pub fn set(&self) {
        self.0.0.set(true);
        self.0.1.notify_waiters();
    }

    pub fn is_set(&self) -> bool {
        self.0.0.get()
    }

    pub async fn wait(&self) {
        while !self.is_set() {
            let n = self.0.1.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            if self.is_set() {
                return;
            }
            n.await;
        }
    }
}

/// The text Node gives a process that could not be started, e.g. "spawn /usr/bin/chromium ENOENT".
pub fn spawn_error(file: &str, e: &std::io::Error) -> String {
    let code = match e.raw_os_error() {
        Some(libc::ENOENT) => "ENOENT",
        Some(libc::EACCES) => "EACCES",
        Some(libc::ENOTDIR) => "ENOTDIR",
        Some(libc::EPERM) => "EPERM",
        _ => return format!("spawn {file} {e}"),
    };
    format!("spawn {file} {code}")
}

/// A signal to the process group of `pid` (the process started in a session of its own); gone already is fine.
pub fn signal_group(pid: u32, sig: i32) {
    if pid > 0 {
        // SAFETY: a signal to a process group this process started
        unsafe { libc::kill(-(pid as i32), sig) };
    }
}

/// How a process ended, as Node words it: "code 3" or "signal SIGTERM".
pub fn exit_text(status: Option<std::process::ExitStatus>) -> String {
    use std::os::unix::process::ExitStatusExt;
    match status {
        Some(s) => match s.signal() {
            Some(sig) => format!("signal {}", crate::agent::calls::signal_name(sig)),
            None => format!("code {}", s.code().map_or("null".to_string(), |c| c.to_string())),
        },
        None => "code null".into(),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::time::Duration;

    /// Waits for `check`, failing the test after `ms`.
    pub async fn until(check: impl Fn() -> bool, ms: u64) {
        let end = tokio::time::Instant::now() + Duration::from_millis(ms);
        while !check() {
            assert!(tokio::time::Instant::now() < end, "condition not met in time");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}
