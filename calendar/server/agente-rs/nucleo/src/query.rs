//! Il percorso e la query di una richiesta come li legge l'agente Python: self.path di http.server (che riduce "//" a "/") e urlparse e
//! parse_qs di urllib.parse 3.12 (il punto e virgola nell'ultimo tratto del percorso è un parametro, "+" nella query è uno spazio, un
//! valore vuoto non conta, vale il primo valore di un nome ripetuto).

use crate::errori::{Errore, Esito};
use indexmap::IndexMap;

const USA_PARAMETRI: [&str; 16] = ["", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu", "sip", "sips", "mms", "sftp", "tel"];

fn schema_valido(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
}

/// urlparse(self.path).path e .query. Un indirizzo che urlsplit rifiuta (ValueError) fa morire il gestore, come in Python.
pub fn analizza_target(target: &str) -> Esito<(String, String)> {
    let mut url: String = if target.starts_with("//") { format!("/{}", target.trim_start_matches('/')) } else { target.to_string() };
    url = url.trim_start_matches(|c: char| c <= ' ').to_string();
    url.retain(|c| !matches!(c, '\t' | '\r' | '\n'));
    let mut schema = String::new();
    if let Some(i) = url.find(':') {
        if i > 0 && url.as_bytes()[0].is_ascii_alphabetic() && schema_valido(&url[..i]) {
            schema = url[..i].to_ascii_lowercase();
            url = url[i + 1..].to_string();
        }
    }
    if url.starts_with("//") {
        let mut fine = url.len();
        for c in ['/', '?', '#'] {
            if let Some(k) = url[2..].find(c) {
                fine = fine.min(k + 2);
            }
        }
        let netloc = &url[2..fine];
        if (netloc.contains('[') && !netloc.contains(']')) || (netloc.contains(']') && !netloc.contains('[')) {
            return Err(Errore::SenzaRisposta("ValueError: Invalid IPv6 URL".to_string()));
        }
        url = url[fine..].to_string();
    }
    if let Some(c) = url.find('#') {
        url.truncate(c);
    }
    let mut query = String::new();
    if let Some(p) = url.find('?') {
        query = url[p + 1..].to_string();
        url.truncate(p);
    }
    if USA_PARAMETRI.contains(&schema.as_str()) && url.contains(';') {
        let k = match url.rfind('/') {
            Some(s) => url[s..].find(';').map(|x| x + s),
            None => url.find(';'),
        };
        if let Some(k) = k {
            url.truncate(k);
        }
    }
    Ok((url, query))
}

/// unquote(): le sequenze %XX valide diventano byte, lette poi come UTF-8 (un byte non valido diventa U+FFFD); i caratteri non ASCII restano.
fn unquote(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut resto = s;
    while !resto.is_empty() {
        let ascii = resto.as_bytes()[0] < 0x80;
        let fine = resto.find(|c: char| (c.is_ascii()) != ascii).unwrap_or(resto.len());
        let pezzo = &resto[..fine];
        resto = &resto[fine..];
        if !ascii {
            out.push_str(pezzo);
            continue;
        }
        let b = pezzo.as_bytes();
        let mut byte: Vec<u8> = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
                byte.push(u8::from_str_radix(&pezzo[i + 1..i + 3], 16).unwrap_or(0));
                i += 3;
            } else {
                byte.push(b[i]);
                i += 1;
            }
        }
        out.push_str(&String::from_utf8_lossy(&byte));
    }
    out
}

fn unquote_plus(s: &str) -> String {
    unquote(&s.replace('+', " "))
}

/// parse_qs(): nome -> valori, senza i valori vuoti.
pub fn parse_qs(qs: &str) -> IndexMap<String, Vec<String>> {
    let mut out: IndexMap<String, Vec<String>> = IndexMap::new();
    for campo in qs.split('&') {
        if campo.is_empty() {
            continue;
        }
        let i = campo.find('=');
        let valore = i.map_or("", |i| &campo[i + 1..]);
        if valore.is_empty() {
            continue;
        }
        let nome = unquote_plus(i.map_or(campo, |i| &campo[..i]));
        out.entry(nome).or_default().push(unquote_plus(valore));
    }
    out
}
