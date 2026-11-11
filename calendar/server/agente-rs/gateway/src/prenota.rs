//! Prenotazione di un meeting dalla pagina pubblica (meet, /disponibilita), era lib/prenota.ts e lib/prenota-regole.ts: chi la apre sceglie
//! uno spazio libero sulla griglia e lascia nome, email, azienda e argomento. L'evento entra subito nel calendario come evento personale che
//! blocca il tempo, per il gruppo della pagina (?gruppo=) o per tutti i gruppi; a noi arriva una notifica push. Le regole: dal lunedì al
//! venerdì, dalle 9 alle 18 di Roma, a quarti d'ora, con due ore di preavviso e fino a 60 giorni avanti, solo in uno spazio libero.

use crate::disponibilita::{dimentica_disponibilita, GRUPPO_MAX};
use crate::fuso::{istante, leggi_naive, utc_js, Naive};
use crate::orologio::adesso;
use crate::web::Resp;
use oe_nucleo::json::Json;
use oe_nucleo::ogg;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Prenotazioni accettate in un'ora, in tutto: dietro sslh chi prenota non si distingue, il limite è uno solo.
pub const PRENOTAZIONI_ORA: usize = 6;
const CORPO_MAX: usize = 8 * 1024;
pub const PASSO_MIN: i64 = 15;
pub const APRE_MIN: i64 = 9 * 60;
pub const CHIUDE_MIN: i64 = 18 * 60;
pub const PREAVVISO_MIN: i64 = 120;
pub const GIORNI_AVANTI: i64 = 60;

const LIMITE_NOME: usize = 100;
const LIMITE_EMAIL: usize = 254;
const LIMITE_AZIENDA: usize = 100;
const LIMITE_ARGOMENTO: usize = 1000;

pub struct Richiesta {
    pub inizio: Naive,
    pub fine: Naive,
    pub nome: String,
    pub email: String,
    pub azienda: String,
    pub argomento: String,
    pub gruppo: String,
}

fn controllo(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}')
}

/// Il testo di un campo: a capo e tabulazioni diventano uno spazio (o restano, per l'argomento), i caratteri di controllo pure, tagliato a max + 1.
fn pulito(v: Option<&Json>, max: usize, righe: bool) -> String {
    let Some(Json::Str(s)) = v else { return String::new() };
    let mut t = String::with_capacity(s.len());
    let mut in_a_capo = false;
    for c in s.chars() {
        if !righe && matches!(c, '\r' | '\n' | '\t') {
            if !in_a_capo {
                t.push(' ');
            }
            in_a_capo = true;
            continue;
        }
        in_a_capo = false;
        t.push(if controllo(c) { ' ' } else { c });
    }
    crate::py::taglia_js(crate::py::js_trim(&t), max + 1).to_string()
}

fn naive_valido(s: &str) -> bool {
    let b = s.as_bytes();
    let base = b.len() >= 16 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15].iter().all(|i| b[*i].is_ascii_digit());
    base && (b.len() == 16 || &s[16..] == ":00")
}

fn email_valida(e: &str) -> bool {
    let vietato = |c: char| crate::py::spazio_js(c) || "@<>()\",;:".contains(c);
    let Some((locale, dominio)) = e.split_once('@') else { return false };
    if locale.is_empty() || locale.chars().any(vietato) || dominio.contains('@') {
        return false;
    }
    // [^...]+\.[^...]{2,}: un punto con almeno un carattere prima e due dopo
    dominio.char_indices().any(|(i, c)| c == '.' && i > 0 && dominio[i + 1..].chars().count() >= 2) && !dominio.chars().any(|c| c != '.' && vietato(c))
}

