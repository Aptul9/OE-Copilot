//! Un VTIMEZONE come lo interpreta Python: icalendar lo riscrive in testo (proprietà in ordine alfabetico) e dateutil (tz.tzical) lo legge. Le
//! righe di quel testo le prepara ics.rs; qui c'è la lettura di tzical con i suoi errori e il suo modo di dare lo scarto a un'ora sul muro,
//! comprese le ore che non esistono e quelle che si ripetono (non è il modo di zoneinfo: nel buco dell'ora legale vale lo scarto nuovo, la
//! prima ora ripetuta ha lo scarto vecchio).

use crate::errori::{Errore, Esito};
use crate::fusi::FusoPersonalizzato;
use crate::regola::{IteraRegola, Naive, chiave_muro, leggi_regola, trim_js};
use crate::tempo::fuso_esistente;
use jiff::civil::DateTime;
use regex::Regex;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigaTz {
    pub nome: String,
    /// I parametri come escono nel testo: "CHIAVE=valore".
    pub param: Vec<String>,
    pub valore: String,
}

static NAIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]{4})([0-9]{2})([0-9]{2})(?:T([0-9]{2})([0-9]{2})([0-9]{2}))?Z?$").expect("valida"));

/// dateutil.parser.parse(testo, ignoretz=True) per quello che un VTIMEZONE scrive: una data, o una data con l'ora; la Z si scarta.
fn leggi_naive(testo: &str) -> Esito<Naive> {
    let errore = || Errore::Valore(format!("Unknown string format: {testo}"));
    let maiuscolo = trim_js(testo).to_uppercase();
    let m = NAIVE.captures(&maiuscolo).ok_or_else(errore)?;
    let num = |i: usize| m.get(i).map_or(0, |x| x.as_str().parse::<i64>().unwrap_or(0));
    let n = Naive { a: num(1), m: num(2), g: num(3), h: num(4), mi: num(5), s: num(6) };
    DateTime::new(n.a as i16, n.m as i8, n.g as i8, n.h as i8, n.mi as i8, n.s as i8, 0).map_err(|_| errore())?;
    Ok(n)
}

type Flusso = Box<dyn Iterator<Item = Esito<i64>>>;

/// I cambi di una osservanza, in ordine, calcolati quando servono e ricordati (rrule con cache=True).
struct Cambi {
    memoria: Vec<i64>,
    finito: bool,
    flusso: Flusso,
}

impl Cambi {
    /// L'ultimo cambio che cade a `muro` o prima (rrule.before(dt, inc=True)), o None se non ce n'è. Un errore del flusso esce una volta:
    /// dopo, il flusso è chiuso e valgono i cambi già visti.
    fn prima(&mut self, muro: i64) -> Esito<Option<i64>> {
        while !self.finito && self.memoria.last().is_none_or(|u| *u <= muro) {
            match self.flusso.next() {
                None => self.finito = true,
                Some(Ok(v)) => self.memoria.push(v),
                Some(Err(e)) => return Err(e),
            }
        }
        let i = self.memoria.partition_point(|x| *x <= muro);
        Ok(if i == 0 { None } else { Some(self.memoria[i - 1]) })
    }
}

/// Le chiavi sul muro dei risultati di una regola, fino a UNTIL.
struct ChiaviRegola {
    regola: Option<(Rc<crate::regola::Regola>, Naive)>,
    it: Option<IteraRegola<'static>>,
}

