//! Le prove di `agente-ts/test/shared-provisioning.test.ts` (archivio simulato) e `shared-provisioning-gestore.test.ts` (GestoreToken vero,
//! Microsoft simulato).

mod comune;

use comune::canonico;
use indexmap::IndexMap;
use oe_agente::diretto::{GestoreToken, OpzioniToken, Voce};
use oe_agente::shared_provisioning::{ArchivioCondiviso, FutureAccesso, attiva_condivisione, capacita_valide, disattiva_condivisione, origine_di, richiesta_condivisione};
use oe_nucleo::json::Json;
use oe_rete::errori::{ErroreAccesso, EsitoAccesso};
use oe_rete::sessione_web::OUTLOOK_WEB_CLIENT_ID;
use oe_rete::shared_teams::{RapportoTeams, StatoTeams};
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::Trasporto;
use std::cell::RefCell;
use std::rc::Rc;

const TID: &str = "11111111-2222-3333-4444-555555555555";
const OID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

fn jwt(claims: &str) -> String {
    use base64::Engine;
    format!("x.{}.x", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims))
}

fn j(testo: &str) -> Json {
    Json::analizza(testo).unwrap()
}

fn uguale(a: &Json, b: &str) {
    assert_eq!(canonico(a), canonico(&j(b)));
}

#[derive(Default)]
struct Finto {
    voci: IndexMap<String, Voce>,
    token: Option<String>,
    rifiuta: Option<&'static str>,
    scritture: RefCell<Vec<Option<Json>>>,
    richieste: RefCell<Vec<String>>,
    forzati: RefCell<Vec<bool>>,
}

impl Finto {
    fn nuovo(voci: &str) -> Finto {
        let mut m = IndexMap::new();
        for (k, v) in j(voci).as_obj().unwrap().iter() {
            m.insert(k.clone(), Rc::new(RefCell::new(v.as_obj().unwrap().clone())));
        }
        Finto { voci: m, ..Default::default() }
    }

    fn voce(&self, e: &str) -> Json {
        Json::Obj(self.voci[e].borrow().clone())
    }
}

impl ArchivioCondiviso for Finto {
    fn voce_account(&self, email: &str) -> Option<Voce> {
        self.voci.get(&email.to_lowercase()).cloned()
    }

