//! The tool list, as the TypeScript server published it (tools.json, recorded from its tools/list), and the input
//! check the SDK ran there with the zod schemas of src/lib/mcp/server.ts: zod 4's rules and messages for the subset of
//! JSON Schema these tools use. Unknown keys are dropped, as z.object does.

use serde_json::{Map, Value};
use std::sync::LazyLock;

pub static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| serde_json::from_str(include_str!("tools.json")).expect("tools.json"));

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

pub fn tool(name: &str) -> Option<&'static Value> {
    TOOLS.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(name))
}

pub fn received(v: Option<&Value>) -> &'static str {
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

fn num(v: &Value) -> String {
    crate::js::string(Some(v))
}

struct Issue {
    path: Vec<String>,
    message: String,
}

fn check(schema: &Value, value: Option<&Value>, path: &[String], issues: &mut Vec<Issue>) -> Option<Value> {
    let push = |issues: &mut Vec<Issue>, message: String| issues.push(Issue { path: path.to_vec(), message });
    let kind = schema.get("type").and_then(Value::as_str).unwrap_or("");
    match (kind, value) {
        ("string", Some(Value::String(s))) => {
            // every string check runs: a long blank name breaks maxLength and the pattern
            let n = s.encode_utf16().count() as u64;
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64).filter(|m| n < *m) {
                push(issues, format!("Too small: expected string to have >={min} characters"));
            }
            if let Some(max) = schema.get("maxLength").and_then(Value::as_u64).filter(|m| n > *m) {
                push(issues, format!("Too big: expected string to have <={max} characters"));
            }
            if let Some(p) = schema.get("pattern").and_then(Value::as_str)
                && !regex::Regex::new(p).is_ok_and(|re| re.is_match(s))
            {
                push(issues, format!("Invalid string: must match pattern /{p}/"));
            }
            value.cloned()
        }
        ("integer", Some(Value::Number(x))) => {
            let f = x.as_f64().unwrap_or(f64::NAN);
            if f.fract() != 0.0 {
                push(issues, "Invalid input: expected int, received number".into());
            } else if f > MAX_SAFE {
                push(issues, "Too big: expected int to be <=9007199254740991".into());
            } else if f < -MAX_SAFE {
                push(issues, "Too small: expected int to be >=-9007199254740991".into());
            } else if let Some(min) = schema.get("minimum").filter(|m| m.as_f64().is_some_and(|m| f < m)) {
                push(issues, format!("Too small: expected number to be >={}", num(min)));
            } else if let Some(max) = schema.get("maximum").filter(|m| m.as_f64().is_some_and(|m| f > m)) {
                push(issues, format!("Too big: expected number to be <={}", num(max)));
            }
            value.cloned()
        }
        ("boolean", Some(Value::Bool(_))) => value.cloned(),
        ("array", Some(Value::Array(items))) => {
            let mut out = vec![];
            let item_schema = schema.get("items").cloned().unwrap_or(Value::Null);
            for (i, item) in items.iter().enumerate() {
                let mut p = path.to_vec();
                p.push(i.to_string());
                out.push(check(&item_schema, Some(item), &p, issues).unwrap_or(Value::Null));
            }
            let n = items.len() as u64;
            if let Some(min) = schema.get("minItems").and_then(Value::as_u64).filter(|m| n < *m) {
                push(issues, format!("Too small: expected array to have >={min} items"));
            } else if let Some(max) = schema.get("maxItems").and_then(Value::as_u64).filter(|m| n > *m) {
                push(issues, format!("Too big: expected array to have <={max} items"));
            }
            Some(Value::Array(out))
        }
        ("object", Some(Value::Object(o))) => Some(Value::Object(check_object(schema, o, path, issues))),
        _ => {
            let expected = if kind == "integer" { "number" } else { kind };
            push(issues, format!("Invalid input: expected {expected}, received {}", received(value)));
            None
        }
    }
}

