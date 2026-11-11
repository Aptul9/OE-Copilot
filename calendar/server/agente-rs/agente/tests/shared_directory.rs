//! Le prove di `agente-ts/test/shared-directory.test.ts`: la chiave stretta di TeamsRelay, l'accesso guidato, il rapporto degli account
//! Teams, la capacità calendario spenta, i comandi di gestione e le etichette. Le caselle sono fonti dirette vere su un Microsoft finto:
//! dove la prova TypeScript sostituisce i metodi del gestore, qui risponde il finto allo stesso modo.

mod comune;

use comune::canonico;
use oe_agente::diretto::{CalendarioDiretto, GestoreToken, OpzioniToken};
use oe_agente::fonte::{Adesso, Fonte};
use oe_agente::http::servi;
use oe_agente::server::{Agente, OpzioniAgente, contesto_di};
use oe_nucleo::json::Json;
use oe_rete::shared_account::TEAMS_APP_CLIENT_ID;
use oe_rete::shared_teams::{STANTIO_S, StatoTeams, leggi_rapporto_teams};
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::{Richiesta, Trasporto};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use tokio::net::TcpListener;
use tokio::task::LocalSet;

const TID: &str = "22222222-2222-4222-8222-222222222222";
const OID: &str = "11111111-1111-4111-8111-111111111111";

