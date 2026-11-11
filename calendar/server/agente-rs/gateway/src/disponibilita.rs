//! La pagina pubblica (/disponibilita) è il calendario di sempre, senza accesso, con gli impegni ridotti a "Occupato" (era lib/disponibilita.ts).
//! Qui le regole dell'API che la serve (/api/disponibilita): dalla lettura dei calendari (get_disponibilita) arrivano solo orari e festività,
//! e quello che arriva si ricostruisce campo per campo.

use crate::fuso::{da_istante, pezzi_utc, utc_js};
use crate::web::Resp;
use indexmap::IndexMap;
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};

pub const OCCUPATO: &str = "Occupato";
pub const COLORE_OCCUPATO: &str = "#6366f1";
pub const RANGE_MAX: i64 = 62;
pub const ANNI_MAX_GIORNI: i64 = 400;
pub const GRUPPO_MAX: usize = 60;
/// Le letture sono a blocchi di 28 giorni fissi: chi guarda sposta le viste dentro lo stesso blocco.
pub const GIORNI_BLOCCO: i64 = 28;
// un lunedì: da qui partono i blocchi
const ANCORA: &str = "2024-01-01";

pub type Coppia = [String; 2];
pub type Festa = [String; 3];

// ---- Date: sempre testi AAAA-MM-GG, senza il fuso orario del server ---------------------------------
fn pezzi(s: &str) -> Option<(i64, i64, i64)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || ![0, 1, 2, 3, 5, 6, 8, 9].iter().all(|i| b[*i].is_ascii_digit()) {
        return None;
    }
    Some((s[0..4].parse().ok()?, s[5..7].parse().ok()?, s[8..10].parse().ok()?))
}

pub fn data_valida(s: &str) -> bool {
    let Some((a, m, g)) = pezzi(s) else { return false };
    let (a2, m2, g2, ..) = pezzi_utc(utc_js(a, m - 1, g, 0, 0, 0));
    a2 == a && m2 == m && g2 == g
}

/// `Date.UTC` del testo: le parti si leggono con Number, come in JavaScript (qui le date arrivano sempre valide).
fn giorno(s: &str) -> i64 {
    let mut p = s.split('-').map(|x| x.parse::<i64>().unwrap_or(0));
    let (a, m, g) = (p.next().unwrap_or(0), p.next().unwrap_or(1), p.next().unwrap_or(1));
    utc_js(a, m - 1, g, 0, 0, 0)
}

pub fn giorni_tra(da: &str, a: &str) -> i64 {
    ((giorno(a) - giorno(da)) as f64 / 86_400_000.0).round() as i64
}

pub fn sposta_data(data: &str, giorni: i64) -> String {
    let (a, m, g, ..) = pezzi_utc(giorno(data) + giorni * 86_400_000);
    format!("{a:04}-{m:02}-{g:02}")
}

/// Che giorno è a Roma.
pub fn oggi_a_roma(adesso_ms: i64) -> String {
    let n = da_istante(adesso_ms, 0);
    format!("{:04}-{:02}-{:02}", n.a, n.m, n.g)
}

// ---- La richiesta ----------------------------------------------------------------------------------
pub struct Richiesta {
    pub da: String,
    pub a: String,
    pub gruppo: String,
}

fn controllo(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}')
}

/// `da` e `a` come li manda la pagina (2026-10-12T00:00:00+02:00): conta il giorno, `a` è escluso.
pub fn leggi_richiesta(q: &url::Url) -> Result<Richiesta, String> {
    let primo = |k: &str| q.query_pairs().find(|(x, _)| x == k).map(|(_, v)| v.into_owned()).unwrap_or_default();
    let da = crate::py::taglia_js(&primo("da"), 10).to_string();
    let a = crate::py::taglia_js(&primo("a"), 10).to_string();
    if !data_valida(&da) || !data_valida(&a) {
        return Err("da, a: servono date AAAA-MM-GG".to_string());
    }
    let giorni = giorni_tra(&da, &a);
    if !(1..=RANGE_MAX).contains(&giorni) {
        return Err(format!("a: da 1 a {RANGE_MAX} giorni dopo da"));
    }
    let gruppo = crate::py::js_trim(&primo("gruppo")).to_string();
    if crate::py::lunghezza_js(&gruppo) > GRUPPO_MAX || gruppo.chars().any(controllo) {
        return Err("gruppo: nome non valido".to_string());
    }
    Ok(Richiesta { da, a, gruppo })
}

