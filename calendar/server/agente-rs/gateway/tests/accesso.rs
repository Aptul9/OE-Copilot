//! L'accesso su HTTP vero, contro il gateway intero (servizio::Gateway su 127.0.0.1): le prove di web-ts/test/secondo-fattore.test.ts e
//! passkey.test.ts, che lì girano contro Better Auth. Password, blocco per account, TOTP con i codici di recupero, passkey con un
//! autenticatore in software (ES256, attestazione "none"), il cancello di Caddy e il passaggio fra i modi di AUTH_2FA.

mod comune;

use base64::Engine;
use comune::{ascolta, Cartella};
use oe_gateway::config::imposta_per_prova as imposta;
use oe_gateway::lockout::MAX_FAILED_LOGINS;
use oe_gateway::servizio::Gateway;
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::rc::Rc;
use tokio::task::LocalSet;

const APP: &str = "https://oe.test";
const RP_ID: &str = "oe.test";

/// Un gateway nuovo per la prova, con il suo database, nel modo dato; la prova riceve l'indirizzo.
fn con_gateway<F, Fut>(modo: &str, prova: F)
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    LocalSet::new().block_on(&rt, async {
        let dati = Cartella::nuova("oe-accesso");
        imposta("BETTER_AUTH_SECRET", Some(&"s".repeat(48)));
        imposta("APP_URL", Some(APP));
        imposta("AUTH_2FA", Some(modo));
        imposta("OE_DATI", Some(&dati.0.to_string_lossy()));
        imposta("GATEWAY_DB", Some(&dati.0.join("gateway.db").to_string_lossy()));
        imposta("ADMIN_EMAIL", Some("proprietario@oe.test"));
        imposta("ADMIN_PASSWORD", Some("password-del-proprietario"));
        imposta("AGENTE_URL", Some("http://127.0.0.1:1"));
        imposta("AGENTE_CHIAVE", Some("x"));
        oe_gateway::servizio::prepara().await.expect("avvio");
        let base = ascolta(Rc::new(Gateway)).await;
        prova(base).await;
        oe_gateway::archivio::chiudi_archivio();
        oe_gateway::db::chiudi_tutto();
        drop(dati);
    });
}

fn modo(m: &str) {
    imposta("AUTH_2FA", Some(m));
}

fn a_caso(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("caso");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Clone)]
struct Utente {
    email: String,
    password: String,
}

/// Un utente a caso, creato come il proprietario (owner.rs).
async fn crea_utente() -> Utente {
    let u = Utente { email: format!("prova-{}@oe.test", a_caso(4)), password: a_caso(12) };
    oe_gateway::owner::sync_owner(&u.email, &u.password).await.expect("utente");
    u
}

fn sql<T: rusqlite::types::FromSql>(q: &str, email: &str) -> T {
    oe_gateway::db::app_db().expect("db").query_row(q, [email], |r| r.get(0)).unwrap_or_else(|e| panic!("{q}: {e}"))
}

fn esegui(q: &str, p: &[&dyn rusqlite::ToSql]) {
    oe_gateway::db::app_db().expect("db").execute(q, p).expect("sql");
}

