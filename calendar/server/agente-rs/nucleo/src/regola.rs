//! L'espansione di una regola RRULE come la fa dateutil.rrule (la libreria sotto recurring_ical_events): stesse regole di default, stesso
//! ordine dei risultati, stesso conteggio di COUNT. Lavora sull'ora sul muro (anno, mese, giorno, ora...): il fuso lo applica chi la chiama.

use crate::errori::{Errore, Esito};
use crate::tempo::{civile_da_giorni, giorni_da_civile};
use indexmap::IndexMap;
use regex::Regex;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Naive {
    pub a: i64,
    pub m: i64,
    pub g: i64,
    pub h: i64,
    pub mi: i64,
    pub s: i64,
}

pub const ANNUALE: u8 = 0;
pub const MENSILE: u8 = 1;
pub const SETTIMANALE: u8 = 2;
pub const GIORNALIERA: u8 = 3;
pub const ORARIA: u8 = 4;
pub const MINUTALE: u8 = 5;
pub const SECONDALE: u8 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Orario {
    pub byhour: Option<Vec<i64>>,
    pub byminute: Option<Vec<i64>>,
    pub bysecond: Option<Vec<i64>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Regola {
    pub freq: u8,
    pub intervallo: i64,
    pub wkst: i64,
    pub conta: Option<i64>,
    pub until: Option<String>,
    pub bymonth: Option<Vec<i64>>,
    pub byyearday: Option<Vec<i64>>,
    pub bymonthday: Vec<i64>,
    pub bynmonthday: Vec<i64>,
    pub byweekday: Option<Vec<i64>>,
    pub bynweekday: Option<Vec<(i64, i64)>>,
    pub bysetpos: Option<Vec<i64>>,
    pub byweekno: Option<Vec<i64>>,
    /// BYEASTER (solo di dateutil): i giorni prima o dopo la Pasqua.
    pub byeaster: Option<Vec<i64>>,
    pub tempi: Vec<(i64, i64, i64)>,
    /// Le ore, i minuti e i secondi di una regola oraria, minutale o secondale (None: nessun vincolo).
    pub orario: Option<Orario>,
}

fn frequenza(f: &str) -> Option<u8> {
    Some(match f {
        "YEARLY" => ANNUALE,
        "MONTHLY" => MENSILE,
        "WEEKLY" => SETTIMANALE,
        "DAILY" => GIORNALIERA,
        "HOURLY" => ORARIA,
        "MINUTELY" => MINUTALE,
        "SECONDLY" => SECONDALE,
        _ => return None,
    })
}

fn giorno_settimana(g: &str) -> Option<i64> {
    Some(match g {
        "MO" => 0,
        "TU" => 1,
        "WE" => 2,
        "TH" => 3,
        "FR" => 4,
        "SA" => 5,
        "SU" => 6,
        _ => return None,
    })
}

const ELENCHI: [&str; 9] = ["BYSETPOS", "BYMONTH", "BYMONTHDAY", "BYYEARDAY", "BYWEEKNO", "BYEASTER", "BYHOUR", "BYMINUTE", "BYSECOND"];

/// Gli spazi di `String.prototype.trim`.
pub fn spazio_js(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

pub fn trim_js(s: &str) -> &str {
    s.trim_matches(spazio_js)
}

static INTERO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?[0-9]+$").expect("valida"));
static UNTIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{8}(T[0-9]{6}Z?)?$").expect("valida"));
static BYDAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([+-]?[0-9]{0,2})(SU|MO|TU|WE|TH|FR|SA)$").expect("valida"));

/// `Number(testo)` di un intero con segno già controllato; oltre i 64 bit resta al limite.
fn numero_intero(t: &str) -> i64 {
    let t = trim_js(t);
    let (neg, cifre) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut n: i64 = 0;
    for b in cifre.bytes() {
        n = n.saturating_mul(10).saturating_add(i64::from(b - b'0'));
    }
    if neg { -n } else { n }
}

fn intero(t: &str) -> Esito<i64> {
    if !INTERO.is_match(trim_js(t)) {
        return Err(Errore::Valore(format!("invalid literal for int(): '{t}'")));
    }
    Ok(numero_intero(t))
}

fn ordinati(v: &[i64]) -> Vec<i64> {
    let mut out = senza_doppi(v);
    out.sort_unstable();
    out
}

fn senza_doppi(v: &[i64]) -> Vec<i64> {
    let mut visti = HashSet::new();
    v.iter().copied().filter(|x| visti.insert(*x)).collect()
}

/// Il testo di una RRULE come lo legge rrulestr (nomi e valori in maiuscolo) e lo valida icalendar: una regola non valida è un errore.
pub fn leggi_regola(testo: &str, inizio: &Naive) -> Esito<Regola> {
    let mut r: IndexMap<String, String> = IndexMap::new();
    for coppia in testo.split(';') {
        let pezzi: Vec<&str> = coppia.split('=').collect();
        if pezzi.len() != 2 {
            continue;
        }
        r.insert(trim_js(pezzi[0]).to_uppercase(), trim_js(pezzi[1]).to_uppercase());
    }
    for k in r.keys() {
        if !(k == "FREQ" || k == "UNTIL" || k == "WKST" || k == "BYDAY" || k == "COUNT" || k == "INTERVAL" || ELENCHI.contains(&k.as_str())) {
            return Err(Errore::Valore(format!("unknown parameter '{k}'")));
        }
    }
    let Some(f) = r.get("FREQ") else { return Err(Errore::Tipo("rrule.__init__() missing 1 required positional argument: 'freq'".to_string())) };
    let Some(freq) = frequenza(f) else { return Err(Errore::Valore(format!("invalid 'FREQ': {f}"))) };
    let elenco = |k: &str| -> Esito<Option<Vec<i64>>> {
        match r.get(k) {
            Some(v) => v.split(',').map(intero).collect::<Esito<Vec<_>>>().map(Some),
            None => Ok(None),
        }
    };
    let wk = r.get("WKST");
    if let Some(w) = wk {
        if giorno_settimana(w).is_none() {
            return Err(Errore::Valore(format!("invalid 'WKST': {w}")));
        }
    }
    let mut conta = match r.get("COUNT") {
        Some(c) => Some(intero(c)?),
        None => None,
    };
    if conta.is_some_and(|c| c < 0) {
        conta = None;
    }
    let intervallo = match r.get("INTERVAL") {
        Some(i) => intero(i)?,
        None => 1,
    };
    let until = r.get("UNTIL").cloned();
    if let Some(u) = &until {
        if !UNTIL.is_match(u) {
            return Err(Errore::Valore(format!("invalid until date: {u}")));
        }
    }

    let mut bymonth = elenco("BYMONTH")?;
    let byyearday = elenco("BYYEARDAY")?;
    let mut bymonthday = elenco("BYMONTHDAY")?;
    let bysetpos = elenco("BYSETPOS")?;
    if bysetpos.as_ref().is_some_and(|v| v.iter().any(|p| *p == 0 || *p < -366 || *p > 366)) {
        return Err(Errore::Valore("bysetpos must be between 1 and 366, or between -366 and -1".to_string()));
    }
    let mut giorni: Option<Vec<(Option<i64>, i64)>> = None;
    if let Some(byday) = r.get("BYDAY") {
        let mut v = Vec::new();
        for g in byday.split(',') {
            let Some(m) = BYDAY.captures(g) else { return Err(Errore::Valore(format!("Expected weekday abbrevation, got: {g}"))) };
            let n = &m[1];
            // "+" e "-" da soli sono NaN per Number: come lo 0, nessun ordinale
            let ordinale = if n.is_empty() || n == "+" || n == "-" { None } else { Some(numero_intero(n)).filter(|x| *x != 0) };
            v.push((ordinale, giorno_settimana(&m[2]).unwrap_or_default()));
        }
        giorni = Some(v);
    }
    let wkst = wk.and_then(|w| giorno_settimana(w)).unwrap_or(0);
    let dtstart_settimana = settimana(ordinale(inizio.a, inizio.m, inizio.g));

    // Senza nessun BY di giorno o settimana la regola si aggancia a DTSTART
    let mut byweekday: Option<Vec<i64>> = None;
    if !r.contains_key("BYWEEKNO") && !r.contains_key("BYEASTER") && byyearday.is_none() && bymonthday.is_none() && giorni.is_none() {
        if freq == ANNUALE {
            bymonth = Some(bymonth.unwrap_or_else(|| vec![inizio.m]));
            bymonthday = Some(vec![inizio.g]);
        } else if freq == MENSILE {
            bymonthday = Some(vec![inizio.g]);
        } else if freq == SETTIMANALE {
            byweekday = Some(vec![dtstart_settimana]);
        }
    }

    let mut bynweekday: Option<Vec<(i64, i64)>> = None;
    if let Some(giorni) = &giorni {
        let mut semplici: Vec<i64> = Vec::new();
        let mut con_ordinale: Vec<(i64, i64)> = Vec::new();
        for (n, g) in giorni {
            match n {
                Some(n) if freq <= MENSILE => {
                    if !con_ordinale.contains(&(*g, *n)) {
                        con_ordinale.push((*g, *n));
                    }
                }
                _ => {
                    if !semplici.contains(g) {
                        semplici.push(*g);
                    }
                }
            }
        }
        semplici.sort_unstable();
        con_ordinale.sort_unstable();
        byweekday = if semplici.is_empty() { None } else { Some(semplici) };
        bynweekday = if con_ordinale.is_empty() { None } else { Some(con_ordinale) };
    }
    let mensili = senza_doppi(bymonthday.as_deref().unwrap_or_default());
    let valori = |k: &str, da: i64, max: i64| -> Esito<Vec<i64>> {
        let v = elenco(k)?;
        if v.as_ref().is_some_and(|v| v.iter().any(|x| *x < 0 || *x > max)) {
            return Err(Errore::Valore(format!("{} must be in 0..{max}", k[2..].to_lowercase())));
        }
        Ok(v.map_or_else(|| vec![da], |v| ordinati(&v)))
    };
    let mut tempi: Vec<(i64, i64, i64)> = Vec::new();
    let mut orario: Option<Orario> = None;
    if freq < ORARIA {
        let ore = valori("BYHOUR", inizio.h, 23)?;
        let minuti = valori("BYMINUTE", inizio.mi, 59)?;
        let secondi = valori("BYSECOND", inizio.s, 59)?;
        for h in &ore {
            for mi in &minuti {
                for s in &secondi {
                    tempi.push((*h, *mi, *s));
                }
            }
        }
    } else {
        // __construct_byset: il valore deve poter essere raggiunto con l'intervallo, se no la regola sarebbe vuota
        let insieme = |v: &[i64], da: i64, base: i64| -> Esito<Vec<i64>> {
            let mcd = massimo_comune_divisore(intervallo, base);
            let mut ok: Vec<i64> = senza_doppi(v).into_iter().filter(|x| mcd == 1 || (x - da).rem_euclid(mcd) == 0).collect();
            if ok.is_empty() {
                return Err(Errore::Valore("Invalid rrule byxxx generates an empty set.".to_string()));
            }
            ok.sort_unstable();
            Ok(ok)
        };
        let h = elenco("BYHOUR")?;
        let mi = elenco("BYMINUTE")?;
        let s = elenco("BYSECOND")?;
        orario = Some(Orario {
            byhour: match h {
                Some(h) => Some(if freq == ORARIA { insieme(&h, inizio.h, 24)? } else { ordinati(&h) }),
                None => None,
            },
            byminute: match mi {
                Some(mi) => Some(if freq == MINUTALE { insieme(&mi, inizio.mi, 60)? } else { ordinati(&mi) }),
                None if freq < MINUTALE => Some(vec![inizio.mi]),
                None => None,
            },
            bysecond: match s {
                Some(s) => Some(if freq == SECONDALE { insieme(&s, inizio.s, 60)? } else { ordinati(&s) }),
                None if freq < SECONDALE => Some(vec![inizio.s]),
                None => None,
            },
        });
    }
    let settimane = elenco("BYWEEKNO")?;
    let pasque = elenco("BYEASTER")?;
    let mut bymonthday_pos: Vec<i64> = mensili.iter().copied().filter(|x| *x > 0).collect();
    bymonthday_pos.sort_unstable();
    let mut bynmonthday: Vec<i64> = mensili.iter().copied().filter(|x| *x < 0).collect();
    bynmonthday.sort_unstable();
    Ok(Regola {
        freq,
        intervallo,
        wkst,
        conta,
        until,
        bymonth: bymonth.map(|v| ordinati(&v)),
        byyearday: byyearday.map(|v| ordinati(&v)),
        bymonthday: bymonthday_pos,
        bynmonthday,
        byweekday,
        bynweekday,
        bysetpos,
        byweekno: settimane.map(|v| ordinati(&v)),
        byeaster: pasque.map(|mut v| {
            v.sort_unstable();
            v
        }),
        tempi,
        orario,
    })
}

/// `Date.UTC(a, m - 1, g) / 86400000`: mese e giorno fuori dai limiti scorrono, gli anni da 0 a 99 sono del Novecento.
pub fn ordinale(a: i64, m: i64, g: i64) -> i64 {
    let a = if (0..=99).contains(&a) { a + 1900 } else { a };
    let a = a + (m - 1).div_euclid(12);
    let m = (m - 1).rem_euclid(12) + 1;
    giorni_da_civile(a, m, 1) + g - 1
}

fn dall_ordinale(n: i64) -> (i64, i64, i64) {
    civile_da_giorni(n)
}

/// Lunedì = 0 (il 1/1/1970 era un giovedì).
fn settimana(n: i64) -> i64 {
    (n + 3).rem_euclid(7)
}

fn bisestile(a: i64) -> bool {
    (a % 4 == 0 && a % 100 != 0) || a % 400 == 0
}

fn giorni_del_mese(a: i64, m: i64) -> i64 {
    [31, if bisestile(a) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(m - 1).clamp(0, 11) as usize]
}

fn massimo_comune_divisore(a: i64, b: i64) -> i64 {
    let (mut x, mut y) = (a.saturating_abs(), b.saturating_abs());
    while y != 0 {
        (x, y) = (y, x % y);
    }
    x
}

/// Divisione con arrotondamento in basso; per zero dà zero invece del NaN di JavaScript (un INTERVAL=0 non ha senso).
fn div_giu(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a.div_euclid(b) - if b < 0 && a.rem_euclid(b) != 0 { 1 } else { 0 } }
}

fn segna(maschera: &mut [u8], i: i64) {
    if let Some(x) = usize::try_from(i).ok().and_then(|i| maschera.get_mut(i)) {
        *x = 1;
    }
}

fn segnato(maschera: &[u8], i: i64) -> bool {
    usize::try_from(i).ok().and_then(|i| maschera.get(i)).is_some_and(|x| *x != 0)
}

/// La maschera delle settimane di BYWEEKNO per un anno (_iterinfo.rebuild di dateutil): per ogni giorno dell'anno, e dei sette dopo, se cade in
/// una delle settimane chieste. La settimana 1 è la prima che ha almeno quattro giorni nell'anno, e inizia con il giorno WKST.
fn maschere_settimane(anno: i64, wkst: i64, byweekno: &[i64]) -> Vec<u8> {
    let lunghezza = 365 + i64::from(bisestile(anno));
    let giorno_del_lunedi = settimana(ordinale(anno, 1, 1));
    let giorno_settimana = |i: i64| (giorno_del_lunedi + i).rem_euclid(7);
    let mut maschera = vec![0u8; (lunghezza + 14) as usize];
    let mut no1 = (7 - giorno_del_lunedi + wkst) % 7;
    let primo = no1;
    let lunghezza_settimane = if no1 >= 4 {
        no1 = 0;
        lunghezza + (giorno_del_lunedi - wkst).rem_euclid(7)
    } else {
        lunghezza - no1
    };
    let numero = lunghezza_settimane / 7 + (lunghezza_settimane % 7) / 4;
    for n in byweekno {
        let mut n = *n;
        if n < 0 {
            n += numero + 1;
        }
        if !(n > 0 && n <= numero) {
            continue;
        }
        let mut i = if n > 1 {
            let mut i = no1 + (n - 1) * 7;
            if no1 != primo {
                i -= 7 - primo;
            }
            i
        } else {
            no1
        };
        for _ in 0..7 {
            segna(&mut maschera, i);
            i += 1;
            if giorno_settimana(i) == wkst {
                break;
            }
        }
    }
    if byweekno.contains(&1) {
        // anche la settimana 1 dell'anno dopo
        let mut i = no1 + numero * 7;
        if no1 != primo {
            i -= 7 - primo;
        }
        if i < lunghezza {
            for _ in 0..7 {
                segna(&mut maschera, i);
                i += 1;
                if giorno_settimana(i) == wkst {
                    break;
                }
            }
        }
    }
    if no1 != 0 {
        // e l'ultima settimana dell'anno prima, se è fra quelle chieste
        let ultime_settimane = if !byweekno.contains(&-1) {
            let giorno_prima = settimana(ordinale(anno - 1, 1, 1));
            let no1_prima = (7 - giorno_prima + wkst) % 7;
            let lunghezza_prima = 365 + i64::from(bisestile(anno - 1));
            if no1_prima >= 4 {
                52 + ((lunghezza_prima + (giorno_prima - wkst).rem_euclid(7)) % 7) / 4
            } else {
                52 + ((lunghezza - no1) % 7) / 4
            }
        } else {
            -1
        };
        if byweekno.contains(&ultime_settimane) {
            for i in 0..no1 {
                segna(&mut maschera, i);
            }
        }
    }
    maschera
}

/// easter(anno) di dateutil (la Pasqua occidentale): il giorno come numero di giorni dall'epoca.
fn pasqua(a: i64) -> i64 {
    let g = a % 19;
    let c = a.div_euclid(100);
    let h = (c - c.div_euclid(4) - (8 * c + 13).div_euclid(25) + 19 * g + 15).rem_euclid(30);
    let i = h - (h / 28) * (1 - (h / 28) * (29 / (h + 1)) * ((21 - g) / 11));
    let j = (a + a.div_euclid(4) + i + 2 - c + c.div_euclid(4)).rem_euclid(7);
    let p = i - j;
    ordinale(a, 3 + (p + 26).div_euclid(30), 1 + (p + 27 + (p + 6).div_euclid(40)).rem_euclid(31))
}

/// eastermask: i giorni dell'anno scelti da BYEASTER. Un indice fuori dalla lista dà IndexError, uno negativo conta dalla fine, come in Python.
fn maschere_di_pasqua(anno: i64, scarti: &[i64]) -> Esito<Vec<u8>> {
    let lunghezza = 365 + i64::from(bisestile(anno)) + 7;
    let mut maschera = vec![0u8; lunghezza as usize];
    let giorno = pasqua(anno) - ordinale(anno, 1, 1);
    for s in scarti {
        let mut i = giorno.saturating_add(*s);
        if i >= lunghezza || i < -lunghezza {
            return Err(Errore::Altro("list assignment index out of range".to_string()));
        }
        if i < 0 {
            i += lunghezza;
        }
        segna(&mut maschera, i);
    }
    Ok(maschera)
}

/// `Date.UTC(...) / 1000` di un'ora sul muro.
pub fn chiave_muro(n: &Naive) -> i64 {
    ordinale(n.a, n.m, n.g) * 86400 + n.h * 3600 + n.mi * 60 + n.s
}

/// I giorni (come numero di giorni dall'epoca) in cui cade l'ennesimo giorno della settimana di BYDAY, nel mese o nell'anno del periodo.
fn con_ordinale(r: &Regola, anno: i64, mese: i64) -> HashSet<i64> {
    let mut intervalli: Vec<(i64, i64)> = Vec::new();
    if r.freq == ANNUALE {
        match &r.bymonth {
            Some(mesi) => {
                for m in mesi.iter().filter(|m| (1..=12).contains(*m)) {
                    intervalli.push((ordinale(anno, *m, 1), ordinale(anno, *m, giorni_del_mese(anno, *m))));
                }
            }
            None => intervalli.push((ordinale(anno, 1, 1), ordinale(anno, 12, 31))),
        }
    } else {
        intervalli.push((ordinale(anno, mese, 1), ordinale(anno, mese, giorni_del_mese(anno, mese))));
    }
    let mut segnati = HashSet::new();
    for (primo, ultimo) in intervalli {
        for (g, n) in r.bynweekday.as_deref().unwrap_or_default() {
            let i = if *n < 0 {
                let base = ultimo + (n + 1) * 7;
                base - (settimana(base) - g).rem_euclid(7)
            } else {
                let base = primo + (n - 1) * 7;
                base + (7 - settimana(base) + g).rem_euclid(7)
            };
            if i >= primo && i <= ultimo {
                segnati.insert(i);
            }
        }
    }
    segnati
}

const LIMITE_GIRI: u64 = 3_000_000;
const LIMITE_GIRI_ORARI: u64 = 200_000_000;

/// Dice se un risultato supera UNTIL (il confronto dipende dal fuso, lo fa chi chiama).
pub type DopoUntil<'a> = Box<dyn Fn(&Naive) -> Esito<bool> + 'a>;

/// BYSETPOS: le posizioni scelte fra i giorni validi per gli orari, senza doppioni, in ordine.
fn per_posizione(bysetpos: &[i64], giorni: &[i64], tempi: &[(i64, i64, i64)], coda: &mut VecDeque<Naive>) {
    let t_n = tempi.len() as i64;
    let mut visti = HashSet::new();
    let mut risultati = Vec::new();
    for pos in bysetpos {
        let pos = *pos;
        let q = if pos < 0 { pos.div_euclid(t_n) } else { (pos - 1).div_euclid(t_n) };
        let t = (if pos < 0 { pos } else { pos - 1 }) - q * t_n;
        let k = if q < 0 { giorni.len() as i64 + q } else { q };
        if k < 0 || k >= giorni.len() as i64 {
            continue;
        }
        let (a, m, g) = dall_ordinale(giorni[k as usize]);
        let (h, mi, s) = tempi[t as usize];
        let res = Naive { a, m, g, h, mi, s };
        if visti.insert(chiave_muro(&res)) {
            risultati.push(res);
        }
    }
    risultati.sort_by_key(chiave_muro);
    coda.extend(risultati);
}

/// I risultati della regola in ordine, a partire da DTSTART (compreso se rientra nella regola), un periodo alla volta come il generatore di
/// dateutil: al primo risultato che supera UNTIL la regola finisce, dopo un errore non dà altro.
pub struct IteraRegola<'a> {
    r: Rc<Regola>,
    dall_inizio: i64,
    conta: Option<i64>,
    dopo_until: Option<DopoUntil<'a>>,
    coda: VecDeque<Naive>,
    /// Un errore del passo al periodo dopo: esce quando i risultati del periodo sono finiti.
    errore: Option<Errore>,
    finito: bool,
    giri: u64,
    anno: i64,
    mese: i64,
    giorno: i64,
    ora: i64,
    minuto: i64,
    secondo: i64,
    giorno_settimana: i64,
    nth: Option<HashSet<i64>>,
    nth_di: Option<(i64, i64)>,
    anno_settimane: Option<i64>,
    settimane_di: Option<Vec<u8>>,
    anno_pasqua: Option<i64>,
    pasqua_di: Option<Vec<u8>>,
    timeset: Vec<(i64, i64, i64)>,
}