impl Iterator for ChiaviRegola {
    type Item = Esito<i64>;
    fn next(&mut self) -> Option<Esito<i64>> {
        if let Some((r, inizio)) = self.regola.take() {
            let fine = match &r.until {
                Some(u) => match leggi_naive(u) {
                    Ok(n) => Some(chiave_muro(&n)),
                    Err(e) => return Some(Err(e)),
                },
                None => None,
            };
            let dopo = fine.map(|f| Box::new(move |x: &Naive| Ok(chiave_muro(x) > f)) as crate::regola::DopoUntil<'static>);
            self.it = Some(IteraRegola::nuova(r, &inizio, dopo));
        }
        let it = self.it.as_mut()?;
        let v = it.next()?;
        Some(v.map(|n| chiave_muro(&n)))
    }
}

/// Più flussi crescenti in uno, senza ripetere un valore (rruleset).
struct Unione {
    flussi: Vec<Flusso>,
    teste: Option<Vec<Option<i64>>>,
    ultimo: Option<i64>,
    finito: bool,
}

impl Unione {
    fn nuova(flussi: Vec<Flusso>) -> Self {
        Unione { flussi, teste: None, ultimo: None, finito: false }
    }

    fn passo(&mut self) -> Esito<Option<i64>> {
        if self.teste.is_none() {
            let mut teste = Vec::with_capacity(self.flussi.len());
            for f in &mut self.flussi {
                teste.push(f.next().transpose()?);
            }
            self.teste = Some(teste);
        }
        loop {
            let teste = self.teste.as_mut().expect("impostate sopra");
            let mut scelto: Option<usize> = None;
            for (i, t) in teste.iter().enumerate() {
                if let Some(v) = t {
                    if scelto.is_none_or(|s| *v < teste[s].unwrap_or(i64::MAX)) {
                        scelto = Some(i);
                    }
                }
            }
            let Some(s) = scelto else { return Ok(None) };
            let v = teste[s].unwrap_or_default();
            teste[s] = self.flussi[s].next().transpose()?;
            if self.ultimo != Some(v) {
                self.ultimo = Some(v);
                return Ok(Some(v));
            }
        }
    }
}

impl Iterator for Unione {
    type Item = Esito<i64>;
    fn next(&mut self) -> Option<Esito<i64>> {
        if self.finito {
            return None;
        }
        let esito = self.passo().transpose();
        if !matches!(esito, Some(Ok(_))) {
            self.finito = true;
        }
        esito
    }
}

/// I cambi tolti quelli di EXDATE ed EXRULE.
struct Escludi {
    flusso: Flusso,
    exdate: HashSet<i64>,
    exrule: Option<Flusso>,
    con_fuso: bool,
    avviato: bool,
    testa: Option<i64>,
    finito: bool,
}

impl Escludi {
    fn tira_exrule(&mut self) -> Esito<()> {
        self.testa = match &mut self.exrule {
            Some(f) => f.next().transpose()?,
            None => None,
        };
        Ok(())
    }

    fn passo(&mut self) -> Esito<Option<i64>> {
        if !self.avviato {
            self.avviato = true;
            if self.con_fuso {
                return Err(Errore::Tipo("can't compare offset-naive and offset-aware datetimes".to_string()));
            }
            self.tira_exrule()?;
        }
        while let Some(v) = self.flusso.next().transpose()? {
            while self.testa.is_some_and(|t| t < v) {
                self.tira_exrule()?;
            }
            if self.testa == Some(v) || self.exdate.contains(&v) {
                continue;
            }
            return Ok(Some(v));
        }
        Ok(None)
    }
}

impl Iterator for Escludi {
    type Item = Esito<i64>;
    fn next(&mut self) -> Option<Esito<i64>> {
        if self.finito {
            return None;
        }
        let esito = self.passo().transpose();
        if !matches!(esito, Some(Ok(_))) {
            self.finito = true;
        }
        esito
    }
}

struct Osservanza {
    a: i64,
    diff: i64,
    dst: bool,
    cambi: RefCell<Cambi>,
}

/// _tzicalvtz: un fuso fatto di osservanze STANDARD e DAYLIGHT.
pub struct ZonaVtz {
    osservanze: Vec<Osservanza>,
}

impl ZonaVtz {
    fn trova(&self, muro: i64, fold: u8) -> Esito<&Osservanza> {
        if self.osservanze.len() == 1 {
            return Ok(&self.osservanze[0]);
        }
        let mut ultimo_cambio: Option<i64> = None;
        let mut ultima: Option<&Osservanza> = None;
        for o in &self.osservanze {
            let cercato = if o.diff < 0 && fold != 0 { muro - o.diff } else { muro };
            let cambio = o.cambi.borrow_mut().prima(cercato)?;
            if let Some(c) = cambio {
                if ultimo_cambio.is_none_or(|u| u < c) {
                    ultimo_cambio = Some(c);
                    ultima = Some(o);
                }
            }
        }
        match ultima {
            Some(o) => Ok(o),
            // prima del primo cambio: la prima osservanza standard; se non ce n'è Python inciampa (comp[0] su un oggetto che non si indicizza)
            None => self.osservanze.iter().find(|o| !o.dst).ok_or_else(|| Errore::Tipo("'_tzicalvtzcomp' object is not subscriptable".to_string())),
        }
    }
}

impl FusoPersonalizzato for ZonaVtz {
    fn scarto(&self, muro: i64, fold: u8) -> Esito<i64> {
        Ok(self.trova(muro, fold)?.a)
    }

