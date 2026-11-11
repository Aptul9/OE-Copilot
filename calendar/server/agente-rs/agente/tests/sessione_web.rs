//! Le prove di `agente-ts/test/sessione-web.test.ts`: l'accesso Microsoft dalla sessione del browser (client web di Outlook), l'accesso
//! guidato che ripiega da Outlook Mobile, i rinnovi e i controlli sul token restituito.

use oe_agente::diretto::{GestoreToken, OUTLOOK_CLIENT_ID, OpzioniToken};
use oe_nucleo::json::{Json, Mappa};
use oe_rete::errori::ErroreAccesso;
use oe_rete::sessione_web::{OUTLOOK_WEB_CLIENT_ID, OUTLOOK_WEB_REDIRECT_URI, leggi_sessione};
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::{EsitoRete, Richiesta, Risposta, Trasporto};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use url::Url;

const CASELLA: &str = "mario.rossi@msc.example";

fn ora_ms() -> i64 {
    "2026-11-08T08:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn ora_s() -> i64 {
    ora_ms() / 1000
}

fn j(t: &str) -> Json {
    Json::analizza(t).unwrap()
}

fn fake_jwt(claims: &Json) -> String {
    use base64::Engine;
    let b = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.sig", b(r#"{"alg":"none","typ":"JWT"}"#), b(&claims.in_js()))
}

/// Il token di Outlook della casella; `altro` sostituisce un campo, `None` lo toglie (`{...base, campo: undefined}`).
fn jwt(altro: &[(&str, Option<Json>)]) -> String {
    let mut m = Mappa::new();
    m.insert("upn", CASELLA);
    m.insert("tid", "tid-msc");
    m.insert("aud", "https://outlook.office.com");
    m.insert("exp", ora_s() + 3600);
    m.insert("scp", "Mail.ReadWrite Calendars.ReadWrite");
    for (k, v) in altro {
        match v {
            Some(v) => {
                m.insert(*k, v.clone());
            }
            None => {
                m.remove(*k);
            }
        }
    }
    fake_jwt(&Json::Obj(m))
}

fn con(k: &'static str, v: impl Into<Json>) -> (&'static str, Option<Json>) {
    (k, Some(v.into()))
}

fn sessione_con(cookies: &str) -> Json {
    let sep = if cookies.is_empty() { "" } else { "," };
    j(&format!(
        r#"{{"userAgent":"Mozilla/5.0 Chrome","loginHint":"{CASELLA}","cookies":[{cookies}{sep}{{"name":"altro","value":"x","domain":"teams.microsoft.com","path":"/","expires":-1}}]}}"#
    ))
}

