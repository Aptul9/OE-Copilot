//! Mail writes on Outlook: send, reply, forward, drafts, manage.

use crate::html::{format_body_for_html, looks_like_html};
use crate::js::{Obj, encode_uri_component as q, string};
use crate::outlook::Outlook;
use crate::{Error, Result};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;

/// An address and an optional display name, from "Name <a@b>", "a@b", or a "a@b, c@d" list.
pub struct Recipient {
    pub address: String,
    pub name: Option<String>,
}

pub fn parse_recipients(val: Option<&[String]>) -> Vec<Recipient> {
    static ANGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([^>]+)>").expect("fixed"));
    let Some(val) = val else { return vec![] };
    let mut parts: Vec<String> = vec![];
    for item in val {
        if !item.contains('<') && item.contains([',', ';']) {
            parts.extend(item.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(String::from));
        } else {
            parts.push(item.clone());
        }
    }
    let mut out = vec![];
    let mut seen = HashSet::new();
    for p in parts {
        let r = if let Some(m) = ANGLE.captures(&p) {
            let start = m.get(0).map_or(0, |x| x.start());
            let name = p[..start].trim().trim_matches(['"', '\'']).to_string();
            Some(Recipient { address: m[1].trim().to_string(), name: (!name.is_empty()).then_some(name) })
        } else if !p.trim().is_empty() {
            Some(Recipient { address: p.trim().to_string(), name: None })
        } else {
            None
        };
        if let Some(r) = r {
            let key = r.address.trim().to_lowercase();
            if !key.is_empty() && seen.insert(key) {
                out.push(r);
            }
        }
    }
    out
}

fn outlook_recipients(list: &[Recipient]) -> Vec<Value> {
    list.iter()
        .map(|r| match &r.name {
            Some(n) => json!({"EmailAddress": {"Address": r.address, "Name": n}}),
            None => json!({"EmailAddress": {"Address": r.address}}),
        })
        .collect()
}

fn addr(r: &Value) -> String {
    match r.get("EmailAddress").and_then(|e| e.get("Address")) {
        None | Some(Value::Null) => String::new(),
        v => string(v),
    }
    .trim()
    .to_lowercase()
}

fn addr_list(recs: &[Value]) -> Vec<Value> {
    recs.iter().filter_map(|r| r.get("EmailAddress").and_then(|e| e.get("Address")).filter(|a| crate::js::truthy(a)).cloned()).collect()
}

fn merge(existing: &[Value], extra: &[Value]) -> Vec<Value> {
    let mut out = existing.to_vec();
    let mut seen: HashSet<String> = out.iter().map(addr).filter(|a| !a.is_empty()).collect();
    for r in extra {
        let k = addr(r);
        if !k.is_empty() && seen.insert(k) {
            out.push(r.clone());
        }
    }
    out
}

fn list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// A file sent with the request: the caller's disk is not reachable.
pub struct InlineFile {
    pub name: String,
    pub content_base64: String,
    pub content_type: String,
}

/// FileAttachment objects; a malformed file stops the send before anything is created.
pub fn attachment_objects(files: &[InlineFile]) -> Result<Vec<Value>> {
    static BASE64: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+/]*={0,2}$").expect("fixed"));
    files
        .iter()
        .map(|f| {
            let norm = f.name.replace('\\', "/");
            let name = norm.rsplit('/').next().unwrap_or("").trim().to_string();
            let bytes: String = f.content_base64.chars().filter(|c| !c.is_whitespace()).collect();
            if name.is_empty() {
                return Err(Error::provider("Attachment name is empty"));
            }
            if bytes.is_empty() || !bytes.len().is_multiple_of(4) || !BASE64.is_match(&bytes) {
                return Err(Error::provider(format!("Attachment {name}: content_base64 is not base64")));
            }
            let mut a = Obj::new();
            a.insert("@odata.type".into(), json!("#Microsoft.OutlookServices.FileAttachment"));
            a.insert("Name".into(), json!(name));
            a.insert("ContentBytes".into(), json!(bytes));
            if !f.content_type.is_empty() {
                a.insert("ContentType".into(), json!(f.content_type));
            }
            Ok(Value::Object(a))
        })
        .collect()
}

/// Outlook takes a file under this size in one POST; from it up to 150 MB, only through an upload session.
const LARGE: usize = 3_000_000;

