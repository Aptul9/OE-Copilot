//! TeamsRelay light agent in Rust (src/agent/light/main.ts):
//!   teamsrelay-light --host    runs the slots the supervisor sends over IPC, all in this process
//!   teamsrelay-light --check   checks the runtime (SQLite), then exits
//!   teamsrelay-light           runs the one slot of its environment until SIGTERM

use teamsrelay_web::agent::{host, info, slot, warn};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--check") {
        let sqlite =
            rusqlite::Connection::open_in_memory().and_then(|c| c.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))).unwrap_or_default();
        info("light", "check ok", &[("sqlite", sqlite.into())]);
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let local = tokio::task::LocalSet::new();
    let code = local.block_on(&rt, async {
        let done = if args.iter().any(|a| a == "--host") {
            host::run_host().await
        } else {
            let env = std::env::vars().collect();
            let abort = host::Abort::new();
            let stop = abort.clone();
            tokio::task::spawn_local(async move {
                if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    term.recv().await;
                }
                stop.abort();
            });
            slot::start_slot(0, env, abort).await
        };
        match done {
            Ok(()) => 0,
            Err(e) => {
                warn("light", &e, &[]);
                1
            }
        }
    });
    std::process::exit(code);
}