/// Troppo indietro o troppo avanti: si risponde senza impegni, senza disturbare i calendari.
pub fn fuori_finestra(r: &Richiesta, oggi: &str) -> bool {
    r.a < sposta_data(oggi, -ANNI_MAX_GIORNI) || r.da > sposta_data(oggi, ANNI_MAX_GIORNI)
}

/// I blocchi fissi (la loro data di inizio) che coprono da `da` a `a`.
pub fn blocchi(da: &str, a: &str) -> Vec<String> {
    let primo = giorni_tra(ANCORA, da).div_euclid(GIORNI_BLOCCO);
    let ultimo = giorni_tra(ANCORA, &sposta_data(a, -1)).div_euclid(GIORNI_BLOCCO);
    (primo..=ultimo).map(|i| sposta_data(ANCORA, i * GIORNI_BLOCCO)).collect()
}

/// Impegni che si sovrappongono o si toccano sono un blocco solo; gli orari si confrontano come testi.
pub fn unisci(impegni: &[Coppia]) -> Vec<Coppia> {
    let mut ordinati = impegni.to_vec();
    ordinati.sort_by(|x, y| crate::stato::confronta_utf16(&x[0], &y[0]));
    let mut uniti: Vec<Coppia> = Vec::new();
    for [i, f] in ordinati {
        match uniti.last_mut() {
            Some(u) if i <= u[1] => {
                if f > u[1] {
                    u[1] = f;
                }
            }
            _ => uniti.push([i, f]),
        }
    }
    uniti
}

/// Impegni e festività dei blocchi, tenuti solo se toccano da-a, nella forma degli eventi del calendario.
pub fn eventi_pubblici(impegni: &[Coppia], festivita: &[Festa], da: &str, a: &str) -> Vec<Json> {
    let inizio = format!("{da}T00:00:00");
    let fine = format!("{a}T00:00:00");
    let mut visti = HashSet::new();
    let mut out = Vec::new();
    for [i, f] in unisci(impegni) {
        if f <= inizio || i >= fine {
            continue;
        }
        out.push(ogg! { "id": format!("occ{}", out.len()), "title": OCCUPATO, "start": i, "end": f, "allDay": false, "color": COLORE_OCCUPATO, "account": "" });
    }
    for [i, f, nome] in festivita {
        if !visti.insert(format!("{i}|{f}|{nome}")) || *f <= inizio || *i >= fine {
            continue;
        }
        out.push(ogg! { "id": format!("festa{}", out.len()), "title": nome.clone(), "start": i.clone(), "end": f.clone(), "allDay": true, "color": "#8a8a8a", "account": "Festività" });
    }
    out
}

// ---- Lettura dei blocchi ---------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub enum Esito {
    Buono { impegni: Vec<Coppia>, festivita: Vec<Festa>, completo: bool },
    Guasto(u16),
}

const TTL_MS: i64 = 60_000;
const TTL_GUASTO_MS: i64 = 15_000;
/// Fin quando un blocco letto bene si può ancora dare subito, mentre se ne legge uno nuovo: la prenotazione ricontrolla comunque i calendari.
pub const VECCHIO_MAX_MS: i64 = 6 * 3600_000;
const CACHE_MAX: usize = 64;
const LETTURE_MAX: usize = 20;
const FINESTRA_MS: i64 = 60_000;
/// Il giro che tiene pronti i blocchi: ogni due minuti, da una settimana fa a 61 giorni avanti (le prenotazioni arrivano a 60).
pub const CALDO_OGNI_MS: u64 = 120_000;
const CALDO_INDIETRO: i64 = 7;
const CALDO_AVANTI: i64 = 61;
/// I gruppi chiesti dalla pagina (?gruppo=) restano caldi per un giorno, al più otto.
const RICHIESTI_PER_MS: i64 = 24 * 3600_000;
const RICHIESTI_MAX: usize = 8;

struct Voce {
    fino: i64,
    scade: i64,
    esito: Esito,
}

#[derive(Default)]
struct Memoria {
    cache: IndexMap<String, Voce>,
    in_corso: IndexMap<String, tokio::sync::watch::Receiver<Option<Esito>>>,
    letture: VecDeque<i64>,
    richiesti: IndexMap<String, i64>,
}

thread_local! {
    static MEMORIA: RefCell<Memoria> = RefCell::new(Memoria::default());
}

