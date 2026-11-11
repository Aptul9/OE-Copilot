//! Il testo iCalendar (ICS) e la sua espansione: la lettura che fa icalendar 7.3.0 (righe, parametri, date, durate, fusi, proprietà rotte),
//! la correzione di X-WR-TIMEZONE di x-wr-timezone 2.0.1, e l'uscita dell'agente Python (eventi con ora locale di Roma senza fuso). I testi
//! degli errori sono quelli di Python: il gateway li mostra così com'escono dall'agente.

use crate::errori::{Errore, Esito};
use crate::fusi::{cache_fusi_get, cache_fusi_ha, cache_fusi_imposta, registra_fuso_personalizzato};
use crate::json::Json;
use crate::ogg;
use crate::pyfmt::{SPAZI_PY, py_repr_str, py_strip};
use crate::regola::trim_js;
use crate::serie::{Componente, Dati, FineODurata, Rdate, espandi};
use crate::tempo::{Ora, Rotta, Tempo, e_utc, errore_rotta, fuso_esistente, fuso_iana, in_fuso, in_ora_locale, istante, ora, testo_naive};
use crate::vtimezone::{RigaTz, crea_zona_da_righe};
use crate::windows::windows;
use indexmap::IndexMap;
use jiff::civil::{Date, DateTime};
use regex::Regex;
use std::rc::Rc;
use std::sync::LazyLock;

pub const FUSO_LOCALE: &str = "Europe/Rome";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValoreParametro {
    Uno(String),
    Molti(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Riga {
    pub nome: String,
    pub param: IndexMap<String, ValoreParametro>,
    /// Il valore con le sequenze di escape tolte (\n, \, \; \: \\).
    pub valore: String,
    /// Il valore com'è nel file: lo leggono le proprietà che iCalendar non conosce (X-WR-CALNAME...).
    pub grezzo: String,
    /// Il fuso del parametro TZID com'era quando la riga è stata letta (icalendar legge i valori subito: un VTIMEZONE più avanti nel file
    /// non conta). `Some(None)`: il nome non si è trovato.
    pub risolto: Option<Option<String>>,
}

struct Comp {
    nome: String,
    righe: Vec<Riga>,
    figli: Vec<Comp>,
}

pub fn parametro<'a>(r: &'a Riga, nome: &str) -> Option<&'a str> {
    match r.param.get(nome) {
        Some(ValoreParametro::Uno(v)) => Some(v),
        _ => None,
    }
}

