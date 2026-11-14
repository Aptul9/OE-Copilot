//! Access tokens for Outlook REST from the OE Copilot token broker (GET /api/token-posta). OE Copilot owns sign-in and
//! renewal of every mailbox; this process only reads current access tokens and keeps them in memory, never on disk.

use crate::js::{base64_decode_lenient, number, string};
use crate::{Error, Result, http_client};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

const MARGIN_S: i64 = 120;

#[derive(Clone, Debug)]
pub struct Account {
    pub upn: String,
    pub token: String,
    pub exp: i64,
    pub status: String,
}

pub fn select_account<'a>(all: &'a [Account], selector: Option<&str>) -> Result<&'a Account> {
    let want = selector.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    if want.is_empty() {
        return all.iter().find(|a| a.status == "ok").or(all.first()).ok_or_else(|| Error::provider("No mailbox available"));
    }
    if let Some(a) = all.iter().find(|a| a.upn.to_lowercase() == want) {
        return Ok(a);
    }
    let hits: Vec<&Account> = all.iter().filter(|a| a.upn.to_lowercase().contains(&want)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(Error::provider("Mailbox not found")),
        _ => Err(Error::provider("Mailbox selector is ambiguous")),
    }
}

pub fn claims(jwt: &str) -> serde_json::Map<String, Value> {
    let Some(part) = jwt.split('.').nth(1).filter(|p| !p.is_empty()) else { return Default::default() };
    match serde_json::from_slice::<Value>(&base64_decode_lenient(part)) {
        Ok(Value::Object(m)) => m,
        _ => Default::default(),
    }
}

/// The broker rows that hold a usable Outlook token: audience, mail permission, the same mailbox as the row and time
/// left are read from the token itself, never from the row. The API address is fixed in outlook.rs.
pub fn accounts_from(payload: &Value, now_s: i64) -> Vec<Account> {
    let Some(rows) = payload.get("account").and_then(Value::as_array) else { return vec![] };
    let mut out: Vec<Account> = vec![];
    for r in rows {
        let (Some(token), Some(row_upn)) = (r.get("token").and_then(Value::as_str), r.get("upn").and_then(Value::as_str)) else { continue };
        let c = claims(token);
        let pick = crate::js::or(&[c.get("upn"), c.get("preferred_username"), c.get("unique_name")]);
        let upn = match pick {
            Some(v) if crate::js::truthy(v) => string(Some(v)).to_lowercase(),
            _ => String::new(),
        };
        if upn.is_empty() || upn != row_upn.to_lowercase() {
            continue;
        }
        let aud = c.get("aud").and_then(Value::as_str);
        if aud != Some("https://outlook.office.com") && aud != Some("https://outlook.office365.com") {
            continue;
        }
        let scp = match c.get("scp") {
            None | Some(Value::Null) => String::new(),
            v => string(v),
        };
        let scopes: Vec<&str> = scp.split(' ').collect();
        if !scopes.contains(&"Mail.Read") && !scopes.contains(&"Mail.ReadWrite") {
            continue;
        }
        let claim_exp = number(c.get("exp"));
        let row_exp = if r.get("exp").is_none() { claim_exp } else { number(r.get("exp")) };
        if !claim_exp.is_finite() || !row_exp.is_finite() {
            continue;
        }
        let exp = claim_exp.min(row_exp);
        if exp <= (now_s + MARGIN_S) as f64 {
            continue;
        }
        if out.iter().any(|a| a.upn == upn) {
            continue;
        }
        let status = match r.get("phone_status") {
            None | Some(Value::Null) => "ok".into(),
            v => string(v),
        };
        out.push(Account { upn, token: token.to_string(), exp: exp as i64, status });
    }
    out
}

pub fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub struct Tokens {
    url: Url,
    key: String,
    cache: std::sync::Mutex<Vec<Account>>,
    load: Mutex<()>,
}

impl Tokens {
    /// HTTP straight to the broker: https, or http on this machine or its container network only (url.rs); no
    /// redirects, so the key goes nowhere else.
    pub fn new(url: &str, key: &str) -> Result<Self> {
        let url = crate::url::backend_url(url).ok_or_else(|| {
            Error::provider("OUTLOOK_MCP_BROKER_URL must be https, or http on localhost or a container service name, without credentials or query")
        })?;
        Ok(Tokens { url, key: key.to_string(), cache: Default::default(), load: Mutex::new(()) })
    }

