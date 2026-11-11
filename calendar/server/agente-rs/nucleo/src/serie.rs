//! Le serie di eventi con le loro eccezioni, come le calcola recurring_ical_events 3.8.2 (Series, RecurrenceRules, ComponentAdapter): un
//! evento "nucleo" con le sue regole, le modifiche (RECURRENCE-ID) che prendono il posto di un'occorrenza, le date escluse e aggiunte. Ogni
//! scelta qui ha un'origine nella libreria Python, comprese le stranezze: le prove confrontano i risultati con le sue risposte.

use crate::errori::{Errore, Esito};
use crate::regola::{DopoUntil, IteraRegola, Naive, Regola, chiave_muro, leggi_regola};
use crate::tempo::{
    Ora, Tempo, aggiungi_giorni, come_data, come_ora, confronta, confronta_grande, confronta_t, differenza_t, id_ricorrenza, in_fuso, istante, ora, rendi_comparabili, somma, somma_t,
    testo_data, testo_py,
};
use indexmap::IndexMap;
use jiff::civil::DateTime;
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct Dati {
    pub uid: String,
    pub titolo: String,
    pub luogo: String,
    pub descrizione: String,
    pub organizzatore: String,
}

#[derive(Clone, Debug)]
pub enum FineODurata {
    Tempo(Tempo),
    Durata(f64),
}

#[derive(Clone, Debug)]
pub struct Rdate {
    pub inizio: Tempo,
    pub fine_o_durata: Option<FineODurata>,
}

/// Un VEVENT: le proprietà che servono al calcolo delle ricorrenze, più quelle che l'agente restituisce.
#[derive(Clone, Debug)]
pub struct Componente {
    pub uid: String,
    pub inizio: Tempo,
    pub fine_prop: Option<Tempo>,
    pub durata: Option<f64>,
    pub rec_id: Option<Tempo>,
    pub rec_range: bool,
    pub sequenza: i64,
    pub exdate: Vec<Tempo>,
    pub rdate: Vec<Rdate>,
    pub rrule: Vec<String>,
    pub ha_regole: bool,
    /// Gli errori di proprietà rotte, nell'ordine in cui il calcolo di Python le incontra: il primo esce quando si costruisce la serie.
    pub problemi: Vec<Errore>,
    pub dati: Dati,
}

#[derive(Clone, Debug)]
pub struct Occorrenza {
    pub c: Rc<Componente>,
    pub inizio: Tempo,
    pub fine: Tempo,
}

fn secondi_del_giorno(d: f64) -> f64 {
    ((d % 86400.0) + 86400.0) % 86400.0
}

fn dopo(t: &Tempo, secondi: f64) -> Esito<Tempo> {
    somma_t(t, secondi)
}

/// data - timedelta: a una data si tolgono solo i giorni interi della durata.
fn prima(t: &Tempo, secondi: f64) -> Esito<Tempo> {
    match t {
        Tempo::Data(d) => Ok(Tempo::Data(aggiungi_giorni(*d, -((secondi / 86400.0).floor() as i64))?)),
        Tempo::Ora(o) => Ok(Tempo::Ora(somma(o, -secondi)?)),
    }
}

/// time_span_contains_event: gli estremi finali sono esclusi, quelli iniziali compresi; un evento senza durata conta nell'istante in cui cade.
pub fn contiene_evento(inizio_finestra: &Tempo, fine_finestra: &Tempo, inizio_evento: &Tempo, fine_evento: &Tempo) -> Esito<bool> {
    let v = rendi_comparabili(&[inizio_finestra.clone(), fine_finestra.clone(), inizio_evento.clone(), fine_evento.clone()]);
    let (a, b, c, d) = (&v[0], &v[1], &v[2], &v[3]);
    if confronta_t(c, d)? > 0 {
        return Err(Errore::Valore(format!("the event must start before it ends(start: {} end: {})", testo_py(c)?, testo_py(d)?)));
    }
    if confronta_t(a, b)? > 0 {
        return Err(Errore::Valore(format!("the time span must start before it ends(start: {} end: {})", testo_py(a)?, testo_py(b)?)));
    }
    if confronta_t(c, d)? == 0 {
        if confronta_t(a, b)? == 0 {
            return Ok(confronta_t(c, a)? == 0);
        }
        return Ok(confronta_t(a, c)? <= 0 && confronta_t(c, b)? < 0);
    }
    if confronta_t(a, b)? == 0 {
        return Ok(confronta_t(c, a)? <= 0 && confronta_t(a, d)? < 0);
    }
    Ok(confronta_t(c, b)? < 0 && confronta_t(a, d)? < 0)
}

