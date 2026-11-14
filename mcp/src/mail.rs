//! Mail reads: list, search, message, thread, attachments.

use crate::js::{Obj, decode_uri_component, encode_uri_component as q, string, truthy_opt};
use crate::outlook::{Outlook, resolve_folder_id, rest_base};
use crate::{Error, Result};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;
use url::Url;

pub const MAX_LIST_LIMIT: i64 = 1000;
pub const DEFAULT_MAX_PAGES: usize = 20;
const PAGE_SIZE: i64 = 100;
const SUMMARY_FIELDS: &str = "Id,Subject,From,Sender,ToRecipients,CcRecipients,ReceivedDateTime,CreatedDateTime,BodyPreview,HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId,ParentFolderId";
const ATTACHMENT_FIELDS: &str = "$select=Id,Name,ContentType,Size,IsInline";
const CASELESS: [&str; 11] =
    ["me", "mailfolders", "messages", "childfolders", "inbox", "sentitems", "drafts", "archive", "deleteditems", "junkemail", "outbox"];

/// Collection identity of a REST path: segments decoded one by one, MailFolders('x') read as MailFolders/x, trailing
/// slash ignored, path words compared without case.
fn collection_key(p: &str) -> Option<Vec<String>> {
    static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^MailFolders\('((?:[^']|'')*)'\)$").expect("fixed"));
    let mut parts = vec![];
    for raw in p.trim_end_matches('/').split('/') {
        let seg = decode_uri_component(raw)?;
        if let Some(c) = QUOTED.captures(&seg) {
            parts.push("MailFolders".to_string());
            parts.push(c[1].replace("''", "'"));
        } else {
            parts.push(seg);
        }
    }
    Some(parts.into_iter().map(|s| if CASELESS.contains(&s.to_lowercase().as_str()) { s.to_lowercase() } else { s }).collect())
}

/// The path to request for an @odata.nextLink, or None unless the link is absolute https on the Outlook REST origin
/// (no credentials, default port), under its API path, and on the same collection as `base`. The request is rebuilt
/// on the fixed base from that path, so the token never leaves the origin whatever the link says.
pub fn next_link_path(link: &Value, base: &str) -> Option<String> {
    let link = link.as_str().filter(|s| !s.is_empty())?;
    let u = Url::parse(link).ok()?;
    let rest = Url::parse(&rest_base()).ok()?;
    let prefix = format!("{}/", rest.path());
    if u.origin().ascii_serialization() != rest.origin().ascii_serialization()
        || !u.username().is_empty()
        || u.password().is_some()
        || !u.path().starts_with(&prefix)
    {
        return None;
    }
    let path = &u.path()[prefix.len()..];
    let key = collection_key(path)?;
    if Some(key) != collection_key(base) {
        return None;
    }
    let search = match u.query() {
        Some(qs) if !qs.is_empty() => format!("?{qs}"),
        _ => String::new(),
    };
    Some(format!("{path}{search}"))
}

/// Reads `base?query` and its nextLinks until `limit` kept messages, the end, `max_pages`, or a refused or repeated
/// link. Duplicate Ids across pages are dropped.
async fn collect(
    o: &Outlook,
    account: &str,
    base: &str,
    query: &str,
    limit: i64,
    max_pages: usize,
    keep: Option<&(dyn Fn(&Value) -> bool + Sync)>,
) -> Result<Obj> {
    let mut items: Vec<Value> = vec![];
    let mut ids: HashSet<String> = HashSet::new();
    let mut visited: HashSet<String> = HashSet::new();
    let id_of = |m: &Value| crate::js::present(m.get("Id")).map(|v| v.to_string());
    let mut path = format!("{base}?{query}");
    let mut pages = 0;
    let mut scanned = 0;
    loop {
        visited.insert(path.clone());
        let res = o.get(account, &path).await?;
        pages += 1;
        let value = res.get("value").and_then(Value::as_array).cloned().unwrap_or_default();
        scanned += value.len();
        let mut i = 0;
        while i < value.len() && (items.len() as i64) < limit {
            let m = &value[i];
            let id = id_of(m);
            if id.as_ref().is_some_and(|x| ids.contains(x)) {
                i += 1;
                continue;
            }
            if let Some(x) = id {
                ids.insert(x);
            }
            if keep.is_none_or(|k| k(m)) {
                items.push(m.clone());
            }
            i += 1;
        }
        let link = res.get("@odata.nextLink").filter(|l| crate::js::truthy(l));
        let done = |items: Vec<Value>, stop: &str, has_more: bool| {
            let mut o = Obj::new();
            o.insert("items".into(), Value::Array(items));
            o.insert("has_more".into(), json!(has_more));
            o.insert("incomplete".into(), json!(stop != "limit" && stop != "exhausted"));
            o.insert("stop_reason".into(), json!(stop));
            o.insert("limit".into(), json!(limit));
            o.insert("pages".into(), json!(pages));
            o.insert("scanned".into(), json!(scanned));
            o
        };
        if items.len() as i64 >= limit {
            let more = link.is_some() || value[i..].iter().any(|m| id_of(m).is_none_or(|x| !ids.contains(&x)) && keep.is_none_or(|k| k(m)));
            return Ok(done(items, "limit", more));
        }
        let Some(link) = link else { return Ok(done(items, "exhausted", false)) };
        let Some(next) = next_link_path(link, base) else { return Ok(done(items, "untrusted_next_link", true)) };
        if visited.contains(&next) {
            return Ok(done(items, "repeated_next_link", true));
        }
        if pages >= max_pages {
            return Ok(done(items, "max_pages", true));
        }
        path = next;
    }
}

