//! Le strade del gateway su HTTP vero (servizio::Gateway su 127.0.0.1), come le provano web-ts/test/gateway.test.ts,
//! verifica-sessione.test.ts, prenota.test.ts, condividi.test.ts e mcp-posta.test.ts: le pagine con i valori in window.__OE__, i file
//! dell'interfaccia, il cancello dell'API, i loghi, il cancello di Caddy, la disponibilità e la prenotazione pubbliche, i link di
//! condivisione e il server MCP della posta con un Outlook finto.

mod comune;

use comune::*;
use oe_gateway::config::imposta_per_prova as imposta;
use oe_gateway::servizio::Gateway;
use oe_http::{Esito, Gestore, Richiesta};
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use std::cell::RefCell;
use std::rc::Rc;
use tokio::task::LocalSet;

const APP: &str = "https://oe.test";
const EMAIL: &str = "proprietario@oe.test";
const PASSWORD: &str = "password-del-proprietario";
const MCP: &str = "token-mcp-di-prova";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-finto";

/// Le cartelle restano a con_gateway: la prova cattura solo i campi che usa, e una cartella catturata e non usata sparirebbe subito.
struct Prova {
    base: String,
    agente: Rc<Agente>,
    dati: std::path::PathBuf,
}

/// La cartella dell'interfaccia esportata, ridotta: le pagine con la loro testata, un file di /_next/static, il service worker.
fn interfaccia() -> Cartella {
    let c = Cartella::nuova("oe-ui");
    for p in ["index", "login", "sicurezza", "disponibilita", "404"] {
        std::fs::write(c.0.join(format!("{p}.html")), format!("<!DOCTYPE html><html><head><title>{p}</title></head><body>{p}</body></html>")).unwrap();
    }
    std::fs::create_dir_all(c.0.join("_next/static/chunks")).unwrap();
    std::fs::write(c.0.join("_next/static/chunks/a.js"), "console.log(1)").unwrap();
    std::fs::write(c.0.join("index.txt"), "rsc").unwrap();
    std::fs::write(c.0.join("sw.js"), "self.addEventListener('push',()=>{})").unwrap();
    c
}

fn con_gateway<F, Fut>(prova: F)
where
    F: FnOnce(Prova) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    LocalSet::new().block_on(&rt, async {
        let agente = Agente::nuovo();
        let agente_url = ascolta(agente.clone()).await;
        let dati = Cartella::nuova("oe-servizio");
        std::fs::create_dir_all(dati.0.join("ics_cache")).unwrap();
        std::fs::copy(golden().join("ics").join("festivita.ics"), dati.0.join("ics_cache").join("builtin__festivita-it.ics")).unwrap();
        let ui = interfaccia();
        let loghi = Cartella::nuova("oe-loghi");
        std::fs::write(loghi.0.join("personale.png"), PNG).unwrap();
        ambiente(&dati.0, &agente_url);
        imposta("BETTER_AUTH_SECRET", Some(&"s".repeat(48)));
        imposta("APP_URL", Some(APP));
        imposta("AUTH_2FA", Some("off"));
        imposta("ADMIN_EMAIL", Some(EMAIL));
        imposta("ADMIN_PASSWORD", Some(PASSWORD));
        imposta("UI_STATICA", Some(&ui.0.to_string_lossy()));
        imposta("LOGHI_PREDEFINITI", Some(&loghi.0.to_string_lossy()));
        imposta("MCP_TOKEN", Some(MCP));
        oe_gateway::servizio::prepara().await.expect("avvio");
        let base = ascolta(Rc::new(Gateway)).await;
        prova(Prova { base, agente, dati: dati.0.clone() }).await;
        oe_gateway::archivio::chiudi_archivio();
        oe_gateway::db::chiudi_tutto();
        drop((dati, ui, loghi));
    });
}

struct Risp {
    stato: u16,
    intestazioni: reqwest::header::HeaderMap,
    testo: String,
}

impl Risp {
    fn h(&self, k: &str) -> Option<&str> {
        self.intestazioni.get(k).and_then(|v| v.to_str().ok())
    }
    fn json(&self) -> Json {
        Json::analizza(&self.testo).unwrap_or(Json::Null)
    }
    /// I valori della pagina scritti nella testata.
    fn valori(&self) -> Json {
        let i = self.testo.find("<head><script>window.__OE__=").expect("__OE__ nella testata") + 28;
        let f = self.testo[i..].find("</script>").unwrap();
        Json::analizza(&self.testo[i..i + f]).unwrap()
    }
}

async fn chiedi(metodo: &str, url: &str, intestazioni: &[(&str, &str)], corpo: Option<Vec<u8>>) -> Risp {
    let c = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let mut r = c.request(reqwest::Method::from_bytes(metodo.as_bytes()).unwrap(), url);
    for (k, v) in intestazioni {
        r = r.header(*k, *v);
    }
    if let Some(b) = corpo {
        r = r.body(b);
    }
    let res = r.send().await.expect("richiesta");
    let stato = res.status().as_u16();
    let intestazioni = res.headers().clone();
    Risp { stato, intestazioni, testo: res.text().await.unwrap_or_default() }
}

async fn get(url: &str) -> Risp {
    chiedi("GET", url, &[], None).await
}

