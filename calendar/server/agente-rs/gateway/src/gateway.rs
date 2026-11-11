//! Il cancello davanti ai metodi dell'API e ai loghi (era lib/gateway.ts). Senza sessione una pagina rimanda a /login?next=... e l'API
//! risponde 401 JSON; con la sessione la richiesta passa, con le regole contro le richieste partite da un altro sito che aveva già il servizio
//! ui (server.py): JSON obbligatorio per i POST e Sec-Fetch-Site dello stesso sito. Quello che non è un metodo servito né un logo è 404.

use crate::auth::cookie::decodifica_componente;
use crate::auth::rotte::{intestazione, sessione_corrente};
use crate::safe_next::login_url;
use crate::web::Resp;
use oe_http::{CorpoLetto, Richiesta};
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use std::path::{Path, PathBuf};

/// Il corpo massimo di una richiesta POST: lo stesso che aveva il servizio Python ui (server.py, MAX_CORPO)
pub const MAX_BODY: usize = 1_000_000;

/// Un logo aperto da solo in una scheda è un documento di questo sito: un SVG potrebbe eseguire script con i suoi permessi. Con questa politica
/// (nessuna risorsa, nessuno script, origine isolata) non esegue e non carica niente; dentro un <img> non cambia nulla.
const POLITICA_LOGHI: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

fn json(stato: u16, corpo: &Json) -> Resp {
    Resp::json(stato, corpo, &[("cache-control", "no-store")])
}

/// La risposta di un metodo servito qui: JSON, senza copia in cache, senza indovinare il tipo.
fn risposta(stato: u16, corpo: &Json) -> Resp {
    Resp::nuova(stato, &[("content-type", "application/json; charset=utf-8"), ("cache-control", "no-store"), ("x-content-type-options", "nosniff")], corpo.in_js().into_bytes())
}

