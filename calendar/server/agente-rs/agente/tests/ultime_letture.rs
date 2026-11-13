//! Outlook che non risponde o risponde 503: ogni casella dà la sua ultima lettura riuscita, gialla con l'ora di quella lettura, senza far
//! aspettare le altre; anche dopo un riavvio, fino a 30 giorni.

mod comune;

use comune::chiedi;
use oe_agente::diretto::{GestoreToken, OUTLOOK_CLIENT_ID, OpzioniToken};
use oe_agente::fonte::Adesso;
use oe_agente::http::servi;
use oe_agente::server::{Agente, OpzioniAgente, carica_fonti, contesto_di};
use oe_nucleo::json::Json;
use oe_rete::trasporto::prova::json;
use oe_rete::trasporto::{ErroreRete, FutureRete, Richiesta, Trasporto};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::task::LocalSet;

const CHIAVE: &str = "segreto-prova";
const SETTIMANA: &str = "/api/eventi?inizio=2026-11-16T00:00:00&fine=2026-11-23T00:00:00";

fn fake_jwt(claims: &str) -> String {
    use base64::Engine;
    let b = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.sig", b(r#"{"alg":"none","typ":"JWT"}"#), b(claims))
}

#[derive(Clone, Copy, PartialEq)]
enum Modo {
    Su,
    Giu,
    Appeso,
}

/// Outlook finto per due caselle: il token dice di chi è la richiesta, `modo` decide come risponde la sua calendarview.
struct Outlook {
    jwt: HashMap<&'static str, String>,
    modo: RefCell<HashMap<&'static str, Modo>>,
}