fn contiene(v: &Option<Vec<i64>>, x: i64) -> bool {
    v.as_ref().is_none_or(|v| v.contains(&x))
}

impl<'a> IteraRegola<'a> {
    pub fn nuova(r: Rc<Regola>, inizio: &Naive, dopo_until: Option<DopoUntil<'a>>) -> Self {
        let mut it = IteraRegola {
            dall_inizio: chiave_muro(inizio),
            conta: r.conta,
            dopo_until,
            coda: VecDeque::new(),
            errore: None,
            finito: false,
            giri: 0,
            anno: inizio.a,
            mese: inizio.m,
            giorno: inizio.g,
            ora: inizio.h,
            minuto: inizio.mi,
            secondo: inizio.s,
            giorno_settimana: settimana(ordinale(inizio.a, inizio.m, inizio.g)),
            nth: None,
            nth_di: None,
            anno_settimane: None,
            settimane_di: None,
            anno_pasqua: None,
            pasqua_di: None,
            timeset: Vec::new(),
            r,
        };
        if it.r.freq >= ORARIA {
            let o = it.orario();
            let freq = it.r.freq;
            it.timeset = if !contiene(&o.byhour, it.ora) || (freq >= MINUTALE && !contiene(&o.byminute, it.minuto)) || (freq >= SECONDALE && !contiene(&o.bysecond, it.secondo)) {
                Vec::new()
            } else {
                it.orari(it.ora, it.minuto, it.secondo)
            };
        }
        it
    }