/// Il TZID di un valore: nessuno, un nome da cercare, o il fuso già trovato quando la riga è stata letta (None: non trovato).
#[derive(Clone, Copy, Debug)]
pub enum Tzid<'a> {
    Nessuno,
    Nome(&'a str),
    Risolto(Option<&'a str>),
}

impl<'a> Tzid<'a> {
    pub fn da(nome: Option<&'a str>) -> Self {
        nome.map_or(Tzid::Nessuno, Tzid::Nome)
    }
}

fn valore(m: impl Into<String>) -> Errore {
    Errore::Valore(m.into())
}

/// La lunghezza di JavaScript, in unità UTF-16.
fn lun16(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.slice(da, a)` per caratteri (uguale a JavaScript finché il testo sta nel piano di base).
fn fetta(s: &str, da: usize, a: usize) -> String {
    s.chars().skip(da).take(a.saturating_sub(da)).collect()
}

fn senza_barre(s: &str) -> &str {
    s.trim_matches('/')
}

// ---- le righe --------------------------------------------------------------------------------------------------------------------
static NOME_TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A[\p{L}\p{N}_.-]+\z").expect("valida"));

/// validate_token: il nome è un solo token fatto di lettere, cifre, "_", "." e "-" (\w di Python, che è Unicode).
fn valida_nome(nome: &str) -> Esito<()> {
    if NOME_TOKEN.is_match(nome) { Ok(()) } else { Err(valore(nome)) }
}

/// q_split: divide su un separatore fuori dalle virgolette.
fn q_split(st: &str, sep: u8, maxsplit: i32) -> Vec<&str> {
    if maxsplit == 0 {
        return vec![st];
    }
    let b = st.as_bytes();
    let mut out = Vec::new();
    let mut cursore = 0;
    let mut dentro = false;
    let mut tagli = 0;
    for i in 0..b.len() {
        let c = b[i];
        if c == b'"' {
            dentro = !dentro;
        }
        if !dentro && c == sep {
            out.push(&st[cursore..i]);
            cursore = i + 1;
            tagli += 1;
        }
        if i + 1 == b.len() || tagli == maxsplit {
            out.push(&st[cursore..]);
            break;
        }
    }
    out
}

/// _strip_ows_around_delimiters: toglie gli spazi intorno a ";" e "=" fuori dalle virgolette.
fn senza_spazi_intorno(st: &str) -> String {
    if st.is_empty() || (!st.contains(' ') && !st.contains('\t')) {
        return st.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut in_sospeso: Vec<char> = Vec::new();
    let mut virgolette = false;
    let mut protetto = false;
    let mut ultimo_delimitatore = false;
    let spazio = |c: char| c == ' ' || c == '\t';
    let scarica = |out: &mut Vec<char>, in_sospeso: &mut Vec<char>, ultimo: bool| {
        if !in_sospeso.is_empty() {
            if !ultimo {
                out.append(in_sospeso);
            }
            in_sospeso.clear();
        }
    };
    for c in st.chars() {
        if protetto {
            scarica(&mut out, &mut in_sospeso, ultimo_delimitatore);
            out.push(c);
            protetto = false;
            ultimo_delimitatore = false;
            continue;
        }
        if c == '\\' && !virgolette {
            scarica(&mut out, &mut in_sospeso, ultimo_delimitatore);
            out.push(c);
            protetto = true;
            ultimo_delimitatore = false;
            continue;
        }
        if c == '"' {
            virgolette = !virgolette;
            scarica(&mut out, &mut in_sospeso, ultimo_delimitatore);
            out.push(c);
            ultimo_delimitatore = false;
            continue;
        }
        if !virgolette && spazio(c) {
            in_sospeso.push(c);
            continue;
        }
        if !virgolette && (c == ';' || c == '=') {
            in_sospeso.clear();
            while out.last().is_some_and(|x| spazio(*x)) {
                out.pop();
            }
            out.push(c);
            ultimo_delimitatore = true;
            continue;
        }
        scarica(&mut out, &mut in_sospeso, ultimo_delimitatore);
        out.push(c);
        ultimo_delimitatore = false;
    }
    if !in_sospeso.is_empty() && !ultimo_delimitatore {
        out.extend(in_sospeso);
    }
    py_strip(&out.into_iter().collect::<String>()).to_string()
}

fn escape_parametro(v: &str) -> String {
    v.replace('%', "%25").replace("\\,", "%2C").replace("\\:", "%3A").replace("\\;", "%3B").replace("\\\\", "%5C")
}

fn unescape_parametro(v: &str) -> String {
    v.replace("%2C", ",").replace("%3A", ":").replace("%3B", ";").replace("%5C", "\\").replace("%25", "%")
}

fn rfc6868(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut c = v.chars().peekable();
    while let Some(x) = c.next() {
        if x == '^' {
            match c.peek() {
                Some('^') => {
                    c.next();
                    out.push('^');
                    continue;
                }
                Some('n') => {
                    c.next();
                    out.push('\n');
                    continue;
                }
                Some('\'') => {
                    c.next();
                    out.push('"');
                    continue;
                }
                _ => {}
            }
        }
        out.push(x);
    }
    out
}

fn valida_valore_parametro(v: &str, tra_virgolette: bool) -> Esito<()> {
    let vietato = |c: char| matches!(c, '\x00'..='\x08' | '\x0a'..='\x1f' | '\x7f' | '"') || (!tra_virgolette && matches!(c, ',' | ':' | ';'));
    if v.chars().any(vietato) { Err(valore(v)) } else { Ok(()) }
}

/// Parameters.from_ical: nome=valore;nome=valore, i valori fra virgolette o separati da virgola.
fn leggi_parametri(st: &str) -> Esito<IndexMap<String, ValoreParametro>> {
    let mut out = IndexMap::new();
    for param in q_split(st, b';', -1) {
        let uno = || -> Esito<(String, ValoreParametro)> {
            let kv = q_split(param, b'=', 1);
            if kv.len() != 2 {
                return Err(valore(format!("not enough values to unpack (expected 2, got {})", kv.len())));
            }
            let (chiave, val) = (kv[0], kv[1]);
            valida_nome(chiave)?;
            let mut valori: Vec<String> = Vec::new();
            for v in q_split(val, b',', -1) {
                if v.starts_with('"') && v.ends_with('"') {
                    let v2 = v.trim_matches('"');
                    valida_valore_parametro(v2, true)?;
                    valori.push(rfc6868(v2));
                } else {
                    valida_valore_parametro(v, false)?;
                    valori.push(rfc6868(v));
                }
            }
            let v = match valori.len() {
                0 => ValoreParametro::Uno(val.to_string()),
                1 => ValoreParametro::Uno(valori.pop().unwrap_or_default()),
                _ => ValoreParametro::Molti(valori),
            };
            Ok((chiave.to_uppercase(), v))
        };
        match uno() {
            Ok((k, v)) => {
                out.insert(k, v);
            }
            Err(Errore::Valore(m)) => return Err(valore(format!("{} is not a valid parameter string: {m}", py_repr_str(param)))),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

fn senza_backslash(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut c = v.chars().peekable();
    while let Some(x) = c.next() {
        if x == '\\' {
            if let Some(y @ ('\\' | ',' | ';' | ':' | 'n' | 'N')) = c.peek().copied() {
                c.next();
                out.push(if y == 'n' || y == 'N' { '\n' } else { y });
                continue;
            }
        }
        out.push(x);
    }
    out
}

/// Contentline.parts(): nome, parametri e valore di una riga. Con una riga che non si legge: ValueError con il testo di Python.
pub fn leggi_riga_ics(riga: &str) -> Esito<Riga> {
    let parti = || -> Esito<Riga> {
        let b = riga.as_bytes();
        let mut taglia_nome: Option<usize> = None;
        let mut taglia_valore: Option<usize> = None;
        let mut virgolette = false;
        let mut protetto = false;
        for (i, c) in b.iter().enumerate() {
            if *c == b'"' && !protetto {
                virgolette = !virgolette;
            } else if *c == b'\\' && !virgolette {
                protetto = true;
                continue;
            } else if !virgolette && !protetto {
                if (*c == b':' || *c == b';') && taglia_nome.is_none() {
                    taglia_nome = Some(i);
                }
                if *c == b':' && taglia_valore.is_none() {
                    taglia_valore = Some(i);
                }
            }
            protetto = false;
        }
        // "not value_split" di Python vale anche per l'indice 0
        let taglia_valore = taglia_valore.filter(|v| *v > 0).unwrap_or(b.len());
        let nome_grezzo = match taglia_nome {
            Some(n) if n > 0 => &riga[..n],
            _ => riga,
        };
        let nome: String = py_strip(nome_grezzo).chars().filter(|c| *c != ' ' && *c != '\t').collect();
        valida_nome(&nome)?;
        let taglia_nome = match taglia_nome {
            Some(n) if n > 0 && n + 1 != taglia_valore => n,
            _ => return Err(valore("Invalid content line")),
        };
        let testo_parametri = escape_parametro(&senza_spazi_intorno(&riga[taglia_nome + 1..taglia_valore.max(taglia_nome + 1)]));
        let mut param = IndexMap::new();
        for (k, v) in leggi_parametri(&testo_parametri)? {
            let v = match v {
                ValoreParametro::Uno(x) => ValoreParametro::Uno(unescape_parametro(&x)),
                ValoreParametro::Molti(x) => ValoreParametro::Molti(x.iter().map(|y| unescape_parametro(y)).collect()),
            };
            param.insert(unescape_parametro(&k).to_uppercase(), v);
        }
        let grezzo = riga.get(taglia_valore + 1..).unwrap_or_default().to_string();
        Ok(Riga { nome: nome.to_uppercase(), param, valore: senza_backslash(&grezzo), grezzo, risolto: None })
    };
    parti().map_err(|e| match e {
        Errore::Valore(m) => valore(format!("Content line could not be parsed into parts: '{riga}': {m}")),
        altro => altro,
    })
}

// ---- i componenti -------------------------------------------------------------------------------------------------------------
/// Le righe piegate rimesse insieme: un a capo seguito da altri a capo e da uno spazio o una tabulazione sparisce, ma non quello che
/// segue un altro a capo (la sostituzione di DOPPIO_ACCAPO in ics.ts, scritta senza lookbehind).
fn srotola(testo: &str) -> String {
    let b = testo.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let prima = if i > 0 { Some(b[i - 1]) } else { None };
        let mut j = if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') && prima != Some(b'\n') {
            Some(i + 2)
        } else if b[i] == b'\n' && prima != Some(b'\r') && prima != Some(b'\n') {
            Some(i + 1)
        } else {
            None
        };
        if let Some(mut k) = j.take() {
            loop {
                if b.get(k) == Some(&b'\n') {
                    k += 1;
                } else if b.get(k) == Some(&b'\r') && b.get(k + 1) == Some(&b'\n') {
                    k += 2;
                } else {
                    break;
                }
            }
            if matches!(b.get(k), Some(b' ' | b'\t')) {
                j = Some(k + 1);
            }
        }
        match j {
            Some(fine) => i = fine,
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    // si tolgono solo byte ASCII interi: il testo resta UTF-8
    String::from_utf8(out).unwrap_or_default()
}

/// _format_error di icalendar: la descrizione e l'inizio del testo, accorciato a 97 caratteri in tutto.
fn formato_errore(descrizione: &str, testo: &str) -> String {
    let massimo = 100 - 3;
    let puntini = "[...]";
    let punti = testo.chars().count();
    if descrizione.len() + punti + puntini.len() > massimo {
        let tieni = massimo.saturating_sub(descrizione.len() + puntini.len());
        return format!("{descrizione}: {} {puntini}", testo.chars().take(tieni).collect::<String>());
    }
    format!("{descrizione}: {testo}")
}

const NOMI_DATA_ORA: [&str; 6] = ["DTSTART", "DTEND", "RECURRENCE-ID", "DUE", "RDATE", "EXDATE"];

/// I componenti di primo livello del testo. In un VEVENT una riga che non si legge si salta (Event ignora gli errori), altrove ferma tutto.
fn leggi_componenti(testo: &str) -> Esito<Vec<Comp>> {
    let srotolato = srotola(testo);
    let mut pila: Vec<Comp> = Vec::new();
    let mut radici: Vec<Comp> = Vec::new();
    for r in srotolato.split('\n') {
        let r = r.strip_suffix('\r').unwrap_or(r);
        if r.is_empty() {
            continue;
        }
        let mut p = match leggi_riga_ics(r) {
            Ok(p) => p,
            Err(Errore::Valore(_)) if pila.last().is_some_and(|c| c.nome == "VEVENT") => continue,
            Err(e) => return Err(e),
        };
        if p.nome == "BEGIN" {
            pila.push(Comp { nome: p.valore.to_uppercase(), righe: Vec::new(), figli: Vec::new() });
        } else if p.nome == "END" {
            let Some(c) = pila.pop() else { return Err(valore("END encountered without an accompanying BEGIN!")) };
            if p.valore.to_uppercase() == "VTIMEZONE" {
                registra_vtimezone(&c)?;
            }
            match pila.last_mut() {
                Some(sopra) => sopra.figli.push(c),
                None => radici.push(c),
            }
        } else {
            let n = pila.len();
            let Some(sopra) = pila.last() else {
                if p.nome == "X-COMMENT" {
                    continue;
                }
                return Err(valore(format!("Property \"{}\" does not have a parent component.", p.nome)));
            };
            // icalendar legge i valori man mano: il fuso di un TZID è quello che si trova adesso, e in un VTIMEZONE un valore sbagliato ferma tutto
            if NOMI_DATA_ORA.contains(&p.nome.as_str()) {
                if let Some(tzid) = parametro(&p, "TZID").filter(|t| !t.is_empty()) {
                    p.risolto = Some(risolvi_fuso(&tzid.to_string()));
                }
            }
            let in_vtimezone = sopra.nome == "VTIMEZONE" || ((sopra.nome == "STANDARD" || sopra.nome == "DAYLIGHT") && n >= 2 && pila[n - 2].nome == "VTIMEZONE");
            if in_vtimezone {
                valida_proprieta_fuso(&p)?;
            }
            if let Some(sopra) = pila.last_mut() {
                sopra.righe.push(p);
            }
        }
    }
    Ok(radici)
}

// ---- valori -----------------------------------------------------------------------------------------------------------------
/// tzp.timezone: il nome come fuso, o None se non si trova (e allora l'orario resta senza fuso). Il primo nome che porta a un fuso resta
/// nella cache.
fn risolvi_fuso(tzid: &str) -> Option<String> {
    let pulito = senza_barre(tzid);
    let mut candidati: Vec<String> = vec![pulito.to_string(), windows(pulito).unwrap_or_default().to_string()];
    if tzid.starts_with('/') {
        let pezzi: Vec<&str> = pulito.split('/').collect();
        for i in 1..pezzi.len() {
            candidati.push(pezzi[i..].join("/"));
        }
    }
    candidati.push(tzid.to_string());
    for c in candidati {
        if c.is_empty() {
            continue;
        }
        let fuso = cache_fusi_get(&c).or_else(|| fuso_esistente(&c).then(|| c.clone()));
        if let Some(f) = fuso {
            cache_fusi_imposta(pulito, &f);
            return Some(f);
        }
    }
    None
}

fn due(n: i64) -> String {
    format!("{n:02}")
}

/// Il fuso di una riga con TZID: quello che icalendar ha trovato quando l'ha letta, o il nome da cercare.
fn tzid_di(r: &Riga) -> Tzid<'_> {
    match &r.risolto {
        Some(f) => Tzid::Risolto(f.as_deref()),
        None => Tzid::da(parametro(r, "TZID")),
    }
}

fn intero_con_trattini(t: &str) -> Option<i64> {
    let t = trim_js(t);
    let corpo = t.strip_prefix(['+', '-']).unwrap_or(t);
    let valido = !corpo.is_empty() && corpo.split('_').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !valido {
        return None;
    }
    let n: i64 = corpo.replace('_', "").parse().unwrap_or(i64::MAX / 2);
    Some(if t.starts_with('-') { -n } else { n })
}

/// vUTCOffset.from_ical, in secondi.
fn leggi_scarto_ical(v: &str) -> Esito<i64> {
    let intero = |t: &str| intero_con_trattini(t).ok_or_else(|| valore(format!("Expected UTC offset, got: {v}")));
    let secondi = fetta(v, 5, 7);
    let scarto = intero(&fetta(v, 1, 3))?.saturating_mul(3600).saturating_add(intero(&fetta(v, 3, 5))?.saturating_mul(60)).saturating_add(if secondi.is_empty() { 0 } else { intero(&secondi)? });
    if scarto >= 86400 {
        return Err(valore(format!("Offset must be less than 24 hours, was {v}")));
    }
    Ok(if v.starts_with('-') { -scarto } else { scarto })
}

/// vUTCOffset.to_ical.
fn testo_scarto(s: i64) -> String {
    let a = s.abs();
    format!("{}{}{}{}", if s < 0 { '-' } else { '+' }, due(a / 3600), due(a % 3600 / 60), if a % 60 != 0 { due(a % 60) } else { String::new() })
}

const INTERI_REGOLA: [&str; 9] = ["COUNT", "INTERVAL", "BYSECOND", "BYMINUTE", "BYHOUR", "BYWEEKNO", "BYMONTHDAY", "BYYEARDAY", "BYSETPOS"];
const FREQUENZE: [&str; 7] = ["SECONDLY", "MINUTELY", "HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"];

fn intero_semplice(x: &str) -> bool {
    let t = trim_js(x);
    let corpo = t.strip_prefix(['+', '-']).unwrap_or(t);
    !corpo.is_empty() && corpo.bytes().all(|b| b.is_ascii_digit())
}

static GIORNO_REGOLA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([+-]?[0-9]{1,2})?(MO|TU|WE|TH|FR|SA|SU)$").expect("valida"));
static MESE_REGOLA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{1,2}L?$").expect("valida"));

/// `/^([+-]?\d{1,2})?(MO|...)$/i`: le lettere ASCII in qualunque caso, nessun'altra.
fn giorno_regola(x: &str) -> bool {
    GIORNO_REGOLA.is_match(&x.to_ascii_uppercase())
}

/// Le coppie chiave=valore di una regola, come le divide `split`: le parti senza un solo "=" si saltano.
fn coppie(testo: &str) -> impl Iterator<Item = (String, &str)> {
    testo.split(';').filter_map(|coppia| {
        let mut kv = coppia.split('=');
        let (k, v) = (kv.next()?, kv.next()?);
        if kv.next().is_some() {
            return None;
        }
        Some((k.to_uppercase(), v))
    })
}

/// Il messaggio con cui vRecur.from_ical rifiuta una regola, per le cause che si conoscono.
fn messaggio_regola(testo: &str) -> String {
    for (k, v) in coppie(testo) {
        if k == "FREQ" && !FREQUENZE.contains(&v.to_uppercase().as_str()) {
            return format!("Expected frequency, got: {v}");
        }
        if INTERI_REGOLA.contains(&k.as_str()) {
            if let Some(x) = v.split(',').find(|e| !intero_semplice(e)) {
                return format!("invalid literal for int() with base 10: {}", py_repr_str(x));
            }
        }
        if k == "BYDAY" || k == "WKST" || k == "BYWEEKDAY" {
            if let Some(x) = v.split(',').find(|e| !giorno_regola(e)) {
                return format!("Expected weekday abbrevation, got: {x}");
            }
        }
    }
    format!("Error in recurrence rule: {testo}")
}

/// Quello che icalendar legge subito in un VTIMEZONE e nelle sue osservanze: un valore che non si legge ferma tutto (nei VEVENT no, lì la
/// riga resta rotta).
fn valida_proprieta_fuso(p: &Riga) -> Esito<()> {
    match p.nome.as_str() {
        "TZOFFSETFROM" | "TZOFFSETTO" => {
            leggi_scarto_ical(&p.valore)?;
        }
        "DTSTART" => {
            leggi_tempo(&p.valore, tzid_di(p))?;
        }
        "RRULE" | "EXRULE" => {
            if regola_rifiutata(&p.valore) {
                return Err(valore(messaggio_regola(&p.valore)));
            }
        }
        "RDATE" => {
            if !p.valore.is_empty() {
                leggi_rdate(&p.valore, tzid_di(p))?;
            }
        }
        "EXDATE" => {
            leggi_elenco_tempi(&p.valore, tzid_di(p))?;
        }
        _ => {}
    }
    Ok(())
}

/// canonsort_keys: i nomi canonici per primi, gli altri in ordine alfabetico; le righe dello stesso nome restano come stanno.
fn ordinate<'a>(righe: &'a [Riga], canonici: &[&str]) -> Vec<&'a Riga> {
    let mut nomi: Vec<&str> = Vec::new();
    for r in righe {
        if !nomi.contains(&r.nome.as_str()) {
            nomi.push(&r.nome);
        }
    }
    let mut altri: Vec<&str> = nomi.iter().copied().filter(|n| !canonici.contains(n)).collect();
    altri.sort_unstable();
    let ordine: Vec<&str> = canonici.iter().copied().filter(|n| nomi.contains(n)).chain(altri).collect();
    ordine.iter().flat_map(|n| righe.iter().filter(move |r| r.nome == *n)).collect()
}

fn testi_parametri(r: &Riga) -> Vec<String> {
    let mut chiavi: Vec<&String> = r.param.keys().collect();
    chiavi.sort_unstable();
    chiavi
        .into_iter()
        .map(|k| {
            let t = match &r.param[k] {
                ValoreParametro::Uno(v) => v.clone(),
                ValoreParametro::Molti(v) => v.join(","),
            };
            if t.contains([',', ';', ':']) { format!("{k}=\"{t}\"") } else { format!("{k}={t}") }
        })
        .collect()
}

/// Una riga come la riscrive icalendar. Nel secondo tentativo un DTSTART che è una data diventa la mezzanotte (create_timezone).
fn riga_tz(r: &Riga, data_come_ora: bool) -> Esito<RigaTz> {
    let param = testi_parametri(r);
    let nome = r.nome.clone();
    Ok(match r.nome.as_str() {
        "TZOFFSETFROM" | "TZOFFSETTO" => RigaTz { nome, param, valore: testo_scarto(leggi_scarto_ical(&r.valore)?) },
        "RRULE" | "EXRULE" => RigaTz { nome, param, valore: regola_per_python(&r.valore) },
        "DTSTART" if data_come_ora && r.valore.len() == 8 && r.valore.bytes().all(|b| b.is_ascii_digit()) => RigaTz { nome, param: Vec::new(), valore: format!("{}T000000", r.valore) },
        _ => RigaTz { nome, param, valore: r.valore.clone() },
    })
}

/// Component.to_ical() di un VTIMEZONE, riga per riga. `secondo_tentativo`: tolte le proprietà X-, le date di DTSTART diventano mezzanotte.
fn righe_tz(c: &Comp, secondo_tentativo: bool, figlio_di_vtimezone: bool, fuori: &mut Vec<RigaTz>) -> Esito<()> {
    fuori.push(RigaTz { nome: "BEGIN".to_string(), param: Vec::new(), valore: c.nome.clone() });
    let canonici: &[&str] = if c.nome == "VTIMEZONE" { &["TZID"] } else { &[] };
    for r in ordinate(&c.righe, canonici) {
        if secondo_tentativo && r.nome.starts_with("X-") {
            continue;
        }
        fuori.push(riga_tz(r, secondo_tentativo && figlio_di_vtimezone)?);
    }
    for f in &c.figli {
        righe_tz(f, secondo_tentativo, c.nome == "VTIMEZONE", fuori)?;
    }
    fuori.push(RigaTz { nome: "END".to_string(), param: Vec::new(), valore: c.nome.clone() });
    Ok(())
}

/// tzp.cache_timezone_component: un VTIMEZONE che definisce un nome che il resto non conosce diventa quel fuso, per sempre (finché
/// l'agente non riparte).
fn registra_vtimezone(c: &Comp) -> Esito<()> {
    let Some(riga) = c.righe.iter().find(|r| r.nome == "TZID") else { return Ok(()) };
    let grezzo = &riga.valore;
    let id = senza_barre(grezzo);
    if fuso_esistente(id) || fuso_esistente(grezzo) || cache_fusi_ha(id) {
        return Ok(());
    }
    let tentativo = |secondo: bool| -> Esito<crate::vtimezone::ZonaVtz> {
        let mut righe = Vec::new();
        righe_tz(c, secondo, false, &mut righe)?;
        crea_zona_da_righe(&righe)
    };
    let zona = match tentativo(false) {
        Ok(z) => z,
        Err(Errore::Valore(_)) => tentativo(true)?,
        Err(e) => return Err(e),
    };
    registra_fuso_personalizzato(id, Rc::new(zona));
    cache_fusi_imposta(id, id);
    Ok(())
}

fn cifre(s: &str, da: usize, a: usize) -> i64 {
    s.get(da..a).and_then(|x| x.parse().ok()).unwrap_or(0)
}

/// vDatetime.from_ical.
fn data_ora(valore_ics: &str, fuso: Option<&str>) -> Esito<Ora> {
    let errore = || valore(format!("Wrong datetime format: {valore_ics}"));
    let v = valore_ics.rfind(':').map_or(valore_ics, |i| &valore_ics[i + 1..]);
    let b = v.as_bytes();
    if b.len() < 15 || b[8] != b'T' || !b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit) {
        return Err(errore());
    }
    let w = DateTime::new(cifre(v, 0, 4) as i16, cifre(v, 4, 6) as i8, cifre(v, 6, 8) as i8, cifre(v, 9, 11) as i8, cifre(v, 11, 13) as i8, cifre(v, 13, 15) as i8, 0).map_err(|_| errore())?;
    if let Some(f) = fuso.filter(|f| !f.is_empty()) {
        return Ok(ora(w, Some(Rc::from(f)), 0));
    }
    match &v[15..] {
        "" => Ok(ora(w, None, 0)),
        "Z" => Ok(ora(w, Some(Rc::from("UTC")), 0)),
        _ => Err(errore()),
    }
}

/// vDate.from_ical.
fn solo_data(v: &str) -> Esito<Date> {
    let errore = || valore(format!("Wrong date format {v}"));
    if v.len() != 8 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(errore());
    }
    Date::new(cifre(v, 0, 4) as i16, cifre(v, 4, 6) as i8, cifre(v, 6, 8) as i8).map_err(|_| errore())
}

static DURATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([-+]?)P(?:([0-9]+)W)?(?:([0-9]+)D)?(?:T(?:([0-9]+)H)?(?:([0-9]+)M)?(?:([0-9]+)S)?)?$").expect("valida"));

/// vDuration.from_ical, in secondi.
pub fn leggi_durata(v: &str) -> Esito<f64> {
    let m = DURATA.captures(v).ok_or_else(|| valore(format!("Invalid iCalendar duration: {v}")))?;
    let n = |i: usize| m.get(i).map_or(0.0, |x| x.as_str().parse::<f64>().unwrap_or(0.0));
    let s = n(2) * 604800.0 + n(3) * 86400.0 + n(4) * 3600.0 + n(5) * 60.0 + n(6);
    Ok(if &m[1] == "-" { -s } else { s })
}

enum Vddd {
    T(Tempo),
    Durata(f64),
    Periodo(Rdate),
}

/// vDDDTypes.from_ical: dal formato si capisce se è una durata, un periodo, una data o un orario; con TZID un orario prende quel fuso (se si trova).
fn leggi_vddd(ical: &str, tzid: Tzid<'_>) -> Esito<Vddd> {
    let u = ical.to_uppercase();
    if u.starts_with('P') || u.starts_with("-P") || u.starts_with("+P") {
        return Ok(Vddd::Durata(leggi_durata(ical)?));
    }
    if u.contains('/') {
        return Ok(Vddd::Periodo(leggi_periodo(ical, tzid)?));
    }
    let fuso: Option<String> = match tzid {
        Tzid::Nessuno => None,
        Tzid::Nome(t) => {
            if t.is_empty() {
                None
            } else {
                risolvi_fuso(t)
            }
        }
        Tzid::Risolto(f) => f.map(str::to_string),
    };
    let lunghezza = lun16(ical);
    if lunghezza == 15 || lunghezza == 16 {
        return Ok(Vddd::T(Tempo::Ora(data_ora(ical, fuso.as_deref())?)));
    }
    if lunghezza == 8 {
        let d = solo_data(ical)?;
        return Ok(Vddd::T(match fuso.filter(|f| !f.is_empty()) {
            Some(f) => Tempo::Ora(ora(d.at(0, 0, 0, 0), Some(Rc::from(f.as_str())), 0)),
            None => Tempo::Data(d),
        }));
    }
    if lunghezza == 6 || lunghezza == 7 {
        let orario = ical.rfind(':').map_or(ical, |i| &ical[i + 1..]);
        let orario = orario.strip_suffix('Z').unwrap_or(orario);
        if lun16(orario) != 6 || !orario.bytes().all(|b| b.is_ascii_digit()) {
            return Err(valore(format!("Expected time, got: {orario}")));
        }
        return Err(Errore::Tipo("un orario senza data non è un inizio".to_string()));
    }
    Err(valore(format!("Expected datetime, date, or time. Got: '{ical}'")))
}

/// vPeriod.from_ical: inizio e fine, o inizio e durata.
fn leggi_periodo(ical: &str, tzid: Tzid<'_>) -> Esito<Rdate> {
    let leggi = || -> Esito<Rdate> {
        let parti: Vec<&str> = ical.split('/').collect();
        if parti.len() != 2 {
            return Err(valore("too many values to unpack"));
        }
        let inizio = leggi_vddd(parti[0], tzid)?;
        let fine = leggi_vddd(parti[1], tzid)?;
        let Vddd::T(inizio) = inizio else { return Err(Errore::Tipo("inizio del periodo".to_string())) };
        let fine_o_durata = match fine {
            Vddd::Durata(d) => Some(FineODurata::Durata(d)),
            Vddd::T(t) => Some(FineODurata::Tempo(t)),
            Vddd::Periodo(_) => None,
        };
        Ok(Rdate { inizio, fine_o_durata })
    };
    leggi().map_err(|_| valore(format!("Expected period format, got: {ical}")))
}

/// Un valore che deve essere una data o un orario (DTSTART, DTEND, RECURRENCE-ID, un elemento di EXDATE).
pub fn leggi_tempo(v: &str, tzid: Tzid<'_>) -> Esito<Tempo> {
    match leggi_vddd(v, tzid)? {
        Vddd::T(t) => Ok(t),
        _ => Err(Errore::Tipo("una durata o un periodo non sono una data".to_string())),
    }
}

/// vDDDLists.from_ical: elementi separati da virgola.
pub fn leggi_elenco_tempi(v: &str, tzid: Tzid<'_>) -> Esito<Vec<Tempo>> {
    v.split(',').map(|x| leggi_tempo(x, tzid)).collect()
}

/// RDATE: date, orari o periodi separati da virgola.
pub fn leggi_rdate(v: &str, tzid: Tzid<'_>) -> Esito<Vec<Rdate>> {
    v.split(',')
        .map(|p| match leggi_vddd(p, tzid)? {
            Vddd::T(t) => Ok(Rdate { inizio: t, fine_o_durata: None }),
            Vddd::Periodo(r) => Ok(r),
            Vddd::Durata(_) => Err(Errore::Tipo("una durata non è una data".to_string())),
        })
        .collect()
}

// ---- dal componente al calcolo -----------------------------------------------------------------------------------------------
/// x-wr-timezone: un orario in UTC passa al fuso del calendario, uno senza fuso lo prende.
fn con_fuso_del_calendario(t: Tempo, z: &str) -> Esito<Tempo> {
    match t {
        Tempo::Ora(o) if e_utc(&o) => Ok(Tempo::Ora(in_fuso(istante(&o)?, z)?)),
        Tempo::Ora(o) if o.z.is_none() => Ok(Tempo::Ora(ora(o.w, Some(Rc::from(z)), o.fold))),
        altro => Ok(altro),
    }
}

/// Il testo di una proprietà di testo (vText) come "escape": ';' ',' '\' e gli accapo, per i messaggi dei calendari con una regola rotta.
fn come_testo(v: &str) -> String {
    v.replace("\\N", "\n").replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n").replace('\r', "\\n")
}

/// vRecur.from_ical rifiuta la regola? (una frequenza che non c'è, un intero che non lo è, un giorno che non è un giorno, un UNTIL che non è una data)
fn regola_rifiutata(testo: &str) -> bool {
    for (k, v) in coppie(testo) {
        if k == "FREQ" && !FREQUENZE.contains(&v.to_uppercase().as_str()) {
            return true;
        }
        if INTERI_REGOLA.contains(&k.as_str()) && v.split(',').any(|x| !intero_semplice(x) || trim_js(x).trim_start_matches(['+', '-']).parse::<u64>().map_or(true, |n| n > 2_147_483_647)) {
            return true;
        }
        if (k == "BYDAY" || k == "WKST" || k == "BYWEEKDAY") && v.split(',').any(|x| !giorno_regola(x)) {
            return true;
        }
        if k == "BYMONTH" && v.split(',').any(|x| !MESE_REGOLA.is_match(x)) {
            return true;
        }
        if k == "UNTIL" && leggi_tempo(v, Tzid::Nessuno).is_err() {
            return true;
        }
    }
    false
}

/// Il testo della regola come lo passa Python a dateutil: quello che iCalendar non legge resta un testo con gli escape, il resto com'è.
const ORDINE_REGOLA: [&str; 17] = ["RSCALE", "FREQ", "UNTIL", "COUNT", "INTERVAL", "BYSECOND", "BYMINUTE", "BYHOUR", "BYDAY", "BYWEEKDAY", "BYMONTHDAY", "BYYEARDAY", "BYWEEKNO", "BYMONTH", "BYSETPOS", "WKST", "SKIP"];

/// vRecur.to_ical(): le parti nell'ordine di RFC 5545 (le altre in fondo, in ordine alfabetico), in maiuscolo, gli interi come numeri.
fn regola_canonica(v: &str) -> String {
    let mut parti: IndexMap<String, String> = IndexMap::new();
    for (k, x) in coppie(v) {
        let valore = if k == "FREQ" || k == "BYDAY" || k == "WKST" || k == "BYWEEKDAY" {
            x.to_uppercase()
        } else if INTERI_REGOLA.contains(&k.as_str()) {
            x.split(',').map(|e| trim_js(e).parse::<i64>().map_or_else(|_| e.to_string(), |n| n.to_string())).collect::<Vec<_>>().join(",")
        } else if k == "BYMONTH" {
            x.split(',')
                .map(|e| {
                    let (cifre, l) = e.strip_suffix('L').map_or((e, ""), |c| (c, "L"));
                    match cifre.parse::<u64>() {
                        Ok(n) if !cifre.is_empty() && cifre.bytes().all(|b| b.is_ascii_digit()) => format!("{n}{l}"),
                        _ => e.to_string(),
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        } else {
            x.to_string()
        };
        parti.insert(k, valore);
    }
    let mut altri: Vec<&String> = parti.keys().filter(|k| !ORDINE_REGOLA.contains(&k.as_str())).collect();
    altri.sort_unstable();
    ORDINE_REGOLA
        .iter()
        .filter_map(|k| parti.get_key_value(*k))
        .chain(altri.into_iter().map(|k| (k, &parti[k])))
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(";")
}

pub fn regola_per_python(v: &str) -> String {
    if regola_rifiutata(v) { come_testo(v) } else { regola_canonica(trim_js(v)) }
}

/// Il testo di una proprietà letta in un VEVENT: il valore, o la proprietà rotta con l'errore che il calcolo troverà quando la userà.
pub fn leggi_proprieta<T>(r: &Riga, atteso: &str, attributo: &str, leggi: impl FnOnce() -> Esito<T>) -> Esito<Result<T, Rotta>> {
    match leggi() {
        Ok(v) => Ok(Ok(v)),
        Err(Errore::Valore(m) | Errore::Tipo(m)) => Ok(Err(Rotta { nome: r.nome.clone(), atteso: atteso.to_string(), attributo: attributo.to_string(), errore: m })),
        Err(e) => Err(e),
    }
}

fn da_vevento(ev: &Comp, zona_calendario: Option<&str>, n: usize) -> Esito<Componente> {
    let prima = |nome: &str| ev.righe.iter().find(|r| r.nome == nome);
    let tutte = |nome: &'static str| ev.righe.iter().filter(move |r| r.nome == nome);
    let zona = zona_calendario.filter(|z| !z.is_empty());
    let nel_fuso = |t: Tempo| -> Esito<Tempo> {
        match zona {
            Some(z) => con_fuso_del_calendario(t, z),
            None => Ok(t),
        }
    };
    // l'ordine in cui recurring_ical_events usa le proprietà è quello in cui un errore esce: RECURRENCE-ID, DTSTART, DTEND o DURATION, EXDATE, RDATE
    let mut problemi: Vec<Errore> = Vec::new();
    let tempo = |r: &Riga| leggi_proprieta(r, "vDDDTypes", "dt", || nel_fuso(leggi_tempo(&r.valore, tzid_di(r))?));

    let r_id = prima("RECURRENCE-ID");
    let id_grezzo = match r_id {
        Some(r) => Some(leggi_proprieta(r, "vDDDTypes", "dt", || leggi_tempo(&r.valore, tzid_di(r)))?),
        None => None,
    };
    if let Some(Err(rotta)) = &id_grezzo {
        problemi.push(errore_rotta(rotta));
    }
    let mut inizio = Tempo::Data(Date::new(2000, 1, 1).map_err(|_| valore("data"))?);
    match prima("DTSTART") {
        None => problemi.push(Errore::Altro("'DTSTART'".to_string())),
        Some(r) => match tempo(r)? {
            Err(rotta) => problemi.push(errore_rotta(&rotta)),
            Ok(i) => inizio = i,
        },
    }
    let mut fine_prop: Option<Tempo> = None;
    let mut durata: Option<f64> = None;
    if let Some(r) = prima("DTEND") {
        match tempo(r)? {
            Err(rotta) => problemi.push(errore_rotta(&rotta)),
            Ok(f) => fine_prop = Some(f),
        }
    } else if let Some(r) = prima("DURATION") {
        let d = leggi_proprieta(r, "vDDDTypes", "dt", || match leggi_vddd(&r.valore, Tzid::da(parametro(r, "TZID")))? {
            Vddd::Durata(d) => Ok(d),
            _ => Err(Errore::Tipo("una data non è una durata".to_string())),
        })?;
        match d {
            Err(rotta) => problemi.push(errore_rotta(&rotta)),
            Ok(d) => durata = Some(d),
        }
    }
    let mut exdate: Vec<Tempo> = Vec::new();
    for r in tutte("EXDATE") {
        let e = leggi_proprieta(r, "vDDDLists", "dts", || leggi_elenco_tempi(&r.valore, tzid_di(r))?.into_iter().map(nel_fuso).collect::<Esito<Vec<_>>>())?;
        match e {
            Err(rotta) => problemi.push(errore_rotta(&rotta)),
            Ok(e) => exdate.extend(e),
        }
    }
    let mut rdate: Vec<Rdate> = Vec::new();
    for r in tutte("RDATE") {
        let e = leggi_proprieta(r, "vDDDLists", "dts", || {
            leggi_rdate(&r.valore, tzid_di(r))?
                .into_iter()
                .map(|d| if d.fine_o_durata.is_none() { Ok(Rdate { inizio: nel_fuso(d.inizio)?, fine_o_durata: None }) } else { Ok(d) })
                .collect::<Esito<Vec<_>>>()
        })?;
        match e {
            Err(rotta) => problemi.push(errore_rotta(&rotta)),
            Ok(e) => rdate.extend(e),
        }
    }
    let sequenza = prima("SEQUENCE").and_then(|s| {
        let t = trim_js(&s.valore);
        let corpo = t.strip_prefix(['+', '-']).unwrap_or(t);
        (!corpo.is_empty() && corpo.bytes().all(|b| b.is_ascii_digit())).then(|| {
            let n: i64 = corpo.parse().unwrap_or(i64::MAX);
            if t.starts_with('-') { -n } else { n }
        })
    });
    let testo = |nome: &str| prima(nome).map(|r| r.valore.clone()).unwrap_or_default();
    let id_valido = match &id_grezzo {
        Some(Ok(t)) => Some(t.clone()),
        _ => None,
    };
    // x-wr-timezone ricrea il valore che cambia (UTC o senza fuso) e con lui perde i parametri: RANGE=THISANDFUTURE non vale più
    let ricreato = zona.is_some() && matches!(&id_valido, Some(Tempo::Ora(o)) if o.z.is_none() || e_utc(o));
    let rec_range = r_id.is_some_and(|r| r.param.get("RANGE") == Some(&ValoreParametro::Uno("THISANDFUTURE".to_string()))) && !ricreato;
    let mut rrule: Vec<String> = Vec::new();
    for r in tutte("RRULE") {
        let t = regola_per_python(&r.valore);
        if !rrule.contains(&t) {
            rrule.push(t);
        }
    }
    let uid = prima("UID").map(|r| r.valore.clone());
    Ok(Componente {
        uid: uid.clone().unwrap_or_else(|| format!("\u{0}senza-uid-{n}")),
        inizio,
        fine_prop,
        durata,
        rec_id: match id_valido {
            Some(t) => Some(nel_fuso(t)?),
            None => None,
        },
        rec_range,
        sequenza: sequenza.unwrap_or(-1),
        exdate,
        rdate,
        rrule,
        ha_regole: tutte("RRULE").count() + tutte("RDATE").count() + tutte("EXDATE").count() > 0,
        problemi,
        dati: Dati { uid: uid.unwrap_or_default(), titolo: testo("SUMMARY"), luogo: testo("LOCATION"), descrizione: testo("DESCRIPTION"), organizzatore: testo("ORGANIZER") },
    })
}

fn eventi<'a>(c: &'a Comp, fuori: &mut Vec<&'a Comp>) {
    if c.nome == "VEVENT" {
        fuori.push(c);
    }
    for f in &c.figli {
        eventi(f, fuori);
    }
}

/// Calendar.from_ical: un solo componente, il primo livello del testo.
fn leggi_calendario(testo: &str) -> Esito<Comp> {
    let mut radici = leggi_componenti(testo)?;
    if radici.len() > 1 {
        return Err(valore(formato_errore("Found multiple components where only one is allowed", testo)));
    }
    radici.pop().ok_or_else(|| valore(formato_errore("Found no components where exactly one is required", testo)))
}

/// Una proprietà del calendario: quelle che iCalendar non conosce (X-WR-CALNAME...) restano com'erano nel file.
fn proprieta<'a>(cal: &'a Comp, nome: &str) -> Option<&'a str> {
    let r = cal.righe.iter().find(|x| x.nome == nome)?;
    Some(if nome.starts_with("X-") { &r.grezzo } else { &r.valore })
}

/// Il nome che il calendario dichiara (X-WR-CALNAME, o ""), o un errore di valore se il testo non è un calendario.
pub fn valida_ics(testo: &str) -> Esito<String> {
    let cal = leggi_calendario(testo)?;
    Ok(proprieta(&cal, "X-WR-CALNAME").unwrap_or_default().to_string())
}

// ---- la finestra e l'uscita ----------------------------------------------------------------------------------------------------
// \w e \s di Python (re, testo Unicode) non sono quelli di JavaScript: lettere e cifre di ogni alfabeto, e gli spazi di str.isspace()
pub static MEETING_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i)https?://(?:[\p{{L}}\p{{N}}_.-]*teams\.microsoft\.com|[\p{{L}}\p{{N}}_.-]*zoom\.us|meet\.google\.com|[\p{{L}}\p{{N}}_.-]*webex\.com|meet\.jit\.si|[\p{{L}}\p{{N}}_.-]*gotomeeting\.com)/[^{SPAZI_PY}<>"']+"#
    ))
    .expect("valida")
});

/// Un estremo dell'intervallo (ora locale senza fuso) come lo dà astimezone(): un istante con lo scarto che Roma ha in quel momento.
fn estremo_con_scarto(testo_locale: &str, fuso: &str) -> Esito<Ora> {
    let w = leggi_iso_locale(testo_locale)?;
    let tz = fuso_iana(fuso)?;
    let z = tz.to_ambiguous_zoned(w).compatible().map_err(|_| Errore::Altro("RangeError: date value out of range".to_string()))?;
    let s = z.offset().seconds();
    let a = s.abs();
    let mut scarto = format!("{}{:02}:{:02}", if s < 0 { '-' } else { '+' }, a / 3600, a % 3600 / 60);
    if a % 60 != 0 {
        scarto.push_str(&format!(":{:02}", a % 60));
    }
    Ok(ora(w, Some(Rc::from(scarto.as_str())), 0))
}

static ISO_LOCALE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[T ]([0-9]{2})(?::([0-9]{2})(?::([0-9]{2})(?:\.([0-9]{1,6}))?)?)?)?$").expect("valida"));

/// datetime.fromisoformat di un estremo senza scarto: AAAA-MM-GGTHH:MM:SS (con i microsecondi, se ci sono) o solo la data.
pub fn leggi_iso_locale(testo: &str) -> Esito<DateTime> {
    let errore = || valore(format!("Invalid isoformat string: {}", py_repr_str(testo)));
    let m = ISO_LOCALE.captures(trim_js(testo)).ok_or_else(errore)?;
    let n = |i: usize| m.get(i).map_or(0, |x| x.as_str().parse::<i64>().unwrap_or(0));
    let micro: i64 = m.get(7).map_or(0, |x| format!("{:0<6}", x.as_str()).parse().unwrap_or(0));
    DateTime::new(n(1) as i16, n(2) as i8, n(3) as i8, n(4) as i8, n(5) as i8, n(6) as i8, (micro * 1000) as i32).map_err(|_| errore())
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventoIcs {
    pub uid: String,
    pub inizio: String,
    pub fine: String,
    pub tutto_giorno: bool,
    pub titolo: String,
    pub luogo: String,
    pub organizzatore: String,
    pub link: Option<String>,
}

impl EventoIcs {
    pub fn in_json(&self) -> Json {
        ogg! {
            "uid": self.uid.as_str(),
            "inizio": self.inizio.as_str(),
            "fine": self.fine.as_str(),
            "tutto_giorno": self.tutto_giorno,
            "titolo": self.titolo.as_str(),
            "luogo": self.luogo.as_str(),
            "organizzatore": self.organizzatore.as_str(),
            "link": self.link.clone(),
        }
    }
}

/// Gli eventi del calendario nell'intervallo (ora locale senza fuso, come nell'agente Python), con le ricorrenze espanse.
pub fn espandi_ics(testo: &str, inizio: &str, fine: &str, fuso: Option<&str>) -> Esito<Vec<EventoIcs>> {
    let fuso = fuso.unwrap_or(FUSO_LOCALE);
    let cal = leggi_calendario(testo)?;
    let finestra_da = estremo_con_scarto(inizio, fuso)?;
    let finestra_a = estremo_con_scarto(fine, fuso)?;
    let nome_fuso = proprieta(&cal, "X-WR-TIMEZONE");
    if let Some(n) = nome_fuso {
        if !fuso_esistente(n) {
            return Err(Errore::Altro(py_repr_str(&format!("No time zone found with key {n}"))));
        }
    }
    if proprieta(&cal, "CALSCALE").unwrap_or("GREGORIAN") != "GREGORIAN" {
        return Err(valore("Only Gregorian calendars are supported."));
    }
    let mut vevent: Vec<&Comp> = Vec::new();
    eventi(&cal, &mut vevent);
    let componenti = vevent.iter().enumerate().map(|(n, ev)| da_vevento(ev, nome_fuso, n)).collect::<Esito<Vec<_>>>()?;
    let mut out = Vec::new();
    for o in espandi(componenti, &Tempo::Ora(finestra_da), &Tempo::Ora(finestra_a))? {
        let ini = in_ora_locale(&o.inizio, fuso, false)?;
        let fin = in_ora_locale(&o.fine, fuso, false)?;
        let d = &o.c.dati;
        let link = MEETING_URL.find(&d.luogo).or_else(|| MEETING_URL.find(&d.descrizione));
        out.push(EventoIcs {
            uid: d.uid.clone(),
            inizio: testo_naive(&ini),
            fine: testo_naive(&fin),
            tutto_giorno: matches!(o.inizio, Tempo::Data(_)),
            titolo: d.titolo.clone(),
            luogo: d.luogo.clone(),
            organizzatore: d.organizzatore.replace("mailto:", ""),
            link: link.map(|l| l.as_str().trim_end_matches(['>', ')', '.', ',', ';', '"', '\'']).to_string()),
        });
    }
    Ok(out)
}
