//! TeamsRelay supervisor in Rust (src/supervisor/main.ts): the browser and the agent of every Teams account, in the
//! browsers container, started, stopped and wiped on request of the web app. Runs as root, an s6 service of the image.
//!   teamsrelay-supervisor           runs the supervisor (environment: supervisor/config.rs)
//!   teamsrelay-supervisor --check   loads the configuration, then exits
//!   teamsrelay-supervisor status    prints the state of the accounts, asked to the running supervisor

use std::rc::Rc;
use std::time::Duration;
use teamsrelay_web::push::notify::error_text;
use teamsrelay_web::supervisor::accounts::{Accounts, Deps};
use teamsrelay_web::supervisor::broker::{Capture, Directory};
use teamsrelay_web::supervisor::cdp::Browsers;
use teamsrelay_web::supervisor::config::{Config, from_process};
use teamsrelay_web::supervisor::desktop::{Desktop, Options};
use teamsrelay_web::supervisor::server::{Control, listen, serve};
use teamsrelay_web::supervisor::{process::Lines, say};

fn main() {
    let cfg = match from_process() {
        Ok(c) => c,
        Err(e) => {
            say(&format!("configuration: {e}"));
            std::process::exit(1);
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--check") {
        say("check ok");
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let local = tokio::task::LocalSet::new();
    let code = local.block_on(&rt, async {
        if args.first().map(String::as_str) == Some("status") {
            return status(&cfg.socket).await;
        }
        match run(cfg).await {
            Ok(()) => 0,
            Err(e) => {
                say(&error_text(&e));
                1
            }
        }
    });
    std::process::exit(code);
}

async fn status(socket: &str) -> i32 {
    match teamsrelay_web::control::call(socket, "GET", "/accounts", None, Duration::from_secs(10)).await {
        Ok((s, body)) => {
            println!("{body}");
            if s == 200 { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("supervisor not reachable on {socket}: {e}");
            1
        }
    }
}

async fn run(cfg: Config) -> Result<(), String> {
    let log: Lines = Rc::new(|line: &str| println!("{line}"));
    let accounts = Accounts::new(cfg.accounts.clone(), Deps { log, process: None, focus: None, light_host: None });
    let d = &cfg.desktop;
    let idle = if d.on_demand { d.idle_ms } else { f64::INFINITY };
    let desktop = Desktop::new(Options::real(d.socket.display().to_string(), d.s6rc.clone(), d.microphone.clone(), idle, Rc::new(say)));
    let desktop_error = |e: String| say(&format!("desktop: {}", error_text(&e)));
    // a release that keeps the desktop up brings it up with the supervisor
    if !d.on_demand {
        let up = desktop.ensure();
        tokio::task::spawn_local(async move {
            if let Err(e) = up.await {
                desktop_error(e);
            }
        });
    }
    let (dt, a) = (desktop.clone(), accounts.clone());
    tokio::task::spawn_local(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = dt.tick(a.browser_running()).await {
                desktop_error(e);
            }
        }
    });
    let directory = cfg.accounts.shared_broker.clone().map(|broker| {
        let http = teamsrelay_web::agent::http::slot_http();
        let capture: Rc<dyn Capture> = Browsers::new(cfg.accounts.cdp_base_port, http.clone());
        Rc::new(Directory::new(broker, http, Some(cfg.accounts.data_dir.clone()), Some(capture)))
    });
    if let Some(b) = &cfg.accounts.shared_broker {
        let host = url::Url::parse(&b.url)
            .map(|u| format!("{}{}", u.host_str().unwrap_or(""), u.port().map(|p| format!(":{p}")).unwrap_or_default()))
            .unwrap_or_default();
        say(&format!("shared Microsoft accounts from {host}"));
    }
    let dt = desktop.clone();
    let control = Rc::new(Control { accounts: accounts.clone(), desktop: Rc::new(move || Box::pin(dt.ensure())), log: Rc::new(say), directory });
    if let Some(dir) = std::path::Path::new(&cfg.socket).parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let listener = listen(&cfg.socket).map_err(|e| e.to_string())?;
    say(&format!("listening socket={}", cfg.socket));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).map_err(|e| e.to_string())?;
    let stop = async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    };
    serve(listener, control, stop).await;
    accounts.stop_all().await;
    Ok(())
}