/// Il corpo della richiesta, controllato campo per campo; la data e gli orari sono ore di Roma senza scarto.
pub fn leggi_richiesta(corpo: &Json) -> Result<Richiesta, String> {
    let Json::Obj(c) = corpo else { return Err("Richiesta non valida.".to_string()) };
    // il campo nascosto: lo riempie solo un programma che compila tutto quello che trova
    if c.get("sito").and_then(Json::as_str).is_some_and(|s| !crate::py::js_trim(s).is_empty()) {
        return Err("Richiesta non valida.".to_string());
    }
    let errata = || "Data o orario non validi.".to_string();
    let (Some(Json::Str(i)), Some(Json::Str(f))) = (c.get("inizio"), c.get("fine")) else { return Err(errata()) };
    if !naive_valido(i) || !naive_valido(f) {
        return Err(errata());
    }
    let (inizio, fine) = (leggi_naive(i).map_err(|_| errata())?, leggi_naive(f).map_err(|_| errata())?);
    let nome = pulito(c.get("nome"), LIMITE_NOME, false);
    let email = pulito(c.get("email"), LIMITE_EMAIL, false);
    let azienda = pulito(c.get("azienda"), LIMITE_AZIENDA, false);
    let argomento = pulito(c.get("argomento"), LIMITE_ARGOMENTO, true);
    let gruppo = pulito(c.get("gruppo"), GRUPPO_MAX, false);
    let l = crate::py::lunghezza_js;
    if l(&nome) < 2 {
        return Err("Scrivi il tuo nome.".to_string());
    }
    if l(&nome) > LIMITE_NOME {
        return Err(format!("Il nome è troppo lungo (al più {LIMITE_NOME} caratteri)."));
    }
    if l(&email) > LIMITE_EMAIL || !email_valida(&email) {
        return Err("L'email non sembra giusta.".to_string());
    }
    if l(&azienda) > LIMITE_AZIENDA {
        return Err(format!("Il nome dell'azienda è troppo lungo (al più {LIMITE_AZIENDA} caratteri)."));
    }
    if l(&argomento) > LIMITE_ARGOMENTO {
        return Err(format!("L'argomento è troppo lungo (al più {LIMITE_ARGOMENTO} caratteri)."));
    }
    if l(&gruppo) > GRUPPO_MAX {
        return Err("Gruppo non valido.".to_string());
    }
    Ok(Richiesta { inizio, fine, nome, email, azienda, argomento, gruppo })
}

fn minuti(n: &Naive) -> i64 {
    n.h * 60 + n.mi
}

fn giorno(n: &Naive) -> String {
    format!("{}-{:02}-{:02}", n.a, n.m, n.g)
}

fn ora(n: &Naive) -> String {
    format!("{:02}:{:02}", n.h, n.mi)
}

fn testo(n: &Naive) -> String {
    format!("{}T{}:00", giorno(n), ora(n))
}

/// Il giorno della settimana (0 domenica) di una data, come getUTCDay.
fn dow(n: &Naive) -> i64 {
    (utc_js(n.a, n.m - 1, n.g, 0, 0, 0).div_euclid(86_400_000) + 4).rem_euclid(7)
}

/// Perché quell'orario non si prenota (frase per chi prenota), o None se va bene. Non sa dei calendari: quello lo dice il server.
pub fn fuori_regola(inizio: &Naive, fine: &Naive, ora_ms: i64) -> Option<String> {
    if giorno(inizio) != giorno(fine) {
        return Some("Il meeting deve iniziare e finire nello stesso giorno.".to_string());
    }
    if inizio.s != 0 || fine.s != 0 || minuti(inizio) % PASSO_MIN != 0 || minuti(fine) % PASSO_MIN != 0 {
        return Some("Gli orari vanno a quarti d'ora (:00, :15, :30, :45).".to_string());
    }
    if minuti(fine) - minuti(inizio) < PASSO_MIN {
        return Some("Il meeting dura almeno 15 minuti.".to_string());
    }
    let d = dow(inizio);
    if d == 0 || d == 6 {
        return Some("Si prenota dal lunedì al venerdì.".to_string());
    }
    if minuti(inizio) < APRE_MIN || minuti(fine) > CHIUDE_MIN {
        return Some("Si prenota dalle 9:00 alle 18:00.".to_string());
    }
    let ms = istante(inizio);
    if ms < ora_ms + PREAVVISO_MIN * 60_000 {
        return Some("Serve almeno due ore di preavviso.".to_string());
    }
    if ms > ora_ms + GIORNI_AVANTI * 86_400_000 {
        return Some(format!("Si prenota al più {GIORNI_AVANTI} giorni avanti."));
    }
    None
}

