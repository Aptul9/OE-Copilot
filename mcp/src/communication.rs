//! The unified tools: account inventory, mail, chats and calendars over every provider, with account-bound handles in
//! place of native ids.

use crate::gateway::Gateway;
use crate::gmail::{Compose, Gmail, GmailFile, GmailResult};
use crate::identity::{Resource, handle, resource, resource_id};
use crate::js::{Obj, obj, or, set, string, truthy, truthy_opt};
use crate::outlook::Outlook;
use crate::send::{DraftUpdate, InlineFile, SendOptions, SendResult};
use crate::slack::Slack;
use crate::teams::{Teams, unpack};
use crate::{Error, Result, calendar, mail, send, unsupported};
use serde_json::{Value, json};

pub struct Communication {
    pub outlook: Outlook,
    pub google: Option<Gateway>,
    pub teams: Option<Teams>,
    pub slack: Option<Slack>,
}

/// An account as an operation sees it: the exact id, its provider and the native account behind it.
pub struct Account {
    pub account_id: String,
    pub provider: String,
    pub native: String,
}

const OUTLOOK_CAPS: [&str; 3] = ["email.read", "email.write", "calendar.read"];
const GMAIL_CAPS: [&str; 2] = ["email.read", "email.write"];
const GCHAT_CAPS: [&str; 8] = ["chat.list", "chat.read", "chat.search", "chat.send", "chat.edit", "chat.delete", "chat.react", "chat.attachment"];
const TEAMS_CAPS: [&str; 7] = ["chat.list", "chat.read", "chat.send", "chat.edit", "chat.delete", "chat.react", "chat.attachment"];

pub type Args = serde_json::Map<String, Value>;

fn s<'a>(x: &'a Args, k: &str) -> Option<&'a str> {
    x.get(k).and_then(Value::as_str)
}

fn strings(x: &Args, k: &str) -> Option<Vec<String>> {
    x.get(k).and_then(Value::as_array).map(|a| a.iter().map(|v| string(Some(v))).collect())
}

fn limit(x: &Args) -> Option<i64> {
    x.get("limit").and_then(Value::as_i64)
}

/// `{...m, id, thread_id}` of a message: handles for its id and thread.
fn wrap_email(account: &str, m: &Value) -> Value {
    let mut o = obj(m);
    let id = or(&[m.get("Id"), m.get("id")]);
    o.insert("id".into(), json!(handle(account, "email", &string(id))));
    let thread = or(&[m.get("ConversationId"), m.get("threadId")]);
    set(&mut o, "thread_id", thread.filter(|t| truthy(t)).map(|t| json!(handle(account, "email_thread", &string(Some(t))))));
    Value::Object(o)
}

/// The same answer for every provider: handles, never native ids.
#[allow(clippy::too_many_arguments)]
fn sent(
    account: &str,
    status: &str,
    message_id: Option<String>,
    thread_id: Option<String>,
    draft_id: Option<String>,
    subject: Option<Value>,
    to: Option<Vec<Value>>,
    cc: Option<Vec<Value>>,
    bcc: Option<Vec<Value>>,
) -> Value {
    let mut o = Obj::new();
    o.insert("status".into(), json!(status));
    if let Some(m) = message_id.filter(|m| !m.is_empty()) {
        o.insert("message_id".into(), json!(handle(account, "email", &m)));
    }
    if let Some(t) = thread_id.filter(|t| !t.is_empty()) {
        o.insert("thread_id".into(), json!(handle(account, "email_thread", &t)));
    }
    if status != "deleted" {
        o.insert("draft_id".into(), draft_id.filter(|d| !d.is_empty()).map(|d| json!(handle(account, "email_draft", &d))).unwrap_or(Value::Null));
        set(&mut o, "subject", subject);
        set(&mut o, "to", to.map(Value::Array));
        set(&mut o, "cc", cc.map(Value::Array));
        set(&mut o, "bcc", bcc.map(Value::Array));
    }
    Value::Object(o)
}

/// Outlook drafts are messages: the draft handle and the message handle carry the same id.
fn outlook_sent(account: &str, r: SendResult) -> Value {
    sent(account, r.status, r.draft_id.clone(), None, r.draft_id, Some(r.subject), Some(r.to), Some(r.cc), Some(r.bcc))
}

fn gmail_sent(account: &str, r: GmailResult) -> Value {
    if r.status == "deleted" {
        return sent(account, "deleted", None, None, None, None, None, None, None);
    }
    sent(account, r.status, r.message_id, r.thread_id, r.draft_id, r.subject.map(Value::String), Some(r.to), Some(r.cc), Some(r.bcc))
}

impl Communication {
    async fn outlook_accounts(&self, refresh: bool) -> Result<Vec<crate::tokens::Account>> {
        self.outlook.tokens.accounts(refresh).await
    }