    fn orario(&self) -> Orario {
        self.r.orario.clone().unwrap_or(Orario { byhour: None, byminute: None, bysecond: None })
    }

    fn orari(&self, h: i64, mi: i64, s: i64) -> Vec<(i64, i64, i64)> {
        let o = self.r.orario.as_ref();
        let elenco = |v: Option<&Option<Vec<i64>>>| v.and_then(|x| x.clone()).unwrap_or_default();
        match self.r.freq {
            ORARIA => {
                let secondi = elenco(o.map(|o| &o.bysecond));
                elenco(o.map(|o| &o.byminute)).iter().flat_map(|m2| secondi.iter().map(move |s2| (h, *m2, *s2))).collect()
            }
            MINUTALE => elenco(o.map(|o| &o.bysecond)).iter().map(|s2| (h, mi, *s2)).collect(),
            _ => vec![(h, mi, s)],
        }
    }

    fn chiudi(&mut self) {
        self.coda.clear();
        self.errore = None;
        self.finito = true;
    }

    fn maschere(&mut self) -> Esito<()> {
        if let Some(w) = &self.r.byweekno {
            if self.anno_settimane != Some(self.anno) {
                self.settimane_di = Some(maschere_settimane(self.anno, self.r.wkst, w));
                self.anno_settimane = Some(self.anno);
            }
        }
        if let Some(p) = &self.r.byeaster {
            if self.anno_pasqua != Some(self.anno) {
                self.pasqua_di = Some(maschere_di_pasqua(self.anno, p)?);
                self.anno_pasqua = Some(self.anno);
            }
        }
        Ok(())
    }

