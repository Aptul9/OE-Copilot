//! The OE Copilot agent's communication API (Gmail, Google Chat): OE Copilot owns the grants, this server sends bounded
//! requests to a fixed list of endpoints.

use crate::{Error, Result, http_client};
use serde_json::Value;
use std::time::Duration;
use url::Url;

const ENDPOINTS: [&str; 7] = [
    "/api/communication/accounts",
    "/api/communication/read",
    "/api/communication/google/accounts",
    "/api/communication/gmail/read",
    "/api/communication/gmail/write",
    "/api/communication/gchat/read",
    "/api/communication/gchat/write",
];

pub struct Gateway {
    base: Url,
    key: String,
}

impl Gateway {
    /// The origin of the broker address: the agent serves both under one host.
    pub fn from_broker(broker_url: &str, key: &str) -> Result<Self> {
        let parsed = Url::parse(broker_url).map_err(|_| Error::provider("Invalid communication service URL"))?;
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some_and(|q| !q.is_empty())
            || parsed.fragment().is_some_and(|f| !f.is_empty())
        {
            return Err(Error::provider("Invalid communication service URL"));
        }
        let base = parsed.join("/").map_err(|_| Error::provider("Invalid communication service URL"))?;
        let base = crate::url::backend_url(base.as_str()).filter(|u| u.path() == "/").ok_or_else(|| Error::provider("Invalid communication service URL"))?;
        Ok(Gateway { base, key: key.to_string() })
    }

    pub async fn call(&self, path: &str, body: Option<&Value>) -> Result<Value> {
        if !ENDPOINTS.contains(&path) {
            return Err(Error::provider("Unknown communication endpoint"));
        }
        let url = self.base.join(path).map_err(|_| Error::provider("url"))?;
        let mut req = match body {
            Some(_) => http_client().post(url),
            None => http_client().get(url),
        };
        req = req.header("Authorization", format!("Bearer {}", self.key)).header("Content-Type", "application/json").timeout(Duration::from_secs(45));
        if let Some(b) = body {
            req = req.body(serde_json::to_vec(b).unwrap_or_default());
        }
        let r = req.send().await.map_err(|e| Error::provider(format!("Communication service: {e}")))?;
        let status = r.status().as_u16();
        let payload: Value =
            crate::json_body(r).await.ok_or(()).map_err(|_| Error::provider(format!("Communication service returned invalid JSON (HTTP {status})")))?;
        if !(200..300).contains(&status) {
            let msg = payload.get("errore").map(|v| crate::js::string(Some(v))).unwrap_or_else(|| "request failed".into());
            return Err(Error::provider(format!("Communication service HTTP {status}: {}", msg.chars().take(200).collect::<String>())));
        }
        Ok(payload)
    }

    /// The same call for Everflow (everflow.rs): any status, the body as text, at most `max_bytes`. Not reaching the
    /// agent is UNREACHABLE.
    pub async fn raw(&self, path: &str, body: Option<&Value>, max_bytes: usize) -> Result<(u16, String)> {
        if !ENDPOINTS.contains(&path) {
            return Err(Error::provider("Unknown communication endpoint"));
        }
        let url = self.base.join(path).map_err(|_| Error::provider("url"))?;
        let mut req = match body {
            Some(_) => http_client().post(url),
            None => http_client().get(url),
        };
        req = req.header("Authorization", format!("Bearer {}", self.key)).header("Content-Type", "application/json").timeout(Duration::from_secs(30));
        if let Some(b) = body {
            req = req.body(serde_json::to_vec(b).unwrap_or_default());
        }
        let r = req.send().await.map_err(|_| Error::comm("UNREACHABLE", "OE Copilot agent unreachable"))?;
        let status = r.status().as_u16();
        Ok((status, crate::text_body(r, max_bytes).await?))
    }
}
