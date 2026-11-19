//! Teams through the TeamsRelay /mcp endpoint: a stateless MCP client that calls one relay tool per request
//! (Streamable HTTP, answer as JSON or as an SSE stream). The bearer token goes to that one address only: no redirects,
//! and no tool argument can change where a call goes.

use crate::{Error, Result, http_client};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::Url;

pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

pub struct Teams {
    url: Url,
    token: String,
    next: AtomicU64,
}

/// The relay's answer to an attachment request: status, the two headers Everflow needs, the body as bytes.
pub struct Attachment {
    pub status: u16,
    pub content_type: Option<String>,
    pub disposition: Option<String>,
    pub body: Vec<u8>,
}

impl Teams {
    /// None when the URL is unset (Teams off). https, or http on localhost or a container name; token required.
    pub fn from_env(url: Option<&str>, token: Option<&str>) -> std::result::Result<Option<Self>, String> {
        let Some(url) = url.map(str::trim).filter(|u| !u.is_empty()) else { return Ok(None) };
        let u = crate::url::backend_url(url)
            .ok_or("OUTLOOK_MCP_TEAMS_URL must be https, or http on localhost or a container service name, without credentials, query or fragment")?;
        let t = token.map(str::trim).unwrap_or("");
        if t.is_empty() {
            return Err("OUTLOOK_MCP_TEAMS_URL needs OUTLOOK_MCP_TEAMS_TOKEN".into());
        }
        if !t.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err("OUTLOOK_MCP_TEAMS_TOKEN holds characters an HTTP header cannot carry".into());
        }
        Ok(Some(Teams { url: u, token: t.to_string(), next: AtomicU64::new(1) }))
    }

    /// The CallToolResult of one relay tool; any failure comes back as an error result, never as relay text.
    pub async fn call(&self, name: &str, args: Value) -> Value {
        match self.request(name, args).await {
            Ok(v) => v,
            Err(e) => json!({"content": [{"type": "text", "text": format!("Teams relay error: {e}")}], "isError": true}),
        }
    }

    /// The relay's saved messages since a time, for Everflow (everflow.rs): GET /api/everflow/saved at the origin of the
    /// relay address, with the same token. The status and the body as text, at most `max_bytes`.
    pub async fn saved(&self, since: u64, max_rows: u32, max_bytes: usize) -> Result<(u16, String)> {
        let mut url = self.url.join("/api/everflow/saved").map_err(|_| Error::provider("url"))?;
        url.query_pairs_mut().append_pair("since", &since.to_string()).append_pair("max_rows", &max_rows.to_string());
        let r = http_client()
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| Error::provider(format!("Teams relay: {e}")))?;
        let status = r.status().as_u16();
        Ok((status, crate::text_body(r, max_bytes).await?))
    }

    /// One image or file of a saved message, for Everflow (everflow.rs): GET /api/everflow/attachment at the origin of
    /// the relay address, with the same token. The relay may wait for a download by the agent, hence the long timeout.
    pub async fn attachment(&self, slot: u32, chat: &str, mid: &str, index: u16, max_bytes: usize) -> Result<Attachment> {
        let mut url = self.url.join("/api/everflow/attachment").map_err(|_| Error::provider("url"))?;
        url.query_pairs_mut()
            .append_pair("slot", &slot.to_string())
            .append_pair("chat", chat)
            .append_pair("mid", mid)
            .append_pair("index", &index.to_string());
        let r = http_client()
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(90))
            .send()
            .await
            .map_err(|e| Error::provider(format!("Teams relay: {e}")))?;
        let header = |name: &str| r.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from);
        let (status, content_type, disposition) = (r.status().as_u16(), header("content-type"), header("content-disposition"));
        Ok(Attachment { status, content_type, disposition, body: crate::bytes_body(r, max_bytes).await? })
    }

    async fn request(&self, name: &str, args: Value) -> std::result::Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}});
        let r = http_client()
            .post(self.url.clone())
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18")
            .body(body.to_string())
            .timeout(CALL_TIMEOUT)
            .send()
            .await
            .map_err(|e| if e.is_timeout() { "timed out".to_string() } else { "network error".to_string() })?;
        let status = r.status().as_u16();
        if status == 401 {
            return Err("the relay refused the token".into());
        }
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        let sse = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.to_lowercase().starts_with("text/event-stream"));
        let text = r.text().await.map_err(|_| "connection closed".to_string())?;
        let messages: Vec<Value> = if sse {
            sse_data(&text).iter().filter_map(|d| serde_json::from_str(d).ok()).collect()
        } else {
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Array(a)) => a,
                Ok(v) => vec![v],
                Err(_) => vec![],
            }
        };
        let answer = messages.into_iter().find(|m| m.get("id").and_then(Value::as_u64) == Some(id)).ok_or("no answer")?;
        if let Some(e) = answer.get("error") {
            return Err(format!("JSON-RPC error {}", crate::js::string(e.get("code"))));
        }
        answer.get("result").cloned().ok_or_else(|| "no result".into())
    }
}

