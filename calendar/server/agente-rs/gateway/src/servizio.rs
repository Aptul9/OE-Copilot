//! Il servizio HTTP: le strade sono quelle di Next, nello stesso ordine: le rotte con il loro nome, le pagine, i file dell'interfaccia, e
//! tutto il resto al cancello dei metodi dell'API e dei loghi (gateway.rs). `prepara` fa i passi dell'avvio prima di aprire la porta.

use crate::auth::rotte::{self, Richiesta as RichiestaAuth};
use crate::config;
use crate::web::{frase, indirizzo, Resp};
use crate::{condividi, disponibilita, gateway, pagine, prenota, verifica_sessione};
use oe_http::{CorpoLetto, Esito, Gestore, Richiesta, Risposta};
use oe_nucleo::ogg;

fn json(stato: u16, corpo: &oe_nucleo::json::Json) -> Resp {
    Resp::json(stato, corpo, &[("cache-control", "no-store")])
}

fn non_ammesso(ammessi: &str) -> Resp {
    Resp::vuota(405, &[("allow", ammessi), ("cache-control", "no-store")])
}

async fn corpo(r: &mut Richiesta<'_>) -> Result<Vec<u8>, Resp> {
    match r.leggi_corpo_fino(gateway::MAX_BODY).await {
        CorpoLetto::Letto(c) => Ok(c),
        CorpoLetto::Troppo => {
            let mut x = json(413, &ogg! { "error": "richiesta troppo grande" });
            x.chiudi = true;
            Err(x)
        }
        CorpoLetto::Interrotto => {
            let mut x = Resp::vuota(400, &[]);
            x.chiudi = true;
            Err(x)
        }
    }
}

async fn instrada(r: &mut Richiesta<'_>) -> Resp {
    let Some(url) = indirizzo(&r.url) else { return Resp::vuota(400, &[]) };
    let percorso = url.path().to_string();
    let metodo = r.metodo.clone();
    let testa = metodo == "HEAD";
    // Next: un percorso con la barra in fondo rimanda a quello senza
    if percorso.len() > 1 && percorso.ends_with('/') {
        let mut dove = percorso.trim_end_matches('/').to_string();
        if dove.is_empty() {
            dove.push('/');
        }
        if let Some(q) = url.query() {
            dove.push('?');
            dove.push_str(q);
        }
        return Resp::rimando(308, &dove, &[]);
    }
    let intestazioni = r.intestazioni.clone();
    match percorso.as_str() {
        "/salute" => {
            if metodo != "GET" && !testa {
                return non_ammesso("GET, HEAD");
            }
            return Resp::nuova(200, &[("content-type", "text/plain"), ("cache-control", "no-store")], if testa { Vec::new() } else { b"ok".to_vec() });
        }
        "/api/disponibilita" => {
            if metodo != "GET" && !testa {
                return non_ammesso("GET, HEAD");
            }
            return disponibilita::get(&url).await;
        }
        "/api/prenota" => {
            if metodo != "POST" {
                return non_ammesso("POST");
            }
            let tipo = r.intestazione("content-type").unwrap_or("").to_string();
            // il limite vero è CORPO_MAX di prenota.rs: qui si legge al più il doppio, il resto non si aspetta
            return match r.leggi_corpo_fino(16 * 1024).await {
                CorpoLetto::Letto(c) => prenota::post(&tipo, Some(&c)).await,
                CorpoLetto::Troppo => {
                    let mut x = prenota::post(&tipo, None).await;
                    x.chiudi = true;
                    x
                }
                CorpoLetto::Interrotto => Resp::vuota(400, &[]),
            };
        }
        "/api/verifica-sessione" => {
            if metodo != "GET" && !testa {
                return non_ammesso("GET, HEAD");
            }
            return verifica_sessione::verifica_sessione(&intestazioni, &url);
        }
        "/mcp" => return crate::mcp::gestisci(r).await,
        _ => {}
    }
    if percorso == "/api/auth" || percorso.starts_with("/api/auth/") {
        if metodo != "GET" && metodo != "POST" {
            return non_ammesso("GET, POST");
        }
        let resto = percorso.strip_prefix("/api/auth").unwrap_or("");
        if !crate::secondo_fattore::percorso_ammesso(config::auth2fa(), &metodo, resto) {
            return json(404, &ogg! { "error": "secondo fattore non attivo" });
        }
        let c = if metodo == "POST" {
            match corpo(r).await {
                Ok(c) => c,
                Err(x) => return x,
            }
        } else {
            Vec::new()
        };
        let a = rotte::gestisci(&RichiestaAuth { metodo: &metodo, url: &url, intestazioni: &intestazioni, corpo: &c }, config::auth2fa()).await;
        let mut x = Resp { stato: a.stato, intestazioni: a.intestazioni, corpo: a.corpo.map(String::into_bytes).unwrap_or_default(), chiudi: false };
        if testa {
            x.corpo.clear();
        }
        return x;
    }
    if let Some(resto) = percorso.strip_prefix("/condividi/") {
        if !resto.is_empty() && !resto.contains('/') {
            return condividi::condividi(&metodo, &url, &intestazioni).await;
        }
    }
    if metodo == "GET" || testa {
        if let Some(p) = pagine::servi_pagina(&percorso, &url, &intestazioni, testa) {
            return p;
        }
        if let Some(f) = pagine::file_statico(&percorso) {
            return pagine::servi_file(&f, &percorso, testa);
        }
    }
    gateway::handle(r, &url).await
}

