//! Date e ore "locali" dell'app: ora di Roma senza scarto, come i datetime senza fuso del servizio Python (TZ=Europe/Rome). Si
//! rappresentano con i loro pezzi e si scrivono come isoformat() di Python: AAAA-MM-GGTHH:MM:SS, e .ffffff solo se i microsecondi non sono
//! zero (era lib/core/fuso.ts). Le operazioni sui giorni sono quelle di `Date.UTC` di JavaScript, anni da 0 a 99 compresi (valgono 1900 + anno).

use crate::py::{Errore, Esito};
use jiff::tz::TimeZone;
use jiff::Timestamp;
use oe_nucleo::tempo::{civile_da_giorni, giorni_da_civile};
use regex::Regex;
use std::sync::LazyLock;

pub const FUSO: &str = "Europe/Rome";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Naive {
    pub a: i64,
    pub m: i64,
    pub g: i64,
    pub h: i64,
    pub mi: i64,
    pub s: i64,
    pub us: i64,
}

thread_local! {
    static ROMA: TimeZone = TimeZone::get(FUSO).expect("il database dei fusi porta Europe/Rome");
}

/// `Date.UTC(a, m0, g, h, mi, s)` in millisecondi: il mese parte da 0 e può uscire dall'anno, gli anni da 0 a 99 sono 1900 + anno.
pub fn utc_js(a: i64, m0: i64, g: i64, h: i64, mi: i64, s: i64) -> i64 {
    let a = if (0..=99).contains(&a) { a + 1900 } else { a };
    let anno = a + m0.div_euclid(12);
    let mese = m0.rem_euclid(12) + 1;
    let giorni = giorni_da_civile(anno, mese, 1) + g - 1;
    giorni * 86_400_000 + h * 3_600_000 + mi * 60_000 + s * 1000
}

/// I pezzi UTC (anno, mese da 1, giorno, ore, minuti, secondi) di un istante in millisecondi: getUTCFullYear e compagni.
pub fn pezzi_utc(ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let giorni = ms.div_euclid(86_400_000);
    let resto = ms.rem_euclid(86_400_000) / 1000;
    let (a, m, g) = civile_da_giorni(giorni);
    (a, m, g, resto / 3600, resto % 3600 / 60, resto % 60)
}

fn due(n: i64) -> String {
    format!("{n:02}")
}

fn quattro(n: i64) -> String {
    if n < 0 {
        format!("-{:04}", -n)
    } else {
        format!("{n:04}")
    }
}

/// Come datetime.isoformat()
pub fn iso(n: &Naive) -> String {
    let base = format!("{}-{}-{}T{}:{}:{}", quattro(n.a), due(n.m), due(n.g), due(n.h), due(n.mi), due(n.s));
    if n.us != 0 {
        format!("{base}.{:06}", n.us)
    } else {
        base
    }
}

/// Come isoformat(timespec="seconds")
pub fn iso_secondi(n: &Naive) -> String {
    iso(&Naive { us: 0, ..*n })
}

/// Un valore per confrontare due date locali: microsecondi dall'epoca, contando l'ora locale come se fosse UTC.
pub fn chiave(n: &Naive) -> i64 {
    utc_js(n.a, n.m - 1, n.g, n.h, n.mi, n.s) * 1000 + n.us
}

pub fn mezzanotte(a: i64, m: i64, g: i64) -> Naive {
    Naive { a, m, g, h: 0, mi: 0, s: 0, us: 0 }
}

/// La stessa ora locale `giorni` giorni dopo (aritmetica senza fuso: un giorno è sempre un giorno).
pub fn piu_giorni(n: &Naive, giorni: i64) -> Naive {
    let (a, m, g, ..) = pezzi_utc(utc_js(n.a, n.m - 1, n.g + giorni, 0, 0, 0));
    Naive { a, m, g, ..*n }
}

/// Un istante (millisecondi dall'epoca) come ora locale di Roma.
pub fn da_istante(ms: i64, us: i64) -> Naive {
    let secondi = ms.div_euclid(1000);
    let ts = Timestamp::from_second(secondi).unwrap_or(Timestamp::UNIX_EPOCH);
    let d = ROMA.with(|tz| tz.to_datetime(ts));
    Naive { a: d.year() as i64, m: d.month() as i64, g: d.day() as i64, h: d.hour() as i64, mi: d.minute() as i64, s: d.second() as i64, us }
}

