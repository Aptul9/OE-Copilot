//! Il codec protobuf delle RPC di Google Chat: lo schema `proto/googlechat.proto`, letto una volta, e le regole di protobufjs 8.8.1 con cui
//! l'agente TypeScript codifica (`encodeChat`: `fromObject`, `verify`, `encode`) e decodifica (`decodeChat`: `decode`, `toObject` con int64,
//! enum e bytes come stringhe). Lo schema è proto2: ogni campo ha presenza esplicita, gli enum sono chiusi (un valore sconosciuto si scarta),
//! i ripetuti si scrivono uno per uno e si leggono anche impacchettati. Usa bool, bytes, double, int32, int64, string, enum e messaggi.
//!
//! Lo schema viene da gweb e da mautrix-googlechat: provenienza e licenza in proto/NOTICE.md e proto/AGPL-3.0.txt.

use crate::js::{numero_di, spazio_js, stringa_di, vero};
use base64::Engine;
use oe_nucleo::json::{Json, Mappa};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

/// `util.recursionLimit`.
const LIMITE: usize = 100;
/// Un oggetto con `length` passato come bytes si copia fin qui; JavaScript allocherebbe qualunque lunghezza.
const MAX_SIMILE_ARRAY: usize = 64 * 1024 * 1024;
/// Le proprietà che ogni oggetto JavaScript eredita: `schema.fields[chiave]` le trova, quindi `encodeChat` non le dà per sconosciute.
const EREDITATE: [&str; 12] = [
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

static SCHEMA: LazyLock<Schema> = LazyLock::new(|| Schema::analizza(include_str!("../proto/googlechat.proto")).expect("googlechat.proto"));

/// `encodeChat(tipo, valore)`: i campi di primo livello devono esserci tutti nello schema.
pub fn codifica_chat(tipo: &str, valore: &Json) -> Result<Vec<u8>, String> {
    let s = &*SCHEMA;
    let mi = s.tipo(tipo)?;
    let campi = &s.messaggi[mi].campi;
    // Object.keys(valore): gli indici di un array o di una stringa, le chiavi di un oggetto
    let sconosciuta = match valore {
        Json::Obj(m) => m.keys().find(|k| !campi.iter().any(|c| &c.nome == *k) && !EREDITATE.contains(&k.as_str())).cloned(),
        Json::Arr(a) if !a.is_empty() => Some("0".to_string()),
        Json::Str(t) if !t.is_empty() => Some("0".to_string()),
        Json::Null => return Err("Cannot convert undefined or null to object".into()),
        _ => None,
    };
    if let Some(k) = sconosciuta {
        return Err(format!("{tipo}: unknown field {k}"));
    }
    let m = s.da_oggetto(mi, valore, 0)?;
    let mut out = Vec::new();
    s.scrivi(mi, &m, &mut out);
    Ok(out)
}

/// `decodeChat(tipo, byte)`.
pub fn decodifica_chat(tipo: &str, byte: &[u8]) -> Result<Json, String> {
    let s = &*SCHEMA;
    let mi = s.tipo(tipo)?;
    let mut r = Lettore { buf: byte, pos: 0, len: byte.len() };
    let mut m = Valori::vuoti(s.messaggi[mi].campi.len());
    s.leggi(mi, &mut r, None, 0, &mut m)?;
    s.in_oggetto(mi, &m, 0)
}

// ---- lo schema ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Scalare {
    Bool,
    Bytes,
    Double,
    Int32,
    Int64,
    Stringa,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tipo {
    Scalare(Scalare),
    Messaggio(usize),
    Enum(usize),
}

impl Tipo {
    /// `types.basic`: il tipo di filo.
    fn filo(self) -> u32 {
        match self {
            Tipo::Scalare(Scalare::Double) => 1,
            Tipo::Scalare(Scalare::Bytes | Scalare::Stringa) | Tipo::Messaggio(_) => 2,
            _ => 0,
        }
    }

    /// `types.packed`: numeri ed enum si leggono anche impacchettati.
    fn impacchettabile(self) -> bool {
        !matches!(self, Tipo::Scalare(Scalare::Bytes | Scalare::Stringa) | Tipo::Messaggio(_))
    }
}

/// Un elemento con nome dentro un messaggio o alla radice; `Altro` è un campo o un oneof, che la ricerca dei tipi trova e scarta.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Nodo {
    Messaggio(usize),
    Enum(usize),
    Altro,
}

struct Campo {
    nome: String,
    id: u32,
    ripetuto: bool,
    tipo: Tipo,
    /// Il oneof di cui fa parte, indice in `Messaggio::oneof`.
    oneof: Option<usize>,
}

struct Messaggio {
    /// Il nome completo senza il punto iniziale, come "Annotation.UploadMetadata".
    nome: String,
    genitore: Option<usize>,
    /// I tipi e gli enum dichiarati dentro, in ordine.
    figli: Vec<(String, Nodo)>,
    oneof: Vec<String>,
    campi: Vec<Campo>,
    /// Gli indici dei campi in ordine di numero: l'ordine di `encode` e di `toObject`.
    per_id: Vec<usize>,
}

struct Enumerazione {
    nome: String,
    valori: Vec<(String, i32)>,
}

impl Enumerazione {
    fn nome_di(&self, v: i32) -> Option<&str> {
        self.valori.iter().find(|x| x.1 == v).map(|x| x.0.as_str())
    }

    /// Lo `switch` di `fromObject`: il nome o il numero esatto di un valore; con un enum chiuso il resto non imposta nulla.
    fn da_json(&self, v: &Json) -> Option<i32> {
        match v {
            Json::Str(s) => self.valori.iter().find(|x| &x.0 == s).map(|x| x.1),
            Json::Num(n) => self.valori.iter().find(|x| f64::from(x.1) == *n).map(|x| x.1),
            _ => None,
        }
    }
}