/// Dimentica i blocchi letti, non il conteggio delle letture: dopo una prenotazione la pagina rilegge e la vede subito.
pub fn dimentica_disponibilita() {
    MEMORIA.with(|m| m.borrow_mut().cache.clear());
}

fn iso_valido(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':' && b.iter().enumerate().all(|(i, c)| [4, 7, 10, 13, 16].contains(&i) || c.is_ascii_digit())
}

/// Quello che arriva dalla lettura si ricostruisce: solo coppie di orari e nomi di festa, il resto non passa.
fn ripulisci(d: &Json) -> Esito {
    let guasto = Esito::Guasto(502);
    if !matches!(d, Json::Obj(_)) {
        return guasto;
    }
    if d.get("ok") == Some(&Json::Bool(false)) {
        return if d.get("error").and_then(Json::as_str) == Some("Gruppo non trovato") { Esito::Guasto(404) } else { guasto };
    }
    let (Some(Json::Arr(imp)), Some(Json::Arr(fes))) = (d.get("impegni"), d.get("festivita")) else { return guasto };
    if d.get("ok") != Some(&Json::Bool(true)) || imp.len() > 5000 || fes.len() > 200 {
        return guasto;
    }
    let mut impegni = Vec::new();
    for x in imp {
        let Json::Arr(c) = x else { return guasto };
        let [Json::Str(i), Json::Str(f)] = c.as_slice() else { return guasto };
        if !iso_valido(i) || !iso_valido(f) || i >= f {
            return guasto;
        }
        impegni.push([i.clone(), f.clone()]);
    }
    let mut festivita = Vec::new();
    for x in fes {
        let Json::Arr(c) = x else { return guasto };
        let [Json::Str(i), Json::Str(f), Json::Str(n)] = c.as_slice() else { return guasto };
        if !iso_valido(i) || !iso_valido(f) {
            return guasto;
        }
        let nome: String = n.chars().map(|c| if controllo(c) { ' ' } else { c }).collect();
        festivita.push([i.clone(), f.clone(), crate::py::taglia_js(&nome, 80).to_string()]);
    }
    Esito::Buono { impegni, festivita, completo: d.get("completo") == Some(&Json::Bool(true)) }
}

type Lettore = std::rc::Rc<dyn Fn(String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Esito>>>>;

thread_local! {
    /// Solo per le prove: la lettura dei calendari al posto di get_disponibilita.
    static LETTORE_DI_PROVA: RefCell<Option<Lettore>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn imposta_lettore(l: Option<Lettore>) {
    LETTORE_DI_PROVA.with(|x| *x.borrow_mut() = l);
}

async fn leggi(da: &str, gruppo: &str) -> Esito {
    if let Some(l) = LETTORE_DI_PROVA.with(|x| x.borrow().clone()) {
        return l(da.to_string(), gruppo.to_string()).await;
    }
    let args = vec![Json::Str(da.to_string()), Json::Num(GIORNI_BLOCCO as f64), Json::Str(gruppo.to_string())];
    match crate::api::chiama("get_disponibilita", args).await {
        Ok(d) => ripulisci(&d),
        Err(_) => Esito::Guasto(502),
    }
}

fn buono(e: &Esito) -> bool {
    matches!(e, Esito::Buono { .. })
}

/// Legge il blocco e lo mette in memoria (richieste uguali in corso si fondono in una lettura). Una lettura che non riesce non prende il
/// posto di un blocco letto bene: quello resta, e si riprova fra quindici secondi.
async fn aggiorna(chiave: &str, da: &str, gruppo: &str, adesso: i64) -> Esito {
    let (tx, rx_in_corso) = tokio::sync::watch::channel(None);
    let in_corso = MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        match m.in_corso.get(chiave) {
            Some(rx) => Some(rx.clone()),
            None => {
                m.in_corso.insert(chiave.to_string(), rx_in_corso);
                None
            }
        }
    });
    if let Some(mut rx) = in_corso {
        let _ = rx.wait_for(Option::is_some).await;
        let valore = rx.borrow().clone();
        return valore.unwrap_or(Esito::Guasto(502));
    }
    let letto = leggi(da, gruppo).await;
    let esito = MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        m.in_corso.shift_remove(chiave);
        if matches!(letto, Esito::Guasto(s) if s != 404) {
            if let Some(prima) = m.cache.get_mut(chiave).filter(|v| buono(&v.esito) && v.scade > adesso) {
                prima.fino = adesso + TTL_GUASTO_MS;
                return prima.esito.clone();
            }
        }
        m.cache.shift_remove(chiave);
        if m.cache.len() >= CACHE_MAX {
            m.cache.shift_remove_index(0);
        }
        let completo = match &letto {
            Esito::Buono { completo, .. } => *completo,
            Esito::Guasto(s) => *s == 404,
        };
        let scade = if buono(&letto) { adesso + VECCHIO_MAX_MS } else { 0 };
        m.cache.insert(chiave.to_string(), Voce { fino: adesso + if completo { TTL_MS } else { TTL_GUASTO_MS }, scade, esito: letto.clone() });
        letto
    });
    let _ = tx.send(Some(esito.clone()));
    esito
}