/// Lo scarto di Roma da UTC in millisecondi a un istante.
pub fn scarto_roma(ms: i64) -> i64 {
    let n = da_istante(ms, 0);
    utc_js(n.a, n.m - 1, n.g, n.h, n.mi, n.s) - ms.div_euclid(1000) * 1000
}

/// L'istante (millisecondi dall'epoca) di un'ora locale di Roma, con lo scarto del giorno prima o di quello dopo: l'ora che il cambio
/// ripete vale la prima volta, quella che salta vale come prima del salto.
pub fn istante(n: &Naive) -> i64 {
    let come_utc = utc_js(n.a, n.m - 1, n.g, n.h, n.mi, n.s);
    let a_roma = |ms: i64| chiave(&da_istante(ms, 0)) / 1000;
    let prima = come_utc - (a_roma(come_utc - 86_400_000) - (come_utc - 86_400_000));
    let dopo = come_utc - (a_roma(come_utc + 86_400_000) - (come_utc + 86_400_000));
    if a_roma(prima) == come_utc || a_roma(dopo) != come_utc {
        prima
    } else {
        dopo
    }
}

/// I giorni del mese `m` (da 1) dell'anno `a`, come `new Date(Date.UTC(a, m, 0)).getUTCDate()`.
pub fn giorni_nel_mese(a: i64, m: i64) -> i64 {
    pezzi_utc(utc_js(a, m, 0, 0, 0, 0)).2
}

// datetime.fromisoformat (Python 3.12), nelle forme che arrivano davvero: data, data e ora (T o spazio), frazione di secondo, scarto
// (Z, +HH, +HH:MM, +HHMM). Il resto è "Invalid isoformat string", come in Python.
static ISO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([0-9]{4})-?([0-9]{2})-?([0-9]{2})(?:[T ]([0-9]{2})(?::?([0-9]{2})(?::?([0-9]{2})(?:[.,]([0-9]+))?)?)?)?(Z|[+-][0-9]{2}(?::?[0-9]{2}(?::?[0-9]{2}(?:\.[0-9]+)?)?)?)?$").unwrap()
});

/// Un testo ISO come datetime: i pezzi e lo scarto in minuti (None se il testo non ne ha).
pub fn leggi_iso(testo: &str) -> Esito<(Naive, Option<i64>)> {
    let Some(m) = ISO_RE.captures(testo) else { return Err(Errore::Data(format!("Invalid isoformat string: '{testo}'"))) };
    let num = |i: usize| m.get(i).map_or(0, |x| x.as_str().parse::<i64>().unwrap_or(0));
    let (a, mo, g, h, mi, s) = (num(1), num(2), num(3), num(4), num(5), num(6));
    if !(1..=12).contains(&mo) {
        return Err(Errore::Data("month must be in 1..12".to_string()));
    }
    if g < 1 || g > giorni_nel_mese(a, mo) {
        return Err(Errore::Data("day is out of range for month".to_string()));
    }
    if h > 23 {
        return Err(Errore::Data("hour must be in 0..23".to_string()));
    }
    if mi > 59 {
        return Err(Errore::Data("minute must be in 0..59".to_string()));
    }
    if s > 59 {
        return Err(Errore::Data("second must be in 0..59".to_string()));
    }
    let us = m.get(7).map_or(0, |f| {
        let mut cifre: String = f.as_str().chars().take(6).collect();
        while cifre.len() < 6 {
            cifre.push('0');
        }
        cifre.parse::<i64>().unwrap_or(0)
    });
    let scarto = m.get(8).map(|z| {
        let z = z.as_str();
        if z == "Z" {
            return 0;
        }
        let segno = if z.starts_with('-') { -1 } else { 1 };
        let cifre: String = z[1..].chars().filter(|c| *c != ':' && *c != '.').collect();
        let ore = cifre.get(0..2).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        let minuti = cifre.get(2..4).filter(|x| !x.is_empty()).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        segno * (ore * 60 + minuti)
    });
    Ok((Naive { a, m: mo, g, h, mi, s, us }, scarto))
}

fn in_utc(n: &Naive, scarto: i64) -> i64 {
    utc_js(n.a, n.m - 1, n.g, n.h, n.mi, n.s) - scarto * 60_000
}

