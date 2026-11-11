//! /mcp: il server MCP della posta, JSON-RPC su HTTP (era app/mcp/route.ts). Entra solo chi ha MCP_TOKEN (Bearer, confronto in tempo
//! costante) e mai una pagina web (una richiesta con Origin si rifiuta); gli strumenti sono quelli di posta.rs.

pub mod posta;

use crate::auth::rotte::intestazione;
use crate::web::Resp;
use oe_http::{CorpoLetto, Richiesta};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use regex::Regex;
use std::sync::LazyLock;

const PROTOCOL_VERSION: &str = "2025-06-18";
/// Il corpo più grande che si legge: un messaggio con gli allegati in base64. Si legge solo dopo il controllo del token.
const CORPO_MAX: usize = 32 * 1024 * 1024;
const NC: [(&str, &str); 1] = [("cache-control", "no-store")];

fn token_valido(autorizzazione: Option<&str>) -> bool {
    static BEARER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Bearer\s+(\S+)\s*$").expect("fissa"));
    let segreto = crate::config::mcp_token();
    let (false, Some(a)) = (segreto.is_empty(), autorizzazione) else { return false };
    let Some(c) = BEARER.captures(a) else { return false };
    let (x, y) = (c[1].as_bytes(), segreto.as_bytes());
    x.len() == y.len() && x.iter().zip(y).fold(0u8, |d, (p, q)| d | (p ^ q)) == 0
}

fn non_autorizzato() -> Resp {
    Resp::json(
        401,
        &ogg! { "jsonrpc": "2.0", "error": ogg! { "code": -32001, "message": "Missing or invalid Bearer token" }, "id": Json::Null },
        &[("www-authenticate", "Bearer realm=\"oe-copilot\""), ("cache-control", "no-store")],
    )
}

fn errore_rpc(id: Json, codice: i32, messaggio: String) -> Json {
    ogg! { "jsonrpc": "2.0", "id": id, "error": ogg! { "code": codice, "message": messaggio } }
}

/// Un messaggio JSON-RPC; None per una notifica (senza id).
pub async fn gestisci_messaggio(req: &Json) -> Option<Json> {
    let id = req.get("id").filter(|i| !i.is_null())?.clone();
    let metodo = req.get("method");
    let vuoti = Json::obj();
    let params = req.get("params").filter(|p| matches!(p, Json::Obj(_) | Json::Arr(_))).unwrap_or(&vuoti);
    Some(match metodo.and_then(Json::as_str) {
        Some("initialize") => {
            let versione = params.get("protocolVersion").and_then(Json::as_str).filter(|v| !v.is_empty()).unwrap_or(PROTOCOL_VERSION);
            let istruzioni = "OE Copilot Mail MCP: read and send emails across all Outlook accounts signed in on the OE Copilot virtual phone. \
Supports reading full conversation threads, searching, downloading/expanding attachments, and sending/replying/forwarding \
with rich HTML formatting, selective recipients (to, cc, replace_cc, exclude_recipients), and attachments.";
            ogg! {
                "jsonrpc": "2.0",
                "id": id,
                "result": ogg! {
                    "protocolVersion": versione,
                    "capabilities": ogg! { "tools": Json::obj() },
                    "serverInfo": ogg! { "name": "oe-copilot-mail", "version": "1.0.0" },
                    "instructions": istruzioni,
                },
            }
        }
        Some("ping") => ogg! { "jsonrpc": "2.0", "id": id, "result": Json::obj() },
        Some("tools/list") => ogg! { "jsonrpc": "2.0", "id": id, "result": ogg! { "tools": posta::STRUMENTI.clone() } },
        Some("tools/call") => {
            let nome = params.get("name").and_then(Json::as_str).unwrap_or("").to_string();
            if !crate::py::lista(Some(&posta::STRUMENTI)).iter().any(|t| t.get("name").and_then(Json::as_str) == Some(nome.as_str())) {
                return Some(errore_rpc(id, -32602, format!("Unknown tool: {nome}")));
            }
            let args = match params.get("arguments") {
                Some(Json::Obj(m)) => m.clone(),
                _ => Mappa::new(),
            };
            match posta::esegui(&nome, &args).await {
                Ok(content) => ogg! { "jsonrpc": "2.0", "id": id, "result": ogg! { "content": Json::Arr(content) } },
                Err(e) => {
                    let testo = format!("Error: {}: {}", e.nome(), e.messaggio());
                    ogg! { "jsonrpc": "2.0", "id": id, "result": ogg! { "content": Json::Arr(vec![ogg! { "type": "text", "text": testo }]), "isError": true } }
                }
            }
        }
        _ => {
            let m = match metodo {
                None | Some(Json::Null) => String::new(),
                x => crate::py::string_js(x),
            };
            errore_rpc(id, -32601, format!("Method not found: {m}"))
        }
    })
}

