//! The zod schemas the routes check bodies against, with zod 4's messages for the problems a client can make: wrong
//! types, too long, not an integer, not an option. A parse strips unknown keys, as z.object does.

use serde_json::{Map, Value};

pub enum Z {
    Str(Option<usize>),
    Int,
    PosInt,
    Bool,
    Enum(&'static [&'static str]),
    Nullable(Box<Z>),
    Optional(Box<Z>),
    Arr(Box<Z>, Option<usize>),
    Obj(Vec<(&'static str, Z)>),
    /// record of string keys (max length) to values, at most `n` keys (refine message)
    Record(Option<usize>, Box<Z>, Option<(usize, &'static str)>),
    /// discriminated union on `op`: option name and its object
    Union(&'static str, Vec<(&'static str, Z)>),
}

pub fn s(max: usize) -> Z {
    Z::Str(Some(max))
}
pub fn null(z: Z) -> Z {
    Z::Nullable(Box::new(z))
}
pub fn opt(z: Z) -> Z {
    Z::Optional(Box::new(z))
}
pub fn arr(z: Z, max: usize) -> Z {
    Z::Arr(Box::new(z), Some(max))
}
pub fn obj(fields: Vec<(&'static str, Z)>) -> Z {
    Z::Obj(fields)
}

fn received(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

pub struct Issue {
    pub path: Vec<String>,
    pub message: String,
}

fn check(z: &Z, v: Option<&Value>, path: &[String], out: &mut Vec<Issue>) -> Option<Value> {
    let mut issue = |m: String| out.push(Issue { path: path.to_vec(), message: m });
    match z {
        Z::Optional(inner) => match v {
            None => None,
            Some(_) => check(inner, v, path, out),
        },
        Z::Nullable(inner) => match v {
            Some(Value::Null) => Some(Value::Null),
            _ => check(inner, v, path, out),
        },
        Z::Str(max) => match v {
            Some(Value::String(t)) => {
                if let Some(m) = max.filter(|m| t.encode_utf16().count() > *m) {
                    issue(format!("Too big: expected string to have <={m} characters"));
                }
                v.cloned()
            }
            _ => {
                issue(format!("Invalid input: expected string, received {}", received(v)));
                None
            }
        },
        Z::Int | Z::PosInt => match v {
            Some(Value::Number(n)) => {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if f.fract() != 0.0 {
                    issue("Invalid input: expected int, received number".into());
                } else if f.abs() > 9_007_199_254_740_991.0 {
                    issue(format!("Too big: expected int to be {}9007199254740991", if f > 0.0 { "<=" } else { ">=-" }));
                } else if matches!(z, Z::PosInt) && f <= 0.0 {
                    issue("Too small: expected number to be >0".into());
                }
                v.cloned()
            }
            _ => {
                issue(format!("Invalid input: expected number, received {}", received(v)));
                None
            }
        },
        Z::Bool => match v {
            Some(Value::Bool(_)) => v.cloned(),
            _ => {
                issue(format!("Invalid input: expected boolean, received {}", received(v)));
                None
            }
        },
        Z::Enum(options) => match v.and_then(Value::as_str) {
            Some(t) if options.contains(&t) => v.cloned(),
            _ => {
                issue(format!("Invalid option: expected one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|")));
                None
            }
        },
        Z::Arr(item, max) => match v {
            Some(Value::Array(a)) => {
                let mut res = vec![];
                for (i, x) in a.iter().enumerate() {
                    let mut p = path.to_vec();
                    p.push(i.to_string());
                    res.push(check(item, Some(x), &p, out).unwrap_or(Value::Null));
                }
                if let Some(m) = max.filter(|m| a.len() > *m) {
                    out.push(Issue { path: path.to_vec(), message: format!("Too big: expected array to have <={m} items") });
                }
                Some(Value::Array(res))
            }
            _ => {
                issue(format!("Invalid input: expected array, received {}", received(v)));
                None
            }
        },
        Z::Obj(fields) => match v {
            Some(Value::Object(o)) => {
                let mut res = Map::new();
                for (k, fz) in fields {
                    let mut p = path.to_vec();
                    p.push(k.to_string());
                    if let Some(x) = check(fz, o.get(*k), &p, out) {
                        res.insert(k.to_string(), x);
                    }
                }
                Some(Value::Object(res))
            }
            _ => {
                issue(format!("Invalid input: expected object, received {}", received(v)));
                None
            }
        },
        Z::Record(key_max, value, limit) => match v {
            Some(Value::Object(o)) => {
                let mut res = Map::new();
                for (k, x) in o {
                    let mut p = path.to_vec();
                    p.push(k.clone());
                    if let Some(m) = key_max.filter(|m| k.encode_utf16().count() > *m) {
                        out.push(Issue { path: p.clone(), message: format!("Too big: expected string to have <={m} characters") });
                    }
                    if let Some(y) = check(value, Some(x), &p, out) {
                        res.insert(k.clone(), y);
                    }
                }
                if let Some((n, msg)) = limit.filter(|(n, _)| o.len() > *n) {
                    let _ = n;
                    out.push(Issue { path: path.to_vec(), message: msg.to_string() });
                }
                Some(Value::Object(res))
            }
            _ => {
                issue(format!("Invalid input: expected record, received {}", received(v)));
                None
            }
        },
        Z::Union(key, options) => {
            let Some(Value::Object(o)) = v else {
                issue(format!("Invalid input: expected object, received {}", received(v)));
                return None;
            };
            let tag = o.get(*key).and_then(Value::as_str);
            match options.iter().find(|(name, _)| Some(*name) == tag) {
                Some((_, z)) => check(z, v, path, out),
                None => {
                    let mut p = path.to_vec();
                    p.push(key.to_string());
                    out.push(Issue { path: p, message: "Invalid input".into() });
                    None
                }
            }
        }
    }
}

/// The value as the schema outputs it, or the issues found.
pub fn parse(z: &Z, v: &Value) -> Result<Value, Vec<Issue>> {
    let mut out = vec![];
    let r = check(z, Some(v), &[], &mut out);
    if out.is_empty() { Ok(r.unwrap_or(Value::Null)) } else { Err(out) }
}

/// "path: message; ..." of the first three issues (relayJson of src/lib/relay.ts).
pub fn first_three(issues: &[Issue]) -> String {
    issues.iter().take(3).map(|i| format!("{}: {}", i.path.join("."), i.message)).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages() {
        let z = obj(vec![("a", s(3)), ("b", opt(Z::PosInt)), ("c", null(Z::Int)), ("e", opt(Z::Enum(&["x", "y"])))]);
        assert_eq!(parse(&z, &json!({"a": "ab", "c": null, "zz": 1})).ok(), Some(json!({"a": "ab", "c": null})));
        let e = parse(&z, &json!({"a": "abcd", "b": 0, "c": 1.5, "e": "q"})).err().unwrap();
        assert_eq!(
            first_three(&e),
            "a: Too big: expected string to have <=3 characters; b: Too small: expected number to be >0; c: Invalid input: expected int, received number"
        );
    }
}