/// Un estremo dell'intervallo (caselle.parse_bound): la pagina manda 2026-07-20T00:00:00+02:00, gli eventi sono senza fuso, quindi lo
/// scarto, che è autentico, si converte nell'ora locale. Senza scarto il testo vale com'è.
pub fn parse_bound(testo: &str) -> Esito<Naive> {
    let t = crate::py::js_trim(testo);
    let t = match t.strip_suffix('Z') {
        Some(resto) => format!("{resto}+00:00"),
        None => t.to_string(),
    };
    let (naive, scarto) = leggi_iso(&t)?;
    Ok(match scarto {
        None => naive,
        Some(s) => da_istante(in_utc(&naive, s), naive.us),
    })
}

/// Come datetime.fromisoformat, ma dei soli testi senza scarto (a quelli con lo scarto Python non sa confrontare un'ora locale).
pub fn leggi_naive(testo: &str) -> Esito<Naive> {
    let (naive, scarto) = leggi_iso(testo)?;
    if scarto.is_some() {
        return Err(Errore::Data("can't compare offset-naive and offset-aware datetimes".to_string()));
    }
    Ok(naive)
}

/// Un testo ISO come ora locale senza scarto (disponibilita._naive): con lo scarto si converte, senza vale com'è.
pub fn naive_locale(testo: &str) -> Esito<Naive> {
    let t = crate::py::js_trim(testo).replace('Z', "+00:00");
    let (naive, scarto) = leggi_iso(&t)?;
    Ok(match scarto {
        None => naive,
        Some(s) => da_istante(in_utc(&naive, s), naive.us),
    })
}

/// L'ora, HH:MM, di un istante a Roma (time.strftime("%H:%M"))
pub fn ora_hhmm(ms: i64) -> String {
    let n = da_istante(ms, 0);
    format!("{}:{}", due(n.h), due(n.mi))
}

/// datetime.fromtimestamp(secondi).isoformat(timespec="seconds"): l'ora di Roma di un istante
pub fn da_secondi(secondi: f64) -> String {
    iso_secondi(&da_istante(secondi.floor() as i64 * 1000, 0))
}

/// AAAA-MM-GG
pub fn data_testo(n: &Naive) -> String {
    format!("{}-{}-{}", quattro(n.a), due(n.m), due(n.g))
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn iso_e_scarti() {
        let (n, z) = leggi_iso("2026-10-25T02:30:00.5+01:00").unwrap();
        assert_eq!(iso(&n), "2026-10-25T02:30:00.500000");
        assert_eq!(z, Some(60));
        assert_eq!(iso(&parse_bound("2026-07-20T00:00:00+02:00").unwrap()), "2026-07-20T00:00:00");
        assert_eq!(iso(&parse_bound("2026-07-19T22:00:00Z").unwrap()), "2026-07-20T00:00:00");
        assert!(matches!(leggi_iso("2026-02-29"), Err(Errore::Data(m)) if m == "day is out of range for month"));
        assert!(matches!(leggi_iso("2026-1-01"), Err(Errore::Data(m)) if m == "Invalid isoformat string: '2026-1-01'"));
        assert!(leggi_iso("٢٠٢٦-01-01").is_err());
    }

    #[test]
    fn cambio_dell_ora() {
        // l'ora ripetuta vale la prima volta (+02:00), quella che salta vale con lo scarto di prima (+01:00)
        assert_eq!(istante(&leggi_naive("2026-10-25T02:30:00").unwrap()), 1_792_888_200_000);
        assert_eq!(istante(&leggi_naive("2026-03-29T02:30:00").unwrap()), 1_774_747_800_000);
        assert_eq!(ora_hhmm(1_792_888_200_000), "02:30");
        assert_eq!(da_secondi(1_792_891_800.9), "2026-10-25T02:30:00");
    }

    #[test]
    fn gli_anni_di_date_utc() {
        // Date.UTC(50, 1, 29): febbraio del 1950, che ne ha 28
        assert!(leggi_iso("0050-02-29").is_err());
        assert_eq!(piu_giorni(&mezzanotte(50, 1, 1), 1).a, 1950);
        assert_eq!(piu_giorni(&mezzanotte(2026, 12, 31), 1), mezzanotte(2027, 1, 1));
    }
}
