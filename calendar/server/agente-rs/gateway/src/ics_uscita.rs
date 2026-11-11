//! Il calendario che l'app pubblica per un altro OE Copilot, o per qualunque programma che legge un link iCalendar: il testo di RFC 5545
//! scritto qui (era lib/ics-uscita.ts). Due livelli: "occupato" (solo gli orari, ogni blocco si chiama Occupato) e "titoli" (titolo e luogo).
//! Il livello lo fa rispettare questo file: nel livello "occupato" titolo e luogo non entrano nel testo anche se arrivano.

use crate::fuso::{pezzi_utc, scarto_roma, utc_js, Naive};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;

pub const CRLF: &str = "\r\n";
pub const TESTO_OCCUPATO: &str = "Occupato";
/// Ogni quanto chi legge il link deve rileggerlo: 30 minuti.
pub const RILETTURA: &str = "PT30M";
const DOMINIO_UID: &str = "oe-copilot";

/// Un evento da pubblicare: con gli orari (istanti, millisecondi UTC) o di un giorno intero (date AAAA-MM-GG, la fine esclusa).
#[derive(Clone, Debug)]
pub enum EventoUscita {
    Ora { inizio: i64, fine: i64, titolo: String, luogo: String },
    Giorno { inizio: String, fine: String, titolo: String, luogo: String },
}

pub struct Pubblicazione<'a> {
    pub nome: &'a str,
    pub sale: &'a str,
    pub livello: &'a str,
    pub stamp: i64,
    pub eventi: &'a [EventoUscita],
}

/// Un valore di tipo TEXT (RFC 5545, 3.3.11): barra rovesciata, punto e virgola e virgola protetti, ogni a capo diventa \n, i caratteri di
/// controllo si tolgono.
pub fn testo_ics(valore: &str) -> String {
    let mut out = String::with_capacity(valore.len());
    let mut c = valore.chars().peekable();
    while let Some(x) = c.next() {
        match x {
            '\r' => {
                if c.peek() == Some(&'\n') {
                    c.next();
                }
                out.push_str("\\n");
            }
            '\n' | '\u{2028}' | '\u{2029}' | '\u{85}' => out.push_str("\\n"),
            '\t' => out.push(' '),
            '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => {}
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            x => out.push(x),
        }
    }
    out
}

/// Una riga logica in righe fisiche di al più 75 ottetti, CRLF escluso (RFC 5545, 3.1): un carattere non si spezza mai.
pub fn piega(riga: &str) -> String {
    let mut out = String::with_capacity(riga.len() + riga.len() / 70 * 3);
    let mut ottetti = 0;
    for c in riga.chars() {
        let n = c.len_utf8();
        if ottetti + n > 75 {
            out.push_str(CRLF);
            out.push(' ');
            ottetti = 1;
        }
        out.push(c);
        ottetti += n;
    }
    out
}

/// Un istante come data e ora UTC di iCalendar: 20261025T003000Z.
pub fn istante_ics(ms: i64) -> String {
    let (a, m, g, h, mi, s) = pezzi_utc(ms);
    format!("{a:04}{m:02}{g:02}T{h:02}{mi:02}{s:02}Z")
}

/// L'istante di un'ora di Roma scritta senza scarto: un'ora che c'è due volte vale la prima, una che non c'è vale con lo scarto di prima del
/// salto (datetime di Python con fold=0). Si prova lo scarto di ieri e quello di domani.
pub fn istante_da_roma(n: &Naive) -> i64 {
    let locale = utc_js(n.a, n.m - 1, n.g, n.h, n.mi, n.s);
    let prima = scarto_roma(locale - 86_400_000);
    let dopo = scarto_roma(locale + 86_400_000);
    let con_prima = locale - prima;
    if scarto_roma(con_prima) == prima {
        return con_prima;
    }
    let con_dopo = locale - dopo;
    if scarto_roma(con_dopo) == dopo {
        return con_dopo;
    }
    con_prima
}

/// L'identificativo di un evento: un HMAC del sale del link e dell'evento, tagliato.
pub fn uid_condiviso(sale: &str, parti: &[String]) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(sale.as_bytes()).expect("chiave di qualunque lunghezza");
    m.update(parti.join("\u{0}").as_bytes());
    let h: String = m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("{}@{DOMINIO_UID}", &h[..32])
}