    /// Se il giorno passa i vincoli che non sono BYDAY con l'ordinale (che conta solo per le regole di giorni).
    fn giorno_valido(&self, n: i64, capodanno: i64) -> bool {
        let r = &self.r;
        let (a, m, g) = dall_ordinale(n);
        if r.bymonth.as_ref().is_some_and(|v| !v.contains(&m)) {
            return false;
        }
        if self.settimane_di.as_ref().is_some_and(|s| !segnato(s, n - capodanno)) {
            return false;
        }
        if r.byweekday.as_ref().is_some_and(|v| !v.contains(&settimana(n))) {
            return false;
        }
        if self.nth.as_ref().is_some_and(|s| !s.contains(&n)) {
            return false;
        }
        if self.pasqua_di.as_ref().is_some_and(|p| !segnato(p, n - capodanno)) {
            return false;
        }
        if (!r.bymonthday.is_empty() || !r.bynmonthday.is_empty()) && !r.bymonthday.contains(&g) && !r.bynmonthday.contains(&(g - giorni_del_mese(a, m) - 1)) {
            return false;
        }
        if let Some(yd) = &r.byyearday {
            let dell_anno = n - ordinale(a, 1, 1) + 1;
            if !yd.contains(&dell_anno) && !yd.contains(&(dell_anno - if bisestile(a) { 366 } else { 365 } - 1)) {
                return false;
            }
        }
        true
    }

