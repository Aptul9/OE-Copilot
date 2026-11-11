//! I nomi e i loghi degli account Microsoft come li mostra il calendario, mandati dal gateway (POST /api/auth/shared/labels, chiave piena):
//! l'agente li tiene su disco e li ridà con l'elenco degli account (GET /api/auth/shared), così TeamsRelay mostra gli stessi del calendario.
//!   {"names": {"<indirizzo>": "NPO"}, "logos": {"NPO": "data:image/png;base64,..."}, "forwarded": {"NPO": "Bandpey"}}
//! forwarded: le aziende le cui riunioni arrivano copiate in un'altra casella, con il nome di quella casella.

use crate::js::{js_trim, lunghezza_utf16, primi_utf16};
use oe_nucleo::json::{Json, Mappa};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

const MAX_VOCI: usize = 128;
const MAX_LOGO: usize = 300_000;
static DATA_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^data:image/(?:png|jpeg|gif|webp|svg\+xml|vnd\.microsoft\.icon|x-icon);base64,[A-Za-z0-9+/]+=*$").expect("valida"));

#[derive(Debug, Clone, PartialEq)]
pub struct ErroreEtichette(pub String);

fn rifiuta<T>(m: impl Into<String>) -> Result<T, ErroreEtichette> {
    Err(ErroreEtichette(m.into()))
}

fn nome(v: &Json) -> Result<String, ErroreEtichette> {
    match v {
        Json::Str(s) if !js_trim(s).is_empty() && lunghezza_utf16(s) <= 64 && !s.chars().any(|c| (c as u32) < 0x20 || c == '\x7f') => Ok(js_trim(s).to_string()),
        _ => rifiuta("nome non valido"),
    }
}

fn email(e: &str) -> bool {
    let vietato = |c: char| crate::js::spazio_js(c) || c == '@';
    let Some((l, d)) = e.split_once('@') else { return false };
    !l.chars().any(vietato) && !d.chars().any(vietato) && (1..=128).contains(&lunghezza_utf16(l)) && (1..=253).contains(&lunghezza_utf16(d))
}

fn mappa(v: Option<&Json>, campo: &str, chiave: impl Fn(&str) -> Result<String, ErroreEtichette>, valore: impl Fn(&Json) -> Result<String, ErroreEtichette>) -> Result<Mappa, ErroreEtichette> {
    let Some(v) = v else { return Ok(Mappa::new()) };
    let Json::Obj(m) = v else { return rifiuta(format!("{campo}: serve un oggetto")) };
    if m.len() > MAX_VOCI {
        return rifiuta(format!("{campo}: al massimo {MAX_VOCI} voci"));
    }
    let mut out = Mappa::new();
    for (k, x) in m.iter() {
        out.insert(chiave(k)?, valore(x)?);
    }
    Ok(out)
}

/// Il corpo del gateway, controllato: indirizzi, nomi brevi, loghi solo come immagini in data URL. Nessun altro campo.
pub fn leggi_etichette(corpo: &Json) -> Result<Json, ErroreEtichette> {
    let Json::Obj(r) = corpo else { return rifiuta("serve {\"names\", \"logos\", \"forwarded\"}") };
    let estranei: Vec<&str> = r.keys().map(String::as_str).filter(|k| !["names", "logos", "forwarded"].contains(k)).collect();
    if !estranei.is_empty() {
        return rifiuta(format!("campi non ammessi: {}", primi_utf16(&estranei.join(", "), 80)));
    }
    let names = mappa(
        r.get("names"),
        "names",
        |k| {
            let e = js_trim(k).to_lowercase();
            if !email(&e) {
                return rifiuta("names: indirizzo non valido");
            }
            Ok(e)
        },
        nome,
    )?;
    let logos = mappa(
        r.get("logos"),
        "logos",
        |k| nome(&Json::from(k)),
        |x| match x {
            Json::Str(s) if lunghezza_utf16(s) <= MAX_LOGO && DATA_URL.is_match(s) => Ok(s.clone()),
            _ => rifiuta("logos: serve un'immagine in data URL"),
        },
    )?;
    let forwarded = mappa(r.get("forwarded"), "forwarded", |k| nome(&Json::from(k)), nome)?;
    let mut m = Mappa::new();
    m.insert("names", Json::Obj(names));
    m.insert("logos", Json::Obj(logos));
    m.insert("forwarded", Json::Obj(forwarded));
    Ok(Json::Obj(m))
}

/// Le ultime etichette salvate, o nessuna se il file manca o non è valido.
pub fn carica_etichette(file: &Path) -> Option<Json> {
    let testo = crate::file::leggi_utf8(file).ok()?;
    leggi_etichette(&Json::analizza(&testo).ok()?).ok()
}

pub fn salva_etichette(file: &Path, e: &Json) -> Result<(), oe_nucleo::errori::Errore> {
    crate::file::scrivi_atomico(file, &e.in_js(), Some(0o600))
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn etichette_controllate() {
        let j = |t: &str| Json::analizza(t).unwrap();
        let e = leggi_etichette(&j(r#"{"names":{" A@B.it ":" NPO "},"logos":{"NPO":"data:image/png;base64,AAA="}}"#)).unwrap();
        assert_eq!(e.in_js(), r#"{"names":{"a@b.it":"NPO"},"logos":{"NPO":"data:image/png;base64,AAA="},"forwarded":{}}"#);
        assert_eq!(leggi_etichette(&j(r#"{"x":1}"#)).unwrap_err().0, "campi non ammessi: x");
        assert_eq!(leggi_etichette(&j(r#"{"logos":{"A":"http://x"}}"#)).unwrap_err().0, "logos: serve un'immagine in data URL");
        assert_eq!(leggi_etichette(&j(r#"{"names":[]}"#)).unwrap_err().0, "names: serve un oggetto");
    }
}
