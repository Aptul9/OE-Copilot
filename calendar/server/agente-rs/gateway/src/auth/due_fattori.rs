//! Il secondo fattore di Better Auth (plugin two-factor): il codice TOTP con i codici di recupero, attivato con la password e verificato una
//! volta, e la sfida dopo la password (cookie two_factor, 10 minuti, 5 tentativi per sfida e 5 errori di fila per account, poi 15 minuti di
//! blocco). I segreti stanno cifrati nella tabella twoFactor come li cifra Better Auth.

use super::cookie::{self, nome};
use super::cripto::{cifra, decifra, hmac_base64url, testo_casuale_alfanumerico, testo_casuale_auth, verifica_totp};
use super::righe::{self, iso_ms, ms_da, Sessione, Utente};
use super::rotte::{errore, errore_messaggio, valida, verifica_password, hash_password, Campo, Ctx, Tipo, Uscita};
use crate::config::PASSWORD_MAX;
use crate::lockout::{LOCKOUT_MINUTES, MAX_FAILED_LOGINS};
use crate::py::Esito;
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use rusqlite::{params, OptionalExtension};

/// "Fidati di questo dispositivo" è spento: il cookie nasce già scaduto (trustDeviceMaxAge 0).
const DISPOSITIVO_FIDATO_S: i64 = 0;
const SFIDA_S: i64 = 600;
const TENTATIVI_PER_SFIDA: i64 = 5;

struct Riga {
    id: String,
    secret: String,
    backup_codes: String,
    verified: Option<bool>,
    locked_until: Option<i64>,
}

fn riga(user_id: &str) -> Esito<Option<Riga>> {
    Ok(righe::db()?
        .query_row(r#"SELECT id, secret, backupCodes, verified, lockedUntil FROM "twoFactor" WHERE userId = ? LIMIT 1"#, [user_id], |r| {
            Ok(Riga {
                id: r.get(0)?,
                secret: r.get(1)?,
                backup_codes: r.get(2)?,
                verified: r.get::<_, Option<i64>>(3)?.map(|v| v != 0),
                locked_until: ms_da(&r.get::<_, rusqlite::types::Value>(4)?),
            })
        })
        .optional()?)
}

/// I codici di recupero: 10 di 10 lettere e cifre, scritti xxxxx-xxxxx.
fn nuovi_codici() -> Vec<String> {
    (0..10).map(|_| testo_casuale_alfanumerico(10)).map(|c| format!("{}-{}", &c[..5], &c[5..])).collect()
}

fn codici_cifrati(codici: &[String], segreto: &str) -> String {
    cifra(segreto, &Json::Arr(codici.iter().cloned().map(Json::Str).collect()).in_js())
}

/// validatePassword / checkPassword: la password dell'account "credential".
async fn password_giusta(ctx: &Ctx<'_>, user_id: &str, password: &str) -> Uscita<bool> {
    if crate::py::lunghezza_js(password) > PASSWORD_MAX {
        return Err(errore(400, "PASSWORD_TOO_LONG", "Password too long"));
    }
    let _ = ctx;
    match righe::password_di(user_id)? {
        Some(Some(hash)) => Ok(verifica_password(hash, password.to_string()).await.unwrap_or(false)),
        _ => {
            hash_password(password.to_string()).await;
            Ok(false)
        }
    }
}

fn password_errata() -> super::rotte::ErroreApi {
    errore(400, "INVALID_PASSWORD", "Invalid password")
}

fn uri_totp(segreto: &str, email: &str) -> String {
    let emittente = "OE Copilot";
    let parametri = form_urlencoded::Serializer::new(String::new())
        .append_pair("secret", &super::cripto::base32(segreto.as_bytes()))
        .append_pair("issuer", emittente)
        .append_pair("digits", "6")
        .append_pair("period", "30")
        .finish();
    format!("otpauth://totp/{}:{}?{parametri}", cookie::codifica_componente(emittente), cookie::codifica_componente(email))
}

