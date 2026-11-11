//! Quello che le prove del gateway hanno in comune (era web-ts/test/golden-comune.ts): i file di strumenti/golden (registrati dal servizio
//! ui in Python, mai rigenerati: una differenza è un comportamento cambiato), l'agente finto che ripete le risposte registrate, il server
//! dei calendari ICS di prova e la cartella dei dati costruita da golden/ui/fixture.json. Server e prova girano nello stesso thread.
#![allow(dead_code)]

use oe_gateway::rete_sicura::{indirizzo_ammesso, SoloProve};
use oe_http::{servi, Esito, Gestore, Richiesta, Risposta};
use oe_nucleo::json::{Json, Mappa};
use regex::Regex;
use std::cell::RefCell;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use tokio::net::TcpListener;

pub const CHIAVE: &str = "chiave-di-prova";
pub const ORA_FISSA_MS: i64 = 1_700_000_000_000;

pub fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../strumenti/golden")
}

pub fn leggi(pezzi: &[&str]) -> Json {
    let mut p = golden();
    for x in pezzi {
        p.push(x);
    }
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    Json::analizza(&t).expect("golden JSON")
}

pub static FIXTURE: LazyLock<Json> = LazyLock::new(|| leggi(&["ui", "fixture.json"]));
pub static AGENTE: LazyLock<Json> = LazyLock::new(|| leggi(&["agente.json"]));

/// Le chiavi in ordine (UTF-16, come in JavaScript): due JSON uguali a meno dell'ordine danno lo stesso testo, come toEqual.
pub fn ordinato(v: &Json) -> Json {
    match v {
        Json::Arr(a) => Json::Arr(a.iter().map(ordinato).collect()),
        Json::Obj(m) => {
            let mut chiavi: Vec<&String> = m.keys().collect();
            chiavi.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let mut out = Mappa::new();
            for k in chiavi {
                out.insert(k.as_str(), ordinato(m.get(k).unwrap_or(&Json::Null)));
            }
            Json::Obj(out)
        }
        altro => altro.clone(),
    }
}

pub fn canonico(v: &Json) -> String {
    ordinato(v).in_js()
}

/// Le ore del giorno cambiano a ogni lettura, la porta del server ICS a ogni giro e con lei l'id (sha1 del link) dei calendari aggiunti.
pub fn normalizza(v: &Json) -> Json {
    static ORA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(Letto )?alle \d\d:\d\d").unwrap());
    static PORTA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"http://127\.0\.0\.1:\d+").unwrap());
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ics_[0-9a-f]{12}").unwrap());
    let t = v.in_js();
    let t = ORA.replace_all(&t, "${1}alle HH:MM");
    let t = PORTA.replace_all(&t, "http://127.0.0.1:PORTA");
    let t = ID.replace_all(&t, "ics_HASH");
    Json::analizza(&t).expect("normalizzato")
}

/// Le regole di prova: 127.0.0.1 e una porta qualunque; il resto (reti private, metadati) resta vietato.
pub fn prove_locali() -> SoloProve {
    SoloProve {
        ammetti_indirizzo: Some(Arc::new(|ip: &IpAddr| ip.is_loopback() || indirizzo_ammesso(ip))),
        ammetti_porta: Some(Arc::new(|_| true)),
        ..Default::default()
    }
}

pub fn risposta(codice: u16, motivo: &str, intestazioni: &[(&str, &str)], corpo: Vec<u8>) -> Esito {
    let mut h: Vec<(String, String)> = intestazioni.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    h.push(("Content-Length".into(), corpo.len().to_string()));
    h.push(("Connection".into(), "close".into()));
    Esito::Risposta(Risposta { codice, motivo: motivo.to_string(), intestazioni: h, corpo, scarta_corpo: false })
}

pub fn json(codice: u16, corpo: &Json) -> Esito {
    risposta(codice, "OK", &[("Content-Type", "application/json; charset=utf-8")], corpo.in_js().into_bytes())
}

/// Un server su 127.0.0.1, porta a caso; gira finché gira il LocalSet della prova.
pub async fn ascolta<G: Gestore + 'static>(g: Rc<G>) -> String {
    let a = TcpListener::bind("127.0.0.1:0").await.expect("porta");
    let porta = a.local_addr().expect("indirizzo").port();
    tokio::task::spawn_local(servi(a, g));
    format!("http://127.0.0.1:{porta}")
}

