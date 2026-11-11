//! Un valore JSON con il comportamento di JavaScript, perché l'agente TypeScript leggeva con `JSON.parse` e scriveva da oggetti di JavaScript:
//! i numeri sono tutti f64 e si scrivono come `String(numero)`, le chiavi di un oggetto tengono l'ordine di inserimento con le chiavi intere
//! davanti in ordine crescente, una chiave ripetuta tiene il posto della prima e il valore dell'ultima.

use indexmap::IndexMap;
use std::fmt::Write as _;
use std::ops::Index;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Json {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Mappa),
}

/// Un oggetto di JavaScript: le chiavi che sono indici (interi canonici da 0 a 2^32 - 2) vengono prima, in ordine crescente; le altre
/// nell'ordine in cui sono state inserite.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mappa {
    voci: IndexMap<String, Json>,
    indici: usize,
}

fn indice(chiave: &str) -> Option<u32> {
    let b = chiave.as_bytes();
    if b.is_empty() || b.len() > 10 || (b.len() > 1 && b[0] == b'0') || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    chiave.parse::<u64>().ok().filter(|n| *n < u32::MAX as u64).map(|n| n as u32)
}

impl Mappa {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.voci.len()
    }

    pub fn is_empty(&self) -> bool {
        self.voci.is_empty()
    }

    pub fn get(&self, chiave: &str) -> Option<&Json> {
        self.voci.get(chiave)
    }

    pub fn get_mut(&mut self, chiave: &str) -> Option<&mut Json> {
        self.voci.get_mut(chiave)
    }

    pub fn contains_key(&self, chiave: &str) -> bool {
        self.voci.contains_key(chiave)
    }

    /// Come `oggetto[chiave] = valore`: una chiave che c'è già resta al suo posto.
    pub fn insert(&mut self, chiave: impl Into<String>, valore: impl Into<Json>) -> Option<Json> {
        let chiave = chiave.into();
        let nuova = !self.voci.contains_key(&chiave);
        if nuova && indice(&chiave).is_some() {
            self.indici += 1;
        }
        self.voci.insert(chiave, valore.into())
    }

    /// Come `delete oggetto[chiave]`: le altre chiavi tengono il loro ordine.
    pub fn remove(&mut self, chiave: &str) -> Option<Json> {
        let tolto = self.voci.shift_remove(chiave);
        if tolto.is_some() && indice(chiave).is_some() {
            self.indici -= 1;
        }
        tolto
    }

    /// Le voci nell'ordine di `Object.entries`.
    pub fn iter(&self) -> Box<dyn Iterator<Item = (&String, &Json)> + '_> {
        if self.indici == 0 {
            return Box::new(self.voci.iter());
        }
        let mut davanti: Vec<(u32, (&String, &Json))> = self.voci.iter().filter_map(|v| indice(v.0).map(|n| (n, v))).collect();
        davanti.sort_by_key(|(n, _)| *n);
        Box::new(davanti.into_iter().map(|(_, v)| v).chain(self.voci.iter().filter(|v| indice(v.0).is_none())))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Json> + '_ {
        self.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Json> + '_ {
        self.voci.values_mut()
    }
}

impl<K: Into<String>, V: Into<Json>> FromIterator<(K, V)> for Mappa {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut m = Mappa::new();
        for (k, v) in iter {
            m.insert(k, v);
        }
        m
    }
}

static NULL: Json = Json::Null;

