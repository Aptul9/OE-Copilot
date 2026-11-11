//! Le piccole operazioni di JavaScript che i moduli di rete usano sulle stringhe e sui valori: `trim`, `slice` in unita' UTF-16, `String(x)`,
//! `Number(x)`, la verita' di un valore e la decodifica base64 di `Buffer.from(s, "base64")`.

use oe_nucleo::json::{Json, numero_js};

/// I caratteri che `String.prototype.trim` toglie.
pub fn spazio_js(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// `s.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(spazio_js)
}

/// `s.length`.
pub fn lunghezza_utf16(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(0, n)`: un carattere tagliato a meta' resta mezza coppia surrogata, che scrivendo in UTF-8 diventa U+FFFD.
pub fn primi_utf16(s: &str, n: usize) -> String {
    let mut out = String::new();
    let mut unita = 0;
    for c in s.chars() {
        let l = c.len_utf16();
        if unita + l > n {
            if unita < n {
                out.push('\u{fffd}');
            }
            break;
        }
        out.push(c);
        unita += l;
    }
    out
}

/// `Boolean(valore)`.
pub fn vero(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Num(n) => *n != 0.0 && !n.is_nan(),
        Json::Str(s) => !s.is_empty(),
        Json::Arr(_) | Json::Obj(_) => true,
    }
}

/// `String(valore)`.
pub fn stringa_di(v: &Json) -> String {
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => numero_js(*n),
        Json::Str(s) => s.clone(),
        Json::Arr(a) => a.iter().map(|x| if x.is_null() { String::new() } else { stringa_di(x) }).collect::<Vec<_>>().join(","),
        Json::Obj(_) => "[object Object]".into(),
    }
}

/// `Number(valore)`.
pub fn numero_di(v: &Json) -> f64 {
    match v {
        Json::Null => 0.0,
        Json::Bool(b) => f64::from(u8::from(*b)),
        Json::Num(n) => *n,
        Json::Str(s) => numero_da_testo(s),
        Json::Arr(a) => match a.as_slice() {
            [] => 0.0,
            [x] => numero_di(&Json::Str(if x.is_null() { String::new() } else { stringa_di(x) })),
            _ => f64::NAN,
        },
        Json::Obj(_) => f64::NAN,
    }
}

/// `Number(stringa)`: vuota vale 0, 0x/0o/0b come interi, "Infinity", altrimenti un decimale scritto per intero.
pub fn numero_da_testo(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    let radice = |p: &str, r: u32| -> Option<f64> {
        let d = t.strip_prefix(p)?;
        if d.is_empty() || !d.chars().all(|c| c.is_digit(r)) {
            return Some(f64::NAN);
        }
        Some(d.chars().fold(0.0, |a, c| a * f64::from(r) + f64::from(c.to_digit(r).unwrap_or(0))))
    };
    for (p, r) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(n) = radice(p, r) {
            return n;
        }
    }
    let (segno, corpo) = match t.as_bytes()[0] {
        b'+' => (1.0, &t[1..]),
        b'-' => (-1.0, &t[1..]),
        _ => (1.0, t),
    };
    if corpo == "Infinity" {
        return segno * f64::INFINITY;
    }
    // Rust accetta "inf", "nan", "1_0" no, ma "infinity"/"nan" si': solo cifre, punto ed esponente passano
    let b = corpo.as_bytes();
    if b.is_empty() || !b.iter().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) || !b.iter().any(u8::is_ascii_digit) {
        return f64::NAN;
    }
    corpo.parse::<f64>().map_or(f64::NAN, |n| segno * n)
}

/// `Buffer.from(testo, "base64")`: l'alfabeto standard e quello URL insieme, i caratteri estranei saltati, si ferma al primo `=`.
pub fn buffer_base64(testo: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(testo.len() * 3 / 4);
    let (mut acc, mut bit) = (0u32, 0u32);
    for c in testo.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bit += 6;
        if bit >= 8 {
            bit -= 8;
            out.push((acc >> bit) as u8);
            acc &= (1 << bit) - 1;
        }
    }
    out
}

/// `/^[^\s@<escl>]{1,max_l}@[^\s@<escl>]{1,max_d}$/`: le lunghezze sono in unita' UTF-16 come quelle di una regex di JavaScript.
pub fn forma_indirizzo(s: &str, esclusi: &str, max_l: usize, max_d: usize) -> bool {
    let parte = |p: &str, max: usize| {
        let n = lunghezza_utf16(p);
        n >= 1 && n <= max && !p.chars().any(|c| c == '@' || spazio_js(c) || esclusi.contains(c))
    };
    s.split_once('@').is_some_and(|(l, d)| parte(l, max_l) && parte(d, max_d))
}

/// `s.split(/\s+/)`.
pub fn dividi_spazi(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut inizio = 0;
    let mut in_spazi: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if spazio_js(c) {
            in_spazi.get_or_insert(i);
        } else if let Some(da) = in_spazi.take() {
            out.push(&s[inizio..da]);
            inizio = i;
        }
    }
    match in_spazi {
        Some(da) => {
            out.push(&s[inizio..da]);
            out.push("");
        }
        None => out.push(&s[inizio..]),
    }
    out
}

/// `s.replace(/[\x00-\x1f\x7f]/g, " ")`.
pub fn controlli_in_spazi(s: &str) -> String {
    s.chars().map(|c| if (c as u32) < 0x20 || c as u32 == 0x7f { ' ' } else { c }).collect()
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn utf16() {
        assert_eq!(lunghezza_utf16("a\u{1f600}"), 3);
        assert_eq!(primi_utf16("ab\u{1f600}c", 3), "ab\u{fffd}");
        assert_eq!(primi_utf16("ab\u{1f600}c", 4), "ab\u{1f600}");
        assert_eq!(primi_utf16("abc", 10), "abc");
    }

    #[test]
    fn numeri() {
        assert_eq!(numero_da_testo(" 12 "), 12.0);
        assert_eq!(numero_da_testo(""), 0.0);
        assert_eq!(numero_da_testo("0x10"), 16.0);
        assert!(numero_da_testo("inf").is_nan());
        assert!(numero_da_testo("nan").is_nan());
        assert!(numero_da_testo("1e").is_nan());
        assert_eq!(numero_da_testo("-1.5e2"), -150.0);
    }

    #[test]
    fn base64() {
        assert_eq!(buffer_base64("aGk"), b"hi");
        assert_eq!(buffer_base64("aGk="), b"hi");
        assert_eq!(buffer_base64("a-_"), vec![0x6b, 0xef]);
        assert_eq!(js_trim("\u{feff} x\u{a0}"), "x");
    }
}
