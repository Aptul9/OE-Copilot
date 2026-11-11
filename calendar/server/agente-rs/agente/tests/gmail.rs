//! Le prove di `agente-ts/test/gmail.test.ts`: il gateway Gmail con le sue credenziali, i percorsi di lettura ammessi, le scritture
//! confermate, l'accesso PKCE e l'importazione verificati, i limiti sulle risposte.

use oe_agente::gmail::GmailGateway;
use oe_nucleo::json::Json;
use oe_rete::google_web::ErroreGoogle;
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::{EsitoRete, FutureRete, Richiesta, Risposta, Trasporto};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use tokio::sync::Notify;

const ACCOUNT: &str = "me@example.com";

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

fn campo(r: &Richiesta, k: &str) -> Option<String> {
    let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
    form_urlencoded::parse(corpo.as_bytes()).find(|(n, _)| n == k).map(|(_, v)| v.into_owned())
}

fn corpo_json(r: &Richiesta) -> Json {
    r.corpo.as_deref().map_or(Json::Null, |c| j(&String::from_utf8_lossy(c)))
}

/// Google simulato: la funzione risponde; con il cancello il rinnovo del refresh_token "refresh" aspetta finché non lo si rilascia.
struct Google {
    finto: Rc<TrasportoFinto>,
    cancello: Option<(Rc<Notify>, Rc<Notify>)>,
}

impl Trasporto for Google {
    fn invia(&self, r: Richiesta) -> FutureRete<'_> {
        Box::pin(async move {
            if let Some((iniziato, rilascia)) = &self.cancello {
                if r.url.contains("oauth2.googleapis.com/token") && campo(&r, "refresh_token").as_deref() == Some("refresh") {
                    iniziato.notify_one();
                    rilascia.notified().await;
                }
            }
            self.finto.invia(r).await
        })
    }
}

struct Prova {
    dir: PathBuf,
    file: PathBuf,
    service: GmailGateway,
    finto: Rc<TrasportoFinto>,
}

