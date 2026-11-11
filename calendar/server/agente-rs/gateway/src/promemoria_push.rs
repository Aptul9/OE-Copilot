//! Il canale "notifiche del browser" (Web Push, RFC 8030), era lib/core/promemoria-push.ts: il server spedisce il testo al servizio di
//! notifiche del browser (FCM per Chrome, Mozilla per Firefox, Apple per Safari) che lo consegna al browser anche a pagina chiusa. Il testo
//! viaggia cifrato da un capo all'altro (RFC 8291, aes128gcm): il servizio di notifiche vede a chi e quando, non cosa. Chi manda si
//! identifica con una coppia di chiavi VAPID (RFC 8292) che sta nell'ambiente del server. I browser iscritti stanno nella tabella
//! push_iscrizioni di gateway.db; l'indirizzo di un'iscrizione vale come una password, quindi non si scrive in un log e non esce dall'API.

use crate::archivio::archivio;
use crate::b64::{codifica_url, decodifica_node};
use crate::orologio::adesso;
use crate::py::Esito;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use hkdf::Hkdf;
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use p256::ecdsa::signature::Signer;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use rusqlite::{params, OptionalExtension};
use sha2::Sha256;
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

pub const ISCRIZIONI_MAX: usize = 20;
const TTL_S: u32 = 300;
const ATTESA_MS: u64 = 10_000;
// un servizio di notifiche accetta almeno 4096 byte di corpo: 86 di intestazione, 16 della firma, 1 di chiusura e il margine
const TESTO_MAX_BYTE: usize = 3800;

#[derive(Clone, Debug)]
pub struct Iscrizione {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// Un base64url senza altro dentro (niente spazi, niente caratteri che Buffer scarterebbe in silenzio).
fn da_b64(s: Option<&str>) -> Option<Vec<u8>> {
    let s = s?;
    let corpo = s.trim_end_matches('=');
    if corpo.is_empty() || !corpo.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return None;
    }
    Some(decodifica_node(s))
}

// ---- Le chiavi VAPID -----------------------------------------------------------------------------------------------------
#[derive(Clone)]
pub struct ChiaviVapid {
    pub pubblica: String,
    privata: p256::ecdsa::SigningKey,
    pub soggetto: String,
}