/// twoFactorEnabled: NULL finché l'utente non attiva niente.
fn acceso(email: &str) -> bool {
    sql::<Option<i64>>(r#"SELECT twoFactorEnabled FROM "user" WHERE email = ?"#, email).unwrap_or(0) != 0
}

fn iso(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).expect("istante").strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

struct Risp {
    stato: u16,
    corpo: Json,
}

thread_local! {
    static INDIRIZZI: Cell<u32> = const { Cell::new(0) };
}

/// Un browser: il suo vasetto dei cookie e un indirizzo diverso a ogni richiesta (il limite per indirizzo non è quello che si prova qui).
struct Browser {
    base: String,
    origine: String,
    jar: Vec<(String, String)>,
}

impl Browser {
    fn nuovo(base: &str) -> Browser {
        Browser { base: base.to_string(), origine: APP.to_string(), jar: Vec::new() }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().expect("client")
    }

    fn cookie(&self) -> String {
        self.jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
    }

    async fn chiama(&mut self, percorso: &str, corpo: Option<Json>) -> Risp {
        self.chiama_con(percorso, corpo, &[]).await
    }

    async fn chiama_con(&mut self, percorso: &str, corpo: Option<Json>, extra: &[(&str, &str)]) -> Risp {
        let n = INDIRIZZI.with(|c| {
            c.set(c.get() + 1);
            c.get()
        });
        let url = format!("{}/api/auth{percorso}", self.base);
        let mut r = match &corpo {
            None => Self::client().get(url),
            Some(c) => Self::client().post(url).header("content-type", "application/json").body(c.in_js()),
        };
        r = r.header("origin", &self.origine).header("x-forwarded-for", format!("10.1.{}.{}", (n >> 8) & 255, n & 255));
        for (k, v) in extra {
            r = r.header(*k, *v);
        }
        if !self.jar.is_empty() {
            r = r.header("cookie", self.cookie());
        }
        let res = r.send().await.expect("richiesta");
        for riga in res.headers().get_all("set-cookie") {
            let riga = riga.to_str().expect("cookie");
            let mut pezzi = riga.split(';').map(str::trim);
            let coppia = pezzi.next().unwrap_or("");
            let (nome, valore) = coppia.split_once('=').unwrap_or((coppia, ""));
            let via = valore.is_empty() || pezzi.any(|a| a.eq_ignore_ascii_case("max-age=0"));
            self.jar.retain(|(k, _)| k != nome);
            if !via {
                self.jar.push((nome.to_string(), valore.to_string()));
            }
        }
        let stato = res.status().as_u16();
        let testo = res.text().await.unwrap_or_default();
        Risp { stato, corpo: Json::analizza(&testo).unwrap_or(Json::Null) }
    }

    async fn accedi(&mut self, u: &Utente) -> Risp {
        self.chiama("/sign-in/email", Some(ogg! { "email": u.email.clone(), "password": u.password.clone() })).await
    }

    fn con_sessione(&self) -> bool {
        self.jar.iter().any(|(k, _)| k.ends_with("session_token"))
    }

    fn ha_cookie(&self, fine: &str) -> bool {
        self.jar.iter().any(|(k, _)| k.ends_with(fine))
    }

    async fn utente(&mut self) -> Json {
        self.chiama("/get-session", None).await.corpo.get("user").cloned().unwrap_or(Json::Null)
    }

    /// La risposta del cancello di Caddy (forward_auth) a una richiesta con i cookie di questo browser.
    async fn cancello(&self) -> u16 {
        Self::client()
            .get(format!("{}/api/verifica-sessione", self.base))
            .header("cookie", self.cookie())
            .header("x-forwarded-uri", "/telefono/")
            .header("sec-fetch-dest", "document")
            .send()
            .await
            .expect("cancello")
            .status()
            .as_u16()
    }
}

// ---- TOTP di RFC 6238, fatto qui: le prove non usano il codice che controlla ----------------------------------------------------
fn base32(testo: &str) -> Vec<u8> {
    const ALFABETO: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut bit, mut valore, mut out) = (0u32, 0u32, Vec::new());
    for c in testo.trim_end_matches('=').to_ascii_uppercase().bytes() {
        let i = ALFABETO.iter().position(|x| *x == c).expect("base32") as u32;
        valore = (valore << 5) | i;
        bit += 5;
        if bit >= 8 {
            out.push(((valore >> (bit - 8)) & 0xff) as u8);
            bit -= 8;
        }
    }
    out
}

fn hotp(segreto: &[u8], contatore: u64) -> String {
    use hmac::Mac;
    let mut m = hmac::Hmac::<sha1::Sha1>::new_from_slice(segreto).expect("chiave");
    m.update(&contatore.to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    let n = (u32::from(h[o] & 0x7f) << 24) | (u32::from(h[o + 1]) << 16) | (u32::from(h[o + 2]) << 8) | u32::from(h[o + 3]);
    format!("{:06}", n % 1_000_000)
}

fn ora_s() -> i64 {
    oe_gateway::orologio::ora_vera() / 1000
}

fn totp(segreto: &[u8]) -> String {
    hotp(segreto, (ora_s() / 30) as u64)
}

fn segreto_da_uri(uri: &str) -> Vec<u8> {
    let u = url::Url::parse(uri).expect("uri");
    base32(&u.query_pairs().find(|(k, _)| k == "secret").expect("secret").1)
}

/// Un codice che non vale nel passo di adesso né in quelli accanto.
fn codice_sbagliato(segreto: &[u8]) -> String {
    let buoni: Vec<String> = (-2..=2).map(|d| hotp(segreto, ((ora_s() + d * 30) / 30) as u64)).collect();
    (0..).map(|n| format!("{}", 100_000 + n * 7919)[..6].to_string()).find(|c| !buoni.contains(c)).expect("codice")
}

/// Accede, attiva il TOTP e lo conferma col primo codice: il segreto, i codici di recupero e il browser con la sessione.
async fn iscrivi(base: &str, u: &Utente) -> (Vec<u8>, Vec<String>, Browser) {
    let mut b = Browser::nuovo(base);
    assert_eq!(b.accedi(u).await.stato, 200);
    let abilita = b.chiama("/two-factor/enable", Some(ogg! { "password": u.password.clone() })).await;
    assert_eq!(abilita.stato, 200, "{}", abilita.corpo.in_js());
    let segreto = segreto_da_uri(abilita.corpo["totpURI"].as_str().expect("uri"));
    let codici = abilita.corpo["backupCodes"].as_arr().expect("codici").iter().map(|c| c.as_str().unwrap_or("").to_string()).collect();
    assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
    (segreto, codici, b)
}

#[test]
fn il_generatore_di_codici_delle_prove() {
    let s = b"12345678901234567890";
    assert_eq!([0, 1, 2, 9].map(|c| hotp(s, c)), ["755224", "287082", "359152", "520489"]);
    assert_eq!(base32("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"), s);
}

// ---- modo totp ------------------------------------------------------------------------------------------------------------------
#[test]
fn totp_senza_secondo_fattore_basta_la_password() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let mut b = Browser::nuovo(&base);
        let r = b.accedi(&u).await;
        assert_eq!(r.stato, 200);
        assert!(r.corpo.get("twoFactorRedirect").is_none());
        assert!(b.con_sessione());
        assert_eq!(b.utente().await["email"].as_str(), Some(u.email.as_str()));
        assert_eq!(b.cancello().await, 204);
    });
}