/// Il corpo di un POST già autorizzato: un messaggio o una lista di messaggi.
pub async fn post(corpo: &[u8]) -> Resp {
    let testo = String::from_utf8_lossy(corpo);
    let Ok(payload) = Json::analizza(testo.strip_prefix('\u{feff}').unwrap_or(&testo)) else {
        return Resp::json(400, &ogg! { "jsonrpc": "2.0", "error": ogg! { "code": -32700, "message": "Parse error" }, "id": Json::Null }, &NC);
    };
    if let Json::Arr(voci) = &payload {
        let mut risposte = Vec::new();
        for v in voci.iter().filter(|v| matches!(v, Json::Obj(_) | Json::Arr(_))) {
            if let Some(r) = gestisci_messaggio(v).await {
                risposte.push(r);
            }
        }
        if risposte.is_empty() {
            return Resp::vuota(202, &NC);
        }
        return Resp::json(200, &Json::Arr(risposte), &NC);
    }
    if !matches!(payload, Json::Obj(_)) {
        return Resp::json(400, &ogg! { "jsonrpc": "2.0", "error": ogg! { "code": -32600, "message": "Invalid Request" }, "id": Json::Null }, &NC);
    }
    match gestisci_messaggio(&payload).await {
        Some(r) => Resp::json(200, &r, &NC),
        None => Resp::vuota(202, &NC),
    }
}

pub async fn gestisci(r: &mut Richiesta<'_>) -> Resp {
    let autorizzazione = intestazione(&r.intestazioni, "authorization");
    match r.metodo.as_str() {
        "POST" => {
            if r.intestazioni.iter().any(|(k, _)| k == "origin") {
                return Resp::json(403, &ogg! { "error": "Richieste da pagine web non ammesse" }, &NC);
            }
            if !token_valido(autorizzazione.as_deref()) {
                return non_autorizzato();
            }
            match r.leggi_corpo_fino(CORPO_MAX).await {
                CorpoLetto::Letto(c) => post(&c).await,
                CorpoLetto::Troppo => {
                    let mut x = Resp::json(413, &ogg! { "error": "richiesta troppo grande" }, &NC);
                    x.chiudi = true;
                    x
                }
                CorpoLetto::Interrotto => {
                    let mut x = Resp::vuota(400, &NC);
                    x.chiudi = true;
                    x
                }
            }
        }
        "GET" | "HEAD" | "DELETE" => {
            if !token_valido(autorizzazione.as_deref()) {
                return non_autorizzato();
            }
            if r.metodo == "DELETE" {
                Resp::vuota(204, &NC)
            } else {
                Resp::vuota(405, &[("allow", "POST, DELETE"), ("cache-control", "no-store")])
            }
        }
        _ => Resp::vuota(405, &[("allow", "DELETE, GET, HEAD, POST"), ("cache-control", "no-store")]),
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    fn corpo(r: &Resp) -> Json {
        Json::analizza(&String::from_utf8_lossy(&r.corpo)).unwrap()
    }

    #[test]
    fn il_token() {
        crate::config::imposta_per_prova("MCP_TOKEN", Some("segreto-lungo"));
        assert!(token_valido(Some("Bearer segreto-lungo")));
        assert!(token_valido(Some("bearer   segreto-lungo  ")));
        assert!(!token_valido(Some("Bearer segreto-lung")));
        assert!(!token_valido(Some("Basic segreto-lungo")));
        assert!(!token_valido(None));
        crate::config::imposta_per_prova("MCP_TOKEN", Some(""));
        crate::config::imposta_per_prova("AGENTE_CHIAVE", Some(""));
        assert!(!token_valido(Some("Bearer ")));
        crate::config::azzera_prova();
        assert_eq!(non_autorizzato().stato, 401);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn il_dispatcher() {
        let r = post(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#).await;
        let j = corpo(&r);
        assert_eq!((r.stato, j["result"]["protocolVersion"].as_str()), (200, Some("2024-11-05")));
        assert_eq!(j["result"]["serverInfo"].in_js(), r#"{"name":"oe-copilot-mail","version":"1.0.0"}"#);
        assert_eq!(corpo(&post(br#"{"id":"a","method":"ping"}"#).await).in_js(), r#"{"jsonrpc":"2.0","id":"a","result":{}}"#);
        let l = corpo(&post(br#"{"id":2,"method":"tools/list"}"#).await);
        assert_eq!(l["result"]["tools"].in_js(), include_str!("strumenti.json"));
        assert_eq!(corpo(&post(br#"{"id":3,"method":"boh"}"#).await)["error"].in_js(), r#"{"code":-32601,"message":"Method not found: boh"}"#);
        assert_eq!(corpo(&post(br#"{"id":4,"method":"tools/call","params":{"name":"x"}}"#).await)["error"]["message"].as_str(), Some("Unknown tool: x"));
        assert_eq!(post(br#"{"method":"notifications/initialized"}"#).await.stato, 202);
        let b = post(br#"[{"id":1,"method":"ping"},{"method":"n"},5,{"id":2,"method":"ping"}]"#).await;
        assert_eq!(corpo(&b).in_js(), r#"[{"jsonrpc":"2.0","id":1,"result":{}},{"jsonrpc":"2.0","id":2,"result":{}}]"#);
        assert_eq!(post(b"[]").await.stato, 202);
        let e = post(b"{non json").await;
        assert_eq!((e.stato, corpo(&e)["error"]["code"].as_f64()), (400, Some(-32700.0)));
        assert_eq!(corpo(&post(b"5").await)["error"]["message"].as_str(), Some("Invalid Request"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn uno_strumento_che_fallisce_risponde_con_l_errore() {
        posta::azzera();
        crate::config::imposta_per_prova("AGENTE_URL", Some("http://127.0.0.1:1"));
        let r = corpo(&post(br#"{"id":5,"method":"tools/call","params":{"name":"list_accounts"}}"#).await);
        crate::config::azzera_prova();
        assert_eq!(r["result"].in_js(), r#"{"content":[{"type":"text","text":"Error: TypeError: fetch failed"}],"isError":true}"#);
    }
}