/// Entra come il proprietario: il cookie della sessione.
async fn accedi(base: &str) -> String {
    let r = chiedi(
        "POST",
        &format!("{base}/api/auth/sign-in/email"),
        &[("origin", APP), ("content-type", "application/json")],
        Some(ogg! { "email": EMAIL, "password": PASSWORD }.in_js().into_bytes()),
    )
    .await;
    assert_eq!(r.stato, 200, "{}", r.testo);
    let riga = r.h("set-cookie").expect("cookie");
    riga.split(';').next().unwrap().to_string()
}

async fn api(base: &str, cookie: &str, metodo: &str, args: &Json) -> Risp {
    chiedi("POST", &format!("{base}/api/{metodo}"), &[("cookie", cookie), ("content-type", "application/json")], Some(args.in_js().into_bytes())).await
}

#[test]
fn salute_barre_e_metodi() {
    con_gateway(|p| async move {
        let r = get(&format!("{}/salute", p.base)).await;
        assert_eq!((r.stato, r.testo.as_str(), r.h("cache-control")), (200, "ok", Some("no-store")));
        let h = chiedi("HEAD", &format!("{}/salute", p.base), &[], None).await;
        assert_eq!((h.stato, h.testo.as_str()), (200, ""));
        assert_eq!(chiedi("POST", &format!("{}/salute", p.base), &[], None).await.h("allow"), Some("GET, HEAD"));
        let b = get(&format!("{}/login/?next=/a", p.base)).await;
        assert_eq!((b.stato, b.h("location")), (308, Some("/login?next=%2Fa")));
        assert_eq!(chiedi("PUT", &format!("{}/api/auth/get-session", p.base), &[], None).await.stato, 405);
        assert_eq!(chiedi("POST", &format!("{}/api/disponibilita", p.base), &[], None).await.stato, 405);
        assert_eq!(get(&format!("{}/api/prenota", p.base)).await.stato, 405);
        assert_eq!(chiedi("POST", &format!("{}/api/verifica-sessione", p.base), &[], None).await.stato, 405);
    });
}

