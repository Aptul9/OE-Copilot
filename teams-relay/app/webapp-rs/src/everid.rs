//! EverID sign-in of the pages (EVERID_MODE=on). Caddy sends the pages through the outpost of EverID (Authentik), which
//! lets a request through only for a person signed in there and adds X-Authentik-Email and X-Authentik-Groups; Caddy drops
//! those headers from every request first (caddy/Caddyfile), so here they come only from the outpost. A page request
//! with them and without a session of the same person gets a session of the user with that email (EVERID_ALIASES maps
//! another address): the same session and cookie as a password sign-in, so the API, the push of the phones and the
//! desktop work as before. No user with that email: refused, users are still added by an administrator.

use crate::auth::cookie::{self, EXPIRES_S};
use crate::auth::{read_session, rows};
use crate::web::{Req, Res, Resp, now_ms};
use hyper::header::HeaderMap;
use std::net::IpAddr;

fn is_everid_header(name: &str) -> bool {
    name.starts_with("x-authentik-")
}

/// Drops X-Authentik-* from a request whose peer is not `proxy` (the Caddy of this compose, by name): any other
/// container on the compose network reaches this port directly and could otherwise sign in as anyone. Only requests
/// that carry such a header pay the name lookup.
pub async fn screen(headers: &mut HeaderMap, peer: Option<IpAddr>, proxy: &str) {
    if !headers.keys().any(|k| is_everid_header(k.as_str())) {
        return;
    }
    if let Some(peer) = peer
        && is_host(peer, proxy).await
    {
        return;
    }
    let names: Vec<_> = headers.keys().filter(|k| is_everid_header(k.as_str())).cloned().collect();
    for name in names {
        headers.remove(name);
    }
}

async fn is_host(peer: IpAddr, host: &str) -> bool {
    let peer = peer.to_canonical();
    match tokio::net::lookup_host((host, 0)).await {
        Ok(mut addrs) => addrs.any(|a| a.ip().to_canonical() == peer),
        Err(_) => false,
    }
}

/// What a page request does about EverID.
pub enum Step {
    /// EverID off, no EverID header, or the session already belongs to that person
    Keep,
    /// a page of refusal, to send as it is
    Refuse(Resp),
    /// a new session: the Cookie header the page is rendered with, the Set-Cookie lines of its answer
    Signed { cookie: String, set_cookie: Vec<String> },
}

/// The address of the user here for an EverID address.
pub fn local_email(everid: &str, aliases: &[(String, String)]) -> String {
    let e = everid.trim().to_lowercase();
    aliases.iter().find(|(a, _)| *a == e).map(|(_, l)| l.clone()).unwrap_or(e)
}

/// X-Authentik-Groups: the groups of the person, separated by |.
pub fn in_group(groups: &str, group: &str) -> bool {
    groups.split('|').any(|g| g.trim() == group)
}

pub fn refused(text: &str) -> Resp {
    crate::web::bytes(
        403,
        &[("content-type", "text/plain; charset=utf-8".into()), ("cache-control", "no-store".into())],
        format!("{text}\n").into_bytes(),
    )
}

