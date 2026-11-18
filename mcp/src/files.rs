//! Download links: `download: true` on email_attachment or chat_attachment answers a link valid ten minutes in place of
//! the bytes, so a client saves the file with one GET and the bytes never pass through the model.
//!   GET /files/<ticket>   no bearer: the ticket is the signed tool call and its expiry (identity.rs); Host in
//!                         MCP_HTTP_HOSTS, no Origin, as /mcp
//! Nothing is kept on the server: the attachment is fetched from the provider when the link is used.

use crate::communication::{Args, Communication, run};
use crate::http::{App, Body, empty, header, json};
use crate::identity::{open_ticket, resource, ticket};
use crate::js::{base64_decode_lenient, encode_uri_component, string};
use crate::{Error, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const TTL_SECS: u64 = 600;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// MCP_PUBLIC_URL, or the path the TeamsRelay Caddy proxies on the first public MCP_HTTP_HOSTS name (README.md).
fn public_url() -> Option<&'static str> {
    static URL: OnceLock<Option<String>> = OnceLock::new();
    URL.get_or_init(|| {
        let env = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        env("MCP_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()).or_else(|| {
            let hosts = env("MCP_HTTP_HOSTS")?;
            public_host(&hosts).map(|h| format!("https://{h}/communication"))
        })
    })
    .as_deref()
}

/// The first name of MCP_HTTP_HOSTS that is not loopback.
fn public_host(hosts: &str) -> Option<String> {
    hosts
        .split(',')
        .map(|h| h.trim().to_lowercase())
        .find(|h| !h.is_empty() && !h.starts_with("127.") && !h.starts_with("localhost") && !h.starts_with("[::1]"))
}

/// The answer of a call with `download: true`: the handles are checked now, the attachment is fetched on the GET.
pub async fn link(c: &Communication, name: &str, x: &Args) -> Result<Value> {
    let a = c.account(x.get("account_id").and_then(Value::as_str).unwrap_or("")).await?;
    let kinds: &[(&str, &str)] = if name == "email_attachment" {
        &[("attachment_id", "email_attachment")]
    } else {
        &[("attachment_id", "chat_attachment"), ("conversation_id", "conversation"), ("message_id", "chat_message")]
    };
    for (key, kind) in kinds {
        if let Some(id) = x.get(*key).and_then(Value::as_str) {
            resource(&a.account_id, kind, id)?;
        }
    }
    let base = public_url().ok_or_else(|| Error::comm("UNSUPPORTED_CAPABILITY", "Download links need MCP_PUBLIC_URL on the server"))?;
    let mut args = x.clone();
    args.remove("download");
    let exp = now() + TTL_SECS;
    let expires_at = jiff::Timestamp::from_second(exp as i64).map(|t| t.to_string()).unwrap_or_default();
    let url = format!("{base}/files/{}", ticket(name, &Value::Object(args), exp));
    Ok(json!({"account_id": a.account_id, "provider": a.provider, "data": {"download_url": url, "expires_at": expires_at}}))
}

struct File {
    bytes: Vec<u8>,
    mime: String,
    name: Option<String>,
}

/// The file in the answer of email_attachment or chat_attachment, whatever the provider.
fn file_of(r: &Value) -> Result<File> {
    let d = r.get("data").unwrap_or(&Value::Null);
    let text = |v: Option<&Value>| v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    let mime_or = |v: Option<&Value>| text(v).unwrap_or_else(|| "application/octet-stream".into());
    // Outlook FileAttachment; an attached item or a link has no ContentBytes
    if let Some(b) = d.get("ContentBytes").and_then(Value::as_str) {
        return Ok(File { bytes: base64_decode_lenient(b), mime: mime_or(d.get("ContentType")), name: text(d.get("Name")) });
    }
    // Teams: the summary, then one block per attachment it could carry
    if let Some(blocks) = d.get("content").and_then(Value::as_array) {
        let summary: Value = serde_json::from_str(blocks.first().and_then(|b| b.get("text")).and_then(Value::as_str).unwrap_or("")).unwrap_or(Value::Null);
        let listed = summary.get("attachments").and_then(Value::as_array).cloned().unwrap_or_default();
        let files = &blocks[1.min(blocks.len())..];
        if files.len() > 1 || listed.len() > 1 {
            return Err(Error::comm("INVALID_INPUT", format!("The message has {} attachments: pass index", listed.len().max(files.len()))));
        }
        let Some(f) = files.first() else {
            let note = listed.first().and_then(|a| a.get("note")).map(|n| string(Some(n))).unwrap_or_else(|| "Teams did not give the file".into());
            return Err(Error::comm("NO_FILE", note));
        };
        let name = listed.first().and_then(|a| text(a.get("name")));
        return match f.get("type").and_then(Value::as_str) {
            Some("image") => Ok(File { bytes: base64_decode_lenient(&string(f.get("data"))), mime: mime_or(f.get("mimeType")), name }),
            Some("resource") => {
                let res = f.get("resource").unwrap_or(&Value::Null);
                Ok(File { bytes: base64_decode_lenient(&string(res.get("blob"))), mime: mime_or(res.get("mimeType")), name })
            }
            // a text file comes as "--- <name> ---\n<text>"
            Some("text") => {
                let t = string(f.get("text"));
                let body = t.split_once('\n').map_or("", |(_, b)| b).to_string();
                Ok(File { bytes: body.into_bytes(), mime: "text/plain; charset=utf-8".into(), name })
            }
            _ => Err(Error::comm("NO_FILE", "Teams did not give the file")),
        };
    }
    // Gmail {data: base64url, size}; Google Chat and Slack {data, mime_type}
    if let Some(b) = d.get("data").and_then(Value::as_str) {
        return Ok(File { bytes: base64_decode_lenient(b), mime: mime_or(d.get("mime_type")), name: None });
    }
    Err(Error::comm("NO_FILE", "This attachment has no file content (an attached item or a link)"))
}