/// Il limite delle letture vere: al più venti al minuto in tutto. Vero se c'è posto (e lo prende).
fn posto_libero(m: &mut Memoria, adesso: i64) -> bool {
    while m.letture.front().is_some_and(|l| *l <= adesso - FINESTRA_MS) {
        m.letture.pop_front();
    }
    if m.letture.len() >= LETTURE_MAX {
        return false;
    }
    m.letture.push_back(adesso);
    true
}

/// Un blocco di 28 giorni, per una richiesta pubblica: il risultato resta in memoria un minuto (un guasto quindici secondi), e dopo si dà
/// ancora subito, fino a sei ore, mentre se ne legge uno nuovo in secondo piano: una lettura dei calendari dura secondi e chi apre la pagina
/// non la aspetta. Le letture vere sono al più venti al minuto in tutto: oltre, 429 se non c'è niente da dare.
pub async fn leggi_blocco(da: &str, gruppo: &str) -> Esito {
    let adesso = crate::orologio::adesso();
    let chiave = format!("{da}|{gruppo}");
    enum Passo {
        Fatto(Esito),
        Vecchio(Esito, bool),
        Leggi,
    }
    let passo = MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(v) = m.cache.get(&chiave) {
            if v.fino > adesso {
                return Passo::Fatto(v.esito.clone());
            }
            if buono(&v.esito) && v.scade > adesso {
                let e = v.esito.clone();
                let rileggi = !m.in_corso.contains_key(&chiave) && posto_libero(&mut m, adesso);
                return Passo::Vecchio(e, rileggi);
            }
        }
        if m.in_corso.contains_key(&chiave) || posto_libero(&mut m, adesso) { Passo::Leggi } else { Passo::Fatto(Esito::Guasto(429)) }
    });
    match passo {
        Passo::Fatto(e) => e,
        Passo::Vecchio(e, rileggi) => {
            if rileggi {
                let (c, d, g) = (chiave.clone(), da.to_string(), gruppo.to_string());
                tokio::task::spawn_local(async move {
                    aggiorna(&c, &d, &g, adesso).await;
                });
            }
            e
        }
        Passo::Leggi => aggiorna(&chiave, da, gruppo, adesso).await,
    }
}

/// Un gruppo chiesto dalla pagina pubblica: il giro lo tiene caldo per un giorno.
fn ricorda_gruppo(gruppo: &str, adesso: i64) {
    if gruppo.is_empty() {
        return;
    }
    MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        m.richiesti.shift_remove(gruppo);
        m.richiesti.insert(gruppo.to_string(), adesso);
        if m.richiesti.len() > RICHIESTI_MAX {
            m.richiesti.shift_remove_index(0);
        }
    });
}

/// Un giro: rilegge, uno alla volta, i blocchi da una settimana fa a 61 giorni avanti, per tutti i calendari e per i gruppi chiesti
/// nell'ultimo giorno. Fuori dal limite delle letture pubbliche, che resta tutto per chi apre la pagina. Restituisce quanti blocchi ha letto.
pub async fn rinfresca(oggi: &str) -> usize {
    let adesso = crate::orologio::adesso();
    let gruppi: Vec<String> = MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        m.richiesti.retain(|_, quando| adesso - *quando <= RICHIESTI_PER_MS);
        std::iter::once(String::new()).chain(m.richiesti.keys().cloned()).collect()
    });
    let mut letti = 0;
    for gruppo in gruppi {
        for b in blocchi(&sposta_data(oggi, -CALDO_INDIETRO), &sposta_data(oggi, CALDO_AVANTI)) {
            let esito = aggiorna(&format!("{b}|{gruppo}"), &b, &gruppo, crate::orologio::adesso()).await;
            letti += 1;
            // un gruppo che non c'è più non si rilegge
            if matches!(esito, Esito::Guasto(404)) {
                MEMORIA.with(|m| m.borrow_mut().richiesti.shift_remove(&gruppo));
                break;
            }
        }
    }
    letti
}