/// Il server dei calendari ICS di prova: i file di golden/ics; un nome che non c'è dà 404 con la frase di http.server di Python.
pub struct Ics;

impl Gestore for Ics {
    async fn gestisci(&self, r: &mut Richiesta<'_>) -> Esito {
        let nome = r.url.rsplit('/').next().unwrap_or("").split('?').next().unwrap_or("").to_string();
        match std::fs::read(golden().join("ics").join(&nome)) {
            Ok(d) if !nome.is_empty() => risposta(200, "OK", &[("Content-Type", "text/calendar; charset=utf-8")], d),
            _ => risposta(404, "File not found", &[], Vec::new()),
        }
    }
}

fn unico(t: &str) -> String {
    t.replace("\r\n", "\n")
}

/// L'agente finto: le risposte registrate dall'agente vero, con lo stato che lo scenario chiede (`modo`). Ogni richiesta resta in `viste`.
pub struct Agente {
    pub modo: RefCell<String>,
    pub viste: RefCell<Vec<String>>,
    /// La risposta di /api/token-posta (le caselle per il server MCP); None: 404.
    pub posta: RefCell<Option<Json>>,
    /// Una finestra di /api/eventi non registrata riceve la prima registrata invece di 404 (per le prove che non leggono a ora fissa).
    pub qualunque_finestra: std::cell::Cell<bool>,
    /// Stati delle caselle messi dalla prova, dopo quelli del modo: { account: { campo: valore } }.
    pub stati: RefCell<Json>,
    nomi_ics: Vec<(String, String)>,
}

impl Agente {
    pub fn nuovo() -> Rc<Agente> {
        let nomi_ics = ["festivita", "riunioni", "html", "rotto"]
            .iter()
            .map(|n| (unico(&std::fs::read_to_string(golden().join("ics").join(format!("{n}.ics"))).expect("ics")), n.to_string()))
            .collect();
        Rc::new(Agente { modo: RefCell::new("normale".into()), viste: RefCell::new(Vec::new()), posta: RefCell::new(None), qualunque_finestra: std::cell::Cell::new(false), stati: RefCell::new(Json::obj()), nomi_ics })
    }
}

impl Gestore for Agente {
    async fn gestisci(&self, r: &mut Richiesta<'_>) -> Esito {
        let corpo = r.leggi_corpo().await.unwrap_or_default();
        self.viste.borrow_mut().push(format!("{} {}", r.metodo, r.url));
        let modo = FIXTURE["modi"][self.modo.borrow().as_str()].clone();
        if let Some(h) = modo.get("http").and_then(Json::as_f64) {
            return json(h as u16, &oe_nucleo::ogg! { "errore": "finto" });
        }
        if r.intestazione("authorization") != Some(&format!("Bearer {CHIAVE}")) {
            return json(401, &oe_nucleo::ogg! { "errore": "non autorizzato" });
        }
        let (percorso, query) = r.url.split_once('?').unwrap_or((&r.url, ""));
        if percorso == "/api/token-posta" {
            return match &*self.posta.borrow() {
                Some(p) => json(200, p),
                None => json(404, &oe_nucleo::ogg! { "errore": "sconosciuto" }),
            };
        }
        if r.metodo == "POST" && percorso == "/api/ics" {
            let q = Json::analizza(&String::from_utf8_lossy(&corpo)).expect("corpo JSON");
            let testo = unico(q["testo"].as_str().unwrap_or(""));
            let Some(voce) = self.nomi_ics.iter().find(|(t, _)| *t == testo).and_then(|(_, n)| AGENTE["ics"].get(n)) else {
                return json(200, &oe_nucleo::ogg! { "ok": false, "errore": "calendario non registrato" });
            };
            // senza intervallo il testo si valida soltanto
            return match q.get("inizio") {
                None => json(200, &voce["valida"]),
                Some(i) => {
                    let chiave = format!("{}|{}", i.as_str().unwrap_or(""), q["fine"].as_str().unwrap_or(""));
                    json(200, voce["eventi"].get(&chiave).unwrap_or(&oe_nucleo::ogg! { "ok": false, "errore": "finestra non registrata" }))
                }
            };
        }
        let mut dato = match percorso {
            "/api/calendari" => AGENTE["calendari"].clone(),
            "/api/eventi" => {
                let p: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
                let v = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, x)| x.clone()).unwrap_or_default();
                let registrata = AGENTE["eventi"].get(&format!("{}|{}", v("inizio"), v("fine")));
                let prima = || AGENTE["eventi"].as_obj().and_then(|m| m.values().next());
                match registrata.or_else(|| if self.qualunque_finestra.get() { prima() } else { None }) {
                    Some(d) => d.clone(),
                    None => return json(404, &oe_nucleo::ogg! { "errore": "finestra non registrata" }),
                }
            }
            _ => return json(404, &oe_nucleo::ogg! { "errore": "sconosciuto" }),
        };
        if let (Some(s), Some(o)) = (modo.get("servizio"), dato.as_obj_mut()) {
            o.insert("servizio", s.clone());
        }
        if let Some(Json::Arr(caselle)) = dato.as_obj_mut().and_then(|o| o.get_mut("caselle")) {
            for c in caselle.iter_mut() {
                let account = c["account"].as_str().unwrap_or("").to_string();
                let propri = self.stati.borrow().get(&account).cloned();
                for extra in [modo.get("stati").and_then(|s| s.get(&account)).cloned(), propri].into_iter().flatten() {
                    if let (Json::Obj(extra), Some(o)) = (extra, c.as_obj_mut()) {
                        for (k, v) in extra.iter() {
                            o.insert(k.as_str(), v.clone());
                        }
                    }
                }
            }
        }
        json(200, &dato)
    }
}