#[test]
fn totp_attivazione_e_conferma() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let mut b = Browser::nuovo(&base);
        b.accedi(&u).await;
        assert_eq!(b.chiama("/two-factor/enable", Some(ogg! { "password": "una password che non è la sua" })).await.stato, 400);
        let r = b.chiama("/two-factor/enable", Some(ogg! { "password": u.password.clone() })).await;
        assert_eq!(r.stato, 200);
        let uri = url::Url::parse(r.corpo["totpURI"].as_str().unwrap()).unwrap();
        assert_eq!((uri.scheme(), uri.host_str()), ("otpauth", Some("totp")));
        let q = |k: &str| uri.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
        assert_eq!((q("issuer").as_deref(), q("digits").as_deref(), q("period").as_deref()), (Some("OE Copilot"), Some("6"), Some("30")));
        assert!(percent_decode(uri.path()).contains(&u.email));
        let codici = r.corpo["backupCodes"].as_arr().unwrap();
        assert_eq!(codici.len(), 10);
        let forma = regex::Regex::new("^[a-zA-Z0-9]{5}-[a-zA-Z0-9]{5}$").unwrap();
        assert!(codici.iter().all(|c| forma.is_match(c.as_str().unwrap())));
        // finché il primo codice non è confermato il secondo fattore non vale
        let verificato: i64 = sql(r#"SELECT t.verified FROM twoFactor t JOIN "user" u ON u.id = t.userId WHERE u.email = ?"#, &u.email);
        assert_eq!(verificato, 0);
        assert!(!acceso(&u.email));
        let mut altro = Browser::nuovo(&base);
        assert_eq!(altro.accedi(&u).await.stato, 200);
        assert!(altro.con_sessione());
        // un primo codice sbagliato non attiva niente, quello giusto sì
        let segreto = segreto_da_uri(r.corpo["totpURI"].as_str().unwrap());
        assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": codice_sbagliato(&segreto) })).await.stato, 401);
        assert!(!acceso(&u.email));
        assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
        assert!(acceso(&u.email));
        // il segreto sta cifrato
        let (s, c): (String, String) = oe_gateway::db::app_db()
            .unwrap()
            .query_row(r#"SELECT t.secret, t.backupCodes FROM twoFactor t JOIN "user" u ON u.id = t.userId WHERE u.email = ?"#, [&u.email], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        let esadecimale = regex::Regex::new("^[0-9a-f]+$").unwrap();
        assert!(esadecimale.is_match(&s) && esadecimale.is_match(&c));
        assert!(!s.contains(&String::from_utf8_lossy(&segreto).into_owned()));
    });
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
}

#[test]
fn totp_la_password_da_sola_non_apre_e_il_codice_si() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (segreto, _, _) = iscrivi(&base, &u).await;
        let mut b = Browser::nuovo(&base);
        let r = b.accedi(&u).await;
        assert_eq!(r.stato, 200);
        assert_eq!(r.corpo["twoFactorRedirect"].as_bool(), Some(true));
        assert_eq!(r.corpo["twoFactorMethods"].in_js(), r#"["totp"]"#);
        assert!(!b.con_sessione());
        assert!(b.utente().await.is_null());
        assert_eq!(b.cancello().await, 302);
        assert!(b.ha_cookie("two_factor"));
        // un codice sbagliato lascia la sfida valida per un altro tentativo
        assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": codice_sbagliato(&segreto) })).await.stato, 401);
        assert!(!b.con_sessione());
        let ok = b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto), "trustDevice": true })).await;
        assert_eq!(ok.stato, 200);
        assert!(b.con_sessione());
        assert_eq!(b.utente().await["email"].as_str(), Some(u.email.as_str()));
        assert_eq!(b.cancello().await, 204);
        assert!(!b.ha_cookie("two_factor"));
        // "fidati di questo dispositivo" non esiste
        assert!(!b.ha_cookie("trust_device"));
        let mut dopo = Browser::nuovo(&base);
        assert_eq!(dopo.accedi(&u).await.corpo["twoFactorRedirect"].as_bool(), Some(true));
        // senza aver passato la password il codice non apre niente
        let mut senza = Browser::nuovo(&base);
        assert_eq!(senza.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 401);
        assert!(!senza.con_sessione());
    });
}

