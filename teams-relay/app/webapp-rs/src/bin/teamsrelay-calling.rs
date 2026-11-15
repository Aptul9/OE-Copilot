//! TeamsRelay calls of one account in Rust (in place of scripts/calling-live-host.mjs):
//!   teamsrelay-calling --mode=echo --discovery=FILE [--seconds=40] [--peer=8:orgid:...]   one call to Microsoft's
//!                                     test-call service, or to that person
//!   teamsrelay-calling --mode=serve   the calls of the account of HOST_STATE/discovery.json, on the IPC channel of
//!                                     the light host (NODE_CHANNEL_FD), until SIGTERM or 45 idle minutes
//!   teamsrelay-calling --check                                       the codecs load, then exits

use teamsrelay_web::calling;

fn arg(name: &str) -> Option<String> {
    std::env::args().find_map(|a| a.strip_prefix(&format!("--{name}=")).map(String::from))
}

fn main() {
    if std::env::args().any(|a| a == "--check") {
        let ok = calling::codec::OpusCodec::new().is_ok() && calling::codec::Codec::of("G722").is_some();
        println!("{}", serde_json::json!({ "calling": "check", "ok": ok }));
        std::process::exit(if ok { 0 } else { 1 });
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
    let code = rt.block_on(async {
        match arg("mode").as_deref() {
            // the IPC channel runs on this thread (spawn_local), the media on the others
            Some("serve") => tokio::task::LocalSet::new().run_until(calling::serve::serve()).await,
            Some("echo") => {
                let file = arg("discovery").unwrap_or_else(|| "/state/discovery.json".into());
                let discovery: serde_json::Value = match std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                    Some(d) => d,
                    None => {
                        eprintln!("no discovery at {file}");
                        return 2;
                    }
                };
                let seconds = arg("seconds").and_then(|s| s.parse().ok()).unwrap_or(40);
                let peer = arg("peer").filter(|p| regex::Regex::new(r"(?i)^8:orgid:[a-f0-9-]{36}$").expect("fixed").is_match(p));
                match calling::echo(&discovery, peer.as_deref(), seconds).await {
                    Ok(out) if out["melodyHeard"] == true => 0,
                    Ok(_) => 1,
                    Err(e) => {
                        println!("{}", serde_json::json!({ "failure": calling::Fail::to_string(&e) }));
                        1
                    }
                }
            }
            _ => {
                eprintln!("usage: teamsrelay-calling --mode=echo --discovery=FILE [--seconds=40] | --check");
                2
            }
        }
    });
    std::process::exit(code);
}
