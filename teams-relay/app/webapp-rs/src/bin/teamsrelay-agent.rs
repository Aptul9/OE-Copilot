//! TeamsRelay browser agent of one slot in Rust (src/agent/main.ts): drives Teams web in the slot's Chromium over the
//! DevTools protocol, keeps data/N/messages.db for the web app, sends the push notifications.
//!   teamsrelay-agent           runs the agent (environment: src/agent/config.ts)
//!   teamsrelay-agent --check   checks the runtime (SQLite) and the page scripts, then exits

use teamsrelay_web::agent::{info, warn, web};

fn main() {
    if std::env::args().skip(1).any(|a| a == "--check") {
        let sqlite =
            rusqlite::Connection::open_in_memory().and_then(|c| c.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))).unwrap_or_default();
        info("agent", "check ok", &[("sqlite", sqlite.into()), ("scripts", web::check_scripts().into())]);
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let local = tokio::task::LocalSet::new();
    let code = local.block_on(&rt, async {
        // PID 1 in the container: without a handler SIGTERM is ignored and "docker stop" waits for its timeout
        tokio::task::spawn_local(async {
            if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                term.recv().await;
                std::process::exit(0);
            }
        });
        match web::start_agent(std::env::vars().collect()).await {
            Ok(()) => 0,
            Err(e) => {
                warn("agent", &e, &[]);
                1
            }
        }
    });
    std::process::exit(code);
}
