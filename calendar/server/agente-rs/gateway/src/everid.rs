//! EverID (Authentik) davanti all'interfaccia: Caddy chiede a EverID con forward_auth e passa al gateway l'email in X-Authentik-Email.
//! Il gateway la prende per buona solo con EVERID=on e solo da Caddy (EVERID_PROXY, di norma il servizio caddy-ui di compose): da
//! chiunque altro le intestazioni X-Authentik-* si tolgono prima di ogni strada. L'email deve essere un utente del gateway (il
//! proprietario di ADMIN_EMAIL): EverID decide chi entra (gruppo app-calendar), il gateway resta il padrone dei suoi utenti.

use crate::auth::righe::{self, Sessione, Utente};
use crate::auth::rotte::intestazione;
use crate::config;
use oe_http::Richiesta;
use std::net::IpAddr;

/// L'uscita da EverID, servita dall'outpost sullo stesso nome del sito (Caddyfile).
pub const USCITA: &str = "/outpost.goauthentik.io/sign_out";
const PREFISSO: &str = "x-authentik-";

/// EVERID=on: senza valore, o con un altro valore, è spento.
pub fn attivo() -> bool {
    config::var("EVERID").is_some_and(|v| crate::py::js_trim(&v).eq_ignore_ascii_case("on"))
}

/// Il nome (o l'indirizzo) di Caddy sulla rete di compose.
fn proxy() -> String {
    config::var("EVERID_PROXY").map(|v| crate::py::js_trim(&v).to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| "caddy-ui".to_string())
}

/// La richiesta arriva dal container di Caddy: l'indirizzo di chi ha aperto la connessione è uno di quelli del suo nome.
async fn da_caddy(remoto: &str) -> bool {
    let Ok(ip) = remoto.parse::<IpAddr>().map(|i| i.to_canonical()) else { return false };
    let nome = proxy();
    if let Ok(p) = nome.parse::<IpAddr>() {
        return p.to_canonical() == ip;
    }
    match tokio::net::lookup_host((nome.as_str(), 0)).await {
        Ok(mut indirizzi) => indirizzi.any(|a| a.ip().to_canonical() == ip),
        Err(_) => false,
    }
}

/// Prima di ogni strada: le X-Authentik-* restano solo se EverID è attivo e la richiesta viene da Caddy.
pub async fn filtra(r: &mut Richiesta<'_>) {
    if !r.intestazioni.iter().any(|(k, _)| k.starts_with(PREFISSO)) {
        return;
    }
    if attivo() && da_caddy(r.remoto).await {
        return;
    }
    r.intestazioni.retain(|(k, _)| !k.starts_with(PREFISSO));
}

/// L'utente che EverID ha fatto entrare, se è un utente del gateway. Dopo `filtra` l'intestazione c'è solo se viene da Caddy.
pub fn utente(intestazioni: &[(String, String)]) -> Option<Utente> {
    if !attivo() {
        return None;
    }
    let email = intestazione(intestazioni, "x-authentik-email")?;
    let email = crate::py::js_trim(&email).to_lowercase();
    if email.is_empty() {
        return None;
    }
    righe::utente_per_email(&email).ok().flatten()
}

/// La sessione di una richiesta passata da EverID: dura quanto la richiesta, nel database non c'è.
pub fn sessione(u: &Utente) -> Sessione {
    let ora = crate::orologio::ora_vera();
    Sessione { id: "everid".to_string(), expires_at: ora + 60_000, token: String::new(), created_at: Some(ora), updated_at: Some(ora), ip_address: None, user_agent: None, user_id: u.id.clone() }
}

/// ACCESSO_PASSWORD=off chiude l'accesso con la password (POST /api/auth/sign-in/email): resta solo EverID. Predefinito on.
pub fn accesso_con_password() -> bool {
    !config::var("ACCESSO_PASSWORD").is_some_and(|v| crate::py::js_trim(&v).eq_ignore_ascii_case("off"))
}
