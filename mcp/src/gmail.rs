//! Gmail compose over the OE Copilot agent: MIME for new mail, replies, forwards and drafts. The agent holds the grant.

use crate::gateway::Gateway;
use crate::html::{html_escape, html_to_markdown, looks_like_html};
use crate::js::{base64_decode_lenient, base64_std, base64_url, encode_uri_component, string};
use crate::send::parse_recipients;
use crate::{Error, Result};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

const NL: &str = "\r\n";
const FILES_MAX: usize = 5_000_000;
/// The agent refuses a raw message longer than this (base64url characters).
const RAW_MAX: usize = 8 * 1024 * 1024;

static ADDRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^[^\s@<>",;]+@[^\s@<>",;]+$"#).expect("fixed"));
static ANGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([^>]+)>").expect("fixed"));

fn invalid<T>(message: &str) -> Result<T> {
    Err(Error::comm("INVALID_INPUT", message))
}

fn safe(v: &str) -> Result<&str> {
    if v.contains(['\r', '\n']) { invalid("Header newline refused") } else { Ok(v) }
}

fn ascii(v: &str) -> bool {
    v.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn wrap(b64: &str) -> String {
    let lines: Vec<&str> = b64.as_bytes().chunks(76).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    lines.join(NL)
}

/// RFC 2047 encoded words for non-ASCII header text, folded so no line passes 998 characters.
fn word(v: &str) -> Result<String> {
    safe(v)?;
    if ascii(v) {
        return Ok(v.to_string());
    }
    let mut chunks = vec![];
    let mut cur = String::new();
    for ch in v.chars() {
        if cur.len() + ch.len_utf8() > 45 {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    Ok(chunks.iter().map(|c| format!("=?UTF-8?B?{}?=", base64_std(c.as_bytes()))).collect::<Vec<_>>().join(&format!("{NL} ")))
}

fn addresses(v: Option<&[String]>) -> Result<Vec<String>> {
    parse_recipients(v)
        .into_iter()
        .map(|r| {
            let a = safe(&r.address)?.to_string();
            if !ADDRESS.is_match(&a) {
                return invalid("Invalid email address");
            }
            Ok(match r.name.filter(|n| !n.is_empty()) {
                Some(n) => format!("{} <{a}>", word(&if ascii(&n) { format!("\"{}\"", n.replace(['"', '\\'], "")) } else { n })?),
                None => a,
            })
        })
        .collect()
}

/// Address list from a received header: commas inside quotes or angle brackets do not split; groups and junk are
/// dropped.
fn from_header(v: Option<&str>) -> Result<Vec<String>> {
    static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^[^"<]*:"#).expect("fixed"));
    let mut items = vec![];
    let mut cur = String::new();
    let (mut quoted, mut angle) = (false, false);
    for ch in v.unwrap_or("").chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted && ch == '<' {
            angle = true;
        } else if !quoted && ch == '>' {
            angle = false;
        }
        if !quoted && !angle && (ch == ',' || ch == ';') {
            items.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    items.push(cur);
    let mut out = vec![];
    for s in items {
        let s = GROUP.replace(&s, "").trim().to_string();
        let target = ANGLE.captures(&s).map(|c| c[1].trim().to_string()).unwrap_or_else(|| s.clone());
        if ADDRESS.is_match(&target) {
            out.extend(addresses(Some(&[s]))?);
        }
    }
    Ok(out)
}

fn bare(list: &[String]) -> Vec<Value> {
    list.iter().map(|v| json!(ANGLE.captures(v).map(|c| c[1].to_string()).unwrap_or_else(|| v.clone()))).collect()
}

fn lower(v: &str) -> String {
    ANGLE.captures(v).map(|c| c[1].to_string()).unwrap_or_else(|| v.to_string()).trim().to_lowercase()
}

fn merge(base: &[String], extra: &[String], without: &HashSet<String>) -> Vec<String> {
    let mut seen = without.clone();
    let mut out = vec![];
    for v in base.iter().chain(extra) {
        let k = lower(v);
        if !k.is_empty() && seen.insert(k) {
            out.push(v.clone());
        }
    }
    out
}

#[derive(Clone)]
pub struct GmailFile {
    pub name: String,
    pub mime_type: Option<String>,
    pub data: String,
}

fn check_files(files: &[GmailFile]) -> Result<()> {
    static B64: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+/]*={0,2}$").expect("fixed"));
    static MIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9.+-]+[/][A-Za-z0-9.+-]+$").expect("fixed"));
    let mut total = 0;
    for f in files {
        if !B64.is_match(&f.data) || f.data.len() % 4 != 0 {
            return invalid("Invalid base64 file");
        }
        total += base64_decode_lenient(&f.data).len();
        if total > FILES_MAX {
            return Err(Error::comm("LIMIT_EXCEEDED", "Files exceed 5 MB"));
        }
        safe(&f.name)?;
        if let Some(m) = f.mime_type.as_deref().filter(|m| !m.is_empty())
            && !MIME.is_match(m)
        {
            return invalid("Invalid MIME type");
        }
    }
    Ok(())
}

struct Message {
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    reply_to: Vec<String>,
    subject: String,
    in_reply_to: Option<String>,
    references: Option<String>,
    body: String,
    html: bool,
    files: Vec<GmailFile>,
}

fn random_hex() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("random");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn mime(m: &Message) -> Result<String> {
    let mut head = vec![];
    if !m.to.is_empty() {
        head.push(format!("To: {}", m.to.join(", ")));
    }
    if !m.cc.is_empty() {
        head.push(format!("Cc: {}", m.cc.join(", ")));
    }
    if !m.bcc.is_empty() {
        head.push(format!("Bcc: {}", m.bcc.join(", ")));
    }
    if !m.reply_to.is_empty() {
        head.push(format!("Reply-To: {}", m.reply_to.join(", ")));
    }
    head.push(format!("Subject: {}", word(&m.subject)?));
    if let Some(v) = &m.in_reply_to {
        head.push(format!("In-Reply-To: {}", safe(v)?));
    }
    if let Some(v) = &m.references {
        head.push(format!("References: {}", safe(v)?));
    }
    head.push("MIME-Version: 1.0".into());
    static CRLF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n").expect("fixed"));
    let text = format!(
        "Content-Type: text/{}; charset=UTF-8{NL}Content-Transfer-Encoding: base64{NL}{NL}{}",
        if m.html { "html" } else { "plain" },
        wrap(&base64_std(CRLF.replace_all(&m.body, NL).as_bytes()))
    );
    let mut body = text.clone();
    if !m.files.is_empty() {
        let boundary = format!("oe_mcp_{}", random_hex());
        let mut parts = vec![text];
        for f in &m.files {
            let name = safe(&f.name)?.rsplit(['\\', '/']).next().unwrap_or("").to_string();
            let name = if name.is_empty() { "attachment".to_string() } else { name };
            // Gmail shows the plain filename parameter and ignores filename*: non-ASCII names go there as one encoded word
            let plain = if ascii(&name) { name.replace(['"', '\\'], "_") } else { format!("=?UTF-8?B?{}?=", base64_std(name.as_bytes())) };
            let kind = f.mime_type.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
            let star = encode_uri_component(&name).replace('\'', "%27").replace('(', "%28").replace(')', "%29").replace('*', "%2A");
            parts.push(format!(
                "Content-Type: {kind}; name=\"{plain}\"{NL}Content-Disposition: attachment; filename=\"{plain}\"; filename*=UTF-8''{star}{NL}Content-Transfer-Encoding: base64{NL}{NL}{}",
                wrap(&f.data)
            ));
        }
        body = format!(
            "Content-Type: multipart/mixed; boundary={boundary}{NL}{NL}--{boundary}{NL}{}{NL}--{boundary}--",
            parts.join(&format!("{NL}--{boundary}{NL}"))
        );
    }
    let raw = base64_url(format!("{}{NL}{body}", head.join(NL)).as_bytes());
    if raw.len() > RAW_MAX {
        return Err(Error::comm("LIMIT_EXCEEDED", "Message with attachments exceeds the Gmail size limit"));
    }
    Ok(raw)
}

struct Parsed {
    headers: HashMap<String, String>,
    text: Option<String>,
    html: Option<String>,
    files: Vec<GmailFile>,
}

impl Parsed {
    fn h(&self, k: &str) -> Option<&str> {
        self.headers.get(k).map(String::as_str)
    }
    /// `h[k] || ""`.
    fn hs(&self, k: &str) -> &str {
        self.h(k).unwrap_or("")
    }
}

fn decode(data: Option<&Value>) -> String {
    match data.and_then(Value::as_str) {
        Some(d) if !d.is_empty() => String::from_utf8_lossy(&base64_decode_lenient(d)).into_owned(),
        _ => String::new(),
    }
}

fn first_headers(message: &Value) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for h in message.pointer("/payload/headers").and_then(Value::as_array).cloned().unwrap_or_default() {
        headers.entry(string(h.get("name")).to_lowercase()).or_insert_with(|| string(h.get("value")));
    }
    headers
}

#[derive(Default)]
pub struct Compose {
    pub to: Option<Vec<String>>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub reply_to: Option<Vec<String>>,
    pub subject: Option<String>,
    pub text: Option<String>,
    pub files: Vec<GmailFile>,
    pub draft: bool,
}

/// What a compose returns before handles: the same fields as the Outlook result.
pub struct GmailResult {
    pub status: &'static str,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub draft_id: Option<String>,
    pub subject: Option<String>,
    pub to: Vec<Value>,
    pub cc: Vec<Value>,
    pub bcc: Vec<Value>,
}

fn opt_string(v: Option<&Value>) -> Option<String> {
    v.filter(|x| !x.is_null()).map(|x| string(Some(x)))
}

pub struct Gmail<'a> {
    gw: &'a Gateway,
    account: String,
}

impl<'a> Gmail<'a> {
    pub fn new(gw: &'a Gateway, account: &str) -> Self {
        Gmail { gw, account: account.to_lowercase() }
    }

    async fn read(&self, path: &str) -> Result<Value> {
        self.gw.call("/api/communication/gmail/read", Some(&json!({"account": self.account, "path": path}))).await
    }

    async fn write(&self, mut body: serde_json::Map<String, Value>) -> Result<Value> {
        let mut b = serde_json::Map::new();
        b.insert("account".into(), json!(self.account));
        b.insert("confirm".into(), json!(true));
        b.append(&mut body);
        self.gw.call("/api/communication/gmail/write", Some(&Value::Object(b))).await
    }

    /// Headers, first text/plain and text/html bodies, and every attachment with its bytes.
    async fn parse(&self, message: &Value, with_files: bool) -> Result<Parsed> {
        let mut out = Parsed { headers: first_headers(message), text: None, html: None, files: vec![] };
        let mid = string(message.get("id"));
        let mut stack = vec![message.get("payload").cloned().unwrap_or(json!({}))];
        // depth first, parts in order, like the recursive walk
        while let Some(part) = stack.pop() {
            let filename = part.get("filename");
            let att = part.pointer("/body/attachmentId");
            if crate::js::truthy_opt(filename) || crate::js::truthy_opt(att) {
                if with_files {
                    let data = if crate::js::truthy_opt(att) {
                        self.read(&format!("messages/{}/attachments/{}", encode_uri_component(&mid), encode_uri_component(&string(att))))
                            .await?
                            .get("data")
                            .cloned()
                    } else {
                        part.pointer("/body/data").cloned()
                    };
                    let data = match data {
                        Some(d) if crate::js::truthy(&d) => string(Some(&d)),
                        _ => String::new(),
                    };
                    out.files.push(GmailFile {
                        name: if crate::js::truthy_opt(filename) { string(filename) } else { "attachment".into() },
                        mime_type: opt_string(part.get("mimeType")),
                        data: base64_std(&base64_decode_lenient(&data)),
                    });
                }
            } else if part.get("mimeType").and_then(Value::as_str) == Some("text/plain") && out.text.is_none() {
                out.text = Some(decode(part.pointer("/body/data")));
            } else if part.get("mimeType").and_then(Value::as_str) == Some("text/html") && out.html.is_none() {
                out.html = Some(decode(part.pointer("/body/data")));
            }
            if let Some(children) = part.get("parts").and_then(Value::as_array) {
                stack.extend(children.iter().rev().cloned());
            }
        }
        Ok(out)
    }

    async fn original(&self, id: &str, with_files: bool) -> Result<(Value, Parsed)> {
        let m = self.read(&format!("messages/{}?format=full", encode_uri_component(id))).await?;
        let p = self.parse(&m, with_files).await?;
        Ok((m, p))
    }

    async fn deliver(&self, m: &Message, draft: bool, thread: Option<Value>) -> Result<GmailResult> {
        let raw = mime(m)?;
        let mut body = serde_json::Map::new();
        body.insert("action".into(), json!(if draft { "draft" } else { "send" }));
        body.insert("raw".into(), json!(raw));
        crate::js::set(&mut body, "thread_id", thread.filter(|t| !t.is_null()));
        let r = self.write(body).await?;
        Ok(GmailResult {
            status: if draft { "draft" } else { "sent" },
            message_id: if draft { opt_string(r.pointer("/message/id")) } else { opt_string(r.get("id")) },
            thread_id: if draft { opt_string(r.pointer("/message/threadId")) } else { opt_string(r.get("threadId")) },
            draft_id: if draft { opt_string(r.get("id")) } else { None },
            subject: Some(m.subject.clone()),
            to: bare(&m.to),
            cc: bare(&m.cc),
            bcc: bare(&m.bcc),
        })
    }

    fn threading(p: &Parsed) -> Result<(String, String)> {
        let Some(id) = p.h("message-id").filter(|s| !s.is_empty()) else { return invalid("Source message has no Message-ID") };
        let id = safe(id)?.to_string();
        let refs = format!("{} {id}", p.hs("references")).trim().to_string();
        Ok((id, safe(&refs)?.to_string()))
    }

    fn base(&self, x: &Compose, body: String) -> Result<Message> {
        Ok(Message {
            to: addresses(x.to.as_deref())?,
            cc: addresses(x.cc.as_deref())?,
            bcc: addresses(x.bcc.as_deref())?,
            reply_to: addresses(x.reply_to.as_deref())?,
            subject: x.subject.clone().unwrap_or_default(),
            in_reply_to: None,
            references: None,
            html: looks_like_html(&body),
            body,
            files: x.files.clone(),
        })
    }

    /// New mail. message_id is the old reply path of email_send: recipients and subject exactly as given, no quote.
    pub async fn send(&self, x: &Compose, message_id: Option<&str>) -> Result<GmailResult> {
        let text = x.text.clone().unwrap_or_default();
        let mut m = self.base(x, text)?;
        check_files(&x.files)?;
        let Some(id) = message_id else { return self.deliver(&m, x.draft, None).await };
        let (o, p) = self.original(id, false).await?;
        let (irt, refs) = Self::threading(&p)?;
        m.in_reply_to = Some(irt);
        m.references = Some(refs);
        self.deliver(&m, x.draft, o.get("threadId").cloned()).await
    }

    pub async fn reply(&self, id: &str, x: &Compose, all: bool) -> Result<GmailResult> {
        check_files(&x.files)?;
        let (o, p) = self.original(id, false).await?;
        let me: HashSet<String> = [self.account.clone()].into();
        let reply_to = p.h("reply-to").filter(|s| !s.is_empty());
        let from = from_header(reply_to.or(p.h("from")))?;
        // Reply-To wins, as in Outlook; without it a reply to own sent mail goes to its recipients, as in Gmail
        let own = reply_to.is_none() && from_header(p.h("from"))?.iter().any(|v| lower(v) == self.account);
        let mut to = if own { from_header(p.h("to"))? } else { from.clone() };
        let mut cc: Vec<String> = vec![];
        if all {
            let base = if own { to.clone() } else { merge(&to, &from_header(p.h("to"))?, &HashSet::new()) };
            to = merge(&base, &[], &me);
            if to.is_empty() {
                to = from.clone();
            }
            let mut skip = me.clone();
            skip.extend(to.iter().map(|v| lower(v)));
            cc = merge(&from_header(p.h("cc"))?, &[], &skip);
        }
        to = merge(&to, &addresses(x.to.as_deref())?, &HashSet::new());
        cc = merge(&cc, &addresses(x.cc.as_deref())?, &to.iter().map(|v| lower(v)).collect());
        let subject = x.subject.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| {
            let s = p.hs("subject");
            if s.to_lowercase().starts_with("re:") { s.to_string() } else { format!("Re: {s}") }
        });
        let text = x.text.clone().unwrap_or_default();
        let html = looks_like_html(&text);
        let intro = format!("On {} {} wrote:", p.hs("date"), p.hs("from"));
        let quote = if html {
            format!(
                "<br><div class=\"gmail_quote\"><div>{}</div><blockquote style=\"margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex\">{}</blockquote></div>",
                html_escape(&intro),
                inner(&p)
            )
        } else {
            static LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n").expect("fixed"));
            format!("{NL}{NL}{intro}{NL}{}", LINES.split(&plain(&p)).map(|l| format!("> {l}")).collect::<Vec<_>>().join(NL))
        };
        let (irt, refs) = Self::threading(&p)?;
        let m = Message {
            to,
            cc,
            bcc: addresses(x.bcc.as_deref())?,
            reply_to: addresses(x.reply_to.as_deref())?,
            subject,
            in_reply_to: Some(irt),
            references: Some(refs),
            body: format!("{text}{quote}"),
            html,
            files: x.files.clone(),
        };
        self.deliver(&m, x.draft, o.get("threadId").cloned()).await
    }

    pub async fn forward(&self, id: &str, x: &Compose) -> Result<GmailResult> {
        check_files(&x.files)?;
        let (o, p) = self.original(id, true).await?;
        let subject = x.subject.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| {
            let s = p.hs("subject");
            let l = s.to_lowercase();
            if l.starts_with("fwd:") || l.starts_with("fw:") { s.to_string() } else { format!("Fwd: {s}") }
        });
        let text = x.text.clone().unwrap_or_default();
        let html = looks_like_html(&text);
        let mut lines = vec![
            "---------- Forwarded message ---------".to_string(),
            format!("From: {}", p.hs("from")),
            format!("Date: {}", p.hs("date")),
            format!("Subject: {}", p.hs("subject")),
            format!("To: {}", p.hs("to")),
        ];
        if let Some(cc) = p.h("cc").filter(|s| !s.is_empty()) {
            lines.push(format!("Cc: {cc}"));
        }
        let quote = if html {
            format!(
                "<br><br><div class=\"gmail_quote\"><div>{}</div><br>{}</div>",
                lines.iter().map(|l| html_escape(l)).collect::<Vec<_>>().join("<br>"),
                inner(&p)
            )
        } else {
            format!("{NL}{NL}{}{NL}{NL}{}", lines.join(NL), plain(&p))
        };
        let (irt, refs) = Self::threading(&p)?;
        let mut files = p.files.clone();
        files.extend(x.files.iter().cloned());
        let m = Message {
            to: addresses(x.to.as_deref())?,
            cc: addresses(x.cc.as_deref())?,
            bcc: addresses(x.bcc.as_deref())?,
            reply_to: addresses(x.reply_to.as_deref())?,
            subject,
            in_reply_to: Some(irt),
            references: Some(refs),
            body: format!("{text}{quote}"),
            html,
            files,
        };
        self.deliver(&m, x.draft, o.get("threadId").cloned()).await
    }

    async fn draft(&self, id: &str, with_files: bool) -> Result<(Value, Parsed)> {
        let d = self.read(&format!("drafts/{}?format=full", encode_uri_component(id))).await?;
        let Some(message) = d.get("message").filter(|m| crate::js::truthy_opt(m.get("id"))) else {
            return Err(Error::comm("NOT_FOUND", "Draft not found"));
        };
        let p = self.parse(message, with_files).await?;
        Ok((d, p))
    }

    /// Recipient lists given replace the draft's; text replaces the author part and keeps the quoted original; files
    /// are added.
    pub async fn update_draft(&self, id: &str, x: &Compose) -> Result<GmailResult> {
        static QUOTE_TEXT: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\r?\n\r?\n(On [^\r\n]* wrote:\r?\n|---------- Forwarded message ---------)").expect("fixed"));
        static QUOTE_HTML: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<div class="gmail_quote">"#).expect("fixed"));
        static NLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n").expect("fixed"));
        check_files(&x.files)?;
        let (d, p) = self.draft(id, true).await?;
        let (body, html) = match &x.text {
            None => {
                let html = p.html.is_some();
                (if html { p.html.clone().unwrap_or_default() } else { p.text.clone().unwrap_or_default() }, html)
            }
            Some(t) => {
                let html = looks_like_html(t);
                let current = if html {
                    p.html.clone().unwrap_or_else(|| NLS.replace_all(&html_escape(p.text.as_deref().unwrap_or("")), "<br>").into_owned())
                } else {
                    p.text.clone().unwrap_or_else(|| html_to_markdown(p.html.as_deref().unwrap_or("")))
                };
                let at = if html { QUOTE_HTML.find(&current).map(|m| m.start()) } else { QUOTE_TEXT.find(&current).map(|m| m.start()) };
                (format!("{t}{}", at.map(|a| format!("{}{}", if html { "<br>" } else { "" }, &current[a..])).unwrap_or_default()), html)
            }
        };
        let pick = |given: &Option<Vec<String>>, header: &str| -> Result<Vec<String>> {
            if given.is_some() { addresses(given.as_deref()) } else { from_header(p.h(header)) }
        };
        let mut files = p.files.clone();
        files.extend(x.files.iter().cloned());
        let m = Message {
            to: pick(&x.to, "to")?,
            cc: pick(&x.cc, "cc")?,
            bcc: pick(&x.bcc, "bcc")?,
            reply_to: pick(&x.reply_to, "reply-to")?,
            subject: x.subject.clone().or_else(|| p.h("subject").map(String::from)).unwrap_or_default(),
            in_reply_to: p.h("in-reply-to").filter(|s| !s.is_empty()).map(|s| safe(s).map(String::from)).transpose()?,
            references: p.h("references").filter(|s| !s.is_empty()).map(|s| safe(s).map(String::from)).transpose()?,
            body,
            html,
            files,
        };
        let mut req = serde_json::Map::new();
        req.insert("action".into(), json!("update_draft"));
        req.insert("draft_id".into(), json!(id));
        req.insert("raw".into(), json!(mime(&m)?));
        crate::js::set(&mut req, "thread_id", d.pointer("/message/threadId").cloned().filter(|t| !t.is_null()));
        let r = self.write(req).await?;
        Ok(GmailResult {
            status: "draft",
            message_id: opt_string(r.pointer("/message/id")),
            thread_id: opt_string(r.pointer("/message/threadId")),
            draft_id: Some(r.get("id").filter(|v| crate::js::truthy(v)).map(|v| string(Some(v))).unwrap_or_else(|| id.to_string())),
            subject: Some(m.subject.clone()),
            to: bare(&m.to),
            cc: bare(&m.cc),
            bcc: bare(&m.bcc),
        })
    }

    pub async fn send_draft(&self, id: &str) -> Result<GmailResult> {
        let d = self.read(&format!("drafts/{}?format=metadata", encode_uri_component(id))).await?;
        let h = d.get("message").map(first_headers).unwrap_or_default();
        let get = |k: &str| h.get(k).map(String::as_str).filter(|s| !s.is_empty());
        if get("to").is_none() && get("cc").is_none() && get("bcc").is_none() {
            return invalid("Draft has no recipients");
        }
        let mut req = serde_json::Map::new();
        req.insert("action".into(), json!("send_draft"));
        req.insert("draft_id".into(), json!(id));
        let r = self.write(req).await?;
        Ok(GmailResult {
            status: "sent",
            message_id: opt_string(r.get("id")),
            thread_id: opt_string(r.get("threadId")),
            draft_id: None,
            subject: Some(get("subject").unwrap_or("").to_string()),
            to: bare(&from_header(get("to"))?),
            cc: bare(&from_header(get("cc"))?),
            bcc: bare(&from_header(get("bcc"))?),
        })
    }

    pub async fn delete_draft(&self, id: &str) -> Result<()> {
        let mut req = serde_json::Map::new();
        req.insert("action".into(), json!("delete_draft"));
        req.insert("draft_id".into(), json!(id));
        self.write(req).await?;
        Ok(())
    }
}

