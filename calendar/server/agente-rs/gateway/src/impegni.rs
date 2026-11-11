//! Quello che la pagina pubblica (senza accesso) può sapere degli impegni: quando, mai cosa (era lib/core/impegni.ts). Da qui escono solo
//! orari di inizio e fine, e le festività (date pubbliche): né un titolo, né un luogo, né chi partecipa, né il nome di un calendario.

use crate::caselle::Ev;
use crate::fuso::{chiave, iso_secondi, mezzanotte, naive_locale, piu_giorni, Naive};
use crate::py::{py_bool, py_str, string_js, Esito};
use crate::stato::{confronta_utf16, nel_gruppo, Membri};
use oe_nucleo::json::Json;

/// Un giorno in unità di chiave (microsecondi).
const GIORNO: i64 = 86_400_000_000;

/// Un blocco voluto: un evento personale con "Blocca il tempo", o uno delle caselle segnato "Occupato" in Outlook o Google.
fn blocca(e: &Ev) -> bool {
    if py_bool(e.get("is_local")) {
        e.get("blocca") == Some(&Json::Bool(true))
    } else {
        e.get("occupato") == Some(&Json::Bool(true))
    }
}

/// Un giorno intero, con la fine esclusiva come in iCalendar: da mezzanotte del primo giorno a mezzanotte dopo l'ultimo. Una fine con l'ora
/// o non dopo l'inizio chiude il suo giorno.
fn giorni_interi(start: &str, end: &str) -> Esito<(Naive, Naive)> {
    let s = naive_locale(start)?;
    let f = naive_locale(end)?;
    let inizio = mezzanotte(s.a, s.m, s.g);
    let fine = mezzanotte(f.a, f.m, f.g);
    let fine = if chiave(&fine) != chiave(&f) || chiave(&fine) <= chiave(&inizio) { piu_giorni(&fine, 1) } else { fine };
    Ok((inizio, fine))
}

fn fine_o_inizio(e: &Ev) -> String {
    if py_bool(e.get("end")) {
        string_js(e.get("end"))
    } else {
        string_js(e.get("start"))
    }
}

/// [[inizio, fine], ...] (ISO, ora locale, al secondo): quando si è occupati nei `giorni` giorni da `primo_giorno`, in ordine. Con `aziende`
/// contano solo gli eventi di quelle aziende. Gli impegni che si sovrappongono o si toccano diventano un blocco solo. Un giorno intero o un
/// impegno da GIORNO in su conta solo se è un blocco.
pub fn impegni(eventi: &[Ev], primo_giorno: (i64, i64, i64), giorni: i64, aziende: Option<&Membri>) -> Vec<[String; 2]> {
    let da = mezzanotte(primo_giorno.0, primo_giorno.1, primo_giorno.2);
    let a = piu_giorni(&da, giorni);
    let mut trovati: Vec<(Naive, Naive)> = Vec::new();
    for e in eventi {
        if !py_bool(e.get("start")) {
            continue;
        }
        if let Some(m) = aziende {
            if !nel_gruppo(e, m) {
                continue;
            }
        }
        let blocco = blocca(e);
        if py_bool(e.get("is_local")) && e.get("blocca") == Some(&Json::Bool(false)) {
            continue;
        }
        let tutto_giorno = py_bool(e.get("allDay"));
        let coppia = if tutto_giorno {
            giorni_interi(&string_js(e.get("start")), &fine_o_inizio(e))
        } else {
            naive_locale(&string_js(e.get("start"))).and_then(|i| Ok((i, naive_locale(&fine_o_inizio(e))?)))
        };
        let Ok((inizio, fine)) = coppia else { continue };
        let durata = chiave(&fine) - chiave(&inizio);
        if (tutto_giorno || durata >= GIORNO) && !blocco {
            continue;
        }
        if durata > 0 && chiave(&fine) > chiave(&da) && chiave(&inizio) < chiave(&a) {
            trovati.push((inizio, fine));
        }
    }
    trovati.sort_by(|x, y| chiave(&x.0).cmp(&chiave(&y.0)).then(chiave(&x.1).cmp(&chiave(&y.1))));
    let mut uniti: Vec<(Naive, Naive)> = Vec::new();
    for (inizio, fine) in trovati {
        match uniti.last_mut() {
            Some(ultimo) if chiave(&inizio) <= chiave(&ultimo.1) => {
                if chiave(&fine) > chiave(&ultimo.1) {
                    ultimo.1 = fine;
                }
            }
            _ => uniti.push((inizio, fine)),
        }
    }
    uniti.iter().map(|(i, f)| [iso_secondi(i), iso_secondi(f)]).collect()
}

/// [[inizio, fine, nome], ...]: le festività, cioè gli eventi di un giorno intero del calendario `calendario`.
pub fn festivita(eventi: &[Ev], calendario: &str) -> Vec<[String; 3]> {
    let mut trovate: Vec<[String; 3]> = Vec::new();
    for e in eventi {
        if py_bool(e.get("allDay")) && e.get("account").and_then(Json::as_str) == Some(calendario) && py_bool(e.get("start")) {
            let fine = if py_bool(e.get("end")) { e.get("end") } else { e.get("start") };
            let titolo = if py_bool(e.get("title")) { py_str(e.get("title")) } else { String::new() };
            let voce = [py_str(e.get("start")), py_str(fine), titolo];
            if !trovate.contains(&voce) {
                trovate.push(voce);
            }
        }
    }
    trovate.sort_by(|x, y| confronta_utf16(&x[0], &y[0]).then_with(|| confronta_utf16(&x[1], &y[1])).then_with(|| confronta_utf16(&x[2], &y[2])));
    trovate
}