fn shared(capacita: &str) -> String {
    format!(r#"{{"tenantId":"{TID}","objectId":"{OID}","capabilities":{capacita},"origin":"mobile"}}"#)
}

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

fn token(upn: &str, aud: &str) -> String {
    use base64::Engine;
    let exp = oe_agente::fonte::adesso_vero() / 1000 + 3600;
    let c = format!(r#"{{"upn":"{upn}","tid":"{TID}","oid":"{OID}","aud":"{aud}","exp":{exp},"scp":"Mail.ReadWrite Calendars.ReadWrite"}}"#);
    format!("x.{}.x", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(c))
}

fn campo(r: &Richiesta, k: &str) -> String {
    let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
    form_urlencoded::parse(corpo.as_bytes()).find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default()
}

struct Avviato {
    dir: PathBuf,
    porta: u16,
    tokens: Rc<GestoreToken>,
    finto: Rc<TrasportoFinto>,
}

impl Drop for Avviato {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Avviato {
    async fn chiama(&self, percorso: &str, chiave: &str, corpo: Option<&str>) -> (u16, Json) {
        let metodo = if corpo.is_some() { "POST" } else { "GET" };
        let (stato, testo) = comune::chiedi(self.porta, metodo, percorso, Some(chiave), corpo).await;
        (stato, Json::analizza(&testo).unwrap_or(Json::Null))
    }

    async fn stato(&self, percorso: &str, chiave: &str, corpo: Option<&str>) -> u16 {
        self.chiama(percorso, chiave, corpo).await.0
    }

    async fn rapporto(&self, corpo: &str) -> u16 {
        self.stato("/api/auth/shared/teams", "stretta", Some(corpo)).await
    }

    async fn elenco(&self, chiave: &str) -> Json {
        self.chiama("/api/auth/shared", chiave, None).await.1
    }
}

async fn avvia_con(chiave_teams: &str, adesso: Option<Adesso>, finto: Rc<TrasportoFinto>) -> Avviato {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("elenco-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("tokens.json");
    std::fs::write(
        &file,
        format!(
            r#"{{"accounts":{{"on@example.com":{{"provider":"microsoft","refresh_token":"r","shared":{}}},"off@example.com":{{"provider":"microsoft","refresh_token":"r","shared":{}}},"legacy@example.com":{{"provider":"microsoft","refresh_token":"r"}}}}}}"#,
            shared(r#"["mail","calendar","chat"]"#),
            shared(r#"["chat"]"#)
        ),
    )
    .unwrap();
    let trasporto: Rc<dyn Trasporto> = finto.clone();
    let tokens = Rc::new(GestoreToken::nuovo(file, OpzioniToken { token_posta: None, adesso: None, trasporto, strict: false }));
    let mut ctx = contesto_di(dir.join("accessi.json"), "Europe/Rome", adesso.clone(), Some(Rc::new(|_, _| {})));
    ctx.tokens = Some(tokens.clone());
    let fonti = ["on@example.com", "off@example.com", "legacy@example.com"]
        .iter()
        .map(|a| Fonte::Diretto(CalendarioDiretto::nuovo(format!("microsoft::{a}"), a.to_string(), a, None, ctx.clone(), tokens.clone())))
        .collect();
    let mut o = OpzioniAgente::nuove("piena", fonti, dir.join("battito.json"), "Europe/Rome");
    o.chiave_teams = chiave_teams.to_string();
    o.registro = Rc::new(|_, _| {});
    o.tokens = Some(tokens.clone());
    if let Some(a) = adesso {
        o.adesso = a;
    }
    let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let porta = ascolto.local_addr().unwrap().port();
    tokio::task::spawn_local(servi(ascolto, Rc::new(Agente::nuovo(o))));
    Avviato { dir, porta, tokens, finto }
}

/// Come la prova TypeScript: Microsoft risponde 400 a tutto.
async fn avvia() -> Avviato {
    avvia_con("stretta", None, TrasportoFinto::con_funzione(|_| Ok(json(400, "{}")))).await
}

async fn in_locale(f: impl std::future::Future<Output = ()>) {
    LocalSet::new().run_until(f).await;
}

const NESSUNO: &str = r#"{"accounts":[],"others":[]}"#;

// ---- chiave di TeamsRelay ------------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn spegnere_un_servizio_annulla_l_accesso_guidato_in_attesa() {
    in_locale(async {
        let a = avvia().await;
        a.rapporto(r#"{"accounts":[],"others":[{"email":"on@example.com","slot":4,"state":"ok","detail":"green"}]}"#).await;
        a.chiama("/api/auth/shared", "stretta", Some(r#"{"operation":"connect","account":"on@example.com","capabilities":["mail"]}"#)).await;
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(r#"{"account":"on@example.com","capabilities":["mail","calendar"]}"#)).await, 200);
        let dopo = a.elenco("stretta").await;
        let s = &dopo["signins"][0];
        assert!(s["phase"].as_str() == Some("blocked") && s["error"].as_str().unwrap_or_default().contains("cancelled"), "{}", dopo.in_js());
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn collegare_la_posta_tiene_calendario_e_teams_del_browser_gia_accesi() {
    in_locale(async {
        let a = avvia().await;
        a.rapporto(r#"{"accounts":[],"others":[{"email":"legacy@example.com","slot":4,"state":"ok","detail":"green"}]}"#).await;
        let (_, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"operation":"connect","account":"legacy@example.com","capabilities":["mail"]}"#)).await;
        assert_eq!(r["ok"], Json::Bool(true));
        let caps = r["signin"]["capabilities"].as_arr().unwrap();
        assert!(["mail", "calendar", "chat"].iter().all(|c| caps.contains(&Json::from(*c))), "{}", r.in_js());
        assert_eq!(r["signin"]["phase"], Json::from("queued"));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn se_microsoft_rifiuta_il_client_di_famiglia_per_teams_si_passa_dal_browser() {
    in_locale(async {
        // il client di Teams è rifiutato, Outlook Mobile no; nessun refresh token nuovo, come nella prova TypeScript che non rinnova davvero
        let finto = TrasportoFinto::con_funzione(|r| {
            if campo(r, "client_id") == TEAMS_APP_CLIENT_ID {
                return Ok(json(400, r#"{"error":"unauthorized_client"}"#));
            }
            let aud = campo(r, "scope").split("/.default").next().unwrap_or_default().to_string();
            Ok(json(200, &format!(r#"{{"access_token":"{}","expires_in":3600}}"#, token("legacy@example.com", &aud))))
        });
        let a = avvia_con("stretta", None, finto).await;
        a.rapporto(NESSUNO).await;
        let (_, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"operation":"connect","account":"legacy@example.com","capabilities":["mail","chat"]}"#)).await;
        assert!(r["ok"] == Json::Bool(true) && r["signin"]["phase"] == Json::from("queued"), "{}", r.in_js());
        let voce = a.tokens.voce_account("legacy@example.com").unwrap();
        assert_eq!(voce.borrow()["refresh_token"], Json::from("r"));
        // la casella resta com'era: nessuna condivisione lasciata a metà
        assert!(voce.borrow().get("shared").is_none());
        assert!(a.finto.richieste().iter().any(|r| campo(r, "client_id") == TEAMS_APP_CLIENT_ID));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn accesso_guidato_senza_credenziali_avanzamento_correlato_importazione_rifiutata() {
    in_locale(async {
        let a = avvia().await;
        a.rapporto(NESSUNO).await;
        let (stato, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"operation":"connect","account":" NEW@EXAMPLE.COM ","capabilities":["mail","chat"]}"#)).await;
        assert_eq!(stato, 200);
        let s = r["signin"].clone();
        assert_eq!((s["account"].clone(), s["phase"].clone()), (Json::from("new@example.com"), Json::from("queued")));
        assert_eq!(a.elenco("stretta").await["signins"].as_arr().unwrap().len(), 1);
        let corpo = |extra: &str, account: &str| format!(r#"{{"account":"{account}","id":{}{extra}}}"#, s["id"].in_js());
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(&corpo(r#","operation":"signin-progress","phase":"browser","slot":4"#, "other@example.com"))).await, 400);
        assert_eq!(
            a.stato("/api/auth/shared", "stretta", Some(&corpo(r#","operation":"signin-progress","phase":"browser","slot":4,"url":"https://teams.example/api/desktop/4""#, "new@example.com"))).await,
            200
        );
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(&corpo(r#","operation":"signin-import","slot":5,"sessione":{}"#, "new@example.com"))).await, 400);
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn la_chiave_di_teamsrelay_annulla_solo_l_accesso_di_quell_indirizzo() {
    in_locale(async {
        let a = avvia().await;
        a.rapporto(NESSUNO).await;
        let (_, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"operation":"connect","account":"new@example.com","capabilities":["mail","chat"]}"#)).await;
        let id = r["signin"]["id"].in_js();
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(&format!(r#"{{"operation":"signin-cancel","account":"other@example.com","id":{id}}}"#))).await, 400);
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(&format!(r#"{{"operation":"signin-cancel","account":"new@example.com","id":{id}}}"#))).await, 200);
        let s = a.elenco("stretta").await["signins"][0].clone();
        assert_eq!((s["phase"].clone(), s["error"].clone()), (Json::from("blocked"), Json::from("Connection cancelled.")));
        assert!(s["ended"].as_f64().is_some_and(|n| n > 0.0));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn la_chiave_stretta_apre_solo_elenco_rapporto_e_token_della_chat() {
    in_locale(async {
        let a = avvia().await;
        assert_eq!(a.stato("/api/auth/shared", "stretta", None).await, 200);
        assert_eq!(a.rapporto(NESSUNO).await, 200);
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(r#"{"account":"legacy@example.com","enabled":false}"#)).await, 200);
        // la richiesta passa l'autorizzazione: il rifiuto viene dal broker (Microsoft finto), non dalla chiave
        assert_eq!(a.stato("/api/token-resource?account=on@example.com&resource=csa", "stretta", None).await, 403);
        for p in [
            "/api/token-resource?account=on@example.com&resource=outlook",
            // lo stesso parametro due volte, un frammento, un nome codificato: due letture non danno mai ragione a outlook
            "/api/token-resource?account=on@example.com&resource=outlook&resource=csa",
            "/api/token-resource?account=on@example.com&resource=outlook#&resource=csa",
            "/api/token-resource?account=on@example.com&resource=%6Futlook",
            "/api/token-posta",
            "/api/communication/accounts",
            "/api/auth/stato",
            "/api/calendari",
            "/api/salute",
        ] {
            assert_eq!(a.stato(p, "stretta", None).await, 401, "{p}");
        }
        for p in ["/api/communication/read", "/api/auth/rimuovi", "/api/auth/inizio", "/api/communication/gmail/read"] {
            assert_eq!(a.stato(p, "stretta", Some(r#"{"account":"on@example.com"}"#)).await, 401, "{p}");
        }
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn la_chiave_piena_apre_tutto_e_senza_teams_broker_key_teamsrelay_non_entra() {
    in_locale(async {
        let a = avvia_con("", None, TrasportoFinto::con_funzione(|_| Ok(json(400, "{}")))).await;
        assert_eq!(a.stato("/api/auth/shared", "piena", None).await, 200);
        assert_eq!(a.stato("/api/token-posta", "piena", None).await, 200);
        assert_eq!(a.stato("/api/auth/shared", "", None).await, 401);
        assert_eq!(a.stato("/api/auth/shared", "stretta", None).await, 401);
    })
    .await;
}

// ---- rapporto degli account Teams --------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn l_elenco_mostra_gli_account_teams_finche_il_rapporto_e_recente() {
    in_locale(async {
        let ora = Rc::new(Cell::new(1_000_000_000_000i64));
        let o2 = ora.clone();
        let a = avvia_con("stretta", Some(Rc::new(move || o2.get())), TrasportoFinto::con_funzione(|_| Ok(json(400, "{}")))).await;
        let rapporto = r#"{"accounts":[{"account":"ON@example.com","slot":3,"state":"running","detail":"ok"}],"others":[{"email":"browser@example.com","slot":4,"state":"running","detail":""},{"email":"","slot":5,"state":"starting","detail":""}]}"#;
        assert_eq!(a.rapporto(rapporto).await, 200);
        let elenco = a.elenco("piena").await;
        let teams_di = |e: &Json, account: &str| e["accounts"].as_arr().unwrap().iter().find(|x| x["account"].as_str() == Some(account)).map(|x| x["teams"].clone()).unwrap();
        assert_eq!(canonico(&teams_di(&elenco, "on@example.com")), canonico(&j(r#"{"slot":3,"state":"running","detail":"ok"}"#)));
        assert_eq!(teams_di(&elenco, "legacy@example.com"), Json::Null);
        assert_eq!(canonico(&elenco["teamsOnly"]), canonico(&j(r#"[{"email":"browser@example.com","slot":4,"state":"running","detail":""},{"email":"","slot":5,"state":"starting","detail":""}]"#)));
        ora.set(ora.get() + (STANTIO_S + 1) * 1000);
        let vecchio = a.elenco("piena").await;
        assert!(vecchio["accounts"].as_arr().unwrap().iter().all(|x| x["teams"] == Json::Null));
        assert_eq!(vecchio["teamsOnly"].in_js(), "[]");
    })
    .await;
}

#[test]
fn il_rapporto_rifiuta_forme_inattese() {
    let troppe = format!(r#"{{"others":[{}]}}"#, (1..=65).map(|i| format!(r#"{{"email":"","slot":{i},"state":"running"}}"#)).collect::<Vec<_>>().join(","));
    for (nome, corpo) in [
        ("un campo in più", r#"{"accounts":[],"others":[],"extra":1}"#.to_string()),
        ("un indirizzo non valido", r#"{"accounts":[{"account":"nope","slot":1,"state":"running"}]}"#.to_string()),
        ("uno slot non intero", r#"{"accounts":[{"account":"a@example.com","slot":1.5,"state":"running"}]}"#.to_string()),
        ("uno stato che non è una parola", r#"{"accounts":[{"account":"a@example.com","slot":1,"state":"<b>"}]}"#.to_string()),
        ("un account senza indirizzo", r#"{"accounts":[{"account":"","slot":1,"state":"running"}]}"#.to_string()),
        ("troppe voci", troppe),
    ] {
        assert!(leggi_rapporto_teams(&j(&corpo), 0).is_err(), "{nome}");
    }
}

#[test]
fn tiene_calls_solo_se_e_un_booleano() {
    let r = leggi_rapporto_teams(
        &j(r#"{"accounts":[{"account":"a@example.com","slot":1,"state":"ok","calls":false},{"account":"b@example.com","slot":2,"state":"ok","calls":"no"}],"others":[{"email":"c@example.com","slot":3,"state":"ok","calls":true}]}"#),
        0,
    )
    .unwrap();
    assert_eq!(r.accounts["a@example.com"], StatoTeams { slot: 1, state: "ok".into(), detail: String::new(), calls: Some(false) });
    assert_eq!(r.accounts["b@example.com"], StatoTeams { slot: 2, state: "ok".into(), detail: String::new(), calls: None });
    assert_eq!((r.others[0].0.as_str(), r.others[0].1.calls), ("c@example.com", Some(true)));
}

#[test]
fn toglie_i_caratteri_di_controllo_dal_dettaglio_e_lo_accorcia() {
    let corpo = format!(r#"{{"accounts":[{{"account":"a@example.com","slot":1,"state":"signed-out","detail":"a\nb{}"}}]}}"#, "x".repeat(300));
    let r = leggi_rapporto_teams(&j(&corpo), 0).unwrap();
    let d = &r.accounts["a@example.com"].detail;
    assert!(d.starts_with("a b"));
    assert_eq!(d.encode_utf16().count(), 200);
}

// ---- capacità calendario spenta ------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn il_calendario_non_legge_ne_mostra_la_casella_senza_capacita_calendario() {
    in_locale(async {
        // Microsoft risponde: token per la risorsa chiesta, un calendario, nessun evento
        let finto = TrasportoFinto::con_funzione(|r| {
            if r.url.contains("/oauth2/v2.0/token") {
                let aud = campo(r, "scope").split("/.default").next().unwrap_or_default().to_string();
                return Ok(json(200, &format!(r#"{{"access_token":"{}","expires_in":3600}}"#, token("x@example.com", &aud))));
            }
            if r.url.contains("/me/calendars?") {
                return Ok(json(200, r#"{"value":[{"Id":"c1","Name":"Calendar","IsDefaultCalendar":true}]}"#));
            }
            if r.url.contains("/calendarview?") {
                return Ok(json(200, r#"{"value":[]}"#));
            }
            Ok(json(404, "{}"))
        });
        let a = avvia_con("stretta", None, finto).await;
        let (_, eventi) = a.chiama("/api/eventi?inizio=2026-11-09T00:00:00&fine=2026-11-10T00:00:00", "piena", None).await;
        assert_eq!(eventi["errori"].in_js(), "[]", "{}", eventi.in_js());
        let (_, calendari) = a.chiama("/api/calendari", "piena", None).await;
        let caselle: Vec<String> = calendari["caselle"].as_arr().unwrap().iter().map(|c| c["account"].as_str().unwrap_or_default().to_string()).collect();
        assert_eq!(caselle, ["on@example.com", "legacy@example.com"]);
        // due letture di eventi (on, legacy) e quattro elenchi di calendari (eventi e riepilogo di ognuna), niente per off
        let richieste = a.finto.richieste();
        assert_eq!(richieste.iter().filter(|r| r.url.contains("/calendarview?")).count(), 2);
        assert_eq!(richieste.iter().filter(|r| r.url.contains("/me/calendars?")).count(), 4);
    })
    .await;
}

// ---- comandi di gestione ---------------------------------------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread")]
async fn la_rimozione_con_la_chiave_stretta_resta_in_coda_fino_al_rapporto() {
    in_locale(async {
        let a = avvia().await;
        a.rapporto(r#"{"accounts":[{"account":"on@example.com","slot":1,"state":"ok"}],"others":[]}"#).await;
        assert_eq!(a.stato("/api/auth/rimuovi", "stretta", Some(r#"{"account":"on@example.com"}"#)).await, 401);
        let (_, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"account":"ON@example.com","operation":"remove"}"#)).await;
        assert!(r["ok"] == Json::Bool(true) && r["pending"] == Json::Bool(true), "{}", r.in_js());
        let stato = a.elenco("stretta").await;
        assert!(!stato["accounts"].as_arr().unwrap().iter().any(|x| x["account"].as_str() == Some("on@example.com")));
        let comandi = stato["commands"].as_arr().unwrap().clone();
        assert_eq!(comandi.len(), 1);
        a.rapporto(&format!(r#"{{"accounts":[],"others":[],"results":[{{"id":{}}}]}}"#, comandi[0]["id"].in_js())).await;
        assert_eq!(a.elenco("stretta").await["commands"].in_js(), "[]");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn nomi_e_loghi_del_calendario_solo_dalla_chiave_piena_e_a_teamsrelay_con_l_elenco() {
    in_locale(async {
        let a = avvia().await;
        let etichette = r#"{"names":{"On@Example.com":"NPO"},"logos":{"NPO":"data:image/png;base64,iVBORw=="},"forwarded":{"NPO":"Bandpey"}}"#;
        assert_eq!(a.stato("/api/auth/shared/labels", "stretta", Some(etichette)).await, 401);
        for cattiva in [
            r#"{"names":{"On@Example.com":"NPO"},"logos":{"NPO":"data:image/png;base64,iVBORw=="},"forwarded":{"NPO":"Bandpey"},"extra":1}"#,
            r#"{"names":{"On@Example.com":"NPO"},"logos":{"NPO":"https://example.com/x.png"},"forwarded":{"NPO":"Bandpey"}}"#,
            r#"{"names":{"On@Example.com":"NPO"},"logos":{"NPO":"data:text/html;base64,PGI+"},"forwarded":{"NPO":"Bandpey"}}"#,
            r#"{"names":{"nope":"NPO"},"logos":{"NPO":"data:image/png;base64,iVBORw=="},"forwarded":{"NPO":"Bandpey"}}"#,
            r#"{"names":{"a@example.com":"a\nb"},"logos":{"NPO":"data:image/png;base64,iVBORw=="},"forwarded":{"NPO":"Bandpey"}}"#,
        ] {
            assert_eq!(a.stato("/api/auth/shared/labels", "piena", Some(cattiva)).await, 400, "{cattiva}");
        }
        assert_eq!(a.elenco("stretta").await["labels"], Json::Null);
        assert_eq!(a.stato("/api/auth/shared/labels", "piena", Some(etichette)).await, 200);
        assert_eq!(
            canonico(&a.elenco("stretta").await["labels"]),
            canonico(&j(r#"{"names":{"on@example.com":"NPO"},"logos":{"NPO":"data:image/png;base64,iVBORw=="},"forwarded":{"NPO":"Bandpey"}}"#))
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn il_cambio_delle_chiamate_solo_per_chi_il_rapporto_dice_che_puo() {
    in_locale(async {
        let a = avvia().await;
        a.rapporto(r#"{"accounts":[{"account":"on@example.com","slot":1,"state":"ok","calls":true},{"account":"legacy@example.com","slot":2,"state":"ok"}],"others":[]}"#).await;
        for cattiva in [
            r#"{"account":"legacy@example.com","operation":"calls","on":false}"#,
            r#"{"account":"on@example.com","operation":"calls"}"#,
            r#"{"account":"on@example.com","operation":"calls","on":"off"}"#,
        ] {
            assert_eq!(a.stato("/api/auth/shared", "stretta", Some(cattiva)).await, 400, "{cattiva}");
        }
        let (_, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"account":"ON@example.com","operation":"calls","on":false}"#)).await;
        assert!(r["ok"] == Json::Bool(true) && r["pending"] == Json::Bool(true), "{}", r.in_js());
        let stato = a.elenco("stretta").await;
        let comandi = stato["commands"].as_arr().unwrap();
        assert_eq!(comandi.len(), 1);
        assert_eq!((comandi[0]["account"].clone(), comandi[0]["operation"].clone(), comandi[0]["on"].clone()), (Json::from("on@example.com"), Json::from("calls"), Json::Bool(false)));
        let teams = stato["accounts"].as_arr().unwrap().iter().find(|x| x["account"].as_str() == Some("on@example.com")).unwrap()["teams"].clone();
        assert_eq!((teams["slot"].clone(), teams["calls"].clone()), (Json::from(1), Json::Bool(true)));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn account_solo_browser_fermo_ripresa_e_rimozione_identita_sconosciute_e_rapporti_vecchi() {
    in_locale(async {
        let a = avvia().await;
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(r#"{"account":"browser@example.com","operation":"remove"}"#)).await, 409);
        a.rapporto(r#"{"accounts":[],"others":[{"email":"browser@example.com","slot":4,"state":"ok"}]}"#).await;
        assert_eq!(a.stato("/api/auth/shared", "stretta", Some(r#"{"account":"other@example.com","operation":"remove"}"#)).await, 404);
        let (_, r) = a.chiama("/api/auth/shared", "stretta", Some(r#"{"account":"browser@example.com","operation":"chat","on":false}"#)).await;
        assert!(r["ok"] == Json::Bool(true) && r["pending"] == Json::Bool(true), "{}", r.in_js());
        let c = a.elenco("stretta").await["commands"][0].clone();
        assert_eq!((c["account"].clone(), c["operation"].clone(), c["on"].clone()), (Json::from("browser@example.com"), Json::from("chat"), Json::Bool(false)));
    })
    .await;
}