    pub async fn accounts(&self) -> (Vec<Value>, Obj) {
        let mut errors = Obj::new();
        let mut out: Vec<Value> = vec![];
        match self.outlook_accounts(false).await {
            Ok(list) => {
                for a in list {
                    out.push(json!({"account_id": format!("outlook:{}", a.upn), "provider": "outlook", "native": a.upn, "name": a.upn, "status": a.status, "capabilities": OUTLOOK_CAPS}));
                }
            }
            Err(_) => {
                errors.insert("outlook".into(), json!("Provider inventory unavailable"));
            }
        }
        if let Some(g) = &self.google {
            match g.call("/api/communication/google/accounts", None).await {
                Ok(r) => {
                    for a in r.get("accounts").and_then(Value::as_array).cloned().unwrap_or_default() {
                        let provider = a.get("provider");
                        let mut o = Obj::new();
                        o.insert("account_id".into(), json!(format!("{}:{}", string(provider), string(a.get("account")))));
                        set(&mut o, "provider", provider.cloned());
                        set(&mut o, "native", a.get("account").cloned());
                        set(&mut o, "name", a.get("account").cloned());
                        set(&mut o, "status", a.get("status").cloned());
                        o.insert("capabilities".into(), if provider.and_then(Value::as_str) == Some("gmail") { json!(GMAIL_CAPS) } else { json!(GCHAT_CAPS) });
                        out.push(Value::Object(o));
                    }
                }
                Err(_) => {
                    errors.insert("google".into(), json!("Provider inventory unavailable"));
                }
            }
        }
        if let Some(t) = &self.teams {
            match unpack(&t.call("list_accounts", json!({})).await) {
                Ok(r) => {
                    for a in r.get("accounts").and_then(Value::as_array).cloned().unwrap_or_default() {
                        let mut o = Obj::new();
                        o.insert("account_id".into(), json!(format!("teams:{}", string(a.get("slot")))));
                        o.insert("provider".into(), json!("teams"));
                        o.insert("native".into(), json!(string(a.get("slot"))));
                        set(&mut o, "name", or(&[a.get("name"), a.get("email")]).cloned());
                        set(&mut o, "email", a.get("email").cloned());
                        o.insert("capabilities".into(), json!(TEAMS_CAPS));
                        out.push(Value::Object(o));
                    }
                }
                Err(_) => {
                    errors.insert("teams".into(), json!("Provider inventory unavailable"));
                }
            }
        }
        if let Some(sl) = &self.slack {
            out.extend(sl.accounts());
        }
        (out, errors)
    }