impl Json {
    pub fn obj() -> Json {
        Json::Obj(Mappa::new())
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_arr_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&Mappa> {
        match self {
            Json::Obj(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_obj_mut(&mut self) -> Option<&mut Mappa> {
        match self {
            Json::Obj(m) => Some(m),
            _ => None,
        }
    }

    /// La proprietà di un oggetto; `None` se manca o se il valore non è un oggetto.
    pub fn get(&self, chiave: &str) -> Option<&Json> {
        self.as_obj().and_then(|m| m.get(chiave))
    }

    /// `JSON.parse(testo)`.
    pub fn analizza(testo: &str) -> Result<Json, ErroreJson> {
        Lettore { b: testo.as_bytes(), i: 0, py: false }.documento()
    }

    /// `json.loads(testo)` di Python: accetta anche `NaN`, `Infinity` e `-Infinity`.
    pub fn analizza_py(testo: &str) -> Result<Json, ErroreJson> {
        Lettore { b: testo.as_bytes(), i: 0, py: true }.documento()
    }

    /// `json.dumps(valore, ensure_ascii=False)` di Python come lo riproduce `jsonPy`: separatori ", " e ": ", `NaN` e `Infinity` scritti così.
    pub fn in_py(&self) -> String {
        let mut out = String::new();
        self.scrivi_py(&mut out);
        out
    }

    fn scrivi_py(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&numero_js(*n)),
            Json::Str(s) => scrivi_stringa(s, out),
            Json::Arr(v) => {
                out.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    x.scrivi_py(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                out.push('{');
                for (i, (k, x)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    scrivi_stringa(k, out);
                    out.push_str(": ");
                    x.scrivi_py(out);
                }
                out.push('}');
            }
        }
    }

    /// `JSON.stringify(valore)`.
    pub fn in_js(&self) -> String {
        let mut out = String::new();
        self.scrivi_js(&mut out, 0, 0);
        out
    }

    /// `JSON.stringify(valore, null, rientro)`.
    pub fn in_js_rientrato(&self, rientro: usize) -> String {
        let mut out = String::new();
        self.scrivi_js(&mut out, rientro, 0);
        out
    }

    fn scrivi_js(&self, out: &mut String, rientro: usize, livello: usize) {
        let a_capo = |out: &mut String, livello: usize| {
            if rientro > 0 {
                out.push('\n');
                out.extend(std::iter::repeat_n(' ', rientro * livello));
            }
        };
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&if n.is_finite() { numero_js(*n) } else { "null".to_string() }),
            Json::Str(s) => scrivi_stringa(s, out),
            Json::Arr(v) if v.is_empty() => out.push_str("[]"),
            Json::Arr(v) => {
                out.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    a_capo(out, livello + 1);
                    x.scrivi_js(out, rientro, livello + 1);
                }
                a_capo(out, livello);
                out.push(']');
            }
            Json::Obj(m) if m.is_empty() => out.push_str("{}"),
            Json::Obj(m) => {
                out.push('{');
                for (i, (k, x)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    a_capo(out, livello + 1);
                    scrivi_stringa(k, out);
                    out.push(':');
                    if rientro > 0 {
                        out.push(' ');
                    }
                    x.scrivi_js(out, rientro, livello + 1);
                }
                a_capo(out, livello);
                out.push('}');
            }
        }
    }
}

/// `String(numero)` di JavaScript: 1 e non 1.0, 1e+21, NaN, Infinity, -0 come 0.
pub fn numero_js(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_string()
}

/// `JSON.stringify(stringa)`.
pub fn stringa_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    scrivi_stringa(s, &mut out);
    out
}

fn scrivi_stringa(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

impl Index<&str> for Json {
    type Output = Json;
    /// `valore[chiave]`, con `Null` al posto di `undefined`.
    fn index(&self, chiave: &str) -> &Json {
        self.get(chiave).unwrap_or(&NULL)
    }
}

impl Index<&str> for Mappa {
    type Output = Json;
    /// `oggetto[chiave]`, con `Null` al posto di `undefined`.
    fn index(&self, chiave: &str) -> &Json {
        self.get(chiave).unwrap_or(&NULL)
    }
}

impl Index<usize> for Json {
    type Output = Json;
    fn index(&self, i: usize) -> &Json {
        self.as_arr().and_then(|v| v.get(i)).unwrap_or(&NULL)
    }
}

impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Json::Bool(v)
    }
}
impl From<f64> for Json {
    fn from(v: f64) -> Self {
        Json::Num(v)
    }
}
impl From<i64> for Json {
    fn from(v: i64) -> Self {
        Json::Num(v as f64)
    }
}
impl From<i32> for Json {
    fn from(v: i32) -> Self {
        Json::Num(v as f64)
    }
}
impl From<u32> for Json {
    fn from(v: u32) -> Self {
        Json::Num(v as f64)
    }
}
impl From<usize> for Json {
    fn from(v: usize) -> Self {
        Json::Num(v as f64)
    }
}
impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Json::Str(v.to_string())
    }
}
impl From<String> for Json {
    fn from(v: String) -> Self {
        Json::Str(v)
    }
}
impl From<&String> for Json {
    fn from(v: &String) -> Self {
        Json::Str(v.clone())
    }
}
impl From<Mappa> for Json {
    fn from(v: Mappa) -> Self {
        Json::Obj(v)
    }
}
impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(v: Vec<T>) -> Self {
        Json::Arr(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(v: Option<T>) -> Self {
        v.map_or(Json::Null, Into::into)
    }
}

/// Un oggetto scritto come in JavaScript: `ogg! { "a": 1, "b": "x" }`.
#[macro_export]
macro_rules! ogg {
    ($($k:literal : $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = $crate::json::Mappa::new();
        $( m.insert($k, $crate::json::Json::from($v)); )*
        $crate::json::Json::Obj(m)
    }};
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErroreJson {
    pub messaggio: String,
    /// La posizione nel testo, in byte.
    pub posizione: usize,
}

impl std::fmt::Display for ErroreJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.messaggio)
    }
}

