//! Le pagine e i file dell'interfaccia: l'esportazione statica di web-ts (UI_STATICA, di norma /ui) con i file di public/. Le quattro pagine
//! (/, /login, /sicurezza, /disponibilita) sono quelle di app/*/page.tsx: il controllo della sessione e i valori che la pagina riceveva dal
//! server stanno qui, e arrivano alla pagina in `window.__OE__`, scritto nella testata prima di ogni script.

use crate::auth::rotte::sessione_corrente;
use crate::config::{auth2fa, Modo2fa};
use crate::safe_next::{login_url, safe_next};
use crate::web::Resp;
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use std::path::{Component, Path, PathBuf};

const NESSUNA_COPIA: &str = "private, no-cache, no-store, max-age=0, must-revalidate";

fn radice() -> PathBuf {
    PathBuf::from(crate::config::ui())
}

fn tipo(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=UTF-8",
        Some("css") => "text/css; charset=UTF-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

/// Il file dell'esportazione per un percorso, se c'è: niente .. né segmenti strani, niente pagine (.html) e niente dati delle pagine (.txt)
/// fuori da /_next/, che passano dalle regole delle pagine.
pub fn file_statico(percorso: &str) -> Option<PathBuf> {
    let decodificato = crate::auth::cookie::decodifica_componente(percorso)?;
    let relativo = decodificato.strip_prefix('/')?;
    if relativo.is_empty() || relativo.contains('\\') || relativo.contains('\0') {
        return None;
    }
    let p = Path::new(relativo);
    if !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let dentro_next = relativo.starts_with("_next/");
    let est = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !dentro_next && (est == "html" || est == "txt") {
        return None;
    }
    let file = radice().join(p);
    std::fs::metadata(&file).ok().filter(|m| m.is_file()).map(|_| file)
}

/// GET e HEAD di un file dell'esportazione: i file di /_next/static hanno il nome che cambia con il contenuto e si tengono un anno.
pub fn servi_file(file: &Path, percorso: &str, testa: bool) -> Resp {
    let Ok(dati) = std::fs::read(file) else { return Resp::vuota(404, &[]) };
    let cache = if percorso.starts_with("/_next/static/") { "public, max-age=31536000, immutable" } else { "public, max-age=0" };
    let lunghezza = dati.len().to_string();
    let mut r = Resp::nuova(200, &[("content-type", tipo(file)), ("content-length", &lunghezza), ("cache-control", cache)], if testa { Vec::new() } else { dati });
    if percorso == "/sw.js" {
        // il service worker deve poter controllare tutto il sito, e rileggersi a ogni visita
        r = r.con("service-worker-allowed", "/");
    }
    r
}

/// Il JSON dentro un <script>: niente che chiuda il tag o apra un commento HTML.
fn in_script(v: &Json) -> String {
    v.in_js().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// Una pagina dell'esportazione con i suoi valori dal server.
fn pagina(nome: &str, valori: &Json, stato: u16, testa: bool) -> Resp {
    let Ok(html) = std::fs::read_to_string(radice().join(nome)) else {
        eprintln!("pagine: {nome} non trovata in {}", radice().display());
        return Resp::nuova(500, &[("content-type", "text/plain; charset=utf-8"), ("cache-control", "no-store")], b"Interfaccia non installata\n".to_vec());
    };
    let script = format!("<script>window.__OE__={}</script>", in_script(valori));
    let html = match html.find("<head>") {
        Some(i) => format!("{}{script}{}", &html[..i + 6], &html[i + 6..]),
        None => format!("{script}{html}"),
    };
    let lunghezza = html.len().to_string();
    Resp::nuova(stato, &[("content-type", "text/html; charset=utf-8"), ("content-length", &lunghezza), ("cache-control", NESSUNA_COPIA)], if testa { Vec::new() } else { html.into_bytes() })
}

/// `redirect()` di Next in una pagina: 307 verso `dove`.
fn rimando(dove: &str) -> Resp {
    Resp::rimando(307, dove, &[("cache-control", NESSUNA_COPIA)])
}

pub fn non_trovata(testa: bool) -> Resp {
    pagina("404.html", &Json::obj(), 404, testa)
}

/// Le pagine di app/*/page.tsx. None: il percorso non è una pagina.
pub fn servi_pagina(percorso: &str, url: &url::Url, intestazioni: &[(String, String)], testa: bool) -> Option<Resp> {
    let sessione = || sessione_corrente(intestazioni, url).ok().flatten();
    Some(match percorso {
        "/" => match sessione() {
            None => rimando(&login_url("/")),
            Some(_) => {
                let mut valori = ogg! { "accesso2fa": auth2fa().nome() };
                // entrati da EverID: Esci chiude anche la sessione di EverID (lib/esci.ts)
                if let (Json::Obj(m), Some(_)) = (&mut valori, crate::everid::utente(intestazioni)) {
                    m.insert("esci", crate::everid::USCITA.to_string());
                }
                pagina("index.html", &valori, 200, testa)
            }
        },
        "/login" => {
            let next = url.query_pairs().find(|(k, _)| k == "next").map(|(_, v)| v.into_owned());
            let next = safe_next(next.as_deref());
            match sessione() {
                Some(_) => rimando(&next),
                None => pagina("login.html", &ogg! { "next": next, "modo": auth2fa().nome() }, 200, testa),
            }
        }
        "/sicurezza" => {
            let modo = auth2fa();
            if modo == Modo2fa::Off {
                return Some(non_trovata(testa));
            }
            match sessione() {
                None => rimando(&login_url("/sicurezza")),
                Some((_, u)) => pagina("sicurezza.html", &ogg! { "modo": modo.nome(), "email": u.email, "attivo": u.two_factor_enabled == Some(true) }, 200, testa),
            }
        }
        "/disponibilita" => {
            let valori: Vec<String> = url.query_pairs().filter(|(k, _)| k == "gruppo").map(|(_, v)| v.into_owned()).collect();
            // ?gruppo= ripetuto è una lista: come in Next, vale solo un testo
            let gruppo = match valori.as_slice() {
                [g] if crate::py::lunghezza_js(g) <= crate::disponibilita::GRUPPO_MAX && !g.chars().any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}')) => crate::py::js_trim(g).to_string(),
                _ => String::new(),
            };
            pagina("disponibilita.html", &ogg! { "gruppo": gruppo }, 200, testa)
        }
        "/nuova" => rimando("/"),
        _ => return None,
    })
}
