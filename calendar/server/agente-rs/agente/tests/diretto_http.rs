//! Le prove HTTP di `agente-ts/test/diretto.test.ts`: accesso PKCE e codice dispositivo di Microsoft, PKCE di Google, rimozione di una
//! casella che resta fuori dopo un riavvio e torna com'era con un nuovo accesso.

mod comune;

use comune::{canonico, chiedi};
use oe_agente::diretto::{GestoreToken, OUTLOOK_CLIENT_ID, OUTLOOK_REDIRECT_URI, OpzioniToken};
use oe_agente::fonte::Adesso;
use oe_agente::http::servi;
use oe_agente::server::{Agente, OpzioniAgente, carica_fonti, contesto_di};
use oe_nucleo::json::Json;
use oe_rete::sessione_web::codifica_componente;
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::{ErroreRete, Richiesta, Trasporto};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tokio::net::TcpListener;
use tokio::task::LocalSet;

const CHIAVE: &str = "segreto-prova";

fn fake_jwt(claims: &str) -> String {
    use base64::Engine;
    let b = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.sig", b(r#"{"alg":"none","typ":"JWT"}"#), b(claims))
}

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("{nome}-{}-{}", std::process::id(), oe_agente::fonte::adesso_vero()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ms(iso: &str) -> i64 {
    iso.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn corpo(r: &Richiesta) -> String {
    String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned()
}

/// Un agente con il gestore dei token sul trasporto finto, come `creaServer` nelle prove TypeScript.
async fn avvia(dir: &Path, tokens_file: &Path, fonti_file: &Path, ora_ms: i64, trasporto: Rc<dyn Trasporto>) -> u16 {
    let adesso: Adesso = Rc::new(move || ora_ms);
    let gestore = Rc::new(GestoreToken::nuovo(tokens_file.to_path_buf(), OpzioniToken { token_posta: None, adesso: Some(adesso.clone()), trasporto, strict: false }));
    let mut ctx = contesto_di(dir.join("accessi.json"), "Europe/Rome", Some(adesso.clone()), Some(Rc::new(|_, _| {})));
    ctx.tokens = Some(gestore.clone());
    let fonti = carica_fonti(fonti_file, &dir.join("nessun-db"), &ctx).expect("fonti");
    let mut o = OpzioniAgente::nuove(CHIAVE, fonti, dir.join("nessun-battito.json"), "Europe/Rome");
    o.adesso = adesso;
    o.registro = Rc::new(|_, _| {});
    o.tokens = Some(gestore);
    let ascolto = TcpListener::bind("127.0.0.1:0").await.expect("porta");
    let porta = ascolto.local_addr().expect("indirizzo").port();
    tokio::task::spawn_local(servi(ascolto, Rc::new(Agente::nuovo(o))));
    porta
}

async fn get(porta: u16, percorso: &str) -> Json {
    let (stato, testo) = chiedi(porta, "GET", percorso, Some(CHIAVE), None).await;
    assert_eq!(stato, 200, "{percorso}: {testo}");
    Json::analizza(&testo).expect("JSON")
}

async fn post(porta: u16, percorso: &str, dati: &str) -> Json {
    let (_, testo) = chiedi(porta, "POST", percorso, Some(CHIAVE), Some(dati)).await;
    Json::analizza(&testo).expect("JSON")
}

fn uguale(ottenuto: &Json, atteso: &str) {
    assert_eq!(canonico(ottenuto), canonico(&Json::analizza(atteso).unwrap()));
}

#[tokio::test(flavor = "current_thread")]
async fn completa_il_pkce_via_http() {
    LocalSet::new()
        .run_until(async {
            let dir = cartella("diretto-http");
            let tokens_file = dir.join("tokens.json");
            let fonti_file = dir.join("fonti.json");
            let ora_ms = ms("2026-11-05T10:00:00Z");
            std::fs::write(&tokens_file, r#"{"accounts":{}}"#).unwrap();
            std::fs::write(&fonti_file, r#"[{"tipo":"android","id":"android::mario@alfa.example","nome":"Alfa","account":"mario@alfa.example"}]"#).unwrap();
            let jwt = fake_jwt(&format!(r#"{{"upn":"mario@alfa.example","tid":"tid-123","exp":{},"scp":"Calendars.ReadWrite Mail.ReadWrite"}}"#, ora_ms / 1000 + 3600));
            let j2 = jwt.clone();
            let finto = TrasportoFinto::con_funzione(move |r| {
                if r.url == "https://login.microsoftonline.com/common/oauth2/v2.0/token" {
                    let b = corpo(r);
                    assert!(b.contains("grant_type=authorization_code") && b.contains("code=CODICE_AUTH_123") && b.contains("code_verifier="), "{b}");
                    return Ok(json(200, &format!(r#"{{"access_token":"{j2}","refresh_token":"rt-pkce-123","expires_in":3600}}"#)));
                }
                Err(ErroreRete::Rete(format!("URL inatteso: {}", r.url)))
            });
            let porta = avvia(&dir, &tokens_file, &fonti_file, ora_ms, finto).await;

            // con fonti dirette il servizio è sano anche senza battito.json
            uguale(&get(porta, "/api/salute").await["servizio"], r#"{"eta_s":0,"pausa":false,"telefono":true}"#);

            let inizio = post(porta, "/api/auth/inizio", r#"{"email":"mario@alfa.example"}"#).await;
            assert_eq!(inizio["ok"], Json::Bool(true));
            let url = inizio["url"].as_str().unwrap();
            for parte in [codifica_componente(OUTLOOK_CLIENT_ID), codifica_componente(OUTLOOK_REDIRECT_URI), "login_hint=mario%40alfa.example".to_string()] {
                assert!(url.contains(&parte), "{parte} in {url}");
            }

            // l'indirizzo msauth://... completo incollato dal browser
            let incollato = format!("{OUTLOOK_REDIRECT_URI}?code=CODICE_AUTH_123&state={}", codifica_componente(inizio["state"].as_str().unwrap()));
            let completa = post(porta, "/api/auth/completa", &oe_nucleo::ogg! { "provider": "microsoft", "codice": incollato }.in_js()).await;
            uguale(&completa, r#"{"ok":true,"account":"mario@alfa.example","provider":"microsoft"}"#);

            assert_eq!(get(porta, "/api/auth/stato").await["caselle"][0]["configurato"], Json::Bool(true));
            let posta = get(porta, "/api/token-posta").await;
            assert_eq!(posta["account"].as_arr().unwrap().len(), 1);
            assert_eq!(posta["account"][0]["upn"], Json::from("mario@alfa.example"));
            assert_eq!(posta["account"][0]["token"], Json::from(jwt.as_str()));
            let _ = std::fs::remove_dir_all(&dir);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn completa_il_pkce_google_e_il_codice_dispositivo_microsoft() {
    LocalSet::new()
        .run_until(async {
            let dir = cartella("diretto-auth2");
            let tokens_file = dir.join("tokens.json");
            let fonti_file = dir.join("fonti.json");
            let ora_ms = ms("2026-11-05T10:00:00Z");
            std::fs::write(&fonti_file, r#"[{"tipo":"outlook","id":"outlook::mario@alfa.example","nome":"Alfa","account":"mario@alfa.example"}]"#).unwrap();
            let jwt_ms = fake_jwt(&format!(r#"{{"upn":"mario@alfa.example","tid":"tid-alfa","exp":{}}}"#, ora_ms / 1000 + 3600));
            let id_token = fake_jwt(&format!(r#"{{"email":"nuovo@gmail.com","exp":{}}}"#, ora_ms / 1000 + 3600));
            let mut giri_device = 0;
            let finto = TrasportoFinto::con_funzione(move |r| {
                let b = corpo(r);
                if r.url.ends_with("/oauth2/v2.0/devicecode") {
                    return Ok(json(200, r#"{"device_code":"dev-code-123","user_code":"ABCD-1234","verification_uri":"https://microsoft.com/devicelogin","interval":4}"#));
                }
                if r.url.ends_with("/oauth2/v2.0/token") && b.contains("device_code") {
                    giri_device += 1;
                    if giri_device == 1 {
                        return Ok(json(400, r#"{"error":"authorization_pending"}"#));
                    }
                    return Ok(json(200, &format!(r#"{{"access_token":"{jwt_ms}","refresh_token":"rt-dev-999","expires_in":3600}}"#)));
                }
                if r.url == "https://oauth2.googleapis.com/token" {
                    assert!(b.contains("grant_type=authorization_code") && b.contains("code=4%2F0Agtest123"), "{b}");
                    return Ok(json(200, &format!(r#"{{"access_token":"ya29.google-pkce","refresh_token":"1//0g-refresh-pkce","id_token":"{id_token}","expires_in":3600}}"#)));
                }
                Err(ErroreRete::Rete(format!("URL inatteso: {}", r.url)))
            });
            let porta = avvia(&dir, &tokens_file, &fonti_file, ora_ms, finto).await;

            let inizio = post(porta, "/api/auth/inizio", r#"{"provider":"microsoft","modo":"device","email":"mario@alfa.example"}"#).await;
            assert_eq!((inizio["ok"].clone(), inizio["modo"].clone(), inizio["user_code"].clone()), (Json::Bool(true), Json::from("device"), Json::from("ABCD-1234")));
            let controlla = oe_nucleo::ogg! { "state": inizio["state"].clone() }.in_js();
            uguale(&post(porta, "/api/auth/controlla", &controlla).await, r#"{"ok":true,"stato":"in_attesa"}"#);
            uguale(&post(porta, "/api/auth/controlla", &controlla).await, r#"{"ok":true,"stato":"completato","account":"mario@alfa.example","provider":"microsoft"}"#);

            // Google: la nuova casella "Personale" si aggiunge da sola
            let g = post(porta, "/api/auth/inizio", r#"{"provider":"google","email":"nuovo@gmail.com"}"#).await;
            assert_eq!((g["ok"].clone(), g["provider"].clone()), (Json::Bool(true), Json::from("google")));
            let url = g["url"].as_str().unwrap();
            assert!(url.contains("accounts.google.com/o/oauth2/v2/auth") && url.contains("login_hint=nuovo%40gmail.com"), "{url}");
            let codice = format!("http://localhost/?state={}&code=4%2F0Agtest123&scope=https://www.googleapis.com/auth/calendar", codifica_componente(g["state"].as_str().unwrap()));
            uguale(&post(porta, "/api/auth/completa", &oe_nucleo::ogg! { "provider": "google", "codice": codice }.in_js()).await, r#"{"ok":true,"account":"nuovo@gmail.com","provider":"google"}"#);

            let stato = get(porta, "/api/auth/stato").await;
            let caselle = stato["caselle"].as_arr().unwrap();
            assert_eq!(caselle.iter().map(|c| c["account"].as_str().unwrap()).collect::<Vec<_>>(), ["mario@alfa.example", "nuovo@gmail.com"]);
            assert_eq!((caselle[1]["nome"].clone(), caselle[1]["configurato"].clone()), (Json::from("Personale"), Json::Bool(true)));
            let _ = std::fs::remove_dir_all(&dir);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn rimuove_una_casella_la_tiene_fuori_e_la_riprende_com_era() {
    LocalSet::new()
        .run_until(async {
            let dir = cartella("diretto-rimuovi");
            let tokens_file = dir.join("tokens.json");
            let fonti_file = dir.join("fonti.json");
            let ora_ms = ms("2026-11-07T10:00:00Z");
            std::fs::write(
                &fonti_file,
                r#"[{"tipo":"android","id":"android::mario@alfa.example","nome":"Alfa","account":"mario@alfa.example"},{"tipo":"android","id":"android::Mario@Beta.example","nome":"Beta","account":"Mario@Beta.example","calendari":["Calendar","Gamma"]}]"#,
            )
            .unwrap();
            std::fs::write(
                &tokens_file,
                format!(
                    r#"{{"accounts":{{"mario@alfa.example":{{"provider":"microsoft","client_id":"{OUTLOOK_CLIENT_ID}","refresh_token":"rt-alfa"}},"mario@beta.example":{{"provider":"microsoft","client_id":"{OUTLOOK_CLIENT_ID}","refresh_token":"rt-beta","errore":"invalid_grant"}}}}}}"#
                ),
            )
            .unwrap();
            let jwt_beta = fake_jwt(&format!(r#"{{"upn":"mario@beta.example","tid":"tid-beta","exp":{}}}"#, ora_ms / 1000 + 3600));
            let finto: Rc<dyn Trasporto> = TrasportoFinto::con_funzione(move |r| {
                if r.url.ends_with("/oauth2/v2.0/devicecode") {
                    return Ok(json(200, r#"{"device_code":"dev-beta","user_code":"BETA-0001","verification_uri":"https://microsoft.com/devicelogin","interval":4}"#));
                }
                if r.url.ends_with("/oauth2/v2.0/token") && corpo(r).contains("device_code") {
                    return Ok(json(200, &format!(r#"{{"access_token":"{jwt_beta}","refresh_token":"rt-beta-nuovo","expires_in":3600}}"#)));
                }
                Err(ErroreRete::Rete(format!("URL inatteso: {}", r.url)))
            });
            let caselle = |stato: Json, campi: &[&str]| -> Vec<String> {
                stato["caselle"].as_arr().unwrap().iter().map(|c| campi.iter().map(|k| c[*k].in_js()).collect::<Vec<_>>().join(" ")).collect()
            };

            let porta = avvia(&dir, &tokens_file, &fonti_file, ora_ms, finto.clone()).await;
            assert_eq!(caselle(get(porta, "/api/auth/stato").await, &["account", "scaduto"]), [r#""mario@alfa.example" false"#, r#""mario@beta.example" true"#]);
            uguale(&post(porta, "/api/auth/rimuovi", r#"{"account":"nessuno@alfa.example"}"#).await, r#"{"ok":false,"errore":"Casella non trovata"}"#);
            uguale(&post(porta, "/api/auth/rimuovi", r#"{"account":"  MARIO@beta.example "}"#).await, r#"{"ok":true,"account":"mario@beta.example"}"#);
            assert_eq!(caselle(get(porta, "/api/auth/stato").await, &["account"]), [r#""mario@alfa.example""#]);
            let su_disco = Json::analizza(&std::fs::read_to_string(&tokens_file).unwrap()).unwrap();
            assert_eq!(su_disco["accounts"].as_obj().unwrap().keys().cloned().collect::<Vec<_>>(), ["mario@alfa.example"]);
            assert_eq!(su_disco["rimossi"].in_js(), r#"["mario@beta.example"]"#);

            // riavvio: fonti.json la elenca ancora, ma resta fuori
            let porta = avvia(&dir, &tokens_file, &fonti_file, ora_ms, finto).await;
            assert_eq!(caselle(get(porta, "/api/auth/stato").await, &["account"]), [r#""mario@alfa.example""#]);

            // nuovo accesso: torna la fonte di fonti.json (id e calendari scelti), non una nuova
            let inizio = post(porta, "/api/auth/inizio", r#"{"provider":"microsoft","modo":"device"}"#).await;
            let controlla = oe_nucleo::ogg! { "state": inizio["state"].clone() }.in_js();
            uguale(&post(porta, "/api/auth/controlla", &controlla).await, r#"{"ok":true,"stato":"completato","account":"mario@beta.example","provider":"microsoft"}"#);
            assert_eq!(caselle(get(porta, "/api/auth/stato").await, &["id", "scaduto"]), [r#""android::mario@alfa.example" false"#, r#""android::Mario@Beta.example" false"#]);
            let su_disco = Json::analizza(&std::fs::read_to_string(&tokens_file).unwrap()).unwrap();
            assert!(su_disco.get("rimossi").is_none(), "{}", su_disco.in_js());
            let _ = std::fs::remove_dir_all(&dir);
        })
        .await;
}
