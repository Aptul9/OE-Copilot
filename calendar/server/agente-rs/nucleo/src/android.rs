//! Le righe di calendar.db del telefono Android rimesse in serie e espanse: quello che l'agente Python faceva fra la query SQL e l'espansione
//! (_quando, la durata "P900S", le colonne RRULE/RDATE/EXDATE, le occorrenze annullate che diventano EXDATE, le modifiche).

use crate::errori::{Errore, Esito};
use crate::ics::{FUSO_LOCALE, Riga, Tzid, leggi_elenco_tempi, leggi_proprieta, leggi_rdate, leggi_riga_ics, parametro, regola_per_python};
use crate::pyfmt::{py_repr_str, py_strip};
use crate::regola::trim_js;
use crate::serie::{Componente, Dati, Rdate, espandi};
use crate::tempo::{Tempo, data_da_giorni, errore_rotta, fuso_esistente, in_fuso, in_ora_locale, ora, testo_naive};
use jiff::civil::DateTime;
use regex::Regex;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

/// Le colonne di Events che servono all'espansione (la riga com'è nel database, già senza gli eventi cancellati).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RigaAndroid {
    pub id: i64,
    pub sync_id: Option<String>,
    pub title: Option<String>,
    pub event_location: Option<String>,
    pub description: Option<String>,
    pub event_status: Option<i64>,
    pub dtstart: Option<i64>,
    pub dtend: Option<i64>,
    pub event_timezone: Option<String>,
    pub event_end_timezone: Option<String>,
    pub duration: Option<String>,
    pub all_day: Option<i64>,
    pub rrule: Option<String>,
    pub rdate: Option<String>,
    pub exrule: Option<String>,
    pub exdate: Option<String>,
    pub original_id: Option<i64>,
    pub original_sync_id: Option<String>,
    pub original_instance_time: Option<i64>,
    pub original_all_day: Option<i64>,
    /// Come li scrive il telefono (1 e 0); le prove e i database vecchi possono non averli.
    pub is_organizer: Option<i64>,
    pub has_attendee_data: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccorrenzaAndroid {
    pub uid: String,
    pub titolo: String,
    pub luogo: String,
    pub descrizione: String,
    pub inizio: String,
    pub fine: String,
    pub tutto_giorno: bool,
}

/// Events.STATUS_CANCELED
const ANNULLATO: i64 = 2;

/// La riga è annullata. Outlook scrive STATUS_CANCELED anche per le occorrenze delle serie che sono copie fatte da un flusso nel calendario
/// di chi le legge (`isOrganizer` 1 e nessun invitato, `hasAttendeeData` 0): per un'eccezione di quel tipo lo stato non conta. `eccezione` è
/// vero per una riga che sostituisce un'occorrenza di una serie; una riga sola con lo stesso stato resta annullata.
pub fn annullata(event_status: Option<i64>, is_organizer: Option<i64>, has_attendee_data: Option<i64>, eccezione: bool) -> bool {
    event_status == Some(ANNULLATO) && !(eccezione && is_organizer == Some(1) && has_attendee_data == Some(0))
}

fn annullata_riga(r: &RigaAndroid, eccezione: bool) -> bool {
    annullata(r.event_status, r.is_organizer, r.has_attendee_data, eccezione)
}

fn fuori_limite() -> Errore {
    Errore::Altro("RangeError: date value out of range".to_string())
}

/// Istante di Android (millisecondi UTC + nome del fuso) -> data/ora: i giorni interi a mezzanotte UTC sono date, un fuso che non c'è è UTC.
fn quando(ms: i64, fuso: Option<&str>, tutto_il_giorno: bool) -> Esito<Tempo> {
    if !(-8_640_000_000_000_000..=8_640_000_000_000_000).contains(&ms) {
        return Err(fuori_limite());
    }
    if tutto_il_giorno {
        return Ok(Tempo::Data(data_da_giorni(ms.div_euclid(86_400_000))?));
    }
    match fuso {
        Some(f) if !f.is_empty() && f != "UTC" && fuso_esistente(f) => Ok(Tempo::Ora(in_fuso(ms, f)?)),
        _ => Ok(Tempo::Ora(in_fuso(ms, "UTC")?)),
    }
}

static DURATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([+-])?P(?:([0-9]+)W)?(?:([0-9]+)D)?T?(?:([0-9]+)H)?(?:([0-9]+)M)?(?:([0-9]+)S)?$").expect("valida"));