struct Schema {
    messaggi: Vec<Messaggio>,
    enums: Vec<Enumerazione>,
    radice: Vec<(String, Nodo)>,
    /// Tipi ed enum per nome completo: `root._fullyQualifiedObjects`.
    qualificati: HashMap<String, Nodo>,
}

impl Schema {
    fn tipo(&self, nome: &str) -> Result<usize, String> {
        match self.cerca(None, nome, true) {
            Some(Tipo::Messaggio(m)) => Ok(m),
            _ => Err(format!("no such type: {nome}")),
        }
    }

    fn figli(&self, ns: Option<usize>) -> &[(String, Nodo)] {
        ns.map_or(&self.radice, |m| &self.messaggi[m].figli)
    }

    /// `get`: in un messaggio prima i campi, poi i oneof, poi i tipi dichiarati dentro; alla radice solo i tipi.
    fn prendi(&self, ns: Option<usize>, nome: &str) -> Option<Nodo> {
        if let Some(m) = ns.map(|m| &self.messaggi[m]) {
            if m.campi.iter().any(|c| c.nome == nome) || m.oneof.iter().any(|o| o == nome) {
                return Some(Nodo::Altro);
            }
        }
        self.figli(ns).iter().find(|f| f.0 == nome).map(|f| f.1)
    }

    /// `_lookupImpl`: il primo pezzo qui, e se non c'è in ogni messaggio dichiarato dentro, in profondità.
    fn cerca_qui(&self, ns: Option<usize>, percorso: &[&str]) -> Option<Nodo> {
        match self.prendi(ns, percorso[0]) {
            Some(n) if percorso.len() == 1 => Some(n),
            Some(Nodo::Messaggio(m)) => self.cerca_qui(Some(m), &percorso[1..]),
            Some(_) => None,
            None => self.figli(ns).iter().find_map(|f| match f.1 {
                Nodo::Messaggio(m) => self.cerca_qui(Some(m), percorso),
                _ => None,
            }),
        }
    }

    /// `lookup(percorso, [Type, Enum])` (o solo `[Type]`): qui e sotto, poi il nome completo, poi ogni antenato e quello che contiene.
    fn cerca(&self, ns: Option<usize>, testo: &str, solo_messaggi: bool) -> Option<Tipo> {
        if let Some(resto) = testo.strip_prefix('.') {
            return self.cerca(None, resto, solo_messaggi);
        }
        let accetta = |n: Option<Nodo>| match n {
            Some(Nodo::Messaggio(m)) => Some(Tipo::Messaggio(m)),
            Some(Nodo::Enum(e)) if !solo_messaggi => Some(Tipo::Enum(e)),
            _ => None,
        };
        let percorso: Vec<&str> = testo.split('.').collect();
        if let Some(t) = accetta(self.cerca_qui(ns, &percorso)).or_else(|| accetta(self.qualificati.get(testo).copied())) {
            return Some(t);
        }
        let mut corrente = ns;
        while let Some(m) = corrente {
            let su = self.messaggi[m].genitore;
            if let Some(t) = accetta(self.cerca_qui(su, &percorso)) {
                return Some(t);
            }
            corrente = su;
        }
        None
    }

    fn analizza(testo: &str) -> Result<Schema, String> {
        let mut a = Analisi { parole: parole(testo)?, i: 0, messaggi: Vec::new(), enums: Vec::new(), radice: Vec::new(), da_risolvere: Vec::new() };
        loop {
            match a.parola() {
                Err(_) => break,
                Ok(";") => {}
                Ok("syntax" | "option") => a.salta_istruzione()?,
                Ok("message") => a.messaggio(None)?,
                Ok("enum") => a.enumerazione(None)?,
                Ok(p) => return Err(format!("parola inattesa nello schema: {p}")),
            }
        }
        let mut qualificati = HashMap::new();
        for (i, m) in a.messaggi.iter().enumerate() {
            qualificati.insert(m.nome.clone(), Nodo::Messaggio(i));
        }
        for (i, e) in a.enums.iter().enumerate() {
            qualificati.insert(e.nome.clone(), Nodo::Enum(i));
        }
        let mut s = Schema { messaggi: a.messaggi, enums: a.enums, radice: a.radice, qualificati };
        for (m, c, scritto) in a.da_risolvere {
            let tipo = match scritto.as_str() {
                "bool" => Tipo::Scalare(Scalare::Bool),
                "bytes" => Tipo::Scalare(Scalare::Bytes),
                "double" => Tipo::Scalare(Scalare::Double),
                "int32" => Tipo::Scalare(Scalare::Int32),
                "int64" => Tipo::Scalare(Scalare::Int64),
                "string" => Tipo::Scalare(Scalare::Stringa),
                "float" | "uint32" | "uint64" | "sint32" | "sint64" | "fixed32" | "fixed64" | "sfixed32" | "sfixed64" => {
                    return Err(format!("tipo {scritto} non gestito"));
                }
                t => s.cerca(Some(m), t, false).ok_or_else(|| format!("tipo {t} non trovato in {}", s.messaggi[m].nome))?,
            };
            s.messaggi[m].campi[c].tipo = tipo;
        }
        for m in &mut s.messaggi {
            let mut per_id: Vec<usize> = (0..m.campi.len()).collect();
            per_id.sort_by_key(|&i| m.campi[i].id);
            m.per_id = per_id;
        }
        Ok(s)
    }
}

