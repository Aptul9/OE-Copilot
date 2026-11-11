//! Date e orari come li tratta Python (date e datetime), per riprodurre le risposte di recurring_ical_events senza una libreria di
//! calendari: un orario con fuso è l'ora sul muro più il nome del fuso (e se è la seconda delle due ore ripetute dal cambio dell'ora).
//! Due orari dello stesso fuso si confrontano e si sottraggono sul muro, due di fusi diversi sugli istanti: è la regola di Python.

use crate::errori::{Errore, Esito};
use crate::fusi::fuso_personalizzato;
use crate::pyfmt::py_repr_str;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum Tempo {
    Data(Date),
    Ora(Ora),
}

/// `w` è l'ora sul muro, `z` il nome del fuso (None: senza fuso), `fold` vale 1 per la seconda delle due ore ripetute.
#[derive(Clone, Debug)]
pub struct Ora {
    pub w: DateTime,
    pub z: Option<Rc<str>>,
    pub fold: u8,
}

/// Una proprietà che iCalendar non ha saputo leggere (vBroken): l'errore esce solo quando il calcolo la usa, e dice quale attributo si cercava.
#[derive(Clone, Debug)]
pub struct Rotta {
    pub nome: String,
    pub atteso: String,
    pub attributo: String,
    pub errore: String,
}

pub fn errore_rotta(r: &Rotta) -> Errore {
    Errore::Valore(format!(
        "Cannot access {} on broken property {} (expected {}): {}",
        py_repr_str(&r.attributo),
        py_repr_str(&r.nome),
        py_repr_str(&r.atteso),
        r.errore
    ))
}

pub fn data(d: Date) -> Tempo {
    Tempo::Data(d)
}

pub fn ora(w: DateTime, z: Option<Rc<str>>, fold: u8) -> Ora {
    Ora { w, z, fold }
}

pub fn ora_t(w: DateTime, z: Option<Rc<str>>, fold: u8) -> Tempo {
    Tempo::Ora(ora(w, z, fold))
}

fn fuori_limite<E>(_: E) -> Errore {
    Errore::Altro("OverflowError: date value out of range".to_string())
}

const UTC_ALIAS: [&str; 8] = ["UTC", "Etc/UTC", "Etc/UCT", "UCT", "Universal", "Etc/Universal", "Zulu", "Etc/Zulu"];

pub fn e_utc(o: &Ora) -> bool {
    o.z.as_deref().is_some_and(|z| UTC_ALIAS.contains(&z))
}

