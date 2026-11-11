//! html.unescape di Python 3.12 e _pulisci_link dell'agente Python (tolto dal repository): le entità HTML5 con la tabella completa (entita.rs),
//! i riferimenti numerici con le regole dello standard (caratteri non validi, intervalli proibiti).

use crate::entita::html5;
use crate::errori::{Errore, Esito};
use regex::Regex;
use std::sync::LazyLock;

fn non_valido(n: u64) -> Option<&'static str> {
    Some(match n {
        0x00 => "\u{fffd}",
        0x0d => "\r",
        0x80 => "\u{20ac}",
        0x81 => "\u{81}",
        0x82 => "\u{201a}",
        0x83 => "\u{192}",
        0x84 => "\u{201e}",
        0x85 => "\u{2026}",
        0x86 => "\u{2020}",
        0x87 => "\u{2021}",
        0x88 => "\u{2c6}",
        0x89 => "\u{2030}",
        0x8a => "\u{160}",
        0x8b => "\u{2039}",
        0x8c => "\u{152}",
        0x8d => "\u{8d}",
        0x8e => "\u{17d}",
        0x8f => "\u{8f}",
        0x90 => "\u{90}",
        0x91 => "\u{2018}",
        0x92 => "\u{2019}",
        0x93 => "\u{201c}",
        0x94 => "\u{201d}",
        0x95 => "\u{2022}",
        0x96 => "\u{2013}",
        0x97 => "\u{2014}",
        0x98 => "\u{2dc}",
        0x99 => "\u{2122}",
        0x9a => "\u{161}",
        0x9b => "\u{203a}",
        0x9c => "\u{153}",
        0x9d => "\u{9d}",
        0x9e => "\u{17e}",
        0x9f => "\u{178}",
        _ => return None,
    })
}

fn codice_proibito(n: u64) -> bool {
    if (0x1..=0x8).contains(&n) || (0xe..=0x1f).contains(&n) || (0x7f..=0x9f).contains(&n) || (0xfdd0..=0xfdef).contains(&n) || n == 0xb {
        return true;
    }
    (n & 0xfffe) == 0xfffe && n <= 0x10ffff
}

static RIFERIMENTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\x0c <&#;]{1,32};?)").expect("regex valida"));

/// Python limita le cifre di un intero a 4300: oltre, int() solleva ValueError e l'errore esce dall'agente com'è.
const MAX_CIFRE: usize = 4300;

fn sostituisci(gruppo: &str, out: &mut String) -> Esito<()> {
    if gruppo.starts_with('#') {
        let esadecimale = matches!(gruppo.as_bytes().get(1), Some(b'x' | b'X'));
        let cifre = gruppo[if esadecimale { 2 } else { 1 }..].trim_end_matches(';');
        // il numero di cifre di JS conta unità UTF-16, ma qui sono tutte ASCII
        if !esadecimale && cifre.len() > MAX_CIFRE {
            return Err(Errore::Valore(format!(
                "Exceeds the limit ({MAX_CIFRE} digits) for integer string conversion: value has {} digits; use sys.set_int_max_str_digits() to increase the limit",
                cifre.len()
            )));
        }
        let senza_zeri = cifre.trim_start_matches('0');
        let significative = if senza_zeri.is_empty() && !cifre.is_empty() { &cifre[cifre.len() - 1..] } else { senza_zeri };
        let n: Option<u64> = if significative.len() > if esadecimale { 13 } else { 15 } {
            None
        } else {
            u64::from_str_radix(significative, if esadecimale { 16 } else { 10 }).ok()
        };
        match n {
            None => out.push('\u{fffd}'),
            Some(n) => {
                if let Some(s) = non_valido(n) {
                    out.push_str(s);
                } else if (0xd800..=0xdfff).contains(&n) || n > 0x10ffff {
                    out.push('\u{fffd}');
                } else if !codice_proibito(n) {
                    out.push(char::from_u32(n as u32).unwrap_or('\u{fffd}'));
                }
            }
        }
        return Ok(());
    }
    if let Some(intero) = html5(gruppo) {
        out.push_str(intero);
        return Ok(());
    }
    // il nome più lungo che esiste (come dice lo standard), contando i punti di codice
    let offset: Vec<usize> = gruppo.char_indices().map(|(i, _)| i).collect();
    for x in (2..offset.len()).rev() {
        if let Some(testa) = html5(&gruppo[..offset[x]]) {
            out.push_str(testa);
            out.push_str(&gruppo[offset[x]..]);
            return Ok(());
        }
    }
    out.push('&');
    out.push_str(gruppo);
    Ok(())
}

/// html.unescape().
pub fn html_unescape(s: &str) -> Esito<String> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut ultimo = 0;
    for c in RIFERIMENTO.captures_iter(s) {
        let tutto = c.get(0).expect("gruppo 0");
        out.push_str(&s[ultimo..tutto.start()]);
        sostituisci(&c[1], &mut out)?;
        ultimo = tutto.end();
    }
    out.push_str(&s[ultimo..]);
    Ok(out)
}

/// Il primo "%3c" (in una delle due scritture) seguito da http:// o https://: la copia protetta dei safelinks.
fn inizio_copia_protetta(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(2)).find(|&i| {
        b[i] == b'%' && b[i + 1] == b'3' && matches!(b[i + 2], b'C' | b'c') && (s[i + 3..].starts_with("http://") || s[i + 3..].starts_with("https://"))
    })
}

/// Link di riunione come lo si trova nella descrizione -> link usabile (_pulisci_link): "&amp;" letto come "&", la copia protetta dei safelinks tolta, la punteggiatura in coda tolta.
pub fn pulisci_link(url: &str) -> Esito<String> {
    let pulito = html_unescape(url)?;
    let testa = match inizio_copia_protetta(&pulito) {
        Some(i) => &pulito[..i],
        None => &pulito[..],
    };
    Ok(testa.trim_end_matches(['>', ')', '.', ',', ';', '"', '\'']).to_string())
}