fn sessione() -> Json {
    sessione_con(&format!(r#"{{"name":"ESTSAUTHPERSISTENT","value":"ests-1","domain":".login.microsoftonline.com","path":"/","expires":{}}}"#, ora_s() + 86400))
}

const ROTATO: &str = "ESTSAUTHPERSISTENT=ests-2; domain=.login.microsoftonline.com; path=/; max-age=7776000; secure; HttpOnly";

#[derive(Default, Clone)]
struct Opzioni {
    errore_authorize: Option<&'static str>,
    rifiuta_refresh: bool,
    token_accesso: Option<String>,
    token_refresh: Option<String>,
    stato_codice: Option<u16>,
    set_cookie: Option<&'static str>,
}

fn campo(r: &Richiesta, k: &str) -> Option<String> {
    let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
    form_urlencoded::parse(corpo.as_bytes()).find(|(n, _)| n == k).map(|(_, v)| v.into_owned())
}

/// Microsoft finto: authorize risponde col codice (o con l'errore dato), token scambia codice e refresh_token.
fn rispondi(o: &Opzioni, r: &Richiesta, gettoni: &Cell<u32>) -> EsitoRete {
    if r.url.contains("/oauth2/v2.0/authorize?") {
        let q = Url::parse(&r.url).unwrap();
        let state = q.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.into_owned()).unwrap_or_default();
        let mut ritorno = Url::parse(OUTLOOK_WEB_REDIRECT_URI).unwrap();
        match o.errore_authorize {
            Some(e) => ritorno.query_pairs_mut().append_pair("state", &state).append_pair("error", e),
            None => ritorno.query_pairs_mut().append_pair("state", &state).append_pair("code", "codice-1"),
        };
        let mut risposta = Risposta::nuova(302, "").con_intestazione("Location", ritorno.to_string());
        if let Some(c) = o.set_cookie {
            risposta = risposta.con_intestazione("Set-Cookie", c);
        }
        return Ok(risposta);
    }
    if r.url.contains("/oauth2/v2.0/token") {
        // contano tutte le chiamate a /token, anche quelle rifiutate
        gettoni.set(gettoni.get() + 1);
        let grant = campo(r, "grant_type").unwrap_or_default();
        if grant == "refresh_token" && o.rifiuta_refresh {
            return Ok(json(400, r#"{"error":"invalid_grant"}"#));
        }
        if let (Some(s), "authorization_code") = (o.stato_codice, grant.as_str()) {
            return Ok(json(s, r#"{"error":"temporarily_unavailable"}"#));
        }
        let token = match (&o.token_refresh, grant.as_str()) {
            (Some(t), "refresh_token") => t.clone(),
            _ => o.token_accesso.clone().unwrap_or_else(|| jwt(&[])),
        };
        let mut m = Mappa::new();
        m.insert("access_token", token);
        m.insert("refresh_token", format!("rt-{}", gettoni.get()));
        m.insert("expires_in", 3600);
        return Ok(json(200, &Json::Obj(m).in_js()));
    }
    Err(oe_rete::trasporto::ErroreRete::Rete(format!("URL inatteso: {}", r.url)))
}

fn microsoft_finto(o: Opzioni) -> Rc<TrasportoFinto> {
    let gettoni = Cell::new(0);
    TrasportoFinto::con_funzione(move |r| rispondi(&o, r, &gettoni))
}

struct Archivio {
    dir: PathBuf,
    file: PathBuf,
}

impl Drop for Archivio {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn archivio(contenuto: Option<&Json>) -> Archivio {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("sessione-web-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("tokens.json");
    if let Some(c) = contenuto {
        std::fs::write(&file, c.in_js()).unwrap();
    }
    Archivio { dir, file }
}

fn gestore(a: &Archivio, t: Rc<dyn Trasporto>) -> GestoreToken {
    GestoreToken::nuovo(a.file.clone(), OpzioniToken { token_posta: None, adesso: Some(Rc::new(ora_ms)), trasporto: t, strict: false })
}

/// La casella del client web con il suo refresh_token scaduto e la sessione già filtrata; `altro` come in `jwt`.
fn voce_web(altro: &[(&str, Option<Json>)]) -> Json {
    let mut v = Mappa::new();
    v.insert("provider", "microsoft");
    v.insert("client_id", OUTLOOK_WEB_CLIENT_ID);
    v.insert("tenant_id", "tid-msc");
    v.insert("refresh_token", "rt-vecchio");
    v.insert("access_token", "scaduto");
    v.insert("expires_at", ora_s() - 10);
    v.insert("sessione", leggi_sessione(&sessione()).unwrap().in_json());
    for (k, x) in altro {
        match x {
            Some(x) => {
                v.insert(*k, x.clone());
            }
            None => {
                v.remove(*k);
            }
        }
    }
    let mut accounts = Mappa::new();
    accounts.insert(CASELLA, Json::Obj(v));
    let mut m = Mappa::new();
    m.insert("accounts", Json::Obj(accounts));
    Json::Obj(m)
}

fn su_disco(a: &Archivio) -> Json {
    j(&std::fs::read_to_string(&a.file).unwrap())["accounts"][CASELLA].clone()
}

fn ultimi(t: &TrasportoFinto) -> Vec<String> {
    t.richieste().iter().map(|r| Url::parse(&r.url).unwrap().path().rsplit('/').next().unwrap_or_default().to_string()).collect()
}

fn nomi_cookie(v: &Json) -> Vec<(String, String)> {
    v["sessione"]["cookies"].as_arr().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["value"].as_str().unwrap().to_string())).collect()
}

fn rotati() -> Vec<(String, String)> {
    vec![("ESTSAUTHPERSISTENT".to_string(), "ests-2".to_string())]
}

#[test]
fn leggi_sessione_tiene_solo_i_cookie_di_login_e_rifiuta_forme_sbagliate() {
    let s = leggi_sessione(&sessione()).unwrap();
    assert_eq!(s.cookies.iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["ESTSAUTHPERSISTENT"]);
    let mut rotta = sessione();
    rotta.as_obj_mut().unwrap().insert("userAgent", "a\r\nb");
    assert!(leggi_sessione(&rotta).is_none());
    assert!(leggi_sessione(&j(&format!(r#"{{"loginHint":"{CASELLA}","cookies":[]}}"#))).is_none());
    assert!(leggi_sessione(&Json::Null).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn importa_la_sessione_con_i_cookie_ruotati_e_il_refresh_token() {
    let a = archivio(None);
    let ms = microsoft_finto(Opzioni { set_cookie: Some(ROTATO), ..Default::default() });
    let g = gestore(&a, ms.clone());
    assert_eq!(g.importa_sessione(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap(), (CASELLA.to_string(), "tid-msc".to_string()));
    let chiamate = ms.richieste();
    let auth = Url::parse(&chiamate[0].url).unwrap();
    let param = |k: &str| auth.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    assert_eq!(auth.path(), "/tid-msc/oauth2/v2.0/authorize");
    assert_eq!((param("client_id").as_deref(), param("prompt").as_deref(), param("login_hint").as_deref()), (Some(OUTLOOK_WEB_CLIENT_ID), Some("none"), Some(CASELLA)));
    assert_eq!(chiamate[0].intestazioni.ottieni("cookie").as_deref(), Some("ESTSAUTHPERSISTENT=ests-1"));
    assert_eq!(chiamate[1].intestazioni.ottieni("origin").as_deref(), Some("https://outlook.cloud.microsoft"));
    assert_eq!(campo(&chiamate[1], "code").as_deref(), Some("codice-1"));
    let voce = su_disco(&a);
    assert_eq!((voce["client_id"].clone(), voce["refresh_token"].clone()), (Json::from(OUTLOOK_WEB_CLIENT_ID), Json::from("rt-1")));
    assert_eq!(nomi_cookie(&voce), rotati());
}

#[tokio::test(flavor = "current_thread")]
async fn l_accesso_guidato_ripiega_una_volta_da_mobile_bloccato_e_ricorda_il_web() {
    let a = archivio(None);
    let client: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let c2 = client.clone();
    let gettoni = Cell::new(0);
    let t: Rc<dyn Trasporto> = TrasportoFinto::con_funzione(move |r| {
        let u = Url::parse(&r.url).unwrap();
        if u.path().ends_with("/authorize") {
            let p = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default();
            c2.borrow_mut().push(p("client_id"));
            if p("client_id") == OUTLOOK_CLIENT_ID {
                let mut indietro = Url::parse(&p("redirect_uri")).unwrap();
                indietro.query_pairs_mut().clear().append_pair("state", &p("state")).append_pair("error", "interaction_required").append_pair("error_description", "AADSTS53003: policy blocked");
                return Ok(Risposta::nuova(302, "").con_intestazione("Location", indietro.to_string()));
            }
        }
        rispondi(&Opzioni::default(), r, &gettoni)
    });
    let g = gestore(&a, t.clone());
    let (account, via) = g.importa_guidata(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap();
    assert_eq!((account.as_str(), via.as_str()), (CASELLA, "outlook-web"));
    assert_eq!(*client.borrow(), [OUTLOOK_CLIENT_ID, OUTLOOK_WEB_CLIENT_ID]);
    client.borrow_mut().clear();
    let riavviato = gestore(&a, t);
    riavviato.importa_guidata(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap();
    assert_eq!(*client.borrow(), [OUTLOOK_WEB_CLIENT_ID]);
}

#[tokio::test(flavor = "current_thread")]
async fn l_accesso_guidato_non_trasforma_un_errore_passeggero_in_un_altro_accesso() {
    let a = archivio(None);
    let ms = TrasportoFinto::con_funzione(|_| Ok(Risposta::nuova(503, "")));
    let g = gestore(&a, ms.clone());
    let e = g.importa_guidata(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap_err();
    assert!(e.to_string().contains("503"), "{e}");
    assert_eq!(ms.chiamate(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn un_errore_passeggero_del_token_in_http_400_non_chiede_un_altra_via() {
    let a = archivio(None);
    let auth = Rc::new(Cell::new(0));
    let a2 = auth.clone();
    let g = gestore(
        &a,
        TrasportoFinto::con_funzione(move |r| {
            if r.url.contains("/authorize?") {
                a2.set(a2.get() + 1);
                let u = Url::parse(&r.url).unwrap();
                let p = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default();
                let mut indietro = Url::parse(&p("redirect_uri")).unwrap();
                indietro.query_pairs_mut().clear().append_pair("state", &p("state")).append_pair("code", "fixture");
                return Ok(Risposta::nuova(302, "").con_intestazione("Location", indietro.to_string()));
            }
            Ok(json(400, r#"{"error":"temporarily_unavailable"}"#))
        }),
    );
    let e = g.importa_guidata(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap_err();
    assert!(e.to_string().contains("temporarily unavailable"), "{e}");
    assert_eq!(auth.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn solo_posta_controlla_i_permessi_della_posta_senza_chiedere_il_calendario() {
    let a = archivio(None);
    let g = gestore(&a, microsoft_finto(Opzioni { token_accesso: Some(jwt(&[con("scp", "Mail.Read")])), ..Default::default() }));
    assert_eq!(g.importa_guidata(&sessione(), "tid-msc", None, &["mail"]).await.unwrap().0, CASELLA);
}

#[tokio::test(flavor = "current_thread")]
async fn un_rinnovo_che_fallisce_per_poco_non_segna_la_casella_come_uscita() {
    let a = archivio(Some(&j(&format!(r#"{{"accounts":{{"{CASELLA}":{{"provider":"microsoft","refresh_token":"existing","client_id":"{OUTLOOK_CLIENT_ID}"}}}}}}"#))));
    let g = gestore(&a, TrasportoFinto::con_funzione(|_| Ok(json(400, r#"{"error":"temporarily_unavailable"}"#))));
    let e = g.ottieni_access_token(CASELLA, false).await.unwrap_err();
    assert!(e.to_string().contains("temporarily unavailable"), "{e}");
    assert!(g.voce_account(CASELLA).unwrap().borrow().get("errore").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn importa_solo_se_tornano_casella_risorsa_e_calendari() {
    for (token, testo) in [(jwt(&[con("upn", "altra@msc.example")]), "altra casella"), (jwt(&[con("aud", "https://graph.microsoft.com")]), "altra risorsa"), (jwt(&[con("scp", "Mail.Read")]), "calendari")] {
        let a = archivio(None);
        let g = gestore(&a, microsoft_finto(Opzioni { token_accesso: Some(token), ..Default::default() }));
        let e = g.importa_sessione(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap_err();
        assert!(e.to_string().contains(testo), "{testo}: {e}");
        assert!(!a.file.exists(), "{testo}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn importazione_respinta_errore_leggibile_e_niente_salvato() {
    let a = archivio(None);
    let g = gestore(&a, microsoft_finto(Opzioni { errore_authorize: Some("interaction_required"), ..Default::default() }));
    let errore = |e: Result<(String, String), ErroreAccesso>| e.unwrap_err().to_string();
    assert!(errore(g.importa_sessione(&sessione(), "tid-msc", None, &["calendar"]).await).contains("Accesso Microsoft non riuscito"));
    assert!(errore(g.importa_sessione(&sessione_con(""), "tid-msc", None, &["calendar"]).await).contains("Sessione Microsoft finita"));
    assert!(errore(g.importa_sessione(&sessione(), "../common", None, &["calendar"]).await).contains("Tenant"));
    assert!(!a.file.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn rinnova_col_refresh_token_del_client_web_e_l_origin_di_outlook_web() {
    let a = archivio(Some(&voce_web(&[])));
    let ms = microsoft_finto(Opzioni::default());
    let g = gestore(&a, ms.clone());
    assert_eq!(g.ottieni_access_token(CASELLA, false).await.unwrap(), jwt(&[]));
    let c = ms.richieste();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].intestazioni.ottieni("origin").as_deref(), Some("https://outlook.cloud.microsoft"));
    assert_eq!((campo(&c[0], "client_id").as_deref(), campo(&c[0], "refresh_token").as_deref()), (Some(OUTLOOK_WEB_CLIENT_ID), Some("rt-vecchio")));
}

#[tokio::test(flavor = "current_thread")]
async fn refresh_token_rifiutato_nuovo_accesso_dalla_sessione() {
    let a = archivio(Some(&voce_web(&[])));
    let ms = microsoft_finto(Opzioni { rifiuta_refresh: true, ..Default::default() });
    let g = gestore(&a, ms.clone());
    assert_eq!(g.ottieni_access_token(CASELLA, false).await.unwrap(), jwt(&[]));
    assert_eq!(ultimi(&ms), ["token", "authorize", "token"]);
    let voce = su_disco(&a);
    assert_eq!(voce["refresh_token"], Json::from("rt-2"));
    assert!(voce.get("errore").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn sessione_finita_anche_lei_accesso_scaduto_con_il_motivo() {
    let a = archivio(Some(&voce_web(&[])));
    let g = gestore(&a, microsoft_finto(Opzioni { rifiuta_refresh: true, errore_authorize: Some("login_required"), ..Default::default() }));
    assert!(matches!(g.ottieni_access_token(CASELLA, false).await, Err(ErroreAccesso::Scaduto(_))));
    assert!(su_disco(&a)["errore"].as_str().unwrap_or_default().contains("Sessione del browser scaduta"));
}

#[tokio::test(flavor = "current_thread")]
async fn la_sessione_di_un_altra_casella_non_sostituisce_il_token() {
    let a = archivio(Some(&voce_web(&[("refresh_token", None)])));
    let g = gestore(&a, microsoft_finto(Opzioni { token_accesso: Some(jwt(&[con("upn", "altra@msc.example")])), ..Default::default() }));
    assert!(g.ottieni_access_token(CASELLA, false).await.unwrap_err().to_string().contains("altra casella"));
    assert_eq!(su_disco(&a)["access_token"], Json::from("scaduto"));
}

#[tokio::test(flavor = "current_thread")]
async fn il_recupero_di_una_casella_mobile_ricorda_il_client_web_quando_la_policy_lo_impone() {
    let a = archivio(Some(&voce_web(&[con("client_id", OUTLOOK_CLIENT_ID)])));
    let g = gestore(&a, microsoft_finto(Opzioni { rifiuta_refresh: true, ..Default::default() }));
    g.ottieni_access_token(CASELLA, false).await.unwrap();
    assert_eq!(su_disco(&a)["client_id"], Json::from(OUTLOOK_WEB_CLIENT_ID));
}

fn token_sbagliati() -> Vec<(String, &'static str)> {
    vec![
        (jwt(&[con("upn", "altra@msc.example")]), "altra casella"),
        (jwt(&[con("aud", "https://graph.microsoft.com")]), "altra risorsa"),
        (jwt(&[con("exp", ora_s() - 1)]), "scaduto"),
        (jwt(&[("exp", None)]), "scaduto"),
        (jwt(&[con("scp", "Mail.ReadWrite")]), "calendari"),
        ("non-un-jwt".to_string(), "altra casella"),
    ]
}

#[tokio::test(flavor = "current_thread")]
async fn importazione_un_token_gia_scaduto_e_rifiutato() {
    let a = archivio(None);
    let g = gestore(&a, microsoft_finto(Opzioni { token_accesso: Some(jwt(&[con("exp", ora_s() - 1)])), ..Default::default() }));
    assert!(g.importa_sessione(&sessione(), "tid-msc", None, &["calendar"]).await.unwrap_err().to_string().contains("scaduto"));
    assert!(!a.file.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn rinnovo_web_senza_sessione_un_token_sbagliato_non_si_salva() {
    for (token, testo) in token_sbagliati() {
        let a = archivio(Some(&voce_web(&[("sessione", None)])));
        let g = gestore(&a, microsoft_finto(Opzioni { token_refresh: Some(token), ..Default::default() }));
        let e = g.ottieni_access_token(CASELLA, false).await.unwrap_err();
        assert!(matches!(&e, ErroreAccesso::Scaduto(m) if m.contains(testo)), "{testo}: {e:?}");
        let voce = su_disco(&a);
        assert_eq!((voce["access_token"].clone(), voce["refresh_token"].clone(), voce["expires_at"].clone()), (Json::from("scaduto"), Json::from("rt-vecchio"), Json::from(ora_s() - 10)), "{testo}");
        assert!(voce["errore"].as_str().unwrap_or_default().contains(testo), "{testo}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rinnovo_web_con_un_token_sbagliato_ripiega_sulla_sessione() {
    let a = archivio(Some(&voce_web(&[])));
    let ms = microsoft_finto(Opzioni { token_refresh: Some(jwt(&[con("aud", "https://graph.microsoft.com")])), ..Default::default() });
    let g = gestore(&a, ms.clone());
    assert_eq!(g.ottieni_access_token(CASELLA, false).await.unwrap(), jwt(&[]));
    assert_eq!(ultimi(&ms), ["token", "authorize", "token"]);
    let voce = su_disco(&a);
    assert_eq!((voce["access_token"].clone(), voce["refresh_token"].clone(), voce.get("errore").cloned()), (Json::from(jwt(&[]).as_str()), Json::from("rt-2"), None));
}

#[tokio::test(flavor = "current_thread")]
async fn nuovo_accesso_dalla_sessione_stessi_controlli_e_cookie_ruotati_salvati() {
    for (token, testo) in token_sbagliati() {
        let a = archivio(Some(&voce_web(&[])));
        let g = gestore(&a, microsoft_finto(Opzioni { rifiuta_refresh: true, token_accesso: Some(token), set_cookie: Some(ROTATO), ..Default::default() }));
        assert!(matches!(g.ottieni_access_token(CASELLA, false).await, Err(ErroreAccesso::Scaduto(_))), "{testo}");
        let voce = su_disco(&a);
        assert_eq!((voce["access_token"].clone(), voce["refresh_token"].clone()), (Json::from("scaduto"), Json::from("rt-vecchio")), "{testo}");
        assert!(voce["errore"].as_str().unwrap_or_default().contains(testo), "{testo}");
        assert_eq!(nomi_cookie(&voce), rotati(), "{testo}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn errore_passeggero_dopo_l_autorizzazione_i_cookie_ruotati_restano() {
    let a = archivio(Some(&voce_web(&[])));
    let g = gestore(&a, microsoft_finto(Opzioni { rifiuta_refresh: true, stato_codice: Some(503), set_cookie: Some(ROTATO), ..Default::default() }));
    let e = g.ottieni_access_token(CASELLA, false).await.unwrap_err();
    assert!(!matches!(e, ErroreAccesso::Scaduto(_)) && e.to_string().contains("non disponibile"), "{e:?}");
    let voce = su_disco(&a);
    assert!(voce.get("errore").is_none());
    assert_eq!(voce["access_token"], Json::from("scaduto"));
    assert_eq!(nomi_cookie(&voce), rotati());
}

#[tokio::test(flavor = "current_thread")]
async fn la_sessione_salvata_ripassa_dal_filtro() {
    let sporca = j(&format!(
        r#"{{"userAgent":"Mozilla/5.0 Chrome","loginHint":"{CASELLA}","cookies":[{{"name":"ESTSAUTHPERSISTENT","value":"ests-1","domain":".login.microsoftonline.com","path":"/","expires":{}}},{{"name":"rubami","value":"x","domain":"evil.example","path":"/","expires":-1}},{{"name":"iniezione","value":"a; ESTSAUTH=falso","domain":"login.microsoftonline.com","path":"/","expires":-1}},{{"name":"senza-scadenza","value":"x","domain":"login.microsoftonline.com","path":"/"}}]}}"#,
        ora_s() + 86400
    ));
    let a = archivio(Some(&voce_web(&[("refresh_token", None), ("sessione", Some(sporca))])));
    let ms = microsoft_finto(Opzioni::default());
    let g = gestore(&a, ms.clone());
    assert_eq!(g.ottieni_access_token(CASELLA, false).await.unwrap(), jwt(&[]));
    assert_eq!(ms.richieste()[0].intestazioni.ottieni("cookie").as_deref(), Some("ESTSAUTHPERSISTENT=ests-1"));
    assert_eq!(nomi_cookie(&su_disco(&a)).into_iter().map(|c| c.0).collect::<Vec<_>>(), ["ESTSAUTHPERSISTENT"]);
}

#[tokio::test(flavor = "current_thread")]
async fn sessione_salvata_rotta_o_di_un_altra_casella_nessuna_chiamata() {
    let buona = leggi_sessione(&sessione()).unwrap().in_json();
    let mut altra = buona.clone();
    altra.as_obj_mut().unwrap().insert("loginHint", "altra@msc.example");
    let mut rotta = buona.clone();
    rotta.as_obj_mut().unwrap().insert("userAgent", "a\r\nb");
    for s in [altra, rotta, j(r#"{"cookies":"x"}"#)] {
        let a = archivio(Some(&voce_web(&[("refresh_token", None), ("sessione", Some(s.clone()))])));
        let ms = microsoft_finto(Opzioni::default());
        let g = gestore(&a, ms.clone());
        let e = g.ottieni_access_token(CASELLA, false).await.unwrap_err();
        assert!(e.to_string().contains("Sessione del browser non valida"), "{e}");
        assert_eq!(ms.chiamate(), 0, "{}", s.in_js());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn le_caselle_di_outlook_mobile_non_controllano_il_token_rinnovato() {
    let a = archivio(Some(&voce_web(&[("client_id", None), ("sessione", None)])));
    let altro = jwt(&[con("aud", "https://graph.microsoft.com"), con("scp", "Mail.Read")]);
    let g = gestore(&a, microsoft_finto(Opzioni { token_refresh: Some(altro.clone()), ..Default::default() }));
    assert_eq!(g.ottieni_access_token(CASELLA, false).await.unwrap(), altro);
    assert_eq!(su_disco(&a)["refresh_token"], Json::from("rt-1"));
}

#[tokio::test(flavor = "current_thread")]
async fn le_caselle_di_outlook_mobile_non_cambiano_niente_origin_e_nessuna_sessione() {
    let a = archivio(Some(&voce_web(&[("client_id", None), ("sessione", None)])));
    let ms = microsoft_finto(Opzioni { rifiuta_refresh: true, ..Default::default() });
    let g = gestore(&a, ms.clone());
    assert!(matches!(g.ottieni_access_token(CASELLA, false).await, Err(ErroreAccesso::Scaduto(_))));
    let c = ms.richieste();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].intestazioni.ottieni("origin"), None);
}
