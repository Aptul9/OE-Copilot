//! base64 come lo usa Node: `Buffer.from(testo, "base64")` è permissivo (alfabeto normale e url insieme, i caratteri estranei si saltano,
//! il primo = chiude, un gruppo finale incompleto dà i byte che può), la scrittura è quella standard.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;

pub fn codifica(dati: &[u8]) -> String {
    STANDARD.encode(dati)
}

/// base64url senza "=" in fondo.
pub fn codifica_url(dati: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(dati)
}

fn valore(c: u8) -> Option<u32> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    } as u32)
}

/// `Buffer.from(testo, "base64")` (e "base64url", che per Node è lo stesso decodificatore).
pub fn decodifica_node(testo: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(testo.len() * 3 / 4);
    let mut accumulo = 0u32;
    let mut bit = 0;
    for &c in testo.as_bytes() {
        if c == b'=' {
            break;
        }
        let Some(v) = valore(c) else { continue };
        accumulo = (accumulo << 6) | v;
        bit += 6;
        if bit >= 8 {
            bit -= 8;
            out.push((accumulo >> bit) as u8);
            accumulo &= (1 << bit) - 1;
        }
    }
    out
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn come_buffer_di_node() {
        assert_eq!(decodifica_node("QUJD"), b"ABC");
        assert_eq!(decodifica_node("QQ"), b"A");
        assert_eq!(decodifica_node("QQ=="), b"A");
        assert_eq!(decodifica_node("Q"), b"");
        assert_eq!(decodifica_node("_-8"), vec![0xff, 0xef]);
        assert_eq!(decodifica_node("QU JD\n"), b"ABC");
        assert_eq!(codifica_url(&[0xfb, 0xff]), "-_8");
    }
}
