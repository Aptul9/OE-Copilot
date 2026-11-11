//! Le ricorrenze contro le risposte registrate delle librerie Python (ricorrenze-casuali, ricorrenze-extra, ricorrenze-orarie e
//! ricorrenze-rare): le stesse prove di `agente-ts/test/ausiliari/confronto.ts`, con la stessa severità.

mod comune;

use comune::{canonico, db_in_memoria, leggi_golden, primi_utf16};
use oe_agente::fonte::{InfoCasella, estrai, eventi_da_lettura, riga_android};
use oe_nucleo::android::{OccorrenzaAndroid, espandi_righe_android};
use oe_nucleo::errori::Errore;
use oe_nucleo::ics::{EventoIcs, espandi_ics, leggi_iso_locale, valida_ics};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq)]
enum Stato {
    Uguale,
    Diverso,
    NonSupportato,
}

struct Esito {
    stato: Stato,
    dettaglio: String,
}

fn uguale() -> Esito {
    Esito { stato: Stato::Uguale, dettaglio: String::new() }
}

fn diverso(d: String) -> Esito {
    Esito { stato: Stato::Diverso, dettaglio: d }
}

const ID_PROVA: &str = "android::mario@alfa.example";
const ACCOUNT_PROVA: &str = "mario@alfa.example";

