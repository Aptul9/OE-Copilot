//! JavaScript semantics the answers depend on: truthiness, String(), Number(), encodeURIComponent, and object spreads
//! where a key set to undefined disappears from JSON.stringify.

use serde_json::{Map, Value};

pub type Obj = Map<String, Value>;

/// `o[key] = value`: an existing key keeps its place; None removes it, as an undefined value is left out of JSON.
pub fn set(o: &mut Obj, key: &str, value: Option<Value>) {
    match value {
        Some(v) => {
            o.insert(key.to_string(), v);
        }
        None => {
            o.shift_remove(key);
        }
    }
}

pub fn obj(v: &Value) -> Obj {
    match v {
        Value::Object(m) => m.clone(),
        _ => Obj::new(),
    }
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub fn truthy_opt(v: Option<&Value>) -> bool {
    v.is_some_and(truthy)
}

/// `a || b`: the first truthy value, else the last one (None stands for undefined).
pub fn or<'a>(values: &[Option<&'a Value>]) -> Option<&'a Value> {
    let mut last = None;
    for v in values {
        if truthy_opt(*v) {
            return *v;
        }
        last = *v;
    }
    last
}

/// `v ?? fallback` for a present-but-null check: None or null gives None.
pub fn present(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| !x.is_null())
}

pub fn number_to_string(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == f.trunc() && f.abs() < 1e21 {
        return format!("{}", f as i128);
    }
    format!("{f}")
}

/// `String(v)`; None is "undefined".
pub fn string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        },
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { string(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `Number(v)`; None is undefined (NaN).
pub fn number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() { 0.0 } else { t.parse::<f64>().unwrap_or(f64::NAN) }
        }
        Some(_) => f64::NAN,
    }
}

pub fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

pub fn stringify(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `decodeURIComponent`: None where JavaScript would throw (a bad escape or bytes that are not UTF-8).
pub fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `new URLSearchParams(pairs).toString()`.
pub fn search_params(pairs: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

/// Lenient base64 decoding like Node's Buffer.from(s, "base64" | "base64url"): either alphabet, padding optional, and
/// decoding stops at the first character outside both.
pub fn base64_decode_lenient(s: &str) -> Vec<u8> {
    let mut bits: u32 = 0;
    let mut n = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => break,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    out
}

pub fn base64_std(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spreads_and_strings() {
        let mut o = obj(&json!({"a": 1, "id": "x", "b": 2}));
        set(&mut o, "id", Some(json!("y")));
        set(&mut o, "b", None);
        set(&mut o, "c", Some(json!(3)));
        assert_eq!(stringify(&Value::Object(o)), r#"{"a":1,"id":"y","c":3}"#);
        assert_eq!(string(Some(&json!(5))), "5");
        assert_eq!(string(Some(&json!(1.5))), "1.5");
        assert_eq!(string(None), "undefined");
        assert_eq!(number(Some(&json!(" 42 "))), 42.0);
        assert!(number(Some(&json!("x"))).is_nan());
        assert_eq!(number(Some(&json!(""))), 0.0);
    }

    #[test]
    fn encodings() {
        assert_eq!(encode_uri_component("a b/c'(é)"), "a%20b%2Fc'(%C3%A9)");
        assert_eq!(decode_uri_component("a%2Fb%C3%A9").as_deref(), Some("a/bé"));
        assert_eq!(decode_uri_component("%zz"), None);
        assert_eq!(decode_uri_component("%C3"), None);
        assert_eq!(search_params(&[("$top", "5"), ("q", "a b,c/d:e")]), "%24top=5&q=a+b%2Cc%2Fd%3Ae");
        assert_eq!(base64_decode_lenient("aGk_"), vec![b'h', b'i', 0x3f]);
        assert_eq!(base64_decode_lenient("aGk/"), vec![b'h', b'i', 0x3f]);
        assert_eq!(base64_decode_lenient("aGVsbG8="), b"hello");
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(&json!(0)));
        assert!(!truthy(&json!("")));
        assert!(truthy(&json!([])));
        assert_eq!(or(&[Some(&json!("")), Some(&json!(0))]), Some(&json!(0)));
        assert_eq!(or(&[None, Some(&json!("a"))]), Some(&json!("a")));
    }
}
