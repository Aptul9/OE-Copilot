//! Le prove di `agente-ts/test/shared-account.test.ts`: il broker delle caselle condivise (rinnovi in fila, identità e audience
//! controllate, cache per risorsa), il recupero dalla sessione del browser, le rotte HTTP che lo usano.

mod comune;

use comune::{canonico, chiedi, chiedi_tutto};
use oe_agente::diretto::{GestoreToken, OpzioniToken};
use oe_agente::http::servi;
use oe_agente::server::{Agente, OpzioniAgente};
use oe_nucleo::json::Json;
use oe_rete::shared_account::{SharedAccount, TEAMS_APP_CLIENT_ID, allowed, client_for, renew_shared};
use oe_rete::trasporto::prova::json;
use oe_rete::trasporto::{EsitoRete, FutureRete, Richiesta, Risposta, Trasporto};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::LocalSet;

const OUTLOOK_MOBILE: &str = "27922004-5251-4030-b22d-91ecd9a37ea4";
const ENTRY: &str = r#"{"provider":"microsoft","refresh_token":"initial","shared":{"tenantId":"tenant","objectId":"object","capabilities":["mail","calendar","chat"],"origin":"mobile"}}"#;

fn b64(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

fn token(claims: &str) -> String {
    format!("x.{}.x", b64(claims))
}

fn jwt(aud: &str, oid: &str, appid: Option<&str>) -> String {
    let app = appid.map(|a| format!(r#","appid":"{a}""#)).unwrap_or_default();
    token(&format!(r#"{{"aud":"{aud}","oid":"{oid}","tid":"tenant","exp":9000,"scp":"Mail.ReadWrite Calendars.ReadWrite"{app}}}"#))
}

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

fn campo(r: &Richiesta, k: &str) -> String {
    let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
    form_urlencoded::parse(corpo.as_bytes()).find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default()
}

/// `scope` fino a "/.default": la risorsa chiesta.
fn risorsa(r: &Richiesta) -> String {
    campo(r, "scope").split("/.default").next().unwrap_or_default().to_string()
}

/// Microsoft simulato: una funzione per richiesta, con un'attesa e un cancello facoltativi prima della risposta.
struct Microsoft {
    f: RefCell<Box<dyn FnMut(&Richiesta) -> EsitoRete>>,
    attesa: Duration,
    cancello: Option<Rc<Notify>>,
    chiamate: RefCell<Vec<String>>,
}

impl Trasporto for Microsoft {
    fn invia(&self, r: Richiesta) -> FutureRete<'_> {
        Box::pin(async move {
            self.chiamate.borrow_mut().push(r.url.clone());
            if let Some(c) = &self.cancello {
                c.notified().await;
            }
            if !self.attesa.is_zero() {
                tokio::time::sleep(self.attesa).await;
            }
            (self.f.borrow_mut())(&r)
        })
    }
}

fn microsoft(f: impl FnMut(&Richiesta) -> EsitoRete + 'static) -> Rc<Microsoft> {
    Rc::new(Microsoft { f: RefCell::new(Box::new(f)), attesa: Duration::ZERO, cancello: None, chiamate: RefCell::new(Vec::new()) })
}

struct Prova {
    dir: PathBuf,
    file: PathBuf,
    ms: Rc<Microsoft>,
    manager: Rc<GestoreToken>,
}

impl Prova {
    fn nuova(ms: Rc<Microsoft>) -> Prova {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("shared-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tokens.json");
        std::fs::write(&file, format!(r#"{{"accounts":{{"user":{ENTRY}}}}}"#)).unwrap();
        let manager = Rc::new(Prova::gestore(&file, &ms, None));
        Prova { dir, file, ms, manager }
    }

    fn gestore(file: &std::path::Path, ms: &Rc<Microsoft>, token_posta: Option<PathBuf>) -> GestoreToken {
        let t: Rc<dyn Trasporto> = ms.clone();
        GestoreToken::nuovo(file.to_path_buf(), OpzioniToken { token_posta, adesso: Some(Rc::new(|| 1_000_000)), trasporto: t, strict: false })
    }

    fn scrivi(&self, accounts: &str) {
        std::fs::write(&self.file, format!(r#"{{"accounts":{accounts}}}"#)).unwrap();
    }

    fn su_disco(&self) -> Json {
        j(&std::fs::read_to_string(&self.file).unwrap())
    }

    fn voce(&self, account: &str) -> Json {
        self.manager.voce_account(account).map_or(Json::Null, |v| Json::Obj(v.borrow().clone()))
    }

    /// L'agente con questo gestore, in ascolto: la porta.
    async fn server(&self, token_posta: Option<PathBuf>) -> u16 {
        let mut o = OpzioniAgente::nuove("key", Vec::new(), self.dir.join("heartbeat"), "Europe/Rome");
        o.registro = Rc::new(|_, _| {});
        o.tokens = Some(self.manager.clone());
        if let Some(p) = token_posta {
            o.token_posta = p;
        }
        let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let porta = ascolto.local_addr().unwrap().port();
        tokio::task::spawn_local(servi(ascolto, Rc::new(Agente::nuovo(o))));
        porta
    }
}

impl Drop for Prova {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn token_risposta(access: &str, refresh: Option<&str>) -> Risposta {
    let r = refresh.map(|r| format!(r#","refresh_token":"{r}""#)).unwrap_or_default();
    json(200, &format!(r#"{{"access_token":"{access}"{r}}}"#))
}

async fn in_locale(f: impl std::future::Future<Output = ()>) {
    LocalSet::new().run_until(f).await;
}

// ---- broker ----------------------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn rinnovi_di_outlook_e_teams_in_fila_e_cache_dopo_un_riavvio() {
    let visti: Rc<RefCell<Vec<String>>> = Rc::default();
    let v2 = visti.clone();
    let f = move |r: &Richiesta| {
        v2.borrow_mut().push(campo(r, "refresh_token"));
        let n = v2.borrow().len();
        Ok(token_risposta(&jwt(&risorsa(r), "object", Some(&campo(r, "client_id"))), Some(&format!("rotated{n}"))))
    };
    let ms = Rc::new(Microsoft { f: RefCell::new(Box::new(f)), attesa: Duration::from_millis(5), cancello: None, chiamate: RefCell::default() });
    let p = Prova::nuova(ms);
    let (mail, chat) = tokio::join!(p.manager.ottieni_access_token("user", false), p.manager.ottieni_risorsa("user", "csa", false));
    let (mail, chat) = (mail.unwrap(), chat.unwrap());
    assert_ne!(mail, chat);
    assert_eq!(*visti.borrow(), ["initial", "rotated1"]);
    let di_nuovo = Prova::gestore(&p.file, &p.ms, None);
    assert_eq!(di_nuovo.ottieni_risorsa("user", "csa", false).await.unwrap(), chat);
    assert_eq!(visti.borrow().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_audience_o_oggetto_sbagliati_senza_salvare_la_rotazione() {
    for (aud, oid) in [("https://evil.invalid", "object"), ("https://outlook.office.com", "other")] {
        let t = jwt(aud, oid, None);
        let p = Prova::nuova(microsoft(move |_| Ok(token_risposta(&t, Some("bad")))));
        let e = p.manager.ottieni_access_token("user", false).await.unwrap_err();
        assert!(e.to_string().contains("mismatch"), "{e}");
        assert_eq!(p.su_disco()["accounts"]["user"]["refresh_token"], Json::from("initial"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_la_credenziale_tolta_o_sostituita_durante_il_rinnovo() {
    for azione in ["remove", "replace"] {
        let cancello = Rc::new(Notify::new());
        let t = jwt("https://outlook.office.com", "object", None);
        let ms = Rc::new(Microsoft {
            f: RefCell::new(Box::new(move |_| Ok(token_risposta(&t, Some("stale"))))),
            attesa: Duration::ZERO,
            cancello: Some(cancello.clone()),
            chiamate: RefCell::default(),
        });
        let p = Prova::nuova(ms);
        let (esito, ()) = tokio::join!(p.manager.ottieni_access_token("user", false), async {
            tokio::task::yield_now().await;
            if azione == "remove" {
                p.manager.rimuovi("user").unwrap();
            } else {
                let sostituita = ENTRY.replace("\"initial\"", "\"replacement\"");
                p.scrivi(&format!(r#"{{"user":{sostituita}}}"#));
            }
            cancello.notify_one();
        });
        let e = esito.unwrap_err();
        assert!(e.to_string().contains("changed"), "{azione}: {e}");
        let atteso = if azione == "remove" { Json::Null } else { Json::from("replacement") };
        assert_eq!(p.su_disco()["accounts"]["user"]["refresh_token"], atteso, "{azione}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn broker_http_autenticato_senza_errori_ne_cache_dei_token() {
    in_locale(async {
        let t = jwt("https://chatsvcagg.teams.microsoft.com", "object", None);
        let t2 = t.clone();
        let p = Prova::nuova(microsoft(move |_| Ok(token_risposta(&t2, Some("next")))));
        let porta = p.server(None).await;
        let percorso = "/api/token-resource?account=user&resource=csa";
        assert_eq!(chiedi(porta, "GET", percorso, None, None).await.0, 401);
        let (stato, h, testo) = chiedi_tutto(porta, "GET", percorso, Some("key"), None).await;
        assert_eq!(stato, 200);
        assert_eq!(h.iter().find(|(k, _)| k == "cache-control").map(|(_, v)| v.as_str()), Some("no-store"));
        assert_eq!(j(&testo)["token"], Json::from(t.as_str()));
        let (stato, testo) = chiedi(porta, "GET", &percorso.replace("resource=csa", "resource=evil"), Some("key"), None).await;
        assert_eq!((stato, canonico(&j(&testo))), (403, r#"{"errore":"shared token unavailable"}"#.to_string()));
    })
    .await;
}

fn condivisa(capacita: &[&str], origine: &str) -> SharedAccount {
    SharedAccount { tenant_id: "tenant".into(), object_id: "object".into(), capabilities: capacita.iter().map(|c| c.to_string()).collect(), origin: origine.into() }
}

#[test]
fn capacita_isolate_e_risorse_ammesse() {
    let solo_chat = condivisa(&["chat"], "mobile");
    assert!(allowed(&solo_chat, "outlook").unwrap_err().to_string().contains("denied"));
    assert!(allowed(&solo_chat, "constructor").unwrap_err().to_string().contains("denied"));
    let solo_posta = condivisa(&["mail"], "mobile");
    assert!(allowed(&solo_posta, "csa").unwrap_err().to_string().contains("denied"));
    assert_eq!(allowed(&solo_posta, "outlook").unwrap()[0], "https://outlook.office.com");
}

#[tokio::test(flavor = "current_thread")]
async fn origine_esplicita_per_ogni_provenienza() {
    for origine in ["mobile", "outlook-spa", "teams-spa"] {
        let osservata: Rc<RefCell<Option<String>>> = Rc::default();
        let o2 = osservata.clone();
        let t: Rc<dyn Trasporto> = microsoft(move |r| {
            *o2.borrow_mut() = r.intestazioni.ottieni("origin");
            Ok(token_risposta(&jwt("https://chatsvcagg.teams.microsoft.com", "object", None), None))
        });
        renew_shared(&condivisa(&["chat"], origine), "csa", "rt", "client", &t, 1000).await.unwrap();
        let attesa = match origine {
            "mobile" => None,
            "teams-spa" => Some("https://teams.microsoft.com".to_string()),
            _ => Some("https://outlook.cloud.microsoft".to_string()),
        };
        assert_eq!(*osservata.borrow(), attesa, "{origine}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn le_risorse_teams_di_outlook_mobile_si_chiedono_come_teams() {
    let chieste: Rc<RefCell<Vec<[String; 3]>>> = Rc::default();
    let c2 = chieste.clone();
    let p = Prova::nuova(microsoft(move |r| {
        let res = risorsa(r);
        c2.borrow_mut().push([res.clone(), campo(r, "client_id"), campo(r, "refresh_token")]);
        let n = c2.borrow().len();
        Ok(token_risposta(&jwt(&res, "object", Some(&campo(r, "client_id"))), Some(&format!("family{n}"))))
    }));
    p.manager.ottieni_risorsa("user", "spaces", false).await.unwrap();
    p.manager.ottieni_risorsa("user", "csa", false).await.unwrap();
    p.manager.ottieni_access_token("user", false).await.unwrap();
    let s = |x: &str| x.to_string();
    assert_eq!(
        *chieste.borrow(),
        [
            [s("https://api.spaces.skype.com"), s(TEAMS_APP_CLIENT_ID), s("initial")],
            [s("https://chatsvcagg.teams.microsoft.com"), s(TEAMS_APP_CLIENT_ID), s("family1")],
            [s("https://outlook.office.com"), s(OUTLOOK_MOBILE), s("family2")],
        ]
    );
    assert_eq!(p.su_disco()["accounts"]["user"]["refresh_token"], Json::from("family3"));
}

#[tokio::test(flavor = "current_thread")]
async fn un_token_teams_di_un_altra_app_si_richiede_uno_di_teams_si_riusa() {
    let p = Prova::nuova(microsoft(|r| Ok(token_risposta(&jwt("https://chatsvcagg.teams.microsoft.com", "object", Some(&campo(r, "client_id"))), Some("next")))));
    let vecchio = jwt("https://chatsvcagg.teams.microsoft.com", "object", Some(OUTLOOK_MOBILE));
    let con_cache = ENTRY.replacen('{', &format!(r#"{{"resourceTokens":{{"csa":{{"token":"{vecchio}","expiresAt":9000}}}},"#), 1);
    p.scrivi(&format!(r#"{{"user":{con_cache}}}"#));
    let manager = Prova::gestore(&p.file, &p.ms, None);
    let fresco = manager.ottieni_risorsa("user", "csa", false).await.unwrap();
    assert_eq!(p.ms.chiamate.borrow().len(), 1);
    assert_eq!(fresco, jwt("https://chatsvcagg.teams.microsoft.com", "object", Some(TEAMS_APP_CLIENT_ID)));
    assert_eq!(manager.ottieni_risorsa("user", "csa", false).await.unwrap(), fresco);
    assert_eq!(p.ms.chiamate.borrow().len(), 1);
}

#[test]
fn solo_un_accesso_outlook_mobile_si_scambia() {
    assert_eq!(client_for(&condivisa(&["chat"], "mobile"), "ic3", "stored"), TEAMS_APP_CLIENT_ID);
    assert_eq!(client_for(&condivisa(&["chat"], "mobile"), "outlook", "stored"), "stored");
    assert_eq!(client_for(&condivisa(&["chat"], "outlook-spa"), "csa", "stored"), "stored");
    assert_eq!(client_for(&condivisa(&["chat"], "teams-spa"), "csa", "stored"), "stored");
}

// ---- confini -----------------------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn il_vecchio_file_non_resuscita_credenziali_senza_posta_o_scadute() {
    in_locale(async {
        for capacita in [r#"["calendar"]"#, r#"["mail"]"#] {
            let p = Prova::nuova(microsoft(|_| Ok(json(400, r#"{"error":"invalid_grant"}"#))));
            let voce = ENTRY.replace(r#"["mail","calendar","chat"]"#, capacita);
            p.scrivi(&format!(r#"{{"user":{voce}}}"#));
            let vecchio = p.dir.join("legacy.json");
            std::fs::write(&vecchio, r#"{"account":[{"upn":"user","token":"must-not-leak"}]}"#).unwrap();
            let porta = p.server(Some(vecchio)).await;
            let (stato, testo) = chiedi(porta, "GET", "/api/token-posta", Some("key"), None).await;
            assert_eq!(stato, 200);
            assert_eq!(j(&testo)["account"].in_js(), "[]", "{capacita}: {testo}");
        }
    })
    .await;
}

/// La risposta OAuth di un nuovo accesso (PKCE di Outlook Mobile) per una casella già condivisa.
async fn nuovo_accesso(p: &Prova) -> Result<(String, String), oe_rete::errori::ErroreAccesso> {
    let (_, state) = p.manager.inizia_pkce(None);
    p.manager.completa_pkce(&format!("?code=c&state={state}"), None).await
}

#[tokio::test(flavor = "current_thread")]
async fn un_nuovo_accesso_controlla_l_identita_e_svuota_la_cache() {
    for oid in ["object", "other"] {
        let t = token(&format!(r#"{{"upn":"user","aud":"https://outlook.office.com","tid":"tenant","oid":"{oid}","exp":9000,"scp":"Mail.ReadWrite Calendars.ReadWrite"}}"#));
        let p = Prova::nuova(microsoft(move |_| Ok(token_risposta(&t, Some("new")))));
        let esito = nuovo_accesso(&p).await;
        if oid == "other" {
            assert!(esito.unwrap_err().to_string().contains("mismatch"));
        } else {
            esito.unwrap();
            assert_eq!(p.voce("user")["shared"]["capabilities"].in_js(), r#"["mail","calendar","chat"]"#);
            assert!(p.voce("user").get("resourceTokens").is_none());
        }
        assert_eq!(p.voce("user")["refresh_token"], Json::from(if oid == "other" { "initial" } else { "new" }), "{oid}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn un_nuovo_accesso_non_riaccende_i_servizi_spenti() {
    let t = token(r#"{"upn":"user","tid":"tenant","oid":"object","aud":"https://outlook.office.com","exp":9000,"scp":"Mail.ReadWrite Calendars.ReadWrite"}"#);
    let p = Prova::nuova(microsoft(move |_| Ok(token_risposta(&t, Some("new")))));
    let spenta = ENTRY.replace(r#"["mail","calendar","chat"]"#, "[]");
    p.scrivi(&format!(r#"{{"user":{spenta}}}"#));
    nuovo_accesso(&p).await.unwrap();
    assert_eq!(p.voce("user")["shared"]["capabilities"].in_js(), "[]");
}

/// La casella condivisa del client web con la sua sessione del browser.
fn con_sessione(refresh: bool, cookie: Option<&str>) -> String {
    let cookies = cookie.map(|n| format!(r#"{{"name":"{n}","value":"cookie","domain":"login.microsoftonline.com","path":"/","expires":-1}}"#)).unwrap_or_default();
    let refresh = if refresh { r#""refresh_token":"initial","# } else { "" };
    format!(
        r#"{{"user@example.com":{{"provider":"microsoft",{refresh}"shared":{{"tenantId":"tenant","objectId":"object","capabilities":["mail","calendar","chat"],"origin":"outlook-spa"}},"sessione":{{"userAgent":"test","loginHint":"user@example.com","cookies":[{cookies}]}}}}}}"#
    )
}

/// Il codice dell'autorizzazione silenziosa torna a Outlook web con lo state chiesto.
fn autorizza(r: &Richiesta) -> Option<Risposta> {
    let u = url::Url::parse(&r.url).ok()?;
    if !u.path().ends_with("/authorize") {
        return None;
    }
    let state = u.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.into_owned()).unwrap_or_default();
    Some(Risposta::nuova(302, "").con_intestazione("Location", format!("https://outlook.cloud.microsoft/mail/?code=fixture&state={state}")))
}

#[tokio::test(flavor = "current_thread")]
async fn la_sessione_del_browser_scaduta_si_segna_per_la_guida() {
    let p = Prova::nuova(microsoft(|_| Ok(json(400, r#"{"error":"invalid_grant"}"#))));
    p.scrivi(&con_sessione(true, None));
    assert!(p.manager.ottieni_risorsa("user@example.com", "outlook", false).await.is_err());
    assert!(p.voce("user@example.com")["errore"].as_str().unwrap_or_default().contains("Sessione del browser"), "{}", p.voce("user@example.com").in_js());
}

#[tokio::test(flavor = "current_thread")]
async fn il_refresh_token_che_manca_si_recupera_dalla_sessione() {
    let t = token(r#"{"upn":"user@example.com","tid":"tenant","oid":"object","aud":"https://outlook.office.com","exp":9000,"scp":"Mail.ReadWrite Calendars.ReadWrite"}"#);
    let t2 = t.clone();
    let p = Prova::nuova(microsoft(move |r| Ok(autorizza(r).unwrap_or_else(|| token_risposta(&t2, Some("recovered"))))));
    p.scrivi(&con_sessione(false, Some("ESTSAUTH")));
    assert_eq!(p.manager.ottieni_risorsa("user@example.com", "outlook", false).await.unwrap(), t);
}

#[tokio::test(flavor = "current_thread")]
async fn identita_controllata_nel_recupero_e_nell_importazione() {
    for (oid, importa) in [("object", false), ("other", false), ("object", true), ("other", true)] {
        let t = token(&format!(r#"{{"upn":"user@example.com","tid":"tenant","oid":"{oid}","aud":"https://outlook.office.com","exp":9000,"scp":"Mail.ReadWrite Calendars.ReadWrite"}}"#));
        let t2 = t.clone();
        let p = Prova::nuova(microsoft(move |r| {
            if let Some(a) = autorizza(r) {
                return Ok(a);
            }
            if campo(r, "grant_type") == "refresh_token" {
                return Ok(json(400, r#"{"error":"invalid_grant"}"#));
            }
            Ok(token_risposta(&t2, Some("recovered")))
        }));
        p.scrivi(&con_sessione(true, Some("ESTSAUTHPERSISTENT")));
        let sessione = p.su_disco()["accounts"]["user@example.com"]["sessione"].clone();
        let esito = if importa {
            p.manager.importa_sessione(&sessione, "tenant", None, &["calendar"]).await.map(|(a, t)| format!("{a} {t}"))
        } else {
            p.manager.ottieni_risorsa("user@example.com", "outlook", false).await
        };
        let caso = format!("{oid} importa={importa}");
        if oid == "other" {
            assert!(esito.unwrap_err().to_string().contains("mismatch"), "{caso}");
        } else {
            assert_eq!(esito.unwrap(), if importa { "user@example.com tenant".to_string() } else { t.clone() }, "{caso}");
            let v = p.voce("user@example.com");
            assert_eq!(v["shared"]["capabilities"].in_js(), r#"["mail","calendar","chat"]"#, "{caso}");
            assert!(v.get("resourceTokens").and_then(|r| r.get("csa")).is_none(), "{caso}");
        }
        assert_eq!(p.voce("user@example.com")["refresh_token"], Json::from(if oid == "object" { "recovered" } else { "initial" }), "{caso}");
        let chiamate = p.ms.chiamate.borrow().clone();
        assert_eq!(chiamate.len(), if importa { 2 } else { 3 }, "{caso}");
        assert!(chiamate.iter().all(|u| u.starts_with("https://login.microsoftonline.com/")), "{caso}");
    }
}

// ---- cache ed esportazione ----------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn un_token_outlook_in_cache_senza_i_permessi_chiesti_non_si_riusa() {
    let p = Prova::nuova(microsoft(|_| Ok(token_risposta(&jwt("https://outlook.office.com", "object", None), None))));
    let invalido = token(r#"{"aud":"https://outlook.office.com","tid":"tenant","oid":"object","exp":9000,"scp":"Mail.Read"}"#);
    let con_cache = ENTRY.replacen('{', &format!(r#"{{"resourceTokens":{{"outlook":{{"token":"{invalido}","expiresAt":9000}}}},"#), 1);
    p.scrivi(&format!(r#"{{"user":{con_cache}}}"#));
    assert_ne!(p.manager.ottieni_risorsa("user", "outlook", false).await.unwrap(), invalido);
    assert_eq!(p.ms.chiamate.borrow().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn il_token_di_una_casella_solo_calendario_non_va_nel_file_della_posta() {
    let p = Prova::nuova(microsoft(|_| Ok(token_risposta(&jwt("https://outlook.office.com", "object", None), None))));
    let uscita = p.dir.join("mail.json");
    let solo_calendario = ENTRY.replace(r#"["mail","calendar","chat"]"#, r#"["calendar"]"#);
    p.scrivi(&format!(r#"{{"user":{solo_calendario}}}"#));
    let manager = Prova::gestore(&p.file, &p.ms, Some(uscita.clone()));
    manager.ottieni_risorsa("user", "outlook", false).await.unwrap();
    assert_eq!(j(&std::fs::read_to_string(&uscita).unwrap())["account"].in_js(), "[]");
}

// ---- la rotta di attivazione ----------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn elenca_spegne_e_riaccende_la_condivisione_via_http_svuotando_la_cache() {
    in_locale(async {
        const OID: &str = "11111111-1111-4111-8111-111111111111";
        const TID: &str = "22222222-2222-4222-8222-222222222222";
        let p = Prova::nuova(microsoft(|r| {
            let t = token(&format!(r#"{{"aud":"{}","oid":"{OID}","tid":"{TID}","exp":9000,"upn":"user","scp":"Mail.ReadWrite Calendars.ReadWrite"}}"#, risorsa(r)));
            Ok(token_risposta(&t, Some("next")))
        }));
        let voce = ENTRY.replace(r#""tenantId":"tenant","objectId":"object""#, &format!(r#""tenantId":"{TID}","objectId":"{OID}""#));
        p.scrivi(&format!(r#"{{"user":{voce}}}"#));
        let porta = p.server(None).await;
        let base = "/api/auth/shared";
        assert_eq!(chiedi(porta, "GET", base, None, None).await.0, 401);
        assert_eq!(chiedi(porta, "POST", base, Some("key"), Some("{")).await.0, 400);
        p.manager.ottieni_risorsa("user", "csa", false).await.unwrap();
        assert!(p.su_disco()["accounts"]["user"].get("resourceTokens").is_some());
        let spenta = j(&chiedi(porta, "POST", base, Some("key"), Some(r#"{"account":"user","enabled":false}"#)).await.1);
        assert_eq!(spenta["ok"], Json::Bool(true));
        let salvata = p.su_disco()["accounts"]["user"].clone();
        assert!(salvata.get("shared").is_none() && salvata.get("resourceTokens").is_none(), "{}", salvata.in_js());
        let elenco = j(&chiedi(porta, "GET", base, Some("key"), None).await.1);
        let user = elenco["accounts"].as_arr().unwrap().iter().find(|a| a["account"].as_str() == Some("user")).cloned().unwrap();
        assert_eq!(user["shared"], Json::Null);
        let accesa = j(&chiedi(porta, "POST", base, Some("key"), Some(r#"{"account":"user","capabilities":["mail","chat"],"tenantId":"evil","objectId":"evil"}"#)).await.1);
        assert!(accesa.get("errore").is_none() && accesa["ok"] == Json::Bool(true), "{}", accesa.in_js());
        let di_nuovo = p.su_disco()["accounts"]["user"]["shared"].clone();
        assert_eq!((di_nuovo["tenantId"].clone(), di_nuovo["objectId"].clone(), di_nuovo["capabilities"].in_js()), (Json::from(TID), Json::from(OID), r#"["mail","chat"]"#.to_string()));
        let falsa = j(&chiedi(porta, "POST", base, Some("key"), Some(r#"{"account":"user","capabilities":["bogus"]}"#)).await.1);
        assert_eq!(falsa["ok"], Json::Bool(false));
    })
    .await;
}