fn clamp(v: Option<i64>, dflt: i64, max: i64) -> i64 {
    let v = v.filter(|x| *x != 0).unwrap_or(dflt);
    v.clamp(1, max)
}

pub struct ListOptions<'a> {
    pub folder: Option<&'a str>,
    pub query: Option<&'a str>,
    pub limit: Option<i64>,
    pub unread_only: Option<bool>,
}

/// Newest first, up to limit, following nextLink pages so local filters reach older mail. An unknown folder name is an
/// error.
pub async fn list_emails_page(o: &Outlook, account: &str, opt: &ListOptions<'_>) -> Result<Obj> {
    let limit = clamp(opt.limit, 20, MAX_LIST_LIMIT);
    let folder = opt.folder.unwrap_or("Inbox");
    let mut base = "me/messages".to_string();
    if !["all", "*", "any"].contains(&folder.trim().to_lowercase().as_str()) {
        let fid = resolve_folder_id(o, account, Some(folder)).await?.ok_or_else(|| Error::provider(format!("Folder not found: {folder}")))?;
        base = format!("me/MailFolders/{}/messages", q(&fid));
    }
    let query = opt.query.filter(|s| !s.is_empty());
    let unread = opt.unread_only == Some(true);
    let post_filter = query.is_some() && unread;
    let top = if post_filter { PAGE_SIZE } else { limit.min(PAGE_SIZE) };
    let mut params = vec![format!("$top={top}"), format!("$select={SUMMARY_FIELDS}")];
    if let Some(qs) = query {
        params.push(format!("$search={}", q(&format!("\"{}\"", qs.replace('"', "")))));
    } else {
        let mut filters = vec![];
        if unread {
            // $filter next to $orderby: Outlook answers 400 InefficientFilter unless the ordering property leads the
            // filter, so ReceivedDateTime goes first with an always-true bound
            filters.push("ReceivedDateTime ge 1900-01-01T00:00:00Z");
            filters.push("IsRead eq false");
        }
        if !filters.is_empty() {
            params.push(format!("$filter={}", q(&filters.join(" and "))));
        }
        params.push(format!("$orderby={}", q("ReceivedDateTime desc")));
    }
    let keep = |m: &Value| m.get("IsRead") == Some(&Value::Bool(false));
    collect(o, account, &base, &params.join("&"), limit, DEFAULT_MAX_PAGES, if post_filter { Some(&keep) } else { None }).await
}

/// list_emails_page across all folders unless one is named; a query is required.
pub async fn search_emails_page(o: &Outlook, account: &str, opt: &ListOptions<'_>) -> Result<Obj> {
    if opt.query.is_none_or(str::is_empty) {
        return Err(Error::provider("query or from_address is required"));
    }
    let folder = opt.folder.filter(|f| !f.is_empty()).unwrap_or("all");
    list_emails_page(o, account, &ListOptions { folder: Some(folder), ..*opt }).await
}

pub async fn list_attachments(o: &Outlook, account: &str, message_id: &str) -> Result<Vec<Value>> {
    let res = o.get(account, &format!("me/messages/{}/attachments?{ATTACHMENT_FIELDS}", q(message_id))).await?;
    Ok(res.get("value").and_then(Value::as_array).cloned().unwrap_or_default())
}