    fn dst(&self, muro: i64, fold: u8) -> Esito<i64> {
        let o = self.trova(muro, fold)?;
        Ok(if o.dst { o.diff } else { 0 })
    }
}

/// tzical._parse_offset.
fn leggi_scarto(testo: &str) -> Esito<i64> {
    let mut s = trim_js(testo);
    if s.is_empty() {
        return Err(Errore::Valore("empty offset".to_string()));
    }
    let mut segno = 1;
    if let Some(resto) = s.strip_prefix('+') {
        s = resto;
    } else if let Some(resto) = s.strip_prefix('-') {
        segno = -1;
        s = resto;
    }
    let num = |t: &str| -> Esito<i64> {
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Errore::Valore(format!("invalid literal for int() with base 10: '{t}'")));
        }
        Ok(t.parse::<i64>().unwrap_or(i64::MAX))
    };
    let c: Vec<char> = s.chars().collect();
    let pezzo = |da: usize, a: usize| c[da..a].iter().collect::<String>();
    match c.len() {
        4 => Ok((num(&pezzo(0, 2))? * 3600 + num(&pezzo(2, 4))? * 60) * segno),
        6 => Ok((num(&pezzo(0, 2))? * 3600 + num(&pezzo(2, 4))? * 60 + num(&pezzo(4, 6))?) * segno),
        _ => Err(Errore::Valore(format!("invalid offset: {s}"))),
    }
}

/// rrulestr._parse_date_value: un elenco di date e ore separate da virgola. Con TZID di un fuso che esiste i valori hanno il fuso e non si
/// confrontano con i cambi, che non ce l'hanno: Python dà TypeError quando li usa; con un TZID che non trova li lascia senza fuso. I parametri
/// nei messaggi sono in maiuscolo, rrulestr maiuscolizza tutto il testo prima.
fn leggi_valori_data(valore: &str, param: &[String]) -> Esito<(Vec<Naive>, bool)> {
    let mut trovato = false;
    let mut con_fuso = false;
    for p in param {
        let maiuscolo = p.to_uppercase();
        if maiuscolo.starts_with("TZID=") {
            if fuso_esistente(p.get(5..).unwrap_or_default()) {
                con_fuso = true;
            }
            continue;
        }
        if maiuscolo != "VALUE=DATE-TIME" && maiuscolo != "VALUE=DATE" {
            return Err(Errore::Valore(format!("unsupported parm: {maiuscolo}")));
        }
        if trovato {
            return Err(Errore::Valore(format!("Multiple VALUE parameters specified: {}", param.join(";").to_uppercase())));
        }
        trovato = true;
    }
    Ok((valore.split(',').map(leggi_naive).collect::<Esito<Vec<_>>>()?, con_fuso))
}

