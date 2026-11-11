//! L'agente in HTTP contro le risposte registrate dell'agente Python: le risposte intere su un calendar.db inventato (agente.json) e le
//! richieste di bordo su 14 scenari del disco, più i modi di non partire (agente-bordi.json). Le prove di `golden-agente.test.ts` e
//! `agente-bordi.test.ts`, con le stesse differenze volute dell'agente TypeScript.

mod comune;

use comune::{leggi_golden, leggi_testo_golden, primi_utf16, scrivi_db};
use oe_agente::fonte::{Contesto, Registro};
use oe_agente::http::servi;
use oe_agente::server::{Agente, OpzioniAgente, carica_fonti, contesto_di};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::LocalSet;

fn muto() -> Registro {
    Rc::new(|_, _| {})
}

fn cartella_prova(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("{nome}-{}-{}", std::process::id(), oe_agente::fonte::adesso_vero()));
    std::fs::create_dir_all(&d).expect("cartella");
    d
}

/// Un agente in ascolto su una porta libera, dentro il LocalSet della prova.
async fn avvia(chiave: &str, fonti_json: &Path, db: &Path, battito: &Path, ctx: &Contesto) -> u16 {
    let fonti = carica_fonti(fonti_json, db, ctx).expect("fonti");
    let mut o = OpzioniAgente::nuove(chiave, fonti, battito.to_path_buf(), "Europe/Rome");
    o.registro = muto();
    let agente = Agente::nuovo(o);
    let ascolto = TcpListener::bind("127.0.0.1:0").await.expect("porta");
    let porta = ascolto.local_addr().expect("indirizzo").port();
    tokio::task::spawn_local(servi(ascolto, Rc::new(agente)));
    porta
}

#[derive(Debug, Default)]
struct RispostaGrezza {
    stato: Option<u16>,
    motivo: Option<String>,
    tipo: Option<String>,
    lunghezza: Option<String>,
    corpo: Option<String>,
    chiusa: bool,
    timeout: bool,
    azzerata: bool,
}

/// Un testo scritto come latin1 (`Buffer.from(testo, "latin1")`: di ogni unità UTF-16 il byte basso).
fn latin1(s: &str) -> Vec<u8> {
    s.encode_utf16().map(|u| u as u8).collect()
}

fn dal_latin1(b: &[u8]) -> String {
    b.iter().map(|x| *x as char).collect()
}

fn trova(dove: &[u8], cosa: &[u8]) -> Option<usize> {
    dove.windows(cosa.len()).position(|f| f == cosa)
}

fn corpo_di(r: &Json, ics: &[(&str, String)]) -> Option<Vec<u8>> {
    let c = r.get("corpo")?;
    let mut b = if let Some(h) = c.get("hex").and_then(Json::as_str) {
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or_default()).collect()
    } else {
        let mut t = c.get("testo").and_then(Json::as_str).unwrap_or_default().to_string();
        for (nome, testo) in ics {
            let j = Json::from(testo.as_str()).in_js();
            t = t.replace(&format!("@@{nome}@@"), &j[1..j.len() - 1]);
        }
        let mut b = t.into_bytes();
        if c.get("codifica").and_then(Json::as_str) == Some("utf8-bom") {
            b.splice(0..0, [0xef, 0xbb, 0xbf]);
        }
        b
    };
    if let Some(n) = r.get("riempiA").and_then(Json::as_f64) {
        b.resize(n as usize, b' ');
    }
    Some(b)
}

