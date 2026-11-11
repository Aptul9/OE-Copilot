//! datetime.fromisoformat di Python 3.12 (il C di _datetimemodule.c, passo per passo: accetta più forme di quelle che ci si aspetta, e il
//! testo dell'errore dipende da dove si ferma) e _locale() dell'agente Python: un estremo ISO, con o senza scarto, come ora locale senza fuso.

use crate::errori::{Errore, Esito};
use crate::pyfmt::{py_repr_delta, py_repr_str};
use crate::tempo::fuso_iana;
use jiff::SignedDuration;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

#[derive(Clone, Debug, Default)]
pub struct DataOraPy {
    pub a: i64,
    pub m: i64,
    pub g: i64,
    pub h: i64,
    pub mi: i64,
    pub s: i64,
    pub us: i64,
    /// Lo scarto dall'UTC in secondi e microsecondi (con il segno), se il testo ne ha uno.
    pub scarto: Option<(i64, i64)>,
}

fn cifra_ascii(c: u8) -> bool {
    c.is_ascii_digit()
}

struct Lettore<'a> {
    b: &'a [u8],
}

impl Lettore<'_> {
    fn at(&self, i: i64) -> u8 {
        if i >= 0 && (i as usize) < self.b.len() { self.b[i as usize] } else { 0 }
    }

    /// parse_digits: `n` cifre ASCII di seguito, sommate a `v`; p = -1 se una non lo è.
    fn cifre(&self, mut p: i64, n: usize, mut v: i64) -> (i64, i64) {
        for _ in 0..n {
            let t = self.at(p) as i64 - 0x30;
            p += 1;
            if !(0..=9).contains(&t) {
                return (-1, v);
            }
            v = v * 10 + t;
        }
        (p, v)
    }
}

fn bisestile(a: i64) -> bool {
    a % 4 == 0 && (a % 100 != 0 || a % 400 == 0)
}