impl Trasporto for Outlook {
    fn invia(&self, r: Richiesta) -> FutureRete<'_> {
        Box::pin(async move {
            let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
            if r.url.contains("/oauth2/v2.0/token") {
                let chi = if corpo.contains("rt-alfa") { "alfa" } else { "beta" };
                return Ok(json(200, &format!(r#"{{"access_token":"{}","refresh_token":"rt-{chi}","expires_in":7200}}"#, self.jwt[chi])));
            }
            let token = r.intestazioni.primo("authorization").unwrap_or_default().trim_start_matches("Bearer ").to_string();
            let chi = *self.jwt.iter().find(|(_, j)| **j == token).map(|(c, _)| c).expect("token sconosciuto");
            if r.url.contains("/me/calendars?") {
                return Ok(json(200, r#"{"value":[{"Id":"cal-1","Name":"Calendar","IsDefaultCalendar":true}]}"#));
            }
            let modo = self.modo.borrow()[chi];
            match modo {
                Modo::Su => Ok(json(
                    200,
                    &format!(
                        r#"{{"value":[{{"Id":"ev-{chi}","Subject":"Riunione {chi}","Start":{{"DateTime":"2026-11-17T10:00:00.0000000","TimeZone":"Europe/Rome"}},"End":{{"DateTime":"2026-11-17T11:00:00.0000000","TimeZone":"Europe/Rome"}},"IsAllDay":false,"IsCancelled":false,"ShowAs":"Busy"}}]}}"#
                    ),
                )),
                Modo::Giu => Ok(json(503, r#"{"error":{"code":"ErrorServiceUnavailable"}}"#)),
                Modo::Appeso => {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Err(ErroreRete::Timeout)
                }
            }
        })
    }
}

fn cartella() -> PathBuf {
    let d = std::env::temp_dir().join(format!("oe-ultime-letture-{}-{}", std::process::id(), oe_agente::fonte::adesso_vero()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

async fn avvia(dir: &Path, orologio: Rc<Cell<i64>>, outlook: Rc<Outlook>) -> u16 {
    let adesso: Adesso = Rc::new(move || orologio.get());
    let trasporto: Rc<dyn Trasporto> = outlook;
    let gestore = Rc::new(GestoreToken::nuovo(dir.join("tokens.json"), OpzioniToken { token_posta: None, adesso: Some(adesso.clone()), trasporto, strict: false }));
    let mut ctx = contesto_di(dir.join("accessi.json"), "Europe/Rome", Some(adesso.clone()), Some(Rc::new(|_, _| {})));
    ctx.tokens = Some(gestore.clone());
    let fonti = carica_fonti(&dir.join("fonti.json"), &dir.join("nessun-db"), &ctx).expect("fonti");
    let mut o = OpzioniAgente::nuove(CHIAVE, fonti, dir.join("nessun-battito.json"), "Europe/Rome");
    o.adesso = adesso;
    o.registro = Rc::new(|_, _| {});
    o.tokens = Some(gestore);
    o.cache_s = 0.0;
    o.ultime_letture = Some(dir.join("ultime-letture.json"));
    let ascolto = TcpListener::bind("127.0.0.1:0").await.expect("porta");
    let porta = ascolto.local_addr().expect("indirizzo").port();
    tokio::task::spawn_local(servi(ascolto, Rc::new(Agente::nuovo(o))));
    porta
}

async fn settimana(porta: u16) -> Json {
    let (stato, testo) = chiedi(porta, "GET", SETTIMANA, Some(CHIAVE), None).await;
    assert_eq!(stato, 200, "{testo}");
    Json::analizza(&testo).unwrap()
}

fn titoli(r: &Json) -> Vec<String> {
    r["eventi"].as_arr().unwrap().iter().map(|e| e["title"].as_str().unwrap().to_string()).collect()
}

fn stati(r: &Json) -> Vec<(String, String)> {
    r["caselle"].as_arr().unwrap().iter().map(|c| (c["stato"].as_str().unwrap().to_string(), c["msg"].in_js())).collect()
}

#[tokio::test(flavor = "current_thread")]
async fn una_casella_che_non_si_legge_da_la_sua_ultima_lettura() {
    LocalSet::new()
        .run_until(async {
            let dir = cartella();
            let ora = "2026-11-16T08:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
            let orologio = Rc::new(Cell::new(ora));
            std::fs::write(
                dir.join("fonti.json"),
                r#"[{"tipo":"outlook","id":"outlook::mario@alfa.example","nome":"Alfa","account":"mario@alfa.example"},{"tipo":"outlook","id":"outlook::mario@beta.example","nome":"Beta","account":"mario@beta.example"}]"#,
            )
            .unwrap();
            std::fs::write(
                dir.join("tokens.json"),
                format!(
                    r#"{{"accounts":{{"mario@alfa.example":{{"provider":"microsoft","client_id":"{OUTLOOK_CLIENT_ID}","refresh_token":"rt-alfa"}},"mario@beta.example":{{"provider":"microsoft","client_id":"{OUTLOOK_CLIENT_ID}","refresh_token":"rt-beta"}}}}}}"#
                ),
            )
            .unwrap();
            let scade = ora / 1000 + 90 * 86_400;
            let jwt = HashMap::from([
                ("alfa", fake_jwt(&format!(r#"{{"upn":"mario@alfa.example","tid":"t","exp":{scade}}}"#))),
                ("beta", fake_jwt(&format!(r#"{{"upn":"mario@beta.example","tid":"t","exp":{scade}}}"#))),
            ]);
            let outlook = Rc::new(Outlook { jwt, modo: RefCell::new(HashMap::from([("alfa", Modo::Su), ("beta", Modo::Su)])) });
            let porta = avvia(&dir, orologio.clone(), outlook.clone()).await;

            let r = settimana(porta).await;
            assert_eq!(titoli(&r), ["Riunione alfa", "Riunione beta"]);
            assert!(stati(&r).iter().all(|(s, _)| s == "ok"), "{:?}", stati(&r));
            assert!(dir.join("ultime-letture.json").exists());

            // un'ora dopo Alfa non risponde più e Beta dà 503: si aspetta Alfa 10 s, non i 60 della sua richiesta
            orologio.set(ora + 3_600_000);
            *outlook.modo.borrow_mut() = HashMap::from([("alfa", Modo::Appeso), ("beta", Modo::Giu)]);
            let partito = Instant::now();
            let r = settimana(porta).await;
            let durata = partito.elapsed();
            assert!(durata >= Duration::from_secs(9) && durata < Duration::from_secs(15), "{durata:?}");
            assert_eq!(titoli(&r), ["Riunione alfa", "Riunione beta"]);
            assert_eq!(r["errori"].in_js(), "[]");
            let s = stati(&r);
            assert_eq!(s[0], ("avviso".to_string(), r#""nessuna risposta in 10 s · dati della lettura del 16/11 alle 09:00""#.to_string()));
            assert!(s[1].0 == "avviso" && s[1].1.contains("· dati della lettura del 16/11 alle 09:00"), "{s:?}");
            assert_eq!(r["fonti"]["Beta"]["stato"], Json::from("avviso"));

            // Alfa ha ancora la lettura appesa: la richiesta dopo non la aspetta di nuovo
            let partito = Instant::now();
            let r = settimana(porta).await;
            assert!(partito.elapsed() < Duration::from_secs(5), "{:?}", partito.elapsed());
            assert_eq!(titoli(&r), ["Riunione alfa", "Riunione beta"]);

            // riavvio con Beta ancora giù: l'ultima lettura è su disco
            *outlook.modo.borrow_mut() = HashMap::from([("alfa", Modo::Su), ("beta", Modo::Giu)]);
            let porta = avvia(&dir, orologio.clone(), outlook.clone()).await;
            let r = settimana(porta).await;
            assert_eq!(titoli(&r), ["Riunione alfa", "Riunione beta"]);
            assert_eq!(stati(&r).iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), ["ok", "avviso"]);

            // oltre 30 giorni Beta non dà più niente: torna l'errore
            orologio.set(ora + 31 * 86_400_000);
            let r = settimana(porta).await;
            assert_eq!(titoli(&r), ["Riunione alfa"]);
            assert_eq!(stati(&r)[1].0, "errore");
            assert_eq!(r["errori"].as_arr().unwrap().len(), 1);
            let _ = std::fs::remove_dir_all(&dir);
        })
        .await;
}