/// Le parole del file .proto: nomi e numeri, stringhe tra virgolette, un carattere di punteggiatura alla volta; i commenti spariscono.
fn parole(testo: &str) -> Result<Vec<&str>, String> {
    let b = testo.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if b[i..].starts_with(b"//") {
            i = b[i..].iter().position(|&x| x == b'\n').map_or(b.len(), |p| i + p);
        } else if b[i..].starts_with(b"/*") {
            i = testo[i + 2..].find("*/").map_or(b.len(), |p| i + p + 4);
        } else if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let fine = (j + 1).min(b.len());
            out.push(&testo[i..fine]);
            i = fine;
        } else if b"{}[]();=,<>".contains(&c) {
            out.push(&testo[i..i + 1]);
            i += 1;
        } else {
            let j = b[i..].iter().position(|x| !(x.is_ascii_alphanumeric() || b"_.-+".contains(x))).map_or(b.len(), |p| i + p);
            if j == i {
                return Err(format!("carattere inatteso nello schema al byte {i}"));
            }
            out.push(&testo[i..j]);
            i = j;
        }
    }
    Ok(out)
}

struct Analisi<'a> {
    parole: Vec<&'a str>,
    i: usize,
    messaggi: Vec<Messaggio>,
    enums: Vec<Enumerazione>,
    radice: Vec<(String, Nodo)>,
    /// Messaggio, campo e tipo scritto: si risolvono a schema intero, come fa protobufjs.
    da_risolvere: Vec<(usize, usize, String)>,
}

impl<'a> Analisi<'a> {
    fn parola(&mut self) -> Result<&'a str, String> {
        let p = self.parole.get(self.i).copied().ok_or("lo schema finisce a metà")?;
        self.i += 1;
        Ok(p)
    }

    fn attendi(&mut self, attesa: &str) -> Result<(), String> {
        match self.parola()? {
            p if p == attesa => Ok(()),
            p => Err(format!("nello schema serve {attesa}, c'è {p}")),
        }
    }

    fn salta_istruzione(&mut self) -> Result<(), String> {
        while self.parola()? != ";" {}
        Ok(())
    }

    /// Le opzioni fra quadre di un campo o di un valore, poi il punto e virgola.
    fn fine_dichiarazione(&mut self) -> Result<(), String> {
        let mut p = self.parola()?;
        if p == "[" {
            while self.parola()? != "]" {}
            p = self.parola()?;
        }
        if p == ";" { Ok(()) } else { Err(format!("nello schema serve ;, c'è {p}")) }
    }

    fn aggiungi(&mut self, genitore: Option<usize>, nome: &str, n: Nodo) {
        let figli = match genitore {
            Some(g) => &mut self.messaggi[g].figli,
            None => &mut self.radice,
        };
        figli.push((nome.to_string(), n));
    }

    fn nome_completo(&self, genitore: Option<usize>, nome: &str) -> String {
        genitore.map_or_else(|| nome.to_string(), |g| format!("{}.{nome}", self.messaggi[g].nome))
    }

    fn messaggio(&mut self, genitore: Option<usize>) -> Result<(), String> {
        let nome = self.parola()?;
        let id = self.messaggi.len();
        let completo = self.nome_completo(genitore, nome);
        self.messaggi.push(Messaggio { nome: completo, genitore, figli: Vec::new(), oneof: Vec::new(), campi: Vec::new(), per_id: Vec::new() });
        self.aggiungi(genitore, nome, Nodo::Messaggio(id));
        self.attendi("{")?;
        loop {
            match self.parola()? {
                "}" => return Ok(()),
                ";" => {}
                "message" => self.messaggio(Some(id))?,
                "enum" => self.enumerazione(Some(id))?,
                "option" | "reserved" | "extensions" => self.salta_istruzione()?,
                "oneof" => {
                    let nome = self.parola()?;
                    let oneof = self.messaggi[id].oneof.len();
                    self.messaggi[id].oneof.push(nome.to_string());
                    self.attendi("{")?;
                    loop {
                        match self.parola()? {
                            "}" => break,
                            ";" => {}
                            "option" => self.salta_istruzione()?,
                            tipo => self.campo(id, tipo, false, Some(oneof))?,
                        }
                    }
                }
                "optional" => {
                    let tipo = self.parola()?;
                    self.campo(id, tipo, false, None)?;
                }
                "repeated" => {
                    let tipo = self.parola()?;
                    self.campo(id, tipo, true, None)?;
                }
                p @ ("required" | "group" | "map" | "extend") => return Err(format!("{p} non gestito nello schema")),
                tipo => self.campo(id, tipo, false, None)?,
            }
        }
    }

    fn campo(&mut self, m: usize, tipo: &str, ripetuto: bool, oneof: Option<usize>) -> Result<(), String> {
        let nome = self.parola()?;
        self.attendi("=")?;
        let id = self.parola()?;
        let id = id.parse::<u32>().map_err(|_| format!("numero di campo non valido: {id}"))?;
        self.fine_dichiarazione()?;
        let campi = &mut self.messaggi[m].campi;
        self.da_risolvere.push((m, campi.len(), tipo.to_string()));
        campi.push(Campo { nome: nome.to_string(), id, ripetuto, tipo: Tipo::Scalare(Scalare::Bool), oneof });
        Ok(())
    }

    fn enumerazione(&mut self, genitore: Option<usize>) -> Result<(), String> {
        let nome = self.parola()?;
        let id = self.enums.len();
        self.enums.push(Enumerazione { nome: self.nome_completo(genitore, nome), valori: Vec::new() });
        self.aggiungi(genitore, nome, Nodo::Enum(id));
        self.attendi("{")?;
        loop {
            match self.parola()? {
                "}" => return Ok(()),
                ";" => {}
                "option" | "reserved" => self.salta_istruzione()?,
                valore => {
                    self.attendi("=")?;
                    let n = self.parola()?;
                    let n = n.parse::<i32>().map_err(|_| format!("valore di enum non valido: {n}"))?;
                    self.fine_dichiarazione()?;
                    self.enums[id].valori.push((valore.to_string(), n));
                }
            }
        }
    }
}