#[test]
fn totp_blocco_per_account() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (segreto, _, _) = iscrivi(&base, &u).await;
        for _ in 0..MAX_FAILED_LOGINS {
            let mut b = Browser::nuovo(&base);
            b.accedi(&u).await;
            assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": codice_sbagliato(&segreto) })).await.stato, 401);
        }
        let mut b = Browser::nuovo(&base);
        b.accedi(&u).await;
        let r = b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await;
        assert_eq!(r.stato, 429);
        assert!(!b.con_sessione());
        let fino: String = sql(r#"SELECT t.lockedUntil FROM twoFactor t JOIN "user" u ON u.id = t.userId WHERE u.email = ?"#, &u.email);
        let resta = fino.parse::<jiff::Timestamp>().expect("data").as_millisecond() - oe_gateway::orologio::ora_vera();
        assert!(resta > 14 * 60_000 && resta <= 15 * 60_000, "{resta}");
        // finito il quarto d'ora il codice giusto torna a valere
        esegui(r#"UPDATE twoFactor SET lockedUntil = ? WHERE userId = (SELECT id FROM "user" WHERE email = ?)"#, &[&iso(oe_gateway::orologio::ora_vera() - 1000), &u.email]);
        assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
    });
}

#[test]
fn totp_blocco_su_una_sfida_e_sullo_stesso_indirizzo() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (segreto, _, _) = iscrivi(&base, &u).await;
        let mut b = Browser::nuovo(&base);
        let stesso = [("x-forwarded-for", "172.18.0.1")];
        b.chiama_con("/sign-in/email", Some(ogg! { "email": u.email.clone(), "password": u.password.clone() }), &stesso).await;
        for _ in 0..MAX_FAILED_LOGINS {
            assert_eq!(b.chiama_con("/two-factor/verify-totp", Some(ogg! { "code": codice_sbagliato(&segreto) }), &stesso).await.stato, 401);
        }
        let bloccato = b.chiama_con("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) }), &stesso).await;
        assert_eq!(bloccato.stato, 429);
        assert_eq!(bloccato.corpo["code"].as_str(), Some("ACCOUNT_TEMPORARILY_LOCKED"));
    });
}

#[test]
fn totp_un_accesso_riuscito_azzera_gli_errori_e_le_password_giuste_non_bloccano() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (segreto, _, _) = iscrivi(&base, &u).await;
        for _ in 0..3 {
            let mut b = Browser::nuovo(&base);
            b.accedi(&u).await;
            for _ in 0..MAX_FAILED_LOGINS - 1 {
                b.chiama("/two-factor/verify-totp", Some(ogg! { "code": codice_sbagliato(&segreto) })).await;
            }
            assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
        }
        for _ in 0..MAX_FAILED_LOGINS + 2 {
            let mut b = Browser::nuovo(&base);
            assert_eq!(b.accedi(&u).await.stato, 200);
            assert_eq!(b.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
        }
        assert_eq!(sql::<i64>("SELECT COUNT(*) FROM login_failures WHERE email = ?", &u.email), 0);
        // la password sbagliata conta come prima
        for i in 0..MAX_FAILED_LOGINS {
            let falsa = format!("sbagliata {i}{}", "x".repeat(12));
            assert_eq!(Browser::nuovo(&base).chiama("/sign-in/email", Some(ogg! { "email": u.email.clone(), "password": falsa })).await.stato, 401);
        }
        assert_eq!(Browser::nuovo(&base).accedi(&u).await.stato, 429);
    });
}

#[test]
fn totp_codici_di_recupero() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (_, codici, mut sessione) = iscrivi(&base, &u).await;
        let mut b = Browser::nuovo(&base);
        b.accedi(&u).await;
        assert_eq!(b.chiama("/two-factor/verify-backup-code", Some(ogg! { "code": codici[0].clone() })).await.stato, 200);
        assert!(b.con_sessione());
        assert_eq!(b.cancello().await, 204);
        let mut secondo = Browser::nuovo(&base);
        secondo.accedi(&u).await;
        assert_eq!(secondo.chiama("/two-factor/verify-backup-code", Some(ogg! { "code": codici[0].clone() })).await.stato, 401);
        assert!(!secondo.con_sessione());
        assert_eq!(secondo.chiama("/two-factor/verify-backup-code", Some(ogg! { "code": codici[1].clone() })).await.stato, 200);
        assert!(secondo.con_sessione());
        // un codice inventato conta come un errore
        let mut terzo = Browser::nuovo(&base);
        terzo.accedi(&u).await;
        assert_eq!(terzo.chiama("/two-factor/verify-backup-code", Some(ogg! { "code": "aaaaa-bbbbb" })).await.stato, 401);
        let conto: i64 = sql(r#"SELECT t.failedVerificationCount FROM twoFactor t JOIN "user" x ON x.id = t.userId WHERE x.email = ?"#, &u.email);
        assert_eq!(conto, 1);
        // dieci codici nuovi sostituiscono i vecchi, e serve la password
        assert_eq!(sessione.chiama("/two-factor/generate-backup-codes", Some(ogg! { "password": "una password che non è la sua" })).await.stato, 400);
        let r = sessione.chiama("/two-factor/generate-backup-codes", Some(ogg! { "password": u.password.clone() })).await;
        assert_eq!(r.stato, 200);
        let nuovi = r.corpo["backupCodes"].as_arr().unwrap().clone();
        assert_eq!(nuovi.len(), 10);
        let mut c = Browser::nuovo(&base);
        c.accedi(&u).await;
        assert_eq!(c.chiama("/two-factor/verify-backup-code", Some(ogg! { "code": codici[2].clone() })).await.stato, 401);
        assert_eq!(c.chiama("/two-factor/verify-backup-code", Some(ogg! { "code": nuovi[0].clone() })).await.stato, 200);
    });
}