struct Adattatore {
    c: Rc<Componente>,
    span: OnceCell<Esito<(Tempo, Tempo)>>,
}

impl Adattatore {
    fn nuovo(c: Rc<Componente>) -> Self {
        Adattatore { c, span: OnceCell::new() }
    }

    fn fine_grezza(&self) -> Esito<Tempo> {
        let c = &self.c;
        if let Some(f) = &c.fine_prop {
            return Ok(f.clone());
        }
        if let Some(d) = c.durata {
            let inizio = if secondi_del_giorno(d) != 0.0 && matches!(c.inizio, Tempo::Data(_)) { Tempo::Ora(come_ora(&c.inizio, &None)) } else { c.inizio.clone() };
            return dopo(&inizio, d);
        }
        match &c.inizio {
            Tempo::Data(_) => dopo(&c.inizio, 86400.0),
            altro => Ok(altro.clone()),
        }
    }

    fn span(&self) -> Esito<(Tempo, Tempo)> {
        self.span
            .get_or_init(|| {
                let v = rendi_comparabili(&[self.c.inizio.clone(), self.fine_grezza()?]);
                let (inizio, fine) = (v[0].clone(), v[1].clone());
                Ok(if confronta_t(&inizio, &fine)? > 0 { (fine, inizio) } else { (inizio, fine) })
            })
            .clone()
    }

    fn inizio(&self) -> Esito<Tempo> {
        Ok(self.span()?.0)
    }

    fn fine(&self) -> Esito<Tempo> {
        Ok(self.span()?.1)
    }

    fn durata(&self) -> Esito<f64> {
        let (inizio, fine) = self.span()?;
        differenza_t(&fine, &inizio)
    }

    fn id_ric(&self) -> Esito<Vec<String>> {
        match &self.c.rec_id {
            Some(r) => id_ricorrenza(r),
            None => Ok(Vec::new()),
        }
    }

    fn e_modifica(&self) -> Esito<bool> {
        Ok(!self.id_ric()?.is_empty())
    }

    /// Quanto spostare le occorrenze: solo per RANGE=THISANDFUTURE, la differenza fra il nuovo inizio e RECURRENCE-ID.
    fn spostamento(&self) -> Esito<f64> {
        let Some(rec_id) = self.c.rec_range.then_some(()).and(self.c.rec_id.as_ref()) else { return Ok(0.0) };
        let v = rendi_comparabili(&[self.inizio()?, rec_id.clone()]);
        differenza_t(&v[0], &v[1])
    }

    /// Di quanto allargare la finestra delle regole per non perdere le occorrenze lunghe o spostate: (indietro, avanti).
    fn estensione(&self) -> Esito<(f64, f64)> {
        let mut indietro = self.durata()?;
        let mut avanti = 0.0;
        if let Some(rec_id) = &self.c.rec_id {
            let v = rendi_comparabili(&[self.inizio()?, self.fine()?, rec_id.clone()]);
            let (inizio, fine, id) = (&v[0], &v[1], &v[2]);
            if confronta_t(inizio, id)? < 0 {
                avanti = differenza_t(id, inizio)?;
            }
            if confronta_t(inizio, id)? > 0 {
                indietro = differenza_t(fine, id)?;
            }
        }
        Ok((indietro, avanti))
    }
}

