//! La parte "Google session" di `agente-ts/test/gchat.test.ts`: cookie per host e percorso, importazione filtrata, rotazione, archivio.

use oe_nucleo::json::Json;
use oe_rete::google_web::{ArchivioGoogleWeb, CookieGoogle, SessioneGoogleWeb, assorbi_cookie, intestazione_cookie, leggi_sessione_importata};
use oe_rete::prova_util::CartellaTemp;
use std::cell::RefCell;
use std::rc::Rc;

const NOW: f64 = 1_790_000_000.0;

fn cookie(name: &str, value: &str, domain: &str, path: &str, expires: f64) -> CookieGoogle {
    CookieGoogle { name: name.into(), value: value.into(), domain: domain.into(), path: path.into(), expires }
}

fn sessione() -> SessioneGoogleWeb {
    SessioneGoogleWeb {
        account: "me@example.com".into(),
        user_agent: "test-browser".into(),
        cookies: vec![cookie("SID", "domain", ".google.com", "/", NOW + 3600.0), cookie("OSID", "chat-only", "chat.google.com", "/", 0.0)],
        self_id: None,
        salvata: NOW,
        errore: None,
    }
}

#[test]
fn cookie_per_host_e_confine_del_percorso_i_contenuti_senza_cookie() {
    let mut cookies = sessione().cookies;
    cookies.push(cookie("PATH", "one", "chat.google.com", "/u/0", 0.0));
    assert!(!intestazione_cookie(&cookies, "chat.google.com", "/u/01", NOW).contains("PATH"));
    assert!(intestazione_cookie(&cookies, "chat.google.com", "/u/0/api", NOW).contains("PATH=one"));
    assert_eq!(intestazione_cookie(&cookies, "chat-wsx.usercontent.google.com", "/download", NOW), "SID=domain");
    assert_eq!(intestazione_cookie(&cookies, "lh3.googleusercontent.com", "/image", NOW), "");
    assert_eq!(intestazione_cookie(&cookies, "google.com.evil.test", "/", NOW), "");
}

#[test]
fn filtra_cookie_estranei_e_scaduti_e_rifiuta_intestazioni_rotte() {
    let mut dati = sessione_json();
    let altri = Json::analizza(&format!(
        r#"[{{"name":"BAD","value":"x\r\nInjected: y","domain":".google.com"}},{{"name":"OLD","value":"x","domain":".google.com","expires":{}}},{{"name":"JIRA","value":"x","domain":".atlassian.net"}}]"#,
        NOW - 1.0
    ))
    .unwrap();
    if let Some(Json::Arr(c)) = dati.as_obj_mut().unwrap().get_mut("cookies") {
        c.extend(altri.as_arr().unwrap().iter().cloned());
    }
    let (_, cookies) = leggi_sessione_importata(&dati, NOW).unwrap();
    assert_eq!(cookies.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["SID", "OSID"]);
    let e = leggi_sessione_importata(&Json::analizza(r#"{"cookies":[]}"#).unwrap(), NOW).unwrap_err();
    assert!(e.0.contains("SID"), "{e}");
}

fn sessione_json() -> Json {
    sessione().in_json()
}

#[test]
fn assorbe_la_rotazione_con_la_scadenza_e_toglie_i_cookie_scaduti() {
    let mut cookies = sessione().cookies;
    assorbi_cookie(&mut cookies, "accounts.google.com", &["SIDCC=new; Domain=.google.com; Path=/; Max-Age=60", "X=x; Domain=evil.test; Path=/"], NOW, None);
    assert_eq!(cookies.iter().find(|c| c.name == "SIDCC").map(|c| c.expires), Some(NOW + 60.0));
    assorbi_cookie(&mut cookies, "accounts.google.com", &["SIDCC=; Domain=.google.com; Path=/; Max-Age=0"], NOW, None);
    assert!(!cookies.iter().any(|c| c.name == "SIDCC"));
    assert!(!cookies.iter().any(|c| c.name == "X"));
}

#[test]
fn salva_in_modo_atomico_si_rilegge_e_un_salvataggio_fallito_si_vede() {
    let d = CartellaTemp::nuova();
    let file = d.percorso().join("state.json");
    let archivio = ArchivioGoogleWeb::nuovo(file.clone());
    archivio.metti(Rc::new(RefCell::new(sessione()))).unwrap();
    let riletta = ArchivioGoogleWeb::nuovo(file.clone()).sessione("ME@EXAMPLE.COM").unwrap().unwrap();
    assert_eq!(riletta.borrow().cookies, sessione().cookies);
    let cattivo = ArchivioGoogleWeb::nuovo(file.join("impossible.json"));
    assert!(cattivo.metti(Rc::new(RefCell::new(sessione()))).is_err());
    assert!(cattivo.elenco().unwrap().is_empty());
    let nomi: Vec<String> = std::fs::read_dir(d.percorso()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(nomi, ["state.json"]);
    // il file come lo scrive l'agente TypeScript: due spazi, a capo in fondo, 0600
    let testo = std::fs::read_to_string(&file).unwrap();
    assert!(testo.starts_with("{\n  \"sessioni\": {\n    \"me@example.com\": {\n      \"user_agent\": \"test-browser\",") && testo.ends_with("}\n"), "{testo}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    std::fs::write(&file, "broken json").unwrap();
    assert!(ArchivioGoogleWeb::nuovo(file).elenco().is_err());
}
