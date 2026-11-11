//! Avvio del gateway: le variabili d'ambiente sono quelle del gateway TypeScript (lib/config.ts), più UI_STATICA (la cartella dell'interfaccia
//! esportata) e PORT (3000). Le strade sono in servizio.rs.

use oe_gateway::config;
use oe_gateway::servizio::{avvia_giri, prepara, Gateway};
use oe_http::servi;
use std::rc::Rc;
use tokio::net::TcpListener;
use tokio::signal::unix::{signal, SignalKind};
use tokio::task::LocalSet;

fn porta() -> u16 {
    config::var("PORT").and_then(|p| p.trim().parse().ok()).unwrap_or(3000)
}

/// `gateway --salute`: il controllo del compose. L'immagine non ha né node né curl; /salute risponde 200 se il gateway è su.
fn salute() -> i32 {
    use std::io::{Read, Write};
    let indirizzo = std::net::SocketAddr::from(([127, 0, 0, 1], porta()));
    let attesa = std::time::Duration::from_secs(5);
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&indirizzo, attesa) else { return 1 };
    let _ = s.set_read_timeout(Some(attesa));
    if s.write_all(b"GET /salute HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").is_err() {
        return 1;
    }
    let mut testa = [0u8; 12];
    if s.read_exact(&mut testa).is_ok() && &testa == b"HTTP/1.1 200" { 0 } else { 1 }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--salute") {
        std::process::exit(salute());
    }
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    std::process::exit(LocalSet::new().block_on(&runtime, avvia()));
}

async fn avvia() -> i32 {
    if let Err(e) = prepara().await {
        eprintln!("{e}");
        return 1;
    }
    avvia_giri();
    let p = porta();
    let ascolto = match TcpListener::bind(("0.0.0.0", p)).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("listen {e}");
            return 1;
        }
    };
    println!("gateway in ascolto sulla porta {p}");
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return 1 };
    tokio::select! {
        () = servi(ascolto, Rc::new(Gateway)) => 1,
        _ = term.recv() => 0,
        _ = int.recv() => 0,
    }
}
