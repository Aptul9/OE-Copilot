//! The one rule for every backend address that receives a credential (OE Copilot broker or gateway, TeamsRelay /mcp):
//! https anywhere; plain http only to this machine or to a single-label host name (a service on the same container
//! network, which public DNS does not resolve). Never credentials, query or fragment in the address.

use regex::Regex;
use std::sync::LazyLock;
use url::Url;

pub fn plain_http_allowed(hostname: &str) -> bool {
    static SERVICE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z0-9](?:[a-z0-9_-]{0,61}[a-z0-9])?$").expect("fixed"));
    matches!(hostname, "localhost" | "127.0.0.1" | "::1" | "[::1]") || SERVICE_NAME.is_match(hostname)
}

pub fn backend_url(raw: &str) -> Option<Url> {
    let u = Url::parse(raw.trim()).ok()?;
    if !u.username().is_empty() || u.password().is_some() || u.query().is_some_and(|q| !q.is_empty()) || u.fragment().is_some_and(|f| !f.is_empty()) {
        return None;
    }
    match u.scheme() {
        "https" => Some(u),
        "http" if plain_http_allowed(u.host_str().unwrap_or("")) => Some(u),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule() {
        assert!(backend_url("https://example.com/x").is_some());
        assert!(backend_url("http://agent:8180/api/token-posta").is_some());
        assert!(backend_url("http://127.0.0.1:3180/").is_some());
        assert!(backend_url("http://[::1]:3180/").is_some());
        assert!(backend_url("http://example.com/").is_none());
        assert!(backend_url("https://u:p@example.com/").is_none());
        assert!(backend_url("https://example.com/?a=1").is_none());
        assert!(backend_url("https://example.com/#f").is_none());
        assert!(backend_url("ftp://agent/").is_none());
        assert!(backend_url("nonsense").is_none());
    }
}