    /// `family` is the tool's, `email` or `chat`: an id that names no account exactly may still name one of that family.
    pub async fn account(&self, id: &str, family: &str) -> Result<Account> {
        if id.is_empty() {
            return Err(Error::comm("ACCOUNT_REQUIRED", "account_id required"));
        }
        let unknown = || Error::comm("UNKNOWN_ACCOUNT", "Unknown exact account_id");
        let Some(split) = id.find(':').filter(|i| *i >= 1 && !id.ends_with(':')) else { return self.alias(id, family).await.ok_or_else(unknown) };
        let provider = &id[..split];
        let native = &id[split + 1..];
        let found = match provider {
            "outlook" => {
                // a row the broker sent within two minutes of expiry is not cached, and the cache does not reload for it
                self.outlook_accounts(false).await?.iter().any(|v| v.upn == native) || self.outlook_accounts(true).await?.iter().any(|v| v.upn == native)
            }
            "gmail" | "gchat" if self.google.is_some() => {
                let r = self.google.as_ref().map(|g| g.call("/api/communication/google/accounts", None));
                let r = match r {
                    Some(f) => f.await?,
                    None => Value::Null,
                };
                r.get("accounts").and_then(Value::as_array).is_some_and(|l| {
                    l.iter().any(|v| v.get("provider").and_then(Value::as_str) == Some(provider) && v.get("account").and_then(Value::as_str) == Some(native))
                })
            }
            "slack" if self.slack.is_some() => {
                self.slack.as_ref().is_some_and(|sl| sl.accounts().iter().any(|v| v.get("account_id").and_then(Value::as_str) == Some(id)))
            }
            "teams" if self.teams.is_some() => {
                let t = self.teams.as_ref().expect("checked");
                let r = unpack(&t.call("list_accounts", json!({})).await)?;
                r.get("accounts").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| string(v.get("slot")) == native))
            }
            _ => false,
        };
        if !found {
            return self.alias(id, family).await.ok_or_else(unknown);
        }
        Ok(Account { account_id: id.to_string(), provider: provider.to_string(), native: native.to_string() })
    }

    /// The one account of the family whose address an inexact id carries: a bare address, another prefix such as
    /// `teamsrelay:`, another letter case. A known provider prefix keeps to that provider. The answer and every handle
    /// carry the exact id.
    async fn alias(&self, id: &str, family: &str) -> Option<Account> {
        let (prefix, address) = id.split_once(':').map_or(("", id), |(p, rest)| (p, rest));
        if !address.contains('@') {
            return None;
        }
        let providers = if ["outlook", "gmail", "gchat", "teams", "slack"].contains(&prefix) {
            vec![prefix]
        } else if family == "chat" {
            vec!["teams", "gchat", "slack"]
        } else {
            vec!["outlook", "gmail"]
        };
        let (list, _) = self.accounts().await;
        let mut hits = list.iter().filter(|a| {
            providers.contains(&a.get("provider").and_then(Value::as_str).unwrap_or(""))
                && ["native", "name", "email"].iter().any(|k| a.get(*k).and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(address)))
        });
        let (Some(a), None) = (hits.next(), hits.next()) else { return None };
        Some(Account { account_id: string(a.get("account_id")), provider: string(a.get("provider")), native: string(a.get("native")) })
    }

    pub async fn email(&self, a: &Account, op: &str, x: &Args) -> Result<Value> {
        let acct = a.account_id.as_str();
        let id = |kind: &str, key: &str| resource(acct, kind, s(x, key).unwrap_or("")).map(|r| r.native);
        match a.provider.as_str() {
            "outlook" => self.outlook_email(a, op, x).await,
            "gmail" if self.google.is_some() => {
                let g = self.google.as_ref().expect("checked");
                let read = |path: String| {
                    let body = json!({"account": a.native, "path": path});
                    async move { g.call("/api/communication/gmail/read", Some(&body)).await }
                };
                let q = crate::js::encode_uri_component;
                match op {
                    "attachments" => {
                        let mid = id("email", "message_id")?;
                        let r = read(format!("messages/{}?format=full", q(&mid))).await?;
                        let mut items = vec![];
                        let mut stack = vec![r.get("payload").cloned().unwrap_or(json!({}))];
                        while let Some(part) = stack.pop() {
                            if let Some(att) = part.pointer("/body/attachmentId").filter(|v| truthy(v)) {
                                let mut ctx = Obj::new();
                                ctx.insert("message".into(), json!(mid));
                                let mut o = Obj::new();
                                set(&mut o, "name", part.get("filename").cloned());
                                set(&mut o, "mime_type", part.get("mimeType").cloned());
                                set(&mut o, "size", part.pointer("/body/size").cloned());
                                o.insert("id".into(), json!(resource_id(acct, "email_attachment", &string(Some(att)), ctx)));
                                items.push(Value::Object(o));
                            }
                            if let Some(children) = part.get("parts").and_then(Value::as_array) {
                                stack.extend(children.iter().rev().cloned());
                            }
                        }
                        return Ok(json!({"items": items}));
                    }
                    "attachment" => {
                        let f = resource(acct, "email_attachment", s(x, "attachment_id").unwrap_or(""))?;
                        return read(format!("messages/{}/attachments/{}", q(&f.ctx_str("message").unwrap_or_default()), q(&f.native))).await;
                    }
                    "folders" => {
                        let r = read("labels".into()).await?;
                        let labels = r.get("labels").and_then(Value::as_array).cloned().ok_or_else(|| Error::provider("labels"))?;
                        let items: Vec<Value> = labels
                            .iter()
                            .map(|f| {
                                let mut o = obj(f);
                                o.insert("id".into(), json!(handle(acct, "folder", &string(f.get("id")))));
                                Value::Object(o)
                            })
                            .collect();
                        return Ok(json!({"items": items}));
                    }
                    "read" => return Ok(wrap_email(acct, &read(format!("messages/{}?format=full", q(&id("email", "message_id")?))).await?)),
                    "thread" => {
                        let r = read(format!("threads/{}?format=full", q(&id("email_thread", "thread_id")?))).await?;
                        let mut o = obj(&r);
                        let msgs: Vec<Value> =
                            r.get("messages").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|m| wrap_email(acct, m)).collect();
                        o.insert("messages".into(), Value::Array(msgs));
                        return Ok(Value::Object(o));
                    }
                    "list" | "search" => {
                        let mut qs = vec![
                            ("maxResults".to_string(), string(x.get("limit"))),
                            ("q".into(), format!("{}{}", s(x, "query").unwrap_or(""), if truthy_opt(x.get("unread_only")) { " is:unread" } else { "" })),
                        ];
                        if x.get("cursor").is_some_and(truthy) {
                            qs.push(("pageToken".into(), id("cursor", "cursor")?));
                        }
                        if x.get("folder_id").is_some_and(truthy) {
                            qs.push(("labelIds".into(), id("folder", "folder_id")?));
                        }
                        let pairs: Vec<(&str, &str)> = qs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                        let r = read(format!("messages?{}", crate::js::search_params(&pairs))).await?;
                        let items: Vec<Value> =
                            r.get("messages").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|m| wrap_email(acct, m)).collect();
                        let mut o = Obj::new();
                        o.insert("items".into(), Value::Array(items));
                        set(&mut o, "next_cursor", r.get("nextPageToken").filter(|t| truthy(t)).map(|t| json!(handle(acct, "cursor", &string(Some(t))))));
                        return Ok(Value::Object(o));
                    }
                    _ => {}
                }
                let gmail = Gmail::new(g, &a.native);
                let compose = Compose {
                    to: strings(x, "to"),
                    cc: strings(x, "cc"),
                    bcc: strings(x, "bcc"),
                    reply_to: strings(x, "reply_to"),
                    subject: s(x, "subject").map(String::from),
                    text: s(x, "text").map(String::from),
                    files: gmail_files(x),
                    draft: truthy_opt(x.get("draft")),
                };
                let draft_id = || id("email_draft", "draft_id");
                match op {
                    "send" => {
                        let mid = if x.get("message_id").is_some_and(truthy) { Some(id("email", "message_id")?) } else { None };
                        return Ok(gmail_sent(acct, gmail.send(&compose, mid.as_deref()).await?));
                    }
                    "reply" | "reply_all" => return Ok(gmail_sent(acct, gmail.reply(&id("email", "message_id")?, &compose, op == "reply_all").await?)),
                    "forward" => return Ok(gmail_sent(acct, gmail.forward(&id("email", "message_id")?, &compose).await?)),
                    "draft_update" => return Ok(gmail_sent(acct, gmail.update_draft(&draft_id()?, &compose).await?)),
                    "draft_send" => return Ok(gmail_sent(acct, gmail.send_draft(&draft_id()?).await?)),
                    "draft_delete" => {
                        gmail.delete_draft(&draft_id()?).await?;
                        return Ok(json!({"status": "deleted"}));
                    }
                    "manage" => {
                        let action = s(x, "action").unwrap_or("");
                        let extra = match action {
                            "read" => json!({"action": "modify", "remove_labels": ["UNREAD"]}),
                            "unread" => json!({"action": "modify", "add_labels": ["UNREAD"]}),
                            "archive" => json!({"action": "modify", "remove_labels": ["INBOX"]}),
                            "delete" => json!({"action": "trash"}),
                            "flag" => json!({"action": "modify", "add_labels": ["STARRED"]}),
                            "unflag" => json!({"action": "modify", "remove_labels": ["STARRED"]}),
                            _ => return unsupported(&a.provider, action),
                        };
                        let mut body = Obj::new();
                        body.insert("account".into(), json!(a.native));
                        body.insert("message_id".into(), json!(id("email", "message_id")?));
                        body.insert("confirm".into(), json!(true));
                        body.extend(obj(&extra));
                        return g.call("/api/communication/gmail/write", Some(&Value::Object(body))).await;
                    }
                    _ => {}
                }
                unsupported(&a.provider, &format!("email {op}"))
            }
            p => unsupported(p, &format!("email {op}")),
        }
    }

    async fn outlook_email(&self, a: &Account, op: &str, x: &Args) -> Result<Value> {
        let acct = a.account_id.as_str();
        let o = &self.outlook;
        // a handle signed here; for a message, thread or folder also the Outlook id itself (one another system kept, or a
        // well-known folder name such as sentitems): it goes out with this account's own token, so it reaches nothing else
        let id = |kind: &str, key: &str| {
            let raw = s(x, key).unwrap_or("");
            match resource(acct, kind, raw) {
                Err(Error::Comm { code: "INVALID_RESOURCE", .. })
                    if ["email", "email_thread", "folder"].contains(&kind) && !raw.is_empty() && !raw.contains('.') =>
                {
                    Ok(raw.to_string())
                }
                r => r.map(|r| r.native),
            }
        };
        let files: Vec<InlineFile> = x
            .get("files")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|f| InlineFile {
                        name: string(f.get("name")),
                        content_base64: string(f.get("data")),
                        content_type: f.get("mime_type").and_then(Value::as_str).unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let opt = || SendOptions {
            to: strings(x, "to"),
            cc: strings(x, "cc"),
            bcc: strings(x, "bcc"),
            reply_to: strings(x, "reply_to"),
            subject: s(x, "subject").map(String::from),
            body: s(x, "text").map(String::from),
            files: files
                .iter()
                .map(|f| InlineFile { name: f.name.clone(), content_base64: f.content_base64.clone(), content_type: f.content_type.clone() })
                .collect(),
            save_as_draft: truthy_opt(x.get("draft")),
            ..Default::default()
        };
        match op {
            "folders" => {
                let items: Vec<Value> = crate::outlook::list_folders(o, &a.native)
                    .await?
                    .into_iter()
                    .map(|mut f| {
                        let fid = string(f.get("id"));
                        f.insert("id".into(), json!(handle(acct, "folder", &fid)));
                        Value::Object(f)
                    })
                    .collect();
                Ok(json!({"items": items}))
            }
            "list" | "search" => {
                if x.get("cursor").is_some_and(truthy) {
                    return unsupported(&a.provider, "continuation cursor; Outlook lists bounded pages internally");
                }
                let folder = if x.get("folder_id").is_some_and(truthy) { Some(id("folder", "folder_id")?) } else { None };
                let lo = mail::ListOptions {
                    folder: folder.as_deref(),
                    query: s(x, "query"),
                    limit: limit(x),
                    unread_only: x.get("unread_only").and_then(Value::as_bool),
                };
                let mut page =
                    if op == "search" { mail::search_emails_page(o, &a.native, &lo).await? } else { mail::list_emails_page(o, &a.native, &lo).await? };
                wrap_items(acct, &mut page);
                Ok(Value::Object(page))
            }
            "read" => Ok(wrap_email(acct, &mail::get_message(o, &a.native, &id("email", "message_id")?).await?)),
            "thread" => {
                let mut page = mail::read_thread_page(o, &a.native, &id("email_thread", "thread_id")?, limit(x)).await?;
                wrap_items(acct, &mut page);
                Ok(Value::Object(page))
            }
            "send" => {
                // old reply path: recipients replace the ones Outlook proposes
                let reply = if x.get("message_id").is_some_and(truthy) { Some(id("email", "message_id")?) } else { None };
                Ok(outlook_sent(acct, send::send_email(o, &a.native, &SendOptions { reply_to_id: reply, ..opt() }).await?))
            }
            "reply" | "reply_all" => {
                let target = id("email", "message_id")?;
                Ok(outlook_sent(
                    acct,
                    send::send_email(o, &a.native, &SendOptions { reply_to_id: Some(target), reply_all: op == "reply_all", add_recipients: true, ..opt() })
                        .await?,
                ))
            }
            "forward" => {
                let target = id("email", "message_id")?;
                Ok(outlook_sent(acct, send::send_email(o, &a.native, &SendOptions { forward_to_id: Some(target), ..opt() }).await?))
            }
            "draft_update" => {
                let d = id("email_draft", "draft_id")?;
                let p = opt();
                let u = DraftUpdate { to: p.to, cc: p.cc, bcc: p.bcc, reply_to: p.reply_to, subject: p.subject, body: p.body, files: p.files };
                Ok(outlook_sent(acct, send::update_draft(o, &a.native, &d, &u).await?))
            }
            "draft_send" => Ok(outlook_sent(acct, send::send_draft(o, &a.native, &id("email_draft", "draft_id")?).await?)),
            "draft_delete" => {
                send::delete_draft(o, &a.native, &id("email_draft", "draft_id")?).await?;
                Ok(json!({"status": "deleted"}))
            }
            "manage" => {
                let action = match s(x, "action").unwrap_or("") {
                    "read" => "mark_read",
                    "unread" => "mark_unread",
                    other => other,
                };
                Ok(json!({"items": send::manage_email(o, &a.native, &id("email", "message_id")?, action).await?}))
            }
            "attachments" => {
                let mid = id("email", "message_id")?;
                let items: Vec<Value> = mail::list_attachments(o, &a.native, &mid)
                    .await?
                    .iter()
                    .map(|f| {
                        let mut m = obj(f);
                        let mut ctx = Obj::new();
                        ctx.insert("message".into(), json!(mid));
                        m.insert("id".into(), json!(resource_id(acct, "email_attachment", &string(f.get("Id")), ctx)));
                        Value::Object(m)
                    })
                    .collect();
                Ok(json!({"items": items}))
            }
            "attachment" => {
                let f = resource(acct, "email_attachment", s(x, "attachment_id").unwrap_or(""))?;
                let mid = f.ctx_str("message").ok_or_else(|| Error::provider("attachment handle without message"))?;
                mail::get_attachment(o, &a.native, &mid, &f.native).await
            }
            _ => unsupported(&a.provider, &format!("email {op}")),
        }
    }

    pub async fn chat(&self, a: &Account, op: &str, x: &Args) -> Result<Value> {
        let acct = a.account_id.as_str();
        if ["edit", "delete", "react"].contains(&op) && !x.get("message_id").is_some_and(truthy) {
            return Err(Error::comm("INVALID_INPUT", "message_id required"));
        }
        let handle_of = |kind: &str, key: &str| -> Result<Option<Resource>> {
            if x.get(key).is_some_and(truthy) { resource(acct, kind, s(x, key).unwrap_or("")).map(Some) } else { Ok(None) }
        };
        let c = handle_of("conversation", "conversation_id")?;
        let m = handle_of("chat_message", "message_id")?;
        let t = handle_of("chat_thread", "thread_id")?;
        if let (Some(m), Some(c)) = (&m, &c)
            && m.ctx("conversation").map(|v| string(Some(v))) != Some(c.native.clone())
        {
            return Err(Error::comm("RESOURCE_MISMATCH", "Message belongs to another conversation"));
        }
        if let (Some(t), Some(c)) = (&t, &c)
            && t.ctx("conversation").map(|v| string(Some(v))) != Some(c.native.clone())
        {
            return Err(Error::comm("RESOURCE_MISMATCH", "Thread belongs to another conversation"));
        }
        let c_native = c.as_ref().map(|c| c.native.clone());
        let c_kind = c.as_ref().and_then(|c| c.ctx("kind").filter(|k| truthy(k)).cloned());
        let conversation = |v: &Value| {
            let mut o = obj(v);
            let mut ctx = Obj::new();
            ctx.insert("kind".into(), or(&[v.get("kind"), Some(&json!("space"))]).cloned().unwrap_or(json!("space")));
            o.insert("id".into(), json!(resource_id(acct, "conversation", &string(or(&[v.get("id"), v.get("name")])), ctx)));
            Value::Object(o)
        };
        let message = |v: &Value| {
            let mut o = obj(v);
            let conv = c_native
                .clone()
                .filter(|n| !n.is_empty())
                .map(Value::String)
                .or_else(|| or(&[v.get("channel").and_then(|ch| ch.get("id")), v.get("channel_id")]).filter(|x| truthy(x)).cloned())
                .unwrap_or(json!(""));
            let reply_ts = if truthy_opt(v.get("reply_count")) { v.get("ts") } else { None };
            let thread = or(&[v.get("thread_id"), v.get("thread"), v.get("thread_ts"), reply_ts]).filter(|x| truthy(x)).cloned();
            let mut ctx = Obj::new();
            ctx.insert("conversation".into(), conv.clone());
            ctx.insert("kind".into(), c_kind.clone().unwrap_or(json!("space")));
            ctx.insert("thread".into(), thread.clone().unwrap_or(json!("")));
            o.insert("id".into(), json!(resource_id(acct, "chat_message", &string(or(&[v.get("id"), v.get("ts")])), ctx)));
            let files: Vec<Value> = v
                .get("files")
                .filter(|f| truthy(f))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|f| match f {
                    Value::String(name) => json!({"name": name}),
                    _ => {
                        let mut fo = obj(f);
                        fo.insert("id".into(), json!(handle(acct, "chat_attachment", &string(f.get("id")))));
                        Value::Object(fo)
                    }
                })
                .collect();
            o.insert("files".into(), Value::Array(files));
            let attachments: Vec<Value> = v
                .get("attachments")
                .filter(|f| truthy(f))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|f| {
                    let mut fo = obj(f);
                    let id = match f.get("token").filter(|t| truthy(t)) {
                        Some(token) => {
                            let mut ctx = Obj::new();
                            set(&mut ctx, "mime_type", f.get("mime_type").cloned());
                            Some(json!(resource_id(acct, "chat_attachment", &string(Some(token)), ctx)))
                        }
                        None => f.get("id").cloned(),
                    };
                    set(&mut fo, "id", id);
                    Value::Object(fo)
                })
                .collect();
            o.insert("attachments".into(), Value::Array(attachments));
            let mut tctx = Obj::new();
            tctx.insert("conversation".into(), conv);
            set(&mut o, "thread_id", thread.map(|th| json!(resource_id(acct, "chat_thread", &string(Some(&th)), tctx))));
            Value::Object(o)
        };
        let map_list =
            |r: &Value, key: &str, f: &dyn Fn(&Value) -> Value| r.get(key).and_then(Value::as_array).map(|l| Value::Array(l.iter().map(f).collect()));

        if x.get("cursor").is_some_and(truthy) && a.provider != "slack" {
            return unsupported(&a.provider, "chat cursor");
        }
        if truthy_opt(x.get("refresh")) && a.provider != "teams" {
            return unsupported(&a.provider, "refresh");
        }
        let text_empty = s(x, "text").is_none_or(|t| t.trim().is_empty());
        let has_files = x.get("files").and_then(Value::as_array).is_some_and(|f| !f.is_empty());
        if op == "send" && text_empty && !has_files {
            return Err(Error::comm("INVALID_INPUT", "Message text or files required"));
        }
        if op == "send" && has_files && a.provider == "teams" {
            return unsupported(&a.provider, "file uploads");
        }
        match a.provider.as_str() {
            "slack" if self.slack.is_some() => {
                let sl = self.slack.as_ref().expect("checked");
                let args = crate::slack::Args {
                    channel: c_native.clone(),
                    ts: m.as_ref().or(t.as_ref()).map(|r| r.native.clone()),
                    file_id: if x.get("attachment_id").is_some_and(truthy) {
                        Some(resource(acct, "chat_attachment", s(x, "attachment_id").unwrap_or(""))?.native)
                    } else {
                        None
                    },
                    cursor: if x.get("cursor").is_some_and(truthy) { Some(resource(acct, "chat_cursor", s(x, "cursor").unwrap_or(""))?.native) } else { None },
                    limit: limit(x).unwrap_or(20),
                    query: s(x, "query").map(String::from),
                    text: s(x, "text").map(String::from),
                    emoji: s(x, "emoji").map(String::from),
                    remove: truthy_opt(x.get("remove")),
                    files: x
                        .get("files")
                        .and_then(Value::as_array)
                        .map(|l| l.iter().map(|f| (string(f.get("name")), string(f.get("data")))).collect())
                        .unwrap_or_default(),
                };
                let r = sl.call(&a.native, op, &args).await?;
                let mut o = obj(&r);
                if op == "list" {
                    set(&mut o, "next_cursor", r.get("next_cursor").filter(|n| truthy(n)).map(|n| json!(handle(acct, "chat_cursor", &string(Some(n))))));
                    o.insert("conversations".into(), map_list(&r, "conversations", &conversation).unwrap_or(json!([])));
                    return Ok(Value::Object(o));
                }
                if let Some(l) = map_list(&r, "matches", &message) {
                    o.insert("matches".into(), l);
                    return Ok(Value::Object(o));
                }
                if let Some(l) = map_list(&r, "messages", &message) {
                    o.insert("messages".into(), l);
                    return Ok(Value::Object(o));
                }
                if let Some(ts) = r.get("ts").filter(|t| truthy(t)) {
                    let mut ctx = Obj::new();
                    let conv = r
                        .get("channel_id")
                        .filter(|v| truthy(v))
                        .cloned()
                        .or(c_native.clone().filter(|n| !n.is_empty()).map(Value::String))
                        .unwrap_or(json!(""));
                    ctx.insert("conversation".into(), conv);
                    o.insert("id".into(), json!(resource_id(acct, "chat_message", &string(Some(ts)), ctx)));
                    return Ok(Value::Object(o));
                }
                Ok(r)
            }
            "gchat" if self.google.is_some() => {
                let g = self.google.as_ref().expect("checked");
                if op == "attachment" {
                    let f = resource(acct, "chat_attachment", s(x, "attachment_id").unwrap_or(""))?;
                    let mut body = Obj::new();
                    body.insert("account".into(), json!(a.native));
                    body.insert("action".into(), json!("attachment"));
                    body.insert("token".into(), json!(f.native));
                    set(&mut body, "mime_type", f.ctx("mime_type").cloned());
                    return g.call("/api/communication/gchat/read", Some(&Value::Object(body))).await;
                }
                let write = ["send", "edit", "delete", "react"].contains(&op);
                if op != "list" && op != "search" && c.is_none() {
                    return Err(Error::comm("INVALID_INPUT", "conversation_id required"));
                }
                let mut body = Obj::new();
                body.insert("account".into(), json!(a.native));
                body.insert("action".into(), json!(op));
                set(&mut body, "conversation", c_native.clone().map(Value::String));
                set(&mut body, "kind", c.as_ref().and_then(|c| c.ctx("kind").cloned()));
                set(&mut body, "message_id", m.as_ref().map(|m| json!(m.native)));
                let thread = t.as_ref().map(|t| json!(t.native)).or_else(|| m.as_ref().and_then(|m| m.ctx("thread").filter(|v| truthy(v)).cloned()));
                set(&mut body, "thread", thread);
                set(&mut body, "text", x.get("text").cloned());
                set(
                    &mut body,
                    "files",
                    x.get("files").and_then(Value::as_array).map(|l| {
                        Value::Array(l.iter().map(|f| json!({"name": f.get("name"), "mime_type": f.get("mime_type"), "data": f.get("data")})).collect())
                    }),
                );
                set(&mut body, "query", x.get("query").cloned());
                set(&mut body, "emoji", x.get("emoji").cloned());
                set(&mut body, "remove", x.get("remove").cloned());
                set(&mut body, "topics", x.get("limit").cloned());
                set(&mut body, "confirm", write.then_some(json!(true)));
                let r = g.call(if write { "/api/communication/gchat/write" } else { "/api/communication/gchat/read" }, Some(&Value::Object(body))).await?;
                let mut o = obj(&r);
                if op == "list" {
                    o.insert("conversations".into(), map_list(&r, "conversations", &conversation).unwrap_or(json!([])));
                    return Ok(Value::Object(o));
                }
                if let Some(list) = r.get("messages").filter(|v| truthy(v)).and_then(Value::as_array) {
                    let kept: Vec<Value> = list
                        .iter()
                        .filter(|v| t.as_ref().is_none_or(|t| or(&[v.get("thread"), v.get("thread_id")]).and_then(Value::as_str) == Some(t.native.as_str())))
                        .map(&message)
                        .collect();
                    o.insert("messages".into(), Value::Array(kept));
                    return Ok(Value::Object(o));
                }
                if let Some(l) = r.get("results").filter(|v| truthy(v)).and_then(|_| map_list(&r, "results", &message)) {
                    o.insert("results".into(), l);
                    return Ok(Value::Object(o));
                }
                if let Some(hits) = r.get("hits").filter(|v| truthy(v)).and_then(Value::as_array) {
                    let mapped: Vec<Value> = hits
                        .iter()
                        .map(|h| {
                            let conv = h.get("conversation").cloned().unwrap_or(Value::Null);
                            let mut msg = obj(h.get("message").unwrap_or(&Value::Null));
                            set(&mut msg, "channel_id", conv.get("id").cloned());
                            json!({"conversation": conversation(&conv), "message": message(&Value::Object(msg))})
                        })
                        .collect();
                    o.insert("hits".into(), Value::Array(mapped));
                    return Ok(Value::Object(o));
                }
                Ok(r)
            }
            "teams" if self.teams.is_some() => {
                let teams = self.teams.as_ref().expect("checked");
                let tool = match op {
                    "list" => "list_chats",
                    // refresh: Teams opens the chat first, so its newest messages and images are saved (and it reads as read)
                    "read" if truthy_opt(x.get("refresh")) => "refresh_chat",
                    "read" => "read_chat",
                    "send" => "send_message",
                    "edit" => "edit_message",
                    "delete" => "delete_message",
                    "react" => "react_message",
                    "attachment" => "get_attachment",
                    _ => return unsupported(&a.provider, op),
                };
                if op != "list" && c.is_none() {
                    return Err(Error::comm("INVALID_INPUT", "conversation_id required"));
                }
                if t.is_some() {
                    return unsupported(&a.provider, "threads");
                }
                if op == "react" && x.contains_key("remove") {
                    return unsupported(&a.provider, "explicit reaction removal; Teams toggles reactions");
                }
                if op == "attachment" && m.is_none() {
                    return Err(Error::comm("INVALID_INPUT", "message_id required"));
                }
                let mut args = Obj::new();
                let slot = crate::js::number(Some(&json!(a.native)));
                args.insert("account".into(), crate::outlook::num(slot));
                set(&mut args, "chat", c_native.clone().map(Value::String));
                set(&mut args, "text", x.get("text").cloned());
                set(&mut args, "message_id", m.as_ref().map(|m| json!(m.native)));
                set(&mut args, "reply_to", if op == "send" { m.as_ref().map(|m| json!(m.native)) } else { None });
                set(&mut args, "reaction", x.get("emoji").cloned());
                set(&mut args, "index", x.get("index").cloned());
                let raw = teams.call(tool, Value::Object(args)).await;
                if op == "attachment" {
                    if truthy_opt(raw.get("isError")) {
                        return Err(Error::comm("PROVIDER_ERROR", "Teams attachment request failed"));
                    }
                    let mut o = Obj::new();
                    set(&mut o, "content", raw.get("content").cloned());
                    return Ok(Value::Object(o));
                }
                let r = unpack(&raw)?;
                let mut o = obj(&r);
                if op == "list" {
                    o.insert("chats".into(), map_list(&r, "chats", &conversation).unwrap_or(json!([])));
                    return Ok(Value::Object(o));
                }
                if let Some(msg) = r.get("message").filter(|v| truthy(v)) {
                    o.insert("message".into(), message(msg));
                    return Ok(Value::Object(o));
                }
                if let Some(l) = r.get("messages").filter(|v| truthy(v)).and_then(|_| map_list(&r, "messages", &message)) {
                    o.insert("messages".into(), l);
                    return Ok(Value::Object(o));
                }
                Ok(r)
            }
            p => unsupported(p, &format!("chat {op}")),
        }
    }

    pub async fn calendar_list(&self, x: &Args) -> Result<Value> {
        let a = self.account(s(x, "account_id").unwrap_or(""), "email").await?;
        if a.provider != "outlook" {
            return unsupported(&a.provider, "calendar");
        }
        let items: Vec<Value> = calendar::list_calendars(&self.outlook, &a.native)
            .await?
            .iter()
            .map(|v| {
                let mut o = obj(v);
                o.insert("id".into(), json!(handle(&a.account_id, "calendar", &string(v.get("Id")))));
                Value::Object(o)
            })
            .collect();
        Ok(json!({"account_id": a.account_id, "provider": a.provider, "items": items}))
    }

    pub async fn calendar_events(&self, x: &Args) -> Result<Value> {
        let a = self.account(s(x, "account_id").unwrap_or(""), "email").await?;
        if a.provider != "outlook" {
            return unsupported(&a.provider, "calendar");
        }
        let cal =
            if x.get("calendar_id").is_some_and(truthy) { Some(resource(&a.account_id, "calendar", s(x, "calendar_id").unwrap_or(""))?.native) } else { None };
        let (events, more) =
            calendar::list_calendar_events(&self.outlook, &a.native, s(x, "start").unwrap_or(""), s(x, "end").unwrap_or(""), cal.as_deref(), limit(x)).await?;
        let events: Vec<Value> = events
            .iter()
            .map(|v| {
                let mut o = obj(v);
                o.insert("id".into(), json!(handle(&a.account_id, "event", &string(v.get("Id")))));
                Value::Object(o)
            })
            .collect();
        Ok(json!({"account_id": a.account_id, "provider": a.provider, "events": events, "has_more": more}))
    }
}

