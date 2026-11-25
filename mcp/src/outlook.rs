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
            let path = p.split('?').next().unwrap_or("");
            let r = req.send().await.map_err(|e| failed(method, path, e))?;
            let status = r.status().as_u16();
            if status == 401 && retry {
                retry = false;
                continue;
            }
            let text = r.text().await.map_err(|e| failed(method, path, e))?;
            if !(200..300).contains(&status) {
                return Err(outlook_error(status, method, path, &text));
            }
            if text.is_empty() {
                return Ok(json!({}));
            }
            return serde_json::from_str(&text).map_err(|_| Error::provider(format!("Outlook API answered non-JSON on {method} {path}")));
        }
    }

    /// A file of 3 MB or more on a message: an upload session, then the bytes in ranges under 4 MB to the address it
    /// gives, which carries its own authorization: no token goes with them.
    pub async fn upload(&self, account: &str, message_id: &str, name: &str, content_type: &str, bytes: &[u8]) -> Result<()> {
        let total = bytes.len();
        let mut item = json!({"AttachmentType": "File", "Name": name, "Size": total});
        if !content_type.is_empty() {
            item["ContentType"] = json!(content_type);
        }
        let session = format!("me/messages/{}/attachments/createUploadSession", q(message_id));
        let s = self.request(account, &session, "POST", Some(&json!({"AttachmentItem": item})), &[]).await?;
        let url = s.get("UploadUrl").and_then(Value::as_str).unwrap_or("");
        let same_host = |u: &str| url::Url::parse(u).ok().map(|u| u.origin());
        if url.is_empty() || same_host(url) != same_host(&rest_base()) {
            return Err(Error::comm("PROVIDER_ERROR", "Outlook upload session address is not on the Outlook API host"));
        }
        let mut start = 0;
        while start < total {
            let end = (start + UPLOAD_CHUNK).min(total);
            let r = http_client()
                .put(url)
                .timeout(Duration::from_secs(120))
                .header("Content-Type", "application/octet-stream")
                .header("Content-Range", format!("bytes {start}-{}/{total}", end - 1))
                .body(bytes[start..end].to_vec())
                .send()
                .await;
            let refused = match r {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let text = r.text().await.unwrap_or_default();
                    match status {
                        201 if end == total => return Ok(()),
                        200 if end < total => None,
                        200..300 => Some(Error::comm("PROVIDER_ERROR", format!("Outlook upload of {name} stopped at byte {end} of {total} with {status}"))),
                        _ => Some(outlook_error(status, "PUT", "upload session", &text)),
                    }
                }
                Err(e) => Some(failed("PUT", "upload session", e)),
            };
            if let Some(e) = refused {
                let _ = http_client().delete(url).timeout(Duration::from_secs(30)).send().await;
                return Err(e);
            }
            start = end;
        }
        Err(Error::comm("PROVIDER_ERROR", format!("Attachment {name} is empty")))
    }
}

/// Byte range per PUT of an upload session: Outlook wants each under 4 MB.
const UPLOAD_CHUNK: usize = 3 * 1024 * 1024;

/// Outlook's own status, error code and message as the tool error; long ids in the path shortened, the text cut short.
fn outlook_error(status: u16, method: &str, path: &str, text: &str) -> Error {
    let e = serde_json::from_str::<Value>(text).ok().and_then(|v| v.get("error").cloned());
    let field = |k: &str| e.as_ref().and_then(|e| e.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
    let detail = if e.is_some() { format!("{}: {}", field("code"), field("message")) } else { text.trim().to_string() };
    let detail: String = detail.chars().take(300).collect();
    Error::comm("PROVIDER_ERROR", format!("Outlook answered {status} on {method} {}: {detail}", shape(path)))
}

/// No answer from Outlook; the URL stays out of the text (an upload address carries a token).
fn failed(method: &str, path: &str, e: reqwest::Error) -> Error {
    Error::comm("PROVIDER_ERROR", format!("Outlook request {method} {} failed: {}; write outcome may be unknown", shape(path), e.without_url()))
}

fn shape(path: &str) -> String {
    path.split('/').map(|s| if s.len() > 40 { "{id}" } else { s }).collect::<Vec<_>>().join("/")
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
