//! Le prove di `agente-ts/test/mcas-outlook.test.ts`.

use oe_nucleo::json::Json;
use oe_rete::mcas_auth_forms::mcas_moved;
use oe_rete::mcas_outlook::{ProxySession, authorize_proxy};
use oe_rete::sessione_web::nuovo_cookie;
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::{Risposta, Trasporto};
use std::cell::RefCell;
use std::rc::Rc;
use url::Url;

#[test]
fn context_restore_di_outlook_e_destinazioni_estranee() {
    let html = r#"<title>Object Moved</title><script>window.location.replace("https://outlook.cloud.microsoft.mcas.ms/mail/")</script>"#;
    assert_eq!(mcas_moved(html, "https://outlook.cloud.microsoft").unwrap(), "https://outlook.cloud.microsoft.mcas.ms/mail/");
    assert!(mcas_moved(&html.replace("outlook.cloud.microsoft.mcas.ms", "evil.example"), "https://outlook.cloud.microsoft").is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn bootstrap_non_fidati_rifiutati_prima_di_mandare_cookie() {
    for b in ["http://mcas-proxyweb.mcas.ms/js-bootstrap.js", "https://evil.example/js-bootstrap.js", "https://mcas-proxyweb.mcas.ms@evil.example/js-bootstrap.js"] {
        let finto = TrasportoFinto::con_funzione(|_| Ok(Risposta::nuova(200, "")));
        let t: Rc<dyn Trasporto> = finto.clone();
        let mut s = ProxySession { bootstrap_url: b.to_string(), cookies: vec![] };
        assert!(authorize_proxy(&mut s, "a@b", "Chrome", &t).await.is_err(), "{b}");
        assert_eq!(finto.chiamate(), 0, "{b}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn la_sessione_del_proxy_rinnova_il_contesto_e_ruota_i_cookie() {
    let chiamate: Rc<RefCell<Vec<String>>> = Rc::default();
    let state: Rc<RefCell<String>> = Rc::default();
    let (c2, s2) = (chiamate.clone(), state.clone());
    let finto = TrasportoFinto::con_funzione(move |r| {
        let u = Url::parse(&r.url).unwrap();
        let host = u.host_str().unwrap().to_string();
        c2.borrow_mut().push(host.clone());
        if u.path() == "/js-bootstrap.js" {
            return Ok(Risposta::nuova(
                200,
                r#"__JSWRAPPER.initWrapperFixer({"urlIntegrityEnforcementEnabled":false,"suffixDomainName":"mcas.ms","niptuckSubDomainName":"mcas-proxyweb","authUserData":"context","topFrameSaasId":1});"#,
            ));
        }
        if u.path().ends_with("/authorize") {
            *s2.borrow_mut() = u.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
            if host.ends_with(".mcas.ms") {
                return Ok(Risposta::nuova(302, "").con_intestazione("Location", format!("https://login.microsoftonline.com{}?{}", u.path(), u.query().unwrap_or_default())));
            }
            return Ok(Risposta::nuova(200, r#"<form method="post" action="https://tenant.access.mcas.ms/aad_login"><input type="hidden" name="id_token" value="private-id"></form>"#));
        }
        if u.path() == "/aad_login" {
            return Ok(Risposta::nuova(302, "")
                .con_intestazione("Location", format!("https://outlook.cloud.microsoft.mcas.ms/mail/?code=fixture&state={}", s2.borrow()))
                .con_intestazione("Set-Cookie", "AdallomAuth=rotated; Domain=mcas.ms; Path=/; Secure"));
        }
        if u.path() == "/mail/" {
            assert!(r.intestazioni.primo("cookie").unwrap_or_default().contains("AdallomAuth=rotated"));
            return Ok(Risposta::nuova(200, r#"<script src="https://mcas-proxyweb.mcas.ms/js-bootstrap.js?fixture=2"></script>"#));
        }
        assert!(String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).contains("code=fixture"));
        Ok(json(200, r#"{"access_token":"access","refresh_token":"refresh"}"#))
    });
    let t: Rc<dyn Trasporto> = finto;
    let mut s = ProxySession {
        bootstrap_url: "https://mcas-proxyweb.mcas.ms/js-bootstrap.js?fixture=1".to_string(),
        cookies: vec![nuovo_cookie("ESTSAUTH", "private", "login.microsoftonline.com", "/", -1.0)],
    };
    let esito = authorize_proxy(&mut s, "user@example.com", "Chrome", &t).await.unwrap();
    assert_eq!(esito.get("access_token"), Some(&Json::from("access")));
    assert!(chiamate.borrow().iter().any(|h| h == "tenant.access.mcas.ms"));
    assert!(s.bootstrap_url.contains("fixture=2"), "{}", s.bootstrap_url);
}
