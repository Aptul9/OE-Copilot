//! Le passkey di Better Auth (plugin @better-auth/passkey 1.7): le opzioni per registrare e per accedere (sfida in un cookie firmato e in
//! una riga di verification, 5 minuti), la verifica delle due cerimonie (webauthn.rs), l'elenco, il nome e la cancellazione. Le righe della
//! tabella passkey sono quelle che scrive il plugin: una passkey registrata dal gateway TypeScript vale qui, e viceversa.

use super::cookie::{self, nome};
use super::cripto::{genera_id, testo_casuale_auth, testo_casuale_minuscolo};
use super::righe::{self, iso_ms, ms_da};
use super::rotte::{errore, valida, Campo, Ctx, ErroreApi, Tipo, Uscita};
use super::webauthn::{self, senza_riempimento, sfida};
use crate::b64::{codifica, codifica_url, decodifica_node};
use crate::py::{string_js, Errore, Esito};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use rusqlite::{params, OptionalExtension, Row};

const RP_NOME: &str = "OE Copilot";
const SFIDA_S: i64 = 300;
const ATTESA_MS: i64 = 60_000;

/// L'identità del sito per il browser: il nome dell'host e l'origine di APP_URL, mai quelli della richiesta.
fn rp_id() -> String {
    url::Url::parse(&crate::config::app_url()).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| "localhost".to_string())
}

fn origine() -> String {
    url::Url::parse(&crate::config::app_url()).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
}

fn codice(stato: u16, codice: &str) -> ErroreApi {
    let messaggio = match codice {
        "CHALLENGE_NOT_FOUND" => "Challenge not found",
        "YOU_ARE_NOT_ALLOWED_TO_REGISTER_THIS_PASSKEY" => "You are not allowed to register this passkey",
        "FAILED_TO_VERIFY_REGISTRATION" => "Failed to verify registration",
        "PASSKEY_NOT_FOUND" => "Passkey not found",
        "AUTHENTICATION_FAILED" => "Authentication failed",
        "UNABLE_TO_CREATE_SESSION" => "Unable to create session",
        "USER_NOT_FOUND" => "User not found",
        "FAILED_TO_UPDATE_PASSKEY" => "Failed to update passkey",
        _ => "Unknown error",
    };
    errore(stato, codice, messaggio)
}

#[derive(Clone, Debug)]
struct Riga {
    id: String,
    name: Option<String>,
    public_key: String,
    user_id: String,
    credential_id: String,
    counter: i64,
    device_type: String,
    backed_up: bool,
    transports: Option<String>,
    created_at: Option<i64>,
    aaguid: Option<String>,
}

impl Riga {
    fn da(r: &Row) -> rusqlite::Result<Self> {
        Ok(Riga {
            id: r.get("id")?,
            name: r.get("name")?,
            public_key: r.get("publicKey")?,
            user_id: r.get("userId")?,
            credential_id: r.get("credentialID")?,
            counter: r.get("counter")?,
            device_type: r.get("deviceType")?,
            backed_up: r.get::<_, Option<i64>>("backedUp")?.is_some_and(|v| v != 0),
            transports: r.get("transports")?,
            created_at: ms_da(&r.get::<_, rusqlite::types::Value>("createdAt")?),
            aaguid: r.get("aaguid")?,
        })
    }

    fn json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("name", self.name.clone());
        m.insert("publicKey", self.public_key.clone());
        m.insert("userId", self.user_id.clone());
        m.insert("credentialID", self.credential_id.clone());
        m.insert("counter", self.counter as f64);
        m.insert("deviceType", self.device_type.clone());
        m.insert("backedUp", self.backed_up);
        m.insert("transports", self.transports.clone());
        m.insert("createdAt", self.created_at.map(iso_ms));
        m.insert("aaguid", self.aaguid.clone());
        m.insert("id", self.id.clone());
        Json::Obj(m)
    }

    /// { id, transports?, type } per excludeCredentials e allowCredentials: `transports?.split(",")`, l'id senza \"=\".
    fn credenziale(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("id", senza_riempimento(&self.credential_id));
        if let Some(t) = &self.transports {
            m.insert("transports", Json::Arr(t.split(',').map(|x| Json::Str(x.to_string())).collect()));
        }
        m.insert("type", "public-key");
        Json::Obj(m)
    }
}

