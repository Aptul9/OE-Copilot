//! Risposta per il `forward_auth` di Caddy davanti allo schermo del telefono (`/telefono/`), era lib/verifica-sessione.ts: 204 se la
//! richiesta porta una sessione valida, altrimenti la risposta che Caddy rimanda al browser com'è. Una pagina (navigazione, o Accept html) va
//! alla pagina di accesso e poi torna dove stava; il resto, WebSocket compreso, riceve 401 in JSON. Caddy inoltra i cookie e l'indirizzo
//! originale in `X-Forwarded-Uri`, che passa da `safe_next`.

use crate::auth::rotte::{intestazione, sessione_corrente};
use crate::safe_next::{login_url, safe_next};
use crate::web::Resp;
use oe_nucleo::ogg;

pub fn verifica_sessione(intestazioni: &[(String, String)], url: &url::Url) -> Resp {
    let nc = [("cache-control", "no-store")];
    if sessione_corrente(intestazioni, url).ok().flatten().is_some() {
        return Resp::vuota(204, &nc);
    }
    let aggiornamento = intestazione(intestazioni, "upgrade").is_some_and(|u| !u.is_empty());
    let accept = intestazione(intestazioni, "accept").unwrap_or_default().to_lowercase();
    let pagina = !aggiornamento && (intestazione(intestazioni, "sec-fetch-dest").as_deref() == Some("document") || accept.contains("text/html"));
    if pagina {
        let ritorno = safe_next(intestazione(intestazioni, "x-forwarded-uri").as_deref());
        return Resp::rimando(302, &login_url(&ritorno), &nc);
    }
    Resp::json(401, &ogg! { "error": "non autenticato" }, &nc)
}
