//! The page scripts of the agent (page-scripts.json, made by scripts/page-scripts.mjs from src/agent/teams/scripts):
//! each function as the source text Playwright sends for the Node agent, and the selectors and texts they take, a
//! RegExp kept as {"$re": [source, flags]} and made a RegExp again in the page.

use serde_json::Value;
use std::sync::LazyLock;

static JSON: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(include_str!("page-scripts.json")).expect("page-scripts.json"));

/// Makes the RegExps of an argument again, in the page.
const REVIVE: &str = r#"const R = (v) => v === null || typeof v !== "object" ? v : Array.isArray(v) ? v.map(R) : Array.isArray(v.$re) && Object.keys(v).length === 1 ? new RegExp(v.$re[0], v.$re[1]) : Object.fromEntries(Object.entries(v).map(([k, x]) => [k, R(x)]));"#;

/// The source of script `name` ("page-state.makeVisible").
pub fn source(name: &str) -> &'static str {
    JSON["scripts"][name].as_str().unwrap_or_else(|| panic!("no page script {name}"))
}

/// Every page script, each a function the page can compile: the number of them.
pub fn count() -> usize {
    JSON["scripts"]
        .as_object()
        .map_or(0, |m| m.values().filter(|v| v.as_str().is_some_and(|s| s.starts_with("function ") || s.starts_with("async function "))).count())
}

/// SEL as the page scripts take it.
pub fn sel() -> Value {
    JSON["sel"].clone()
}

/// TEXTS as the page scripts take it.
pub fn texts() -> Value {
    JSON["texts"].clone()
}

/// {s: SEL, t: TEXTS}, the argument of most scripts, with more fields.
pub fn st(more: Value) -> Value {
    let mut v = serde_json::json!({ "s": sel(), "t": texts() });
    if let Value::Object(m) = more {
        for (k, x) in m {
            v[k] = x;
        }
    }
    v
}

/// A selector of SEL, as text ("editor").
pub fn s(key: &str) -> String {
    JSON["sel"][key].as_str().unwrap_or_else(|| panic!("no selector {key}")).to_string()
}

/// One entry of another table of selectors.ts (actions, bar_reactions, picker_reactions).
pub fn table(table: &str, key: &str) -> Option<String> {
    JSON[table][key].as_str().map(String::from)
}

/// A list of selectors.ts (teams_hosts, login_hosts).
pub fn list(name: &str) -> Vec<String> {
    JSON[name].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

/// A RegExp of TEXTS (or of SEL) as a regex of Rust: same source, its i flag.
pub fn regex(table: &str, key: &str) -> regex::Regex {
    let r = &JSON[table][key]["$re"];
    let (src, flags) = (r[0].as_str().unwrap_or_else(|| panic!("no RegExp {table}.{key}")), r[1].as_str().unwrap_or(""));
    regex::Regex::new(&format!("{}{src}", if flags.contains('i') { "(?i)" } else { "" })).expect("a RegExp of selectors.ts")
}

/// A text of TEXTS ("selfChat").
pub fn text(key: &str) -> String {
    JSON["texts"][key].as_str().unwrap_or_else(|| panic!("no text {key}")).to_string()
}

/// The expression that runs script `name` in the page with `arg` (none: called without one), its promise awaited.
pub fn call(name: &str, arg: Option<&Value>) -> String {
    call_source(source(name), arg)
}

/// The same for a function given as source.
pub fn call_source(src: &str, arg: Option<&Value>) -> String {
    match arg {
        Some(a) => format!("(async () => {{ {REVIVE} return await ({src})(R({})); }})()", crate::js::stringify(a)),
        None => format!("(async () => await ({src})())()"),
    }
}

/// The function that runs script `name` on an element (Playwright's locator.evaluate): `(el) => script(el, arg)`.
pub fn on_element(name: &str, arg: Option<&Value>) -> String {
    let src = source(name);
    match arg {
        Some(a) => format!("async (el) => {{ {REVIVE} return await ({src})(el, R({})); }}", crate::js::stringify(a)),
        None => format!("async (el) => await ({src})(el)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_and_the_selectors_of_the_sources() {
        assert!(source("page-state.makeVisible").starts_with("function makeVisible()"));
        assert_eq!(s("editor"), r#"[data-tid="ckeditor"]"#);
        assert_eq!(table("actions", "more").as_deref(), Some("message-actions-more"));
        assert!(list("teams_hosts").contains(&"teams.cloud.microsoft".to_string()));
        assert!(regex("texts", "outbound").is_match("You: ok") && regex("texts", "outbound").is_match("tu: ok"));
        assert_eq!(text("selfChat"), "(You)");
        assert!(call("page-state.drainInput", None).contains("function drainInput"));
        assert!(call("page-state.uncoveredPoint", Some(&Value::from("a"))).contains(r#"(R("a"))"#));
    }
}