/// Il numero d'ordine di un giorno dall'epoca (1970-01-01 = 0).
pub fn giorni_da_civile(a: i64, m: i64, g: i64) -> i64 {
    let a = if m <= 2 { a - 1 } else { a };
    let era = a.div_euclid(400);
    let yoe = a - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + g - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Anno, mese e giorno di un numero d'ordine dall'epoca.
pub fn civile_da_giorni(n: i64) -> (i64, i64, i64) {
    let z = n + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Un'ora sul muro da secondi dall'epoca letti come UTC.
pub fn datetime_da_secondi(s: i64) -> Esito<DateTime> {
    let (a, m, g) = civile_da_giorni(s.div_euclid(86400));
    let r = s.rem_euclid(86400);
    DateTime::new(
        i16::try_from(a).map_err(fuori_limite)?,
        m as i8,
        g as i8,
        (r / 3600) as i8,
        (r % 3600 / 60) as i8,
        (r % 60) as i8,
        0,
    )
    .map_err(fuori_limite)
}

/// L'ora sul muro in secondi dall'epoca letta come se fosse UTC.
pub fn muro(w: &DateTime) -> i64 {
    giorni_da_civile(w.year() as i64, w.month() as i64, w.day() as i64) * 86400 + w.hour() as i64 * 3600 + w.minute() as i64 * 60 + w.second() as i64
}

pub fn data_da_giorni(n: i64) -> Esito<Date> {
    let (a, m, g) = civile_da_giorni(n);
    Date::new(i16::try_from(a).map_err(fuori_limite)?, m as i8, g as i8).map_err(fuori_limite)
}

pub fn giorni_di(d: &Date) -> i64 {
    giorni_da_civile(d.year() as i64, d.month() as i64, d.day() as i64)
}

pub fn aggiungi_giorni(d: Date, giorni: i64) -> Esito<Date> {
    let s = Span::new().try_days(giorni).map_err(fuori_limite)?;
    d.checked_add(s).map_err(fuori_limite)
}

thread_local! {
    static FUSI_IANA: RefCell<HashMap<String, Option<TimeZone>>> = RefCell::new(HashMap::new());
    static FUSI_NOTI: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
}

fn cerca_fuso(nome: &str) -> Option<TimeZone> {
    if let Some(t) = FUSI_IANA.with(|c| c.borrow().get(nome).cloned()) {
        return t;
    }
    let t = TimeZone::get(nome).ok().or_else(|| fuso_fisso(nome));
    FUSI_IANA.with(|c| c.borrow_mut().insert(nome.to_string(), t.clone()));
    t
}

/// Uno scarto fisso come nome di fuso ("+01:00", "-05:30:15"), come lo accetta Temporal: è il fuso degli estremi di una finestra.
fn fuso_fisso(nome: &str) -> Option<TimeZone> {
    let b = nome.as_bytes();
    let segno = match b.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let due = |i: usize| -> Option<i32> {
        let x = b.get(i..i + 2)?;
        x.iter().all(u8::is_ascii_digit).then(|| i32::from(x[0] - b'0') * 10 + i32::from(x[1] - b'0'))
    };
    let (h, m) = (due(1)?, if b.get(3) == Some(&b':') { due(4)? } else { return None });
    let s = match b.len() {
        6 => 0,
        9 if b[6] == b':' => due(7)?,
        _ => return None,
    };
    let offset = jiff::tz::Offset::from_seconds(segno * (h * 3600 + m * 60 + s)).ok()?;
    Some(TimeZone::fixed(offset))
}

pub fn fuso_iana(nome: &str) -> Esito<TimeZone> {
    cerca_fuso(nome).ok_or_else(|| Errore::Altro(format!("RangeError: invalid time zone: {nome}")))
}

/// Se il nome è un fuso per Python: ZoneInfo distingue le maiuscole ("europe/rome" no) e non conosce gli scarti ("+01:00").
pub fn fuso_esistente(nome: &str) -> bool {
    if let Some(ok) = FUSI_NOTI.with(|c| c.borrow().get(nome).copied()) {
        return ok;
    }
    let ok = !nome.starts_with(['+', '-'])
        && cerca_fuso(nome).is_some_and(|tz| match tz.iana_name() {
            Some(id) => id == nome || !id.eq_ignore_ascii_case(nome),
            None => false,
        });
    FUSI_NOTI.with(|c| c.borrow_mut().insert(nome.to_string(), ok));
    ok
}

fn risolvi(w: &DateTime, tz: &TimeZone, fold: u8) -> Esito<Timestamp> {
    let a = tz.to_ambiguous_zoned(*w);
    let z = if fold == 1 { a.later() } else { a.compatible() };
    Ok(z.map_err(fuori_limite)?.timestamp())
}

/// L'istante in millisecondi dall'epoca.
pub fn istante(o: &Ora) -> Esito<i64> {
    let z = o.z.as_deref().ok_or_else(|| Errore::Valore("orario senza fuso".to_string()))?;
    if let Some(p) = fuso_personalizzato(z) {
        let m = muro(&o.w);
        return Ok((m - p.scarto(m, o.fold)?) * 1000);
    }
    Ok(risolvi(&o.w, &fuso_iana(z)?, o.fold)?.as_millisecond())
}

fn segno(n: i64) -> i32 {
    n.signum() as i32
}

/// -1, 0, 1 come il confronto fra datetime: sul muro se hanno lo stesso fuso (o nessuno), sugli istanti se no.
pub fn confronta(a: &Ora, b: &Ora) -> Esito<i32> {
    if a.z.is_none() != b.z.is_none() {
        return Err(Errore::Tipo("can't compare offset-naive and offset-aware datetimes".to_string()));
    }
    if a.z == b.z {
        return Ok(a.w.cmp(&b.w) as i32);
    }
    Ok(segno(istante(a)? - istante(b)?))
}

/// a - b in secondi, con la stessa regola di confronto.
pub fn differenza(a: &Ora, b: &Ora) -> Esito<f64> {
    if a.z.is_none() != b.z.is_none() {
        return Err(Errore::Tipo("can't subtract offset-naive and offset-aware datetimes".to_string()));
    }
    Ok(if a.z == b.z { (muro(&a.w) - muro(&b.w)) as f64 } else { (istante(a)? - istante(b)?) as f64 / 1000.0 })
}

/// datetime + timedelta: aritmetica sul muro, nello stesso fuso, e il risultato è la prima delle due ore ripetute.
pub fn somma(o: &Ora, secondi: f64) -> Esito<Ora> {
    let w = if secondi == 0.0 { o.w } else { o.w.checked_add(SignedDuration::from_secs(secondi as i64)).map_err(fuori_limite)? };
    Ok(ora(w, o.z.clone(), 0))
}

pub fn mezzanotte(d: Date) -> DateTime {
    d.at(0, 0, 0, 0)
}

pub fn in_fuso(ms: i64, fuso: &str) -> Esito<Ora> {
    let nome: Rc<str> = Rc::from(fuso);
    if let Some(p) = fuso_personalizzato(fuso) {
        // _tzinfo.fromutc di dateutil: lo scarto e l'ora legale dell'ora UTC letta come ora sul muro, poi l'ora legale del risultato (con fold=1)
        let utc = ms.div_euclid(1000);
        let delta = p.scarto(utc, 0)? - p.dst(utc, 0)?;
        let prima = utc + delta;
        let muro_secondi = prima + p.dst(prima, 1)?;
        let ripetuta = p.scarto(muro_secondi, 0)? != p.scarto(muro_secondi, 1)?;
        let w = datetime_da_secondi(muro_secondi)?;
        return Ok(ora(w, Some(nome), if ripetuta && muro_secondi - utc == delta { 1 } else { 0 }));
    }
    let tz = fuso_iana(fuso)?;
    let w = tz.to_datetime(Timestamp::from_millisecond(ms).map_err(fuori_limite)?);
    let primo = risolvi(&w, &tz, 0)?.as_millisecond();
    Ok(ora(w, Some(nome), if primo == ms { 0 } else { 1 }))
}

/// convert_to_datetime: una data diventa la mezzanotte; con `z` un orario senza fuso lo prende, senza `z` uno con fuso lo perde.
pub fn come_ora(t: &Tempo, z: &Option<Rc<str>>) -> Ora {
    match t {
        Tempo::Data(d) => ora(mezzanotte(*d), z.clone(), 0),
        Tempo::Ora(o) => match (&o.z, z) {
            (None, None) => o.clone(),
            (None, Some(_)) => ora(o.w, z.clone(), o.fold),
            (Some(_), None) => ora(o.w, None, 0),
            (Some(_), Some(_)) => o.clone(),
        },
    }
}

/// make_comparable: se c'è un orario con fuso, tutti diventano orari con quel fuso (il primo che si trova); se sono tutte date restano date.
pub fn rendi_comparabili(v: &[Tempo]) -> Vec<Tempo> {
    let mut z: Option<Rc<str>> = None;
    let mut tutte_date = true;
    for t in v {
        if let Tempo::Ora(o) = t {
            tutte_date = false;
            if o.z.is_some() {
                z = o.z.clone();
                break;
            }
        }
    }
    if tutte_date {
        return v.to_vec();
    }
    v.iter().map(|t| Tempo::Ora(come_ora(t, &z))).collect()
}

/// Confronto di due valori già resi comparabili (due date o due orari).
pub fn confronta_t(a: &Tempo, b: &Tempo) -> Esito<i32> {
    match (a, b) {
        (Tempo::Data(x), Tempo::Data(y)) => Ok(x.cmp(y) as i32),
        (Tempo::Ora(x), Tempo::Ora(y)) => confronta(x, y),
        _ => Err(Errore::Tipo("can't compare datetime.datetime to datetime.date".to_string())),
    }
}

pub fn confronta_grande(a: &Tempo, b: &Tempo) -> Esito<bool> {
    let v = rendi_comparabili(&[a.clone(), b.clone()]);
    Ok(confronta_t(&v[0], &v[1])? > 0)
}

/// a - b in secondi di due valori comparabili.
pub fn differenza_t(a: &Tempo, b: &Tempo) -> Esito<f64> {
    match (a, b) {
        (Tempo::Data(x), Tempo::Data(y)) => Ok(((giorni_di(x) - giorni_di(y)) * 86400) as f64),
        (Tempo::Ora(x), Tempo::Ora(y)) => differenza(x, y),
        _ => Err(Errore::Tipo("unsupported operand type(s) for -: date and datetime".to_string())),
    }
}

/// Tempo + timedelta: a una data si aggiungono solo i giorni interi.
pub fn somma_t(t: &Tempo, secondi: f64) -> Esito<Tempo> {
    match t {
        Tempo::Data(d) => Ok(Tempo::Data(aggiungi_giorni(*d, (secondi / 86400.0).floor() as i64)?)),
        Tempo::Ora(o) => Ok(Tempo::Ora(somma(o, secondi)?)),
    }
}

/// Il valore come data (convert_to_date).
pub fn come_data(t: &Tempo) -> Date {
    match t {
        Tempo::Data(d) => *d,
        Tempo::Ora(o) => o.w.date(),
    }
}

pub fn testo_data(d: &Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

/// L'offset come lo scrive Python: +HH:MM, con i secondi se ci sono.
fn testo_scarto(s: i64) -> String {
    let a = s.abs();
    let mut out = format!("{}{:02}:{:02}", if s < 0 { '-' } else { '+' }, a / 3600, a % 3600 / 60);
    if a % 60 != 0 {
        out.push_str(&format!(":{:02}", a % 60));
    }
    out
}

fn testo_base(w: &DateTime, sep: char, micro: bool) -> String {
    let mut s = format!("{:04}-{:02}-{:02}{}{:02}:{:02}:{:02}", w.year(), w.month(), w.day(), sep, w.hour(), w.minute(), w.second());
    let us = w.subsec_nanosecond() / 1000;
    if micro && us != 0 {
        s.push_str(&format!(".{us:06}"));
    }
    s
}

/// str() di una data o di un orario come Python lo scrive nei messaggi: AAAA-MM-GG, o AAAA-MM-GG HH:MM:SS[.ffffff] con lo scarto (+HH:MM) se ha un fuso.
pub fn testo_py(t: &Tempo) -> Esito<String> {
    let o = match t {
        Tempo::Data(d) => return Ok(testo_data(d)),
        Tempo::Ora(o) => o,
    };
    let base = testo_base(&o.w, ' ', true);
    let Some(z) = o.z.as_deref() else { return Ok(base) };
    if let Some(p) = fuso_personalizzato(z) {
        return Ok(base + &testo_scarto(p.scarto(muro(&o.w), o.fold)?));
    }
    let tz = fuso_iana(z)?;
    let ts = risolvi(&o.w, &tz, o.fold)?;
    Ok(base + &testo_scarto(tz.to_offset(ts).seconds() as i64))
}

pub fn testo_naive(w: &DateTime) -> String {
    testo_base(w, 'T', false)
}

/// to_recurrence_ids: come si riconosce un'occorrenza. Un orario con fuso ne ha due (l'istante in UTC e l'ora sul muro), uno senza
/// fuso o una data ne ha uno: un'esclusione che coincide in uno qualunque dei due toglie l'occorrenza.
pub fn id_ricorrenza(t: &Tempo) -> Esito<Vec<String>> {
    match t {
        Tempo::Data(d) => Ok(vec![testo_naive(&mezzanotte(*d))]),
        Tempo::Ora(o) if o.z.is_none() => Ok(vec![testo_naive(&o.w)]),
        Tempo::Ora(o) => {
            let utc = datetime_da_secondi(istante(o)?.div_euclid(1000))?;
            Ok(vec![testo_naive(&utc), testo_naive(&o.w)])
        }
    }
}

/// datetime.astimezone(fuso).replace(tzinfo=None): l'ora locale di Roma di un valore (le date a mezzanotte).
pub fn in_ora_locale(t: &Tempo, fuso: &str, stesso_fuso_invariato: bool) -> Esito<DateTime> {
    match t {
        Tempo::Data(d) => Ok(mezzanotte(*d)),
        Tempo::Ora(o) => match o.z.as_deref() {
            None => Ok(o.w),
            Some(z) if stesso_fuso_invariato && z == fuso => Ok(o.w),
            Some(_) => {
                let ms = istante(o)?;
                Ok(fuso_iana(fuso)?.to_datetime(Timestamp::from_millisecond(ms).map_err(fuori_limite)?))
            }
        },
    }
}