// ---- i valori di un messaggio --------------------------------------------------------------------------------------------------

#[derive(Debug)]
enum Dato {
    Bool(bool),
    Bytes(Vec<u8>),
    Double(f64),
    Int32(i32),
    Int64(i64),
    Stringa(String),
    Enum(i32),
    Messaggio(Valori),
}

/// I valori per campo, nell'ordine dei campi dello schema: nessuno se il campo manca, uno se è singolo, tutti se è ripetuto.
#[derive(Debug)]
struct Valori(Vec<Vec<Dato>>);

impl Valori {
    fn vuoti(n: usize) -> Valori {
        Valori((0..n).map(|_| Vec::new()).collect())
    }
}

/// `util.isObject`: oggetti e array.
fn oggetto(v: &Json) -> bool {
    matches!(v, Json::Obj(_) | Json::Arr(_))
}

/// `d[nome]` su un oggetto o un array: di un array si vede solo `length`.
fn proprieta<'a>(d: &'a Json, nome: &str) -> Option<Cow<'a, Json>> {
    match d {
        Json::Obj(m) => m.get(nome).map(Cow::Borrowed),
        Json::Arr(a) if nome == "length" => Some(Cow::Owned(Json::Num(a.len() as f64))),
        _ => None,
    }
}

/// `x | 0`.
fn a_int32(n: f64) -> i32 {
    if n.is_finite() { (n.trunc() % 4_294_967_296.0) as i64 as u32 as i32 } else { 0 }
}

/// Un elemento di un `Uint8Array`.
fn a_uint8(n: f64) -> u8 {
    a_int32(n) as u8
}

/// `Long.fromNumber(n)`: NaN vale 0, oltre i limiti si ferma al minimo o al massimo, i decimali si troncano.
fn long_da_numero(n: f64) -> i64 {
    const P63: f64 = 9_223_372_036_854_775_808.0;
    if n.is_nan() {
        0
    } else if n <= -P63 {
        i64::MIN
    } else if n + 1.0 >= P63 {
        i64::MAX
    } else if n < 0.0 {
        long_da_numero(-n).wrapping_neg()
    } else {
        n.trunc() as i64
    }
}

/// `parseInt(pezzo, 10)` su unità UTF-16: spazi, segno, le cifre che seguono; nessuna cifra è NaN.
fn parse_int(pezzo: &[u16]) -> f64 {
    let mut i = pezzo.iter().take_while(|&&u| char::from_u32(u32::from(u)).is_some_and(spazio_js)).count();
    let segno = match pezzo.get(i) {
        Some(&u) if u == u16::from(b'-') => -1.0,
        _ => 1.0,
    };
    if matches!(pezzo.get(i), Some(&u) if u == u16::from(b'-') || u == u16::from(b'+')) {
        i += 1;
    }
    let cifre: Vec<u16> = pezzo[i..].iter().copied().take_while(|u| (u16::from(b'0')..=u16::from(b'9')).contains(u)).collect();
    if cifre.is_empty() {
        return f64::NAN;
    }
    segno * cifre.iter().fold(0.0, |a, &u| a * 10.0 + f64::from(u - u16::from(b'0')))
}

/// `Long.fromString(testo)`: otto unità alla volta lette con `parseInt`, con il giro dei 64 bit.
fn long_da_testo(u: &[u16]) -> Result<i64, String> {
    if u.is_empty() {
        return Err("empty string".into());
    }
    let testo = String::from_utf16_lossy(u);
    if ["NaN", "Infinity", "+Infinity", "-Infinity"].contains(&testo.as_str()) {
        return Ok(0);
    }
    match u.iter().position(|&x| x == u16::from(b'-')) {
        Some(0) => return Ok(long_da_testo(&u[1..])?.wrapping_neg()),
        Some(_) => return Err("interior hyphen".into()),
        None => {}
    }
    let mut r: i64 = 0;
    for pezzo in u.chunks(8) {
        r = r.wrapping_mul(10i64.pow(pezzo.len() as u32)).wrapping_add(long_da_numero(parse_int(pezzo)));
    }
    Ok(r)
}

/// `Long.fromValue(v)`: un numero, una stringa decimale, altrimenti `low` e `high` dell'oggetto.
fn long_da_valore(v: &Json) -> Result<i64, String> {
    match v {
        Json::Num(n) => Ok(long_da_numero(*n)),
        Json::Str(s) => long_da_testo(&s.encode_utf16().collect::<Vec<_>>()),
        Json::Null => Err("Cannot read properties of null (reading 'low')".into()),
        Json::Obj(o) => {
            let parte = |k: &str| o.get(k).map_or(0, |x| a_int32(numero_di(x))) as u32;
            Ok((u64::from(parte("high")) << 32 | u64::from(parte("low"))) as i64)
        }
        Json::Bool(_) | Json::Arr(_) => Ok(0),
    }
}