fn wrap_items(account: &str, page: &mut Obj) {
    let items: Vec<Value> = page.get("items").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|m| wrap_email(account, m)).collect();
    page.insert("items".into(), Value::Array(items));
}

fn gmail_files(x: &Args) -> Vec<GmailFile> {
    x.get("files")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .map(|f| GmailFile {
                    name: string(f.get("name")),
                    mime_type: f.get("mime_type").and_then(Value::as_str).map(String::from),
                    data: string(f.get("data")),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A tool call: the family dispatch, `{account_id, provider, data}` around the answer.
pub async fn run(c: &Communication, name: &str, x: &Args) -> Result<Value> {
    if name == "communication_accounts" {
        let (accounts, errors) = c.accounts().await;
        let list: Vec<Value> = accounts
            .into_iter()
            .map(|a| {
                let mut o = obj(&a);
                o.shift_remove("native");
                Value::Object(o)
            })
            .collect();
        return Ok(json!({"accounts": list, "errors": errors}));
    }
    if name == "calendar_list" {
        return c.calendar_list(x).await;
    }
    if name == "calendar_events" {
        return c.calendar_events(x).await;
    }
    if x.get("download") == Some(&Value::Bool(true)) {
        return crate::files::link(c, name, x).await;
    }
    let (family, op) = name.split_once('_').ok_or_else(|| Error::provider("tool"))?;
    let a = c.account(s(x, "account_id").unwrap_or(""), family).await?;
    let data = match family {
        "email" => c.email(&a, op, x).await?,
        "chat" => c.chat(&a, op, x).await?,
        _ => return Err(Error::provider("tool")),
    };
    Ok(json!({"account_id": a.account_id, "provider": a.provider, "data": data}))
}

/// The tool result: the data as text and as structuredContent, or the error as `{code, message}` text.
pub async fn call_tool(c: &Communication, name: &str, x: &Args) -> Value {
    match run(c, name, x).await {
        Ok(data) => json!({"content": [{"type": "text", "text": crate::js::stringify(&data)}], "structuredContent": data}),
        Err(e) => {
            let (code, message) = match e {
                Error::Comm { code, message } => (code.to_string(), message),
                Error::Provider(_) => ("PROVIDER_ERROR".into(), "Provider request failed; write outcome may be unknown".into()),
            };
            json!({"content": [{"type": "text", "text": crate::js::stringify(&json!({"code": code, "message": message}))}], "isError": true})
        }
    }
}
