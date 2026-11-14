//! Better Auth's rate limit, in memory: per address and path, a rolling window. The default rules of a production
//! Better Auth without its own: 100 requests per 10 seconds, 3 per 10 seconds on /sign-in, /sign-up, /change-password
//! and /change-email.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

const MAX_ENTRIES: usize = 100_000;

#[derive(Clone, Copy)]
struct Entry {
    count: u32,
    last: i64,
    expires: i64,
}

static MEMORY: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

/// getIP: an X-Forwarded-For holding one valid address (no trusted proxies), IPv6 reduced to its /64.
pub fn ip(xff: Option<&str>) -> Option<String> {
    let values: Vec<&str> = xff?.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let [one] = values.as_slice() else { return None };
    let ip: IpAddr = one.parse().ok()?;
    Some(match ip {
        IpAddr::V4(v) => v.to_string(),
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let g = v.segments();
                (0..8).map(|i| format!("{:04x}", if i < 4 { g[i] } else { 0 })).collect::<Vec<_>>().join(":")
            }
        },
    })
}

fn rule(path: &str) -> (i64, u32) {
    if ["/sign-in", "/sign-up", "/change-password", "/change-email"].iter().any(|p| path.starts_with(p)) { (10, 3) } else { (10, 100) }
}

/// normalizePathname: without /api/auth in front and without trailing slashes.
pub fn normalized(pathname: &str) -> String {
    let p = pathname.trim_end_matches('/');
    let p = if p.is_empty() { "/" } else { p };
    if p == "/api/auth" {
        return "/".into();
    }
    match p.strip_prefix("/api/auth/") {
        Some(rest) => {
            let r = format!("/{rest}");
            let r = r.trim_end_matches('/');
            if r.is_empty() { "/".into() } else { r.to_string() }
        }
        None => p.to_string(),
    }
}

/// One step: None when the request passes, Some(seconds to wait) when not.
pub fn consume(pathname: &str, xff: Option<&str>, now: i64) -> Option<i64> {
    let path = normalized(pathname);
    let k = format!("{}|{path}", ip(xff).unwrap_or_else(|| "no-trusted-ip".into()));
    let (window, max) = rule(&path);
    let mut g = MEMORY.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    m.retain(|_, v| now < v.expires);
    if m.len() > MAX_ENTRIES {
        let extra: Vec<String> = m.keys().take(m.len() - MAX_ENTRIES).cloned().collect();
        for x in extra {
            m.remove(&x);
        }
    }
    let before = m.get(&k).copied().filter(|v| now < v.expires);
    let (count, allowed, wait) = match before {
        None => (1, true, None),
        Some(v) if now - v.last >= window * 1000 => (1, true, None),
        Some(v) if v.count >= max => (v.count, false, Some(((v.last + window * 1000 - now) as f64 / 1000.0).ceil() as i64)),
        Some(v) => (v.count + 1, true, None),
    };
    if allowed {
        m.insert(k, Entry { count, last: now, expires: now + window * 1000 });
    }
    wait
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_sign_ins_per_ten_seconds() {
        for _ in 0..3 {
            assert_eq!(consume("/api/auth/sign-in/email", Some("9.9.9.9"), 1000), None);
        }
        assert_eq!(consume("/api/auth/sign-in/email", Some("9.9.9.9"), 2000), Some(9));
        assert_eq!(consume("/api/auth/sign-in/email", Some("9.9.9.9"), 11_000), None);
        assert_eq!(ip(Some("1.2.3.4, 5.6.7.8")), None);
        assert_eq!(normalized("/api/auth/get-session/"), "/get-session");
    }
}