fn evento(p: &Pubblicazione, e: &EventoUscita, visti: &mut HashMap<String, usize>, righe: &mut Vec<String>) {
    let occupato = p.livello == "occupato";
    let (tipo, inizio, fine, titolo, luogo) = match e {
        EventoUscita::Ora { inizio, fine, titolo, luogo } => ("ora", inizio.to_string(), fine.to_string(), titolo, luogo),
        EventoUscita::Giorno { inizio, fine, titolo, luogo } => ("giorno", inizio.clone(), fine.clone(), titolo, luogo),
    };
    let titolo = if occupato { TESTO_OCCUPATO } else { titolo.as_str() };
    let luogo = if occupato { "" } else { luogo.as_str() };
    // nel livello con i titoli due eventi diversi alla stessa ora non devono avere lo stesso identificativo
    let mut pezzi = vec![tipo.to_string(), inizio, fine];
    if !occupato {
        pezzi.push(titolo.to_string());
        pezzi.push(luogo.to_string());
    }
    let base = pezzi.join("\u{1}");
    let n = visti.get(&base).copied().unwrap_or(0);
    visti.insert(base.clone(), n + 1);
    righe.push("BEGIN:VEVENT".to_string());
    righe.push(format!("UID:{}", uid_condiviso(p.sale, &[base, n.to_string()])));
    righe.push(format!("DTSTAMP:{}", istante_ics(p.stamp)));
    match e {
        EventoUscita::Ora { inizio, fine, .. } => {
            righe.push(format!("DTSTART:{}", istante_ics(*inizio)));
            // senza durata la fine si omette: per iCalendar è un evento di durata zero
            if fine > inizio {
                righe.push(format!("DTEND:{}", istante_ics(*fine)));
            }
        }
        EventoUscita::Giorno { inizio, fine, .. } => {
            righe.push(format!("DTSTART;VALUE=DATE:{}", inizio.replace('-', "")));
            righe.push(format!("DTEND;VALUE=DATE:{}", fine.replace('-', "")));
        }
    }
    righe.push(format!("SUMMARY:{}", testo_ics(titolo)));
    if !luogo.is_empty() {
        righe.push(format!("LOCATION:{}", testo_ics(luogo)));
    }
    // un giorno intero non occupa l'orario, come nella disponibilità pubblica
    righe.push(format!("TRANSP:{}", if matches!(e, EventoUscita::Ora { .. }) { "OPAQUE" } else { "TRANSPARENT" }));
    righe.push("STATUS:CONFIRMED".to_string());
    righe.push("END:VEVENT".to_string());
}

/// Il testo del calendario, con le righe a capo CRLF e piegate a 75 ottetti.
pub fn costruisci_ics(p: &Pubblicazione) -> String {
    let mut righe: Vec<String> = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//OE Copilot//Condivisione//IT".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "METHOD:PUBLISH".to_string(),
        format!("X-WR-CALNAME:{}", testo_ics(p.nome)),
        format!("X-PUBLISHED-TTL:{RILETTURA}"),
        format!("REFRESH-INTERVAL;VALUE=DURATION:{RILETTURA}"),
        "X-OE-COPILOT-SHARE:1".to_string(),
    ];
    let mut visti = HashMap::new();
    for e in p.eventi {
        evento(p, e, &mut visti, &mut righe);
    }
    righe.push("END:VCALENDAR".to_string());
    righe.iter().map(|r| piega(r)).collect::<Vec<_>>().join(CRLF) + CRLF
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn testo_e_piega() {
        assert_eq!(testo_ics("a,b;c\\d\r\ne\tf\u{1}"), "a\\,b\\;c\\\\d\\ne f");
        let lunga = "è".repeat(50);
        let p = piega(&format!("SUMMARY:{lunga}"));
        assert!(p.split(CRLF).all(|r| r.len() <= 75), "{p}");
        assert_eq!(p.replace("\r\n ", ""), format!("SUMMARY:{lunga}"));
    }

    #[test]
    fn istanti() {
        assert_eq!(istante_ics(1_792_888_200_000), "20261025T003000Z");
        assert_eq!(istante_da_roma(&crate::fuso::leggi_naive("2026-10-25T02:30:00").unwrap()), 1_792_888_200_000);
        assert_eq!(istante_da_roma(&crate::fuso::leggi_naive("2026-03-29T02:30:00").unwrap()), 1_774_747_800_000);
    }
}
