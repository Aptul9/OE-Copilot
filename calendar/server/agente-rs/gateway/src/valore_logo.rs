//! Un valore di interfaccia.loghi (personalizzazione.json) letto come lo legge il browser prima di decidere se è un file di questo sito o un
//! indirizzo (era lib/valore-logo.ts). Il gateway e la pagina devono dare la stessa risposta.

/// Il valore come lo vede l'analizzatore degli indirizzi: via i caratteri di controllo e lo spazio ai lati, via tab e a capo dentro, e la
/// barra rovesciata prima di ? e # vale una barra.
pub fn come_lo_legge(valore: &str) -> String {
    let v = valore.trim_matches(|c: char| (c as u32) <= 0x20);
    let v: String = v.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    match v.find(['?', '#']) {
        None => v.replace('\\', "/"),
        Some(f) => format!("{}{}", v[..f].replace('\\', "/"), &v[f..]),
    }
}