fn calendari_prova() -> Json {
    Json::analizza(r#"[[1, "mario@alfa.example", "com.microsoft.office.outlook", "Calendar", 1, 0, 1]]"#).expect("calendari")
}

fn sintesi(v: &Json) -> String {
    primi_utf16(&canonico(v), 260)
}

fn testo_di(v: &Json, k: &str) -> String {
    match v.get(k) {
        None => "undefined".to_string(),
        Some(Json::Str(s)) => s.clone(),
        Some(Json::Bool(b)) => b.to_string(),
        Some(Json::Null) => "null".to_string(),
        Some(altro) => altro.in_js(),
    }
}

fn ordina(v: &[Json], chiavi: &[&str]) -> Vec<Json> {
    let mut out = v.to_vec();
    out.sort_by(|a, b| {
        for k in chiavi {
            if a.get(k) != b.get(k) {
                let (x, y) = (testo_di(a, k), testo_di(b, k));
                return if x.encode_utf16().lt(y.encode_utf16()) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
        }
        std::cmp::Ordering::Equal
    });
    out
}

thread_local! {
    /// Quanti confronti e quante occorrenze attese: una prova che non confronta niente non deve passare.
    static CONTATI: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

fn conta(occorrenze: usize) {
    CONTATI.with(|c| {
        let (a, b) = c.get();
        c.set((a + 1, b + occorrenze));
    });
}

fn conteggi() -> (usize, usize) {
    CONTATI.with(|c| c.get())
}

/// Confronto di una risposta attesa (o un errore) con quello che si ottiene: `Uguale` se coincidono.
fn confronta_lista(atteso: &Json, ottenuto: &Result<Vec<Json>, Errore>, chiavi: &[&str]) -> Esito {
    conta(atteso.as_arr().map_or(0, Vec::len));
    let err_atteso = atteso.as_arr().is_none();
    match ottenuto {
        Err(e) => {
            if matches!(e, Errore::NonSupportato(_)) && !err_atteso {
                return Esito { stato: Stato::NonSupportato, dettaglio: e.messaggio().to_string() };
            }
            if !err_atteso {
                return diverso(format!("errore {e:?} dove Python risponde {}", sintesi(atteso)));
            }
            // il testo dell'errore è quello che il gateway mostra: deve essere lo stesso di Python (registrato fino a 200 caratteri)
            let nostro = primi_utf16(e.messaggio(), 200);
            match atteso.get("messaggio").and_then(Json::as_str) {
                Some(m) if m != nostro => diverso(format!("errore con un altro testo: Python {}, qui {}", Json::from(m).in_js(), Json::from(nostro).in_js())),
                _ => uguale(),
            }
        }
        Ok(v) => {
            if err_atteso {
                return diverso(format!("Python dà errore {}, qui {} occorrenze", sintesi(atteso), v.len()));
            }
            let x = ordina(atteso.as_arr().expect("lista"), chiavi);
            let y = ordina(v, chiavi);
            if canonico(&Json::Arr(x.clone())) == canonico(&Json::Arr(y.clone())) {
                return uguale();
            }
            let i = (0..x.len()).find(|k| y.get(*k).is_none_or(|b| canonico(&x[*k]) != canonico(b))).unwrap_or(x.len().min(y.len()));
            diverso(format!(
                "Python {}, qui {}; primo diverso: atteso {} ottenuto {}",
                x.len(),
                y.len(),
                x.get(i).map(sintesi).unwrap_or_else(|| "niente".to_string()),
                y.get(i).map(sintesi).unwrap_or_else(|| "niente".to_string())
            ))
        }
    }
}

fn grezza_json(o: &OccorrenzaAndroid) -> Json {
    ogg! { "uid": o.uid.as_str(), "titolo": o.titolo.as_str(), "luogo": o.luogo.as_str(), "inizio": o.inizio.as_str(), "fine": o.fine.as_str(), "tuttoGiorno": o.tutto_giorno }
}

fn finestra(f: &str) -> (&str, &str) {
    f.split_once('|').expect("finestra")
}

fn espandi_android(righe: &Json, inizio: &str, fine: &str) -> Result<Vec<OccorrenzaAndroid>, Errore> {
    let righe: Vec<_> = righe.as_arr().expect("righe").iter().map(|r| riga_android(r.as_obj().expect("riga"))).collect();
    espandi_righe_android(&righe, leggi_iso_locale(inizio)?, leggi_iso_locale(fine)?, None)
}

fn esito_android(caso: &Json, f: &str) -> Vec<Esito> {
    let (inizio, fine) = finestra(f);
    let atteso = &caso["atteso"][f];
    let grezzi = espandi_android(&caso["righe"], inizio, fine);
    let registrati = grezzi.as_ref().map(|v| v.iter().map(grezza_json).collect::<Vec<_>>()).map_err(Clone::clone);
    let mut e1 = confronta_lista(&atteso["grezzi"], &registrati, &["uid", "inizio", "fine", "titolo", "luogo"]);
    e1.dettaglio = if e1.dettaglio.is_empty() { e1.dettaglio } else { format!("grezzi: {}", e1.dettaglio) };
    if atteso.get("finali").is_none() {
        return vec![e1];
    }
    // dei doppioni tolti si confrontano i campi che contano
    let finali = (|| -> Result<Vec<Json>, Errore> {
        let sql = db_in_memoria(&calendari_prova(), &caso["righe"], &Json::Arr(Vec::new()));
        let mut tipo = None;
        let lettura = estrai(&sql, ACCOUNT_PROVA, &[], &mut tipo)
            .map_err(|e| match e {
                oe_agente::fonte::ErroreLettura::Sqlite(m) => Errore::Altro(m),
                oe_agente::fonte::ErroreLettura::Altro(e) => e,
            })?
            .ok_or_else(|| Errore::Tipo("Cannot read properties of null (reading 'map')".to_string()))?;
        let info = InfoCasella { id: ID_PROVA, nome: "Alfa", account: ACCOUNT_PROVA, tipo: tipo.as_deref() };
        let eventi = eventi_da_lettura(&lettura, &info, leggi_iso_locale(inizio)?, leggi_iso_locale(fine)?, "Europe/Rome", grezzi.as_ref().ok().cloned())?;
        Ok(eventi
            .iter()
            .map(|e| {
                let mut m = Mappa::new();
                for k in ["id", "start", "end", "allDay", "title", "location"] {
                    m.insert(k, e[k].clone());
                }
                Json::Obj(m)
            })
            .collect())
    })();
    let mut e2 = confronta_lista(&atteso["finali"], &finali, &["start", "title", "id", "end"]);
    e2.dettaglio = if e2.dettaglio.is_empty() { e2.dettaglio } else { format!("finali: {}", e2.dettaglio) };
    vec![e1, e2]
}

fn e_valore(e: &Errore) -> bool {
    matches!(e, Errore::Valore(_) | Errore::Tipo(_))
}

fn esito_ics(caso: &Json, f: &str) -> Vec<Esito> {
    let (inizio, fine) = finestra(f);
    let atteso = &caso["atteso"]["eventi"][f];
    let testo = caso["testo"].as_str().expect("testo");
    let ottenuto = espandi_ics(testo, inizio, fine, None).map(|v| v.iter().map(EventoIcs::in_json).collect::<Vec<_>>());
    let ok = atteso["ok"].as_bool() == Some(true);
    let confronto = if ok { atteso["eventi"].clone() } else { ogg! { "errore": atteso["errore"].clone(), "messaggio": atteso["messaggio"].clone() } };
    let e1 = confronta_lista(&confronto, &ottenuto, &["inizio", "uid", "fine", "titolo", "luogo"]);
    let mut out = Vec::new();
    let aggiunta = match (&ottenuto, ok, &e1.stato) {
        (Err(e), false, Stato::Uguale) if Some(e_valore(e)) != atteso["valore"].as_bool() => Some(diverso(format!("tipo di errore: Python valore={}, qui {}", atteso["valore"].in_js(), e_valore(e)))),
        _ => None,
    };
    out.push(e1);
    out.extend(aggiunta);
    out
}

fn esito_valida(caso: &Json) -> Esito {
    let atteso = &caso["atteso"]["valida"];
    match valida_ics(caso["testo"].as_str().expect("testo")) {
        Err(e) => {
            let messaggio = primi_utf16(e.messaggio(), 200);
            if atteso["ok"].as_bool() != Some(true) && Some(e_valore(&e)) == atteso["valore"].as_bool() && Some(messaggio.as_str()) == atteso["messaggio"].as_str() {
                uguale()
            } else {
                diverso(format!("valida: errore {e:?} contro {}", sintesi(atteso)))
            }
        }
        Ok(nome) => {
            if atteso["ok"].as_bool() == Some(true) && atteso["nome"].as_str() == Some(nome.as_str()) {
                uguale()
            } else {
                diverso(format!("valida: {nome:?} contro {}", sintesi(atteso)))
            }
        }
    }
}

fn finestre(caso: &Json) -> Vec<String> {
    caso["finestre"].as_arr().expect("finestre").iter().map(|f| f.as_arr().expect("finestra").iter().map(|x| x.as_str().unwrap_or_default()).collect::<Vec<_>>().join("|")).collect()
}

fn esito_caso(caso: &Json) -> Esito {
    let mut esiti = Vec::new();
    if caso["tipo"].as_str() == Some("android") {
        for f in finestre(caso) {
            esiti.extend(esito_android(caso, &f));
        }
    } else {
        esiti.push(esito_valida(caso));
        for f in finestre(caso) {
            esiti.extend(esito_ics(caso, &f));
        }
    }
    let diversi: Vec<&Esito> = esiti.iter().filter(|e| e.stato == Stato::Diverso).collect();
    if !diversi.is_empty() {
        return diverso(diversi.iter().take(2).map(|e| e.dettaglio.as_str()).collect::<Vec<_>>().join(" || "));
    }
    match esiti.into_iter().find(|e| e.stato == Stato::NonSupportato) {
        Some(e) => e,
        None => uguale(),
    }
}

#[test]
fn serie_casuali_uguali_a_python() {
    let casuali = leggi_golden("ricorrenze-casuali.json");
    let casi = casuali["casi"].as_arr().expect("casi");
    let mut diversi = Vec::new();
    let mut contati = 0;
    for tipo in ["android", "ics"] {
        for c in casi.iter().filter(|c| c["tipo"].as_str() == Some(tipo)) {
            contati += 1;
            let e = esito_caso(c);
            if e.stato != Stato::Uguale {
                diversi.push(format!("{}: {}", c["nome"].as_str().unwrap_or_default(), e.dettaglio));
            }
        }
    }
    let (confronti, occorrenze) = conteggi();
    println!("casuali: {contati} casi, {confronti} confronti, {occorrenze} occorrenze attese");
    assert!(contati > 0 && confronti >= contati && occorrenze > contati);
    assert_eq!(diversi, Vec::<String>::new(), "{} casi diversi su {contati}", diversi.len());
}

#[test]
fn casi_inventati_uguali_a_python() {
    let extra = leggi_golden("ricorrenze-extra.json");
    let casi = extra["casi"].as_arr().expect("casi");
    let mut categorie: Vec<String> = Vec::new();
    for c in casi {
        let cat = format!("{}/{}", c["tipo"].as_str().unwrap_or_default(), c["categoria"].as_str().unwrap_or_default());
        if !categorie.contains(&cat) {
            categorie.push(cat);
        }
    }
    let mut diversi = Vec::new();
    for cat in &categorie {
        for c in casi.iter().filter(|c| &format!("{}/{}", c["tipo"].as_str().unwrap_or_default(), c["categoria"].as_str().unwrap_or_default()) == cat) {
            let e = esito_caso(c);
            if e.stato != Stato::Uguale {
                diversi.push(format!("{cat} {}: {:?} {}", c["nome"].as_str().unwrap_or_default(), e.stato, e.dettaglio));
            }
        }
    }
    let (confronti, occorrenze) = conteggi();
    println!("extra: {} casi in {} categorie, {confronti} confronti, {occorrenze} occorrenze attese", casi.len(), categorie.len());
    assert!(confronti >= casi.len() && occorrenze > casi.len());
    assert_eq!(diversi, Vec::<String>::new(), "{} casi diversi su {}", diversi.len(), casi.len());
}

fn impronta(v: &Json) -> String {
    let h = Sha256::digest(canonico(v).as_bytes());
    h.iter().map(|b| format!("{b:02x}")).collect::<String>()[..20].to_string()
}

fn riassunto(lista: Vec<Json>) -> Json {
    let mut ordinata = lista.clone();
    ordinata.sort_by_cached_key(canonico);
    ogg! { "n": lista.len(), "h": impronta(&Json::Arr(lista)), "hs": impronta(&Json::Arr(ordinata)) }
}

fn errore_json(e: &Errore) -> Json {
    let tipo = match e {
        Errore::Valore(_) => "ValueError",
        Errore::Tipo(_) => "TypeError",
        _ => "?",
    };
    ogg! { "errore": tipo, "messaggio": primi_utf16(e.messaggio(), 200) }
}

#[test]
fn regole_orarie_settimane_e_pasqua_uguali_a_python() {
    let mut casi: Vec<Json> = Vec::new();
    for nome in ["ricorrenze-orarie.json", "ricorrenze-rare.json"] {
        casi.extend(leggi_golden(nome)["casi"].as_arr().expect("casi").iter().cloned());
    }
    let mut categorie: Vec<String> = Vec::new();
    for c in &casi {
        let cat = c["categoria"].as_str().unwrap_or_default().to_string();
        if !categorie.contains(&cat) {
            categorie.push(cat);
        }
    }
    let mut diversi = Vec::new();
    let mut finestre_viste = 0;
    for cat in &categorie {
        for caso in casi.iter().filter(|c| c["categoria"].as_str() == Some(cat)) {
            let attesi = caso["atteso"].as_obj().expect("atteso");
            let mut ottenuti = Mappa::new();
            for k in finestre(caso) {
                let (inizio, fine) = finestra(&k);
                let o = if caso["tipo"].as_str() == Some("android") {
                    espandi_android(&caso["righe"], inizio, fine).map(|v| riassunto(v.iter().map(grezza_json).collect()))
                } else {
                    espandi_ics(caso["testo"].as_str().unwrap_or_default(), inizio, fine, None).map(|v| riassunto(v.iter().map(EventoIcs::in_json).collect()))
                };
                let o = o.unwrap_or_else(|e| errore_json(&e));
                let a = attesi.get(&k).cloned().unwrap_or_default();
                let valore = if a.get("errore").is_some() {
                    let mut m = Mappa::new();
                    m.insert("errore", a["errore"].clone());
                    m.insert("messaggio", o.get("messaggio").cloned().unwrap_or_default());
                    if let Some(v) = a.get("valore") {
                        m.insert("valore", v.clone());
                    }
                    Json::Obj(m)
                } else {
                    o
                };
                ottenuti.insert(k, valore);
            }
            if caso["tipo"].as_str() == Some("ics") && attesi.get("valida").is_some_and(|v| v.get("errore").is_some()) && valida_ics(caso["testo"].as_str().unwrap_or_default()).is_ok() {
                diversi.push(format!("{cat} {}: valida non dà errore", caso["nome"].as_str().unwrap_or_default()));
            }
            let attesi_senza: Mappa = attesi.iter().filter(|(k, _)| *k != "valida").map(|(k, v)| (k.clone(), v.clone())).collect();
            finestre_viste += ottenuti.len();
            let (a, o) = (canonico(&Json::Obj(attesi_senza)), canonico(&Json::Obj(ottenuti)));
            if a != o {
                diversi.push(format!("{cat} {}: atteso {} ottenuto {}", caso["nome"].as_str().unwrap_or_default(), primi_utf16(&a, 300), primi_utf16(&o, 300)));
            }
        }
    }
    println!("orarie e rare: {} casi, {finestre_viste} finestre", casi.len());
    assert!(finestre_viste >= casi.len());
    assert_eq!(diversi, Vec::<String>::new(), "{} casi diversi su {}", diversi.len(), casi.len());
}
