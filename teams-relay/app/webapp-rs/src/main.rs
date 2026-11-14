//! teamsrelay-web: the web app (src/server/boot.ts, then the server). `teamsrelay-web --health` asks the running server
//! for /healthz, for the container healthcheck (the image has no shell and no curl).

use teamsrelay_web::{appdb, auth, checks, config, mcp, shared_accounts, slots};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

async fn health() -> bool {
    let check = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", config::port())).await.ok()?;
        s.write_all(b"GET /healthz HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").await.ok()?;
        let mut buf = vec![0u8; 64];
        let n = s.read(&mut buf).await.ok()?;
        Some(buf[..n].starts_with(b"HTTP/1.1 200") || buf[..n].starts_with(b"HTTP/1.0 200"))
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), check).await.ok().flatten().unwrap_or(false)
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    tokio::task::LocalSet::new().block_on(&rt, run());
}

async fn run() {
    if std::env::args().nth(1).as_deref() == Some("--health") {
        std::process::exit(if health().await { 0 } else { 1 });
    }
    if config::auth_secret().len() < 32 {
        fatal("BETTER_AUTH_SECRET must be set, at least 32 characters (openssl rand -hex 32)");
    }
    if let Some(e) = mcp::config_error() {
        fatal(&e);
    }
    // tables first: the Better Auth ones, then those of the app
    if let Err(e) = appdb::with(|c| {
        auth::rows::migrate(c)?;
        appdb::migrate(c)
    }) {
        fatal(&format!("database: {e}"));
    }
    match teamsrelay_web::sync_env_admin().await {
        Ok(m) => println!("{m}"),
        Err(e) => fatal(&e),
    }
    if !config::mcp_token().is_empty() {
        println!("MCP endpoint on: /mcp");
    }
    let push = match config::push() {
        Ok(p) => p,
        Err(e) => fatal(&format!("configuration: {e}")),
    };
    match teamsrelay_web::push::load_service_account(&push.fcm_credentials) {
        Ok(Some(_)) => println!("FCM on: phones of the Android app"),
        Ok(None) => {}
        Err(e) => eprintln!("FCM off: {e}"),
    }
    // a check cut by the restart left its browser running: stopped first, through the same queue
    tokio::task::spawn_local(checks::settle_checks());
    slots::keep_slots_up();
    shared_accounts::keep_in_sync();
    checks::run_checks();
    let (host, port) = (config::bind_host(), config::port());
    let listener = match tokio::net::TcpListener::bind((host.as_str(), port)).await {
        Ok(l) => l,
        Err(e) => fatal(&format!("cannot listen on {host}:{port}: {e}")),
    };
    println!("teamsrelay-web: listening on {host}:{port}");
    let stop = async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    };
    teamsrelay_web::server::serve(listener, stop).await;
}
