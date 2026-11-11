//! La parte "Chat binary RPC" di `agente-ts/test/gchat.test.ts`: il client delle RPC binarie di Google Chat contro un Google finto.

use oe_nucleo::json::{Json, Mappa};
use oe_rete::gchat::{GoogleChat, OpzioniChat};
use oe_rete::google_web::{CookieGoogle, ErroreGoogle, SessioneCondivisa, SessioneGoogleWeb};
use oe_rete::protobuf::{codifica_chat, decodifica_chat};
use oe_rete::trasporto::prova::TrasportoFinto;
use oe_rete::trasporto::{EsitoRete, FutureRete, Richiesta, Risposta, Trasporto};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const NOW: i64 = 1_790_000_000;

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

fn cookie(name: &str, value: &str, domain: &str, expires: f64) -> CookieGoogle {
    CookieGoogle { name: name.into(), value: value.into(), domain: domain.into(), path: "/".into(), expires }
}

fn sessione() -> SessioneCondivisa {
    Rc::new(RefCell::new(SessioneGoogleWeb {
        account: "me@example.com".into(),
        user_agent: "test-browser".into(),
        cookies: vec![cookie("SID", "domain", ".google.com", (NOW + 3600) as f64), cookie("OSID", "chat-only", "chat.google.com", 0.0)],
        self_id: None,
        salvata: NOW as f64,
        errore: None,
    }))
}

/// La sessione legata all'account "123" e con cookie di sessione, come `s.self_id = "123"; s.cookies.forEach(c => { c.expires = 0; })`.
fn sessione_legata(scadenze: bool) -> SessioneCondivisa {
    let s = sessione();
    s.borrow_mut().self_id = Some("123".into());
    if scadenze {
        s.borrow_mut().cookies.iter_mut().for_each(|c| c.expires = 0.0);
    }
    s
}

fn wire(tipo: &str, valore: &str) -> EsitoRete {
    Ok(Risposta::nuova(200, codifica_chat(tipo, &j(valore)).unwrap()))
}

fn wiz(entrato: bool) -> EsitoRete {
    let dati = format!(r#"{{"qwAQke":"{}","SMqcke":"xsrf-test"}}"#, if entrato { "ChatUi" } else { "AccountsSignInUi" });
    Ok(Risposta::nuova(200, format!("<script>window.WIZ_global_data = {dati};</script>")))
}

fn ruotato(valore: &str, max_age: u32) -> EsitoRete {
    Ok(Risposta::nuova(200, "{}").con_intestazione("Set-Cookie", format!("SIDCC={valore}; Domain=.google.com; Path=/; Max-Age={max_age}")))
}

const IO: &str = r#"{"user_status":{"user_id":{"id":"123"}}}"#;
const MEMBRI: &str = r#"{"members":[{"user":{"user_id":{"id":"123"},"email":"me@example.com"}}]}"#;

/// Google finto: ogni risposta arriva dopo aver ceduto il passo, come una fetch, così le richieste concorrenti si intrecciano.
struct Google(Rc<TrasportoFinto>);

impl Trasporto for Google {
    fn invia(&self, r: Richiesta) -> FutureRete<'_> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.0.invia(r).await
        })
    }
}

fn client_con(s: SessioneCondivisa, now: Rc<Cell<i64>>, changed: Option<oe_rete::gchat::Salva>, f: impl FnMut(&Richiesta) -> EsitoRete + 'static) -> (GoogleChat, Rc<TrasportoFinto>) {
    let finto = TrasportoFinto::con_funzione(f);
    let t: Rc<dyn Trasporto> = Rc::new(Google(finto.clone()));
    let o = OpzioniChat { now: Some(Rc::new(move || now.get())), changed, guardia: None };
    (GoogleChat::nuovo(s, t, o).unwrap(), finto)
}

fn client(s: SessioneCondivisa, f: impl FnMut(&Richiesta) -> EsitoRete + 'static) -> (GoogleChat, Rc<TrasportoFinto>) {
    client_con(s, Rc::new(Cell::new(NOW * 1000)), None, f)
}

fn percorso(r: &Richiesta) -> String {
    url::Url::parse(&r.url).unwrap().path().to_string()
}

fn scrittura(t: &str) -> Mappa {
    j(t).as_obj().unwrap().clone()
}

fn stato(e: Result<Json, ErroreGoogle>) -> u16 {
    e.unwrap_err().status()
}