thread_local! {
    static ACCETTATE: RefCell<VecDeque<i64>> = const { RefCell::new(VecDeque::new()) };
    // una prenotazione alla volta: due richieste per lo stesso spazio non passano entrambe il controllo
    static CODA: Rc<tokio::sync::Mutex<()>> = Rc::new(tokio::sync::Mutex::new(()));
}

/// `Intl.DateTimeFormat("it-IT", { weekday: "short", day: "numeric", month: "short" })`: "mer 11 nov".
fn data_breve(n: &Naive) -> String {
    const GIORNI: [&str; 7] = ["dom", "lun", "mar", "mer", "gio", "ven", "sab"];
    const MESI: [&str; 12] = ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"];
    format!("{} {} {}", GIORNI[dow(n) as usize], n.g, MESI[(n.m - 1) as usize])
}

async fn prenota_ora(corpo: &Json) -> Result<(String, String), (u16, String)> {
    let r = leggi_richiesta(corpo).map_err(|e| (400, e))?;
    let ora_ms = adesso();
    if let Some(regola) = fuori_regola(&r.inizio, &r.fine, ora_ms) {
        return Err((400, regola));
    }
    let piene = ACCETTATE.with(|a| {
        let mut a = a.borrow_mut();
        while a.front().is_some_and(|x| *x <= ora_ms - 3_600_000) {
            a.pop_front();
        }
        a.len() >= PRENOTAZIONI_ORA
    });
    if piene {
        return Err((429, "Troppe prenotazioni in poco tempo: riprova più tardi.".to_string()));
    }
    let cfg = crate::stato::load_config().map_err(|_| (502, "Non riesco a leggere i calendari in questo momento: riprova fra poco.".to_string()))?;
    let (impegni, feste, _) = match crate::eventi::occupato(&cfg, (r.inizio.a, r.inizio.m, r.inizio.g), 1, &r.gruppo).await {
        Ok(x) => x,
        Err(e) if e.messaggio() == "Gruppo non trovato" => return Err((404, "Non c'è nessun calendario con questo nome.".to_string())),
        Err(e) => {
            eprintln!("prenota: calendari non leggibili {}: {}", e.nome(), e.messaggio());
            return Err((502, "Non riesco a leggere i calendari in questo momento: riprova fra poco.".to_string()));
        }
    };
    if !feste.is_empty() {
        return Err((409, "Quel giorno è festivo.".to_string()));
    }
    let (da, a) = (testo(&r.inizio), testo(&r.fine));
    if impegni.iter().any(|[i, f]| *i < a && *f > da) {
        return Err((409, "Questo orario non è più libero: scegline un altro.".to_string()));
    }
    // per il gruppo della pagina; dalla pagina di tutti, per ogni gruppo (si vede e blocca il tempo in ciascuno)
    let gruppi: Vec<Json> = if !r.gruppo.is_empty() {
        vec![Json::Str(r.gruppo.clone())]
    } else {
        match cfg.get("gruppi_calendari") {
            Some(Json::Obj(g)) => g.keys().map(|k| Json::Str(k.clone())).collect(),
            _ => vec![],
        }
    };
    let con_azienda = if r.azienda.is_empty() { String::new() } else { format!(" ({})", r.azienda) };
    let quando_utc = crate::auth::righe::iso_ms(ora_ms);
    let mut note = vec![format!("Prenotato da meet il {} UTC", quando_utc[..16].replace('T', " ")), format!("Email: {}", r.email)];
    if !r.azienda.is_empty() {
        note.push(format!("Azienda: {}", r.azienda));
    }
    if !r.argomento.is_empty() {
        note.push(format!("Argomento: {}", r.argomento));
    }
    let evento = ogg! {
        "title": format!("Meeting con {}{con_azienda}", r.nome),
        "start": da.clone(),
        "end": a.clone(),
        "allDay": false,
        "location": "",
        "notes": note.join("\n"),
        "blocca": true,
        "gruppi": Json::Arr(gruppi),
    };
    crate::locali::aggiungi_evento(Some(&evento)).map_err(|e| {
        eprintln!("prenota: evento non salvato {}: {}", e.nome(), e.messaggio());
        (502, "Non riesco a leggere i calendari in questo momento: riprova fra poco.".to_string())
    })?;
    ACCETTATE.with(|x| x.borrow_mut().push_back(ora_ms));
    dimentica_disponibilita();
    let mut messaggio = vec![format!("Meeting prenotato: {} {}-{}", data_breve(&r.inizio), ora(&r.inizio), ora(&r.fine)), format!("{}{con_azienda} · {}", r.nome, r.email)];
    if !r.argomento.is_empty() {
        messaggio.push(r.argomento.clone());
    }
    let messaggio = messaggio.join("\n");
    if crate::promemoria_push::configurato() {
        // la prenotazione c'è comunque: una notifica che non parte si scrive nel log
        tokio::task::spawn_local(async move {
            let e = crate::promemoria_push::invia(&messaggio).await;
            if !e.ok {
                eprintln!("prenota: notifica {} non inviata: {}", crate::promemoria_push::NOME, e.errore);
            }
        });
    }
    Ok((da, a))
}

