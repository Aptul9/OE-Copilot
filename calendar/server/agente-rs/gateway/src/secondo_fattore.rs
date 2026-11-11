//! Le strade dei plugin che la pagina usa, per metodo (era lib/secondo-fattore.ts). Tutte le altre sotto /two-factor e /passkey non servono e
//! restano chiuse.

use crate::config::Modo2fa;

const TOTP: [&str; 5] = ["POST /two-factor/enable", "POST /two-factor/disable", "POST /two-factor/generate-backup-codes", "POST /two-factor/verify-totp", "POST /two-factor/verify-backup-code"];
const PASSKEY: [&str; 7] = [
    "GET /passkey/generate-register-options",
    "GET /passkey/generate-authenticate-options",
    "GET /passkey/list-user-passkeys",
    "POST /passkey/verify-registration",
    "POST /passkey/verify-authentication",
    "POST /passkey/delete-passkey",
    "POST /passkey/update-passkey",
];

fn normalizza(percorso: &str) -> String {
    let p = crate::auth::cookie::decodifica_componente(percorso).unwrap_or_else(|| percorso.to_string()).to_lowercase();
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out.trim_end_matches('/').to_string()
}

/// Se una richiesta a /api/auth/<percorso> può arrivare a Better Auth. Fuori da /two-factor e /passkey passa tutto. Dentro passa solo quello
/// che la modalità del secondo fattore (AUTH_2FA) usa: con off niente, con totp i codici, con passkey i codici e le passkey.
pub fn percorso_ammesso(modo: Modo2fa, metodo: &str, percorso_auth: &str) -> bool {
    let p = normalizza(percorso_auth);
    let dentro = ["/two-factor", "/passkey"].iter().any(|r| p == *r || p.starts_with(&format!("{r}/")));
    if !dentro {
        return true;
    }
    let voce = format!("{} {p}", metodo.to_uppercase());
    if modo == Modo2fa::Off {
        return false;
    }
    TOTP.contains(&voce.as_str()) || (modo == Modo2fa::Passkey && PASSKEY.contains(&voce.as_str()))
}
