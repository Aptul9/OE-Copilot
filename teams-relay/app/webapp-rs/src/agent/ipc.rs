//! The IPC channel of Node's child_process (fork, stdio "ipc", serialization "json"): one JSON message per line on a
//! Unix socket pair, the child's end at the descriptor NODE_CHANNEL_FD names. The light host is the child of the
//! supervisor on one, and the parent of the calling workers on others.

use serde_json::Value;
use std::os::fd::{FromRawFd, RawFd};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// The sending side: messages queued in order, written by a task of their own. False once the channel is closed.
#[derive(Clone)]
pub struct Sender(mpsc::UnboundedSender<String>);

impl Sender {
    pub fn send(&self, v: &Value) -> bool {
        self.0.send(format!("{}\n", crate::js::stringify(v))).is_ok()
    }

    pub fn connected(&self) -> bool {
        !self.0.is_closed()
    }
}

/// Splits a channel into its sender and the messages it receives; the receiver ends when the other side closes it.
/// The internal messages of Node (cmd NODE_...) are left out.
pub fn open(stream: tokio::net::UnixStream) -> (Sender, mpsc::UnboundedReceiver<Value>) {
    let (read, mut write) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<Value>();
    tokio::task::spawn_local(async move {
        while let Some(line) = out_rx.recv().await {
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    tokio::task::spawn_local(async move {
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v.get("cmd").and_then(Value::as_str).is_some_and(|c| c.starts_with("NODE_")) {
                continue;
            }
            if in_tx.send(v).is_err() {
                break;
            }
        }
    });
    (Sender(out_tx), in_rx)
}

/// The channel this process was given by a Node parent, when there is one.
pub fn from_env() -> Option<tokio::net::UnixStream> {
    let fd: RawFd = std::env::var("NODE_CHANNEL_FD").ok()?.parse().ok()?;
    // SAFETY: the descriptor Node opened for this process, used by nothing else
    let s = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    s.set_nonblocking(true).ok()?;
    tokio::net::UnixStream::from_std(s).ok()
}

/// A socket pair for a Node child: this process keeps the first end, the child gets the second as descriptor 3.
pub fn pair() -> std::io::Result<(tokio::net::UnixStream, std::os::unix::net::UnixStream)> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    ours.set_nonblocking(true)?;
    Ok((tokio::net::UnixStream::from_std(ours)?, theirs))
}

/// The descriptor of the channel in a Node child.
pub const CHILD_FD: RawFd = 3;

/// Starts `cmd` as Node's fork starts a child: the other end of a new channel as its descriptor 3, NODE_CHANNEL_FD and
/// the JSON serialization set (after the environment the caller gave). `own_session`: in a session and process group of
/// its own, as the option detached of Node, so a signal to the group reaches what it starts too.
pub fn spawn_with_channel(cmd: &mut tokio::process::Command, own_session: bool) -> std::io::Result<(tokio::process::Child, tokio::net::UnixStream)> {
    use std::os::fd::AsRawFd;
    let (ours, theirs) = pair()?;
    let fd = theirs.as_raw_fd();
    cmd.env("NODE_CHANNEL_FD", CHILD_FD.to_string()).env("NODE_CHANNEL_SERIALIZATION_MODE", "json");
    // SAFETY: only async-signal-safe calls between fork and exec
    unsafe {
        cmd.pre_exec(move || {
            if own_session && libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if fd == CHILD_FD {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            } else if libc::dup2(fd, CHILD_FD) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    drop(theirs);
    Ok((child, ours))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_lines_both_ways_without_the_messages_of_node() {
        crate::agent::testing::run(async {
            let (a, b) = tokio::net::UnixStream::pair().expect("pair");
            let (a_tx, mut a_rx) = open(a);
            let (b_tx, mut b_rx) = open(b);
            assert!(a_tx.send(&json!({"op": "start", "slot": 2})));
            assert!(b_tx.send(&json!({"cmd": "NODE_HANDLE_ACK"})));
            assert!(b_tx.send(&json!({"slot": 2, "event": "started"})));
            assert_eq!(b_rx.recv().await, Some(json!({"op": "start", "slot": 2})));
            assert_eq!(a_rx.recv().await, Some(json!({"slot": 2, "event": "started"})));
            drop(b_tx);
            drop(b_rx);
        });
    }
}