/// rrulestr(righe, compatible=True, ignoretz=True, cache=True): l'insieme dei cambi di una osservanza.
fn leggi_cambi(righe: &[RigaTz]) -> Esito<Cambi> {
    let mut regole: Vec<&str> = Vec::new();
    let mut rdate: Vec<&str> = Vec::new();
    let mut exregole: Vec<&str> = Vec::new();
    let mut exdate: Vec<Naive> = Vec::new();
    let mut exdate_con_fuso = false;
    let mut dtstart: Option<Naive> = None;
    for r in righe {
        match r.nome.as_str() {
            "RRULE" => {
                if let Some(p) = r.param.first() {
                    return Err(Errore::Valore(format!("unsupported RRULE parm: {}", p.to_uppercase())));
                }
                regole.push(&r.valore);
            }
            "RDATE" => {
                if let Some(p) = r.param.iter().find(|p| p.to_uppercase() != "VALUE=DATE-TIME") {
                    return Err(Errore::Valore(format!("unsupported RDATE parm: {}", p.to_uppercase())));
                }
                rdate.push(&r.valore);
            }
            "EXRULE" => {
                if let Some(p) = r.param.first() {
                    return Err(Errore::Valore(format!("unsupported EXRULE parm: {}", p.to_uppercase())));
                }
                exregole.push(&r.valore);
            }
            "EXDATE" => {
                let (date, con_fuso) = leggi_valori_data(&r.valore, &r.param)?;
                exdate.extend(date);
                exdate_con_fuso = exdate_con_fuso || con_fuso;
            }
            "DTSTART" => {
                let (v, _) = leggi_valori_data(&r.valore, &r.param)?;
                if v.len() != 1 {
                    return Err(Errore::Valore(format!("Multiple DTSTART values specified:{}", r.valore)));
                }
                dtstart = Some(v[0]);
            }
            altro => return Err(Errore::Valore(format!("unsupported property: {altro}"))),
        }
    }
    let Some(inizio) = dtstart else { return Err(Errore::Valore("Unsupported rule set without DTSTART".to_string())) };
    let mut flussi: Vec<Flusso> = Vec::new();
    for t in &regole {
        flussi.push(Box::new(ChiaviRegola { regola: Some((Rc::new(leggi_regola(t, &inizio)?), inizio)), it: None }));
    }
    let mut esplicite: Vec<i64> = Vec::new();
    for v in &rdate {
        for d in v.split(',') {
            esplicite.push(chiave_muro(&leggi_naive(d)?));
        }
    }
    esplicite.push(chiave_muro(&inizio));
    esplicite.sort_unstable();
    esplicite.dedup();
    flussi.push(Box::new(esplicite.into_iter().map(Ok)));
    let mut esclusi: Vec<Flusso> = Vec::new();
    for t in &exregole {
        esclusi.push(Box::new(ChiaviRegola { regola: Some((Rc::new(leggi_regola(t, &inizio)?), inizio)), it: None }));
    }
    let exrule: Option<Flusso> = if esclusi.is_empty() { None } else { Some(Box::new(Unione::nuova(esclusi))) };
    let flusso = Escludi {
        flusso: Box::new(Unione::nuova(flussi)),
        exdate: exdate.iter().map(chiave_muro).collect(),
        exrule,
        con_fuso: exdate_con_fuso,
        avviato: false,
        testa: None,
        finito: false,
    };
    Ok(Cambi { memoria: Vec::new(), finito: false, flusso: Box::new(flusso) })
}