/// Una cartella temporanea che si toglie alla fine della prova.
pub struct Cartella(pub PathBuf);

impl Cartella {
    pub fn nuova(prefisso: &str) -> Cartella {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("caso");
        let p = std::env::temp_dir().join(format!("{prefisso}-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
        std::fs::create_dir_all(&p).expect("cartella");
        Cartella(p)
    }

    pub fn leggi(&self, nome: &str) -> Json {
        Json::analizza(&std::fs::read_to_string(self.0.join(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))).expect("JSON")
    }

    pub fn elenco(&self, sotto: &str) -> Json {
        let mut v: Vec<String> = std::fs::read_dir(self.0.join(sotto)).expect("elenco").map(|e| e.expect("voce").file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        Json::Arr(v.into_iter().map(Json::Str).collect())
    }
}

impl Drop for Cartella {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// La cartella dei dati di partenza: gli stessi file con cui il servizio ui registrò le risposte.
pub fn prepara_dati(profilo: &str, ics_url: &str) -> Cartella {
    let c = Cartella::nuova("oe-golden");
    let scrivi = |nome: &str, v: &Json| std::fs::write(c.0.join(nome), v.in_js()).expect("scrittura");
    scrivi("config.json", &Json::analizza(&FIXTURE["config"].in_js().replace("{ICS}", ics_url)).expect("config"));
    scrivi("personalizzazione.json", &FIXTURE["profili"][profilo]["personalizzazione"]);
    scrivi("local_events.json", &FIXTURE["locali"]);
    scrivi("agente_caselle.json", &FIXTURE["caselle_note"]);
    scrivi("events_cache.json", &FIXTURE["cache_eventi"]);
    std::fs::create_dir_all(c.0.join("ics_cache")).expect("ics_cache");
    std::fs::copy(golden().join("ics").join("festivita.ics"), c.0.join("ics_cache").join("builtin__festivita-it.ics")).expect("festività");
    c
}

/// L'ambiente del gateway per questo thread: dati, database, agente.
pub fn ambiente(dati: &Path, agente_url: &str) {
    use oe_gateway::config::imposta_per_prova as imposta;
    imposta("OE_DATI", Some(&dati.to_string_lossy()));
    imposta("GATEWAY_DB", Some(&dati.join("gateway.db").to_string_lossy()));
    imposta("AGENTE_URL", Some(agente_url));
    imposta("AGENTE_CHIAVE", Some(CHIAVE));
}

/// Confronto con il messaggio che mostra dove: i due testi canonici, il primo punto in cui differiscono.
pub fn uguali(nome: &str, avuto: &Json, atteso: &Json) {
    let (a, b) = (canonico(avuto), canonico(atteso));
    if a != b {
        let i = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
        let da = a[..i].char_indices().rev().nth(120).map_or(0, |(p, _)| p);
        panic!("{nome}: diverso dal registrato\n  avuto:  ...{}\n  atteso: ...{}", a.get(da..(i + 200).min(a.len())).unwrap_or(&a[da..]), b.get(da..(i + 200).min(b.len())).unwrap_or(&b[da..]));
    }
}
