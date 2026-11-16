//! Outlook REST v2.0 transport, one for mail and calendars, and the folder tree.

use crate::js::{Obj, encode_uri_component as q, string};
use crate::tokens::Tokens;
use crate::{Error, Result, http_client};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::time::Duration;

pub const OUTLOOK_REST: &str = "https://outlook.office.com/api/v2.0";

/// The API base: fixed, so no broker answer or tool argument can send a token anywhere else. Tests point it at a fake
/// server, with the feature that is never in the binary.
pub fn rest_base() -> String {
    #[cfg(feature = "test-hosts")]
    if let Ok(v) = std::env::var("OUTLOOK_MCP_TEST_REST") {
        return v;
    }
    OUTLOOK_REST.to_string()
}

pub struct Outlook {
    pub tokens: Tokens,
}

impl Outlook {
    pub fn new(tokens: Tokens) -> Self {
        Outlook { tokens }
    }

    pub async fn upn(&self, account: Option<&str>) -> Result<String> {
        Ok(self.tokens.pick(account, false).await?.upn)
    }

    pub async fn get(&self, account: &str, p: &str) -> Result<Value> {
        self.request(account, p, "GET", None, &[]).await
    }

    /// One request; a 401 reloads tokens from the broker once and retries. Error bodies are cut short: they may quote
    /// mail content.
    pub async fn request(&self, account: &str, p: &str, method: &str, data: Option<&Value>, extra: &[(&str, &str)]) -> Result<Value> {
        static PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^me(?:[/?]|$)").expect("fixed"));
        if !PATH.is_match(p) || p.contains(['\r', '\n', '\\']) {
            return Err(Error::provider("Invalid Outlook request path"));
        }
        let mut retry = true;
        loop {
            let a = self.tokens.pick(Some(account), !retry).await?;
            let url = format!("{}/{}", rest_base(), p.trim_start_matches('/'));
            let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| Error::provider("method"))?;
            let mut req = http_client().request(m, &url).timeout(Duration::from_secs(30));
            for (k, v) in extra {
                req = req.header(*k, *v);
            }
            req = req.header("Authorization", format!("Bearer {}", a.token)).header("Accept", "application/json").header("User-Agent", "OutlookMCP/3.0.0");
            if let Some(d) = data {
                req = req.header("Content-Type", "application/json; charset=utf-8").body(serde_json::to_vec(d).unwrap_or_default());
            } else if method == "POST" {
                // a POST without a length gets 411 from Outlook (messages/{id}/send)
                req = req.header("Content-Length", "0");
            }
            let r = req.send().await.map_err(|e| Error::provider(format!("Outlook request failed: {e}")))?;
            let status = r.status().as_u16();
            if status == 401 && retry {
                retry = false;
                continue;
            }
            let text = r.text().await.map_err(|e| Error::provider(format!("Outlook body: {e}")))?;
            let path = p.split('?').next().unwrap_or("");
            if !(200..300).contains(&status) {
                let cut: String = text.chars().take(300).collect();
                return Err(Error::provider(format!("Outlook API error {status} on {method} {path}: {cut}")));
            }
            if text.is_empty() {
                return Ok(json!({}));
            }
            return serde_json::from_str(&text).map_err(|_| Error::provider(format!("Outlook API answered non-JSON on {method} {path}")));
        }
    }
}

fn well_known(name: &str) -> Option<&'static str> {
    Some(match name {
        "inbox" | "posta in arrivo" => "Inbox",
        "sent" | "sentitems" | "sent items" | "posta inviata" => "SentItems",
        "drafts" | "bozze" => "Drafts",
        "archive" | "archivio" => "Archive",
        "deleted" | "deleteditems" | "deleted items" | "trash" | "posta eliminata" => "DeletedItems",
        "junk" | "junkemail" | "spam" | "posta indesiderata" => "JunkEmail",
        "outbox" | "posta in uscita" => "Outbox",
        _ => return None,
    })
}

/// Every folder, child folders under "parent/child" names, following pages; explicit errors instead of a silent cut.
pub async fn list_folders(o: &Outlook, account: &str) -> Result<Vec<Obj>> {
    let account = o.upn(Some(account)).await?;
    let mut folders = vec![];
    scan(o, &account, "me/MailFolders".into(), String::new(), 0, &mut folders).await?;
    Ok(folders)
}

fn scan<'a>(
    o: &'a Outlook,
    account: &'a str,
    base: String,
    prefix: String,
    depth: u32,
    out: &'a mut Vec<Obj>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let mut p = format!("{base}?$top=100");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            if !seen.insert(p.clone()) {
                return Err(Error::provider("Repeated folder pagination link"));
            }
            let res = o.get(account, &p).await?;
            for f in res.get("value").and_then(Value::as_array).cloned().unwrap_or_default() {
                let display = f.get("DisplayName");
                let name = format!(
                    "{prefix}{}",
                    match display {
                        None | Some(Value::Null) => String::new(),
                        v => string(v),
                    }
                );
                let mut row = Obj::new();
                crate::js::set(&mut row, "id", f.get("Id").cloned());
                row.insert("name".into(), json!(name));
                crate::js::set(&mut row, "display_name", display.cloned());
                row.insert("total_count".into(), crate::js::present(f.get("TotalItemCount")).cloned().unwrap_or(json!(0)));
                row.insert("unread_count".into(), crate::js::present(f.get("UnreadItemCount")).cloned().unwrap_or(json!(0)));
                let children = crate::js::number(f.get("ChildFolderCount"));
                row.insert("child_count".into(), if children.is_nan() || children == 0.0 { json!(0) } else { num(children) });
                out.push(row);
                if crate::js::truthy_opt(f.get("ChildFolderCount")) && crate::js::truthy_opt(f.get("Id")) {
                    if depth >= 20 {
                        return Err(Error::provider("Folder nesting exceeds 20 levels"));
                    }
                    let id = string(f.get("Id"));
                    scan(o, account, format!("me/MailFolders/{}/childfolders", q(&id)), format!("{name}/"), depth + 1, out).await?;
                }
            }
            let Some(link) = res.get("@odata.nextLink") else { return Ok(()) };
            if !crate::js::truthy(link) {
                return Ok(());
            }
            p = crate::mail::next_link_path(link, &base).ok_or_else(|| Error::provider("Untrusted folder pagination link"))?;
        }
        Err(Error::provider("Folder listing exceeds 50 pages"))
    })
}

pub fn num(f: f64) -> Value {
    if f == f.trunc() && f.abs() < 9.007_199_254_740_992e15 { json!(f as i64) } else { json!(f) }
}

pub async fn resolve_folder_id(o: &Outlook, account: &str, folder: Option<&str>) -> Result<Option<String>> {
    let clean = folder.unwrap_or("").trim();
    if clean.is_empty() {
        return Ok(Some("Inbox".into()));
    }
    if let Some(k) = well_known(&clean.to_lowercase()) {
        return Ok(Some(k.into()));
    }
    if clean.starts_with("AAMk") || clean.starts_with("AQMk") {
        return Ok(Some(clean.into()));
    }
    let folders = list_folders(o, account).await?;
    let want = clean.to_lowercase();
    let name = |f: &Obj, k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
    let id = |f: &Obj| f.get("id").map(|v| string(Some(v)));
    if let Some(f) = folders.iter().find(|f| name(f, "name") == want || name(f, "display_name") == want) {
        return Ok(id(f));
    }
    Ok(folders.iter().find(|f| name(f, "name").contains(&want)).and_then(id))
}