/// The decoded size of checked base64.
fn decoded_len(a: &Value) -> usize {
    let b = a.get("ContentBytes").and_then(Value::as_str).unwrap_or("");
    b.len() / 4 * 3 - b.bytes().rev().take_while(|c| *c == b'=').count()
}

/// Each file on the message: under LARGE in one POST, from it through an upload session.
async fn attach(o: &Outlook, account: &str, message_id: &str, atts: &[Value]) -> Result<()> {
    for a in atts {
        if decoded_len(a) < LARGE {
            o.request(account, &format!("me/messages/{}/attachments", q(message_id)), "POST", Some(a), &[]).await?;
        } else {
            let field = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("");
            let bytes = crate::js::base64_decode_lenient(field("ContentBytes"));
            o.upload(account, message_id, field("Name"), field("ContentType"), &bytes).await?;
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct SendOptions {
    pub to: Option<Vec<String>>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub reply_to: Option<Vec<String>>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub files: Vec<InlineFile>,
    pub save_as_draft: bool,
    pub reply_to_id: Option<String>,
    pub reply_all: bool,
    pub add_recipients: bool,
    pub forward_to_id: Option<String>,
}

/// The common result: status, draft_id (Outlook drafts are messages), subject and recipient addresses.
pub struct SendResult {
    pub status: &'static str,
    pub draft_id: Option<String>,
    pub subject: Value,
    pub to: Vec<Value>,
    pub cc: Vec<Value>,
    pub bcc: Vec<Value>,
}

async fn via_draft(o: &Outlook, account: &str, target: &str, action: &str, opt: &SendOptions, replace_cc: bool) -> Result<SendResult> {
    let atts = attachment_objects(&opt.files)?;
    let draft = o
        .request(
            account,
            &format!("me/messages/{}/{action}", q(target)),
            "POST",
            Some(&json!({"Comment": format_body_for_html(opt.body.as_deref().unwrap_or(""))})),
            &[],
        )
        .await?;
    let draft_id = draft
        .get("Id")
        .filter(|v| crate::js::truthy(v))
        .map(|v| string(Some(v)))
        .ok_or_else(|| Error::provider(format!("Failed to create draft via {action} on message {target}")))?;
    let draft_path = format!("me/messages/{}", q(&draft_id));
    let to = outlook_recipients(&parse_recipients(opt.to.as_deref()));
    let cc = outlook_recipients(&parse_recipients(opt.cc.as_deref()));
    let (d_to, d_cc, d_bcc) = (list(draft.get("ToRecipients")), list(draft.get("CcRecipients")), list(draft.get("BccRecipients")));
    let to_r = if opt.add_recipients {
        merge(&d_to, &to)
    } else if !to.is_empty() {
        to
    } else {
        d_to
    };
    let mut cc_r = if opt.add_recipients {
        merge(&d_cc, &cc)
    } else if replace_cc || action != "createreplyall" {
        if !cc.is_empty() {
            cc
        } else if replace_cc {
            vec![]
        } else {
            d_cc
        }
    } else {
        merge(&d_cc, &cc)
    };
    let bcc_r = merge(&d_bcc, &outlook_recipients(&parse_recipients(opt.bcc.as_deref())));
    if opt.add_recipients {
        let excluded: HashSet<String> = to_r.iter().map(addr).collect();
        if !excluded.is_empty() {
            cc_r.retain(|r| !excluded.contains(&addr(r)));
        }
    }
    if to_r.is_empty() && !opt.save_as_draft {
        o.request(account, &draft_path, "DELETE", None, &[]).await?;
        return Err(Error::provider("No 'To' recipients left after applying recipient filters. Specify 'to' explicitly."));
    }
    let mut patch = Obj::new();
    patch.insert("ToRecipients".into(), Value::Array(to_r.clone()));
    patch.insert("CcRecipients".into(), Value::Array(cc_r.clone()));
    patch.insert("BccRecipients".into(), Value::Array(bcc_r.clone()));
    let reply_to = outlook_recipients(&parse_recipients(opt.reply_to.as_deref()));
    if !reply_to.is_empty() {
        patch.insert("ReplyTo".into(), Value::Array(reply_to));
    }
    if let Some(s) = opt.subject.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        patch.insert("Subject".into(), json!(s));
    }
    let steps = async {
        o.request(account, &draft_path, "PATCH", Some(&Value::Object(patch)), &[]).await?;
        attach(o, account, &draft_id, &atts).await?;
        if !opt.save_as_draft {
            o.request(account, &format!("{draft_path}/send"), "POST", None, &[]).await?;
        }
        Ok::<(), Error>(())
    };
    // the draft was made by this call and the caller gets no id for it: none is left half built
    if let Err(e) = steps.await {
        let _ = o.request(account, &draft_path, "DELETE", None, &[]).await;
        return Err(e);
    }
    let subject = match opt.subject.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => match draft.get("Subject") {
            Some(v) if crate::js::truthy(v) => string(Some(v)),
            _ => String::new(),
        },
    };
    Ok(SendResult {
        status: if opt.save_as_draft { "draft" } else { "sent" },
        draft_id: opt.save_as_draft.then_some(draft_id),
        subject: json!(subject.trim()),
        to: addr_list(&to_r),
        cc: addr_list(&cc_r),
        bcc: addr_list(&bcc_r),
    })
}

pub async fn send_email(o: &Outlook, account: &str, opt: &SendOptions) -> Result<SendResult> {
    if let Some(id) = &opt.reply_to_id {
        return via_draft(o, account, id, if opt.reply_all { "createreplyall" } else { "createreply" }, opt, false).await;
    }
    if let Some(id) = &opt.forward_to_id {
        if opt.to.is_none() && !opt.save_as_draft {
            return Err(Error::provider("Recipient ('to') is required when forwarding an email"));
        }
        return via_draft(o, account, id, "createforward", opt, true).await;
    }
    if opt.to.is_none() && !opt.save_as_draft {
        return Err(Error::provider("Recipient (to) is required for new email"));
    }
    let to_r = outlook_recipients(&parse_recipients(opt.to.as_deref()));
    let cc_r = outlook_recipients(&parse_recipients(opt.cc.as_deref()));
    let bcc_r = outlook_recipients(&parse_recipients(opt.bcc.as_deref()));
    let body = opt.body.clone().unwrap_or_default();
    let mut message = Obj::new();
    message.insert("Subject".into(), json!(opt.subject.clone().unwrap_or_default()));
    message.insert("Body".into(), json!({"ContentType": if looks_like_html(&body) { "HTML" } else { "Text" }, "Content": body}));
    message.insert("ToRecipients".into(), Value::Array(to_r.clone()));
    if !cc_r.is_empty() {
        message.insert("CcRecipients".into(), Value::Array(cc_r.clone()));
    }
    if !bcc_r.is_empty() {
        message.insert("BccRecipients".into(), Value::Array(bcc_r.clone()));
    }
    let reply_to = outlook_recipients(&parse_recipients(opt.reply_to.as_deref()));
    if !reply_to.is_empty() {
        message.insert("ReplyTo".into(), Value::Array(reply_to));
    }
    let atts = attachment_objects(&opt.files)?;
    let result = |status, draft_id| SendResult {
        status,
        draft_id,
        subject: json!(opt.subject.clone().unwrap_or_default()),
        to: addr_list(&to_r),
        cc: addr_list(&cc_r),
        bcc: addr_list(&bcc_r),
    };
    // sendmail carries the files inline, in one request Outlook refuses past a few MB: from LARGE in total the
    // message is built as a draft, the files attached one by one, then sent
    if opt.save_as_draft || atts.iter().map(decoded_len).sum::<usize>() >= LARGE {
        let draft = o.request(account, "me/messages", "POST", Some(&Value::Object(message)), &[]).await?;
        let id = string(draft.get("Id"));
        let steps = async {
            attach(o, account, &id, &atts).await?;
            if !opt.save_as_draft {
                o.request(account, &format!("me/messages/{}/send", q(&id)), "POST", None, &[]).await?;
            }
            Ok::<(), Error>(())
        };
        if let Err(e) = steps.await {
            let _ = o.request(account, &format!("me/messages/{}", q(&id)), "DELETE", None, &[]).await;
            return Err(e);
        }
        return Ok(if opt.save_as_draft { result("draft", Some(id)) } else { result("sent", None) });
    }
    if !atts.is_empty() {
        message.insert("Attachments".into(), Value::Array(atts));
    }
    o.request(account, "me/sendmail", "POST", Some(&json!({"Message": Value::Object(message), "SaveToSentItems": "true"})), &[]).await?;
    Ok(result("sent", None))
}

/// New author text over an existing draft body; the quoted original and the HTML head survive.
pub fn replace_draft_body(body: Option<&Value>, text: &str) -> Value {
    static QUOTE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)<div id="appendonsend"></div>|<hr[^>]*>\s*<div[^>]*id="(?:x_)?divRplyFwdMsg"|<div[^>]*id="(?:x_)?divRplyFwdMsg""#).expect("fixed")
    });
    static BODY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<body[^>]*>").expect("fixed"));
    let html = match body.and_then(|b| b.get("Content")) {
        None | Some(Value::Null) => String::new(),
        v => string(v),
    };
    let is_html = string(body.and_then(|b| b.get("ContentType"))).to_lowercase() == "html";
    let at = if is_html { QUOTE.find(&html).map(|m| m.start()) } else { None };
    let Some(at) = at else {
        return json!({"ContentType": if looks_like_html(text) { "HTML" } else { "Text" }, "Content": text});
    };
    let head = BODY.find(&html[..at]).map(|m| &html[..m.end()]).unwrap_or("");
    json!({"ContentType": "HTML", "Content": format!("{head}{}{}", format_body_for_html(text), &html[at..])})
}