/// tzical(testo).get(): il fuso di un VTIMEZONE, o l'errore con il testo di Python.
pub fn crea_zona_da_righe(righe: &[RigaTz]) -> Esito<ZonaVtz> {
    if righe.is_empty() {
        return Err(Errore::Valore("empty string".to_string()));
    }
    let mut tzid: Option<&str> = None;
    let mut osservanze: Vec<Osservanza> = Vec::new();
    let mut dentro = false;
    let mut tipo: Option<&str> = None;
    let mut trovato_inizio = false;
    let mut da: Option<i64> = None;
    let mut a: Option<i64> = None;
    let mut righe_cambi: Vec<RigaTz> = Vec::new();
    let mut finito = false;
    let valore = |s: &str| Errore::Valore(s.to_string());
    for r in righe {
        if dentro {
            if r.nome == "BEGIN" {
                if r.valore != "STANDARD" && r.valore != "DAYLIGHT" {
                    return Err(valore(&format!("unknown component: {}", r.valore)));
                }
                tipo = Some(&r.valore);
                trovato_inizio = false;
                da = None;
                a = None;
                righe_cambi = Vec::new();
            } else if r.nome == "END" {
                if r.valore == "VTIMEZONE" {
                    if let Some(t) = tipo.filter(|t| !t.is_empty()) {
                        return Err(valore(&format!("component not closed: {t}")));
                    }
                    if tzid.is_none_or(str::is_empty) {
                        return Err(valore("mandatory TZID not found"));
                    }
                    if osservanze.is_empty() {
                        return Err(valore("at least one component is needed"));
                    }
                    dentro = false;
                    finito = true;
                } else if Some(r.valore.as_str()) == tipo {
                    if !trovato_inizio {
                        return Err(valore("mandatory DTSTART not found"));
                    }
                    let (Some(da), Some(a)) = (da, a) else { return Err(valore("mandatory TZOFFSETFROM not found")) };
                    osservanze.push(Osservanza { a, diff: a - da, dst: tipo == Some("DAYLIGHT"), cambi: RefCell::new(leggi_cambi(&righe_cambi)?) });
                    tipo = None;
                } else {
                    return Err(valore(&format!("invalid component end: {}", r.valore)));
                }
            } else if tipo.is_some_and(|t| !t.is_empty()) {
                match r.nome.as_str() {
                    "DTSTART" => {
                        if let Some(p) = r.param.iter().find(|p| *p != "VALUE=DATE-TIME") {
                            return Err(valore(&format!("Unsupported DTSTART param in VTIMEZONE: {p}")));
                        }
                        righe_cambi.push(r.clone());
                        trovato_inizio = true;
                    }
                    "RRULE" | "RDATE" | "EXRULE" | "EXDATE" => righe_cambi.push(r.clone()),
                    "TZOFFSETFROM" => {
                        if let Some(p) = r.param.first() {
                            return Err(valore(&format!("unsupported {} parm: {p} ", r.nome)));
                        }
                        da = Some(leggi_scarto(&r.valore)?);
                    }
                    "TZOFFSETTO" => {
                        if let Some(p) = r.param.first() {
                            return Err(valore(&format!("unsupported TZOFFSETTO parm: {p}")));
                        }
                        a = Some(leggi_scarto(&r.valore)?);
                    }
                    "TZNAME" => {
                        if let Some(p) = r.param.first() {
                            return Err(valore(&format!("unsupported TZNAME parm: {p}")));
                        }
                    }
                    "COMMENT" => {}
                    altro => return Err(valore(&format!("unsupported property: {altro}"))),
                }
            } else if r.nome == "TZID" {
                if let Some(p) = r.param.first() {
                    return Err(valore(&format!("unsupported TZID parm: {p}")));
                }
                tzid = Some(&r.valore);
            } else if r.nome != "TZURL" && r.nome != "LAST-MODIFIED" && r.nome != "COMMENT" {
                return Err(valore(&format!("unsupported property: {}", r.nome)));
            }
        } else if r.nome == "BEGIN" && r.valore == "VTIMEZONE" {
            tzid = None;
            osservanze = Vec::new();
            dentro = true;
        }
    }
    if !finito {
        return Err(valore("no timezones defined"));
    }
    Ok(ZonaVtz { osservanze })
}