/// Il nome del metodo di POST /api/<metodo>, o None per qualunque altro percorso.
fn nome_metodo(percorso: &str) -> Option<&str> {
    let n = percorso.strip_prefix("/api/")?;
    let mut c = n.chars();
    let primo = c.next()?;
    ((primo.is_ascii_alphabetic() || primo == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')).then_some(n)
}

/// Il nome del file di /logos/<nome>, decodificato una volta sola come faceva il servizio ui, o None per qualunque altro percorso.
fn nome_logo(percorso: &str) -> Option<String> {
    decodifica_componente(percorso)?.strip_prefix("/logos/").map(str::to_string)
}

fn tipo_da_estensione(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/vnd.microsoft.icon",
        _ => "application/octet-stream",
    }
}

pub struct Logo {
    pub dati: Vec<u8>,
    pub tipo: &'static str,
}

/// Le cartelle dei loghi, in ordine: quella dei dati (logos/) e poi quella dell'immagine.
pub fn cartelle_loghi() -> Vec<PathBuf> {
    vec![crate::dati::nelle_dati(&["logos"]), PathBuf::from(crate::config::loghi_predefiniti())]
}

/// Il logo `nome` della prima cartella che lo ha. Si risolvono i percorsi veri, collegamenti compresi, e il file deve restare dentro la
/// cartella: un nome con .., un nome assoluto o un collegamento che esce non passano. Solo file normali.
pub fn leggi_logo(nome: &str, cartelle: &[PathBuf]) -> Option<Logo> {
    for cartella in cartelle {
        let Ok(radice) = std::fs::canonicalize(cartella) else { continue };
        // un nome assoluto prende il posto della cartella, come in os.path.join
        let candidato = if Path::new(nome).is_absolute() { PathBuf::from(nome) } else { PathBuf::from(format!("{}/{nome}", radice.display())) };
        let Ok(file) = std::fs::canonicalize(&candidato) else { continue };
        if file == radice || !file.starts_with(&radice) {
            continue;
        }
        if !std::fs::metadata(&file).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if let Ok(dati) = std::fs::read(&file) {
            return Some(Logo { tipo: tipo_da_estensione(&file), dati });
        }
    }
    None
}

/// Il messaggio di un errore come `(e instanceof Error && e.message) || e.constructor.name`.
fn testo_errore(e: &crate::py::Errore) -> String {
    let m = e.messaggio();
    if m.is_empty() { e.nome().to_string() } else { m.to_string() }
}

pub async fn handle(r: &mut Richiesta<'_>, url: &url::Url) -> Resp {
    let intestazioni = r.intestazioni.clone();
    let utente = sessione_corrente(&intestazioni, url).ok().flatten();
    if utente.is_none() {
        if url.path().starts_with("/api/") {
            return json(401, &ogg! { "error": "non autenticato" });
        }
        let mut dove = url.path().to_string();
        if let Some(q) = url.query().filter(|q| !q.is_empty()) {
            dove.push('?');
            dove.push_str(q);
        }
        return Resp::rimando(302, &login_url(&dove), &[("cache-control", "no-store")]);
    }
    if !matches!(r.metodo.as_str(), "GET" | "HEAD" | "POST") {
        return json(405, &ogg! { "error": "metodo non ammesso" });
    }

    if r.metodo == "POST" {
        let lunghezza = crate::py::numero_da_testo(r.intestazione("content-length").unwrap_or(""));
        if lunghezza > MAX_BODY as f64 {
            let mut x = json(413, &ogg! { "error": "richiesta troppo grande" });
            x.chiudi = true;
            return x;
        }
        let tipo = intestazione(&intestazioni, "content-type").unwrap_or_default();
        static JSON_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(&format!(r"(?i)^application/json{}*(;|$)", crate::caselle::SPAZI)).unwrap());
        if !JSON_RE.is_match(&tipo) {
            return json(415, &ogg! { "error": "serve application/json" });
        }
        let sito = intestazione(&intestazioni, "sec-fetch-site").filter(|s| !s.is_empty()).unwrap_or_else(|| "same-origin".to_string());
        if sito != "same-origin" && sito != "none" {
            return json(403, &ogg! { "error": "richiesta partita da un altro sito" });
        }
        let corpo = match r.leggi_corpo_fino(MAX_BODY).await {
            CorpoLetto::Letto(c) => c,
            CorpoLetto::Troppo => {
                let mut x = json(413, &ogg! { "error": "richiesta troppo grande" });
                x.chiudi = true;
                return x;
            }
            CorpoLetto::Interrotto => {
                let mut x = json(400, &ogg! { "error": "corpo non letto" });
                x.chiudi = true;
                return x;
            }
        };
        let Some(metodo) = nome_metodo(url.path()).filter(|m| crate::api::ha(m)) else { return json(404, &ogg! { "error": "metodo non disponibile" }) };
        // gli argomenti sono una lista JSON
        let testo = String::from_utf8_lossy(&corpo);
        let testo = if testo.is_empty() { "[]".into() } else { testo };
        let args = match Json::analizza(testo.strip_prefix('\u{feff}').unwrap_or(&testo)) {
            Ok(Json::Arr(a)) => a,
            Ok(_) => return risposta(400, &ogg! { "error": "gli argomenti vanno in una lista" }),
            Err(e) => return risposta(400, &ogg! { "error": e.messaggio }),
        };
        return match crate::api::chiama(metodo, args).await {
            Ok(v) => risposta(200, &v),
            Err(e) => {
                eprintln!("api.{metodo} {}: {}", e.nome(), e.messaggio());
                risposta(500, &ogg! { "error": testo_errore(&e) })
            }
        };
    }

    // La vecchia pagina del telefono virtuale (/telefono/) rimanda alla pagina principale
    if url.path() == "/telefono" || url.path().starts_with("/telefono/") {
        return Resp::rimando(302, "/", &[("cache-control", "no-store")]);
    }

    // GET e HEAD: solo i loghi
    let Some(logo) = nome_logo(url.path()).and_then(|n| leggi_logo(&n, &cartelle_loghi())) else { return json(404, &ogg! { "error": "non trovato" }) };
    let lunghezza = logo.dati.len().to_string();
    let corpo = if r.metodo == "HEAD" { Vec::new() } else { logo.dati };
    Resp::nuova(
        200,
        &[("content-type", logo.tipo), ("content-length", &lunghezza), ("cache-control", "no-cache"), ("x-content-type-options", "nosniff"), ("content-security-policy", POLITICA_LOGHI)],
        corpo,
    )
}