pub async fn gestisci(ctx: &mut Ctx<'_>, p: &str) -> Uscita<(u16, Json)> {
    match p {
        "/two-factor/enable" => attiva(ctx).await,
        "/two-factor/disable" => disattiva(ctx).await,
        "/two-factor/generate-backup-codes" => nuovi_codici_di_recupero(ctx).await,
        "/two-factor/verify-totp" => verifica_codice(ctx, false).await,
        _ => verifica_codice(ctx, true).await,
    }
    .map(|c| (200, c))
}

async fn attiva(ctx: &mut Ctx<'_>) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("password", Tipo::Testo, true), Campo("issuer", Tipo::Testo, false)], false)?;
    let metodo = match corpo.get("method") {
        None => "totp".to_string(),
        Some(Json::Str(m)) if m == "otp" || m == "totp" => m.clone(),
        Some(_) => return Err(errore(400, "VALIDATION_ERROR", "[body.method] Invalid option: expected one of \"otp\"|\"totp\"")),
    };
    let (_, u) = ctx.richiedi_sessione()?;
    let password = corpo.get("password").and_then(Json::as_str).unwrap_or("");
    if password.is_empty() || !password_giusta(ctx, &u.id, password).await? {
        return Err(password_errata());
    }
    if metodo == "otp" {
        return Err(errore(400, "OTP_NOT_CONFIGURED", "OTP is not available"));
    }
    let esistente = riga(&u.id)?;
    if esistente.as_ref().is_some_and(|r| r.verified != Some(false)) {
        return Err(errore(400, "TOTP_ALREADY_ENABLED", "TOTP is already enabled"));
    }
    let codici = nuovi_codici();
    let segreto = testo_casuale_auth(32);
    let db = righe::db()?;
    let (cifrato, codici_c) = (cifra(&ctx.segreto, &segreto), codici_cifrati(&codici, &ctx.segreto));
    match esistente {
        Some(r) => {
            db.execute(r#"UPDATE "twoFactor" SET secret = ?, backupCodes = ?, verified = 0 WHERE id = ?"#, params![cifrato, codici_c, r.id]).map_err(crate::py::Errore::from)?;
        }
        None => {
            db.execute(
                r#"INSERT INTO "twoFactor" (id, secret, backupCodes, userId, verified, failedVerificationCount) VALUES (?, ?, ?, ?, 0, 0)"#,
                params![super::cripto::genera_id(), cifrato, codici_c, u.id],
            )
            .map_err(crate::py::Errore::from)?;
        }
    }
    let issuer = corpo.get("issuer").and_then(Json::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let uri = match issuer {
        Some(i) => uri_totp(&segreto, &u.email).replacen("OE%20Copilot", &cookie::codifica_componente(&i), 1).replacen("issuer=OE+Copilot", &form_urlencoded::Serializer::new(String::new()).append_pair("issuer", &i).finish(), 1),
        None => uri_totp(&segreto, &u.email),
    };
    Ok(ogg! { "method": "totp", "totpURI": uri, "backupCodes": Json::Arr(codici.into_iter().map(Json::Str).collect()) })
}

async fn disattiva(ctx: &mut Ctx<'_>) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("password", Tipo::Testo, true)], false)?;
    ctx.sessione = None;
    let (s, u) = ctx.richiedi_sessione()?;
    let password = corpo.get("password").and_then(Json::as_str).unwrap_or("");
    if password.is_empty() || !password_giusta(ctx, &u.id, password).await? {
        return Err(password_errata());
    }
    let ora = ctx.ora();
    let aggiornato = righe::imposta_due_fattori(&u.id, false, ora)?.ok_or_else(|| errore(500, "FAILED_TO_UPDATE_USER", "Failed to update user"))?;
    righe::db()?.execute(r#"DELETE FROM "twoFactor" WHERE userId = ?"#, [&aggiornato.id]).map_err(crate::py::Errore::from)?;
    let nuova = righe::crea_sessione(&aggiornato.id, false, s.ip_address.as_deref().unwrap_or(""), s.user_agent.as_deref().unwrap_or(""), ora)?;
    ctx.imposta_cookie_sessione(&nuova, &aggiornato, None);
    righe::elimina_sessione(&s.token)?;
    if let Some(v) = ctx.firmato(cookie::DISPOSITIVO_FIDATO) {
        if let Some(id) = v.split('!').nth(1).filter(|x| !x.is_empty()) {
            righe::elimina_verifica(id)?;
        }
        ctx.scadi(cookie::DISPOSITIVO_FIDATO);
    }
    Ok(ogg! { "status": true })
}

async fn nuovi_codici_di_recupero(ctx: &mut Ctx<'_>) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("password", Tipo::Testo, true)], false)?;
    let (_, u) = ctx.richiedi_sessione()?;
    if u.two_factor_enabled != Some(true) {
        return Err(errore(400, "TWO_FACTOR_NOT_ENABLED", "Two factor isn't enabled"));
    }
    let password = corpo.get("password").and_then(Json::as_str).unwrap_or("");
    if password.is_empty() || !password_giusta(ctx, &u.id, password).await? {
        return Err(password_errata());
    }
    let Some(r) = riga(&u.id)? else { return Err(errore(400, "TWO_FACTOR_NOT_ENABLED", "Two factor isn't enabled")) };
    let codici = nuovi_codici();
    righe::db()?.execute(r#"UPDATE "twoFactor" SET backupCodes = ? WHERE id = ?"#, params![codici_cifrati(&codici, &ctx.segreto), r.id]).map_err(crate::py::Errore::from)?;
    Ok(ogg! { "status": true, "backupCodes": Json::Arr(codici.into_iter().map(Json::Str).collect()) })
}

/// Chi sta verificando un codice: con una sessione (attivazione), o dopo la password (cookie two_factor).
enum Flusso {
    Sessione(Sessione, Utente),
    Accesso { sfida: String, utente: Utente, scadenza: i64, non_ricordare: bool },
}

fn cookie_non_valido() -> super::rotte::ErroreApi {
    errore(401, "INVALID_TWO_FACTOR_COOKIE", "Invalid two factor cookie")
}

fn flusso(ctx: &mut Ctx) -> Uscita<Flusso> {
    if let Some((s, u)) = ctx.sessione()? {
        return Ok(Flusso::Sessione(s, u));
    }
    let sfida = ctx.firmato(cookie::DUE_FATTORI).ok_or_else(cookie_non_valido)?;
    let v = righe::trova_verifica(&sfida, ctx.ora())?.ok_or_else(cookie_non_valido)?;
    let utente = righe::utente_per_id(&v.value)?.ok_or_else(cookie_non_valido)?;
    let non_ricordare = ctx.firmato(cookie::NON_RICORDARE).is_some();
    Ok(Flusso::Accesso { sfida, utente, scadenza: v.expires_at, non_ricordare })
}

/// beginAttempt(5): i tentativi della sfida; finiti, la sfida si chiude e si rientra dalla password.
fn inizia_tentativo(ctx: &mut Ctx, sfida: &str) -> Uscita<i64> {
    let id = format!("2fa-attempts-{sfida}");
    let preso = righe::consuma_verifica(&id, ctx.ora()).ok().flatten().ok_or_else(cookie_non_valido)?;
    let n = crate::py::numero_da_testo(&preso.value);
    let tentativi = if n.fract() == 0.0 && n >= 0.0 && n.is_finite() { n as i64 } else { TENTATIVI_PER_SFIDA };
    if tentativi >= TENTATIVI_PER_SFIDA {
        if righe::consuma_verifica(sfida, ctx.ora()).is_err() {
            ctx.scadi(cookie::DUE_FATTORI);
            return Err(errore(500, "FAILED_TO_INVALIDATE_TWO_FACTOR_CHALLENGE", "Failed to invalidate two-factor challenge"));
        }
        ctx.scadi(cookie::DUE_FATTORI);
        return Err(errore(400, "TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE", "Too many attempts. Please request a new code."));
    }
    Ok(tentativi)
}

fn riarma(ctx: &Ctx, sfida: &str, conto: i64, scadenza: i64) {
    let _ = righe::crea_verifica(&format!("2fa-attempts-{sfida}"), &conto.to_string(), scadenza, ctx.ora());
}

/// assertTwoFactorNotLocked: un account bloccato non verifica niente; un blocco finito si azzera.
fn non_bloccato(ctx: &Ctx, r: &Riga) -> Uscita<()> {
    let Some(fino) = r.locked_until else { return Ok(()) };
    if fino > ctx.ora() {
        return Err(errore(429, "ACCOUNT_TEMPORARILY_LOCKED", "Too many failed verification attempts. Your account is temporarily locked. Please try again later."));
    }
    righe::db()?
        .execute(r#"UPDATE "twoFactor" SET failedVerificationCount = 0, lockedUntil = NULL WHERE id = ? AND lockedUntil <= ?"#, params![r.id, iso_ms(ctx.ora())])
        .map_err(crate::py::Errore::from)?;
    Ok(())
}

/// recordTwoFactorFailure: un errore in più per l'account; al quinto di fila, 15 minuti di blocco.
fn conta_errore(ctx: &Ctx, r: &Riga) -> Esito<()> {
    let db = righe::db()?;
    db.execute(r#"UPDATE "twoFactor" SET failedVerificationCount = COALESCE(failedVerificationCount, 0) + 1 WHERE id = ?"#, [&r.id])?;
    let n: i64 = db.query_row(r#"SELECT COALESCE(failedVerificationCount, 0) FROM "twoFactor" WHERE id = ?"#, [&r.id], |x| x.get(0)).optional()?.unwrap_or(0);
    if n >= MAX_FAILED_LOGINS {
        db.execute(
            r#"UPDATE "twoFactor" SET lockedUntil = ? WHERE id = ? AND failedVerificationCount >= ?"#,
            params![iso_ms(ctx.ora() + LOCKOUT_MINUTES * 60 * 1000), r.id, MAX_FAILED_LOGINS],
        )?;
    }
    Ok(())
}

fn azzera_errori(r: &Riga) -> Esito<()> {
    righe::db()?.execute(r#"UPDATE "twoFactor" SET failedVerificationCount = 0, lockedUntil = NULL WHERE id = ?"#, [&r.id])?;
    Ok(())
}

/// valid(ctx): con la sessione, la risposta e basta; dopo la password, la sfida si consuma e nasce la sessione.
fn valido(ctx: &mut Ctx, f: &Flusso, fidato: bool) -> Uscita<Json> {
    match f {
        Flusso::Sessione(s, u) => Ok(ogg! { "token": s.token.clone(), "user": u.json() }),
        Flusso::Accesso { sfida, utente, non_ricordare, .. } => {
            let preso = righe::consuma_verifica(sfida, ctx.ora())?;
            let Some(preso) = preso.filter(|p| p.value == utente.id) else {
                ctx.scadi(cookie::DUE_FATTORI);
                return Err(cookie_non_valido());
            };
            let s = righe::crea_sessione(&preso.value, *non_ricordare, &ctx.ip(), &ctx.browser(), ctx.ora())?;
            ctx.imposta_cookie_sessione(&s, utente, None);
            ctx.scadi(cookie::DUE_FATTORI);
            if fidato {
                let id = format!("trust-device-{}", testo_casuale_auth(32));
                let token = hmac_base64url(&ctx.segreto, &format!("{}!{id}", utente.id));
                righe::crea_verifica(&id, &utente.id, ctx.ora() + DISPOSITIVO_FIDATO_S * 1000, ctx.ora())?;
                let riga = cookie::scrivi_firmato(&nome(cookie::DISPOSITIVO_FIDATO), &format!("{token}!{id}"), &ctx.segreto, Some(DISPOSITIVO_FIDATO_S));
                ctx.imposta(riga);
                ctx.scadi(cookie::NON_RICORDARE);
            }
            Ok(ogg! { "token": s.token, "user": utente.json() })
        }
    }
}

/// POST /two-factor/verify-totp e /two-factor/verify-backup-code.
async fn verifica_codice(ctx: &mut Ctx<'_>, recupero: bool) -> Uscita<Json> {
    let mut campi = vec![Campo("code", Tipo::Testo, true), Campo("trustDevice", Tipo::Booleano, false)];
    if recupero {
        campi.push(Campo("disableSession", Tipo::Booleano, false));
    }
    let corpo = valida(ctx.corpo.as_ref(), &campi, false)?;
    let codice = corpo.get("code").and_then(Json::as_str).unwrap_or("").to_string();
    let fidato = corpo.get("trustDevice") == Some(&Json::Bool(true));
    let f = flusso(ctx)?;
    let utente = match &f {
        Flusso::Sessione(_, u) | Flusso::Accesso { utente: u, .. } => u.clone(),
    };
    let accesso = matches!(f, Flusso::Accesso { .. });
    let Some(r) = riga(&utente.id)? else {
        return Err(if recupero { errore(400, "BACKUP_CODES_NOT_ENABLED", "Backup codes aren't enabled") } else { errore(400, "TOTP_NOT_ENABLED", "TOTP not enabled") });
    };
    if !recupero && accesso && r.verified == Some(false) {
        return Err(errore(400, "TOTP_NOT_ENABLED", "TOTP not enabled"));
    }
    if accesso {
        non_bloccato(ctx, &r)?;
    }
    let tentativo = match &f {
        Flusso::Accesso { sfida, scadenza, .. } => Some((sfida.clone(), *scadenza, inizia_tentativo(ctx, sfida)?)),
        Flusso::Sessione(..) => None,
    };
    let fallito = |ctx: &Ctx| -> Esito<()> {
        if let Some((sfida, scadenza, n)) = &tentativo {
            riarma(ctx, sfida, n + 1, *scadenza);
            conta_errore(ctx, &r)?;
        }
        Ok(())
    };
    let ripristina = |ctx: &Ctx| {
        if let Some((sfida, scadenza, n)) = &tentativo {
            riarma(ctx, sfida, *n, *scadenza);
        }
    };
    if recupero {
        let codici = match decifra(&ctx.segreto, &r.backup_codes) {
            Ok(t) => Json::analizza(&t).ok(),
            Err(e) => {
                ripristina(ctx);
                return Err(crate::py::Errore::altro(e).into());
            }
        };
        let codici: Option<Vec<Json>> = match codici {
            Some(Json::Arr(v)) => Some(v),
            _ => None,
        };
        let buono = codici.as_ref().is_some_and(|c| c.iter().any(|x| x.as_str() == Some(codice.as_str())));
        if !buono {
            fallito(ctx)?;
            return Err(errore(401, "INVALID_BACKUP_CODE", "Invalid backup code"));
        }
        let restano: Vec<Json> = codici.unwrap_or_default().into_iter().filter(|x| x.as_str() != Some(codice.as_str())).collect();
        let nuovi = cifra(&ctx.segreto, &Json::Arr(restano).in_js());
        let n = righe::db()?.execute(r#"UPDATE "twoFactor" SET backupCodes = ? WHERE id = ? AND backupCodes = ?"#, params![nuovi, r.id, r.backup_codes]).map_err(crate::py::Errore::from)?;
        if n == 0 {
            return Err(errore_messaggio(409, "Failed to verify backup code. Please try again."));
        }
        if accesso {
            azzera_errori(&r)?;
        }
        if corpo.get("disableSession") != Some(&Json::Bool(true)) {
            return valido(ctx, &f, fidato);
        }
        let mut m = Mappa::new();
        if let Flusso::Sessione(s, _) = &f {
            m.insert("token", s.token.clone());
        }
        m.insert("user", utente.json());
        return Ok(Json::Obj(m));
    }
    let segreto = match decifra(&ctx.segreto, &r.secret) {
        Ok(s) => s,
        Err(e) => {
            ripristina(ctx);
            return Err(crate::py::Errore::altro(e).into());
        }
    };
    if !verifica_totp(&codice, &segreto, ctx.ora()) {
        fallito(ctx)?;
        return Err(errore(401, "INVALID_CODE", "Invalid code"));
    }
    if accesso {
        azzera_errori(&r)?;
    }
    if r.verified != Some(true) {
        if utente.two_factor_enabled != Some(true) {
            if let Flusso::Sessione(attiva, _) = &f {
                let aggiornato = righe::imposta_due_fattori(&utente.id, true, ctx.ora())?.ok_or_else(|| errore(500, "FAILED_TO_UPDATE_USER", "Failed to update user"))?;
                let nuova = righe::crea_sessione(&utente.id, false, attiva.ip_address.as_deref().unwrap_or(""), attiva.user_agent.as_deref().unwrap_or(""), ctx.ora())?;
                ctx.imposta_cookie_sessione(&nuova, &aggiornato, None);
                righe::elimina_sessione(&attiva.token)?;
            }
        }
        righe::db()?.execute(r#"UPDATE "twoFactor" SET verified = 1 WHERE id = ?"#, [&r.id]).map_err(crate::py::Errore::from)?;
    }
    valido(ctx, &f, fidato)
}

/// Il gancio dopo l'accesso con la password: con il secondo fattore attivo la sessione appena nata si toglie e al suo posto c'è la sfida.
pub fn dopo_accesso(ctx: &mut Ctx) -> Uscita<Option<Json>> {
    let Some((s, u)) = ctx.nuova.clone() else { return Ok(None) };
    if u.two_factor_enabled != Some(true) {
        return Ok(None);
    }
    if let Some(v) = ctx.firmato(cookie::DISPOSITIVO_FIDATO) {
        let mut pezzi = v.splitn(2, '!');
        if let (Some(token), Some(id)) = (pezzi.next(), pezzi.next()) {
            let atteso = hmac_base64url(&ctx.segreto, &format!("{}!{id}", u.id));
            if !token.is_empty() && !id.is_empty() && token == atteso {
                if let Some(r) = righe::trova_verifica(id, ctx.ora())?.filter(|r| r.value == u.id && r.expires_at > ctx.ora()) {
                    let _ = r;
                    righe::elimina_verifica(id)?;
                    let nuovo = format!("trust-device-{}", testo_casuale_auth(32));
                    let token = hmac_base64url(&ctx.segreto, &format!("{}!{nuovo}", u.id));
                    righe::crea_verifica(&nuovo, &u.id, ctx.ora() + DISPOSITIVO_FIDATO_S * 1000, ctx.ora())?;
                    let riga = cookie::scrivi_firmato(&nome(cookie::DISPOSITIVO_FIDATO), &format!("{token}!{nuovo}"), &ctx.segreto, Some(DISPOSITIVO_FIDATO_S));
                    ctx.imposta(riga);
                    return Ok(None);
                }
            }
        }
        ctx.scadi(cookie::DISPOSITIVO_FIDATO);
    }
    ctx.elimina_cookie_sessione(true);
    righe::elimina_sessione(&s.token)?;
    ctx.nuova = None;
    let sfida = format!("2fa-{}", testo_casuale_auth(20));
    let scadenza = ctx.ora() + SFIDA_S * 1000;
    righe::crea_verifica(&sfida, &u.id, scadenza, ctx.ora())?;
    righe::crea_verifica(&format!("2fa-attempts-{sfida}"), "0", scadenza, ctx.ora())?;
    let riga_cookie = cookie::scrivi_firmato(&nome(cookie::DUE_FATTORI), &sfida, &ctx.segreto, Some(SFIDA_S));
    ctx.imposta(riga_cookie);
    let metodi = if riga(&u.id)?.is_some_and(|r| r.verified != Some(false)) { vec![Json::Str("totp".to_string())] } else { vec![] };
    Ok(Some(ogg! { "twoFactorRedirect": true, "twoFactorMethods": Json::Arr(metodi) }))
}