/// The EverID step of a page request.
pub fn step(req: &Req) -> Res<Step> {
    if !crate::config::everid_on() {
        return Ok(Step::Keep);
    }
    let Some(email) = req.header("x-authentik-email").filter(|e| !e.trim().is_empty()) else { return Ok(Step::Keep) };
    let local = local_email(&email, &crate::config::everid_aliases());
    let cookie_header = req.header("cookie");
    if let Some((_, u)) = read_session(cookie_header.as_deref(), true)?
        && u.email.to_lowercase() == local
    {
        return Ok(Step::Keep);
    }
    let group = crate::config::everid_group();
    if !in_group(&req.header("x-authentik-groups").unwrap_or_default(), &group) {
        return Ok(Step::Refuse(refused(&format!("{email} is not in the EverID group {group}"))));
    }
    let Some(u) = rows::user_by_email(&local)? else {
        return Ok(Step::Refuse(refused(&format!("No TeamsRelay user for {email}: an administrator adds it, or maps it in EVERID_ALIASES"))));
    };
    let now = now_ms();
    if u.banned == Some(true) {
        if u.ban_expires.is_some_and(|e| e < now) {
            rows::unban(&u.id, now)?;
        } else {
            return Ok(Step::Refuse(refused("You have been banned from this application")));
        }
    }
    let ip = crate::auth::limit::ip(req.header("x-forwarded-for").as_deref()).unwrap_or_default();
    let agent = req.header("user-agent").unwrap_or_default();
    let s = rows::create_session(&u.id, false, &ip, &agent, now)?;
    let line = cookie::write_signed(&cookie::name(cookie::SESSION), &s.token, &crate::config::auth_secret(), Some(EXPIRES_S));
    let pair = line.split(';').next().unwrap_or_default().to_string();
    // the cookie of the new session first: of two cookies with one name the first counts (cookie::parse_all)
    let cookie = match cookie_header {
        Some(c) if !c.is_empty() => format!("{pair}; {c}"),
        _ => pair,
    };
    Ok(Step::Signed { cookie, set_cookie: vec![line, cookie::expired(&cookie::name(cookie::DONT_REMEMBER))] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::header::{HeaderMap, HeaderValue};

    fn set_env(k: &str, v: &str) {
        // SAFETY: the tests that set the environment hold TEST_ENV
        unsafe { std::env::set_var(k, v) };
    }

    fn req(headers: &[(&'static str, &str)]) -> Req {
        let mut h = HeaderMap::new();
        for (k, v) in headers {
            h.append(*k, HeaderValue::from_str(v).expect("header"));
        }
        Req { method: "GET".into(), path: "/".into(), query: String::new(), headers: h, body: Bytes::new(), params: vec![], stream_id: None }
    }

    const SPOOF: &str = concat!("spoof", "@", "example.com");

    fn spoofed() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-authentik-email", HeaderValue::from_static(SPOOF));
        h.insert("x-authentik-groups", HeaderValue::from_static("app-teams"));
        h.insert("cookie", HeaderValue::from_static("a=b"));
        h
    }

    #[tokio::test]
    async fn everid_headers_only_from_the_proxy() {
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
        // the proxy itself keeps them
        let me = tokio::net::lookup_host(("localhost", 0)).await.expect("localhost").next().expect("address").ip();
        let mut h = spoofed();
        screen(&mut h, Some(me), "localhost").await;
        assert_eq!(h.get("x-authentik-email").expect("email"), SPOOF);
    }

    #[test]
    fn emails_and_groups() {
        let aliases = vec![("me@everid.example".to_string(), "admin@relay.example".to_string())];
        assert_eq!(local_email(" Me@EverID.example ", &aliases), "admin@relay.example");
        assert_eq!(local_email("Other@x.example", &aliases), "other@x.example");
        assert!(in_group("everid-admins|app-teams", "app-teams"));
        assert!(!in_group("app-teams-old", "app-teams"));
        assert!(!in_group("", "app-teams"));
    }

    #[test]
    fn page_sign_in() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("trweb-everid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        set_env("APP_DB", dir.join("app.db").to_str().expect("path"));
        set_env("APP_URL", "https://relay.example");
        set_env("BETTER_AUTH_SECRET", "0123456789abcdef0123456789abcdef");
        set_env("EVERID_ALIASES", "me@everid.example=admin@relay.example");
        crate::appdb::with(|c| {
            crate::appdb::migrate(c)?;
            rows::migrate(c)
        })
        .expect("schema");
        let admin = rows::create_user("admin@relay.example", "Admin", "admin", now_ms()).expect("user");
        let everid = |email: &str, groups: &str| vec![("x-authentik-email", email.to_string()), ("x-authentik-groups", groups.to_string())];
        let with = |h: &[(&'static str, String)]| req(&h.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());

        // off: the headers are ignored
        set_env("EVERID_MODE", "off");
        assert!(matches!(step(&with(&everid("me@everid.example", "app-teams"))).expect("step"), Step::Keep));

        set_env("EVERID_MODE", "on");
        // no outpost header: the page answers as before
        assert!(matches!(step(&req(&[])).expect("step"), Step::Keep));
        // outside the group, or nobody here with that address
        assert!(matches!(step(&with(&everid("me@everid.example", "app-drive"))).expect("step"), Step::Refuse(_)));
        assert!(matches!(step(&with(&everid("stranger@x.example", "app-teams"))).expect("step"), Step::Refuse(_)));

        // the alias signs in as the administrator; the new cookie is a session of that user
        let Step::Signed { cookie, set_cookie } = step(&with(&everid("me@everid.example", "app-teams"))).expect("step") else { panic!("no session") };
        assert!(set_cookie[0].starts_with("__Secure-better-auth.session_token="), "{}", set_cookie[0]);
        let (_, u) = read_session(Some(&cookie), true).expect("read").expect("session");
        assert_eq!(u.id, admin.id);

        // with that session the next page keeps it
        let mut h = everid("me@everid.example", "app-teams");
        h.push(("cookie", cookie.clone()));
        assert!(matches!(step(&with(&h)).expect("step"), Step::Keep));

        // a session of someone else is replaced by one of the EverID person
        rows::create_user("other@relay.example", "Other", "user", now_ms()).expect("user");
        set_env("EVERID_ALIASES", "");
        let mut h = everid("other@relay.example", "app-teams");
        h.push(("cookie", cookie));
        assert!(matches!(step(&with(&h)).expect("step"), Step::Signed { .. }));

        for k in ["EVERID_MODE", "EVERID_ALIASES"] {
            // SAFETY: under TEST_ENV
            unsafe { std::env::remove_var(k) };
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