impl std::error::Error for ErroreJson {}

struct Lettore<'a> {
    b: &'a [u8],
    i: usize,
    py: bool,
}

/// La profondità oltre la quale un documento si rifiuta, per non esaurire la pila.
const PROFONDITA_MAX: usize = 2000;

impl Lettore<'_> {
    fn errore<T>(&self, cosa: &str) -> Result<T, ErroreJson> {
        Err(ErroreJson { messaggio: format!("{cosa} in JSON at position {}", self.i), posizione: self.i })
    }

    fn spazi(&mut self) {
        while let Some(c) = self.b.get(self.i) {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn documento(mut self) -> Result<Json, ErroreJson> {
        self.spazi();
        let v = self.valore(0)?;
        self.spazi();
        if self.i < self.b.len() {
            return self.errore("Unexpected non-whitespace character after JSON");
        }
        Ok(v)
    }

    fn parola(&mut self, parola: &str, v: Json) -> Result<Json, ErroreJson> {
        if self.b[self.i..].starts_with(parola.as_bytes()) {
            self.i += parola.len();
            Ok(v)
        } else {
            self.errore("Unexpected token")
        }
    }

    fn valore(&mut self, livello: usize) -> Result<Json, ErroreJson> {
        if livello > PROFONDITA_MAX {
            return self.errore("Maximum nesting exceeded");
        }
        match self.b.get(self.i) {
            None => self.errore("Unexpected end of JSON input"),
            Some(b'{') => {
                self.i += 1;
                let mut m = Mappa::new();
                self.spazi();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.spazi();
                    if self.b.get(self.i) != Some(&b'"') {
                        return self.errore("Expected double-quoted property name");
                    }
                    let k = self.stringa()?;
                    self.spazi();
                    if self.b.get(self.i) != Some(&b':') {
                        return self.errore("Expected ':' after property name");
                    }
                    self.i += 1;
                    self.spazi();
                    let v = self.valore(livello + 1)?;
                    m.insert(k, v);
                    self.spazi();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.errore("Expected ',' or '}' after property value"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut v = Vec::new();
                self.spazi();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(v));
                }
                loop {
                    self.spazi();
                    v.push(self.valore(livello + 1)?);
                    self.spazi();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(v));
                        }
                        _ => return self.errore("Expected ',' or ']' after array element"),
                    }
                }
            }
            Some(b'"') => self.stringa().map(Json::Str),
            Some(b't') => self.parola("true", Json::Bool(true)),
            Some(b'f') => self.parola("false", Json::Bool(false)),
            Some(b'n') => self.parola("null", Json::Null),
            Some(b'N') if self.py => self.parola("NaN", Json::Num(f64::NAN)),
            Some(b'I') if self.py => self.parola("Infinity", Json::Num(f64::INFINITY)),
            Some(b'-') if self.py && self.b[self.i..].starts_with(b"-Infinity") => self.parola("-Infinity", Json::Num(f64::NEG_INFINITY)),
            Some(b'-' | b'0'..=b'9') => self.numero(),
            Some(_) => self.errore("Unexpected token"),
        }
    }

    fn cifre(&mut self) -> usize {
        let da = self.i;
        while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - da
    }

    fn numero(&mut self) -> Result<Json, ErroreJson> {
        let da = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.cifre();
            }
            _ => return self.errore("No number after minus sign"),
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if self.cifre() == 0 {
                return self.errore("Unterminated fractional number");
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.cifre() == 0 {
                return self.errore("Exponent part is missing a number");
            }
        }
        // i byte sono cifre, segni, punto ed esponente: ASCII
        let testo = std::str::from_utf8(&self.b[da..self.i]).unwrap_or("0");
        Ok(Json::Num(testo.parse::<f64>().unwrap_or(f64::NAN)))
    }

    fn quattro_hex(&mut self) -> Result<u32, ErroreJson> {
        let Some(pezzo) = self.b.get(self.i..self.i + 4) else { return self.errore("Bad Unicode escape") };
        let mut n = 0u32;
        for c in pezzo {
            let Some(d) = (*c as char).to_digit(16) else { return self.errore("Bad Unicode escape") };
            n = n * 16 + d;
        }
        self.i += 4;
        Ok(n)
    }

    fn stringa(&mut self) -> Result<String, ErroreJson> {
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let da = self.i;
            while let Some(c) = self.b.get(self.i) {
                if *c == b'"' || *c == b'\\' || *c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.extend_from_slice(&self.b[da..self.i]);
            match self.b.get(self.i) {
                None => return self.errore("Unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    // il testo d'ingresso è UTF-8 valido e si taglia solo su byte ASCII
                    return Ok(String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()));
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(c) = self.b.get(self.i).copied() else { return self.errore("Unterminated string") };
                    self.i += 1;
                    let ch = match c {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let alto = self.quattro_hex()?;
                            if (0xd800..0xdc00).contains(&alto) && self.b[self.i..].starts_with(b"\\u") {
                                let ripresa = self.i;
                                self.i += 2;
                                let basso = self.quattro_hex()?;
                                if (0xdc00..0xe000).contains(&basso) {
                                    char::from_u32(0x10000 + ((alto - 0xd800) << 10) + (basso - 0xdc00)).unwrap_or('\u{fffd}')
                                } else {
                                    self.i = ripresa;
                                    '\u{fffd}'
                                }
                            } else {
                                // un surrogato da solo non sta in una stringa di Rust: diventa U+FFFD
                                char::from_u32(alto).unwrap_or('\u{fffd}')
                            }
                        }
                        _ => {
                            self.i -= 1;
                            return self.errore("Bad escaped character");
                        }
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                Some(_) => return self.errore("Bad control character in string literal"),
            }
        }
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn scrive_come_python_e_come_javascript() {
        let v = Json::analizza(r#"{"b": [1, 2.5, 1e21, -0], "a": {"x": null, "y": true}, "t": "è \"q\" \n\u0001"}"#).unwrap();
        assert_eq!(v.in_py(), "{\"b\": [1, 2.5, 1e+21, 0], \"a\": {\"x\": null, \"y\": true}, \"t\": \"è \\\"q\\\" \\n\\u0001\"}");
        assert_eq!(v.in_js(), "{\"b\":[1,2.5,1e+21,0],\"a\":{\"x\":null,\"y\":true},\"t\":\"è \\\"q\\\" \\n\\u0001\"}");
        assert_eq!(Json::analizza(r#"{"a":[1,{"b":[]}],"c":{}}"#).unwrap().in_js_rientrato(2), "{\n  \"a\": [\n    1,\n    {\n      \"b\": []\n    }\n  ],\n  \"c\": {}\n}");
    }

    #[test]
    fn le_chiavi_intere_vengono_prima_e_una_ripetuta_tiene_il_posto() {
        let v = Json::analizza(r#"{"b":1,"2":2,"a":3,"1":4,"01":5,"b":6}"#).unwrap();
        assert_eq!(v.in_js(), r#"{"1":4,"2":2,"b":6,"a":3,"01":5}"#);
    }

    #[test]
    fn nan_e_infinity_solo_per_python() {
        assert!(Json::analizza("NaN").is_err());
        assert_eq!(Json::analizza_py("[NaN, Infinity, -Infinity, -1]").unwrap().in_py(), "[NaN, Infinity, -Infinity, -1]");
        assert_eq!(Json::analizza_py("[NaN]").unwrap().in_js(), "[null]");
    }

    #[test]
    fn rifiuta_quello_che_json_parse_rifiuta() {
        for t in ["", "{", "[1,]", "{\"a\":1,}", "01", "1.", "-", "\"a", "\"\\x\"", "tru", "1 2", "'a'", "\"\t\""] {
            assert!(Json::analizza(t).is_err(), "{t}");
        }
        assert_eq!(Json::analizza(" \"\\ud83d\\ude00\\u00e8\" ").unwrap(), Json::Str("😀è".to_string()));
    }
}