/// La durata di Android in secondi: scrive anche "P900S", senza la T che iCalendar vuole davanti a ore, minuti e secondi.
pub fn durata_android(testo: Option<&str>) -> Esito<f64> {
    let t = py_strip(testo.unwrap_or_default()).to_uppercase();
    let Some(m) = DURATA.captures(&t) else {
        return Err(Errore::Valore(format!("durata non riconosciuta: {}", testo.map_or_else(|| "None".to_string(), py_repr_str))));
    };
    let n = |i: usize| m.get(i).map_or(0.0, |x| x.as_str().parse::<f64>().unwrap_or(0.0));
    let s = n(2) * 604800.0 + n(3) * 86400.0 + n(4) * 3600.0 + n(5) * 60.0 + n(6);
    Ok(if m.get(1).is_some_and(|x| x.as_str() == "-") { -s } else { s })
}

/// Una colonna di ricorrenza (RRULE, RDATE...) come riga iCalendar: EXDATE e RDATE possono arrivare come "TZID=Europe/Rome;2026..." Una riga
/// che non si legge si salta, come fa Event.from_ical.
fn colonna(nome: &str, valore: Option<&str>) -> Esito<Option<Riga>> {
    let v = py_strip(valore.unwrap_or_default());
    if v.is_empty() {
        return Ok(None);
    }
    let testo = match v.rfind(';') {
        Some(i) if nome != "RRULE" && nome != "EXRULE" => format!("{nome};{}:{}", &v[..i], &v[i + 1..]),
        _ => format!("{nome}:{v}"),
    };
    match leggi_riga_ics(&testo) {
        Ok(r) => Ok(Some(r)),
        Err(Errore::Valore(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn non_vuoto(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|x| !x.is_empty())
}

fn da_righe(righe: &[RigaAndroid]) -> Esito<Vec<Componente>> {
    let mut per_sync: HashMap<&str, i64> = HashMap::new();
    for r in righe {
        if let Some(s) = r.sync_id.as_deref().filter(|s| !s.is_empty()) {
            per_sync.insert(s, r.id);
        }
    }
    let mut per_id: HashMap<i64, &RigaAndroid> = HashMap::new();
    for r in righe {
        per_id.insert(r.id, r);
    }
    let mut annullate: HashMap<i64, Vec<(i64, Option<i64>)>> = HashMap::new();
    let mut componenti: Vec<Componente> = Vec::new();
    for r in righe {
        let serie = r
            .original_id
            .filter(|x| *x != 0)
            .or_else(|| r.original_sync_id.as_deref().filter(|s| !s.is_empty()).and_then(|s| per_sync.get(s).copied()))
            .filter(|x| *x != 0);
        if let (Some(s), true, Some(t)) = (serie, annullata_riga(r, true), r.original_instance_time) {
            annullate.entry(s).or_default().push((t, r.original_all_day));
            continue;
        }
        let Some(dtstart) = r.dtstart.filter(|_| !annullata_riga(r, serie.is_some())) else { continue };
        let giorno = r.all_day.is_some_and(|x| x != 0);
        let fuso = r.event_timezone.as_deref();
        let mut rdate: Vec<Rdate> = Vec::new();
        let mut exdate: Vec<Tempo> = Vec::new();
        let mut regole: Vec<String> = Vec::new();
        let mut ha_regole = false;
        let mut problemi: Vec<Errore> = Vec::new();
        if serie.is_none() {
            let rr = colonna("RRULE", r.rrule.as_deref())?;
            if let Some(rr) = &rr {
                regole = vec![regola_per_python(&rr.valore)];
            }
            let rd = colonna("RDATE", r.rdate.as_deref())?;
            colonna("EXRULE", r.exrule.as_deref())?;
            let ex = colonna("EXDATE", r.exdate.as_deref())?;
            // una proprietà che non si legge resta rotta e l'errore esce quando il calcolo la usa: prima EXDATE, poi RDATE
            if let Some(ex) = &ex {
                match leggi_proprieta(ex, "vDDDLists", "dts", || leggi_elenco_tempi(&ex.valore, Tzid::da(parametro(ex, "TZID"))))? {
                    Err(rotta) => problemi.push(errore_rotta(&rotta)),
                    Ok(e) => exdate = e,
                }
            }
            if let Some(rd) = &rd {
                match leggi_proprieta(rd, "vDDDLists", "dts", || leggi_rdate(&rd.valore, Tzid::da(parametro(rd, "TZID"))))? {
                    Err(rotta) => problemi.push(errore_rotta(&rotta)),
                    Ok(d) => rdate = d,
                }
            }
            let pieno = |s: &Option<String>| s.as_deref().is_some_and(|x| !trim_js(x).is_empty());
            ha_regole = rr.is_some() || pieno(&r.rdate) || pieno(&r.exdate);
        }
        let fine_fuso = r.event_end_timezone.as_deref().filter(|s| !s.is_empty()).or(fuso);
        let mut fine_prop = match r.dtend {
            Some(e) => Some(quando(e, fine_fuso, giorno)?),
            None => None,
        };
        let mut durata = match (r.dtend, non_vuoto(&r.duration)) {
            (None, true) => Some(durata_android(r.duration.as_deref())?),
            _ => None,
        };
        // Le copie con lo stato annullato (annullata() le lascia passare) hanno spesso la fine uguale all'inizio: la durata vera è quella della serie
        if let Some(s) = serie {
            let senza_fine = match r.dtend {
                None => !non_vuoto(&r.duration),
                Some(e) => e <= dtstart,
            };
            if r.event_status == Some(ANNULLATO) && senza_fine {
                let secondi = match per_id.get(&s) {
                    None => 0.0,
                    Some(m) => match (m.dtend, m.dtstart) {
                        (Some(e), Some(i)) => ((e - i) as f64 / 1000.0).round(),
                        _ if non_vuoto(&m.duration) => durata_android(m.duration.as_deref())?,
                        _ => 0.0,
                    },
                };
                if secondi > 0.0 {
                    fine_prop = None;
                    durata = Some(secondi);
                }
            }
        }
        let uid = format!("a{}", serie.unwrap_or(r.id));
        let inizio = quando(dtstart, fuso, giorno)?;
        let rec_id = match (serie, r.original_instance_time) {
            (Some(_), Some(t)) => Some(quando(t, fuso, r.original_all_day.is_some_and(|x| x != 0))?),
            _ => None,
        };
        let testo = |s: &Option<String>| s.clone().unwrap_or_default();
        componenti.push(Componente {
            uid: uid.clone(),
            inizio,
            fine_prop,
            durata,
            rec_id,
            rec_range: false,
            sequenza: -1,
            exdate,
            rdate,
            rrule: regole,
            ha_regole,
            problemi,
            dati: Dati { uid, titolo: testo(&r.title), luogo: testo(&r.event_location), descrizione: testo(&r.description), organizzatore: String::new() },
        });
    }
    // Le occorrenze annullate di una serie diventano EXDATE della serie
    for c in &mut componenti {
        let Some(id) = c.uid.strip_prefix('a').filter(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit())).and_then(|x| x.parse::<i64>().ok()) else { continue };
        if c.rrule.is_empty() {
            continue;
        }
        for (ms, giorno) in annullate.get(&id).map(Vec::as_slice).unwrap_or_default() {
            let z = match &c.inizio {
                Tempo::Ora(o) => o.z.as_deref().filter(|z| *z != "UTC").map(str::to_string),
                Tempo::Data(_) => None,
            };
            c.exdate.push(quando(*ms, z.as_deref(), giorno.is_some_and(|x| x != 0))?);
            c.ha_regole = true;
        }
    }
    Ok(componenti)
}

/// Le occorrenze degli eventi del telefono fra due date locali senza fuso, prima di togliere i doppioni. Un errore in una serie fa fallire
/// tutto l'intervallo, come in Python.
pub fn espandi_righe_android(righe: &[RigaAndroid], inizio: DateTime, fine: DateTime, fuso: Option<&str>) -> Esito<Vec<OccorrenzaAndroid>> {
    let fuso = fuso.unwrap_or(FUSO_LOCALE);
    let componenti = da_righe(righe)?;
    let z: Rc<str> = Rc::from(fuso);
    let da = Tempo::Ora(ora(inizio, Some(z.clone()), 0));
    let a = Tempo::Ora(ora(fine, Some(z), 0));
    espandi(componenti, &da, &a)?
        .into_iter()
        .map(|o| {
            Ok(OccorrenzaAndroid {
                uid: o.c.dati.uid.clone(),
                titolo: o.c.dati.titolo.clone(),
                luogo: o.c.dati.luogo.clone(),
                descrizione: o.c.dati.descrizione.clone(),
                inizio: testo_naive(&in_ora_locale(&o.inizio, fuso, true)?),
                fine: testo_naive(&in_ora_locale(&o.fine, fuso, true)?),
                tutto_giorno: matches!(o.inizio, Tempo::Data(_)),
            })
        })
        .collect()
}