/// The data of each SSE event: data lines joined by newlines, comments and other fields dropped.
fn sse_data(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur: Option<String> = None;
    for line in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if line.is_empty() {
            if let Some(d) = cur.take() {
                out.push(d);
            }
            continue;
        }
        if let Some(d) = line.strip_prefix("data:") {
            let d = d.strip_prefix(' ').unwrap_or(d);
            cur = Some(match cur {
                Some(c) => format!("{c}\n{d}"),
                None => d.to_string(),
            });
        }
    }
    if let Some(d) = cur {
        out.push(d);
    }
    out
}

/// A relay answer as data: error results and unparsable text are PROVIDER_ERROR.
pub fn unpack(r: &Value) -> Result<Value> {
    if crate::js::truthy_opt(r.get("isError")) {
        return Err(Error::comm("PROVIDER_ERROR", "Teams request failed"));
    }
    if let Some(s) = r.get("structuredContent").filter(|s| crate::js::truthy(s)) {
        return Ok(s.clone());
    }
    let text = r
        .get("content")
        .and_then(Value::as_array)
        .and_then(|c| c.iter().find(|x| x.get("type").and_then(Value::as_str) == Some("text")))
        .and_then(|x| x.get("text"))
        .and_then(Value::as_str);
    text.and_then(|t| serde_json::from_str(t).ok()).ok_or_else(|| Error::comm("PROVIDER_ERROR", "Invalid Teams response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_and_unpack() {
        assert_eq!(sse_data("event: message\ndata: {\"a\":1}\n\n: keepalive\n\nevent: message\r\ndata: x\r\ndata: y\r\n\r\n"), vec!["{\"a\":1}", "x\ny"]);
        assert_eq!(unpack(&json!({"content": [{"type": "text", "text": "{\"ok\":1}"}]})).ok(), Some(json!({"ok": 1})));
        assert_eq!(unpack(&json!({"structuredContent": {"s": 1}, "content": []})).ok(), Some(json!({"s": 1})));
        assert!(unpack(&json!({"isError": true, "content": []})).is_err());
        assert!(unpack(&json!({"content": [{"type": "text", "text": "nope"}]})).is_err());
    }

    #[test]
    fn config() {
        assert!(Teams::from_env(None, None).ok().unwrap().is_none());
        assert!(Teams::from_env(Some("http://webapp:8090/mcp"), Some("")).is_err());
        assert!(Teams::from_env(Some("http://example.com/mcp"), Some("t")).is_err());
        assert!(Teams::from_env(Some("http://webapp:8090/mcp"), Some("tok en")).is_err());
        assert!(Teams::from_env(Some("http://webapp:8090/mcp"), Some("tok")).ok().unwrap().is_some());
    }
}
