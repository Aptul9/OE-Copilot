//! Skills over MCP (io.modelcontextprotocol/skills): the guides in src/skills, built into the binary and served as
//! resources skill://<name>/SKILL.md. Each manifest is computed from the bytes served, so it changes with the deploy.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

const FILES: [(&str, &str); 4] = [
    ("accounts", include_str!("skills/accounts/SKILL.md")),
    ("chats", include_str!("skills/chats/SKILL.md")),
    ("mail-sweep", include_str!("skills/mail-sweep/SKILL.md")),
    ("safe-send", include_str!("skills/safe-send/SKILL.md")),
];

pub struct Skill {
    pub name: &'static str,
    pub uri: String,
    pub text: &'static str,
    pub frontmatter: Map<String, Value>,
}

/// The `key: value` lines between the opening and closing `---`.
fn frontmatter(text: &str) -> Map<String, Value> {
    let mut out = Map::new();
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return out;
    }
    for line in lines.take_while(|l| *l != "---") {
        if let Some((k, v)) = line.split_once(':') {
            out.insert(k.trim().to_string(), json!(v.trim()));
        }
    }
    out
}

pub static SKILLS: LazyLock<Vec<Skill>> =
    LazyLock::new(|| FILES.iter().map(|(name, text)| Skill { name, uri: format!("skill://{name}/SKILL.md"), text, frontmatter: frontmatter(text) }).collect());

fn find(uri: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.uri == uri)
}

/// The skills/list entry: SKILL.md is the whole manifest.
fn entry(s: &Skill) -> Value {
    let digest: String = Sha256::digest(s.text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    json!({
        "uri": s.uri,
        "frontmatter": s.frontmatter,
        "resources": [{"uri": s.uri, "digest": format!("sha256:{digest}"), "size": s.text.len()}],
    })
}

pub fn list() -> Vec<Value> {
    SKILLS.iter().map(entry).collect()
}

pub fn get(uri: &str) -> Option<Value> {
    find(uri).map(entry)
}

/// resources/list
pub fn resources() -> Vec<Value> {
    SKILLS
        .iter()
        .map(|s| {
            let description = s.frontmatter.get("description").cloned().unwrap_or(Value::Null);
            json!({"uri": s.uri, "name": format!("{}/SKILL.md", s.name), "description": description, "mimeType": "text/markdown"})
        })
        .collect()
}

/// resources/read: the contents, or None for a URI this server does not serve.
pub fn read(uri: &str) -> Option<Value> {
    find(uri).map(|s| json!([{"uri": s.uri, "mimeType": "text/markdown", "text": s.text}]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skill_is_valid() {
        for s in SKILLS.iter() {
            // the Agent Skills rule: the directory is the name, and name and description are present
            assert_eq!(s.frontmatter.get("name").and_then(Value::as_str), Some(s.name), "{}", s.name);
            assert!(s.frontmatter.get("description").and_then(Value::as_str).is_some_and(|d| !d.is_empty() && d.len() <= 1024), "{}", s.name);
            assert!(s.text.ends_with('\n'), "{}", s.name);
        }
        assert_eq!(list().len(), FILES.len());
    }

    #[test]
    fn manifest_matches_the_bytes() {
        let e = get("skill://mail-sweep/SKILL.md").unwrap();
        let text = read("skill://mail-sweep/SKILL.md").unwrap()[0]["text"].as_str().unwrap().to_string();
        let digest: String = Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(e["resources"][0]["digest"], json!(format!("sha256:{digest}")));
        assert_eq!(e["resources"][0]["size"], json!(text.len()));
        assert!(get("skill://nope/SKILL.md").is_none());
        assert!(read("skill://mail-sweep/other.md").is_none());
    }
}