fn dell_utente(user_id: &str) -> Esito<Vec<Riga>> {
    let db = righe::db()?;
    let mut st = db.prepare_cached(r#"SELECT * FROM "passkey" WHERE userId = ? ORDER BY rowid"#)?;
    let v = st.query_map([user_id], Riga::da)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn per_campo(campo: &str, valore: &str) -> Esito<Option<Riga>> {
    let sql = format!(r#"SELECT * FROM "passkey" WHERE "{campo}" = ? LIMIT 1"#);
    Ok(righe::db()?.query_row(&sql, [valore], Riga::da).optional()?)
}

pub fn gestisci(ctx: &mut Ctx<'_>, p: &str) -> Uscita<(u16, Json)> {
    match p {
        "/passkey/generate-register-options" => opzioni_registrazione(ctx),
        "/passkey/generate-authenticate-options" => opzioni_accesso(ctx),
        "/passkey/verify-registration" => verifica_registrazione(ctx),
        "/passkey/verify-authentication" => verifica_accesso(ctx),
        "/passkey/list-user-passkeys" => elenco(ctx),
        "/passkey/delete-passkey" => elimina(ctx),
        _ => aggiorna(ctx),
    }
    .map(|c| (200, c))
}

/// La sfida di una cerimonia: un testo casuale nel cookie firmato better-auth-passkey e, con quel nome, la riga di verification.
fn salva_sfida(ctx: &mut Ctx, valore: &Json) -> Uscita<()> {
    let token = testo_casuale_auth(32);
    let riga = cookie::scrivi_firmato(&nome(cookie::SFIDA_PASSKEY), &token, &ctx.segreto, Some(SFIDA_S));
    ctx.imposta(riga);
    righe::crea_verifica(&token, &valore.in_js(), ctx.ora() + SFIDA_S * 1000, ctx.ora())?;
    Ok(())
}

/// La sfida della cerimonia `tipo`, presa una volta sola: il valore salvato, o CHALLENGE_NOT_FOUND.
fn prendi_sfida(ctx: &mut Ctx, tipo: &str) -> Uscita<Json> {
    let token = ctx.firmato(cookie::SFIDA_PASSKEY).ok_or_else(|| codice(400, "CHALLENGE_NOT_FOUND"))?;
    let v = righe::consuma_verifica(&token, ctx.ora())?.ok_or_else(|| codice(400, "CHALLENGE_NOT_FOUND"))?;
    let dati = Json::analizza(&v.value).map_err(|e| Errore::Altro { nome: "SyntaxError", messaggio: e.messaggio })?;
    if dati.get("type").and_then(Json::as_str) != Some(tipo) {
        return Err(codice(400, "CHALLENGE_NOT_FOUND"));
    }
    Ok(dati)
}

/// GET /passkey/generate-register-options: serve una sessione fresca (meno di un giorno).
fn opzioni_registrazione(ctx: &mut Ctx) -> Uscita<Json> {
    let attacco = ctx.query.get("authenticatorAttachment").cloned();
    if let Some(a) = &attacco {
        if !matches!(a.as_str(), Some("platform" | "cross-platform")) {
            return Err(errore(400, "VALIDATION_ERROR", "[query.authenticatorAttachment] Invalid option: expected one of \"platform\"|\"cross-platform\""));
        }
    }
    let (_, u) = ctx.richiedi_sessione_fresca()?;
    let nome_utente = if u.email.is_empty() { u.id.clone() } else { u.email.clone() };
    let esistenti = dell_utente(&u.id)?;
    let id_utente = codifica_url(testo_casuale_minuscolo(32).as_bytes());
    let nome_scelto = ctx.query.get("name").and_then(Json::as_str).filter(|s| !s.is_empty()).map_or_else(|| nome_utente.clone(), str::to_string);
    let mut selezione = Mappa::new();
    selezione.insert("residentKey", "required");
    selezione.insert("userVerification", "required");
    if let Some(a) = attacco {
        selezione.insert("authenticatorAttachment", a);
    }
    selezione.insert("requireResidentKey", true);
    let algoritmi: Vec<Json> = [-8.0, -7.0, -257.0].iter().map(|a| ogg! { "alg": *a, "type": "public-key" }).collect();
    let challenge = sfida();
    let opzioni = ogg! {
        "challenge": challenge.clone(),
        "rp": ogg! { "name": RP_NOME, "id": rp_id() },
        "user": ogg! { "id": id_utente, "name": nome_scelto, "displayName": nome_utente.clone() },
        "pubKeyCredParams": Json::Arr(algoritmi),
        "timeout": ATTESA_MS as f64,
        "attestation": "none",
        "excludeCredentials": Json::Arr(esistenti.iter().map(Riga::credenziale).collect()),
        "authenticatorSelection": Json::Obj(selezione),
        "extensions": ogg! { "credProps": true },
        "hints": Json::Arr(vec![]),
    };
    let contesto = ctx.query.get("context").cloned().unwrap_or(Json::Null);
    let valore = ogg! {
        "type": "registration",
        "expectedChallenge": challenge,
        "userData": ogg! { "id": u.id.clone(), "name": nome_utente.clone(), "displayName": nome_utente },
        "context": contesto,
    };
    salva_sfida(ctx, &valore)?;
    Ok(opzioni)
}

/// GET /passkey/generate-authenticate-options: senza sessione (\"Accedi con passkey\"); con una sessione si elencano le sue passkey.
fn opzioni_accesso(ctx: &mut Ctx) -> Uscita<Json> {
    let sessione = ctx.sessione()?;
    let esistenti = match &sessione {
        Some((_, u)) => dell_utente(&u.id)?,
        None => Vec::new(),
    };
    let challenge = sfida();
    let mut m = Mappa::new();
    m.insert("rpId", rp_id());
    m.insert("challenge", challenge.clone());
    if !esistenti.is_empty() {
        m.insert("allowCredentials", Json::Arr(esistenti.iter().map(Riga::credenziale).collect()));
    }
    m.insert("timeout", ATTESA_MS as f64);
    m.insert("userVerification", "preferred");
    let id = sessione.map(|(_, u)| u.id).unwrap_or_default();
    salva_sfida(ctx, &ogg! { "type": "authentication", "expectedChallenge": challenge, "userData": ogg! { "id": id } })?;
    Ok(Json::Obj(m))
}

/// `resp.response.transports?.join(",") ?? ""`: un valore che non è una lista è il TypeError di JavaScript.
fn trasporti(risposta: &Json) -> Result<String, String> {
    match risposta.get("response").and_then(|r| r.get("transports")) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Arr(v)) => Ok(v.iter().map(|x| if x.is_null() { String::new() } else { string_js(Some(x)) }).collect::<Vec<_>>().join(",")),
        Some(_) => Err("resp.response.transports?.join is not a function".to_string()),
    }
}

/// POST /passkey/verify-registration
fn verifica_registrazione(ctx: &mut Ctx) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("response", Tipo::Qualunque, false), Campo("name", Tipo::Testo, false), Campo("createSession", Tipo::Booleano, false)], false)?;
    let (_, u) = ctx.richiedi_sessione_fresca()?;
    let dati = prendi_sfida(ctx, "registration")?;
    let utente_sfida = dati.get("userData").and_then(|d| d.get("id")).and_then(Json::as_str).unwrap_or("").to_string();
    if utente_sfida != u.id {
        return Err(codice(401, "YOU_ARE_NOT_ALLOWED_TO_REGISTER_THIS_PASSKEY"));
    }
    let attesa = dati.get("expectedChallenge").and_then(Json::as_str).unwrap_or("").to_string();
    let risposta = corpo.get("response").cloned().unwrap_or(Json::Null);
    let fallita = |e: String| {
        eprintln!("auth: Failed to verify registration {e}");
        codice(500, "FAILED_TO_VERIFY_REGISTRATION")
    };
    let registrata = webauthn::verifica_registrazione(&risposta, &attesa, &origine(), &rp_id()).map_err(fallita)?.ok_or_else(|| codice(400, "FAILED_TO_VERIFY_REGISTRATION"))?;
    let trasporti = trasporti(&risposta).map_err(fallita)?;
    let nome_passkey = corpo.get("name").and_then(Json::as_str).map(|s| crate::py::js_trim(s).to_string()).filter(|s| !s.is_empty());
    let crea_sessione = corpo.get("createSession") == Some(&Json::Bool(true));
    let utente = if crea_sessione { Some(righe::utente_per_id(&utente_sfida)?.ok_or_else(|| codice(500, "USER_NOT_FOUND"))?) } else { None };
    let id = genera_id();
    let ora = ctx.ora();
    righe::db()?
        .execute(
            r#"INSERT INTO "passkey" (id, name, publicKey, userId, credentialID, counter, deviceType, backedUp, transports, createdAt, aaguid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![id, nome_passkey, codifica(&registrata.chiave), utente_sfida, registrata.id_credenziale, i64::from(registrata.contatore), registrata.tipo, i64::from(registrata.salvata), trasporti, iso_ms(ora), registrata.aaguid],
        )
        .map_err(Errore::from)?;
    let passkey = per_campo("id", &id)?.ok_or_else(|| codice(500, "FAILED_TO_VERIFY_REGISTRATION"))?.json();
    let Some(utente) = utente else { return Ok(passkey) };
    let s = righe::crea_sessione(&utente.id, false, &ctx.ip(), &ctx.browser(), ora)?;
    ctx.imposta_cookie_sessione(&s, &utente, None);
    let mut m = match passkey {
        Json::Obj(m) => m,
        _ => Mappa::new(),
    };
    m.insert("session", s.json());
    m.insert("user", utente.json());
    Ok(Json::Obj(m))
}