    /// Un periodo delle regole di giorni: i risultati in coda, poi il passo al periodo dopo.
    fn periodo_per_giorni(&mut self) -> Esito<()> {
        self.maschere()?;
        let r = self.r.clone();
        let (anno, mese, giorno) = (self.anno, self.mese, self.giorno);
        let (da, fine) = match r.freq {
            ANNUALE => {
                let da = ordinale(anno, 1, 1);
                (da, da + if bisestile(anno) { 366 } else { 365 })
            }
            MENSILE => {
                let da = ordinale(anno, mese, 1);
                (da, da + giorni_del_mese(anno, mese))
            }
            SETTIMANALE => {
                let da = ordinale(anno, mese, giorno);
                let mut fine = da;
                for _ in 0..7 {
                    fine += 1;
                    if settimana(fine) == r.wkst {
                        break;
                    }
                }
                (da, fine)
            }
            _ => {
                let da = ordinale(anno, mese, giorno);
                (da, da + 1)
            }
        };
        if r.bynweekday.is_some() && (r.freq == ANNUALE || r.freq == MENSILE) {
            let di = if r.freq == ANNUALE { (anno, 0) } else { (anno, mese) };
            if self.nth_di != Some(di) {
                self.nth = Some(con_ordinale(&r, anno, mese));
                self.nth_di = Some(di);
            }
        }
        let capodanno = ordinale(anno, 1, 1);
        let giorni_validi: Vec<i64> = (da..fine).filter(|n| self.giorno_valido(*n, capodanno)).collect();
        match &r.bysetpos {
            Some(pos) if !r.tempi.is_empty() => per_posizione(pos, &giorni_validi, &r.tempi, &mut self.coda),
            _ => {
                for n in &giorni_validi {
                    let (a, m, g) = dall_ordinale(*n);
                    for (h, mi, s) in &r.tempi {
                        self.coda.push_back(Naive { a, m, g, h: *h, mi: *mi, s: *s });
                    }
                }
            }
        }

        // il periodo dopo (dopo l'anno 9999 dateutil si ferma: una regola che non ha mai un giorno valido finisce)
        match r.freq {
            ANNUALE => {
                self.anno = anno.saturating_add(r.intervallo);
                if self.anno > 9999 {
                    self.finito = true;
                }
            }
            MENSILE => {
                let mut mese = mese.saturating_add(r.intervallo);
                let mut anno = anno;
                if mese > 12 {
                    let div = mese.div_euclid(12);
                    mese -= div * 12;
                    anno = anno.saturating_add(div);
                    if mese == 0 {
                        mese = 12;
                        anno -= 1;
                    }
                    if anno > 9999 {
                        self.finito = true;
                    }
                }
                (self.anno, self.mese) = (anno, mese);
            }
            _ => {
                let passo = if r.freq == SETTIMANALE {
                    let gs = self.giorno_settimana;
                    let p = (if r.wkst > gs { -(gs + 1 + (6 - r.wkst)) } else { -(gs - r.wkst) }).saturating_add(r.intervallo.saturating_mul(7));
                    self.giorno_settimana = r.wkst;
                    p
                } else {
                    r.intervallo
                };
                let n = ordinale(anno, mese, giorno).saturating_add(passo);
                // oltre il 31/12/9999
                if n > 2_932_896 {
                    self.finito = true;
                } else {
                    (self.anno, self.mese, self.giorno) = dall_ordinale(n);
                }
            }
        }
        Ok(())
    }