#[derive(Default)]
pub struct DraftUpdate {
    pub to: Option<Vec<String>>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub reply_to: Option<Vec<String>>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub files: Vec<InlineFile>,
}

async fn load_draft(o: &Outlook, account: &str, id: &str) -> Result<Obj> {
    let d = o.get(account, &format!("me/messages/{}?$select=IsDraft,Subject,Body,ToRecipients,CcRecipients,BccRecipients", q(id))).await?;
    if !crate::js::truthy_opt(d.get("IsDraft")) {
        return Err(Error::provider("Message is not a draft"));
    }
    Ok(crate::js::obj(&d))
}

fn draft_result(status: &'static str, id: Option<String>, d: &Obj) -> SendResult {
    SendResult {
        status,
        draft_id: id,
        subject: json!(match d.get("Subject") {
            None | Some(Value::Null) => String::new(),
            v => string(v),
        }),
        to: addr_list(&list(d.get("ToRecipients"))),
        cc: addr_list(&list(d.get("CcRecipients"))),
        bcc: addr_list(&list(d.get("BccRecipients"))),
    }
}

/// Recipient lists given replace the draft's; text replaces the author part and keeps the quoted original; files are
/// added.
pub async fn update_draft(o: &Outlook, account: &str, id: &str, opt: &DraftUpdate) -> Result<SendResult> {
    let atts = attachment_objects(&opt.files)?;
    let d = load_draft(o, account, id).await?;
    let mut patch = Obj::new();
    let rec = |v: &Option<Vec<String>>| v.as_ref().map(|l| Value::Array(outlook_recipients(&parse_recipients(Some(l)))));
    crate::js::set(&mut patch, "ToRecipients", rec(&opt.to));
    crate::js::set(&mut patch, "CcRecipients", rec(&opt.cc));
    crate::js::set(&mut patch, "BccRecipients", rec(&opt.bcc));
    crate::js::set(&mut patch, "ReplyTo", rec(&opt.reply_to));
    crate::js::set(&mut patch, "Subject", opt.subject.clone().map(Value::String));
    if let Some(b) = &opt.body {
        patch.insert("Body".into(), replace_draft_body(d.get("Body"), b));
    }
    let after = if patch.is_empty() {
        Value::Object(d.clone())
    } else {
        o.request(account, &format!("me/messages/{}", q(id)), "PATCH", Some(&Value::Object(patch.clone())), &[]).await?
    };
    attach(o, account, id, &atts).await?;
    let mut merged = d;
    merged.extend(patch);
    merged.extend(crate::js::obj(&after));
    Ok(draft_result("draft", Some(id.to_string()), &merged))
}

