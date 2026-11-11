//! Come Python scrive, confronta e converte un valore: repr(), bool(), strip() e l'ordine delle stringhe. `json.dumps` è `Json::in_py`.

use crate::json::{Json, numero_js};
use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

/// I caratteri che str.strip() e \s di re considerano spazio (U+001C a U+001F e U+0085 sì, U+FEFF no), come classe di un'espressione regolare.
pub const SPAZI_PY: &str = r"\t\n\x0b\x0c\r\x1c-\x1f \u{85}\u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";

pub fn spazio_py(c: char) -> bool {
    matches!(c, '\t'..='\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// `str.strip()`.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(spazio_py)
}

/// Il nome del tipo di un valore JSON come lo scrive Python nei suoi errori ("'list' object has no attribute 'get'").
pub fn py_tipo(v: &Json) -> &'static str {
    match v {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Num(n) if n.is_finite() && n.fract() == 0.0 => "int",
        Json::Num(_) => "float",
        Json::Str(_) => "str",
        Json::Arr(_) => "list",
        Json::Obj(_) => "dict",
    }
}

/// `bool()` di un valore JSON.
pub fn py_bool(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Num(n) => *n != 0.0,
        Json::Str(s) => !s.is_empty(),
        Json::Arr(a) => !a.is_empty(),
        Json::Obj(m) => !m.is_empty(),
    }
}

static NON_STAMPABILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\p{Cc}\p{Cf}\p{Co}\p{Cn}\p{Zl}\p{Zp}\p{Zs}]$").expect("classe valida"));

/// `repr()` di una str: apici singoli, doppi se il testo ha un apice singolo e nessuno doppio, escape dei caratteri non stampabili.
pub fn py_repr_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        let c = ch as u32;
        if ch == q || ch == '\\' {
            out.push('\\');
            out.push(ch);
        } else if ch == '\n' {
            out.push_str("\\n");
        } else if ch == '\r' {
            out.push_str("\\r");
        } else if ch == '\t' {
            out.push_str("\\t");
        } else if c < 0x20 || c == 0x7f {
            let _ = write!(out, "\\x{c:02x}");
        } else if c < 0x7f {
            out.push(ch);
        } else if NON_STAMPABILE.is_match(ch.encode_utf8(&mut buf)) {
            let _ = if c <= 0xff {
                write!(out, "\\x{c:02x}")
            } else if c <= 0xffff {
                write!(out, "\\u{c:04x}")
            } else {
                write!(out, "\\U{c:08x}")
            };
        } else {
            out.push(ch);
        }
    }
    out.push(q);
    out
}

/// `repr()` di un valore JSON (liste, dizionari, None, True, False, numeri, stringhe).
pub fn py_repr(v: &Json) -> String {
    match v {
        Json::Null => "None".to_string(),
        Json::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Json::Num(n) => numero_js(*n),
        Json::Str(s) => py_repr_str(s),
        Json::Arr(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Json::Obj(m) => format!("{{{}}}", m.iter().map(|(k, x)| format!("{}: {}", py_repr_str(k), py_repr(x))).collect::<Vec<_>>().join(", ")),
    }
}

/// `repr()` di un timedelta di giorni, secondi e microsecondi già normalizzati (Python 3.12).
pub fn py_repr_delta(giorni: i64, secondi: i64, micro: i64) -> String {
    let mut parti: Vec<String> = Vec::new();
    if giorni != 0 {
        parti.push(format!("days={giorni}"));
    }
    if secondi != 0 {
        parti.push(format!("seconds={secondi}"));
    }
    if micro != 0 {
        parti.push(format!("microseconds={micro}"));
    }
    format!("datetime.timedelta({})", if parti.is_empty() { "0".to_string() } else { parti.join(", ") })
}

static CIFRA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{Nd}$").expect("classe valida"));

fn cifra_unicode(c: char) -> bool {
    let mut buf = [0u8; 4];
    CIFRA.is_match(c.encode_utf8(&mut buf))
}

/// float() di un valore JSON: numeri, booleani e stringhe che Python converte; il resto è un TypeError o un ValueError.
pub fn py_float(v: &Json) -> crate::errori::Esito<f64> {
    use crate::errori::Errore;
    let s = match v {
        Json::Num(n) => return Ok(*n),
        Json::Bool(b) => return Ok(if *b { 1.0 } else { 0.0 }),
        Json::Str(s) => s,
        _ => return Err(Errore::Tipo("float() argument must be a string or a real number".to_string())),
    };
    // Python legge anche le cifre decimali di ogni alfabeto (٣ è 3): ogni cifra torna a quella ASCII del suo gruppo di dieci
    let ascii: String = s
        .chars()
        .map(|c| {
            if c.is_ascii() || !cifra_unicode(c) {
                return c;
            }
            let mut inizio = c as u32;
            while inizio > 0 && char::from_u32(inizio - 1).is_some_and(cifra_unicode) {
                inizio -= 1;
            }
            char::from_digit((c as u32 - inizio) % 10, 10).unwrap_or(c)
        })
        .collect();
    let t = py_strip(&ascii);
    let senza_segno = t.strip_prefix(['+', '-']).unwrap_or(t).to_ascii_lowercase();
    if senza_segno == "inf" || senza_segno == "infinity" {
        return Ok(if t.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY });
    }
    if senza_segno == "nan" {
        return Ok(f64::NAN);
    }
    if NUMERO_PY.is_match(t) && t.split(['e', 'E']).next().is_some_and(|m| m.bytes().any(|b| b.is_ascii_digit())) {
        if let Ok(n) = t.replace('_', "").parse::<f64>() {
            return Ok(n);
        }
    }
    Err(Errore::Valore(format!("could not convert string to float: {}", py_repr_str(s))))
}

static NUMERO_PY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+-]?([0-9]+(_[0-9]+)*)?(\.([0-9]+(_[0-9]+)*)?)?([eE][+-]?[0-9]+(_[0-9]+)*)?$").expect("valida"));

/// Il confronto fra stringhe di Python, per punto di codice: è l'ordine dei byte UTF-8, quello di `str` in Rust.
pub fn confronta_py(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn strip_e_repr() {
        assert_eq!(py_strip("\u{1c}\u{85} a b\u{3000}\n"), "a b");
        assert_eq!(py_strip("\u{feff}a"), "\u{feff}a");
        assert_eq!(py_repr_str("it's"), "\"it's\"");
        assert_eq!(py_repr_str("a'b\"c\\\n\u{7f}\u{a0}\u{2028}\u{e000}è😀"), "'a\\'b\"c\\\\\\n\\x7f\\xa0\\u2028\\ue000è😀'");
        assert_eq!(py_repr(&Json::analizza(r#"{"a": [1, 2.5, null, true, "x"]}"#).unwrap()), "{'a': [1, 2.5, None, True, 'x']}");
        assert_eq!(py_repr_delta(0, 0, 0), "datetime.timedelta(0)");
        assert_eq!(py_repr_delta(-1, 3600, 5), "datetime.timedelta(days=-1, seconds=3600, microseconds=5)");
    }
}