    async fn fetch(&self) -> Result<Value> {
        let r = http_client()
            .get(self.url.clone())
            .header("Authorization", format!("Bearer {}", self.key))
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| Error::provider(format!("Token broker: {e}")))?;
        if !r.status().is_success() {
            return Err(Error::provider(format!("Token broker answered HTTP {}", r.status().as_u16())));
        }
        crate::json_body(r).await.ok_or_else(|| Error::provider("Token broker answer is not JSON"))
    }

    pub async fn accounts(&self, refresh: bool) -> Result<Vec<Account>> {
        let fresh = |cache: &Vec<Account>| {
            let now = now_s();
            let f: Vec<Account> = cache.iter().filter(|a| a.exp > now + MARGIN_S).cloned().collect();
            (!f.is_empty() && f.len() == cache.len()).then_some(f)
        };
        if !refresh && let Some(f) = fresh(&self.cache.lock().expect("cache")) {
            return Ok(f);
        }
        let _one = self.load.lock().await;
        // a load that finished while this call waited serves it too
        if !refresh && let Some(f) = fresh(&self.cache.lock().expect("cache")) {
            return Ok(f);
        }
        let payload = self.fetch().await?;
        let list = accounts_from(&payload, now_s());
        *self.cache.lock().expect("cache") = list.clone();
        Ok(list)
    }

    /// Exact address first, then a fragment matching one mailbox only; omitted: the first mailbox.
    pub async fn pick(&self, account: Option<&str>, refresh: bool) -> Result<Account> {
        let all = self.accounts(refresh).await?;
        if all.is_empty() {
            return Err(Error::provider("No Outlook mailbox available from the OE Copilot token broker"));
        }
        match select_account(&all, account) {
            Ok(a) => Ok(a.clone()),
            Err(Error::Provider(m)) if m.contains("ambiguous") => Err(Error::provider(format!("'{}' matches several mailboxes", account.unwrap_or("")))),
            Err(_) if !refresh => Box::pin(self.pick(account, true)).await,
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(c: Value) -> String {
        format!("h.{}.s", crate::js::base64_url(c.to_string().as_bytes()))
    }

    #[test]
    fn only_usable_rows() {
        let now = 1_800_000_000;
        let good = |upn: &str, exp: i64| jwt(json!({"upn": upn, "aud": "https://outlook.office.com", "scp": "Mail.ReadWrite Calendars.Read", "exp": exp}));
        let payload = json!({"account": [
            {"upn": "A@x.it", "token": good("a@x.it", now + 3600), "phone_status": "ok"},
            {"upn": "b@x.it", "token": good("c@x.it", now + 3600)},
            {"upn": "d@x.it", "token": good("d@x.it", now + 60)},
            {"upn": "e@x.it", "token": jwt(json!({"upn": "e@x.it", "aud": "https://graph.microsoft.com", "scp": "Mail.Read", "exp": now + 3600}))},
            {"upn": "f@x.it", "token": jwt(json!({"upn": "f@x.it", "aud": "https://outlook.office365.com", "scp": "Calendars.Read", "exp": now + 3600}))},
            {"upn": "g@x.it", "token": good("g@x.it", now + 3600), "exp": now + 100},
            {"upn": "h@x.it", "token": good("h@x.it", now + 3600), "exp": "x"},
            {"upn": "a@x.it", "token": good("a@x.it", now + 7200)},
            {"upn": "i@x.it", "token": good("i@x.it", now + 3600), "phone_status": "errore"},
        ]});
        let got: Vec<(String, String)> = accounts_from(&payload, now).into_iter().map(|a| (a.upn, a.status)).collect();
        assert_eq!(got, vec![("a@x.it".into(), "ok".into()), ("i@x.it".into(), "errore".into())]);
    }

    #[test]
    fn selectors() {
        let a = |u: &str, s: &str| Account { upn: u.into(), token: String::new(), exp: 0, status: s.into() };
        let all = vec![a("one@msc.com", "errore"), a("two@serco.eu", "ok"), a("three@serco.it", "ok")];
        assert_eq!(select_account(&all, None).ok().unwrap().upn, "two@serco.eu");
        assert_eq!(select_account(&all, Some("MSC")).ok().unwrap().upn, "one@msc.com");
        assert!(matches!(select_account(&all, Some("serco")), Err(Error::Provider(m)) if m.contains("ambiguous")));
        assert!(select_account(&all, Some("zzz")).is_err());
    }
}