/// POST /passkey/verify-authentication: una firma buona di una passkey salvata apre una sessione nuova.
fn verifica_accesso(ctx: &mut Ctx) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("response", Tipo::Record, true)], false)?;
    let risposta = corpo.get("response").cloned().unwrap_or(Json::Null);
    let dati = prendi_sfida(ctx, "authentication")?;
    let attesa = dati.get("expectedChallenge").and_then(Json::as_str).unwrap_or("").to_string();
    let id = risposta.get("id").and_then(Json::as_str).unwrap_or("");
    let passkey = if id.is_empty() { None } else { per_campo("credentialID", id)? };
    let passkey = passkey.ok_or_else(|| codice(401, "PASSKEY_NOT_FOUND"))?;
    let contatore = u32::try_from(passkey.counter).unwrap_or(u32::MAX);
    let (buona, nuovo) = webauthn::verifica_accesso(&risposta, &attesa, &origine(), &rp_id(), &decodifica_node(&passkey.public_key), contatore).map_err(|e| {
        eprintln!("auth: Failed to verify authentication {e}");
        codice(400, "AUTHENTICATION_FAILED")
    })?;
    if !buona {
        return Err(codice(401, "AUTHENTICATION_FAILED"));
    }
    righe::db()?.execute(r#"UPDATE "passkey" SET counter = ? WHERE id = ?"#, params![i64::from(nuovo), passkey.id]).map_err(Errore::from)?;
    let ora = ctx.ora();
    let s = righe::crea_sessione(&passkey.user_id, false, &ctx.ip(), &ctx.browser(), ora)?;
    let utente = righe::utente_per_id(&passkey.user_id)?.ok_or_else(|| super::rotte::errore_messaggio(500, "User not found"))?;
    ctx.imposta_cookie_sessione(&s, &utente, None);
    Ok(ogg! { "session": s.json(), "user": utente.json() })
}

