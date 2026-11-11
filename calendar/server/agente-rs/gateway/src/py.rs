//! Le poche regole di Python su cui il servizio ui faceva affidamento senza dirlo (str() e bool() di un valore JSON), e gli errori del gateway
//! con il testo che la pagina mostra (era lib/core/py.ts).

use oe_nucleo::json::{numero_js, Json};
use std::collections::BTreeSet;
use std::fmt;

/// Lo stato con cui un sito ha risposto, la sua frase e l'indirizzo che ha risposto (rete_sicura.rs).
#[derive(Clone, Debug, PartialEq)]
pub struct DettaglioHttp {
    pub stato: u16,
    pub testo: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motivo {
    NonValido,
    NonAmmesso,
    NonRisponde,
    Certificato,
    ErroreHttp,
    Reindirizzamenti,
    TroppoGrande,
}

/// Gli errori del gateway. `Valore` e `Rete` sono il ValueError di Python: il testo è quello che la pagina mostra.
#[derive(Clone, Debug, PartialEq)]
pub enum Errore {
    Valore(String),
    Rete { motivo: Motivo, messaggio: String, http: Option<DettaglioHttp> },
    /// L'agente non si raggiunge (AgenteNonDisponibile).
    Agente(String),
    /// Una data che Python rifiuta (ErroreData).
    Data(String),
    /// Uno stato HTTP da 400 in su di un calendario da link (ErroreHttp di ics.ts).
    Http { stato: u16, messaggio: String },
    /// Un argomento mancante o di un altro tipo (ErroreArgomenti di api.ts).
    Argomenti(String),
    /// TypeError
    Tipo(String),
    /// Qualunque altro Error, con il suo nome per il log.
    Altro { nome: &'static str, messaggio: String },
}

pub type Esito<T> = Result<T, Errore>;

impl Errore {
    pub fn valore(m: impl Into<String>) -> Self {
        Errore::Valore(m.into())
    }

    pub fn altro(m: impl Into<String>) -> Self {
        Errore::Altro { nome: "Error", messaggio: m.into() }
    }

    pub fn tipo(m: impl Into<String>) -> Self {
        Errore::Tipo(m.into())
    }

    /// `messaggio(e)`: il testo dell'errore.
    pub fn messaggio(&self) -> &str {
        match self {
            Errore::Valore(m) | Errore::Agente(m) | Errore::Data(m) | Errore::Argomenti(m) | Errore::Tipo(m) => m,
            Errore::Rete { messaggio, .. } | Errore::Http { messaggio, .. } | Errore::Altro { messaggio, .. } => messaggio,
        }
    }

    /// `e instanceof ErroreValore`
    pub fn e_valore(&self) -> bool {
        matches!(self, Errore::Valore(_) | Errore::Rete { .. })
    }

    /// Il nome della classe, come `e.name` o `constructor.name` di JavaScript.
    pub fn nome(&self) -> &'static str {
        match self {
            Errore::Tipo(_) => "TypeError",
            Errore::Altro { nome, .. } => nome,
            _ => "Error",
        }
    }
}

impl fmt::Display for Errore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.messaggio())
    }
}

impl From<rusqlite::Error> for Errore {
    fn from(e: rusqlite::Error) -> Self {
        Errore::Altro { nome: "SqliteError", messaggio: e.to_string() }
    }
}

/// str(valore) per i valori di un file JSON: True, False, None come in Python. `None` è undefined.
pub fn py_str(v: Option<&Json>) -> String {
    match v {
        None | Some(Json::Null) => "None".to_string(),
        Some(Json::Bool(true)) => "True".to_string(),
        Some(Json::Bool(false)) => "False".to_string(),
        Some(Json::Str(s)) => s.clone(),
        Some(Json::Num(n)) => numero_js(*n),
        Some(x) => x.in_js(),
    }
}

pub fn py_str_v(v: &Json) -> String {
    py_str(Some(v))
}

/// bool(valore): vuoto e zero sono falsi.
pub fn py_bool(v: Option<&Json>) -> bool {
    match v {
        None | Some(Json::Null) | Some(Json::Bool(false)) => false,
        Some(Json::Num(n)) => *n != 0.0,
        Some(Json::Str(s)) => !s.is_empty(),
        Some(Json::Arr(a)) => !a.is_empty(),
        Some(Json::Obj(m)) => !m.is_empty(),
        Some(Json::Bool(true)) => true,
    }
}