fn check_object(schema: &Value, o: &Map<String, Value>, path: &[String], issues: &mut Vec<Issue>) -> Map<String, Value> {
    let empty = Map::new();
    let props = schema.get("properties").and_then(Value::as_object).unwrap_or(&empty);
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut out = Map::new();
    for (k, s) in props {
        if !o.contains_key(k) && !required.contains(&k.as_str()) {
            continue;
        }
        let mut p = path.to_vec();
        p.push(k.clone());
        if let Some(v) = check(s, o.get(k), &p, issues) {
            out.insert(k.clone(), v);
        }
    }
    out
}

/// The arguments as the schema outputs them (absent: an empty object), or the zod-style message ("path: message, ...").
pub fn validate(tool: &Value, args: Option<&Value>) -> Result<Map<String, Value>, String> {
    let schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);
    let mut issues = vec![];
    let empty = Value::Object(Map::new());
    let out = match args.unwrap_or(&empty) {
        Value::Object(o) => check_object(&schema, o, &[], &mut issues),
        other => {
            issues.push(Issue { path: vec![], message: format!("Invalid input: expected object, received {}", received(Some(other))) });
            Map::new()
        }
    };
    if issues.is_empty() {
        return Ok(out);
    }
    Err(issues
        .iter()
        .map(|i| if i.path.is_empty() { i.message.clone() } else { format!("{}: {}", i.path.join("."), i.message) })
        .collect::<Vec<_>>()
        .join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v(name: &str, args: Value) -> Result<Map<String, Value>, String> {
        validate(tool(name).unwrap(), Some(&args))
    }

    #[test]
    fn the_messages_zod_gave() {
        // recorded from the TypeScript server and from the SDK 2.4 with the same zod schemas
        assert_eq!(v("list_chats", json!({"account": 0})).unwrap_err(), "account: Too small: expected number to be >=1");
        assert_eq!(v("read_chat", json!({"chat": " "})).unwrap_err(), "chat: Invalid string: must match pattern /\\S/");
        assert_eq!(v("read_chat", json!({"account": 1e20, "chat": "x"})).unwrap_err(), "account: Too big: expected int to be <=9007199254740991");
        assert_eq!(v("read_chat", json!({"account": 0.5, "chat": "x"})).unwrap_err(), "account: Invalid input: expected int, received number");
        assert_eq!(v("read_chat", json!({"account": "1", "chat": "x"})).unwrap_err(), "account: Invalid input: expected number, received string");
        assert_eq!(
            v("read_chat", json!({"chat": " ".repeat(201)})).unwrap_err(),
            "chat: Too big: expected string to have <=200 characters, chat: Invalid string: must match pattern /\\S/"
        );
        assert_eq!(v("read_chat", json!({})).unwrap_err(), "chat: Invalid input: expected string, received undefined");
        assert_eq!(validate(tool("read_chat").unwrap(), None).unwrap_err(), "chat: Invalid input: expected string, received undefined");
        assert_eq!(
            v("send_message", json!({"chat": "x", "mentions": ["", 5], "reply_to": ""})).unwrap_err(),
            "reply_to: Too small: expected string to have >=1 characters, mentions.0: Too small: expected string to have >=1 characters, mentions.1: Invalid input: expected string, received number"
        );
        assert_eq!(v("send_message", json!({"chat": "x", "mentions": "a"})).unwrap_err(), "mentions: Invalid input: expected array, received string");
        assert_eq!(v("list_chats", json!({"unread_only": null})).unwrap_err(), "unread_only: Invalid input: expected boolean, received null");
    }

    #[test]
    fn unknown_keys_dropped() {
        assert_eq!(v("read_chat", json!({"chat": "x", "zz": 1})).unwrap(), json!({"chat": "x"}).as_object().unwrap().clone());
        assert_eq!(v("list_accounts", json!({"a": 1})).unwrap(), Map::new());
        assert_eq!(TOOLS.len(), 11);
    }
}