fn piu_sequenza(a: Option<&Rc<Adattatore>>, b: &Rc<Adattatore>) -> Rc<Adattatore> {
    match a {
        Some(a) if b.c.sequenza <= a.c.sequenza => a.clone(),
        _ => b.clone(),
    }
}

struct RegolaUntil {
    regola: Rc<Regola>,
    until: Option<Tempo>,
}

/// "AAAAMMGG" o "AAAAMMGGTHHMMSS" come Temporal.PlainDateTime.from: un secondo 60 vale 59.
fn data_ora_da_testo(s: &str) -> Option<DateTime> {
    let n = |da: usize, a: usize| s.get(da..a).and_then(|x| x.parse::<i64>().ok());
    let (h, mi, sec) = if s.len() > 8 { (n(9, 11)?, n(11, 13)?, n(13, 15)?) } else { (0, 0, 0) };
    DateTime::new(i16::try_from(n(0, 4)?).ok()?, n(4, 6)? as i8, n(6, 8)? as i8, h as i8, mi as i8, sec.min(59) as i8, 0).ok()
}

/// RecurrenceRules: il nucleo della serie con le sue date escluse e aggiunte e le sue regole, tutto confrontabile fra loro.
struct Ricorrenze {
    inizio_orig: Tempo,
    fine_orig: Tempo,
    tz: Option<Rc<str>>,
    tutte_date: bool,
    inizio: Ora,
    chk_ora: HashSet<String>,
    chk_data: HashSet<String>,
    sostituisci_fine: HashMap<String, f64>,
    regole: Vec<RegolaUntil>,
    insieme: Vec<Ora>,
}

const MARGINE: i64 = 3 * 86400;

fn muro(o: &Ora) -> Naive {
    let w = &o.w;
    Naive { a: w.year() as i64, m: w.month() as i64, g: w.day() as i64, h: w.hour() as i64, mi: w.minute() as i64, s: w.second() as i64 }
}

fn come_ora_di(n: &Naive, tz: &Option<Rc<str>>) -> Esito<Ora> {
    let w = DateTime::new(n.a as i16, n.m as i8, n.g as i8, n.h as i8, n.mi as i8, n.s as i8, 0).map_err(|_| Errore::Altro("RangeError: date value out of range".to_string()))?;
    Ok(ora(w, tz.clone(), 0))
}