/// Il gestore delle richieste per oe_http::servi.
pub struct Gateway;

impl Gestore for Gateway {
    async fn gestisci(&self, r: &mut Richiesta<'_>) -> Esito {
        let testa = r.metodo == "HEAD";
        let x = instrada(r).await;
        let mut intestazioni = x.intestazioni;
        let senza_corpo = matches!(x.stato, 204 | 304) || (100..200).contains(&x.stato);
        if !senza_corpo && !intestazioni.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length")) {
            intestazioni.push(("content-length".to_string(), x.corpo.len().to_string()));
        }
        intestazioni.push(("connection".to_string(), if x.chiudi { "close" } else { "keep-alive" }.to_string()));
        Esito::Risposta(Risposta { codice: x.stato, motivo: frase(x.stato).to_string(), intestazioni, corpo: if testa { Vec::new() } else { x.corpo }, scarta_corpo: !x.chiudi })
    }
}

/// I passi dell'avvio: il secondo fattore e il segreto, le tabelle, lo stato dell'app, il proprietario. Un errore ferma il gateway.
pub async fn prepara() -> Result<(), String> {
    let modo = config::modo2fa()?;
    if config::auth_secret().len() < 32 {
        return Err("BETTER_AUTH_SECRET deve essere impostata, almeno 32 caratteri (openssl rand -hex 32)".to_string());
    }
    println!("secondo fattore dell'accesso (AUTH_2FA): {}", modo.nome());
    // prima le tabelle, con tutti i plugin in ogni modo: cambiare AUTH_2FA non tocca il database
    crate::auth::righe::migra()
        .and_then(|()| crate::lockout::migrate_lockout(&*crate::db::app_db()?))
        .map_err(|e| format!("database delle sessioni non pronto: {}", e.messaggio()))?;
    let stato = crate::archivio::archivio().and_then(|_| crate::condivisione::prepara_condivisioni()).and_then(|()| crate::loghi::migra_loghi());
    if let Err(e) = stato {
        eprintln!("archivio: stato dell'app non pronto {}: {}", e.nome(), e.messaggio());
    }
    println!("{}", crate::owner::sync_owner(&config::admin_email(), &config::admin_password()).await?);
    Ok(())
}

/// I giri in secondo piano: i promemoria e la disponibilità pubblica.
pub fn avvia_giri() {
    crate::promemoria::avvia_promemoria();
    // la pagina pubblica (meet): i blocchi delle prossime settimane si rileggono in secondo piano, chi la apre non aspetta i calendari
    disponibilita::avvia_disponibilita();
}