fn risposta(stato: u16, corpo: &Json, extra: &[(&str, &str)]) -> Resp {
    let mut h = vec![("cache-control", "no-store"), ("x-robots-tag", "noindex, nofollow")];
    h.extend_from_slice(extra);
    Resp::json(stato, corpo, &h)
}

/// POST /api/prenota: pubblica, come /api/disponibilita. `corpo` è None se più lungo di quanto si legge.
pub async fn post(tipo: &str, corpo: Option<&[u8]>) -> Resp {
    if !tipo.to_lowercase().starts_with("application/json") {
        return risposta(415, &ogg! { "error": "Serve un corpo JSON." }, &[]);
    }
    let grezzo = corpo.map(|c| String::from_utf8_lossy(c).into_owned());
    let Some(grezzo) = grezzo.filter(|g| crate::py::lunghezza_js(g) <= CORPO_MAX) else { return risposta(413, &ogg! { "error": "Richiesta troppo grande." }, &[]) };
    let Ok(corpo) = Json::analizza(grezzo.strip_prefix('\u{feff}').unwrap_or(&grezzo)) else { return risposta(400, &ogg! { "error": "Richiesta non valida." }, &[]) };
    let coda = CODA.with(Rc::clone);
    let _turno = coda.lock().await;
    match prenota_ora(&corpo).await {
        Ok((inizio, fine)) => risposta(200, &ogg! { "ok": true, "inizio": inizio, "fine": fine }, &[]),
        Err((stato, errore)) => risposta(stato, &ogg! { "error": errore }, if stato == 429 { &[("retry-after", "600")] } else { &[] }),
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn le_regole_degli_orari() {
        let n = |s: &str| leggi_naive(s).unwrap();
        let ora = istante(&n("2026-11-11T08:00:00"));
        assert_eq!(fuori_regola(&n("2026-11-11T10:00:00"), &n("2026-11-11T10:30:00"), ora), None);
        assert_eq!(fuori_regola(&n("2026-11-11T09:00:00"), &n("2026-11-11T09:30:00"), ora).unwrap(), "Serve almeno due ore di preavviso.");
        assert_eq!(fuori_regola(&n("2026-11-14T10:00:00"), &n("2026-11-14T10:30:00"), ora).unwrap(), "Si prenota dal lunedì al venerdì.");
        assert_eq!(fuori_regola(&n("2026-11-12T17:45:00"), &n("2026-11-12T18:15:00"), ora).unwrap(), "Si prenota dalle 9:00 alle 18:00.");
        assert_eq!(fuori_regola(&n("2026-11-12T10:10:00"), &n("2026-11-12T10:30:00"), ora).unwrap(), "Gli orari vanno a quarti d'ora (:00, :15, :30, :45).");
        assert_eq!(data_breve(&n("2026-11-11T10:00:00")), "mer 11 nov");
    }

    #[test]
    fn i_campi() {
        let c = ogg! { "inizio": "2026-11-12T10:00", "fine": "2026-11-12T10:30:00", "nome": " Mario\n Rossi ", "email": "m@x.it", "argomento": "a\nb" };
        let r = leggi_richiesta(&c).unwrap();
        assert_eq!((r.nome.as_str(), r.argomento.as_str()), ("Mario  Rossi", "a\nb"));
        for e in ["m@x", "m@x.i", "m x@y.it", "@y.it", "m@@y.it"] {
            assert!(!email_valida(e), "{e}");
        }
        assert!(email_valida("a.b+c@sub.example.org"));
    }
}