pub async fn get_message(o: &Outlook, account: &str, message_id: &str) -> Result<Value> {
    if message_id.is_empty() {
        return Err(Error::provider("message_id is required"));
    }
    let mut msg = crate::js::obj(&o.get(account, &format!("me/messages/{}", q(message_id))).await?);
    let list = if truthy_opt(msg.get("HasAttachments")) { list_attachments(o, account, message_id).await.unwrap_or_default() } else { vec![] };
    msg.insert("AttachmentsList".into(), Value::Array(list));
    Ok(Value::Object(msg))
}

pub async fn read_thread_page(o: &Outlook, account: &str, thread_id: &str, limit: Option<i64>) -> Result<Obj> {
    if thread_id.is_empty() {
        return Err(Error::provider("thread_id or message_id is required"));
    }
    let mut cid = thread_id.to_string();
    if !thread_id.starts_with("AAQk")
        && let Ok(m) = o.get(account, &format!("me/messages/{}?$select=ConversationId", q(thread_id))).await
        && let Some(c) = m.get("ConversationId").filter(|c| crate::js::truthy(c))
    {
        cid = string(Some(c));
    }
    let fields = "Id,Subject,From,Sender,ToRecipients,CcRecipients,BccRecipients,ReplyTo,ReceivedDateTime,SentDateTime,CreatedDateTime,Body,BodyPreview,HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId";
    let account = o.upn(Some(account)).await?;
    let top = clamp(Some(limit.unwrap_or(50)), 50, PAGE_SIZE);
    let filter = q(&format!("ConversationId eq '{}'", cid.replace('\'', "''")));
    let mut page = collect(o, &account, "me/messages", &format!("$filter={filter}&$top={top}&$select={fields}"), top, DEFAULT_MAX_PAGES, None).await?;
    let mut msgs = page.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    let at = |m: &Value| {
        let v = crate::js::present(m.get("ReceivedDateTime")).or(crate::js::present(m.get("SentDateTime"))).or(crate::js::present(m.get("CreatedDateTime")));
        v.map(|x| string(Some(x))).unwrap_or_default()
    };
    msgs.sort_by_key(|m| at(m));
    // attachment lists, three at a time
    use futures_util::StreamExt;
    let lists: Vec<Vec<Value>> = futures_util::stream::iter(msgs.iter().map(|m| {
        let has = truthy_opt(m.get("HasAttachments"));
        let id = string(m.get("Id"));
        let account = account.clone();
        async move { if has { list_attachments(o, &account, &id).await.unwrap_or_default() } else { vec![] } }
    }))
    .buffered(3)
    .collect()
    .await;
    for (m, list) in msgs.iter_mut().zip(lists) {
        if let Value::Object(o) = m {
            o.insert("AttachmentsList".into(), Value::Array(list));
        }
    }
    page.insert("items".into(), Value::Array(msgs));
    Ok(page)
}

pub async fn get_attachment(o: &Outlook, account: &str, message_id: &str, attachment_id: &str) -> Result<Value> {
    o.get(account, &format!("me/messages/{}/attachments/{}", q(message_id), q(attachment_id))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_links_stay_on_the_collection() {
        let base = "me/MailFolders/AAMk%3D/messages";
        let l = |s: &str| json!(s);
        assert_eq!(
            next_link_path(&l("https://outlook.office.com/api/v2.0/me/MailFolders('AAMk=')/messages?%24skip=10"), base).as_deref(),
            Some("me/MailFolders('AAMk=')/messages?%24skip=10")
        );
        assert_eq!(
            next_link_path(&l("https://outlook.office.com/api/v2.0/Me/mailfolders/AAMk%3D/Messages/?$skip=1"), base).as_deref(),
            Some("Me/mailfolders/AAMk%3D/Messages/?$skip=1")
        );
        assert_eq!(next_link_path(&l("https://outlook.office.com/api/v2.0/me/messages?$skip=1"), base), None);
        assert_eq!(next_link_path(&l("https://evil.example/api/v2.0/me/MailFolders/AAMk%3D/messages"), base), None);
        assert_eq!(next_link_path(&l("https://u:p@outlook.office.com/api/v2.0/me/MailFolders/AAMk%3D/messages"), base), None);
        assert_eq!(next_link_path(&l("https://outlook.office.com:8443/api/v2.0/me/MailFolders/AAMk%3D/messages"), base), None);
        assert_eq!(next_link_path(&l("https://outlook.office.com/api/v1.0/me/MailFolders/AAMk%3D/messages"), base), None);
        assert_eq!(next_link_path(&l("https://outlook.office.com/api/v2.0/me/MailFolders/aamk%3D/messages"), base), None);
        assert_eq!(next_link_path(&json!(5), base), None);
    }
}
