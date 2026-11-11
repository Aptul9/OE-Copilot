//! Il proprietario di .env (ADMIN_EMAIL, ADMIN_PASSWORD) esiste e accede con ADMIN_PASSWORD (era lib/owner.ts). Una password nuova in .env
//! chiude le sessioni aperte.

use crate::auth::righe;
use crate::auth::rotte::{hash_password, verifica_password};
use crate::config::PASSWORD_MIN;

/// Cosa è stato fatto, per il log; Err ferma il gateway.
pub async fn sync_owner(email: &str, password: &str) -> Result<String, String> {
    let utenti = righe::conta_utenti().map_err(|e| e.to_string())?;
    if email.is_empty() || password.is_empty() {
        // senza utenti nessuno può entrare
        if utenti == 0 {
            return Err("Nessun utente: servono ADMIN_EMAIL e ADMIN_PASSWORD in .env per creare il proprietario".to_string());
        }
        return Ok("nessun proprietario in .env".to_string());
    }
    if crate::py::lunghezza_js(password) < PASSWORD_MIN {
        return Err(format!("ADMIN_PASSWORD deve avere almeno {PASSWORD_MIN} caratteri"));
    }
    let ora = crate::orologio::ora_vera();
    let Some(u) = righe::utente_per_email(email).map_err(|e| e.to_string())? else {
        let u = righe::crea_utente(email, "Proprietario", ora).map_err(|e| e.to_string())?;
        let hash = hash_password(password.to_string()).await;
        righe::collega_account(&u.id, &hash, ora).map_err(|e| e.to_string())?;
        return Ok(format!("proprietario {email} creato"));
    };
    let credenziale = righe::password_di(&u.id).map_err(|e| e.to_string())?;
    if let Some(Some(h)) = &credenziale {
        if verifica_password(h.clone(), password.to_string()).await.unwrap_or(false) {
            return Ok(format!("proprietario {email}: in linea con .env"));
        }
    }
    let hash = hash_password(password.to_string()).await;
    let ora = crate::orologio::ora_vera();
    if credenziale.is_some() {
        righe::aggiorna_password(&u.id, &hash, ora).map_err(|e| e.to_string())?;
    } else {
        righe::collega_account(&u.id, &hash, ora).map_err(|e| e.to_string())?;
    }
    righe::elimina_sessioni_utente(&u.id).map_err(|e| e.to_string())?;
    Ok(format!("proprietario {email}: password presa da ADMIN_PASSWORD, sessioni chiuse"))
}