    /// Un giorno delle regole orarie, minutali e secondali: _iter di dateutil per HOURLY, MINUTELY e SECONDLY. L'ora, il minuto e il secondo
    /// avanzano di INTERVAL; con BYHOUR, BYMINUTE o BYSECOND dello stesso livello si salta al valore chiesto che segue; il giorno si controlla
    /// con gli altri vincoli e, se non passa, si salta alla fine del giorno.
    fn periodo_orario(&mut self) -> Esito<()> {
        self.maschere()?;
        let r = self.r.clone();
        let o = self.orario();
        let n = ordinale(self.anno, self.mese, self.giorno);
        let capodanno = ordinale(self.anno, 1, 1);
        let filtrato = !self.giorno_valido(n, capodanno);
        match &r.bysetpos {
            Some(pos) if !self.timeset.is_empty() => {
                let timeset = std::mem::take(&mut self.timeset);
                per_posizione(pos, if filtrato { &[] } else { std::slice::from_ref(&n) }, &timeset, &mut self.coda);
                self.timeset = timeset;
            }
            _ if !filtrato => {
                let (a, m, g) = dall_ordinale(n);
                self.coda.extend(self.timeset.iter().map(|(h, mi, s)| Naive { a, m, g, h: *h, mi: *mi, s: *s }));
            }
            _ => {}
        }
        if let Err(e) = self.passo_orario(&o, filtrato) {
            self.errore = Some(e);
        }
        Ok(())
    }