#[test]
fn totp_disattivazione_e_nuova_attivazione() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (prima, _, mut b) = iscrivi(&base, &u).await;
        // senza sessione non si disattiva niente
        assert_eq!(Browser::nuovo(&base).chiama("/two-factor/disable", Some(ogg! { "password": u.password.clone() })).await.stato, 401);
        assert!(acceso(&u.email));
        assert_eq!(b.chiama("/two-factor/disable", Some(ogg! { "password": "una password che non è la sua" })).await.stato, 400);
        assert_eq!(b.chiama("/two-factor/disable", Some(ogg! { "password": u.password.clone() })).await.stato, 200);
        assert_eq!(sql::<i64>(r#"SELECT COUNT(*) FROM twoFactor t JOIN "user" x ON x.id = t.userId WHERE x.email = ?"#, &u.email), 0);
        assert!(!acceso(&u.email));
        let mut dopo = Browser::nuovo(&base);
        let r = dopo.accedi(&u).await;
        assert!(r.stato == 200 && r.corpo.get("twoFactorRedirect").is_none() && dopo.con_sessione());
        // di nuovo, con un segreto nuovo
        let (nuovo, _, _) = iscrivi(&base, &u).await;
        assert_ne!(nuovo, prima);
        let mut c = Browser::nuovo(&base);
        assert_eq!(c.accedi(&u).await.corpo["twoFactorRedirect"].as_bool(), Some(true));
        assert_eq!(c.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&prima) })).await.stato, 401);
        assert_eq!(c.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&nuovo) })).await.stato, 200);
    });
}

#[test]
fn totp_le_strade_che_la_pagina_non_usa_e_quelle_delle_passkey_non_ci_sono() {
    con_gateway("totp", |base| async move {
        let mut b = Browser::nuovo(&base);
        for p in ["/two-factor/send-otp", "/two-factor/verify-otp", "/two-factor/get-totp-uri"] {
            assert_eq!(b.chiama(p, Some(ogg! { "password": "x" })).await.stato, 404, "{p}");
        }
        assert_eq!(b.chiama("/passkey/generate-authenticate-options", None).await.stato, 404);
        assert_eq!(b.chiama("/passkey/list-user-passkeys", None).await.stato, 404);
    });
}

// ---- modo off -------------------------------------------------------------------------------------------------------------------
#[test]
fn off_le_strade_del_secondo_fattore_non_esistono() {
    con_gateway("off", |base| async move {
        let mut b = Browser::nuovo(&base);
        for p in ["/two-factor/enable", "/two-factor/verify-totp", "/two-factor/verify-backup-code", "/two-factor/disable"] {
            assert_eq!(b.chiama(p, Some(ogg! { "password": "x", "code": "123456" })).await.stato, 404, "{p}");
        }
        for p in ["/passkey/generate-authenticate-options", "/passkey/list-user-passkeys", "/TWO-FACTOR/enable", "/two-factor//enable"] {
            assert_eq!(b.chiama(p, None).await.stato, 404, "{p}");
        }
        // la barra in fondo rimanda prima di tutto, come Next: si arriva al percorso senza, che è 404
        assert_eq!(b.chiama("/two-factor//enable/", None).await.stato, 308);
    });
}