fn prova_risposta(dati: &[u8], metodo: &str, chiusa: bool) -> Option<RispostaGrezza> {
    let i = trova(dati, b"\r\n\r\n")?;
    let testa = dal_latin1(&dati[..i]);
    let mut linee = testa.split("\r\n");
    let prima = linee.next().unwrap_or_default();
    let mut pezzi = prima.split(' ');
    pezzi.next();
    let codice = pezzi.next().and_then(|c| c.parse().ok());
    let motivo = pezzi.collect::<Vec<_>>().join(" ");
    let mut intest: Vec<(String, String)> = Vec::new();
    for l in linee {
        let k = l.find(':').unwrap_or(l.len());
        let nome = l[..k].trim().to_lowercase();
        if !intest.iter().any(|(n, _)| *n == nome) {
            intest.push((nome, l.get(k + 1..).unwrap_or_default().trim().to_string()));
        }
    }
    let prendi = |k: &str| intest.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let resto = &dati[i + 4..];
    let lung: Option<usize> = prendi("content-length").map(|v| v.parse().unwrap_or(usize::MAX));
    let finita = metodo == "HEAD" || lung.is_some_and(|l| resto.len() >= l) || (lung.is_none() && chiusa);
    finita.then(|| RispostaGrezza {
        stato: codice,
        motivo: Some(motivo),
        tipo: prendi("content-type"),
        lunghezza: prendi("content-length"),
        corpo: Some(String::from_utf8_lossy(resto).into_owned()),
        ..Default::default()
    })
}

/// Una richiesta scritta riga per riga su un socket, come fa il registratore di Python, e la risposta letta come la legge lui (fino al
/// Content-Length).
async fn richiesta_completa(porta: u16, r: &Json, chiave: &str, ics: &[(&str, String)]) -> RispostaGrezza {
    let corpo = corpo_di(r, ics);
    let metodo = r["metodo"].as_str().unwrap_or_default();
    let mut righe = vec![format!("{metodo} {} HTTP/1.1", r["percorso"].as_str().unwrap_or_default()), "Host: 127.0.0.1".to_string(), "Connection: close".to_string()];
    let extra: Vec<String> = r.get("intestazioni").and_then(Json::as_arr).map(|v| v.iter().map(|x| x.as_str().unwrap_or_default().to_string()).collect()).unwrap_or_default();
    if r.get("senzaChiave").and_then(Json::as_bool) != Some(true) && !extra.iter().any(|x| x.to_lowercase().starts_with("authorization")) {
        righe.push(format!("Authorization: Bearer {}", r.get("chiave").and_then(Json::as_str).unwrap_or(chiave)));
    }
    righe.extend(extra);
    let dichiarata = r.get("lunghezzaDichiarata").and_then(Json::as_f64).map(|n| n as u64);
    let senza_lunghezza = r.get("senzaLunghezza").and_then(Json::as_bool) == Some(true);
    match (&corpo, dichiarata) {
        (Some(c), d) if !senza_lunghezza => righe.push(format!("Content-Length: {}", d.unwrap_or(c.len() as u64))),
        (_, Some(d)) => righe.push(format!("Content-Length: {d}")),
        _ => {}
    }
    let testa = latin1(&(righe.join("\r\n") + "\r\n\r\n"));
    let senza_corpo = r.get("senzaCorpo").and_then(Json::as_bool) == Some(true);
    let attesa = Duration::from_millis(if senza_corpo { 4000 } else { 40000 });
    let lettura = async {
        let mut s = TcpStream::connect(("127.0.0.1", porta)).await.expect("connessione");
        let _ = s.write_all(&testa).await;
        if let (Some(c), false) = (&corpo, senza_corpo) {
            let _ = s.write_all(c).await;
        }
        let mut dati = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match s.read(&mut buf).await {
                Ok(0) => {
                    return prova_risposta(&dati, metodo, true).unwrap_or(RispostaGrezza { chiusa: trova(&dati, b"\r\n\r\n").is_none(), ..Default::default() });
                }
                Ok(n) => {
                    dati.extend_from_slice(&buf[..n]);
                    if let Some(x) = prova_risposta(&dati, metodo, false) {
                        return x;
                    }
                }
                Err(_) => {
                    return prova_risposta(&dati, metodo, true).unwrap_or(RispostaGrezza { azzerata: trova(&dati, b"\r\n\r\n").is_none(), ..Default::default() });
                }
            }
        }
    };
    tokio::time::timeout(attesa, lettura).await.unwrap_or(RispostaGrezza { timeout: true, ..Default::default() })
}