pub async fn send_draft(o: &Outlook, account: &str, id: &str) -> Result<SendResult> {
    let d = load_draft(o, account, id).await?;
    let none = |k: &str| d.get(k).and_then(Value::as_array).is_none_or(Vec::is_empty);
    if none("ToRecipients") && none("CcRecipients") && none("BccRecipients") {
        return Err(Error::provider("Draft has no recipients"));
    }
    o.request(account, &format!("me/messages/{}/send", q(id)), "POST", None, &[]).await?;
    Ok(draft_result("sent", None, &d))
}

pub async fn delete_draft(o: &Outlook, account: &str, id: &str) -> Result<()> {
    load_draft(o, account, id).await?;
    o.request(account, &format!("me/messages/{}", q(id)), "DELETE", None, &[]).await?;
    Ok(())
}

pub async fn manage_email(o: &Outlook, account: &str, id: &str, action: &str) -> Result<Vec<Value>> {
    if id.is_empty() {
        return Err(Error::provider("message_id is required"));
    }
    let act = action.trim().to_lowercase();
    let dest = match act.as_str() {
        "archive" => Some("Archive".to_string()),
        "delete" => Some("DeletedItems".to_string()),
        "move" => return Err(Error::provider("destination_folder is required when action is move")),
        "mark_read" | "mark_unread" | "flag" | "unflag" => None,
        _ => return Err(Error::provider(format!("Unsupported action '{action}'"))),
    };
    let p = format!("me/messages/{}", q(id));
    let patch = match act.as_str() {
        "mark_read" => Some((json!({"IsRead": true}), "marked_read")),
        "mark_unread" => Some((json!({"IsRead": false}), "marked_unread")),
        "flag" => Some((json!({"Flag": {"FlagStatus": "Flagged"}}), "flagged")),
        "unflag" => Some((json!({"Flag": {"FlagStatus": "NotFlagged"}}), "unflagged")),
        _ => None,
    };
    if let Some((body, status)) = patch {
        o.request(account, &p, "PATCH", Some(&body), &[]).await?;
        return Ok(vec![json!({"id": id, "status": status})]);
    }
    let dest = dest.unwrap_or_default();
    let moved = o.request(account, &format!("{p}/move"), "POST", Some(&json!({"DestinationId": dest})), &[]).await?;
    let new_id = crate::js::present(moved.get("Id")).cloned().unwrap_or(json!(id));
    Ok(vec![json!({"id": id, "new_id": new_id, "status": format!("moved_to_{dest}")})])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients() {
        let v = vec!["\"Rossi, M\" <m@x.it>".to_string(), "a@x.it; b@x.it".into(), "A@X.IT".into(), " ".into()];
        let r = parse_recipients(Some(&v));
        let got: Vec<(String, Option<String>)> = r.into_iter().map(|r| (r.address, r.name)).collect();
        assert_eq!(got, vec![("m@x.it".into(), Some("Rossi, M".into())), ("a@x.it".into(), None), ("b@x.it".into(), None)]);
    }

    #[test]
    fn draft_body_keeps_the_quote() {
        let body = json!({"ContentType": "HTML", "Content": "<html><body dir=\"ltr\"><p>old</p><div id=\"appendonsend\"></div><p>quoted</p></body></html>"});
        assert_eq!(
            replace_draft_body(Some(&body), "new\ntext")["Content"],
            json!(
                "<html><body dir=\"ltr\"><div style=\"font-family:Aptos,Calibri,sans-serif;font-size:11pt;\">new<br>\ntext</div><div id=\"appendonsend\"></div><p>quoted</p></body></html>"
            )
        );
        assert_eq!(replace_draft_body(Some(&json!({"ContentType": "Text", "Content": "x"})), "plain"), json!({"ContentType": "Text", "Content": "plain"}));
    }

    #[test]
    fn attachments() {
        let f = |name: &str, b: &str, t: &str| InlineFile { name: name.into(), content_base64: b.into(), content_type: t.into() };
        let a = attachment_objects(&[f("C:\\tmp\\a.txt", "aGk=", "text/plain"), f("b.bin", "aG k=\n", "")]).ok().unwrap();
        assert_eq!(a[0]["Name"], json!("a.txt"));
        assert_eq!(a[1].get("ContentType"), None);
        assert_eq!(a[1]["ContentBytes"], json!("aGk="));
        assert!(attachment_objects(&[f("x", "abc", "")]).is_err());
        assert!(attachment_objects(&[f(" / ", "aGk=", "")]).is_err());
    }
}
