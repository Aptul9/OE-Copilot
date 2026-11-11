//! Le prove di `agente-ts/test/google-communication.test.ts`: l'agente tiene le credenziali di Gmail e Chat. Da capo a fondo su HTTP vero:
//! il cliente parla all'agente, l'agente con il trasporto vero (reqwest) a un Google finto su 127.0.0.1; il ponte sposta ogni origine su
//! quella del finto e lascia percorso e query.

mod comune;

use comune::chiedi;
use oe_agente::google_communication::GoogleCommunication;
use oe_agente::http::{self, Esito, Gestore, servi};
use oe_agente::server::{Agente, OpzioniAgente};
use oe_nucleo::json::Json;
use oe_rete::google_web::{CookieGoogle, SessioneGoogleWeb};
use oe_rete::protobuf::codifica_chat;
use oe_rete::trasporto::prova::TrasportoFinto;
use oe_rete::trasporto::{FutureRete, Richiesta, Risposta, Trasporto, TrasportoReqwest};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use tokio::net::TcpListener;
use tokio::task::LocalSet;

fn cartella(nome: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("{nome}-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

fn risposta(codice: u16, corpo: Vec<u8>, tipo: Option<&str>) -> Esito {
    let mut h = vec![("Content-Length".to_string(), corpo.len().to_string()), ("Connection".to_string(), "close".to_string())];
    if let Some(t) = tipo {
        h.push(("Content-Type".to_string(), t.to_string()));
    }
    Esito::Risposta(http::Risposta { codice, motivo: oe_rete::trasporto::testo_stato(codice).to_string(), intestazioni: h, corpo, scarta_corpo: false })
}

/// Il Google finto: OAuth, Gmail e le RPC di Chat che le prove usano.
#[derive(Default)]
struct Google {
    visti: RefCell<Vec<String>>,
    autorizzazioni_profilo: RefCell<Vec<Option<String>>>,
}

impl Gestore for Google {
    async fn gestisci(&self, r: &mut http::Richiesta<'_>) -> Esito {
        let _ = r.leggi_corpo().await;
        let p = url::Url::parse(&format!("http://fixture{}", r.url)).unwrap().path().to_string();
        self.visti.borrow_mut().push(format!("{} {p}", r.metodo));
        let json = |t: &str| risposta(200, t.as_bytes().to_vec(), Some("application/json"));
        let proto = |tipo: &str, v: &str| risposta(200, codifica_chat(tipo, &j(v)).unwrap(), None);
        if p == "/token" {
            return json(r#"{"access_token":"mail-private-token","expires_in":3600}"#);
        }
        if p.ends_with("/profile") {
            self.autorizzazioni_profilo.borrow_mut().push(r.intestazione("authorization").map(str::to_string));
            return json(r#"{"emailAddress":"me@example.com"}"#);
        }
        if p.ends_with("/messages") {
            return json(r#"{"messages":[{"id":"m1"}],"nextPageToken":"page2"}"#);
        }
        if p.ends_with("/mole/world") {
            return risposta(200, br#"<script>window.WIZ_global_data = {"qwAQke":"ChatUi","SMqcke":"xsrf"};</script>"#.to_vec(), None);
        }
        if p.ends_with("/get_self_user_status") {
            return proto("GetSelfUserStatusResponse", r#"{"user_status":{"user_id":{"id":"123"}}}"#);
        }
        if p.ends_with("/get_members") {
            return proto("GetMembersResponse", r#"{"members":[{"user":{"user_id":{"id":"123"},"name":"Me","email":"me@example.com"}}]}"#);
        }
        if p.ends_with("/paginated_world") {
            return proto("PaginatedWorldResponse", r#"{"world_items":[]}"#);
        }
        if p.ends_with("/create_topic") {
            return proto("CreateTopicResponse", r#"{"topic":{"id":{"topic_id":"new"}}}"#);
        }
        risposta(404, b"{}".to_vec(), None)
    }
}

/// Ogni indirizzo va al Google finto con lo stesso percorso e la stessa query.
struct Ponte {
    vero: TrasportoReqwest,
    base: String,
}

impl Trasporto for Ponte {
    fn invia(&self, mut r: Richiesta) -> FutureRete<'_> {
        let u = url::Url::parse(&r.url).unwrap();
        r.url = format!("{}{}{}", self.base, u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default());
        self.vero.invia(r)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn l_agente_tiene_le_credenziali_importa_legge_conferma_e_lascia_i_calendari() {
    LocalSet::new()
        .run_until(async {
            let dir = cartella("oe-google-http");
            let google = Rc::new(Google::default());
            let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://127.0.0.1:{}", ascolto.local_addr().unwrap().port());
            tokio::task::spawn_local(servi(ascolto, google.clone()));
            let ponte: Rc<dyn Trasporto> = Rc::new(Ponte { vero: TrasportoReqwest::nuovo().unwrap(), base });
            let mut o = OpzioniAgente::nuove("key", Vec::new(), dir.join("missing"), "Europe/Rome");
            o.registro = Rc::new(|_, _| {});
            o.google = Some(Rc::new(GoogleCommunication::nuovo(&dir, ponte, None)));
            let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let porta = ascolto.local_addr().unwrap().port();
            tokio::task::spawn_local(servi(ascolto, Rc::new(Agente::nuovo(o))));
            let post = |percorso: &'static str, corpo: &'static str, chiave: &'static str| async move {
                let (stato, testo) = chiedi(porta, "POST", percorso, Some(chiave), Some(corpo)).await;
                (stato, Json::analizza(&testo).unwrap_or(Json::Null))
            };

            assert_eq!(post("/api/auth/completa", r#"{"provider":"gmail"}"#, "wrong").await.0, 401);
            assert!(google.visti.borrow().is_empty());
            let gmail = post("/api/auth/completa", r#"{"provider":"gmail","account":"me@example.com","credentials":{"client_id":"client","refresh_token":"mail-refresh"}}"#, "key").await;
            assert_eq!((gmail.0, gmail.1.in_js()), (200, r#"{"ok":true,"account":"me@example.com","provider":"gmail"}"#.to_string()));
            assert_eq!(*google.autorizzazioni_profilo.borrow(), [Some("Bearer mail-private-token".to_string())]);
            let chat = post(
                "/api/auth/completa",
                r#"{"provider":"gchat","account":"me@example.com","sessione":{"user_agent":"test","cookies":[{"name":"SID","value":"private-cookie","domain":".google.com","path":"/","expires":0}]}}"#,
                "key",
            )
            .await;
            assert_eq!((chat.0, chat.1["ok"].clone(), chat.1["provider"].clone()), (200, Json::Bool(true), Json::from("gchat")));

            let (stato, testo) = chiedi(porta, "GET", "/api/communication/google/accounts", Some("key"), None).await;
            assert_eq!((stato, j(&testo)["accounts"].as_arr().map(Vec::len)), (200, Some(2)), "{testo}");
            for segreto in ["private", "refresh", "cookie", "xsrf"] {
                assert!(!testo.contains(segreto), "{segreto} in {testo}");
            }
            assert_eq!(post("/api/communication/gmail/read", r#"{"account":"me@example.com","path":"messages?maxResults=1"}"#, "key").await.1["nextPageToken"], Json::from("page2"));
            assert_eq!(post("/api/communication/gchat/read", r#"{"account":"me@example.com","action":"list"}"#, "key").await.1["self_id"], Json::from("123"));
            let senza_conferma = post("/api/communication/gchat/write", r#"{"account":"me@example.com","action":"send","kind":"space","conversation":"space","text":"private"}"#, "key").await;
            assert_eq!(senza_conferma.0, 400);
            assert!(!google.visti.borrow().iter().any(|v| v.ends_with("/create_topic")));
            let inviato = post("/api/communication/gchat/write", r#"{"account":"me@example.com","action":"send","kind":"space","conversation":"space","text":"private","confirm":true}"#, "key").await;
            assert_eq!(inviato.1["topic"]["id"]["topic_id"], Json::from("new"), "{}", inviato.1.in_js());

            let (_, calendari) = chiedi(porta, "GET", "/api/auth/stato", Some("key"), None).await;
            assert_eq!(calendari, r#"{"caselle": []}"#);
            assert!(!dir.join("tokens.json").exists());
            assert_eq!(post("/api/auth/rimuovi", r#"{"provider":"gchat","account":"me@example.com"}"#, "key").await.1["ok"], Json::Bool(true));
            assert_eq!(post("/api/communication/gchat/read", r#"{"account":"me@example.com","action":"list"}"#, "key").await.0, 404);
            let _ = std::fs::remove_dir_all(&dir);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn un_accesso_chat_rifiutato_si_segna_senza_cancellare_la_sessione() {
    let dir = cartella("oe-google-expired");
    let finto: Rc<dyn Trasporto> = TrasportoFinto::con_funzione(|r| {
        Ok(if r.url.contains("RotateCookies") { Risposta::nuova(401, "") } else { Risposta::nuova(200, r#"<script>window.WIZ_global_data = {"qwAQke":"AccountsSignInUi"};</script>"#) })
    });
    let service = GoogleCommunication::nuovo(&dir, finto, None);
    let sessione = SessioneGoogleWeb {
        account: "me@example.com".into(),
        user_agent: "test".into(),
        cookies: vec![CookieGoogle { name: "SID".into(), value: "expired".into(), domain: ".google.com".into(), path: "/".into(), expires: 0.0 }],
        self_id: Some("123".into()),
        salvata: 0.0,
        errore: None,
    };
    service.sessions.metti(Rc::new(RefCell::new(sessione))).unwrap();
    let e = service.read_chat(j(r#"{"account":"me@example.com","action":"list"}"#).as_obj().unwrap()).await.unwrap_err();
    assert!(e.to_string().contains("expired"), "{e}");
    assert_eq!(
        Json::Arr(service.accounts().unwrap()).in_js(),
        r#"[{"account":"me@example.com","provider":"gchat","self_id":"123","status":"errore","message":"Google Chat session import required"}]"#
    );
    assert_eq!(service.sessions.sessione("me@example.com").unwrap().unwrap().borrow().cookies.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