impl Drop for Prova {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn prova_con(f: impl FnMut(&Richiesta) -> EsitoRete + 'static, configurato: bool, cancello: Option<(Rc<Notify>, Rc<Notify>)>) -> Prova {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("oe-gmail-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gmail.json");
    let scade = oe_agente::fonte::adesso_vero() / 1000 + 3600;
    let accounts = if configurato { format!(r#"{{"{ACCOUNT}":{{"provider":"google","client_id":"client","refresh_token":"refresh","access_token":"old","expires_at":{scade}}}}}"#) } else { "{}".to_string() };
    std::fs::write(&file, format!(r#"{{"accounts":{accounts}}}"#)).unwrap();
    let finto = TrasportoFinto::con_funzione(f);
    let t: Rc<dyn Trasporto> = Rc::new(Google { finto: finto.clone(), cancello });
    Prova { service: GmailGateway::nuovo(file.clone(), t, None), dir, file, finto }
}

fn prova(f: impl FnMut(&Richiesta) -> EsitoRete + 'static) -> Prova {
    prova_con(f, true, None)
}

fn su_disco(p: &Prova) -> Json {
    j(&std::fs::read_to_string(&p.file).unwrap())["accounts"][ACCOUNT].clone()
}

fn auth(r: &Richiesta) -> Option<String> {
    r.intestazioni.ottieni("authorization")
}

fn messaggio(e: &ErroreGoogle) -> String {
    e.to_string()
}

fn base64url(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

#[tokio::test(flavor = "current_thread")]
async fn tiene_paginazione_e_mime_e_le_letture_non_cambiano_etichette() {
    let p = prova(|r| Ok(json(200, if r.url.contains("pageToken=") { r#"{"messages":[{"id":"m2"}]}"# } else { r#"{"messages":[{"id":"m1"}],"nextPageToken":"opaque+page/="}"# })));
    let r = p.service.read(&j(&format!(r#"{{"account":"{ACCOUNT}","path":"messages?maxResults=1&q=is%3Aunread"}}"#))).await.unwrap();
    assert_eq!(r["nextPageToken"], Json::from("opaque+page/="));
    let r = p.service.read(&j(&format!(r#"{{"account":"{ACCOUNT}","path":"messages?maxResults=1&pageToken=opaque%2Bpage%2F%3D"}}"#))).await.unwrap();
    assert_eq!(r["messages"].in_js(), r#"[{"id":"m2"}]"#);
    assert!(p.finto.richieste().iter().all(|r| r.metodo == "GET" && auth(r).as_deref() == Some("Bearer old")));
    let elenco = Json::Arr(p.service.accounts().unwrap());
    assert_eq!(elenco.in_js(), format!(r#"[{{"account":"{ACCOUNT}","provider":"gmail","status":"ok","message":""}}]"#));
    assert!(!["refresh", "Bearer", "client"].iter().any(|s| elenco.in_js().contains(s)));
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_url_impostazioni_altri_utenti_e_limiti_prima_della_rete() {
    let p = prova(|_| Ok(json(200, "{}")));
    for percorso in ["https://evil.test/", "users/victim/messages", "../messages", "messages/%2e%2e/profile", "settings/filters", "messages?maxResults=99999", "messages?bad=x", "messages?format=evil", "messages?maxResults=1&maxResults=2"] {
        let corpo = oe_nucleo::ogg! { "account": ACCOUNT, "path": percorso };
        assert!(p.service.read(&corpo).await.is_err(), "{percorso}");
    }
    assert!(p.service.read(&j(r#"{"account":"other@example.com","path":"profile"}"#)).await.is_err());
    assert_eq!(p.finto.chiamate(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn rinnova_una_volta_su_401_salva_il_refresh_ruotato_e_da_la_risposta() {
    let rinnovi = Rc::new(Cell::new(0));
    let letture = Rc::new(Cell::new(0));
    let (r2, l2) = (rinnovi.clone(), letture.clone());
    let p = prova(move |r| {
        if r.url.contains("oauth2.googleapis.com/token") {
            r2.set(r2.get() + 1);
            assert_eq!(campo(r, "refresh_token").as_deref(), Some("refresh"));
            return Ok(json(200, r#"{"access_token":"fresh","refresh_token":"rotated","expires_in":3600}"#));
        }
        l2.set(l2.get() + 1);
        Ok(if auth(r).as_deref() == Some("Bearer old") { json(401, "{}") } else { json(200, r#"{"id":"m1","payload":{"mimeType":"text/plain","body":{"data":"aGVsbG8"}}}"#) })
    });
    let r = p.service.read(&j(&format!(r#"{{"account":"{ACCOUNT}","path":"messages/m1?format=full"}}"#))).await.unwrap();
    assert_eq!(r["payload"]["body"]["data"], Json::from("aGVsbG8"));
    assert_eq!((rinnovi.get(), letture.get()), (1, 2));
    assert_eq!(su_disco(&p)["refresh_token"], Json::from("rotated"));
}

#[tokio::test(flavor = "current_thread")]
async fn l_importazione_verifica_l_identita_prima_di_salvare() {
    let indirizzo = Rc::new(RefCell::new("wrong@example.com".to_string()));
    let i2 = indirizzo.clone();
    let p = prova(move |r| {
        if r.url.contains("oauth2.googleapis.com/token") {
            return Ok(json(200, r#"{"access_token":"new-token","expires_in":3600}"#));
        }
        Ok(json(200, &oe_nucleo::ogg! { "emailAddress": i2.borrow().as_str() }.in_js()))
    });
    let prima = std::fs::read_to_string(&p.file).unwrap();
    let corpo = j(&format!(r#"{{"account":"{ACCOUNT}","credentials":{{"client_id":"client","client_secret":"secret","refresh_token":"new-refresh"}}}}"#));
    let e = p.service.import(&corpo).await.unwrap_err();
    assert!(messaggio(&e).to_lowercase().contains("account"), "{e}");
    assert_eq!(std::fs::read_to_string(&p.file).unwrap(), prima);
    *indirizzo.borrow_mut() = ACCOUNT.to_string();
    assert_eq!(p.service.import(&corpo).await.unwrap(), ACCOUNT);
    assert_eq!(su_disco(&p)["refresh_token"], Json::from("new-refresh"));
}

#[tokio::test(flavor = "current_thread")]
async fn pkce_con_state_esplicito_e_solo_permessi_della_posta() {
    let p = prova_con(
        |r| Ok(json(200, if r.url.contains("oauth2.googleapis.com/token") { r#"{"access_token":"new","refresh_token":"r","expires_in":3600}"# } else { r#"{"emailAddress":"me@example.com"}"# })),
        false,
        None,
    );
    let login = p.service.begin(Some(ACCOUNT)).unwrap();
    let u = url::Url::parse(&login.url).unwrap();
    let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default();
    assert_eq!(q("code_challenge_method"), "S256");
    assert!(q("scope").contains("gmail.modify") && !q("scope").contains("calendar"));
    let e = p.service.complete("http://localhost/?code=auth&state=wrong", Some(&login.state)).await.unwrap_err();
    assert!(messaggio(&e).contains("state"), "{e}");
    assert_eq!(p.service.complete(&format!("http://localhost/?code=auth&state={}", login.state), Some(&login.state)).await.unwrap(), ACCOUNT);
}

#[tokio::test(flavor = "current_thread")]
async fn chiede_conferma_controlla_il_mime_e_non_ripete_un_invio_ambiguo() {
    let p = prova(|_| Ok(json(503, r#"{"secret":"private upstream"}"#)));
    let raw = base64url("To: self@example.com\r\nSubject: test\r\n\r\nbody");
    let e = p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send", "raw": raw.as_str() }).await.unwrap_err();
    assert!(messaggio(&e).contains("confirm"));
    let e = p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send", "raw": "invalid!", "confirm": true }).await.unwrap_err();
    assert!(messaggio(&e).contains("raw"));
    assert_eq!(p.finto.chiamate(), 0);
    let e = p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send", "raw": raw.as_str(), "confirm": true }).await.unwrap_err();
    assert_eq!(messaggio(&e), "Gmail HTTP 503");
    assert_eq!(p.finto.chiamate(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn bozza_etichette_cestino_e_ripristino_niente_cancellazione_definitiva() {
    let p = prova(|_| Ok(json(200, r#"{"id":"own-draft"}"#)));
    let raw = base64url("Subject: private test\r\n\r\ntest");
    p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "draft", "raw": raw.as_str(), "confirm": true }).await.unwrap();
    p.service.write(&j(&format!(r#"{{"account":"{ACCOUNT}","action":"modify","message_id":"m1","add_labels":["STARRED"],"remove_labels":["UNREAD"],"confirm":true}}"#))).await.unwrap();
    for azione in ["trash", "untrash"] {
        p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": azione, "message_id": "m1", "confirm": true }).await.unwrap();
    }
    assert!(p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "delete", "message_id": "m1", "confirm": true }).await.is_err());
    let c = p.finto.richieste();
    assert_eq!(c.len(), 4);
    assert_eq!(corpo_json(&c[0]).in_js(), format!(r#"{{"message":{{"raw":"{raw}"}}}}"#));
    assert_eq!(corpo_json(&c[1]).in_js(), r#"{"addLabelIds":["STARRED"],"removeLabelIds":["UNREAD"]}"#);
}

#[tokio::test(flavor = "current_thread")]
async fn sostituisce_e_invia_una_bozza_rifiuta_campi_in_piu_e_id_cattivi() {
    let p = prova(|_| Ok(json(200, r#"{"id":"d1","message":{"id":"m1","threadId":"t1"}}"#)));
    let raw = base64url("Subject: draft\r\n\r\nv2");
    p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "update_draft", "draft_id": "d1", "raw": raw.as_str(), "thread_id": "t1", "confirm": true }).await.unwrap();
    p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send_draft", "draft_id": "d1", "confirm": true }).await.unwrap();
    let c = p.finto.richieste();
    assert_eq!((c[0].metodo.as_str(), corpo_json(&c[0]).in_js()), ("PUT", format!(r#"{{"id":"d1","message":{{"raw":"{raw}","threadId":"t1"}}}}"#)));
    assert!(c[0].url.ends_with("/drafts/d1"));
    assert_eq!((c[1].metodo.as_str(), corpo_json(&c[1]).in_js()), ("POST", r#"{"id":"d1"}"#.to_string()));
    assert!(c[1].url.ends_with("/drafts/send"));
    let errore = |e: Result<Json, ErroreGoogle>| messaggio(&e.unwrap_err());
    assert!(errore(p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send_draft", "draft_id": "../x", "confirm": true }).await).contains("id"));
    assert!(errore(p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send_draft", "draft_id": "d1", "raw": raw.as_str(), "confirm": true }).await).contains("unexpected"));
    assert!(errore(p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send", "draft_id": "d1", "raw": raw.as_str(), "confirm": true }).await).contains("unexpected"));
    assert!(errore(p.service.write(&oe_nucleo::ogg! { "account": ACCOUNT, "action": "send_draft", "draft_id": "d1" }).await).contains("confirm"));
    assert_eq!(p.finto.chiamate(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_redirect_e_risposte_troppo_grandi_senza_mostrare_corpo_o_token() {
    let grande = Rc::new(Cell::new(false));
    let g2 = grande.clone();
    let p = prova(move |_| {
        Ok(if g2.get() { Risposta::nuova(200, "huge").con_intestazione("Content-Length", "999999999") } else { Risposta::nuova(302, "").con_intestazione("Location", "https://evil.test/") })
    });
    let corpo = j(&format!(r#"{{"account":"{ACCOUNT}","path":"profile"}}"#));
    assert_eq!(messaggio(&p.service.read(&corpo).await.unwrap_err()), "Gmail HTTP 302");
    grande.set(true);
    let e = messaggio(&p.service.read(&corpo).await.unwrap_err());
    assert!(["size", "byte", "limit"].iter().any(|s| e.contains(s)), "{e}");
}

#[tokio::test(flavor = "current_thread")]
async fn un_rinnovo_vecchio_in_volo_non_sovrascrive_un_accesso_appena_importato() {
    let (iniziato, rilascia) = (Rc::new(Notify::new()), Rc::new(Notify::new()));
    let p = prova_con(
        |r| {
            if r.url.contains("oauth2.googleapis.com/token") {
                if campo(r, "refresh_token").as_deref() == Some("refresh") {
                    return Ok(json(200, r#"{"access_token":"stale-access","refresh_token":"stale-refresh","expires_in":3600}"#));
                }
                return Ok(json(200, r#"{"access_token":"new-access","expires_in":3600}"#));
            }
            Ok(if auth(r).as_deref() == Some("Bearer old") { json(401, "{}") } else { json(200, &format!(r#"{{"emailAddress":"{ACCOUNT}"}}"#)) })
        },
        true,
        Some((iniziato.clone(), rilascia.clone())),
    );
    let lettura = j(&format!(r#"{{"account":"{ACCOUNT}","path":"profile"}}"#));
    let importazione = j(&format!(r#"{{"account":"{ACCOUNT}","credentials":{{"client_id":"new-client","refresh_token":"new-grant"}}}}"#));
    let (letto, ()) = tokio::join!(p.service.read(&lettura), async {
        iniziato.notified().await;
        p.service.import(&importazione).await.unwrap();
        rilascia.notify_one();
    });
    assert!(letto.is_err());
    assert_eq!(su_disco(&p)["refresh_token"], Json::from("new-grant"));
}

#[tokio::test(flavor = "current_thread")]
async fn la_risposta_del_rinnovo_ha_un_limite_prima_di_salvarla_o_usarla() {
    let letture = Rc::new(Cell::new(0));
    let l2 = letture.clone();
    let imbottito = format!(r#"{{"access_token":"oversized","expires_in":3600,"padding":"{}"}}"#, "x".repeat(65536));
    let p = prova(move |r| {
        if r.url.contains("oauth2.googleapis.com/token") {
            return Ok(json(200, &imbottito));
        }
        l2.set(l2.get() + 1);
        Ok(if l2.get() == 1 { json(401, "{}") } else { json(200, &format!(r#"{{"emailAddress":"{ACCOUNT}"}}"#)) })
    });
    let prima = std::fs::read_to_string(&p.file).unwrap();
    let e = p.service.read(&j(&format!(r#"{{"account":"{ACCOUNT}","path":"profile"}}"#))).await.unwrap_err();
    assert_eq!(e.status(), 502);
    assert_eq!(letture.get(), 1);
    assert_eq!(std::fs::read_to_string(&p.file).unwrap(), prima);
}

#[tokio::test(flavor = "current_thread")]
async fn lo_stesso_limite_vale_per_l_importazione_delle_credenziali() {
    let profili = Rc::new(Cell::new(0));
    let p2 = profili.clone();
    let imbottito = format!(r#"{{"access_token":"oversized","expires_in":3600,"padding":"{}"}}"#, "x".repeat(65536));
    let p = prova_con(
        move |r| {
            if r.url.contains("oauth2.googleapis.com/token") {
                return Ok(json(200, &imbottito));
            }
            p2.set(p2.get() + 1);
            Ok(json(200, &format!(r#"{{"emailAddress":"{ACCOUNT}"}}"#)))
        },
        false,
        None,
    );
    let e = p.service.import(&j(&format!(r#"{{"account":"{ACCOUNT}","credentials":{{"client_id":"client","refresh_token":"grant"}}}}"#))).await.unwrap_err();
    assert_eq!(e.status(), 502);
    assert_eq!(profili.get(), 0);
}