/// `util.base64.decode` su un buffer lungo `util.base64.length`: anche gli alfabeti URL, si ferma al primo `=` dopo due caratteri.
fn base64_pbjs(s: &str) -> Result<Vec<u8>, String> {
    let u: Vec<u16> = s.encode_utf16().collect();
    let senza_uguali = u.len() - u.iter().rev().take_while(|&&c| c == u16::from(b'=')).count();
    let mut out = vec![0u8; senza_uguali * 3 / 4];
    let valore = |c: u16| -> Option<u8> {
        let c = u8::try_from(c).ok()?;
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let (mut j, mut t, mut o) = (0, 0u8, 0);
    let mut metti = |o: &mut usize, b: u8| {
        if let Some(x) = out.get_mut(*o) {
            *x = b;
        }
        *o += 1;
    };
    for &c in &u {
        if c == u16::from(b'=') && j > 1 {
            break;
        }
        let c = valore(c).ok_or("invalid encoding")?;
        match j {
            0 => {
                t = c;
                j = 1;
            }
            1 => {
                metti(&mut o, t << 2 | (c & 48) >> 4);
                t = c;
                j = 2;
            }
            2 => {
                metti(&mut o, (t & 15) << 4 | (c & 60) >> 2);
                t = c;
                j = 3;
            }
            _ => {
                metti(&mut o, (t & 3) << 6 | c);
                j = 0;
            }
        }
    }
    if j == 1 {
        return Err("invalid encoding".into());
    }
    Ok(out)
}

/// Un valore bytes di `fromObject`: base64, un array di numeri o un oggetto con `length`; `None` lascia il campo vuoto.
fn byte_da(v: &Json) -> Result<Option<Vec<u8>>, String> {
    match v {
        Json::Str(s) => base64_pbjs(s).map(Some),
        Json::Arr(a) => Ok(Some(a.iter().map(|x| a_uint8(numero_di(x))).collect())),
        Json::Obj(o) => match o.get("length") {
            Some(Json::Num(n)) if *n >= 0.0 => {
                let lunghezza = a_int32(*n) as u32 as usize;
                if lunghezza > MAX_SIMILE_ARRAY {
                    return Err("buffer too large".into());
                }
                Ok(Some((0..lunghezza).map(|i| o.get(&i.to_string()).map_or(0, |x| a_uint8(numero_di(x)))).collect()))
            }
            // `length >= 0` lo prende, `verify` vuole un numero
            Some(l) if numero_di(l) >= 0.0 => Err("buffer expected".into()),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

impl Schema {
    /// `fromObject`, con gli errori che poi darebbe `verify`.
    fn da_oggetto(&self, mi: usize, d: &Json, q: usize) -> Result<Valori, String> {
        let msg = &self.messaggi[mi];
        if !oggetto(d) {
            return Err(format!(".{}: object expected", msg.nome));
        }
        if q > LIMITE {
            return Err("max depth exceeded".into());
        }
        let mut m = Valori::vuoti(msg.campi.len());
        for (i, c) in msg.campi.iter().enumerate() {
            let Some(v) = proprieta(d, &c.nome) else { continue };
            if c.ripetuto {
                if !vero(&v) {
                    continue;
                }
                let Json::Arr(elementi) = &*v else { return Err(format!(".{}.{}: array expected", msg.nome, c.nome)) };
                for e in elementi {
                    if let Some(x) = self.dato_da(mi, c, e, q, true)? {
                        m.0[i].push(x);
                    }
                }
            } else if !v.is_null() {
                if let Some(x) = self.dato_da(mi, c, &v, q, false)? {
                    m.0[i].push(x);
                }
            }
        }
        // `verify`: di un oneof al massimo un campo
        for (o, nome) in msg.oneof.iter().enumerate() {
            if msg.campi.iter().zip(&m.0).filter(|(c, v)| c.oneof == Some(o) && !v.is_empty()).count() > 1 {
                return Err(format!("{nome}: multiple values"));
            }
        }
        Ok(m)
    }

    /// Il setter del oneof dopo una lettura: gli altri campi dello stesso oneof spariscono.
    fn solo_questo(&self, mi: usize, fi: usize, m: &mut Valori) {
        let campi = &self.messaggi[mi].campi;
        if let Some(o) = campi[fi].oneof {
            for (j, c) in campi.iter().enumerate() {
                if j != fi && c.oneof == Some(o) {
                    m.0[j].clear();
                }
            }
        }
    }

    fn dato_da(&self, mi: usize, c: &Campo, v: &Json, q: usize, ripetuto: bool) -> Result<Option<Dato>, String> {
        let nome = || format!(".{}.{}", self.messaggi[mi].nome, c.nome);
        Ok(Some(match c.tipo {
            Tipo::Enum(e) => return Ok(self.enums[e].da_json(v).map(Dato::Enum)),
            Tipo::Messaggio(_) if !oggetto(v) => return Err(format!("{}: object expected", nome())),
            Tipo::Messaggio(t) => Dato::Messaggio(self.da_oggetto(t, v, q + 1)?),
            Tipo::Scalare(Scalare::Bool) => Dato::Bool(vero(v)),
            Tipo::Scalare(Scalare::Double) => Dato::Double(numero_di(v)),
            Tipo::Scalare(Scalare::Int32) => Dato::Int32(a_int32(numero_di(v))),
            Tipo::Scalare(Scalare::Int64) => Dato::Int64(long_da_valore(v)?),
            Tipo::Scalare(Scalare::Stringa) => Dato::Stringa(stringa_di(v)),
            Tipo::Scalare(Scalare::Bytes) => match byte_da(v)? {
                Some(b) => Dato::Bytes(b),
                None if ripetuto => return Err(format!("{}: buffer[] expected", nome())),
                None => return Ok(None),
            },
        }))
    }

    /// `encode`: i campi in ordine di numero, i ripetuti uno per uno.
    fn scrivi(&self, mi: usize, m: &Valori, out: &mut Vec<u8>) {
        let msg = &self.messaggi[mi];
        for &fi in &msg.per_id {
            let c = &msg.campi[fi];
            for d in &m.0[fi] {
                varint(out, u64::from(c.id << 3 | c.tipo.filo()));
                match (d, c.tipo) {
                    (Dato::Bool(b), _) => varint(out, u64::from(*b)),
                    // gli int32 negativi occupano dieci byte, come int64
                    (Dato::Int32(v) | Dato::Enum(v), _) => varint(out, i64::from(*v) as u64),
                    (Dato::Int64(v), _) => varint(out, *v as u64),
                    (Dato::Double(v), _) => out.extend_from_slice(&v.to_le_bytes()),
                    (Dato::Stringa(s), _) => con_lunghezza(out, s.as_bytes()),
                    (Dato::Bytes(b), _) => con_lunghezza(out, b),
                    (Dato::Messaggio(sotto), Tipo::Messaggio(t)) => {
                        let mut dentro = Vec::new();
                        self.scrivi(t, sotto, &mut dentro);
                        con_lunghezza(out, &dentro);
                    }
                    (Dato::Messaggio(_), _) => unreachable!("un messaggio ha sempre il tipo di un messaggio"),
                }
            }
        }
    }

    /// `decode`: `l` è la lunghezza di un messaggio annidato, `None` per tutto il buffer; un messaggio singolo visto due volte si fonde.
    fn leggi(&self, mi: usize, r: &mut Lettore, l: Option<usize>, q: usize, m: &mut Valori) -> Result<(), String> {
        if q > LIMITE {
            return Err("max depth exceeded".into());
        }
        let prima = r.len;
        let fine = match l {
            None => r.len,
            Some(l) if r.pos + l > r.len => return Err("index out of range".into()),
            Some(l) => r.pos + l,
        };
        r.len = fine;
        let msg = &self.messaggi[mi];
        while r.pos < fine {
            let t = r.tag()?;
            // senza campi protobufjs passa il tag intero come numero del campo
            if msg.campi.is_empty() {
                r.salta_tipo(t & 7, q, t)?;
                continue;
            }
            let (id, filo) = (t >> 3, t & 7);
            let letto = match msg.campi.iter().position(|c| c.id == id) {
                Some(fi) => self.leggi_campo(mi, fi, filo, r, q, m)?,
                None => false,
            };
            if !letto {
                r.salta_tipo(filo, q, id)?;
            }
        }
        if l.is_some() {
            if r.pos != fine {
                return Err("index out of range".into());
            }
            r.len = prima;
        }
        Ok(())
    }

    /// Un campo noto; `false` se il tipo di filo non è il suo, e allora si salta come uno sconosciuto.
    fn leggi_campo(&self, mi: usize, fi: usize, filo: u32, r: &mut Lettore, q: usize, m: &mut Valori) -> Result<bool, String> {
        let c = &self.messaggi[mi].campi[fi];
        let noto = |e: usize, v: i32| self.enums[e].nome_di(v).is_some();
        if c.ripetuto && filo == 2 && c.tipo.impacchettabile() {
            let n = r.size()?;
            let fine = r.pos + n;
            if fine > r.len {
                return Err(r.fuori(n));
            }
            let valori = &mut m.0[fi];
            if c.tipo == Tipo::Scalare(Scalare::Double) {
                for _ in 0..n / 8 {
                    valori.push(Dato::Double(r.double()?));
                }
            } else {
                let prima = r.len;
                r.len = fine;
                while r.pos < fine {
                    match c.tipo {
                        Tipo::Enum(e) => {
                            let v = r.int32()?;
                            if noto(e, v) {
                                valori.push(Dato::Enum(v));
                            }
                        }
                        Tipo::Scalare(Scalare::Bool) => valori.push(Dato::Bool(r.bool()?)),
                        Tipo::Scalare(Scalare::Int64) => valori.push(Dato::Int64(r.int64()?)),
                        _ => valori.push(Dato::Int32(r.int32()?)),
                    }
                }
                r.len = prima;
            }
            if r.pos != fine {
                return Err("index out of range".into());
            }
            return Ok(true);
        }
        if filo != c.tipo.filo() {
            return Ok(false);
        }
        let d = match c.tipo {
            Tipo::Messaggio(t) if !c.ripetuto => {
                let l = r.size()?;
                if m.0[fi].is_empty() {
                    m.0[fi].push(Dato::Messaggio(Valori::vuoti(self.messaggi[t].campi.len())));
                }
                let Some(Dato::Messaggio(sotto)) = m.0[fi].first_mut() else { unreachable!("un campo messaggio tiene un messaggio") };
                self.leggi(t, r, Some(l), q + 1, sotto)?;
                self.solo_questo(mi, fi, m);
                return Ok(true);
            }
            Tipo::Messaggio(t) => {
                let l = r.size()?;
                let mut sotto = Valori::vuoti(self.messaggi[t].campi.len());
                self.leggi(t, r, Some(l), q + 1, &mut sotto)?;
                Dato::Messaggio(sotto)
            }
            Tipo::Enum(e) => {
                let v = r.int32()?;
                if !noto(e, v) {
                    return Ok(true);
                }
                Dato::Enum(v)
            }
            Tipo::Scalare(Scalare::Bool) => Dato::Bool(r.bool()?),
            Tipo::Scalare(Scalare::Bytes) => Dato::Bytes(r.bytes()?.to_vec()),
            Tipo::Scalare(Scalare::Double) => Dato::Double(r.double()?),
            Tipo::Scalare(Scalare::Int32) => Dato::Int32(r.int32()?),
            Tipo::Scalare(Scalare::Int64) => Dato::Int64(r.int64()?),
            Tipo::Scalare(Scalare::Stringa) => Dato::Stringa(String::from_utf8_lossy(r.bytes()?).into_owned()),
        };
        if !c.ripetuto {
            m.0[fi].clear();
        }
        m.0[fi].push(d);
        self.solo_questo(mi, fi, m);
        Ok(true)
    }

    /// `toObject` con `{longs: String, enums: String, bytes: String}`: i campi presenti in ordine di numero, i ripetuti solo se non vuoti.
    fn in_oggetto(&self, mi: usize, m: &Valori, q: usize) -> Result<Json, String> {
        let msg = &self.messaggi[mi];
        if msg.campi.is_empty() {
            return Ok(Json::obj());
        }
        if q > LIMITE {
            return Err("max depth exceeded".into());
        }
        let mut d = Mappa::new();
        for &fi in &msg.per_id {
            let c = &msg.campi[fi];
            let valori = &m.0[fi];
            if valori.is_empty() {
                continue;
            }
            let v = if c.ripetuto {
                Json::Arr(valori.iter().map(|x| self.in_json(x, c.tipo, q)).collect::<Result<_, _>>()?)
            } else {
                self.in_json(&valori[0], c.tipo, q)?
            };
            d.insert(c.nome.as_str(), v);
        }
        Ok(Json::Obj(d))
    }

    fn in_json(&self, d: &Dato, tipo: Tipo, q: usize) -> Result<Json, String> {
        Ok(match (d, tipo) {
            (Dato::Bool(b), _) => Json::Bool(*b),
            (Dato::Bytes(b), _) => Json::Str(base64::engine::general_purpose::STANDARD.encode(b)),
            (Dato::Double(x), _) => Json::Num(*x),
            (Dato::Int32(x), _) => Json::Num(f64::from(*x)),
            (Dato::Int64(x), _) => Json::Str(x.to_string()),
            (Dato::Stringa(s), _) => Json::Str(s.clone()),
            (Dato::Enum(v), Tipo::Enum(e)) => self.enums[e].nome_di(*v).map_or(Json::Num(f64::from(*v)), Json::from),
            (Dato::Messaggio(sotto), Tipo::Messaggio(t)) => self.in_oggetto(t, sotto, q + 1)?,
            _ => unreachable!("il valore ha il tipo del suo campo"),
        })
    }
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn con_lunghezza(out: &mut Vec<u8>, b: &[u8]) {
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

// ---- il lettore -------------------------------------------------------------------------------------------------------------

/// Il `Reader` di protobufjs: `len` si accorcia mentre si legge un messaggio annidato, e ogni lettura resta entro `len`.
struct Lettore<'a> {
    buf: &'a [u8],
    pos: usize,
    len: usize,
}

impl<'a> Lettore<'a> {
    fn fuori(&self, n: usize) -> String {
        format!("index out of range: {} + {n} > {}", self.pos, self.len)
    }

    fn resto(&self) -> usize {
        self.len.saturating_sub(self.pos)
    }

    /// `readVarint32NearEnd`: vicino alla fine al massimo quattro byte.
    fn varint32_in_fondo(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for i in 0..4 {
            if self.pos >= self.len {
                return Err(self.fuori(1));
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            v |= u32::from(b & 127) << (i * 7);
            if b < 128 {
                return Ok(v);
            }
        }
        Err(self.fuori(1))
    }

    /// I primi quattro byte di `uint32`, `tag` e `size`: il valore se finisce lì, altrimenti quello parziale e la posizione del quinto.
    fn inizio32(&mut self) -> Result<Result<u32, (u32, usize)>, String> {
        if self.resto() < 5 {
            if self.pos >= self.len {
                return Err(self.fuori(1));
            }
            if self.buf[self.pos] >= 128 {
                return self.varint32_in_fondo().map(Ok);
            }
        }
        let (mut v, mut pos) = (0u32, self.pos);
        for i in 0..4 {
            let b = self.buf[pos];
            pos += 1;
            v |= u32::from(b & 127) << (i * 7);
            if b < 128 {
                self.pos = pos;
                return Ok(Ok(v));
            }
        }
        Ok(Err((v, pos)))
    }

    fn uint32(&mut self) -> Result<u32, String> {
        let (mut v, mut pos) = match self.inizio32()? {
            Ok(v) => return Ok(v),
            Err(p) => p,
        };
        let b = self.buf[pos];
        pos += 1;
        v |= u32::from(b & 15) << 28;
        if b < 128 {
            self.pos = pos;
            return Ok(v);
        }
        for _ in 0..5 {
            if pos >= self.len {
                self.pos = pos;
                return Err(self.fuori(1));
            }
            let b = self.buf[pos];
            pos += 1;
            if b < 128 {
                self.pos = pos;
                return Ok(v);
            }
        }
        self.pos = pos;
        Err("invalid varint encoding".into())
    }

    fn int32(&mut self) -> Result<i32, String> {
        self.uint32().map(|v| v as i32)
    }

    fn tag(&mut self) -> Result<u32, String> {
        let (v, pos) = match self.inizio32()? {
            Ok(v) => return Ok(v),
            Err(p) => p,
        };
        let b = self.buf[pos];
        self.pos = pos + 1;
        if b < 128 && b & 112 == 0 { Ok(v | u32::from(b & 15) << 28) } else { Err("invalid tag encoding".into()) }
    }

    fn size(&mut self) -> Result<usize, String> {
        let (v, pos) = match self.inizio32()? {
            Ok(v) => return Ok(v as usize),
            Err(p) => p,
        };
        let b = self.buf[pos];
        self.pos = pos + 1;
        if b > 7 {
            return Err("invalid length encoding".into());
        }
        Ok((v | u32::from(b) << 28) as usize)
    }

    /// `readLongVarint`.
    fn int64(&mut self) -> Result<i64, String> {
        let (mut lo, mut hi) = (0u32, 0u32);
        let unisci = |lo: u32, hi: u32| (u64::from(hi) << 32 | u64::from(lo)) as i64;
        if self.resto() > 4 {
            for i in 0..4 {
                let b = self.buf[self.pos];
                self.pos += 1;
                lo |= u32::from(b & 127) << (i * 7);
                if b < 128 {
                    return Ok(unisci(lo, hi));
                }
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            lo |= u32::from(b & 127) << 28;
            hi |= u32::from(b & 127) >> 4;
            if b < 128 {
                return Ok(unisci(lo, hi));
            }
        } else {
            for i in 0..4 {
                if self.pos >= self.len {
                    return Err(self.fuori(1));
                }
                let b = self.buf[self.pos];
                self.pos += 1;
                lo |= u32::from(b & 127) << (i * 7);
                if b < 128 {
                    return Ok(unisci(lo, hi));
                }
            }
            return Err(self.fuori(1));
        }
        let controlla = self.resto() <= 4;
        for i in 0..5 {
            if controlla && self.pos >= self.len {
                return Err(self.fuori(1));
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            hi |= u32::from(b & 127) << (i * 7 + 3);
            if b < 128 {
                return Ok(unisci(lo, hi));
            }
        }
        Err("invalid varint encoding".into())
    }

    fn bool(&mut self) -> Result<bool, String> {
        let mut v = false;
        for _ in 0..10 {
            if self.pos >= self.len {
                return Err(self.fuori(1));
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            v |= b & 127 != 0;
            if b < 128 {
                return Ok(v);
            }
        }
        Err("invalid varint encoding".into())
    }

    fn double(&mut self) -> Result<f64, String> {
        if self.pos + 8 > self.len {
            return Err(self.fuori(4));
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.buf[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(f64::from_le_bytes(b))
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.size()?;
        if self.pos + n > self.len {
            return Err(self.fuori(n));
        }
        let b = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(b)
    }

    fn salta(&mut self, n: usize) -> Result<(), String> {
        if self.pos + n > self.len {
            return Err(self.fuori(n));
        }
        self.pos += n;
        Ok(())
    }

    /// `skipType`: un valore sconosciuto del tipo di filo dato; i gruppi fino al loro tag di chiusura.
    fn salta_tipo(&mut self, filo: u32, profondita: usize, numero: u32) -> Result<(), String> {
        if profondita > LIMITE {
            return Err("max depth exceeded".into());
        }
        if numero == 0 {
            return Err("illegal tag: field number 0".into());
        }
        match filo {
            0 => loop {
                if self.pos >= self.len {
                    return Err(self.fuori(1));
                }
                let b = self.buf[self.pos];
                self.pos += 1;
                if b & 128 == 0 {
                    return Ok(());
                }
            },
            1 => self.salta(8),
            2 => {
                let n = self.size()?;
                self.salta(n)
            }
            3 => loop {
                let t = self.tag()?;
                let (annidato, f) = (t >> 3, t & 7);
                if annidato == 0 {
                    return Err("illegal tag: field number 0".into());
                }
                if f == 4 {
                    return if annidato == numero { Ok(()) } else { Err("invalid end group tag".into()) };
                }
                self.salta_tipo(f, profondita + 1, annidato)?;
            },
            5 => self.salta(4),
            _ => Err(format!("invalid wire type {filo} at offset {}", self.pos)),
        }
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    impl Schema {
        /// Lo schema nel formato del generatore delle prove: messaggi con i campi e il tipo risolto, enum con i valori, in ordine.
        fn descrivi(&self, ns: Option<usize>, righe: &mut Vec<String>) {
            for (_, n) in self.figli(ns) {
                match *n {
                    Nodo::Messaggio(m) => {
                        righe.push(format!("M .{}", self.messaggi[m].nome));
                        for c in &self.messaggi[m].campi {
                            let tipo = match c.tipo {
                                Tipo::Scalare(s) => format!("{s:?}").to_lowercase().replace("stringa", "string"),
                                Tipo::Messaggio(t) => format!(".{}", self.messaggi[t].nome),
                                Tipo::Enum(e) => format!(".{}", self.enums[e].nome),
                            };
                            righe.push(format!("F {} {} {} {tipo}", c.nome, c.id, if c.ripetuto { "R" } else { "O" }));
                        }
                        self.descrivi(Some(m), righe);
                    }
                    Nodo::Enum(e) => {
                        let valori: Vec<String> = self.enums[e].valori.iter().map(|(k, v)| format!("{k}={v}")).collect();
                        righe.push(format!("E .{} {}", self.enums[e].nome, valori.join(",")));
                    }
                    Nodo::Altro => {}
                }
            }
        }
    }

    /// Ogni tipo risolto come lo risolve protobufjs: tests/dati/protobuf-schema.txt viene dal generatore delle prove.
    #[test]
    fn risolve_lo_schema_come_protobufjs() {
        let mut righe = Vec::new();
        SCHEMA.descrivi(None, &mut righe);
        let attese: Vec<&str> = include_str!("../tests/dati/protobuf-schema.txt").lines().collect();
        for (i, (a, b)) in righe.iter().zip(&attese).enumerate() {
            assert_eq!(a, b, "riga {}", i + 1);
        }
        assert_eq!(righe.len(), attese.len());
    }

    #[test]
    fn long_come_long_js() {
        let t = |s: &str| long_da_testo(&s.encode_utf16().collect::<Vec<_>>());
        assert_eq!(t("9007199254740993"), Ok(9_007_199_254_740_993));
        assert_eq!(t("-9223372036854775808"), Ok(i64::MIN));
        assert_eq!(t("18446744073709551615"), Ok(-1));
        assert_eq!(t("12abc345"), Ok(12));
        assert_eq!(t("--5"), Ok(5));
        assert!(t("1-2").is_err() && t("").is_err());
        assert_eq!(long_da_numero(1e20), i64::MAX);
        assert_eq!(long_da_numero(-2.7), -2);
    }
}