thread_local! {
    static MEMORIA: RefCell<Option<(String, Option<ChiaviVapid>)>> = const { RefCell::new(None) };
    static PRONTE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn soggetto_valido(s: &str) -> bool {
    if let Some(resto) = s.strip_prefix("mailto:") {
        let mut p = resto.splitn(2, '@');
        let (Some(a), Some(b)) = (p.next(), p.next()) else { return false };
        let buono = |x: &str| !x.is_empty() && !x.chars().any(|c| c == '@' || crate::py::spazio_js(c));
        return buono(a) && buono(b);
    }
    s.strip_prefix("https://").is_some_and(|r| !r.is_empty() && !r.chars().any(crate::py::spazio_js))
}

/// La coppia di chiavi e il contatto dell'ambiente (VAPID_PUBLIC_KEY, VAPID_PRIVATE_KEY, VAPID_SUBJECT), o None se mancano o non sono una coppia.
pub fn chiavi_vapid() -> Option<ChiaviVapid> {
    let leggi = |n: &str| crate::py::js_trim(&crate::config::var(n).unwrap_or_default()).to_string();
    let (pubblica, privata, soggetto) = (leggi("VAPID_PUBLIC_KEY"), leggi("VAPID_PRIVATE_KEY"), leggi("VAPID_SUBJECT"));
    let impronta = format!("{pubblica}\n{privata}\n{soggetto}");
    if let Some(c) = MEMORIA.with(|m| m.borrow().as_ref().filter(|(i, _)| *i == impronta).map(|(_, c)| c.clone())) {
        return c;
    }
    let chiavi = (|| {
        let p = da_b64(Some(&pubblica))?;
        let d = da_b64(Some(&privata))?;
        if p.len() != 65 || p[0] != 4 || d.len() != 32 || !soggetto_valido(&soggetto) {
            return None;
        }
        let chiave = p256::ecdsa::SigningKey::from_slice(&d).ok()?;
        // la pubblica che sta nell'ambiente deve essere quella della privata: se no ogni invio verrebbe rifiutato dal servizio
        let derivata = chiave.verifying_key().to_encoded_point(false);
        (derivata.as_bytes() == p.as_slice()).then(|| ChiaviVapid { pubblica: codifica_url(&p), privata: chiave, soggetto: soggetto.clone() })
    })();
    MEMORIA.with(|m| *m.borrow_mut() = Some((impronta, chiavi.clone())));
    chiavi
}

/// L'intestazione Authorization di RFC 8292: un JWT ES256 per il servizio che riceve (aud), valido 12 ore, e la chiave pubblica.
pub fn autorizzazione_vapid(endpoint: &str, chiavi: &ChiaviVapid, ora_ms: i64) -> String {
    let intestazione = codifica_url(br#"{"typ":"JWT","alg":"ES256"}"#);
    let aud = url::Url::parse(endpoint).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
    let dati = codifica_url(ogg! { "aud": aud, "exp": (ora_ms.div_euclid(1000) + 12 * 3600) as f64, "sub": chiavi.soggetto.clone() }.in_js().as_bytes());
    let firma: p256::ecdsa::Signature = chiavi.privata.sign(format!("{intestazione}.{dati}").as_bytes());
    format!("vapid t={intestazione}.{dati}.{}, k={}", codifica_url(&firma.to_bytes()), chiavi.pubblica)
}

// ---- La cifratura (RFC 8291) ----------------------------------------------------------------------------------------------
/// Il corpo di un messaggio per un browser: intestazione (sale, dimensione del blocco, chiave pubblica effimera del server) e un solo blocco
/// cifrato con AES-128-GCM. `sale` e `effimera` si passano solo nelle prove, per rifare l'esempio dell'RFC.
pub fn cifra(testo: &str, pubblica_browser: &[u8], segreto_auth: &[u8], sale: Option<[u8; 16]>, effimera: Option<p256::SecretKey>) -> Result<Vec<u8>, String> {
    let sale = sale.unwrap_or_else(|| crate::auth::cripto::casuali(16).try_into().unwrap_or([0; 16]));
    let effimera = match effimera {
        Some(e) => e,
        None => loop {
            if let Ok(k) = p256::SecretKey::from_slice(&crate::auth::cripto::casuali(32)) {
                break k;
            }
        },
    };
    let pubblica_server = effimera.public_key().to_encoded_point(false).as_bytes().to_vec();
    let browser = p256::PublicKey::from_sec1_bytes(pubblica_browser).map_err(|e| e.to_string())?;
    let condiviso = p256::ecdh::diffie_hellman(effimera.to_nonzero_scalar(), browser.as_affine());
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(pubblica_browser);
    info.extend_from_slice(&pubblica_server);
    let mut chiave = [0u8; 32];
    Hkdf::<Sha256>::new(Some(segreto_auth), condiviso.raw_secret_bytes()).expand(&info, &mut chiave).map_err(|e| e.to_string())?;
    let h = Hkdf::<Sha256>::new(Some(&sale), &chiave);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    h.expand(b"Content-Encoding: aes128gcm\0", &mut cek).map_err(|e| e.to_string())?;
    h.expand(b"Content-Encoding: nonce\0", &mut nonce).map_err(|e| e.to_string())?;
    let mut chiaro = testo.as_bytes().to_vec();
    chiaro.push(2);
    let blocco = Aes128Gcm::new_from_slice(&cek).map_err(|e| e.to_string())?.encrypt(Nonce::from_slice(&nonce), chiaro.as_slice()).map_err(|e| e.to_string())?;
    let mut out = sale.to_vec();
    out.extend_from_slice(&4096u32.to_be_bytes());
    out.push(pubblica_server.len() as u8);
    out.extend_from_slice(&pubblica_server);
    out.extend_from_slice(&blocco);
    Ok(out)
}

/// Il testo entro il limite in byte (UTF-8), senza spezzare un carattere.
fn tronca_byte(testo: &str) -> String {
    if testo.len() <= TESTO_MAX_BYTE {
        return testo.to_string();
    }
    let mut t = testo.to_string();
    while t.len() + "…".len() > TESTO_MAX_BYTE {
        t.pop();
    }
    t + "…"
}

// ---- Le iscrizioni -------------------------------------------------------------------------------------------------------
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS push_iscrizioni (
  endpoint TEXT PRIMARY KEY,
  p256dh TEXT NOT NULL,
  auth TEXT NOT NULL,
  creata_il INTEGER NOT NULL
)";

fn db() -> Esito<std::rc::Rc<rusqlite::Connection>> {
    let d = archivio()?;
    let file = crate::config::db_stato();
    if !PRONTE.with(|p| p.borrow().contains(&file)) {
        d.execute_batch(SCHEMA)?;
        PRONTE.with(|p| p.borrow_mut().insert(file));
    }
    Ok(d)
}

pub fn iscrizioni() -> Esito<Vec<Iscrizione>> {
    let d = db()?;
    let mut st = d.prepare_cached("SELECT endpoint, p256dh, auth FROM push_iscrizioni ORDER BY creata_il, endpoint")?;
    let v = st.query_map([], |r| Ok(Iscrizione { endpoint: r.get(0)?, p256dh: r.get(1)?, auth: r.get(2)? }))?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

/// L'indirizzo di un servizio di notifiche: https, un nome (non un indirizzo IP, non un nome della rete interna), porta 443, senza
/// credenziali. Il server ci manda dei dati: non deve poterlo fare verso un servizio qualunque della sua rete.
fn indirizzo_valido(v: Option<&Json>) -> Option<String> {
    let Some(Json::Str(s)) = v else { return None };
    if crate::py::lunghezza_js(s) > 2048 {
        return None;
    }
    let u = url::Url::parse(s).ok()?;
    if !u.username().is_empty() || u.password().is_some() || u.fragment().is_some() {
        return None;
    }
    if u.scheme() != "https" || u.port().is_some_and(|p| p != 443) {
        return None;
    }
    let url::Host::Domain(nome) = u.host()? else { return None };
    let minuscolo = nome.to_ascii_lowercase();
    let interno = ["localhost", "local", "internal", "lan", "home", "corp"].iter().any(|s| minuscolo == *s || minuscolo.ends_with(&format!(".{s}")));
    if !nome.contains('.') || interno || crate::rete_sicura::ip_da_testo(nome).is_some() {
        return None;
    }
    Some(u.as_str().to_string())
}

/// La chiave pubblica del browser (65 byte, un punto vero di P-256: RFC 8291 lo chiede) e il segreto di autenticazione (16 byte).
fn chiavi_browser(p256dh: Option<&Json>, auth: Option<&Json>) -> Option<(String, String)> {
    let p = da_b64(p256dh.and_then(Json::as_str))?;
    let a = da_b64(auth.and_then(Json::as_str))?;
    if p.len() != 65 || p[0] != 4 || a.len() != 16 || p256::PublicKey::from_sec1_bytes(&p).is_err() {
        return None;
    }
    Some((codifica_url(&p), codifica_url(&a)))
}

/// Salva l'iscrizione che il browser ha fatto (PushSubscription.toJSON()): la stessa di prima si aggiorna.
pub fn iscrivi(dati: Option<&Json>) -> Esito<Json> {
    let chiavi = dati.and_then(|d| d.get("keys")).filter(|k| matches!(k, Json::Obj(_))).and_then(|k| chiavi_browser(k.get("p256dh"), k.get("auth")));
    let endpoint = dati.filter(|d| matches!(d, Json::Obj(_))).and_then(|d| indirizzo_valido(d.get("endpoint")));
    let (Some(endpoint), Some((p256dh, auth))) = (endpoint, chiavi) else { return Ok(ogg! { "ok": false, "error": "L'iscrizione del browser non è valida." }) };
    let d = db()?;
    let esiste = d.query_row("SELECT 1 FROM push_iscrizioni WHERE endpoint = ?", [&endpoint], |_| Ok(())).optional()?.is_some();
    if !esiste && iscrizioni()?.len() >= ISCRIZIONI_MAX {
        return Ok(ogg! { "ok": false, "error": format!("Ci sono già {ISCRIZIONI_MAX} browser iscritti.") });
    }
    d.execute(
        "INSERT INTO push_iscrizioni(endpoint, p256dh, auth, creata_il) VALUES (?, ?, ?, ?)
     ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth",
        params![endpoint, p256dh, auth, adesso()],
    )?;
    Ok(ogg! { "ok": true })
}

/// Toglie l'iscrizione di un browser; se non c'è non cambia niente.
pub fn disiscrivi(endpoint: Option<&Json>) -> Esito<Json> {
    if let Some(e) = indirizzo_valido(endpoint) {
        db()?.execute("DELETE FROM push_iscrizioni WHERE endpoint = ?", [e])?;
    }
    Ok(ogg! { "ok": true })
}

// ---- Il canale -----------------------------------------------------------------------------------------------------------
pub struct EsitoInvio {
    pub ok: bool,
    pub errore: String,
    pub attesa_s: Option<f64>,
}

impl EsitoInvio {
    pub fn fatto() -> Self {
        EsitoInvio { ok: true, errore: String::new(), attesa_s: None }
    }

    pub fn no(errore: impl Into<String>, attesa_s: Option<f64>) -> Self {
        EsitoInvio { ok: false, errore: errore.into(), attesa_s }
    }
}

enum Consegna {
    Fatta,
    Scaduta,
    Errore(String, Option<f64>),
}

fn client() -> reqwest::Client {
    thread_local! {
        static C: reqwest::Client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().expect("client HTTP");
    }
    C.with(Clone::clone)
}

async fn consegna(testo: &str, s: &Iscrizione, chiavi: &ChiaviVapid) -> Consegna {
    let corpo = match cifra(testo, &decodifica_node(&s.p256dh), &decodifica_node(&s.auth), None, None) {
        Ok(c) => c,
        Err(_) => return Consegna::Errore("Notifiche del browser: servizio non raggiungibile".to_string(), None),
    };
    let richiesta = client()
        .post(&s.endpoint)
        .header("Authorization", autorizzazione_vapid(&s.endpoint, chiavi, adesso()))
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", TTL_S.to_string())
        .header("Urgency", "high")
        .body(corpo)
        .timeout(Duration::from_millis(ATTESA_MS));
    let r = match richiesta.send().await {
        Ok(r) => r,
        Err(e) => {
            let testo = if e.is_timeout() { "Notifiche del browser: il servizio non risponde (tempo scaduto)" } else { "Notifiche del browser: servizio non raggiungibile" };
            return Consegna::Errore(testo.to_string(), None);
        }
    };
    let stato = r.status().as_u16();
    let attesa = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(crate::py::numero_da_testo);
    let _ = r.bytes().await;
    match stato {
        200..=299 => Consegna::Fatta,
        404 | 410 => Consegna::Scaduta,
        429 => Consegna::Errore("Notifiche del browser: il servizio limita gli invii (429)".to_string(), Some(attesa.filter(|a| a.is_finite() && *a > 0.0).unwrap_or(30.0))),
        401 | 403 => Consegna::Errore(format!("Notifiche del browser: il servizio rifiuta le chiavi VAPID ({stato}): controlla VAPID_PUBLIC_KEY, VAPID_PRIVATE_KEY e VAPID_SUBJECT"), None),
        413 => Consegna::Errore("Notifiche del browser: messaggio troppo grande (413)".to_string(), None),
        s if s >= 500 => Consegna::Errore(format!("Notifiche del browser: servizio non disponibile ({s})"), None),
        s => Consegna::Errore(format!("Notifiche del browser: il servizio ha risposto {s}"), None),
    }
}

pub const NOME: &str = "push";

/// Pronto a mandare: le chiavi ci sono e almeno un browser è iscritto.
pub fn configurato() -> bool {
    chiavi_vapid().is_some() && iscrizioni().is_ok_and(|v| !v.is_empty())
}

pub fn dettagli() -> Vec<(&'static str, Json)> {
    let chiavi = chiavi_vapid();
    let n = if chiavi.is_some() { iscrizioni().map(|v| v.len()).unwrap_or(0) } else { 0 };
    vec![("chiavi", Json::Bool(chiavi.is_some())), ("chiave_pubblica", chiavi.map_or(Json::Null, |c| Json::Str(c.pubblica))), ("iscrizioni", Json::Num(n as f64))]
}

pub async fn invia(testo: &str) -> EsitoInvio {
    let Some(chiavi) = chiavi_vapid() else { return EsitoInvio::no("Notifiche del browser: chiavi VAPID non configurate sul server", None) };
    let lista: Vec<Iscrizione> = iscrizioni().unwrap_or_default().into_iter().filter(|s| indirizzo_valido(Some(&Json::Str(s.endpoint.clone()))).is_some()).collect();
    if lista.is_empty() {
        return EsitoInvio::no("Notifiche del browser: nessun browser iscritto", None);
    }
    let corto = tronca_byte(testo);
    let esiti = futures_util::future::join_all(lista.iter().map(|s| consegna(&corto, s, &chiavi))).await;
    // un'iscrizione che il servizio non conosce più (404, 410) è scaduta: si toglie
    for (s, e) in lista.iter().zip(&esiti) {
        if matches!(e, Consegna::Scaduta) {
            if let Ok(d) = db() {
                let _ = d.execute("DELETE FROM push_iscrizioni WHERE endpoint = ?", [&s.endpoint]);
            }
        }
    }
    // basta un browser che la riceve
    if esiti.iter().any(|e| matches!(e, Consegna::Fatta)) {
        return EsitoInvio::fatto();
    }
    let guasti: Vec<(&String, Option<f64>)> = esiti.iter().filter_map(|e| if let Consegna::Errore(t, a) = e { Some((t, *a)) } else { None }).collect();
    if guasti.is_empty() {
        return EsitoInvio::no("Notifiche del browser: le iscrizioni erano scadute e si sono tolte, riattivale dalle impostazioni", None);
    }
    let attesa = guasti.iter().map(|g| g.1.unwrap_or(0.0)).fold(0.0, f64::max);
    EsitoInvio::no(guasti[0].0.clone(), (attesa > 0.0).then_some(attesa))
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn l_esempio_di_rfc_8291() {
        let effimera = p256::SecretKey::from_slice(&decodifica_node("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let sale: [u8; 16] = decodifica_node("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let corpo = cifra(
            "When I grow up, I want to be a watermelon",
            &decodifica_node("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
            &decodifica_node("BTBZMqHH6r4Tts7J_aSIgg"),
            Some(sale),
            Some(effimera),
        )
        .unwrap();
        assert_eq!(
            codifica_url(&corpo),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn gli_indirizzi_dei_servizi() {
        let v = |s: &str| indirizzo_valido(Some(&Json::Str(s.to_string())));
        assert!(v("https://fcm.googleapis.com/fcm/send/abc").is_some());
        for no in ["http://fcm.googleapis.com/x", "https://1.2.3.4/x", "https://[::1]/x", "https://agent/x", "https://x.local/x", "https://a.b:8443/x", "https://u:p@a.b/x", "https://a.b/x#f"] {
            assert!(v(no).is_none(), "{no}");
        }
    }

    #[test]
    fn il_testo_si_tronca_senza_spezzare_un_carattere() {
        let t = "è".repeat(3000);
        let c = tronca_byte(&t);
        assert!(c.len() <= TESTO_MAX_BYTE && c.ends_with('…'));
    }
}