    fn passo_orario(&mut self, o: &Orario, filtrato: bool) -> Esito<()> {
        let intervallo = self.r.intervallo;
        let (mut ora, mut minuto, mut secondo) = (self.ora, self.minuto, self.secondo);
        let mut correggi_giorno = false;
        let mut giorno = self.giorno;
        match self.r.freq {
            ORARIA => {
                if filtrato {
                    ora += div_giu(23 - ora, intervallo) * intervallo;
                }
                let giorni_in_piu;
                if let Some(byhour) = &o.byhour {
                    (giorni_in_piu, ora) = distanza(ora, byhour, 24, intervallo)?;
                } else {
                    giorni_in_piu = (ora + intervallo).div_euclid(24);
                    ora = (ora + intervallo).rem_euclid(24);
                }
                if giorni_in_piu != 0 {
                    giorno += giorni_in_piu;
                    correggi_giorno = true;
                }
            }
            MINUTALE => {
                if filtrato {
                    minuto += div_giu(1439 - (ora * 60 + minuto), intervallo) * intervallo;
                }
                let mut trovato = false;
                let giro24 = 24 * 60;
                for _ in 0..giro24 / massimo_comune_divisore(intervallo, giro24) {
                    let ore_in_piu;
                    if let Some(byminute) = &o.byminute {
                        (ore_in_piu, minuto) = distanza(minuto, byminute, 60, intervallo)?;
                    } else {
                        ore_in_piu = (minuto + intervallo).div_euclid(60);
                        minuto = (minuto + intervallo).rem_euclid(60);
                    }
                    let giorni_in_piu = (ora + ore_in_piu).div_euclid(24);
                    ora = (ora + ore_in_piu).rem_euclid(24);
                    if giorni_in_piu != 0 {
                        giorno += giorni_in_piu;
                        correggi_giorno = true;
                    }
                    if contiene(&o.byhour, ora) {
                        trovato = true;
                        break;
                    }
                }
                if !trovato {
                    return Err(Errore::Valore("Invalid combination of interval and byhour resulting in empty rule.".to_string()));
                }
            }
            _ => {
                if filtrato {
                    secondo += div_giu(86399 - (ora * 3600 + minuto * 60 + secondo), intervallo) * intervallo;
                }
                let mut trovato = false;
                let giro24 = 24 * 3600;
                for _ in 0..giro24 / massimo_comune_divisore(intervallo, giro24) {
                    let minuti_in_piu;
                    if let Some(bysecond) = &o.bysecond {
                        (minuti_in_piu, secondo) = distanza(secondo, bysecond, 60, intervallo)?;
                    } else {
                        minuti_in_piu = (secondo + intervallo).div_euclid(60);
                        secondo = (secondo + intervallo).rem_euclid(60);
                    }
                    let ore_in_piu = (minuto + minuti_in_piu).div_euclid(60);
                    minuto = (minuto + minuti_in_piu).rem_euclid(60);
                    if ore_in_piu != 0 {
                        ora += ore_in_piu;
                        let giorni_in_piu = ora.div_euclid(24);
                        ora = ora.rem_euclid(24);
                        if giorni_in_piu != 0 {
                            giorno += giorni_in_piu;
                            correggi_giorno = true;
                        }
                    }
                    if contiene(&o.byhour, ora) && contiene(&o.byminute, minuto) && contiene(&o.bysecond, secondo) {
                        trovato = true;
                        break;
                    }
                }
                if !trovato {
                    return Err(Errore::Valore("Invalid combination of interval, byhour and byminute resulting in empty rule.".to_string()));
                }
            }
        }
        (self.ora, self.minuto, self.secondo, self.giorno) = (ora, minuto, secondo, giorno);
        self.timeset = self.orari(ora, minuto, secondo);
        if correggi_giorno && self.giorno > 28 {
            let mut giorni_mese = giorni_del_mese(self.anno, self.mese);
            while self.giorno > giorni_mese {
                self.giorno -= giorni_mese;
                self.mese += 1;
                if self.mese == 13 {
                    self.mese = 1;
                    self.anno += 1;
                    if self.anno > 9999 {
                        self.finito = true;
                        return Ok(());
                    }
                }
                giorni_mese = giorni_del_mese(self.anno, self.mese);
            }
        }
        Ok(())
    }
}

