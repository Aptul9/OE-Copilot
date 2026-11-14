//! communication-mcp: the remote MCP server (http.rs). `communication-mcp --health` asks the running server for
//! /healthz, for the container healthcheck (the image has no shell and no curl).

use communication_mcp::communication::Communication;
use communication_mcp::gateway::Gateway;
use communication_mcp::http::{App, MIN_TOKEN, hosts_from_env, serve};
use communication_mcp::outlook::Outlook;
use communication_mcp::slack::Slack;
use communication_mcp::teams::Teams;
use communication_mcp::tokens::Tokens;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn fail(message: &str) -> ! {
    eprintln!("communication-mcp: {message}");
    std::process::exit(1)
}

fn port() -> u16 {
    match env("PORT") {
        None => 8190,
        Some(p) => p.parse::<u16>().ok().filter(|p| *p >= 1).unwrap_or_else(|| fail("PORT must be 1-65535")),
    }
}

async fn health() -> bool {
    let check = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port())).await.ok()?;
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
    let token = env("MCP_HTTP_TOKEN").unwrap_or_default();
    if token.len() < MIN_TOKEN {
        fail(&format!("MCP_HTTP_TOKEN must contain at least {MIN_TOKEN} characters"));
    }
    let hosts = hosts_from_env(&env("MCP_HTTP_HOSTS").unwrap_or_default());
    if hosts.is_empty() {
        fail("MCP_HTTP_HOSTS must list the accepted Host headers");
    }
    let broker = env("OUTLOOK_MCP_BROKER_URL").unwrap_or_else(|| fail("OUTLOOK_MCP_BROKER_URL is required"));
    let key = env("OUTLOOK_MCP_BROKER_KEY").unwrap_or_else(|| fail("OUTLOOK_MCP_BROKER_URL needs OUTLOOK_MCP_BROKER_KEY"));
    let tokens = Tokens::new(&broker, &key).unwrap_or_else(|e| fail(&e.to_string()));
    let google = Gateway::from_broker(&broker, &key).unwrap_or_else(|e| fail(&e.to_string()));
    let teams = match Teams::from_env(env("OUTLOOK_MCP_TEAMS_URL").as_deref(), env("OUTLOOK_MCP_TEAMS_TOKEN").as_deref()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("communication-mcp: Teams tools off, invalid configuration: {e}");
            None
        }
    };
    let slack = env("SLACK_WORKSPACES").map(|p| Slack::new(&p));
    let comm = Communication { outlook: Outlook::new(tokens), google: Some(google), teams, slack };
    let app = Arc::new(App { comm, token, hosts });
    let bind = env("BIND_HOST").unwrap_or_else(|| "127.0.0.1".into());
    let port = port();
    let listener = tokio::net::TcpListener::bind((bind.as_str(), port)).await.unwrap_or_else(|e| fail(&format!("cannot listen on {bind}:{port}: {e}")));
    eprintln!(
        "communication-mcp: remote MCP on {bind}:{port}/mcp, Teams {}, Slack {}",
        if app.comm.teams.is_some() { "on" } else { "off" },
        if app.comm.slack.is_some() { "on" } else { "off" }
    );
    let stop = async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    };
    serve(app, listener, stop).await;
}
