//! Identity header screening and local email aliases. Header values never create a session.

use hyper::header::HeaderMap;
use std::net::IpAddr;

fn is_everid_header(name: &str) -> bool {
    name.starts_with("x-authentik-")
}

/// No request may supply identity through headers, even when it comes from the proxy.
pub async fn screen(headers: &mut HeaderMap, peer: Option<IpAddr>, proxy: &str) {
    let _ = (peer, proxy);
    let names: Vec<_> = headers.keys().filter(|k| is_everid_header(k.as_str())).cloned().collect();
    for name in names {
        headers.remove(name);
    }
}

/// The address of the user here for an EverID address.
pub fn local_email(everid: &str, aliases: &[(String, String)]) -> String {
    let e = everid.trim().to_lowercase();
    aliases.iter().find(|(a, _)| *a == e).map(|(_, l)| l.clone()).unwrap_or(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::{HeaderMap, HeaderValue};

    const SPOOF: &str = concat!("spoof", "@", "example.com");

    fn spoofed() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-authentik-email", HeaderValue::from_static(SPOOF));
        h.insert("x-authentik-groups", HeaderValue::from_static("app-teams"));
        h.insert("cookie", HeaderValue::from_static("a=b"));
        h
    }

    #[tokio::test]
    async fn identity_headers_are_always_dropped() {
        let gone = |h: &HeaderMap| h.get("x-authentik-email").is_none() && h.get("x-authentik-groups").is_none();
        // another container on the network: the EverID headers are dropped, the others stay
        let mut h = spoofed();
        screen(&mut h, Some(IpAddr::from([10, 1, 2, 3])), "localhost").await;
        assert!(gone(&h));
        assert_eq!(h.get("cookie").expect("cookie"), "a=b");
        // no peer address, or a proxy name that does not resolve
        let mut h = spoofed();
        screen(&mut h, None, "localhost").await;
        assert!(gone(&h));
        let mut h = spoofed();
        screen(&mut h, Some(IpAddr::from([127, 0, 0, 1])), "no-such-proxy.invalid").await;
        assert!(gone(&h));
        // the proxy itself cannot supply them either
        let me = tokio::net::lookup_host(("localhost", 0)).await.expect("localhost").next().expect("address").ip();
        let mut h = spoofed();
        screen(&mut h, Some(me), "localhost").await;
        assert!(gone(&h));
    }

    #[test]
    fn emails_and_groups() {
        let aliases = vec![("me@everid.example".to_string(), "admin@relay.example".to_string())];
        assert_eq!(local_email(" Me@EverID.example ", &aliases), "admin@relay.example");
        assert_eq!(local_email("Other@x.example", &aliases), "other@x.example");
    }
}