#[test]
fn pagine_senza_e_con_la_sessione() {
    con_gateway(|p| async move {
        let r = get(&format!("{}/", p.base)).await;
        assert_eq!((r.stato, r.h("location")), (307, Some("/login?next=%2F")));
        let l = get(&format!("{}/login?next=/sicurezza", p.base)).await;
        assert_eq!((l.stato, l.h("content-type"), l.h("cache-control")), (200, Some("text/html; charset=utf-8"), Some("private, no-cache, no-store, max-age=0, must-revalidate")));
        assert_eq!(l.valori().in_js(), r#"{"next":"/sicurezza","modo":"off","accessoEsterno":false}"#);
        // un ritorno che esce dal sito torna alla radice
        assert_eq!(get(&format!("{}/login?next=https://evil.test/", p.base)).await.valori()["next"].as_str(), Some("/"));
        let d = get(&format!("{}/disponibilita?gruppo=%20lavoro%20", p.base)).await;
        assert_eq!(d.valori().in_js(), r#"{"gruppo":"lavoro"}"#);
        assert_eq!(get(&format!("{}/disponibilita?gruppo=a&gruppo=b", p.base)).await.valori().in_js(), r#"{"gruppo":""}"#);
        let x = get(&format!("{}/disponibilita?gruppo=%3C/script%3E", p.base)).await;
        assert!(x.testo.contains(r#"{"gruppo":"\u003c/script\u003e"}"#), "{}", x.testo);
        // in off la pagina della sicurezza non c'è
        assert_eq!(get(&format!("{}/sicurezza", p.base)).await.stato, 404);
        assert_eq!(get(&format!("{}/nuova", p.base)).await.h("location"), Some("/"));
        let cookie = accedi(&p.base).await;
        let casa = chiedi("GET", &format!("{}/", p.base), &[("cookie", &cookie)], None).await;
        assert_eq!((casa.stato, casa.valori().in_js()), (200, r#"{"accesso2fa":"off"}"#.to_string()));
        let testa = chiedi("HEAD", &format!("{}/", p.base), &[("cookie", &cookie)], None).await;
        assert_eq!((testa.stato, testa.testo.as_str()), (200, ""));
        let dentro = chiedi("GET", &format!("{}/login?next=/disponibilita", p.base), &[("cookie", &cookie)], None).await;
        assert_eq!((dentro.stato, dentro.h("location")), (307, Some("/disponibilita")));
        imposta("AUTH_2FA", Some("totp"));
        let s = chiedi("GET", &format!("{}/sicurezza", p.base), &[("cookie", &cookie)], None).await;
        assert_eq!(s.valori().in_js(), format!(r#"{{"modo":"totp","email":"{EMAIL}","attivo":false}}"#));
        assert_eq!(get(&format!("{}/sicurezza", p.base)).await.h("location"), Some("/login?next=%2Fsicurezza"));
        imposta("AUTH_2FA", Some("off"));
    });
}

#[test]
fn file_dell_interfaccia() {
    con_gateway(|p| async move {
        let js = get(&format!("{}/_next/static/chunks/a.js", p.base)).await;
        assert_eq!((js.stato, js.h("content-type"), js.h("cache-control")), (200, Some("application/javascript; charset=UTF-8"), Some("public, max-age=31536000, immutable")));
        let sw = get(&format!("{}/sw.js", p.base)).await;
        assert_eq!((sw.stato, sw.h("service-worker-allowed"), sw.h("cache-control")), (200, Some("/"), Some("public, max-age=0")));
        // le pagine e i loro dati passano solo dalle regole delle pagine: senza sessione rimandano all'accesso
        for f in ["/login.html", "/index.txt", "/_next/../index.html", "/%2e%2e/etc/passwd"] {
            let r = get(&format!("{}{f}", p.base)).await;
            assert!(r.stato == 302 || r.stato == 404, "{f}: {}", r.stato);
        }
        let cookie = accedi(&p.base).await;
        let nessuno = chiedi("GET", &format!("{}/non-esiste", p.base), &[("cookie", &cookie)], None).await;
        assert_eq!((nessuno.stato, nessuno.testo.as_str()), (404, r#"{"error":"non trovato"}"#));
        let tel = chiedi("GET", &format!("{}/telefono/", p.base), &[("cookie", &cookie)], None).await;
        assert_eq!(tel.stato, 308);
        assert_eq!(chiedi("GET", &format!("{}/telefono", p.base), &[("cookie", &cookie)], None).await.h("location"), Some("/"));
    });
}

#[test]
fn il_cancello_dell_api() {
    con_gateway(|p| async move {
        let senza = api(&p.base, "x=y", "get_config", &Json::Arr(vec![])).await;
        assert_eq!((senza.stato, senza.testo.as_str()), (401, r#"{"error":"non autenticato"}"#));
        assert_eq!(get(&format!("{}/logos/personale.png", p.base)).await.h("location"), Some("/login?next=%2Flogos%2Fpersonale.png"));
        let c = accedi(&p.base).await;
        let u = |m: &str| format!("{}/api/{m}", p.base);
        assert_eq!(chiedi("POST", &u("get_config"), &[("cookie", &c), ("content-type", "text/plain")], Some(b"[]".to_vec())).await.stato, 415);
        assert_eq!(chiedi("POST", &u("get_config"), &[("cookie", &c), ("content-type", "application/json"), ("sec-fetch-site", "cross-site")], Some(b"[]".to_vec())).await.stato, 403);
        assert_eq!(chiedi("POST", &u("non_esiste"), &[("cookie", &c), ("content-type", "application/json"), ("sec-fetch-site", "cross-site")], Some(b"[]".to_vec())).await.stato, 403);
        let grande = chiedi("POST", &u("get_config"), &[("cookie", &c), ("content-type", "application/json")], Some(vec![b' '; oe_gateway::gateway::MAX_BODY + 1])).await;
        assert_eq!(grande.stato, 413);
        assert_eq!(chiedi("DELETE", &u("get_config"), &[("cookie", &c)], None).await.stato, 405);
        let ok = chiedi("POST", &u("get_config"), &[("cookie", &c), ("content-type", "application/json; charset=utf-8")], None).await;
        assert_eq!(ok.stato, 200);
        assert!(ok.json().as_obj().is_some());
        assert_eq!(api(&p.base, &c, "get_config", &ogg! { "a": 1 }).await.stato, 400);
        assert_eq!(chiedi("POST", &u("get_config"), &[("cookie", &c), ("content-type", "application/json")], Some(b"{non json".to_vec())).await.stato, 400);
        let no = api(&p.base, &c, "non_esiste", &Json::Arr(vec![])).await;
        assert_eq!((no.stato, no.testo.as_str()), (404, r#"{"error":"metodo non disponibile"}"#));
        // un GET non chiama mai un metodo
        assert_eq!(chiedi("GET", &u("get_config"), &[("cookie", &c)], None).await.stato, 404);
        // i metodi che scrivono, letti di nuovo
        let e = api(&p.base, &c, "add_local_event", &Json::Arr(vec![ogg! { "title": "Prova", "start": "2026-11-20T10:00:00", "end": "2026-11-20T11:00:00" }])).await;
        assert_eq!(e.stato, 200, "{}", e.testo);
        assert_eq!(e.json()["ok"].as_bool(), Some(true), "{}", e.testo);
        p.agente.qualunque_finestra.set(true);
        let ev = api(&p.base, &c, "get_events", &Json::Arr(vec![Json::Str("2026-11-16T00:00:00".into()), Json::Str("2026-11-23T00:00:00".into())])).await;
        assert_eq!(ev.stato, 200);
        assert!(ev.testo.contains("Prova"), "{}", ev.testo);
        for m in ["get_calendars", "get_personalizzazione", "get_loghi", "get_promemoria", "get_condivisioni", "get_stato_caselle"] {
            let r = api(&p.base, &c, m, &Json::Arr(vec![])).await;
            assert_eq!(r.stato, 200, "{m}: {}", r.testo);
        }
    });
}

#[test]
fn i_loghi() {
    con_gateway(|p| async move {
        let c = accedi(&p.base).await;
        let logo = |n: &str, m: &'static str| {
            let (b, c) = (p.base.clone(), c.clone());
            let n = n.to_string();
            async move { chiedi(m, &format!("{b}/logos/{n}"), &[("cookie", &c)], None).await }
        };
        let r = logo("personale.png", "GET").await;
        assert_eq!((r.stato, r.h("content-type"), r.h("cache-control"), r.h("x-content-type-options")), (200, Some("image/png"), Some("no-cache"), Some("nosniff")));
        assert!(r.h("content-security-policy").is_some());
        let h = logo("personale.png?v=3", "HEAD").await;
        assert_eq!((h.stato, h.h("content-length"), h.testo.as_str()), (200, Some(PNG.len().to_string().as_str()), ""));
        // la cartella dei dati vince su quella dell'immagine
        std::fs::create_dir_all(p.dati.join("logos")).unwrap();
        std::fs::write(p.dati.join("logos/personale.png"), b"dati").unwrap();
        std::fs::write(p.dati.join("logos/x.bin"), b"?").unwrap();
        assert_eq!(logo("personale.png", "GET").await.testo, "dati");
        assert_eq!(logo("x.bin", "GET").await.h("content-type"), Some("application/octet-stream"));
        for n in ["..%2Fgateway.db", "%2Fetc%2Fpasswd", "no.png", "logos"] {
            let r = logo(n, "GET").await;
            assert_eq!((r.stato, r.testo.as_str()), (404, r#"{"error":"non trovato"}"#), "{n}");
        }
    });
}

#[test]
fn il_cancello_di_caddy() {
    con_gateway(|p| async move {
        let u = format!("{}/api/verifica-sessione", p.base);
        let pagina = chiedi("GET", &u, &[("x-forwarded-uri", "/telefono/?a=1"), ("sec-fetch-dest", "document"), ("accept", "text/html")], None).await;
        assert_eq!(pagina.stato, 302);
        assert!(pagina.h("location").unwrap().contains("/login?next="), "{:?}", pagina.h("location"));
        let fuori = chiedi("GET", &u, &[("x-forwarded-uri", "//evil.test/"), ("sec-fetch-dest", "document")], None).await;
        assert_eq!(fuori.h("location"), Some("/login?next=%2F"));
        let ws = chiedi("GET", &u, &[("x-forwarded-uri", "/telefono/ws"), ("upgrade", "websocket"), ("accept", "text/html")], None).await;
        assert_eq!(ws.stato, 401);
        let script = chiedi("GET", &u, &[("x-forwarded-uri", "/a.js"), ("sec-fetch-dest", "script")], None).await;
        assert_eq!((script.stato, script.h("content-type")), (401, Some("application/json")));
        let c = accedi(&p.base).await;
        let ok = chiedi("GET", &u, &[("cookie", &c), ("x-forwarded-uri", "/telefono/")], None).await;
        assert_eq!((ok.stato, ok.testo.as_str(), ok.h("cache-control")), (204, "", Some("no-store")));
    });
}

#[test]
fn disponibilita_e_prenotazione_pubbliche() {
    con_gateway(|p| async move {
        let u = |q: &str| format!("{}/api/disponibilita?{q}", p.base);
        assert_eq!(get(&u("da=ieri&a=oggi")).await.stato, 400);
        assert_eq!(get(&u("da=2026-11-16&a=2026-11-10")).await.stato, 400);
        let lontano = get(&u("da=2020-01-06&a=2020-01-13")).await;
        assert_eq!((lontano.stato, lontano.testo.as_str()), (200, r#"{"events":[],"errors":[]}"#));
        let oggi = oe_gateway::disponibilita::oggi_a_roma(oe_gateway::orologio::ora_vera());
        let dopo = oe_gateway::disponibilita::sposta_data(&oggi, 7);
        p.agente.qualunque_finestra.set(true);
        let r = get(&u(&format!("da={oggi}&a={dopo}"))).await;
        assert_eq!(r.stato, 200, "{}", r.testo);
        assert_eq!(r.json()["errors"].in_js(), "[]");
        // nessun titolo esce: solo "Occupato" e le festività
        assert!(!r.testo.contains("\"account\":\"a"), "{}", r.testo);
        let pr = |tipo: &'static str, corpo: &'static [u8]| {
            let b = p.base.clone();
            async move { chiedi("POST", &format!("{b}/api/prenota"), &[("content-type", tipo)], Some(corpo.to_vec())).await }
        };
        assert_eq!(pr("text/plain", b"{}").await.stato, 415);
        assert_eq!(pr("application/json", b"{non json").await.stato, 400);
        let male = pr("application/json", br#"{"nome":"","email":"x","inizio":"2020-01-01T10:00:00","fine":"2020-01-01T10:30:00"}"#).await;
        assert!(male.stato >= 400 && male.json()["error"].as_str().is_some(), "{} {}", male.stato, male.testo);
        let troppo = chiedi("POST", &format!("{}/api/prenota", p.base), &[("content-type", "application/json")], Some(vec![b' '; 20_000])).await;
        assert_eq!(troppo.stato, 413);
        // il campo trappola ("sito") pieno è un robot
        assert_eq!(pr("application/json", br#"{"sito":"x","nome":"a","email":"a@b.it","inizio":"x","fine":"y"}"#).await.stato, 400);
        // una prenotazione vera: un giorno feriale fra tre e quattordici giorni, alle 10:00, fuori dalle festività nazionali
        let feste = ["01-01", "01-06", "04-25", "05-01", "06-02", "08-15", "11-01", "12-08", "12-25", "12-26"];
        let giorno = (3..15)
            .map(|n| oe_gateway::disponibilita::sposta_data(&oggi, n))
            .find(|d| {
                let g: jiff::civil::Date = d.parse().unwrap();
                !matches!(g.weekday(), jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday) && !feste.contains(&&d[5..])
            })
            .expect("un giorno feriale");
        let corpo = ogg! { "nome": "Carla Prova", "email": "carla@esempio.it", "azienda": "Esempio", "argomento": "Presentazione", "inizio": format!("{giorno}T10:00:00"), "fine": format!("{giorno}T10:30:00") }.in_js();
        let indirizzo = format!("{}/api/prenota", p.base);
        let invia = || chiedi("POST", &indirizzo, &[("content-type", "application/json")], Some(corpo.clone().into_bytes()));
        let ok = invia().await;
        assert_eq!(ok.stato, 200, "{}", ok.testo);
        assert_eq!(ok.json()["ok"].as_bool(), Some(true));
        // lo stesso orario non si prende due volte
        let doppia = invia().await;
        assert_eq!(doppia.stato, 409, "{}", doppia.testo);
        // e la disponibilità pubblica lo mostra occupato
        let vista = get(&u(&format!("da={giorno}&a={}", oe_gateway::disponibilita::sposta_data(&giorno, 1)))).await;
        assert!(vista.testo.contains(&format!("\"start\":\"{giorno}T10:00:00\"")), "{}", vista.testo);
        // un evento personale che blocca il tempo si vede subito, anche con il blocco già in memoria
        let domani = oe_gateway::disponibilita::sposta_data(&giorno, 1);
        let prima = get(&u(&format!("da={domani}&a={}", oe_gateway::disponibilita::sposta_data(&domani, 1)))).await;
        assert!(!prima.testo.contains(&format!("\"start\":\"{domani}T00:00:00\"")), "{}", prima.testo);
        let c = accedi(&p.base).await;
        let e = api(&p.base, &c, "add_local_event", &Json::Arr(vec![ogg! { "title": "Trasloco", "start": domani.clone(), "end": oe_gateway::disponibilita::sposta_data(&domani, 1), "allDay": true, "blocca": true }])).await;
        assert_eq!(e.json()["ok"].as_bool(), Some(true), "{}", e.testo);
        let dopo = get(&u(&format!("da={domani}&a={}", oe_gateway::disponibilita::sposta_data(&domani, 1)))).await;
        assert!(dopo.testo.contains(&format!("\"start\":\"{domani}T00:00:00\"")), "{}", dopo.testo);
    });
}

#[test]
fn i_link_di_condivisione() {
    con_gateway(|p| async move {
        let falso = get(&format!("{}/condividi/{}.ics", p.base, "a".repeat(43))).await;
        assert_eq!(falso.stato, 404);
        let c = accedi(&p.base).await;
        p.agente.qualunque_finestra.set(true);
        // una casella dell'azienda che non si legge: il link non pubblica un calendario incompleto
        let r0 = api(&p.base, &c, "add_condivisione", &Json::Arr(vec![ogg! { "nome": "Prima" }])).await;
        let url0 = r0.json()["condivisione"]["url"].as_str().expect("url").to_string();
        let guasto = get(&format!("{}{}", p.base, &url0[url0.find("/condividi/").unwrap()..])).await;
        assert_eq!((guasto.stato, guasto.h("retry-after")), (503, Some("120")));
        *p.agente.stati.borrow_mut() = ogg! { "assente@alfa.example": ogg! { "stato": "ok", "eventi": 0 } };
        let r = api(&p.base, &c, "add_condivisione", &Json::Arr(vec![ogg! { "nome": "Samuele", "nome_calendario": "Antonio, lavoro" }])).await;
        assert_eq!(r.stato, 200, "{}", r.testo);
        let url = r.json()["condivisione"]["url"].as_str().expect("url").to_string();
        let percorso = &url[url.find("/condividi/").expect("percorso")..];
        let ics = get(&format!("{}{percorso}", p.base)).await;
        assert_eq!(ics.stato, 200, "{}", ics.testo);
        assert_eq!(ics.h("content-disposition"), Some("inline; filename=\"calendario.ics\""));
        assert!(ics.h("content-type").unwrap().starts_with("text/calendar"));
        assert!(ics.testo.contains("X-WR-CALNAME:Antonio\\, lavoro"), "{}", ics.testo);
        if let Some(etag) = ics.h("etag") {
            let di_nuovo = chiedi("GET", &format!("{}{percorso}", p.base), &[("if-none-match", etag)], None).await;
            assert_eq!(di_nuovo.stato, 304);
        }
        let id = r.json()["condivisione"]["id"].clone();
        assert_eq!(api(&p.base, &c, "revoke_condivisione", &Json::Arr(vec![id])).await.stato, 200);
        assert_eq!(get(&format!("{}{percorso}", p.base)).await.stato, 404);
        assert_eq!(chiedi("POST", &format!("{}{percorso}", p.base), &[], None).await.stato, 405);
    });
}

// ---- MCP con un Outlook finto ---------------------------------------------------------------------------------------------------
struct Outlook {
    viste: RefCell<Vec<(String, String, String)>>,
}

fn messaggio(id: &str) -> Json {
    ogg! {
        "Id": id, "ConversationId": "AAQkC1", "Subject": "Riunione", "ReceivedDateTime": "2026-11-11T09:05:00Z",
        "From": ogg! { "EmailAddress": ogg! { "Name": "Mario", "Address": "mario@x.it" } },
        "ToRecipients": Json::Arr(vec![ogg! { "EmailAddress": ogg! { "Address": "io@oe.test" } }]),
        "BodyPreview": "ciao", "IsRead": false, "HasAttachments": false, "Importance": "Normal",
        "Body": ogg! { "ContentType": "HTML", "Content": "<p>Ciao <b>Antonio</b></p>" },
    }
}

impl Gestore for Outlook {
    async fn gestisci(&self, r: &mut Richiesta<'_>) -> Esito {
        let corpo = String::from_utf8_lossy(&r.leggi_corpo().await.unwrap_or_default()).into_owned();
        self.viste.borrow_mut().push((r.metodo.clone(), r.url.clone(), corpo));
        if r.intestazione("authorization") != Some("Bearer tok-1") {
            return json(401, &ogg! { "error": "token" });
        }
        let percorso = r.url.split('?').next().unwrap_or("").to_string();
        match (r.metodo.as_str(), percorso.as_str()) {
            ("GET", "/api/v2.0/me/MailFolders") => json(200, &ogg! { "value": Json::Arr(vec![ogg! { "Id": "f1", "DisplayName": "Inbox", "TotalItemCount": 3, "UnreadItemCount": 1, "ChildFolderCount": 0 }]) }),
            ("GET", "/api/v2.0/me/MailFolders/Inbox/messages") | ("GET", "/api/v2.0/me/messages") => json(200, &ogg! { "value": Json::Arr(vec![messaggio("AAMk1")]) }),
            ("GET", "/api/v2.0/me/messages/AAMk1") => json(200, &messaggio("AAMk1")),
            ("POST", "/api/v2.0/me/sendmail") => risposta(202, "Accepted", &[], Vec::new()),
            ("PATCH", "/api/v2.0/me/messages/AAMk1") => json(200, &ogg! {}),
            ("POST", "/api/v2.0/me/messages/AAMk1/move") => json(201, &ogg! { "Id": "AAMk2" }),
            _ => risposta(404, "Not Found", &[], b"sconosciuto".to_vec()),
        }
    }
}

async fn mcp(base: &str, intestazioni: &[(&str, &str)], corpo: &Json) -> Risp {
    let mut h = vec![("content-type", "application/json")];
    h.extend_from_slice(intestazioni);
    chiedi("POST", &format!("{base}/mcp"), &h, Some(corpo.in_js().into_bytes())).await
}

fn chiamata(id: i64, nome: &str, args: Json) -> Json {
    ogg! { "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": ogg! { "name": nome, "arguments": args } }
}

#[test]
fn il_server_mcp_della_posta() {
    con_gateway(|p| async move {
        let outlook = Rc::new(Outlook { viste: RefCell::new(Vec::new()) });
        let o = ascolta(outlook.clone()).await;
        let exp = oe_gateway::orologio::ora_vera() / 1000 + 3600;
        *p.agente.posta.borrow_mut() = Some(ogg! { "account": Json::Arr(vec![ogg! { "upn": "io@oe.test", "token": "tok-1", "exp": exp, "endpoint_type": "outlook", "base_url": format!("{o}/api/v2.0") }]) });
        let bearer = format!("Bearer {MCP}");
        let auth = [("authorization", bearer.as_str())];
        let init = ogg! { "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": ogg! {} };
        assert_eq!(mcp(&p.base, &[], &init).await.stato, 401);
        assert_eq!(mcp(&p.base, &[("authorization", "Bearer sbagliato")], &init).await.stato, 401);
        assert_eq!(mcp(&p.base, &[auth[0], ("origin", APP)], &init).await.stato, 403);
        assert_eq!(chiedi("GET", &format!("{}/mcp", p.base), &auth, None).await.stato, 405);
        assert_eq!(chiedi("DELETE", &format!("{}/mcp", p.base), &auth, None).await.stato, 204);
        let i = mcp(&p.base, &auth, &init).await;
        assert_eq!(i.json()["result"]["protocolVersion"].as_str(), Some("2025-06-18"));
        assert_eq!(mcp(&p.base, &auth, &ogg! { "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).await.json()["result"]["tools"].as_arr().unwrap().len(), 10);

        let testo = |r: &Risp| r.json()["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
        let a = mcp(&p.base, &auth, &chiamata(3, "list_accounts", ogg! {})).await;
        assert!(testo(&a).starts_with("Found 1 active Outlook account(s) on OE Copilot:\n\n- io@oe.test [oe-copilot, outlook, token expires in "), "{}", a.testo);
        let f = mcp(&p.base, &auth, &chiamata(4, "list_folders", ogg! {})).await;
        assert_eq!(testo(&f), "Folders for io@oe.test:\n\n- Inbox [3 items (1 unread)] (id: f1)");
        let l = mcp(&p.base, &auth, &chiamata(5, "list_emails", ogg! { "limit": 5 })).await;
        assert!(testo(&l).starts_with("Account: io@oe.test | Folder: Inbox | Showing 1 message(s) (newest first)\n\n[2026-11-11 10:05] [UNREAD] From: Mario <mario@x.it>"), "{}", l.testo);
        let e = mcp(&p.base, &auth, &chiamata(6, "read_email", ogg! { "message_id": "AAMk1" })).await;
        assert!(testo(&e).ends_with("--------------------------------------------------\n\nCiao **Antonio**"), "{}", e.testo);
        let s = mcp(&p.base, &auth, &chiamata(7, "search_emails", ogg! { "query": "riunione" })).await;
        assert!(testo(&s).starts_with("Search results for 'riunione' (1 hit(s), account: io@oe.test):"), "{}", s.testo);
        let m = mcp(&p.base, &auth, &chiamata(8, "send_email", ogg! { "to": "Carlo <c@z.it>; d@z.it", "subject": "Ciao", "body": "riga\nseconda" })).await;
        assert_eq!(testo(&m), "Email 'Ciao' successfully sent from io@oe.test. | To: [c@z.it, d@z.it]");
        let inviata = outlook.viste.borrow().iter().find(|v| v.1.ends_with("/sendmail")).cloned().expect("sendmail");
        let corpo = Json::analizza(&inviata.2).unwrap();
        assert_eq!(corpo["Message"]["ToRecipients"].in_js(), r#"[{"EmailAddress":{"Address":"c@z.it","Name":"Carlo"}},{"EmailAddress":{"Address":"d@z.it"}}]"#);
        assert_eq!(corpo["Message"]["Body"]["ContentType"].as_str(), Some("Text"));
        let g = mcp(&p.base, &auth, &chiamata(9, "manage_email", ogg! { "message_id": "AAMk1", "action": "mark_read" })).await;
        assert_eq!(testo(&g), "Executed 'mark_read' on 1 message(s) for io@oe.test:\n- AAMk1...: marked_read");
        let arch = mcp(&p.base, &auth, &chiamata(10, "manage_email", ogg! { "message_id": vec!["AAMk1"], "action": "archive" })).await;
        assert!(testo(&arch).ends_with("- AAMk1...: moved_to_Archive"), "{}", arch.testo);
        // gli errori di Outlook arrivano come risultato con isError
        let x = mcp(&p.base, &auth, &chiamata(11, "read_email", ogg! { "message_id": "manca" })).await;
        assert_eq!(x.json()["result"]["isError"].as_bool(), Some(true));
        assert!(testo(&x).starts_with("Error: Error: Errore API Outlook 404 su GET me/messages/manca: sconosciuto"), "{}", x.testo);
        let v = mcp(&p.base, &auth, &chiamata(12, "manage_email", ogg! { "message_id": "AAMk1", "action": "boh" })).await;
        assert!(testo(&v).starts_with("Unsupported action 'boh'"), "{}", v.testo);
        assert_eq!(mcp(&p.base, &auth, &chiamata(13, "send_email", ogg! { "subject": "x" })).await.json()["result"]["content"][0]["text"].as_str(), Some("'to' recipient is required for sending new email"));
    });
}

/// Identity headers never authenticate a request, including requests from the proxy address.
#[test]
fn everid_intestazione_solo_da_caddy() {
    con_gateway(|p| async move {
        let radice = format!("{}/", p.base);
        let finto = [("x-authentik-email", EMAIL)];
        let url_cfg = format!("{}/api/get_config", p.base);
        let api_cfg = || chiedi("POST", &url_cfg, &[("x-authentik-email", EMAIL), ("content-type", "application/json")], Some(b"[]".to_vec()));
        // spento (predefinito): l'intestazione non conta, anche da Caddy
        imposta("EVERID_PROXY", Some("127.0.0.1"));
        let r = chiedi("GET", &radice, &finto, None).await;
        assert_eq!((r.stato, r.h("location")), (307, Some("/login?next=%2F")));
        assert_eq!(api_cfg().await.stato, 401);
        // acceso, ma la richiesta non viene da Caddy: intestazione tolta
        imposta("EVERID", Some("on"));
        imposta("EVERID_PROXY", Some("10.0.0.5"));
        let r = chiedi("GET", &radice, &finto, None).await;
        assert_eq!((r.stato, r.h("location")), (307, Some("/login?next=%2F")));
        assert_eq!(api_cfg().await.stato, 401);
        // Even a request from the proxy cannot supply an authenticated identity.
        imposta("EVERID_PROXY", Some("127.0.0.1"));
        let r = chiedi("GET", &radice, &finto, None).await;
        assert_eq!(r.stato, 307);
        assert_eq!(api_cfg().await.stato, 401);
        let l = chiedi("GET", &format!("{}/login?next=/sicurezza", p.base), &finto, None).await;
        assert_eq!(l.stato, 200);
        // un'email che non è un utente del gateway non entra
        let r = chiedi("GET", &radice, &[("x-authentik-email", "⟦RG:3bc89a03⟧")], None).await;
        assert_eq!(r.stato, 307);
    });
}

/// Le strade pubbliche e quelle con un accesso loro rispondono senza EverID e senza sessione anche con EVERID=on.
#[test]
fn everid_strade_libere() {
    con_gateway(|p| async move {
        imposta("EVERID", Some("on"));
        imposta("EVERID_PROXY", Some("10.0.0.5"));
        assert_eq!(get(&format!("{}/salute", p.base)).await.stato, 200);
        assert_eq!(get(&format!("{}/disponibilita", p.base)).await.stato, 200);
        assert_ne!(get(&format!("{}/api/disponibilita?da=2026-11-23&a=2026-11-30", p.base)).await.stato, 401);
        let lista = mcp(&p.base, &[("authorization", &format!("Bearer {MCP}"))], &ogg! { "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).await;
        assert_eq!(lista.stato, 200);
        assert_eq!(lista.json()["result"]["tools"].as_arr().unwrap().len(), 10);
        // un link che non esiste è 404 come prima, non un rimando all'accesso
        assert_eq!(get(&format!("{}/condividi/{}.ics", p.base, "a".repeat(43))).await.stato, 404);
    });
}

/// ACCESSO_PASSWORD=off: l'accesso con la password non c'è più, EverID sì.
#[test]
fn everid_senza_password() {
    con_gateway(|p| async move {
        imposta("ACCESSO_PASSWORD", Some("off"));
        let r = chiedi(
            "POST",
            &format!("{}/api/auth/sign-in/email", p.base),
            &[("origin", APP), ("content-type", "application/json")],
            Some(ogg! { "email": EMAIL, "password": PASSWORD }.in_js().into_bytes()),
        )
        .await;
        assert_eq!(r.stato, 404);
        assert!(r.h("set-cookie").is_none());
        imposta("EVERID", Some("on"));
        imposta("EVERID_PROXY", Some("127.0.0.1"));
        assert_eq!(chiedi("GET", &format!("{}/", p.base), &[("x-authentik-email", EMAIL)], None).await.stato, 307);
    });
}

struct Identity;

impl Gestore for Identity {
    async fn gestisci(&self, r: &mut Richiesta<'_>) -> Esito {
        let path = r.url.split('?').next().unwrap_or("").to_string();
        let cookie = r.intestazione("cookie").unwrap_or("").to_string();
        let csrf = r.intestazione("x-authentik-csrf").unwrap_or("").to_string();
        let method = r.metodo.clone();
        let body = r.leggi_corpo().await.unwrap_or_default();
        let input = Json::analizza(&String::from_utf8_lossy(&body)).unwrap_or(Json::Null);
        if path == "/api/v3/core/brands/current/" {
            return risposta(200, "OK", &[("content-type", "application/json"), ("set-cookie", "authentik_csrf=csrf-test; Path=/")], br#"{"flow_authentication":"test-flow"}"#.to_vec());
        }
        if path == "/api/v3/flows/executor/test-flow/" {
            if method == "GET" && cookie.contains("auth=ok") {
                return json(200, &ogg! { "component": "xak-flow-redirect", "to": "https://elsewhere.invalid/" });
            }
            if method == "GET" { return json(200, &ogg! { "component": "ak-stage-identification", "password_fields": false }); }
            if csrf != "csrf-test" { return risposta(403, "Forbidden", &[], vec![]); }
            match input["component"].as_str().unwrap_or("") {
                "ak-stage-identification" => {
                    let name = input["uid_field"].as_str().unwrap_or("");
                    if name == "redirect" { return risposta(302, "Found", &[("location", "http://example.invalid/steal")], vec![]); }
                    if name == "mfa" { return json(200, &ogg! { "component": "ak-stage-authenticator-validate" }); }
                    let cookie = if name == "denied" { "group=no; Path=/" } else if name == "cookiepath" { "group=yes; Path=/unrelated" } else { "group=yes; Path=/" };
                    return risposta(200, "OK", &[("content-type", "application/json"), ("set-cookie", cookie)], br#"{"component":"ak-stage-password"}"#.to_vec());
                }
                "ak-stage-password" if input["password"].as_str() == Some("valid-test") => {
                    return risposta(302, "Found", &[("location", "/api/v3/flows/executor/test-flow/?query="), ("set-cookie", "auth=ok; Path=/")], vec![]);
                }
                _ => return json(200, &ogg! { "component": "ak-stage-password", "response_errors": ogg! { "password": "invalid" } }),
            }
        }
        if path == "/api/v3/core/users/me/" && cookie.contains("auth=ok") {
            let group = if cookie.contains("group=yes") { "app-calendar" } else { "other" };
            return json(200, &ogg! { "user": ogg! { "is_active": true, "email": EMAIL, "groups": Json::Arr(vec![ogg! { "name": group }]) } });
        }
        risposta(404, "Not Found", &[], vec![])
    }
}

#[test]
fn accesso_esterno_su_http() {
    con_gateway(|p| async move {
        let old = accedi(&p.base).await;
        let identity = ascolta(Rc::new(Identity)).await;
        imposta("EVERID_URL", Some(&identity));
        imposta("EVERID", Some("on"));
        imposta("ACCESSO_PASSWORD", Some("off"));
        let root = format!("{}/", p.base);
        assert_eq!(chiedi("GET", &root, &[("cookie", &old)], None).await.stato, 307);
        let login = |name: &str, pass: &str| ogg! { "email": name, "password": pass, "rememberMe": true }.in_js().into_bytes();
        let endpoint = format!("{}/api/auth/sign-in/email", p.base);
        let headers = [("origin", APP), ("content-type", "application/json")];
        assert_eq!(chiedi("POST", &endpoint, &headers, Some(login("long", &"a".repeat(129)))).await.stato, 401);
        for (name, pass, expected) in [("invalid", "wrong", 401), ("mfa", "valid-test", 401), ("denied", "valid-test", 403), ("cookiepath", "valid-test", 403), ("redirect", "valid-test", 503)] {
            let result = chiedi("POST", &endpoint, &headers, Some(login(name, pass))).await;
            assert_eq!(result.stato, expected, "scenario {name}");
            assert!(result.h("set-cookie").is_none());
        }
        let good = chiedi("POST", &endpoint, &headers, Some(login("valid", "valid-test"))).await;
        assert_eq!(good.stato, 200, "{}", good.testo);
        let cookie = good.h("set-cookie").unwrap().split(';').next().unwrap().to_string();
        assert_eq!(chiedi("GET", &root, &[("cookie", &cookie)], None).await.stato, 200);
        let logout = chiedi("POST", &format!("{}/api/auth/sign-out", p.base), &[("origin", APP), ("cookie", &cookie), ("content-type", "application/json")], Some(b"{}".to_vec())).await;
        assert_eq!(logout.stato, 200);
        assert_eq!(chiedi("GET", &root, &[("cookie", &cookie)], None).await.stato, 307);
        assert_eq!(get(&format!("{}/disponibilita", p.base)).await.stato, 200);
        assert_eq!(mcp(&p.base, &[("authorization", &format!("Bearer {MCP}"))], &ogg! { "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).await.stato, 200);
    });
}