impl Ricorrenze {
    fn nuove(nucleo: &Adattatore) -> Esito<Self> {
        let inizio_orig = nucleo.inizio()?;
        let fine_orig = nucleo.fine()?;
        let mut chk_ora = HashSet::new();
        let mut chk_data = HashSet::new();
        for e in &nucleo.c.exdate {
            chk_ora.extend(id_ricorrenza(e)?);
            if let Tempo::Data(d) = e {
                chk_data.insert(testo_data(d));
            }
        }
        let mut rdate: Vec<Tempo> = Vec::new();
        let mut sostituisci_fine = HashMap::new();
        for r in &nucleo.c.rdate {
            rdate.push(r.inizio.clone());
            if let Some(fd) = &r.fine_o_durata {
                let d = match fd {
                    FineODurata::Durata(n) => *n,
                    FineODurata::Tempo(t) => {
                        let v = rendi_comparabili(&[t.clone(), r.inizio.clone()]);
                        differenza_t(&v[0], &v[1])?
                    }
                };
                for id in id_ricorrenza(&r.inizio)? {
                    sostituisci_fine.insert(id, d);
                }
            }
        }
        // make_all_dates_comparable: il fuso della serie è il primo che si trova fra inizio, fine, esclusioni e aggiunte
        let tutti: Vec<&Tempo> = [&inizio_orig, &fine_orig].into_iter().chain(nucleo.c.exdate.iter()).chain(rdate.iter()).collect();
        let tutte_date = !tutti.iter().any(|t| matches!(t, Tempo::Ora(_)));
        let tz: Option<Rc<str>> = tutti.iter().find_map(|t| match t {
            Tempo::Ora(o) => o.z.clone(),
            Tempo::Data(_) => None,
        });
        let inizio = come_ora(&inizio_orig, &tz);
        let aggiunte: Vec<Ora> = rdate.iter().map(|t| come_ora(t, &tz)).collect();
        let mut r = Ricorrenze { inizio_orig, fine_orig, tz, tutte_date, inizio, chk_ora, chk_data, sostituisci_fine, regole: Vec::new(), insieme: Vec::new() };

        let mut ultimo_until: Option<Tempo> = None;
        for testo in &nucleo.c.rrule {
            let regola = r.crea_regola(testo)?;
            if let Some(u) = &regola.until {
                let piu_tardi = match &ultimo_until {
                    None => true,
                    Some(ultimo) => confronta_grande(u, ultimo)?,
                };
                if piu_tardi {
                    ultimo_until = Some(u.clone());
                }
            }
            r.regole.push(regola);
        }
        r.insieme.extend(aggiunte);
        let aggiungi_inizio = match &ultimo_until {
            None => true,
            Some(u) => !confronta_grande(&Tempo::Ora(r.inizio.clone()), u)?,
        };
        if aggiungi_inizio {
            r.insieme.push(r.inizio.clone());
        }
        let mut errore: Option<Errore> = None;
        r.insieme.sort_by(|a, b| match confronta(a, b) {
            Ok(x) => x.cmp(&0),
            Err(e) => {
                errore.get_or_insert(e);
                std::cmp::Ordering::Equal
            }
        });
        if let Some(e) = errore {
            return Err(e);
        }
        Ok(r)
    }

    /// create_rule_with_start: UNTIL deve avere la stessa natura di DTSTART (con o senza fuso); se no si adatta, come fa la libreria.
    fn crea_regola(&self, testo: &str) -> Esito<RegolaUntil> {
        let inizio = muro(&self.inizio);
        let regola = match leggi_regola(testo, &inizio) {
            Ok(r) => r,
            Err(Errore::Valore(_)) => {
                // dateutil rifiuta la regola, si riprova con UNTIL adattato a DTSTART; senza UNTIL (o con due) è un errore
                let parti: Vec<&str> = testo.split(";UNTIL=").collect();
                if parti.len() != 2 {
                    return Err(Errore::Valore(format!("UNTIL parameter is missing: {testo}")));
                }
                let fine_data = parti[1].find(';').unwrap_or(parti[1].len());
                let mut until: String = parti[1][..fine_data].to_string();
                if self.tutte_date {
                    until = until.chars().take(8).collect();
                } else if self.tz.is_none() {
                    until.pop();
                } else {
                    if until.chars().count() == 8 {
                        until.push_str("T000000");
                    }
                    if until.chars().count() != 15 {
                        return Err(Errore::Valore(format!("UNTIL parameter has a bad format: {testo}")));
                    }
                    until.push('Z');
                }
                leggi_regola(&format!("{}{};UNTIL={until}", parti[0], &parti[1][fine_data..]), &inizio)?
            }
            Err(e) => return Err(e),
        };
        let mut until: Option<Tempo> = None;
        if let Some(u) = &regola.until {
            let errore = || Errore::Valore(format!("UNTIL non valido: {u}"));
            let parti = |s: &str| data_ora_da_testo(s).ok_or_else(errore);
            until = Some(if self.tz.is_some() {
                Tempo::Ora(ora(parti(if u.len() == 8 { u } else { &u[..15] })?, Some(Rc::from("UTC")), 0))
            } else if self.tutte_date {
                if u.len() == 15 { Tempo::Ora(ora(parti(u)?, None, 0)) } else { Tempo::Data(parti(&u[..8])?.date()) }
            } else if u.len() == 8 {
                Tempo::Data(parti(u)?.date())
            } else {
                Tempo::Ora(ora(parti(&u[..15])?, None, 0))
            });
        }
        Ok(RegolaUntil { regola: Rc::new(regola), until })
    }