pub fn py_bool_v(v: &Json) -> bool {
    py_bool(Some(v))
}

pub fn is_dict(v: Option<&Json>) -> bool {
    matches!(v, Some(Json::Obj(_)))
}

/// Gli spazi per str.strip() di Python: il White_Space di Unicode e i separatori \x1c-\x1f (non il BOM, che trim() di JavaScript toglie).
fn spazio_py(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// str.strip()
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(spazio_py)
}

/// Gli spazi di String.prototype.trim(): WhiteSpace e LineTerminator di ECMAScript (con il BOM, senza \x85).
pub fn spazio_js(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// String.prototype.trim()
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(spazio_js)
}

pub fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(spazio_js)
}

/// Il nome del tipo di un valore JSON come Python lo scrive negli errori.
pub fn tipo_py(v: Option<&Json>) -> &'static str {
    match v {
        None | Some(Json::Null) => "NoneType",
        Some(Json::Bool(_)) => "bool",
        Some(Json::Num(n)) => {
            if n.fract() == 0.0 && n.is_finite() {
                "int"
            } else {
                "float"
            }
        }
        Some(Json::Str(_)) => "str",
        Some(Json::Arr(_)) => "list",
        Some(Json::Obj(_)) => "dict",
    }
}

/// (valore or "").strip(): un valore falso vale "", un testo si ripulisce, qualunque altra cosa è l'AttributeError di Python.
pub fn testo_pulito(v: Option<&Json>) -> Esito<String> {
    if !py_bool(v) {
        return Ok(String::new());
    }
    match v {
        Some(Json::Str(s)) => Ok(py_strip(s).to_string()),
        _ => Err(Errore::tipo(format!("'{}' object has no attribute 'strip'", tipo_py(v)))),
    }
}

/// Un elenco di testi: gli elementi di una lista, str() di ciascuno; qualunque altra cosa è vuota.
pub fn elenco_testi(v: Option<&Json>) -> Vec<String> {
    match v {
        Some(Json::Arr(a)) => a.iter().map(py_str_v).collect(),
        _ => Vec::new(),
    }
}

/// {str(v).strip().lower() per ogni voce non vuota}: l'insieme di nomi in minuscolo di personalizzazione.json.
pub fn minuscole(v: Option<&Json>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(Json::Arr(a)) = v {
        for x in a {
            let s = py_str_v(x);
            let s = js_trim(&s);
            if !s.is_empty() {
                out.insert(minuscolo_js(s));
            }
        }
    }
    out
}

/// toLowerCase() di JavaScript: la mappatura di Unicode senza le regole della lingua, come to_lowercase di Rust.
pub fn minuscolo_js(s: &str) -> String {
    s.to_lowercase()
}