#[test]
fn numeri_dei_campi_e_id_a_64_bit() {
    assert_eq!(codifica_chat("GroupId", &j(r#"{"space_id":{"space_id":"abc"}}"#)).unwrap(), [0x0a, 0x05, 0x0a, 0x03, 0x61, 0x62, 0x63]);
    let input = j(r#"{"client_type":"WEB","client_version":"9007199254740993"}"#);
    assert_eq!(decodifica_chat("RequestHeader", &codifica_chat("RequestHeader", &input).unwrap()).unwrap(), input);
}

#[tokio::test(flavor = "current_thread")]
async fn elenca_e_legge_senza_conferme_di_lettura() {
    let (c, finto) = client(sessione(), |r| {
        let p = percorso(r);
        if p.ends_with("/mole/world") {
            return wiz(true);
        }
        assert_eq!(r.intestazioni.ottieni("x-framework-xsrf-token").as_deref(), Some("xsrf-test"));
        if p.ends_with("/get_self_user_status") {
            return wire("GetSelfUserStatusResponse", IO);
        }
        if p.ends_with("/get_members") {
            return wire("GetMembersResponse", r#"{"members":[{"user":{"user_id":{"id":"123"},"name":"Me","email":"me@example.com"}}]}"#);
        }
        if p.ends_with("/paginated_world") {
            let w = format!(r#"{{"world_items":[{{"group_id":{{"space_id":{{"space_id":"space1"}}}},"room_name":"Test","sort_timestamp":"{}","read_state":{{"unread_message_count":3}}}}]}}"#, NOW * 1_000_000);
            return wire("PaginatedWorldResponse", &w);
        }
        if p.ends_with("/list_topics") {
            let richiesta = decodifica_chat("ListTopicsRequest", r.corpo.as_deref().unwrap()).unwrap();
            assert_eq!(richiesta["page_size_for_topics"], Json::from(25));
            let t = format!(r#"{{"topics":[{{"id":{{"topic_id":"t1"}},"replies":[{{"id":{{"message_id":"t1"}},"creator":{{"user_id":{{"id":"123"}}}},"text_body":"hello","create_time":"{}"}}]}}]}}"#, NOW * 1_000_000);
            return wire("ListTopicsResponse", &t);
        }
        panic!("unexpected {p}");
    });
    assert_eq!(c.identity().await.unwrap(), ("me@example.com".to_string(), "123".to_string()));
    let elenco = c.list().await.unwrap();
    let prima = &elenco["conversations"][0];
    assert_eq!((prima["id"].clone(), prima["name"].clone(), prima["unread"].clone(), prima["kind"].clone()), (Json::from("space1"), Json::from("Test"), Json::from(3), Json::from("space")));
    let letti = c.read("space1", "space", 25.0).await.unwrap();
    assert_eq!((letti["self_id"].clone(), letti["coverage"].clone()), (Json::from("123"), Json::from("recent-topics")));
    let m = &letti["messages"][0];
    assert_eq!((m["id"].clone(), m["thread"].clone(), m["text"].clone(), m["from_id"].clone()), (Json::from("t1"), Json::from("t1"), Json::from("hello"), Json::from("123")));
    assert!(!finto.richieste().iter().any(|r| percorso(r).contains("mark_group")));
}

#[tokio::test(flavor = "current_thread")]
async fn sessione_rifiutata_rinnovata_una_volta_e_cookie_ruotato_salvato() {
    let pronta = Rc::new(Cell::new(false));
    let ruotate = Rc::new(Cell::new(0));
    let salvate = Rc::new(Cell::new(0));
    let s = sessione();
    let (p2, r2, s2) = (pronta.clone(), ruotate.clone(), salvate.clone());
    let (c, _) = client_con(s.clone(), Rc::new(Cell::new(NOW * 1000)), Some(Rc::new(move || {
        s2.set(s2.get() + 1);
        Ok(())
    })), move |r| {
        let p = percorso(r);
        if p == "/RotateCookies" {
            p2.set(true);
            r2.set(r2.get() + 1);
            return ruotato("fresh", 600);
        }
        if p.ends_with("/mole/world") {
            return wiz(p2.get());
        }
        if p.ends_with("/get_self_user_status") {
            return wire("GetSelfUserStatusResponse", IO);
        }
        wire("GetMembersResponse", MEMBRI)
    });
    assert_eq!(c.identity().await.unwrap().1, "123");
    assert_eq!(ruotate.get(), 1);
    assert!(salvate.get() > 0);
    assert!(s.borrow().cookies.iter().any(|c| c.name == "SIDCC" && c.value == "fresh"));
}

#[tokio::test(flavor = "current_thread")]
async fn una_rotazione_riuscita_non_vale_come_accesso() {
    let ruotate = Rc::new(Cell::new(0));
    let r2 = ruotate.clone();
    let (c, _) = client(sessione(), move |r| {
        if r.url.contains("RotateCookies") {
            r2.set(r2.get() + 1);
            return ruotato("new", 600);
        }
        wiz(false)
    });
    let e = c.identity().await.unwrap_err().to_string().to_lowercase();
    assert!(["session", "sign", "access"].iter().any(|x| e.contains(x)), "{e}");
    assert_eq!(ruotate.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn una_rpc_rifiutata_si_recupera_anche_con_la_pagina_xsrf_aperta() {
    let pronta = Rc::new(Cell::new(false));
    let ruotate = Rc::new(Cell::new(0));
    let (p2, r2) = (pronta.clone(), ruotate.clone());
    let (c, _) = client(sessione(), move |r| {
        let u = &r.url;
        if u.contains("mole/world") {
            return wiz(true);
        }
        if u.contains("RotateCookies") {
            p2.set(true);
            r2.set(r2.get() + 1);
            return ruotato("fresh", 600);
        }
        if !p2.get() {
            return Ok(Risposta::nuova(401, "{}"));
        }
        if u.contains("get_self_user_status") {
            return wire("GetSelfUserStatusResponse", IO);
        }
        wire("GetMembersResponse", MEMBRI)
    });
    assert_eq!(c.identity().await.unwrap().1, "123");
    assert_eq!(ruotate.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn flag_di_scrittura_malformati_rifiutati_prima_della_rete() {
    let (c, finto) = client(sessione(), |_| Ok(Risposta::nuova(500, "")));
    for cambio in [r#""files":"not-an-array""#, r#""text":123"#, r#""thread":0"#] {
        let w = scrittura(&format!(r#"{{"action":"send","conversation":"space1","kind":"space","text":"body","confirm":true,{cambio}}}"#));
        assert_eq!(stato(c.write(&w).await), 400, "{cambio}");
    }
    let w = scrittura(r#"{"action":"react","conversation":"space1","kind":"space","message_id":"m","thread":"t","emoji":"a","remove":"true","confirm":true}"#);
    assert_eq!(stato(c.write(&w).await), 400);
    assert_eq!(finto.chiamate(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn mai_ripetere_una_scrittura_ambigua_ne_inviare_senza_conferma() {
    let invii = Rc::new(Cell::new(0));
    let i2 = invii.clone();
    let (c, _) = client(sessione(), move |r| {
        if r.url.contains("mole/world") {
            return wiz(true);
        }
        if r.url.contains("get_self_user_status") {
            return wire("GetSelfUserStatusResponse", IO);
        }
        i2.set(i2.get() + 1);
        Ok(Risposta::nuova(503, "secret upstream body"))
    });
    let e = c.write(&scrittura(r#"{"action":"send","conversation":"space1","kind":"space","text":"private test"}"#)).await.unwrap_err();
    assert!(e.to_string().contains("confirm"), "{e}");
    assert_eq!(invii.get(), 0);
    let e = c.write(&scrittura(r#"{"action":"send","conversation":"space1","kind":"space","text":"private test","confirm":true}"#)).await.unwrap_err();
    assert_eq!(e.to_string(), "Google Chat HTTP 503");
    assert_eq!(invii.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn identita_legata_controllata_prima_di_scrivere_e_rinnovo_condiviso() {
    let (ruotate, xsrf, pronta, invii) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(false)), Rc::new(Cell::new(0)));
    let (r2, x2, p2, i2) = (ruotate.clone(), xsrf.clone(), pronta.clone(), invii.clone());
    let (c, _) = client(sessione_legata(false), move |r| {
        let u = &r.url;
        if u.contains("mole/world") {
            x2.set(x2.get() + 1);
            return wiz(p2.get());
        }
        if u.contains("RotateCookies") {
            r2.set(r2.get() + 1);
            p2.set(true);
            return ruotato("new", 300);
        }
        if u.contains("get_self_user_status") {
            return wire("GetSelfUserStatusResponse", r#"{"user_status":{"user_id":{"id":"other"}}}"#);
        }
        if u.contains("create_topic") {
            i2.set(i2.get() + 1);
            return wire("CreateTopicResponse", "{}");
        }
        wire("GetMembersResponse", "{}")
    });
    let (a, b) = tokio::join!(c.identity(), c.identity());
    assert!(a.is_err() && b.is_err());
    assert_eq!((ruotate.get(), xsrf.get()), (1, 2));
    let e = c.write(&scrittura(r#"{"action":"send","conversation":"space1","kind":"space","text":"test","confirm":true}"#)).await.unwrap_err();
    assert!(e.to_string().contains("another account"), "{e}");
    assert_eq!(invii.get(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn legge_i_membri_per_i_controlli_sugli_spazi_privati() {
    let (c, _) = client(sessione(), |r| {
        if r.url.contains("mole/world") {
            return wiz(true);
        }
        if r.url.contains("get_self_user_status") {
            return wire("GetSelfUserStatusResponse", IO);
        }
        wire("GetGroupResponse", r#"{"joined_member_ids":[{"user_id":{"id":"123"}}]}"#)
    });
    assert_eq!(c.details("space1", "space").await.unwrap()["joined_member_ids"].in_js(), r#"[{"user_id":{"id":"123"}}]"#);
}

#[tokio::test(flavor = "current_thread")]
async fn ricontrolla_l_account_legato_dopo_il_rinnovo_della_xsrf_scaduta() {
    for con_file in [false, true] {
        let now = Rc::new(Cell::new(NOW * 1000));
        let cambiato = Rc::new(Cell::new(false));
        let modifiche = Rc::new(Cell::new(0));
        let (n2, c2, m2) = (now.clone(), cambiato.clone(), modifiche.clone());
        let (c, _) = client_con(sessione_legata(true), now.clone(), None, move |r| {
            let u = &r.url;
            if u.contains("mole/world") {
                return wiz(n2.get() == NOW * 1000 || c2.get());
            }
            if u.contains("RotateCookies") {
                c2.set(true);
                return ruotato("fresh", 600);
            }
            if u.contains("get_self_user_status") {
                return wire("GetSelfUserStatusResponse", if c2.get() { r#"{"user_status":{"user_id":{"id":"other"}}}"# } else { IO });
            }
            if u.contains("get_members") {
                return wire("GetMembersResponse", MEMBRI);
            }
            m2.set(m2.get() + 1);
            if u.contains("create_topic") { wire("CreateTopicResponse", "{}") } else { Ok(Risposta::nuova(500, "")) }
        });
        c.identity().await.unwrap();
        now.set(now.get() + 25 * 3600 * 1000);
        let file = if con_file { r#","files":[{"name":"test.txt","data":"aGk=","mime_type":"text/plain"}]"# } else { "" };
        let w = scrittura(&format!(r#"{{"action":"send","conversation":"space1","kind":"space","text":"private","confirm":true{file}}}"#));
        assert_eq!(stato(c.write(&w).await), 409, "file={con_file}");
        assert_eq!(modifiche.get(), 0, "file={con_file}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_il_salto_all_accesso_invece_di_dare_l_html_come_file() {
    let (c, finto) = client(sessione(), |r| {
        Ok(if r.url.contains("get_attachment_url") {
            Risposta::nuova(302, "").con_intestazione("Location", "https://accounts.google.com/ServiceLogin")
        } else {
            Risposta::nuova(200, "<html>sign in</html>").con_intestazione("Content-Type", "text/html")
        })
    });
    assert_eq!(stato(c.attachment("opaque", "image/png").await), 409);
    assert_eq!(finto.chiamate(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn ricontrolla_l_identita_anche_dopo_un_rinnovo_xsrf_riuscito() {
    let now = Rc::new(Cell::new(NOW * 1000));
    let modifiche = Rc::new(Cell::new(0));
    let (n2, m2) = (now.clone(), modifiche.clone());
    let (c, _) = client_con(sessione_legata(true), now.clone(), None, move |r| {
        let u = &r.url;
        if u.contains("mole/world") {
            return wiz(true);
        }
        if u.contains("get_self_user_status") {
            return wire("GetSelfUserStatusResponse", if n2.get() == NOW * 1000 { IO } else { r#"{"user_status":{"user_id":{"id":"other"}}}"# });
        }
        if u.contains("get_members") {
            return wire("GetMembersResponse", MEMBRI);
        }
        m2.set(m2.get() + 1);
        wire("CreateTopicResponse", "{}")
    });
    c.identity().await.unwrap();
    now.set(now.get() + 25 * 3600 * 1000);
    let w = scrittura(r#"{"action":"send","conversation":"space1","kind":"space","text":"test","confirm":true}"#);
    assert_eq!(stato(c.write(&w).await), 409);
    assert_eq!(modifiche.get(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn controlla_ogni_salto_degli_allegati_e_niente_cookie_fuori_da_google() {
    let (c, finto) = client(sessione(), |r| {
        Ok(if r.url.contains("get_attachment_url") {
            Risposta::nuova(302, "").con_intestazione("Location", "https://lh3.googleusercontent.com/file")
        } else {
            Risposta::nuova(200, "file content")
        })
    });
    let risultato = c.attachment("opaque", "image/png").await.unwrap();
    use base64::Engine;
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(risultato["data"].as_str().unwrap()).unwrap(), b"file content");
    let cookie: Vec<Option<String>> = finto.richieste().iter().map(|r| r.intestazioni.ottieni("cookie")).collect();
    assert_eq!(cookie, [Some("OSID=chat-only; SID=domain".to_string()), None]);
    let e = c.download("http://127.0.0.1/secret").await.unwrap_err().to_string();
    assert!(e.contains("URL") || e.contains("host"), "{e}");
}