fn elenco(ctx: &mut Ctx) -> Uscita<Json> {
    let (_, u) = ctx.richiedi_sessione()?;
    Ok(Json::Arr(dell_utente(&u.id)?.iter().map(Riga::json).collect()))
}

/// requireResourceOwnership: la passkey del corpo deve esserci ed essere dell'utente della sessione.
fn sua(ctx: &mut Ctx, id: Option<&Json>, vietato: Option<&str>) -> Uscita<Riga> {
    let (_, u) = ctx.richiedi_sessione()?;
    let id = id.and_then(Json::as_str).filter(|s| !s.is_empty()).ok_or_else(|| super::rotte::errore_messaggio(400, "Missing required parameter: id"))?;
    let p = per_campo("id", id)?.ok_or_else(|| codice(404, "PASSKEY_NOT_FOUND"))?;
    if p.user_id != u.id {
        return Err(match vietato {
            Some(c) => codice(401, c),
            None => ErroreApi { stato: 401, corpo: None },
        });
    }
    Ok(p)
}

fn elimina(ctx: &mut Ctx) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("id", Tipo::Testo, true)], false)?;
    let p = sua(ctx, corpo.get("id"), None)?;
    righe::db()?.execute(r#"DELETE FROM "passkey" WHERE id = ?"#, [&p.id]).map_err(Errore::from)?;
    Ok(ogg! { "status": true })
}

fn aggiorna(ctx: &mut Ctx) -> Uscita<Json> {
    let corpo = valida(ctx.corpo.as_ref(), &[Campo("id", Tipo::Testo, true), Campo("name", Tipo::Testo, true)], false)?;
    let nome_nuovo = crate::py::js_trim(corpo.get("name").and_then(Json::as_str).unwrap_or("")).to_string();
    if nome_nuovo.is_empty() {
        return Err(errore(400, "VALIDATION_ERROR", "[body.name] Too small: expected string to have >=1 characters"));
    }
    let p = sua(ctx, corpo.get("id"), Some("YOU_ARE_NOT_ALLOWED_TO_REGISTER_THIS_PASSKEY"))?;
    righe::db()?.execute(r#"UPDATE "passkey" SET name = ? WHERE id = ?"#, params![nome_nuovo, p.id]).map_err(Errore::from)?;
    let aggiornata = per_campo("id", &p.id)?.ok_or_else(|| codice(500, "FAILED_TO_UPDATE_PASSKEY"))?;
    Ok(ogg! { "passkey": aggiornata.json() })
}