fn error(status: u16, code: &str, message: &str) -> Response<Body> {
    json(status, &json!({"code": code, "message": message}), &[("Cache-Control", "no-store")])
}

/// `attachment; filename="<ASCII>"; filename*=UTF-8''<name>`
fn disposition(name: &str) -> String {
    let plain: String = name.chars().map(|c| if (c.is_ascii_graphic() && c != '"' && c != '\\') || c == ' ' { c } else { '_' }).collect();
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{}", encode_uri_component(name))
}

pub async fn handle(app: &App, req: Request<Incoming>) -> Response<Body> {
    let host = header(&req, "host").unwrap_or("").to_lowercase();
    if !app.hosts.contains(&host) {
        return empty(421, &[]);
    }
    if req.headers().contains_key("origin") {
        return empty(403, &[]);
    }
    if req.method() != hyper::Method::GET {
        return empty(405, &[("Allow", "GET")]);
    }
    let t = req.uri().path().strip_prefix("/files/").unwrap_or("");
    let Some((tool, args, exp)) = open_ticket(t) else { return error(404, "INVALID_LINK", "Unknown download link") };
    if exp < now() {
        return error(410, "LINK_EXPIRED", "Download link expired: call the tool again with download true");
    }
    let f = match run(&app.comm, &tool, &args).await.and_then(|r| file_of(&r)) {
        Ok(f) => f,
        Err(Error::Comm { code, message }) => return error(400, code, &message),
        Err(Error::Provider(_)) => return error(502, "PROVIDER_ERROR", "Provider request failed"),
    };
    let mime = if hyper::header::HeaderValue::from_str(&f.mime).is_ok() { f.mime } else { "application/octet-stream".into() };
    let mut r = Response::builder().status(200).header("content-type", mime).header("cache-control", "no-store").header("x-content-type-options", "nosniff");
    if let Some(n) = &f.name {
        r = r.header("content-disposition", disposition(n));
    }
    r.body(Full::new(Bytes::from(f.bytes)).boxed_unsync()).expect("response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_of_every_provider() {
        let f = file_of(&json!({"data": {"Name": "a.pdf", "ContentType": "application/pdf", "ContentBytes": "aGk="}})).ok().unwrap();
        assert_eq!((f.bytes.as_slice(), f.mime.as_str(), f.name.as_deref()), (&b"hi"[..], "application/pdf", Some("a.pdf")));
        let f = file_of(&json!({"data": {"data": "_-8", "size": 2}})).ok().unwrap();
        assert_eq!((f.bytes.as_slice(), f.mime.as_str()), (&[0xff, 0xef][..], "application/octet-stream"));
        let f = file_of(&json!({"data": {"data": "aGk=", "mime_type": "image/png"}})).ok().unwrap();
        assert_eq!(f.mime, "image/png");
        let summary = |n: usize| json!({"type": "text", "text": json!({"attachments": vec![json!({"name": "r.txt"}); n]}).to_string()});
        let f = file_of(&json!({"data": {"content": [summary(1), {"type": "text", "text": "--- r.txt ---\nline\n"}]}})).ok().unwrap();
        assert_eq!((f.bytes.as_slice(), f.name.as_deref()), (&b"line\n"[..], Some("r.txt")));
        let f =
            file_of(&json!({"data": {"content": [summary(1), {"type": "resource", "resource": {"uri": "u", "mimeType": "application/zip", "blob": "aGk="}}]}}));
        assert_eq!(f.ok().unwrap().mime, "application/zip");
        let code = |r: Result<File>| match r {
            Err(Error::Comm { code, .. }) => code,
            _ => "ok",
        };
        assert_eq!(code(file_of(&json!({"data": {"content": [summary(2)]}}))), "INVALID_INPUT");
        assert_eq!(code(file_of(&json!({"data": {"content": [summary(1)]}}))), "NO_FILE");
        assert_eq!(code(file_of(&json!({"data": {"@odata.type": "#Microsoft.OutlookServices.ItemAttachment"}}))), "NO_FILE");
    }

    #[test]
    fn public_name() {
        assert_eq!(public_host("127.0.0.1:8190, Teams.Example.com").as_deref(), Some("teams.example.com"));
        assert_eq!(public_host("localhost:8190,[::1]:8190"), None);
    }

    #[test]
    fn content_disposition() {
        assert_eq!(disposition("Q3 \"draft\".pdf"), "attachment; filename=\"Q3 _draft_.pdf\"; filename*=UTF-8''Q3%20%22draft%22.pdf");
        assert_eq!(disposition("caffè.txt"), "attachment; filename=\"caff_.txt\"; filename*=UTF-8''caff%C3%A8.txt");
    }
}