const GIORNI_PRIMA: [i64; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const GIORNI_MESE: [i64; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn giorni_prima_anno(a: i64) -> i64 {
    (a - 1) * 365 + (a - 1).div_euclid(4) - (a - 1).div_euclid(100) + (a - 1).div_euclid(400)
}

/// Il numero d'ordine di un giorno, con 0001-01-01 = 1.
fn ordinale(a: i64, m: i64, g: i64) -> i64 {
    giorni_prima_anno(a) + GIORNI_PRIMA[m as usize] + if m > 2 && bisestile(a) { 1 } else { 0 } + g
}

fn da_ordinale(ord: i64) -> (i64, i64, i64) {
    let mut n = ord - 1;
    let n400 = n.div_euclid(146097);
    n -= n400 * 146097;
    let n100 = n.div_euclid(36524);
    n -= n100 * 36524;
    let n4 = n.div_euclid(1461);
    n -= n4 * 1461;
    let n1 = n.div_euclid(365);
    n -= n1 * 365;
    let anno = n400 * 400 + 1 + n100 * 100 + n4 * 4 + n1;
    if n1 == 4 || n100 == 4 {
        return (anno - 1, 12, 31);
    }
    let bis = n1 == 3 && (n4 != 24 || n100 == 3);
    let mut mese = (n + 50) >> 5;
    let mut prima = GIORNI_PRIMA[mese as usize] + if mese > 2 && bis { 1 } else { 0 };
    if prima > n {
        mese -= 1;
        prima -= if mese == 2 && bis { 29 } else { GIORNI_MESE[mese as usize] };
    }
    n -= prima;
    (anno, mese, n + 1)
}

/// iso_to_ymd: da anno, settimana e giorno ISO a data; Ok se va bene, Err(codice negativo) se la settimana o il giorno non esistono.
fn iso_a_data(anno: i64, settimana: i64, giorno: i64) -> Result<(i64, i64, i64), i64> {
    if !(1..=9999).contains(&anno) {
        return Err(-4);
    }
    if settimana <= 0 || settimana >= 53 {
        let mut fuori = true;
        if settimana == 53 {
            let primo = (ordinale(anno, 1, 1) + 6) % 7;
            if primo == 3 || (primo == 2 && bisestile(anno)) {
                fuori = false;
            }
        }
        if fuori {
            return Err(-2);
        }
    }
    if giorno <= 0 || giorno >= 8 {
        return Err(-3);
    }
    let primo_giorno = ordinale(anno, 1, 1);
    let primo_settimana = (primo_giorno + 6) % 7;
    let mut lunedi = primo_giorno - primo_settimana;
    if primo_settimana > 3 {
        lunedi += 7;
    }
    Ok(da_ordinale(lunedi + (settimana - 1) * 7 + giorno - 1))
}

fn trova_separatore(l: &Lettore) -> i64 {
    let len = l.b.len() as i64;
    if len == 7 {
        return 7;
    }
    const TRATTINO: u8 = 0x2d;
    const W: u8 = 0x57;
    if l.at(4) == TRATTINO {
        if l.at(5) == W {
            if len < 8 {
                return -1;
            }
            if len > 8 && l.at(8) == TRATTINO {
                if len == 9 {
                    return -1;
                }
                if len > 10 && cifra_ascii(l.at(10)) {
                    return 8;
                }
                return 10;
            }
            return 8;
        }
        return 10;
    }
    if l.at(4) == W {
        let mut i = 7;
        while i < len {
            if !cifra_ascii(l.b[i as usize]) {
                break;
            }
            i += 1;
        }
        if i < 9 {
            return i;
        }
        return if i % 2 == 0 { 7 } else { 8 };
    }
    8
}

/// parse_isoformat_date: 0 se va bene, un numero negativo se no. `len` è dove finisce la parte della data.
fn leggi_data(l: &Lettore, len: i64, o: &mut DataOraPy) -> i64 {
    let (mut p, anno) = l.cifre(0, 4, 0);
    if p < 0 {
        return -1;
    }
    let con_separatore = l.at(p) == 0x2d;
    if con_separatore {
        p += 1;
    }
    if l.at(p) == 0x57 {
        p += 1;
        let (np, settimana) = l.cifre(p, 2, 0);
        p = np;
        if p < 0 {
            return -3;
        }
        let giorno;
        if p < len {
            if con_separatore {
                let c = l.at(p);
                p += 1;
                if c != 0x2d {
                    return -2;
                }
            }
            let (np, g) = l.cifre(p, 1, 0);
            p = np;
            if p < 0 {
                return -4;
            }
            giorno = g;
        } else {
            giorno = 1;
        }
        return match iso_a_data(anno, settimana, giorno) {
            Err(cod) => -3 + cod,
            Ok((a, m, g)) => {
                o.a = a;
                o.m = m;
                o.g = g;
                0
            }
        };
    }
    o.a = anno;
    let (np, mese) = l.cifre(p, 2, 0);
    p = np;
    if p < 0 {
        return -1;
    }
    o.m = mese;
    if con_separatore {
        let c = l.at(p);
        p += 1;
        if c != 0x2d {
            return -2;
        }
    }
    let (np, giorno) = l.cifre(p, 2, 0);
    p = np;
    if p < 0 {
        return -1;
    }
    o.g = giorno;
    0
}

const CORREZIONE: [i64; 5] = [100000, 10000, 1000, 100, 10];

#[derive(Default)]
struct Ora {
    h: i64,
    mi: i64,
    s: i64,
    us: i64,
}

/// parse_hh_mm_ss_ff: 0 se la stringa finisce qui, 1 se segue altro, un numero negativo se non è un'ora.
fn leggi_ora(l: &Lettore, inizio: i64, fine: i64, v: &mut Ora) -> i64 {
    *v = Ora::default();
    let mut p = inizio;
    let mut con_separatore = true;
    for i in 0..3 {
        let (np, n) = l.cifre(p, 2, 0);
        p = np;
        if p < 0 {
            return -3;
        }
        match i {
            0 => v.h = n,
            1 => v.mi = n,
            _ => v.s = n,
        }
        let c = l.at(p);
        p += 1;
        if i == 0 {
            con_separatore = c == 0x3a;
        }
        if p >= fine {
            return if c != 0 { 1 } else { 0 };
        }
        if con_separatore && c == 0x3a {
            continue;
        }
        if c == 0x2e || c == 0x2c {
            break;
        }
        if !con_separatore {
            p -= 1;
        } else {
            return -4;
        }
    }
    let resto = fine - p;
    let da_leggere = if !(0..6).contains(&resto) { 6 } else { resto as usize };
    let (np, mut micro) = l.cifre(p, da_leggere, 0);
    p = np;
    if p < 0 {
        return -3;
    }
    if da_leggere < 6 && da_leggere > 0 {
        micro *= CORREZIONE[da_leggere - 1];
    }
    v.us = micro;
    while cifra_ascii(l.at(p)) {
        p += 1;
    }
    if l.at(p) != 0 { 1 } else { 0 }
}

/// parse_isoformat_time: 0 senza scarto, 1 con scarto, un numero negativo se non va.
fn leggi_ora_e_scarto(l: &Lettore, inizio: i64, fine: i64, o: &mut DataOraPy) -> i64 {
    let mut marca = inizio;
    loop {
        let c = l.at(marca);
        if c == 0x5a || c == 0x2b || c == 0x2d {
            break;
        }
        marca += 1;
        if marca >= fine {
            break;
        }
    }
    let mut v = Ora::default();
    let rv = leggi_ora(l, inizio, marca, &mut v);
    o.h = v.h;
    o.mi = v.mi;
    o.s = v.s;
    o.us = v.us;
    if rv < 0 {
        return rv;
    }
    if marca == fine {
        return if rv == 1 { -5 } else { 0 };
    }
    if l.at(marca) == 0x5a {
        o.scarto = Some((0, 0));
        return if l.at(marca + 1) != 0 { -5 } else { 1 };
    }
    let segno = if l.at(marca) == 0x2d { -1 } else { 1 };
    let mut z = Ora::default();
    let rz = leggi_ora(l, marca + 1, fine, &mut z);
    o.scarto = Some((segno * (z.h * 3600 + z.mi * 60 + z.s), segno * z.us));
    if rz != 0 { -5 } else { 1 }
}

/// datetime.fromisoformat(): la data e l'ora come scritte nel testo, con lo scarto se c'è; ValueError con il testo di Python se no.
pub fn from_isoformat(testo: &str) -> Esito<DataOraPy> {
    let invalida = || Errore::Valore(format!("Invalid isoformat string: {}", py_repr_str(testo)));
    if testo.chars().count() < 7 {
        return Err(invalida());
    }
    let l = Lettore { b: testo.as_bytes() };
    let len = l.b.len() as i64;
    let sep = trova_separatore(&l);
    let mut o = DataOraPy::default();
    if sep < 0 {
        return Err(invalida());
    }
    let mut rv = leggi_data(&l, sep, &mut o);
    if rv == 0 && len > sep {
        let mut p = sep;
        let primo = l.at(p);
        if primo & 0x80 == 0 {
            p += 1;
        } else {
            let alto = primo & 0xf0;
            p += if alto == 0xe0 {
                3
            } else if alto == 0xf0 {
                4
            } else {
                2
            };
        }
        rv = leggi_ora_e_scarto(&l, p, len, &mut o);
    }
    if rv < 0 {
        return Err(invalida());
    }
    if rv == 1 {
        if let Some((s, us)) = o.scarto {
            if s != 0 {
                // new_timezone: lo scarto deve stare fra -24 e +24 ore (esclusi)
                let totale = s * 1_000_000 + us;
                let giorni = totale.div_euclid(86_400_000_000);
                let resto = totale - giorni * 86_400_000_000;
                if (giorni == -1 && resto == 0) || giorni < -1 || giorni >= 1 {
                    let secondi = resto.div_euclid(1_000_000);
                    return Err(Errore::Valore(format!(
                        "offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not {}.",
                        py_repr_delta(giorni, secondi, resto - secondi * 1_000_000)
                    )));
                }
            } else {
                o.scarto = Some((0, 0)); // lo scarto zero è UTC, senza i microsecondi
            }
        }
    }
    if !(1..=9999).contains(&o.a) {
        return Err(Errore::Valore(format!("year {} is out of range", o.a)));
    }
    if !(1..=12).contains(&o.m) {
        return Err(Errore::Valore("month must be in 1..12".to_string()));
    }
    let max_giorni = if o.m == 2 && bisestile(o.a) { 29 } else { GIORNI_MESE[o.m as usize] };
    if o.g < 1 || o.g > max_giorni {
        return Err(Errore::Valore("day is out of range for month".to_string()));
    }
    if o.h > 23 {
        return Err(Errore::Valore("hour must be in 0..23".to_string()));
    }
    if o.mi > 59 {
        return Err(Errore::Valore("minute must be in 0..59".to_string()));
    }
    if o.s > 59 {
        return Err(Errore::Valore("second must be in 0..59".to_string()));
    }
    Ok(o)
}

/// isoformat() di una data e ora senza fuso: i microsecondi solo se ci sono.
pub fn iso_naive(w: &DateTime) -> String {
    let mut s = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", w.year(), w.month(), w.day(), w.hour(), w.minute(), w.second());
    let us = w.subsec_nanosecond() / 1000;
    if us != 0 {
        s.push_str(&format!(".{us:06}"));
    }
    s
}

fn overflow() -> Errore {
    Errore::SenzaRisposta("OverflowError: date value out of range".to_string())
}

fn costruisci(a: i64, m: i64, g: i64, h: i64, mi: i64, s: i64, nanos: i64) -> Esito<DateTime> {
    DateTime::new(a as i16, m as i8, g as i8, h as i8, mi as i8, s as i8, nanos as i32).map_err(|_| overflow())
}

/// L'ora sul muro di un'ora UTC nel fuso. jiff non rappresenta gli istanti oltre il 30/12/9999 22:00 UTC, Temporal e Python sì: il
/// calendario gregoriano e le regole dei fusi si ripetono ogni 400 anni, e lo scarto si prende lì.
fn utc_in_fuso(utc: DateTime, tz: &TimeZone) -> Esito<DateTime> {
    if let Ok(z) = TimeZone::UTC.to_zoned(utc) {
        return Ok(tz.to_datetime(z.timestamp()));
    }
    let prima = utc.checked_sub(jiff::Span::new().years(400)).map_err(|_| overflow())?;
    let scarto = tz.to_offset(TimeZone::UTC.to_zoned(prima).map_err(|_| overflow())?.timestamp());
    utc.checked_add(SignedDuration::from_secs(i64::from(scarto.seconds()))).map_err(|_| overflow())
}

/// _locale() dell'agente Python: un estremo ISO come ora locale senza fuso (con uno scarto si converte, senza si lascia com'è).
pub fn ora_locale_da(testo: &str, zona: &str) -> Esito<DateTime> {
    let d = from_isoformat(&testo.replace('Z', "+00:00"))?;
    let Some((scarto_s, scarto_us)) = d.scarto else {
        return costruisci(d.a, d.m, d.g, d.h, d.mi, d.s, d.us * 1000);
    };
    // astimezone(): l'istante in UTC (fuori da 0001..9999 Python solleva OverflowError, che l'agente non prende) e poi l'ora locale
    let mut micro = d.us - scarto_us;
    let riporto = micro.div_euclid(1_000_000);
    micro -= riporto * 1_000_000;
    let base = costruisci(d.a, d.m, d.g, d.h, d.mi, d.s, 0)?;
    let utc = base.checked_sub(SignedDuration::from_secs(scarto_s - riporto)).map_err(|_| overflow())?;
    if !(1..=9999).contains(&(utc.year() as i64)) {
        return Err(overflow());
    }
    let tz = fuso_iana(zona)?;
    let locale = utc_in_fuso(utc, &tz)?;
    if !(1..=9999).contains(&(locale.year() as i64)) {
        return Err(overflow());
    }
    Ok(locale.date().at(locale.hour(), locale.minute(), locale.second(), (micro * 1000) as i32))
}