/// Original body for an HTML quote: its HTML without document wrappers, or its text escaped.
fn inner(p: &Parsed) -> String {
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<head[\s\S]*?</head>").expect("fixed"));
    static WRAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?(html|body)[^>]*>").expect("fixed"));
    static NLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n").expect("fixed"));
    match &p.html {
        Some(h) => WRAP.replace_all(&HEAD.replace(h, ""), "").into_owned(),
        None => NLS.replace_all(&html_escape(p.text.as_deref().unwrap_or("")), "<br>").into_owned(),
    }
}

fn plain(p: &Parsed) -> String {
    p.text.clone().unwrap_or_else(|| html_to_markdown(p.html.as_deref().unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_and_words() {
        assert_eq!(word("Plain").ok().unwrap(), "Plain");
        assert_eq!(word("Caffè").ok().unwrap(), "=?UTF-8?B?Q2FmZsOo?=");
        assert!(word("a\nb").is_err());
        let a = addresses(Some(&["\"Mario Rossi\" <m@x.it>".into(), "Zoë <z@x.it>".into(), "plain@x.it".into()])).ok().unwrap();
        assert_eq!(a, vec!["\"Mario Rossi\" <m@x.it>", "=?UTF-8?B?Wm/Dqw==?= <z@x.it>", "plain@x.it"]);
        assert!(addresses(Some(&["not an address".into()])).is_err());
        assert_eq!(from_header(Some("\"Rossi, M\" <m@x.it>, group: a@x.it;, junk")).ok().unwrap(), vec!["\"Rossi, M\" <m@x.it>", "a@x.it"]);
    }

    #[test]
    fn mime_shape() {
        let m = Message {
            to: vec!["a@x.it".into()],
            cc: vec![],
            bcc: vec![],
            reply_to: vec![],
            subject: "Hi".into(),
            in_reply_to: None,
            references: None,
            body: "line1\nline2".into(),
            html: false,
            files: vec![],
        };
        let raw = String::from_utf8(base64_decode_lenient(&mime(&m).ok().unwrap())).unwrap();
        assert_eq!(
            raw,
            "To: a@x.it\r\nSubject: Hi\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\nbGluZTENCmxpbmUy"
        );
    }
}