    fn dopo_until(&self, r: &RegolaUntil) -> Option<DopoUntil<'static>> {
        let until = r.until.as_ref()?;
        let z = match until {
            Tempo::Ora(o) => o.z.clone(),
            Tempo::Data(_) => None,
        };
        let u = come_ora(until, &z);
        let tz = self.tz.clone();
        Some(Box::new(move |n: &Naive| Ok(confronta(&come_ora_di(n, &tz)?, &u)? > 0)))
    }

    /// La chiave sul muro di un estremo, nel fuso della serie: serve a saltare in fretta i risultati lontani dalla finestra.
    fn chiave_estremo(&self, o: &Ora) -> Esito<i64> {
        match &self.tz {
            Some(tz) if o.z.as_ref() != Some(tz) => Ok(chiave_muro(&muro(&in_fuso(istante(o)?, tz)?))),
            _ => Ok(chiave_muro(&muro(o))),
        }
    }

    /// Gli inizi della serie nella finestra, passati a `visita` finché dà `true`.
    fn tra(&self, inizio_finestra: &Tempo, fine_finestra: &Tempo, visita: &mut dyn FnMut(Ora) -> Esito<bool>) -> Esito<()> {
        let a = come_ora(inizio_finestra, &self.tz);
        let b = come_ora(fine_finestra, &self.tz);
        for x in &self.insieme {
            if confronta(x, &a)? >= 0 && confronta(x, &b)? <= 0 && !visita(x.clone())? {
                return Ok(());
            }
        }
        for r in &self.regole {
            let da = self.chiave_estremo(&a)? - MARGINE;
            let fino = self.chiave_estremo(&b)? + MARGINE;
            let mut iniziato = false;
            for n in IteraRegola::nuova(r.regola.clone(), &muro(&self.inizio), self.dopo_until(r)) {
                let n = n?;
                let k = chiave_muro(&n);
                if k < da {
                    continue;
                }
                if k > fino {
                    break;
                }
                let x = come_ora_di(&n, &self.tz)?;
                if confronta(&x, &b)? > 0 {
                    break;
                }
                if !iniziato {
                    if confronta(&x, &a)? < 0 {
                        continue;
                    }
                    iniziato = true;
                }
                let dentro = match &r.until {
                    None => true,
                    Some(u) => !confronta_grande(&Tempo::Ora(x.clone()), u)?,
                };
                if dentro && !visita(x)? {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Il valore come l'aveva scritto il calendario: una data se inizio e fine lo erano.
    fn nel_tipo_originale(&self, t: Ora) -> Tempo {
        if matches!(self.inizio_orig, Tempo::Data(_)) && matches!(self.fine_orig, Tempo::Data(_)) { Tempo::Data(t.w.date()) } else { Tempo::Ora(t) }
    }
}

/// Una serie: tutti i VEVENT con lo stesso UID.
pub struct Serie {
    modifiche: IndexMap<String, Rc<Adattatore>>,
    elenco_modifiche: Vec<Rc<Adattatore>>,
    da_qui_in_poi: Vec<String>,
    nucleo: Option<Rc<Adattatore>>,
    ricorrenze: Option<Ricorrenze>,
    indietro: f64,
    avanti: f64,
}

impl Serie {
    pub fn nuova(componenti: Vec<Rc<Componente>>) -> Esito<Self> {
        if componenti.is_empty() {
            return Err(Errore::Valore("No components given to calculate a series.".to_string()));
        }
        let mut modifiche: IndexMap<String, Rc<Adattatore>> = IndexMap::new();
        let mut da_qui_in_poi: Vec<String> = Vec::new();
        let mut nucleo: Option<Rc<Adattatore>> = None;
        for c in componenti {
            let ad = Rc::new(Adattatore::nuovo(c));
            if ad.e_modifica()? {
                let ids = ad.id_ric()?;
                for id in &ids {
                    let scelto = piu_sequenza(modifiche.get(id), &ad);
                    modifiche.insert(id.clone(), scelto);
                }
                if ad.c.rec_range {
                    da_qui_in_poi.push(ids[0].clone());
                }
            } else {
                nucleo = Some(piu_sequenza(nucleo.as_ref(), &ad));
            }
        }
        let mut elenco_modifiche: Vec<Rc<Adattatore>> = Vec::new();
        for m in modifiche.values() {
            if !elenco_modifiche.iter().any(|x| Rc::ptr_eq(x, m)) {
                elenco_modifiche.push(m.clone());
            }
        }
        da_qui_in_poi.sort();
        let ricorrenze = match &nucleo {
            Some(n) => Some(Ricorrenze::nuove(n)?),
            None => None,
        };
        let (mut indietro, mut avanti) = match &nucleo {
            Some(n) => n.estensione()?,
            None => (0.0, 0.0),
        };
        for id in &da_qui_in_poi {
            if let Some(m) = modifiche.get(id) {
                let (i, a) = m.estensione()?;
                indietro = indietro.max(i);
                avanti = avanti.max(a);
            }
        }
        Ok(Serie { modifiche, elenco_modifiche, da_qui_in_poi, nucleo, ricorrenze, indietro, avanti })
    }

    fn componente_per(&self, id: &str) -> Option<Rc<Adattatore>> {
        let mut componente = self.nucleo.clone();
        for m in &self.da_qui_in_poi {
            if m.as_str() < id {
                componente = self.modifiche.get(m).cloned();
            } else {
                break;
            }
        }
        componente
    }

    fn regole_tra(&self, inizio: &Tempo, fine: &Tempo, visita: &mut dyn FnMut(Ora) -> Esito<bool>) -> Esito<()> {
        match &self.ricorrenze {
            Some(r) => r.tra(&prima(inizio, self.indietro)?, &dopo(fine, self.avanti)?, visita),
            None => Ok(()),
        }
    }

    fn escluso(&self, ids: &[String]) -> bool {
        self.ricorrenze.as_ref().is_some_and(|r| ids.iter().any(|id| r.chk_ora.contains(id)))
    }

    /// Le occorrenze che si sovrappongono all'intervallo, nell'ordine in cui le dà la libreria.
    pub fn tra(&self, inizio: &Tempo, fine: &Tempo) -> Esito<Vec<Occorrenza>> {
        let mut out: Vec<Occorrenza> = Vec::new();
        let mut visti: Vec<Ora> = Vec::new();
        let mut modifiche_date: Vec<*const Adattatore> = Vec::new();
        self.regole_tra(inizio, fine, &mut |s: Ora| {
            let ids = id_ricorrenza(&Tempo::Ora(s.clone()))?;
            for v in &visti {
                if confronta(v, &s)? == 0 {
                    return Ok(true);
                }
            }
            let ricorrenze = self.ricorrenze.as_ref().expect("le regole vengono dal nucleo");
            if ricorrenze.chk_data.contains(&testo_data(&s.w.date())) || self.escluso(&ids) {
                return Ok(true);
            }
            let mut ad = self.nucleo.clone().expect("le regole vengono dal nucleo");
            for id in &ids {
                if let Some(m) = self.modifiche.get(id) {
                    ad = m.clone();
                }
            }
            let o = if self.nucleo.as_ref().is_some_and(|n| Rc::ptr_eq(n, &ad)) {
                visti.push(s.clone());
                let componente = self.componente_per(&ids[0]).expect("il nucleo c'è");
                let inizio_occ = somma(&s, componente.spostamento()?)?;
                let mut durata = componente.durata()?;
                for id in &ids {
                    if let Some(d) = ricorrenze.sostituisci_fine.get(id) {
                        durata = *d;
                    }
                }
                let fine_occ = somma(&inizio_occ, durata)?;
                Occorrenza { c: componente.c.clone(), inizio: ricorrenze.nel_tipo_originale(inizio_occ), fine: ricorrenze.nel_tipo_originale(fine_occ) }
            } else {
                if modifiche_date.contains(&Rc::as_ptr(&ad)) {
                    return Ok(true);
                }
                modifiche_date.push(Rc::as_ptr(&ad));
                Occorrenza { c: ad.c.clone(), inizio: ad.inizio()?, fine: ad.fine()? }
            };
            if contiene_evento(inizio, fine, &o.inizio, &o.fine)? {
                out.push(o);
            }
            Ok(true)
        })?;
        for m in &self.elenco_modifiche {
            if modifiche_date.contains(&Rc::as_ptr(m)) {
                continue;
            }
            let ids = m.id_ric()?;
            if self.escluso(&ids) || self.salta_modifica(m)? {
                continue;
            }
            let (mi, mf) = (m.inizio()?, m.fine()?);
            if contiene_evento(inizio, fine, &mi, &mf)? {
                modifiche_date.push(Rc::as_ptr(m));
                out.push(Occorrenza { c: m.c.clone(), inizio: mi, fine: mf });
            }
        }
        Ok(out)
    }

    /// skip_core_modification: una modifica con regole proprie e SEQUENCE più vecchia del nucleo vale solo se la regola del nucleo produce
    /// quella data.
    fn salta_modifica(&self, m: &Adattatore) -> Esito<bool> {
        if !m.c.ha_regole || !m.e_modifica()? {
            return Ok(false);
        }
        let sequenza_nucleo = self.nucleo.as_ref().map_or(-1, |n| n.c.sequenza);
        if m.c.sequenza >= sequenza_nucleo {
            return Ok(false);
        }
        // convert_to_date_range del primo id (l'istante UTC senza fuso): il giorno intero, letto nel fuso della serie
        let id_ric = m.id_ric()?;
        let w: DateTime = id_ric[0].parse().map_err(|_| Errore::Altro(format!("RangeError: invalid date: {}", id_ric[0])))?;
        let inizio = Tempo::Ora(ora(w.date().at(0, 0, 0, w.subsec_nanosecond()), None, 0));
        let fine = dopo(&inizio, 86400.0)?;
        let mut saltare = true;
        self.regole_tra(&inizio, &fine, &mut |s: Ora| {
            let ids = id_ricorrenza(&Tempo::Ora(s.clone()))?;
            let ricorrenze = self.ricorrenze.as_ref().expect("le regole vengono dal nucleo");
            if ricorrenze.chk_data.contains(&testo_data(&come_data(&Tempo::Ora(s)))) || self.escluso(&ids) {
                return Ok(true);
            }
            if ids.iter().any(|x| id_ric.contains(x)) {
                saltare = false;
                return Ok(false);
            }
            Ok(true)
        })?;
        Ok(saltare)
    }
}

/// Le occorrenze di tutte le serie di un calendario nell'intervallo, con le serie nell'ordine in cui compaiono i loro UID.
pub fn espandi(componenti: Vec<Componente>, inizio: &Tempo, fine: &Tempo) -> Esito<Vec<Occorrenza>> {
    let mut per_uid: IndexMap<String, Vec<Rc<Componente>>> = IndexMap::new();
    for c in componenti {
        per_uid.entry(c.uid.clone()).or_default().push(Rc::new(c));
    }
    // recurring_ical_events costruisce tutte le serie prima di chiedere la finestra: un errore di costruzione esce prima di ogni calcolo
    let mut serie = Vec::with_capacity(per_uid.len());
    for (_, lista) in per_uid {
        if let Some(p) = lista.iter().find_map(|c| c.problemi.first()) {
            return Err(p.clone());
        }
        serie.push(Serie::nuova(lista)?);
    }
    let mut out = Vec::new();
    for s in &serie {
        out.extend(s.tra(inizio, fine)?);
    }
    Ok(out)
}
