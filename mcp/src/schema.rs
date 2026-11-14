//! The tool list, as the TypeScript server published it (tools.json, recorded from its tools/list), and the input
//! check that ran there: zod's rules and messages for the subset of JSON Schema these tools use, defaults filled in.

use serde_json::{Map, Value};
use std::sync::LazyLock;

pub static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| serde_json::from_str(include_str!("tools.json")).expect("tools.json"));

pub fn tool(name: &str) -> Option<&'static Value> {
    TOOLS.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(name))
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

fn num(v: &Value) -> String {
    crate::js::string(Some(v))
}

struct Issue {
    path: Vec<String>,
    message: String,
}

fn check(schema: &Value, value: Option<&Value>, path: &[String], issues: &mut Vec<Issue>) -> Option<Value> {
    let push = |issues: &mut Vec<Issue>, message: String| issues.push(Issue { path: path.to_vec(), message });
    let value = match value {
        None => match schema.get("default") {
            Some(d) => return Some(d.clone()),
            None => None,
        },
        v => v,
    };
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if value.is_some_and(|v| options.contains(v)) {
            return value.cloned();
        }
        let list: Vec<String> = options.iter().map(|o| o.to_string()).collect();
        push(issues, format!("Invalid option: expected one of {}", list.join("|")));
        return None;
    }
    let kind = schema.get("type").and_then(Value::as_str).unwrap_or("");
    match (kind, value) {
        ("string", Some(Value::String(s))) => {
            let n = s.encode_utf16().count() as u64;
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64).filter(|m| n < *m) {
                push(issues, format!("Too small: expected string to have >={min} characters"));
            } else if let Some(max) = schema.get("maxLength").and_then(Value::as_u64).filter(|m| n > *m) {
                push(issues, format!("Too big: expected string to have <={max} characters"));
            }
            value.cloned()
        }
        ("integer", Some(Value::Number(x))) => {
            let f = x.as_f64().unwrap_or(f64::NAN);
            if f.fract() != 0.0 {
                push(issues, "Invalid input: expected int, received number".into());
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
            let expected = match kind {
                "integer" => "int",
                k => k,
            };
            let expected = if kind == "integer" && !matches!(value, Some(Value::Number(_))) { "number" } else { expected };
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
        // optional and absent: nothing to check, unless it has a default
        if !o.contains_key(k) && !required.contains(&k.as_str()) && s.get("default").is_none() {
            continue;
        }
        let mut p = path.to_vec();
        p.push(k.clone());
        if let Some(v) = check(s, o.get(k), &p, issues) {
            out.insert(k.clone(), v);
        }
    }
    let unknown: Vec<String> = o.keys().filter(|k| !props.contains_key(*k)).map(|k| format!("\"{k}\"")).collect();
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) && !unknown.is_empty() {
        let message = if unknown.len() == 1 { format!("Unrecognized key: {}", unknown[0]) } else { format!("Unrecognized keys: {}", unknown.join(", ")) };
        issues.push(Issue { path: path.to_vec(), message });
    }
    out
}

/// The arguments with defaults filled in, or the zod-style message ("path: message, ...").
pub fn validate(tool: &Value, args: Option<&Value>) -> Result<Map<String, Value>, String> {
    let schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);
    let mut issues = vec![];
    let empty = Value::Object(Map::new());
    let args = args.unwrap_or(&empty);
    let out = match args {
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
        // the answers recorded from the TypeScript server
        assert_eq!(v("email_list", json!({})).unwrap_err(), "account_id: Invalid input: expected string, received undefined");
        assert_eq!(v("email_list", json!({"account_id": "x:y", "zzz": 1})).unwrap_err(), "Unrecognized key: \"zzz\"");
        assert_eq!(v("email_list", json!({"account_id": "x:y", "limit": 0})).unwrap_err(), "limit: Too small: expected number to be >=1");
        assert_eq!(v("email_list", json!({"account_id": "x:y", "limit": 1.5})).unwrap_err(), "limit: Invalid input: expected int, received number");
        assert_eq!(
            v("email_manage", json!({"account_id": "x:y", "message_id": "a", "action": "zap"})).unwrap_err(),
            "action: Invalid option: expected one of \"read\"|\"unread\"|\"archive\"|\"delete\"|\"flag\"|\"unflag\""
        );
        assert_eq!(v("email_list", json!({"account_id": ""})).unwrap_err(), "account_id: Too small: expected string to have >=1 characters");
        assert_eq!(
            v("chat_send", json!({"account_id": "x:y", "conversation_id": "c", "text": 5})).unwrap_err(),
            "text: Invalid input: expected string, received number"
        );
    }

    #[test]
    fn defaults_and_nesting() {
        let ok = v("email_send", json!({"account_id": "a", "to": ["x@y.z"], "subject": "s", "text": "t"})).unwrap();
        assert_eq!(ok.get("draft"), Some(&json!(false)));
        assert_eq!(v("email_list", json!({"account_id": "a"})).unwrap().get("limit"), Some(&json!(20)));
        assert_eq!(v("email_forward", json!({"account_id": "a", "message_id": "m", "to": ["x"]})).unwrap().get("text"), Some(&json!("")));
        assert_eq!(
            v("email_send", json!({"account_id": "a", "to": [], "subject": "s", "text": "t"})).unwrap_err(),
            "to: Too small: expected array to have >=1 items"
        );
        assert_eq!(
            v("email_send", json!({"account_id": "a", "to": ["x"], "subject": "s", "text": "t", "files": [{"name": "n", "data": "x"}]})).unwrap_err(),
            "files.0.mime_type: Invalid input: expected string, received undefined"
        );
        assert_eq!(validate(tool("email_list").unwrap(), Some(&json!([1]))).unwrap_err(), "Invalid input: expected object, received array");
        assert_eq!(TOOLS.len(), 26);
    }
}