    fn elenco_account(&self) -> Vec<(String, Voce)> {
        self.voci.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn ottieni_access_token<'a>(&'a self, _email: &'a str, forza: bool) -> FutureAccesso<'a> {
        self.forzati.borrow_mut().push(forza);
        let token = self.token.clone().unwrap_or_else(|| jwt(&format!(r#"{{"tid":"{TID}","oid":"{OID}","upn":"Mario@Example.com"}}"#)));
        Box::pin(async move { Ok(token) })
    }

    fn ottieni_risorsa<'a>(&'a self, _email: &'a str, risorsa: &'a str, forza: bool) -> FutureAccesso<'a> {
        assert!(forza);
        self.richieste.borrow_mut().push(risorsa.to_string());
        let rifiutata = self.rifiuta == Some(risorsa);
        Box::pin(async move { if rifiutata { Err(ErroreAccesso::altro("shared renewal refused")) } else { Ok("t".to_string()) } })
    }

    fn imposta_shared(&self, email: &str, shared: Option<Json>) -> EsitoAccesso<()> {
        self.scritture.borrow_mut().push(shared.clone());
        let mut v = self.voci[email].borrow_mut();
        match shared {
            Some(s) => {
                v.insert("shared", s);
            }
            None => {
                v.remove("shared");
            }
        }
        Ok(())
    }
}

async fn post(a: &dyn ArchivioCondiviso, corpo: &str) -> Json {
    richiesta_condivisione(Some(a), Some(&j(corpo)), None).await
}

fn prima(capacita: &str, origine: &str) -> String {
    format!(r#"{{"tenantId":"{TID}","objectId":"{OID}","capabilities":{capacita},"origin":"{origine}"}}"#)
}

#[tokio::test(flavor = "current_thread")]
async fn identita_dal_token_rinnovato_e_prova_di_ogni_risorsa_della_chat() {
    let s = Finto::nuovo(r#"{"mario@example.com":{"provider":"microsoft","refresh_token":"r"}}"#);
    let r = post(&s, r#"{"account":" Mario@Example.com ","capabilities":["chat","mail","chat"],"tenantId":"evil","objectId":"evil"}"#).await;
    uguale(&r, r#"{"ok":true,"account":"mario@example.com","shared":{"capabilities":["mail","chat"],"origin":"mobile"}}"#);
    assert_eq!(*s.forzati.borrow(), [true]);
    assert_eq!(s.scritture.borrow().iter().map(|x| x.as_ref().map(canonico)).collect::<Vec<_>>(), [Some(canonico(&j(&prima(r#"["mail","chat"]"#, "mobile"))))]);
    assert_eq!(*s.richieste.borrow(), ["outlook", "csa", "ic3", "spaces", "presence"]);
}

#[tokio::test(flavor = "current_thread")]
async fn una_capacita_rifiutata_rimette_la_casella_com_era() {
    let s = Finto { rifiuta: Some("ic3"), ..Finto::nuovo(r#"{"mario@example.com":{"provider":"microsoft","refresh_token":"r"}}"#) };
    let e = attiva_condivisione(&s, "mario@example.com", Some(&j(r#"["calendar","chat"]"#))).await.unwrap_err();
    let (c, i) = (e.messaggio.find("chat"), e.messaggio.find("ic3"));
    assert!(c.is_some() && i.is_some() && c < i, "{}", e.messaggio);
    assert_eq!(s.scritture.borrow().last().cloned(), Some(None));
    assert!(s.voce("mario@example.com").get("shared").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn cambiare_le_capacita_tiene_identita_e_origine_senza_un_nuovo_token() {
    let p = prima(r#"["mail","chat"]"#, "outlook-spa");
    let s = Finto { rifiuta: Some("csa"), ..Finto::nuovo(&format!(r#"{{"mario@example.com":{{"provider":"microsoft","refresh_token":"r","shared":{p}}}}}"#)) };
    attiva_condivisione(&s, "mario@example.com", Some(&j(r#"["calendar"]"#))).await.unwrap();
    assert!(s.forzati.borrow().is_empty());
    uguale(&s.voce("mario@example.com")["shared"], &prima(r#"["calendar"]"#, "outlook-spa"));
    // la chat non c'è più: nessuna prova sulle sue risorse, solo quella del calendario aggiunto
    assert_eq!(*s.richieste.borrow(), ["outlook"]);
}

#[tokio::test(flavor = "current_thread")]
async fn togliere_soltanto_non_chiede_niente_a_microsoft() {
    let p = prima(r#"["mail","calendar","chat"]"#, "mobile");
    let s = Finto {
        rifiuta: Some("outlook"),
        ..Finto::nuovo(&format!(
            r#"{{"scaduta@example.com":{{"provider":"microsoft","refresh_token":"r","errore":"Accesso scaduto","shared":{p}}},"senza@example.com":{{"provider":"microsoft","shared":{p}}}}}"#
        ))
    };
    uguale(
        &post(&s, r#"{"account":"scaduta@example.com","capabilities":["mail","calendar"]}"#).await,
        r#"{"ok":true,"account":"scaduta@example.com","shared":{"capabilities":["mail","calendar"],"origin":"mobile"}}"#,
    );
    assert_eq!(post(&s, r#"{"account":"senza@example.com","capabilities":[]}"#).await["ok"], Json::Bool(true));
    uguale(&s.voce("scaduta@example.com")["shared"], &prima(r#"["mail","calendar"]"#, "mobile"));
    uguale(&s.voce("senza@example.com")["shared"], &prima("[]", "mobile"));
    assert!(s.richieste.borrow().is_empty() && s.forzati.borrow().is_empty());
    // aggiungere invece chiede ancora un accesso valido
    assert_eq!(post(&s, r#"{"account":"scaduta@example.com","capabilities":["mail","calendar","chat"]}"#).await["ok"], Json::Bool(false));
    uguale(&s.voce("scaduta@example.com")["shared"], &prima(r#"["mail","calendar"]"#, "mobile"));
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_un_token_non_della_casella_senza_scrivere_nulla() {
    for (nome, token) in [
        ("di un'altra casella", jwt(&format!(r#"{{"tid":"{TID}","oid":"{OID}","upn":"altro@example.com"}}"#))),
        ("senza tenant", jwt(&format!(r#"{{"oid":"{OID}","upn":"mario@example.com"}}"#))),
        ("con object id non GUID", jwt(&format!(r#"{{"tid":"{TID}","oid":"x","upn":"mario@example.com"}}"#))),
        ("che non è un JWT", "opaque".to_string()),
    ] {
        let s = Finto { token: Some(token), ..Finto::nuovo(r#"{"mario@example.com":{"provider":"microsoft","refresh_token":"r"}}"#) };
        assert!(attiva_condivisione(&s, "mario@example.com", Some(&j(r#"["mail"]"#))).await.is_err(), "{nome}");
        assert!(s.scritture.borrow().is_empty(), "{nome}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rifiuta_google_senza_refresh_scaduta_e_sconosciuta() {
    let s = Finto::nuovo(
        r#"{"g@example.com":{"provider":"google","refresh_token":"r"},"n@example.com":{"provider":"microsoft"},"e@example.com":{"provider":"microsoft","refresh_token":"r","errore":"Accesso scaduto"}}"#,
    );
    for account in ["g@example.com", "n@example.com", "e@example.com", "x@example.com"] {
        let corpo = oe_nucleo::ogg! { "account": account, "capabilities": vec!["mail".to_string()] }.in_js();
        assert_eq!(post(&s, &corpo).await["ok"], Json::Bool(false), "{account}");
    }
    assert!(s.scritture.borrow().is_empty());
}

#[test]
fn origine_dal_client_dell_accesso_e_capacita_note() {
    assert_eq!(origine_di(Some(OUTLOOK_WEB_CLIENT_ID)), "outlook-spa");
    assert_eq!(origine_di(Some("5e3ce6c0-2b1f-4285-8d4b-75ee78787346")), "teams-spa");
    assert_eq!(origine_di(None), "mobile");
    for v in [r#"["admin"]"#, r#""mail""#, "null", "[1]"] {
        assert!(capacita_valide(Some(&j(v))).is_err(), "{v}");
    }
    assert_eq!(capacita_valide(Some(&j("[]"))).unwrap(), Vec::<&str>::new());
}

#[tokio::test(flavor = "current_thread")]
async fn nessuna_capacita_resta_condivisa_senza_prove() {
    let p = prima(r#"["mail","chat"]"#, "mobile");
    let s = Finto::nuovo(&format!(r#"{{"mario@example.com":{{"provider":"microsoft","refresh_token":"r","shared":{p}}}}}"#));
    uguale(&post(&s, r#"{"account":"mario@example.com","capabilities":[]}"#).await, r#"{"ok":true,"account":"mario@example.com","shared":{"capabilities":[],"origin":"mobile"}}"#);
    uguale(&s.voce("mario@example.com")["shared"], &prima("[]", "mobile"));
    assert!(s.richieste.borrow().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn get_elenca_le_caselle_microsoft_senza_tenant_e_object_id() {
    let s = Finto::nuovo(&format!(r#"{{"a@example.com":{{"provider":"microsoft","shared":{}}},"b@example.com":{{"provider":"microsoft"}},"g@example.com":{{"provider":"google"}}}}"#, prima(r#"["chat"]"#, "mobile")));
    let r = richiesta_condivisione(Some(&s), None, None).await;
    uguale(
        &r,
        r#"{"ok":true,"accounts":[{"account":"a@example.com","shared":{"capabilities":["chat"],"origin":"mobile"},"status":"ok","message":"","teams":null},{"account":"b@example.com","shared":null,"status":"ok","message":"","teams":null}],"teamsOnly":[],"teamsAt":0}"#,
    );
    assert!(!r.in_js().contains(OID));
}

#[tokio::test(flavor = "current_thread")]
async fn get_unisce_lo_stato_degli_account_teams_e_l_accesso_da_rifare() {
    let s = Finto::nuovo(&format!(r#"{{"a@example.com":{{"provider":"microsoft","shared":{}}},"b@example.com":{{"provider":"microsoft","errore":"Accesso scaduto"}}}}"#, prima(r#"["chat"]"#, "mobile")));
    let stato = |slot| StatoTeams { slot, state: "running".into(), detail: String::new(), calls: None };
    let mut accounts = IndexMap::new();
    accounts.insert("a@example.com".to_string(), stato(3));
    let teams = RapportoTeams { at: 100, accounts, others: vec![("c@example.com".to_string(), stato(5))] };
    let r = richiesta_condivisione(Some(&s), None, Some(&teams)).await;
    uguale(
        &r["accounts"],
        r#"[{"account":"a@example.com","shared":{"capabilities":["chat"],"origin":"mobile"},"status":"ok","message":"","teams":{"slot":3,"state":"running","detail":""}},{"account":"b@example.com","shared":null,"status":"errore","message":"Accesso scaduto","teams":null}]"#,
    );
    uguale(&r["teamsOnly"], r#"[{"email":"c@example.com","slot":5,"state":"running","detail":""}]"#);
    assert_eq!(r["teamsAt"], Json::from(100));
}

#[tokio::test(flavor = "current_thread")]
async fn enabled_false_toglie_i_metadati_e_lascia_la_casella() {
    let s = Finto::nuovo(&format!(r#"{{"a@example.com":{{"provider":"microsoft","refresh_token":"r","shared":{}}}}}"#, prima(r#"["chat"]"#, "mobile")));
    uguale(&post(&s, r#"{"account":"a@example.com","enabled":false}"#).await, r#"{"ok":true,"account":"a@example.com","shared":null}"#);
    assert_eq!(*s.scritture.borrow(), [None]);
    uguale(&s.voce("a@example.com"), r#"{"provider":"microsoft","refresh_token":"r"}"#);
    assert_eq!(disattiva_condivisione(&s, "a@example.com").unwrap()["shared"], Json::Null);
    assert_eq!(s.scritture.borrow().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn senza_gestore_token_risponde_con_l_errore() {
    uguale(&richiesta_condivisione(None, None, None).await, r#"{"ok":false,"errore":"gestore token non configurato"}"#);
}

// ---- con il GestoreToken vero ------------------------------------------------------------------------------------------------
struct Scenario {
    dir: std::path::PathBuf,
    file: std::path::PathBuf,
    gestore: GestoreToken,
    scopes: Rc<RefCell<Vec<String>>>,
}

fn scenario(nome: &str, rifiuta: Option<&'static str>) -> Scenario {
    let dir = std::env::temp_dir().join(format!("shared-prov-{nome}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("tokens.json");
    std::fs::write(&file, format!(r#"{{"accounts":{{"mario@example.com":{{"provider":"microsoft","refresh_token":"r0","tenant_id":"{TID}"}}}}}}"#)).unwrap();
    let scopes: Rc<RefCell<Vec<String>>> = Rc::default();
    let s2 = scopes.clone();
    let finto: Rc<dyn Trasporto> = TrasportoFinto::con_funzione(move |r| {
        let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
        let scope = form_urlencoded::parse(corpo.as_bytes()).find(|(k, _)| k == "scope").map(|(_, v)| v.into_owned()).unwrap_or_default();
        let aud = scope.split("/.default").next().unwrap_or_default().split(' ').next().unwrap_or_default().to_string();
        s2.borrow_mut().push(aud.clone());
        if rifiuta.is_some_and(|x| aud.contains(x)) {
            return Ok(json(400, r#"{"error":"invalid_grant"}"#));
        }
        let audience = if aud.starts_with("https://") { aud.clone() } else { "https://outlook.office.com".to_string() };
        let token = jwt(&format!(r#"{{"aud":"{audience}","tid":"{TID}","oid":"{OID}","upn":"mario@example.com","exp":2000000,"scp":"Mail.ReadWrite Calendars.ReadWrite"}}"#));
        let n = s2.borrow().len();
        Ok(json(200, &format!(r#"{{"access_token":"{token}","refresh_token":"r{n}","expires_in":3600}}"#)))
    });
    let gestore = GestoreToken::nuovo(file.clone(), OpzioniToken { token_posta: None, adesso: Some(Rc::new(|| 1_000_000)), trasporto: finto, strict: false });
    Scenario { dir, file, gestore, scopes }
}

impl Scenario {
    fn salvato(&self) -> Json {
        j(&std::fs::read_to_string(&self.file).unwrap())["accounts"]["mario@example.com"].clone()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn con_il_gestore_salva_identita_e_capacita_dopo_ogni_risorsa() {
    let s = scenario("tutte", None);
    let r = richiesta_condivisione(Some(&s.gestore), Some(&j(r#"{"account":"mario@example.com","capabilities":["mail","calendar","chat"]}"#)), None).await;
    uguale(&r, r#"{"ok":true,"account":"mario@example.com","shared":{"capabilities":["mail","calendar","chat"],"origin":"mobile"}}"#);
    uguale(&s.salvato()["shared"], &prima(r#"["mail","calendar","chat"]"#, "mobile"));
    let scopes = s.scopes.borrow().clone();
    assert!(scopes[0].contains("outlook"), "{scopes:?}");
    assert_eq!(
        scopes[1..],
        ["https://outlook.office.com", "https://chatsvcagg.teams.microsoft.com", "https://ic3.teams.office.com", "https://api.spaces.skype.com", "https://presence.teams.microsoft.com"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn con_il_gestore_la_chat_rifiutata_lascia_la_casella_com_era() {
    let s = scenario("rifiuto", Some("ic3.teams"));
    let r = richiesta_condivisione(Some(&s.gestore), Some(&j(r#"{"account":"mario@example.com","capabilities":["calendar","chat"]}"#)), None).await;
    assert_eq!(r["ok"], Json::Bool(false));
    let errore = r["errore"].as_str().unwrap();
    assert!(errore.find("chat").is_some_and(|c| errore[c..].contains("ic3")), "{errore}");
    assert!(s.salvato().get("shared").is_none());
}