fn normalizza(t: &str, db: &str) -> String {
    let s = t.replace(db, "<DB>");
    let generato = Regex::new(r#""generato": "[^"]*""#).expect("valida");
    let ora = Regex::new(r"alle \d\d:\d\d").expect("valida");
    let s = generato.replace(&s, r#""generato": "ORA""#).into_owned();
    ora.replace_all(&s, "alle HH:MM").into_owned()
}

struct Pronto {
    porta: u16,
    db: String,
}

fn scrivi(p: &Path, t: &str) {
    std::fs::write(p, t).expect("scrittura");
}

/// Il disco di uno scenario: calendar.db, fonti.json, battito.json, accessi.json.
fn prepara(base: &Path, nome: &str, sc: &Json, golden: &Json) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let d = base.join(nome);
    let db = d.join("db");
    std::fs::create_dir_all(&db).expect("cartella");
    let file = db.join("calendar.db");
    let t = sc["db"].as_str().unwrap_or_default();
    match t {
        "golden" | "senza-eventi" | "senza-colonna" => {
            scrivi_db(&db, &golden["calendari"], &golden["eventi"], &golden["invitati"]);
            let sql = rusqlite::Connection::open(&file).expect("calendar.db");
            if t == "senza-eventi" {
                sql.execute_batch("DROP TABLE Events").expect("drop");
            }
            if t == "senza-colonna" {
                sql.execute_batch("ALTER TABLE Calendars DROP COLUMN sync_events").expect("alter");
            }
        }
        "vuoto" => scrivi(&file, ""),
        "corrotto" => scrivi(&file, &"questo non e' un database sqlite, e' solo testo ".repeat(100)),
        "cartella" => std::fs::create_dir(&file).expect("cartella"),
        _ => {}
    }
    if let Some(p) = sc.get("permessi").and_then(Json::as_str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(u32::from_str_radix(p, 8).unwrap_or(0))).expect("chmod");
    }
    let fonti = d.join("fonti.json");
    let elenco: Vec<Json> = golden["fonti"]
        .as_arr()
        .expect("fonti")
        .iter()
        .map(|x| {
            let mut m = Mappa::new();
            m.insert("tipo", "android");
            for (k, v) in x.as_obj().expect("fonte").iter() {
                m.insert(k.as_str(), v.clone());
            }
            Json::Obj(m)
        })
        .collect();
    match sc.get("fonti").and_then(Json::as_str).unwrap_or("golden") {
        "golden" => scrivi(&fonti, &Json::Arr(elenco).in_js()),
        "con-sconosciuta" => {
            let mut v = elenco;
            v.push(ogg! { "tipo": "caldav", "id": "x", "nome": "X", "account": "x@x" });
            scrivi(&fonti, &Json::Arr(v).in_js());
        }
        "json-rotto" => scrivi(&fonti, "{non json"),
        "stringa" => scrivi(&fonti, "\"abc\""),
        "numero" => scrivi(&fonti, "5"),
        "senza-id" => scrivi(&fonti, r#"[{"tipo":"android","nome":"A","account":"a@a"}]"#),
        _ => {}
    }
    let battito = d.join("battito.json");
    if let Some(b) = sc.get("battito") {
        scrivi(&battito, &b.in_js());
    } else if let Some(b) = sc.get("battitoTesto").and_then(Json::as_str) {
        scrivi(&battito, b);
    }
    let accessi = d.join("accessi.json");
    if let Some(a) = sc.get("accessi") {
        scrivi(&accessi, &a.in_js());
    } else if let Some(a) = sc.get("accessiTesto").and_then(Json::as_str) {
        scrivi(&accessi, a);
    }
    (db, fonti, battito, accessi)
}

/// Le differenze volute, ognuna con il perché (le stesse di agente-bordi.test.ts). `None`: si confronta con Python.
fn voluta(nome: &str, r: &RispostaGrezza) -> Option<Result<(), String>> {
    let atteso = |ok: bool, cosa: &str| if ok { Ok(()) } else { Err(format!("{cosa}: {r:?}")) };
    match nome {
        // Python legge un testo senza a capo come il percorso di un file; l'agente non apre mai file per un testo
        "ics-testo-percorso-di-file" => {
            let corpo = r.corpo.as_deref().and_then(|c| Json::analizza(c).ok()).map(|j| j.in_js());
            let voluto = ogg! { "ok": false, "errore": "Content line could not be parsed into parts: '/etc/os-release': /etc/os-release", "valore": true }.in_js();
            Some(atteso(r.stato == Some(200) && corpo.as_deref() == Some(voluto.as_str()), "testo come percorso"))
        }
        // Node toglie gli spazi in coda al valore di un'intestazione prima che il gestore la veda, Python no
        "salute-chiave-spazio-finale" => Some(atteso(r.stato == Some(200), "spazio finale")),
        // Node rifiuta con 400 i byte non ASCII grezzi nel percorso, Python li accetta: il gateway codifica sempre il percorso
        "salute-percorso-non-ascii" | "salute-percorso-non-ascii-un-byte" | "eventi-percorso-non-ascii" | "percorso-non-ascii-senza-chiave" | "ics-percorso-non-ascii" => {
            Some(atteso(r.stato == Some(400), "percorso non ASCII"))
        }
        _ => None,
    }
}

fn confronta_bordo(r: &Json, o: &RispostaGrezza, db: &str) -> Result<(), String> {
    let nome = r["nome"].as_str().unwrap_or_default();
    if let Some(v) = voluta(nome, o) {
        return v;
    }
    let a = &r["atteso"];
    let flag = |k: &str| a.get(k).and_then(Json::as_bool).unwrap_or(false);
    if flag("chiusa") || flag("timeout") || flag("azzerata") {
        return if (o.chiusa, o.timeout, o.azzerata) == (flag("chiusa"), flag("timeout"), flag("azzerata")) { Ok(()) } else { Err(format!("atteso chiusa/timeout/azzerata {a:?}, ottenuto {o:?}")) };
    }
    let grezzo = o.corpo.clone().unwrap_or_default();
    let corpo = normalizza(&grezzo, db);
    // un messaggio col percorso del calendario è lungo quanto il percorso della cartella di prova: la lunghezza vale allora come lunghezza del corpo
    let lunghezza = if corpo != grezzo { Some(grezzo.len().to_string()) } else { a.get("lunghezza").and_then(Json::as_str).map(str::to_string) };
    let testa_attesa = (a.get("stato").and_then(Json::as_f64).map(|n| n as u16), a.get("motivo").and_then(Json::as_str).map(str::to_string), a.get("tipo").and_then(Json::as_str).map(str::to_string), lunghezza);
    let testa = (o.stato, o.motivo.clone(), o.tipo.clone(), o.lunghezza.clone());
    if testa != testa_attesa {
        return Err(format!("testa: attesa {testa_attesa:?}, ottenuta {testa:?}; corpo {}", primi_utf16(&corpo, 300)));
    }
    if let Some(sha) = a.get("corpoSha256").and_then(Json::as_str) {
        let nostro = Sha256::digest(corpo.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        let caratteri = corpo.encode_utf16().count() as f64;
        if nostro != sha || Some(caratteri) != a.get("corpoCaratteri").and_then(Json::as_f64) {
            return Err(format!("corpo lungo diverso ({} caratteri): {}", caratteri, primi_utf16(&corpo, 600)));
        }
        return Ok(());
    }
    let atteso_corpo = a.get("corpo").and_then(Json::as_str).unwrap_or_default();
    if corpo != atteso_corpo {
        return Err(format!("corpo: atteso {} ottenuto {}", primi_utf16(atteso_corpo, 400), primi_utf16(&corpo, 400)));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn richieste_di_bordo_come_python() {
    LocalSet::new()
        .run_until(async {
            let bordi = leggi_golden("agente-bordi.json");
            let golden = leggi_golden("agente-righe.json");
            let chiave = bordi["chiave"].as_str().expect("chiave");
            let ics = [("festivita", leggi_testo_golden(&["ics", "festivita.ics"])), ("riunioni", leggi_testo_golden(&["ics", "riunioni.ics"]))];
            let base = cartella_prova("agente-bordi");
            let mut pronti: Vec<(String, Pronto)> = Vec::new();
            for (nome, sc) in bordi["scenari"].as_obj().expect("scenari").iter() {
                let (db, fonti, battito, accessi) = prepara(&base, nome, sc, &golden);
                let ctx = contesto_di(accessi, "Europe/Rome", None, Some(muto()));
                let porta = avvia(chiave, &fonti, &db, &battito, &ctx).await;
                pronti.push((nome.clone(), Pronto { porta, db: db.to_string_lossy().into_owned() }));
            }
            let mut diversi = Vec::new();
            let richieste = bordi["richieste"].as_arr().expect("richieste");
            for r in richieste {
                let scenario = r["scenario"].as_str().unwrap_or_default();
                let p = &pronti.iter().find(|(n, _)| n == scenario).expect("scenario").1;
                let o = richiesta_completa(p.porta, r, chiave, &ics).await;
                if let Err(e) = confronta_bordo(r, &o, &p.db) {
                    diversi.push(format!("{}: {e}", r["nome"].as_str().unwrap_or_default()));
                }
            }
            let _ = std::fs::remove_dir_all(&base);
            println!("bordi: {} richieste su {} scenari", richieste.len(), pronti.len());
            assert!(richieste.len() > 100);
            assert!(diversi.is_empty(), "{} di {} diverse:\n{}", diversi.len(), richieste.len(), diversi.join("\n"));
        })
        .await;
}

#[test]
fn avvii_che_falliscono() {
    let bordi = leggi_golden("agente-bordi.json");
    let golden = leggi_golden("agente-righe.json");
    let base = cartella_prova("agente-avvii");
    let mut diversi = Vec::new();
    for (nome, av) in bordi["avvii"].as_obj().expect("avvii").iter() {
        let sc = ogg! { "db": "golden", "fonti": av.get("fonti").cloned().unwrap_or_default() };
        let (db, fonti, battito, accessi) = prepara(&base, &format!("avvio-{nome}"), &sc, &golden);
        let porta = std::net::TcpListener::bind("127.0.0.1:0").expect("porta").local_addr().expect("indirizzo").port().to_string();
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_agente"));
        c.env("FONTI", &fonti).env("ANDROID_DB", &db).env("BATTITO", &battito).env("ACCESSI", &accessi).env("TZ", "Europe/Rome");
        c.env("PORTA", av.get("porta").and_then(Json::as_str).unwrap_or(&porta));
        if av.get("senzaChiave").and_then(Json::as_bool) == Some(true) {
            c.env_remove("API_KEY");
        } else {
            c.env("API_KEY", bordi["chiave"].as_str().unwrap_or_default());
        }
        let mut figlio = c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().expect("avvio");
        let mut uscita = None;
        for _ in 0..80 {
            if let Some(s) = figlio.try_wait().expect("attesa") {
                uscita = s.code();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if uscita.is_none() {
            let _ = figlio.kill();
        }
        let attesa = av["atteso"]["uscita"].as_f64().map(|n| n as i32);
        if uscita != attesa || av["atteso"]["ascolta"].as_bool() != Some(false) {
            diversi.push(format!("{nome}: uscita {uscita:?}, attesa {attesa:?}"));
        }
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(diversi.is_empty(), "{diversi:#?}");
}

// ---- le risposte intere su un calendar.db inventato --------------------------------------------------------------------------
const CHIAVE_PROVA: &str = "chiave-di-prova";

/// Una richiesta HTTP/1.1 semplice, la risposta intera.
async fn chiedi(porta: u16, metodo: &str, percorso: &str, chiave: Option<&str>, corpo: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", porta)).await.expect("connessione");
    let mut testa = format!("{metodo} {percorso} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Some(k) = chiave {
        testa.push_str(&format!("Authorization: Bearer {k}\r\n"));
    }
    if let Some(c) = corpo {
        testa.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", c.len()));
    }
    testa.push_str("\r\n");
    s.write_all(testa.as_bytes()).await.expect("scrittura");
    if let Some(c) = corpo {
        s.write_all(c.as_bytes()).await.expect("corpo");
    }
    let mut dati = Vec::new();
    s.read_to_end(&mut dati).await.expect("lettura");
    let i = trova(&dati, b"\r\n\r\n").expect("risposta");
    let stato = dal_latin1(&dati[..i]).split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or_default();
    (stato, String::from_utf8_lossy(&dati[i + 4..]).into_owned())
}

/// L'ora del giorno cambia a ogni lettura: si toglie, il resto deve restare identico.
fn normalizza_json(corpo: &str) -> Json {
    let ora = Regex::new(r"alle \d\d:\d\d").expect("valida");
    let mut v = Json::analizza(&ora.replace_all(&Json::analizza(corpo).expect("JSON").in_js(), "alle HH:MM")).expect("JSON");
    if let Some(m) = v.as_obj_mut() {
        if m.contains_key("generato") {
            m.insert("generato", "ORA");
        }
    }
    v
}

fn codifica(testo: &str) -> String {
    let mut out = String::new();
    for b in testo.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn uguali(a: &Json, b: &Json) -> bool {
    comune::canonico(a) == comune::canonico(b)
}

#[tokio::test(flavor = "current_thread")]
async fn risposte_intere_come_python() {
    LocalSet::new()
        .run_until(async {
            let golden = leggi_golden("agente.json");
            let righe = leggi_golden("agente-righe.json");
            let cartella = cartella_prova("agente-golden");
            scrivi_db(&cartella, &righe["calendari"], &righe["eventi"], &righe["invitati"]);
            let fonti_json = cartella.join("fonti.json");
            let fonti: Vec<Json> = righe["fonti"]
                .as_arr()
                .expect("fonti")
                .iter()
                .map(|f| {
                    let mut m = Mappa::new();
                    m.insert("tipo", "android");
                    for (k, v) in f.as_obj().expect("fonte").iter() {
                        m.insert(k.as_str(), v.clone());
                    }
                    Json::Obj(m)
                })
                .collect();
            scrivi(&fonti_json, &Json::Arr(fonti).in_js());
            let ctx = contesto_di(cartella.join("nessun-accesso.json"), "Europe/Rome", None, Some(muto()));
            let porta = avvia(CHIAVE_PROVA, &fonti_json, &cartella, &cartella.join("nessun-battito.json"), &ctx).await;
            let mut diversi = Vec::new();
            let mut confronti = 0;

            let (_, corpo) = chiedi(porta, "GET", "/api/calendari", Some(CHIAVE_PROVA), None).await;
            confronti += 1;
            if !uguali(&normalizza_json(&corpo), &golden["calendari"]) {
                diversi.push(format!("calendari: {}", primi_utf16(&corpo, 400)));
            }
            for (finestra, atteso) in golden["eventi"].as_obj().expect("eventi").iter() {
                let (inizio, fine) = finestra.split_once('|').expect("finestra");
                let (_, corpo) = chiedi(porta, "GET", &format!("/api/eventi?inizio={}&fine={}", codifica(inizio), codifica(fine)), Some(CHIAVE_PROVA), None).await;
                confronti += 1;
                if !uguali(&normalizza_json(&corpo), atteso) {
                    diversi.push(format!("eventi {finestra}: {}", primi_utf16(&corpo, 400)));
                }
            }
            let testi = [
                ("festivita", leggi_testo_golden(&["ics", "festivita.ics"])),
                ("riunioni", leggi_testo_golden(&["ics", "riunioni.ics"])),
                ("html", leggi_testo_golden(&["ics", "html.ics"])),
                ("rotto", leggi_testo_golden(&["ics", "rotto.ics"])),
                ("vuoto", String::new()),
            ];
            let testo = |nome: &str| testi.iter().find(|(n, _)| *n == nome).map(|(_, t)| t.clone()).unwrap_or_default();
            for (nome, registrato) in golden["ics"].as_obj().expect("ics").iter() {
                if nome == "rifiuti" {
                    continue;
                }
                let (stato, corpo) = chiedi(porta, "POST", "/api/ics", Some(CHIAVE_PROVA), Some(&ogg! { "testo": testo(nome) }.in_js())).await;
                confronti += 1;
                if stato != 200 || !uguali(&Json::analizza(&corpo).expect("JSON"), &registrato["valida"]) {
                    diversi.push(format!("ics {nome} validazione: {stato} {}", primi_utf16(&corpo, 400)));
                }
                for (finestra, atteso) in registrato["eventi"].as_obj().expect("eventi").iter() {
                    let (inizio, fine) = finestra.split_once('|').expect("finestra");
                    let (stato, corpo) = chiedi(porta, "POST", "/api/ics", Some(CHIAVE_PROVA), Some(&ogg! { "testo": testo(nome), "inizio": inizio, "fine": fine }.in_js())).await;
                    confronti += 1;
                    // nello stesso ordine di Python: si confronta la lista così com'è
                    if stato != 200 || !uguali(&Json::analizza(&corpo).expect("JSON"), atteso) {
                        diversi.push(format!("ics {nome} {finestra}: {stato} {}", primi_utf16(&corpo, 400)));
                    }
                }
            }
            let prima_finestra = golden["eventi"].as_obj().expect("eventi").keys().next().expect("finestra").split('|').next().unwrap_or_default().to_string();
            let festivita = testo("festivita");
            let mut rifiuti = Mappa::new();
            rifiuti.insert("senza chiave", chiedi(porta, "POST", "/api/ics", None, Some("{}")).await.0 as u32);
            rifiuti.insert("un solo estremo", chiedi(porta, "POST", "/api/ics", Some(CHIAVE_PROVA), Some(&ogg! { "testo": festivita.as_str(), "inizio": prima_finestra }.in_js())).await.0 as u32);
            rifiuti.insert("testo non stringa", chiedi(porta, "POST", "/api/ics", Some(CHIAVE_PROVA), Some(r#"{"testo":5}"#)).await.0 as u32);
            rifiuti.insert("estremo non valido", chiedi(porta, "POST", "/api/ics", Some(CHIAVE_PROVA), Some(&ogg! { "testo": festivita.as_str(), "inizio": "ieri", "fine": "oggi" }.in_js())).await.0 as u32);
            rifiuti.insert("percorso sconosciuto", chiedi(porta, "POST", "/api/altro", Some(CHIAVE_PROVA), Some("{}")).await.0 as u32);
            confronti += 1;
            if !uguali(&Json::Obj(rifiuti.clone()), &golden["ics"]["rifiuti"]) {
                diversi.push(format!("rifiuti: {}", Json::Obj(rifiuti).in_js()));
            }
            let _ = std::fs::remove_dir_all(&cartella);
            println!("golden: {confronti} risposte confrontate");
            assert!(confronti > 10);
            assert!(diversi.is_empty(), "{} diverse:\n{}", diversi.len(), diversi.join("\n"));
        })
        .await;
}