#[test]
fn off_e_ritorno_non_cancellano_niente() {
    con_gateway("totp", |base| async move {
        let u = crea_utente().await;
        let (segreto, _, _) = iscrivi(&base, &u).await;
        // con il TOTP già attivato, in off basta la password: è la strada di recupero
        modo("off");
        let mut b = Browser::nuovo(&base);
        let r = b.accedi(&u).await;
        assert!(r.stato == 200 && r.corpo.get("twoFactorRedirect").is_none() && b.con_sessione());
        assert_eq!(b.utente().await["email"].as_str(), Some(u.email.as_str()));
        assert_eq!(b.cancello().await, 204);
        modo("totp");
        let mut c = Browser::nuovo(&base);
        assert_eq!(c.accedi(&u).await.corpo["twoFactorRedirect"].as_bool(), Some(true));
        assert_eq!(c.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
    });
}

#[test]
fn off_l_accesso_con_la_password_e_il_suo_blocco() {
    con_gateway("off", |base| async move {
        let u = crea_utente().await;
        let mut b = Browser::nuovo(&base);
        let r = b.accedi(&u).await;
        assert_eq!(r.stato, 200);
        let mut chiavi: Vec<String> = r.corpo.as_obj().unwrap().keys().cloned().collect();
        chiavi.sort();
        assert_eq!(chiavi, ["redirect", "token", "user"]);
        // l'uscita toglie la sessione
        assert!(b.con_sessione());
        assert_eq!(b.chiama("/sign-out", Some(ogg! {})).await.stato, 200);
        assert!(!b.con_sessione());
        assert_eq!(b.cancello().await, 302);
        for i in 0..MAX_FAILED_LOGINS {
            let falsa = format!("sbagliata {i}{}", "x".repeat(12));
            assert_eq!(Browser::nuovo(&base).chiama("/sign-in/email", Some(ogg! { "email": u.email.clone(), "password": falsa })).await.stato, 401);
        }
        assert_eq!(Browser::nuovo(&base).accedi(&u).await.stato, 429);
    });
}

// ---- passkey --------------------------------------------------------------------------------------------------------------------
fn b64u(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn sha256(b: &[u8]) -> Vec<u8> {
    Sha256::digest(b).to_vec()
}

fn testa_cbor(maggiore: u8, n: usize) -> Vec<u8> {
    match n {
        0..24 => vec![(maggiore << 5) | n as u8],
        24..256 => vec![(maggiore << 5) | 24, n as u8],
        _ => vec![(maggiore << 5) | 25, (n >> 8) as u8, n as u8],
    }
}

enum Cbor<'a> {
    Int(i64),
    Testo(&'a str),
    Byte(&'a [u8]),
    Mappa(Vec<(Cbor<'a>, Cbor<'a>)>),
}

fn cbor(v: &Cbor) -> Vec<u8> {
    match v {
        Cbor::Int(n) if *n >= 0 => testa_cbor(0, *n as usize),
        Cbor::Int(n) => testa_cbor(1, (-1 - n) as usize),
        Cbor::Testo(t) => [testa_cbor(3, t.len()), t.as_bytes().to_vec()].concat(),
        Cbor::Byte(b) => [testa_cbor(2, b.len()), b.to_vec()].concat(),
        Cbor::Mappa(m) => {
            let mut out = testa_cbor(5, m.len());
            for (k, x) in m {
                out.extend(cbor(k));
                out.extend(cbor(x));
            }
            out
        }
    }
}

fn chiave_nuova() -> SigningKey {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("caso");
    SigningKey::from_bytes(&b.into()).expect("chiave")
}

/// Un autenticatore WebAuthn in software: fa le veci del browser, così le cerimonie si provano per intero contro il server.
struct Autenticatore {
    id: Vec<u8>,
    chiave: SigningKey,
    contatore: u32,
    origine: String,
    rp_id: String,
}

impl Autenticatore {
    fn nuovo() -> Autenticatore {
        let mut id = vec![0u8; 32];
        getrandom::fill(&mut id).expect("caso");
        Autenticatore { id, chiave: chiave_nuova(), contatore: 0, origine: APP.into(), rp_id: RP_ID.into() }
    }

    fn dati_autenticatore(&self, flag: u8, credenziale: &[u8]) -> Vec<u8> {
        [sha256(self.rp_id.as_bytes()), vec![flag], self.contatore.to_be_bytes().to_vec(), credenziale.to_vec()].concat()
    }

    fn dati_client(&self, tipo: &str, challenge: &str) -> Vec<u8> {
        ogg! { "type": tipo, "challenge": challenge, "origin": self.origine.clone(), "crossOrigin": false }.in_js().into_bytes()
    }

    fn registra(&self, opzioni: &Json) -> Json {
        let p = self.chiave.verifying_key().to_encoded_point(false);
        let cose = cbor(&Cbor::Mappa(vec![
            (Cbor::Int(1), Cbor::Int(2)),
            (Cbor::Int(3), Cbor::Int(-7)),
            (Cbor::Int(-1), Cbor::Int(1)),
            (Cbor::Int(-2), Cbor::Byte(p.x().unwrap())),
            (Cbor::Int(-3), Cbor::Byte(p.y().unwrap())),
        ]));
        let credenziale = [vec![0u8; 16], (self.id.len() as u16).to_be_bytes().to_vec(), self.id.clone(), cose].concat();
        // UP, UV, dati della credenziale presenti
        let dati = self.dati_autenticatore(0x01 | 0x04 | 0x40, &credenziale);
        let attestazione = cbor(&Cbor::Mappa(vec![(Cbor::Testo("fmt"), Cbor::Testo("none")), (Cbor::Testo("attStmt"), Cbor::Mappa(vec![])), (Cbor::Testo("authData"), Cbor::Byte(&dati))]));
        let client = self.dati_client("webauthn.create", opzioni["challenge"].as_str().expect("challenge"));
        ogg! {
            "id": b64u(&self.id), "rawId": b64u(&self.id), "type": "public-key", "authenticatorAttachment": "platform",
            "response": ogg! { "clientDataJSON": b64u(&client), "attestationObject": b64u(&attestazione), "transports": Json::Arr(vec![Json::Str("internal".into())]) },
        }
    }

    fn firma_con(&mut self, challenge: &str, chiave: Option<&SigningKey>) -> Json {
        self.contatore += 1;
        let dati = self.dati_autenticatore(0x01 | 0x04, &[]);
        let client = self.dati_client("webauthn.get", challenge);
        let firma: Signature = chiave.unwrap_or(&self.chiave).sign(&[dati.clone(), sha256(&client)].concat());
        ogg! {
            "id": b64u(&self.id), "rawId": b64u(&self.id), "type": "public-key", "authenticatorAttachment": "platform",
            "response": ogg! { "clientDataJSON": b64u(&client), "authenticatorData": b64u(&dati), "signature": b64u(firma.to_der().as_bytes()) },
        }
    }

    fn firma(&mut self, opzioni: &Json) -> Json {
        self.firma_con(opzioni["challenge"].as_str().expect("challenge"), None)
    }
}

/// Accede con la password e registra la passkey dell'autenticatore: il browser con la sessione e la risposta.
async fn registra(base: &str, u: &Utente, aut: &Autenticatore, nome: &str) -> (Browser, Risp) {
    let mut b = Browser::nuovo(base);
    assert_eq!(b.accedi(u).await.stato, 200);
    let opzioni = b.chiama(&format!("/passkey/generate-register-options?name={nome}"), None).await;
    assert_eq!(opzioni.stato, 200, "{}", opzioni.corpo.in_js());
    let r = b.chiama("/passkey/verify-registration", Some(ogg! { "response": aut.registra(&opzioni.corpo), "name": nome })).await;
    (b, r)
}

async fn accedi_con_passkey(base: &str, firma: impl FnOnce(&Json) -> Json) -> (Browser, Risp) {
    let mut b = Browser::nuovo(base);
    let opzioni = b.chiama("/passkey/generate-authenticate-options", None).await;
    let r = b.chiama("/passkey/verify-authentication", Some(ogg! { "response": firma(&opzioni.corpo) })).await;
    (b, r)
}

#[test]
fn passkey_opzioni() {
    con_gateway("passkey", |base| async move {
        let r = Browser::nuovo(&base).chiama("/passkey/generate-authenticate-options", None).await;
        assert_eq!(r.stato, 200);
        assert_eq!(r.corpo["rpId"].as_str(), Some(RP_ID));
        assert!(r.corpo["challenge"].as_str().is_some());
        assert!(r.corpo.get("allowCredentials").and_then(Json::as_arr).is_none_or(Vec::is_empty));
        let mut senza = Browser::nuovo(&base);
        assert_eq!(senza.chiama("/passkey/generate-register-options", None).await.stato, 401);
        assert_eq!(senza.chiama("/passkey/list-user-passkeys", None).await.stato, 401);
        let u = crea_utente().await;
        let mut b = Browser::nuovo(&base);
        b.accedi(&u).await;
        let o = b.chiama("/passkey/generate-register-options?name=prova", None).await;
        assert_eq!(o.stato, 200);
        assert_eq!(o.corpo["rp"].in_js(), r#"{"name":"OE Copilot","id":"oe.test"}"#);
        assert_eq!(o.corpo["authenticatorSelection"]["residentKey"].as_str(), Some("required"));
        assert_eq!(o.corpo["authenticatorSelection"]["userVerification"].as_str(), Some("required"));
        assert_eq!(o.corpo["attestation"].as_str(), Some("none"));
        // il TOTP resta: chi lo ha lo usa con la password
        let v = crea_utente().await;
        let (segreto, _, _) = iscrivi(&base, &v).await;
        let mut c = Browser::nuovo(&base);
        assert_eq!(c.accedi(&v).await.corpo["twoFactorRedirect"].as_bool(), Some(true));
        assert_eq!(c.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
    });
}

#[test]
fn passkey_registrata_apre_senza_password() {
    con_gateway("passkey", |base| async move {
        let u = crea_utente().await;
        let mut aut = Autenticatore::nuovo();
        let (_, r) = registra(&base, &u, &aut, "Portatile").await;
        assert_eq!(r.stato, 200, "{}", r.corpo.in_js());
        assert_eq!(sql::<i64>(r#"SELECT COUNT(*) FROM passkey p JOIN "user" x ON x.id = p.userId WHERE x.email = ?"#, &u.email), 1);
        let (mut b, accesso) = accedi_con_passkey(&base, |o| aut.firma(o)).await;
        assert_eq!(accesso.stato, 200, "{}", accesso.corpo.in_js());
        assert!(b.con_sessione());
        assert_eq!(b.utente().await["email"].as_str(), Some(u.email.as_str()));
        assert_eq!(b.cancello().await, 204);
        // un contatore che non cresce (credenziale copiata) è rifiutato
        assert_eq!(accedi_con_passkey(&base, |o| aut.firma(o)).await.1.stato, 200);
        aut.contatore = 0;
        let (b, r) = accedi_con_passkey(&base, |o| aut.firma(o)).await;
        assert!(r.stato >= 400 && !b.con_sessione());
    });
}

#[test]
fn passkey_lista_nome_e_tolta() {
    con_gateway("passkey", |base| async move {
        let u = crea_utente().await;
        let mut aut = Autenticatore::nuovo();
        let (mut b, _) = registra(&base, &u, &aut, "Telefono").await;
        let lista = b.chiama("/passkey/list-user-passkeys", None).await.corpo;
        assert_eq!(lista.as_arr().unwrap().len(), 1);
        assert_eq!(lista[0]["name"].as_str(), Some("Telefono"));
        assert!(lista[0].get("privateKey").is_none());
        let id = lista[0]["id"].clone();
        assert_eq!(b.chiama("/passkey/update-passkey", Some(ogg! { "id": id.clone(), "name": "Telefono di casa" })).await.stato, 200);
        assert_eq!(b.chiama("/passkey/list-user-passkeys", None).await.corpo[0]["name"].as_str(), Some("Telefono di casa"));
        // la passkey di un altro non si toglie e non si rinomina
        let v = crea_utente().await;
        let (mut altro, _) = registra(&base, &v, &Autenticatore::nuovo(), "Sua").await;
        assert!(altro.chiama("/passkey/delete-passkey", Some(ogg! { "id": id.clone() })).await.stato >= 400);
        assert!(altro.chiama("/passkey/update-passkey", Some(ogg! { "id": id.clone(), "name": "rubata" })).await.stato >= 400);
        assert_eq!(b.chiama("/passkey/list-user-passkeys", None).await.corpo[0]["name"].as_str(), Some("Telefono di casa"));
        assert_eq!(b.chiama("/passkey/delete-passkey", Some(ogg! { "id": id })).await.stato, 200);
        assert_eq!(b.chiama("/passkey/list-user-passkeys", None).await.corpo.in_js(), "[]");
        let (dopo, r) = accedi_con_passkey(&base, |o| aut.firma(o)).await;
        assert_eq!(r.stato, 401);
        assert!(!dopo.con_sessione());
    });
}

#[test]
fn passkey_sessione_vecchia_firma_falsa_altro_sito_sfida_ripetuta() {
    con_gateway("passkey", |base| async move {
        let u = crea_utente().await;
        let mut b = Browser::nuovo(&base);
        b.accedi(&u).await;
        esegui(r#"UPDATE session SET createdAt = ? WHERE userId = (SELECT id FROM "user" WHERE email = ?)"#, &[&iso(oe_gateway::orologio::ora_vera() - 2 * 86_400_000), &u.email]);
        let r = b.chiama("/passkey/generate-register-options", None).await;
        assert_eq!(r.stato, 403);
        assert_eq!(r.corpo["code"].as_str(), Some("SESSION_NOT_FRESH"));

        let v = crea_utente().await;
        let mut aut = Autenticatore::nuovo();
        registra(&base, &v, &aut, "Portatile").await;
        let estranea = chiave_nuova();
        let (b, r) = accedi_con_passkey(&base, |o| aut.firma_con(o["challenge"].as_str().unwrap(), Some(&estranea))).await;
        assert!(r.stato >= 400 && !b.con_sessione(), "firma di un'altra chiave");
        for (origine, rp) in [("https://evil.test", RP_ID), (APP, "evil.test")] {
            aut.origine = origine.into();
            aut.rp_id = rp.into();
            let (b, r) = accedi_con_passkey(&base, |o| aut.firma(o)).await;
            assert!(r.stato >= 400 && !b.con_sessione(), "{origine} {rp}");
        }
        aut.origine = APP.into();
        aut.rp_id = RP_ID.into();
        // una sfida si usa una volta sola
        let mut b = Browser::nuovo(&base);
        let opzioni = b.chiama("/passkey/generate-authenticate-options", None).await.corpo;
        let risposta = aut.firma(&opzioni);
        assert_eq!(b.chiama("/passkey/verify-authentication", Some(ogg! { "response": risposta.clone() })).await.stato, 200);
        let mut altro = Browser::nuovo(&base);
        altro.jar = b.jar.iter().filter(|(k, _)| !k.ends_with("session_token")).cloned().collect();
        assert!(altro.chiama("/passkey/verify-authentication", Some(ogg! { "response": risposta })).await.stato >= 400);
        assert!(!altro.con_sessione());
    });
}

#[test]
fn passkey_con_il_totp_e_in_off() {
    con_gateway("passkey", |base| async move {
        let u = crea_utente().await;
        let mut aut = Autenticatore::nuovo();
        let (mut sessione, _) = registra(&base, &u, &aut, "Portatile").await;
        let abilita = sessione.chiama("/two-factor/enable", Some(ogg! { "password": u.password.clone() })).await.corpo;
        let segreto = segreto_da_uri(abilita["totpURI"].as_str().unwrap());
        assert_eq!(sessione.chiama("/two-factor/verify-totp", Some(ogg! { "code": totp(&segreto) })).await.stato, 200);
        // la passkey è un accesso completo, la password chiede il codice
        let (b, r) = accedi_con_passkey(&base, |o| aut.firma(o)).await;
        assert!(r.stato == 200 && b.con_sessione());
        let mut con_password = Browser::nuovo(&base);
        assert_eq!(con_password.accedi(&u).await.corpo["twoFactorRedirect"].as_bool(), Some(true));
        assert!(!con_password.con_sessione());
        // con off le strade delle passkey non ci sono e la password basta
        modo("off");
        let mut b = Browser::nuovo(&base);
        assert_eq!(b.chiama("/passkey/generate-authenticate-options", None).await.stato, 404);
        assert_eq!(b.chiama("/passkey/verify-authentication", Some(ogg! { "response": aut.firma_con("x", None) })).await.stato, 404);
        assert_eq!(b.accedi(&u).await.stato, 200);
        assert!(b.con_sessione());
    });
}