/// Fa partire il giro: il primo dopo cinque secondi, così la pagina è pronta subito dopo un riavvio.
pub fn avvia_disponibilita() {
    tokio::task::spawn_local(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        loop {
            rinfresca(&oggi_a_roma(crate::orologio::ora_vera())).await;
            tokio::time::sleep(std::time::Duration::from_millis(CALDO_OGNI_MS)).await;
        }
    });
    println!("disponibilità: blocchi tenuti pronti, un giro ogni {} s", CALDO_OGNI_MS / 1000);
}

fn risposta(stato: u16, corpo: &Json, extra: &[(&str, &str)]) -> Resp {
    let mut h = vec![("cache-control", "no-store"), ("x-robots-tag", "noindex, nofollow")];
    h.extend_from_slice(extra);
    Resp::json(stato, corpo, &h)
}

/// GET /api/disponibilita: pubblica, niente sessione e quindi niente dettagli; nella forma di get_events (events, errors).
pub async fn get(url: &url::Url) -> Resp {
    let r = match leggi_richiesta(url) {
        Ok(r) => r,
        Err(e) => return risposta(400, &ogg! { "error": e }, &[]),
    };
    if fuori_finestra(&r, &oggi_a_roma(crate::orologio::ora_vera())) {
        return risposta(200, &ogg! { "events": Json::Arr(vec![]), "errors": Json::Arr(vec![]) }, &[]);
    }
    ricorda_gruppo(&r.gruppo, crate::orologio::adesso());
    let esiti = futures_util::future::join_all(blocchi(&r.da, &r.a).iter().map(|b| leggi_blocco(b, &r.gruppo))).await;
    let stati: Vec<u16> = esiti.iter().filter_map(|e| if let Esito::Guasto(s) = e { Some(*s) } else { None }).collect();
    if !stati.is_empty() {
        // il più specifico: gruppo che non esiste, poi troppe richieste
        let stato = [404, 429, 502].into_iter().find(|s| stati.contains(s)).unwrap_or(502);
        return match stato {
            404 => risposta(404, &ogg! { "error": "Non c'è nessun calendario con questo nome." }, &[]),
            429 => risposta(429, &ogg! { "error": "Troppe richieste in poco tempo: riprova fra un minuto." }, &[("retry-after", "30")]),
            _ => risposta(502, &ogg! { "error": "La disponibilità non si legge in questo momento: riprova fra poco." }, &[]),
        };
    }
    let (mut impegni, mut festivita) = (Vec::new(), Vec::new());
    for e in esiti {
        if let Esito::Buono { impegni: i, festivita: f, .. } = e {
            impegni.extend(i);
            festivita.extend(f);
        }
    }
    // un calendario che non si legge (un accesso scaduto) si vede nella pagina principale, con il pallino rosso: qui non si dice
    risposta(200, &ogg! { "events": Json::Arr(eventi_pubblici(&impegni, &festivita, &r.da, &r.a)), "errors": Json::Arr(vec![]) }, &[])
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn i_blocchi_e_le_date() {
        // 1043 giorni dall'ancora: il blocco 37 comincia 1036 giorni dopo
        assert_eq!(blocchi("2026-11-09", "2026-11-16"), vec!["2026-11-02".to_string()]);
        assert_eq!(blocchi("2026-11-20", "2026-11-30").len(), 1);
        assert_eq!(blocchi("2026-11-25", "2026-12-05"), vec!["2026-11-02".to_string(), "2026-11-30".to_string()]);
        assert!(data_valida("2028-02-29") && !data_valida("2026-02-29") && !data_valida("2026-1-01"));
        assert_eq!(sposta_data("2026-12-31", 1), "2027-01-01");
        assert_eq!(oggi_a_roma(1_794_438_000_000), "2026-11-12");
    }

    #[test]
    fn impegni_uniti() {
        let c = |a: &str, b: &str| [a.to_string(), b.to_string()];
        let u = unisci(&[c("2026-11-11T10:00:00", "2026-11-11T11:00:00"), c("2026-11-11T09:00:00", "2026-11-11T10:00:00"), c("2026-11-11T12:00:00", "2026-11-11T13:00:00")]);
        assert_eq!(u, vec![c("2026-11-11T09:00:00", "2026-11-11T11:00:00"), c("2026-11-11T12:00:00", "2026-11-11T13:00:00")]);
    }

    /// Una lettura finta: la n-esima risponde un impegno che comincia all'ora n, o un guasto se `guasta(n)`.
    fn finta(guasta: fn(usize) -> bool) -> (std::rc::Rc<std::cell::Cell<usize>>, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
        let n = std::rc::Rc::new(std::cell::Cell::new(0));
        let chiesti = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (n2, c2) = (n.clone(), chiesti.clone());
        imposta_lettore(Some(std::rc::Rc::new(move |da: String, gruppo: String| {
            n2.set(n2.get() + 1);
            c2.borrow_mut().push(format!("{da}|{gruppo}"));
            let k = n2.get();
            let esito = if gruppo == "sparito" {
                Esito::Guasto(404)
            } else if guasta(k) {
                Esito::Guasto(502)
            } else {
                Esito::Buono { impegni: vec![[format!("2026-11-12T{k:02}:00:00"), "2026-11-12T23:00:00".to_string()]], festivita: vec![], completo: true }
            };
            Box::pin(async move { esito })
        })));
        (n, chiesti)
    }

    async fn lascia_girare() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    fn primo_impegno(e: &Esito) -> String {
        match e {
            Esito::Buono { impegni, .. } => impegni[0][0].clone(),
            Esito::Guasto(s) => s.to_string(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn il_blocco_vecchio_si_da_subito_e_si_rilegge_in_secondo_piano() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (n, _) = finta(|k| k == 3);
                let t0 = 1_794_000_000_000;
                crate::orologio::fissa_ora(Some(t0));
                assert_eq!(primo_impegno(&leggi_blocco("2026-11-02", "").await), "2026-11-12T01:00:00");
                // scaduto il minuto: si dà il vecchio, senza aspettare, e la lettura nuova parte
                crate::orologio::fissa_ora(Some(t0 + 120_000));
                assert_eq!(primo_impegno(&leggi_blocco("2026-11-02", "").await), "2026-11-12T01:00:00");
                lascia_girare().await;
                assert_eq!(n.get(), 2);
                assert_eq!(primo_impegno(&leggi_blocco("2026-11-02", "").await), "2026-11-12T02:00:00");
                // una lettura nuova che non riesce lascia il blocco di prima
                crate::orologio::fissa_ora(Some(t0 + 240_000));
                leggi_blocco("2026-11-02", "").await;
                lascia_girare().await;
                assert_eq!(n.get(), 3);
                assert_eq!(primo_impegno(&leggi_blocco("2026-11-02", "").await), "2026-11-12T02:00:00");
                // oltre sei ore il vecchio non vale più: si aspetta la lettura
                crate::orologio::fissa_ora(Some(t0 + 7 * 3600_000));
                assert_eq!(primo_impegno(&leggi_blocco("2026-11-02", "").await), "2026-11-12T04:00:00");
                crate::orologio::fissa_ora(None);
                imposta_lettore(None);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn il_giro_tiene_pronti_i_blocchi_e_lascia_i_gruppi_spariti() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_, chiesti) = finta(|_| false);
                crate::orologio::fissa_ora(Some(1_794_000_000_000));
                ricorda_gruppo("lavoro", crate::orologio::adesso());
                ricorda_gruppo("sparito", crate::orologio::adesso());
                let attesi = blocchi("2026-11-07", "2027-01-14");
                assert_eq!(rinfresca("2026-11-14").await, attesi.len() * 2 + 1);
                let c = chiesti.borrow().clone();
                assert_eq!(c.iter().filter(|x| x.ends_with('|')).cloned().collect::<Vec<_>>(), attesi.iter().map(|b| format!("{b}|")).collect::<Vec<_>>());
                assert_eq!(c.iter().filter(|x| x.ends_with("|lavoro")).count(), attesi.len());
                chiesti.borrow_mut().clear();
                rinfresca("2026-11-14").await;
                assert!(!chiesti.borrow().iter().any(|x| x.ends_with("|sparito")));
                // dopo il giro la pagina non rilegge
                chiesti.borrow_mut().clear();
                leggi_blocco(&attesi[0], "").await;
                assert!(chiesti.borrow().is_empty());
                crate::orologio::fissa_ora(None);
                imposta_lettore(None);
            })
            .await;
    }
}