impl Iterator for IteraRegola<'_> {
    type Item = Esito<Naive>;

    fn next(&mut self) -> Option<Esito<Naive>> {
        loop {
            if let Some(res) = self.coda.pop_front() {
                if let Some(d) = &self.dopo_until {
                    match d(&res) {
                        Ok(false) => {}
                        Ok(true) => {
                            self.chiudi();
                            return None;
                        }
                        Err(e) => {
                            self.chiudi();
                            return Some(Err(e));
                        }
                    }
                }
                if chiave_muro(&res) >= self.dall_inizio {
                    if let Some(c) = &mut self.conta {
                        *c -= 1;
                        if *c < 0 {
                            self.chiudi();
                            return None;
                        }
                    }
                    return Some(Ok(res));
                }
                continue;
            }
            if let Some(e) = self.errore.take() {
                self.finito = true;
                return Some(Err(e));
            }
            if self.finito {
                return None;
            }
            let limite = if self.r.freq >= ORARIA { LIMITE_GIRI_ORARI } else { LIMITE_GIRI };
            if self.giri >= limite {
                self.finito = true;
                return Some(Err(Errore::Valore("la regola non finisce".to_string())));
            }
            self.giri += 1;
            let esito = if self.r.freq >= ORARIA { self.periodo_orario() } else { self.periodo_per_giorni() };
            if let Err(e) = esito {
                self.chiudi();
                return Some(Err(e));
            }
        }
    }
}

/// next value di dateutil (__mod_distance): quanti giri e quale valore, partendo da `valore`, finché cade fra quelli chiesti. Senza trovarlo
/// Python ha un TypeError.
fn distanza(mut valore: i64, chiesti: &[i64], base: i64, intervallo: i64) -> Esito<(i64, i64)> {
    let mut accumulo = 0i64;
    for _ in 1..=base {
        let somma = valore + intervallo;
        accumulo += somma.div_euclid(base);
        valore = somma.rem_euclid(base);
        if chiesti.contains(&valore) {
            return Ok((accumulo, valore));
        }
    }
    Err(Errore::Tipo("cannot unpack non-iterable NoneType object".to_string()))
}

#[cfg(test)]
mod prove {
    use super::*;

    fn naive(a: i64, m: i64, g: i64, h: i64, mi: i64, s: i64) -> Naive {
        Naive { a, m, g, h, mi, s }
    }

    fn primi(testo: &str, inizio: Naive, n: usize) -> Esito<Vec<String>> {
        let r = Rc::new(leggi_regola(testo, &inizio)?);
        IteraRegola::nuova(r, &inizio, None).take(n).map(|x| x.map(|x| format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", x.a, x.m, x.g, x.h, x.mi, x.s))).collect()
    }

    #[test]
    fn regole_comuni() {
        let lun = naive(2026, 11, 2, 9, 0, 0);
        assert_eq!(primi("FREQ=WEEKLY;BYDAY=MO,WE,FR", lun, 4).unwrap(), ["2026-11-02 09:00:00", "2026-11-04 09:00:00", "2026-11-06 09:00:00", "2026-11-09 09:00:00"]);
        assert_eq!(primi("FREQ=MONTHLY;BYDAY=-1FR;COUNT=2", lun, 9).unwrap(), ["2026-11-27 09:00:00", "2026-12-25 09:00:00"]);
        assert_eq!(primi("FREQ=MONTHLY", naive(2026, 1, 31, 8, 0, 0), 3).unwrap(), ["2026-01-31 08:00:00", "2026-03-31 08:00:00", "2026-05-31 08:00:00"]);
        assert_eq!(primi("FREQ=YEARLY;BYEASTER=0", naive(2026, 1, 1, 0, 0, 0), 2).unwrap(), ["2026-04-05 00:00:00", "2027-03-28 00:00:00"]);
        assert_eq!(primi("FREQ=YEARLY;BYWEEKNO=1;BYDAY=MO", naive(2026, 1, 1, 0, 0, 0), 1).unwrap(), ["2027-01-04 00:00:00"]);
        assert_eq!(primi("FREQ=HOURLY;INTERVAL=5;BYHOUR=3,8", naive(2026, 1, 1, 3, 0, 0), 2).unwrap(), ["2026-01-01 03:00:00", "2026-01-01 08:00:00"]);
        assert_eq!(primi("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", lun, 2).unwrap(), ["2026-11-30 09:00:00", "2026-12-31 09:00:00"]);
        assert_eq!(primi("FREQ=DAILY;UNTIL=20261104", lun, 9).unwrap().len(), 9);
    }

    #[test]
    fn errori_come_dateutil() {
        let x = naive(2026, 1, 1, 0, 0, 0);
        assert_eq!(leggi_regola("FREQ=DAILY;FOO=1", &x).unwrap_err(), Errore::Valore("unknown parameter 'FOO'".to_string()));
        assert_eq!(leggi_regola("COUNT=1", &x).unwrap_err(), Errore::Tipo("rrule.__init__() missing 1 required positional argument: 'freq'".to_string()));
        assert_eq!(leggi_regola("FREQ=DAILY;BYHOUR=24", &x).unwrap_err(), Errore::Valore("hour must be in 0..23".to_string()));
        assert_eq!(leggi_regola("FREQ=DAILY;COUNT=x", &x).unwrap_err(), Errore::Valore("invalid literal for int(): 'X'".to_string()));
        assert_eq!(leggi_regola("FREQ=WEEKLY;BYDAY=XX", &x).unwrap_err(), Errore::Valore("Expected weekday abbrevation, got: XX".to_string()));
        // un'ora pari non arriva mai a una dispari: l'errore esce al passo, poi niente
        let r = Rc::new(leggi_regola("FREQ=MINUTELY;INTERVAL=120;BYHOUR=5", &naive(2026, 1, 1, 4, 0, 0)).unwrap());
        let v: Vec<Esito<Naive>> = IteraRegola::nuova(r, &naive(2026, 1, 1, 4, 0, 0), None).take(3).collect();
        assert_eq!(v, vec![Err(Errore::Valore("Invalid combination of interval and byhour resulting in empty rule.".to_string()))]);
    }
}