/// Quanti caratteri conta `length` di JavaScript (unità UTF-16).
pub fn lunghezza_js(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.slice(0, n)` di JavaScript, in unità UTF-16; una coppia surrogata tagliata a metà lascia fuori il suo carattere (in Rust non sta da sola).
pub fn taglia_js(s: &str, n: usize) -> &str {
    let mut unita = 0;
    for (i, c) in s.char_indices() {
        let l = c.len_utf16();
        if unita + l > n {
            return &s[..i];
        }
        unita += l;
    }
    s
}

/// Il valore di un oggetto JSON, `None` se non c'è o se il valore non è un oggetto (undefined).
pub fn campo<'a>(v: Option<&'a Json>, nome: &str) -> Option<&'a Json> {
    v.and_then(|x| x.get(nome))
}

/// Gli elementi di una lista, vuoto per qualunque altra cosa (`Array.isArray(v) ? v : []`).
pub fn lista(v: Option<&Json>) -> &[Json] {
    match v {
        Some(Json::Arr(a)) => a,
        _ => &[],
    }
}

/// `String(v)` di JavaScript per i valori JSON (un oggetto è "[object Object]", una lista i suoi elementi con le virgole).
pub fn string_js(v: Option<&Json>) -> String {
    match v {
        None => "undefined".to_string(),
        Some(Json::Null) => "null".to_string(),
        Some(Json::Bool(b)) => b.to_string(),
        Some(Json::Num(n)) => numero_js(*n),
        Some(Json::Str(s)) => s.clone(),
        Some(Json::Arr(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { string_js(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Json::Obj(_)) => "[object Object]".to_string(),
    }
}

/// `Number(v)` di JavaScript per i valori JSON: un testo si legge dopo trim() (vuoto vale 0, 0x/0o/0b, Infinity), il resto NaN.
pub fn numero(v: Option<&Json>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Json::Null) => 0.0,
        Some(Json::Bool(b)) => f64::from(u8::from(*b)),
        Some(Json::Num(n)) => *n,
        Some(Json::Str(s)) => numero_da_testo(s),
        Some(Json::Arr(a)) => match a.as_slice() {
            [] => 0.0,
            [x] if !matches!(x, Json::Arr(_) | Json::Obj(_)) => numero_da_testo(&string_js(Some(x))),
            [x] => numero(Some(x)),
            _ => f64::NAN,
        },
        Some(Json::Obj(_)) => f64::NAN,
    }
}

/// `Number(testo)`
pub fn numero_da_testo(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    let base = |p: &str, r: u32| -> Option<f64> {
        let x = t.strip_prefix(p).or_else(|| t.strip_prefix(&p.to_uppercase()))?;
        (!x.is_empty() && x.chars().all(|c| c.is_digit(r))).then(|| x.chars().fold(0.0, |n, c| n * f64::from(r) + f64::from(c.to_digit(r).unwrap_or(0))))
    };
    if let Some(n) = base("0x", 16).or_else(|| base("0o", 8)).or_else(|| base("0b", 2)) {
        return n;
    }
    let (segno, corpo) = match t.as_bytes()[0] {
        b'-' => (-1.0, &t[1..]),
        b'+' => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    if corpo == "Infinity" {
        return segno * f64::INFINITY;
    }
    // la grammatica di StrDecimalLiteral: cifre, punto, esponente; niente "inf", "nan" o separatori che parse() di Rust accetterebbe
    let b = corpo.as_bytes();
    let mut i = 0;
    let cifre = |i: &mut usize| {
        let da = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - da
    };
    let mut n = cifre(&mut i);
    if i < b.len() && b[i] == b'.' {
        i += 1;
        n += cifre(&mut i);
    }
    if n == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if cifre(&mut i) == 0 {
            return f64::NAN;
        }
    }
    if i != b.len() {
        return f64::NAN;
    }
    corpo.parse::<f64>().map_or(f64::NAN, |x| segno * x)
}

/// L'oggetto di un valore JSON costruito con `ogg!`.
pub fn mappa(v: Json) -> oe_nucleo::json::Mappa {
    match v {
        Json::Obj(m) => m,
        _ => oe_nucleo::json::Mappa::new(),
    }
}

/// `valore || ""` per i campi di testo che arrivano dall'agente: un valore falso diventa il testo vuoto, un testo resta, il resto come String().
pub fn testo_o_vuoto(v: Option<&Json>) -> Json {
    if py_bool(v) {
        v.cloned().unwrap_or_default()
    } else {
        Json::Str(String::new())
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn str_e_bool_di_python() {
        assert_eq!(py_str(None), "None");
        assert_eq!(py_str(Some(&Json::Bool(true))), "True");
        assert_eq!(py_str(Some(&Json::Num(1.0))), "1");
        assert_eq!(py_str(Some(&Json::Arr(vec![Json::Num(1.0), Json::Str("a".into())]))), "[1,\"a\"]");
        assert!(!py_bool(Some(&Json::Num(-0.0))));
        assert!(py_bool(Some(&Json::Num(f64::NAN))));
        assert!(!py_bool(Some(&Json::obj())));
    }

    #[test]
    fn strip_di_python_e_trim_di_javascript() {
        assert_eq!(py_strip("\u{1c}\u{85} a \u{3000}"), "a");
        assert_eq!(py_strip("\u{feff}a"), "\u{feff}a");
        assert_eq!(js_trim("\u{feff} a \u{a0}"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
        assert_eq!(testo_pulito(Some(&Json::Num(3.0))), Err(Errore::Tipo("'int' object has no attribute 'strip'".into())));
        assert_eq!(testo_pulito(Some(&Json::Num(0.0))), Ok(String::new()));
    }

    #[test]
    fn taglio_in_unita_utf16() {
        assert_eq!(taglia_js("a😀b", 2), "a");
        assert_eq!(taglia_js("a😀b", 3), "a😀");
        assert_eq!(lunghezza_js("a😀"), 3);
    }
}
