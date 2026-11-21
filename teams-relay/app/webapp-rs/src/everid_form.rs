//! Server-side credential verification. No provider cookies or redirects reach the browser.

use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Allowed(String),
    Invalid,
    Forbidden,
    Unavailable,
}

#[derive(Default)]
struct Cookies(Vec<(String, String)>);

impl Cookies {
    fn update(&mut self, headers: &reqwest::header::HeaderMap) {
        for value in headers.get_all(reqwest::header::SET_COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            let Some((name, content)) = value.split(';').next().and_then(|v| v.split_once('=')) else { continue };
            let (name, content) = (name.trim(), content.trim());
            if name.is_empty() { continue; }
            self.0.retain(|(n, _)| n != name);
            if !content.is_empty() && !value.to_ascii_lowercase().contains("max-age=0") {
                self.0.push((name.to_owned(), content.to_owned()));
            }
        }
    }

    fn get(&self, name: &str) -> &str {
        self.0.iter().find(|(n, _)| n == name).map_or("", |(_, v)| v.as_str())
    }

    fn header(&self) -> String {
        self.0.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ")
    }
}

async fn request(client: &reqwest::Client, cookies: &mut Cookies, url: &str, body: Option<Value>) -> Result<(u16, Value), ()> {
    let post = body.is_some();
    let mut req = match body {
        Some(body) => client.post(url).header("content-type", "application/json").body(body.to_string()),
        None => client.get(url),
    }.header("accept", "application/json");
    if !cookies.0.is_empty() {
        req = req.header("cookie", cookies.header()).header("x-authentik-csrf", cookies.get("authentik_csrf"));
    }
    let mut response = req.send().await.map_err(|_| ())?;
    // The executor advances by redirecting a POST back to itself. Do not send credentials or cookies to any other URL.
    if post && response.status() == reqwest::StatusCode::FOUND {
        cookies.update(response.headers());
        let location = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or(())?;
        let original = url::Url::parse(url).map_err(|_| ())?;
        let target = original.join(location).map_err(|_| ())?;
        if target.origin() != original.origin() || target.path() != original.path() || !target.username().is_empty() || target.password().is_some() || target.fragment().is_some() {
            return Err(());
        }
        let mut next = client.get(target).header("accept", "application/json");
        if !cookies.0.is_empty() {
            next = next.header("cookie", cookies.header()).header("x-authentik-csrf", cookies.get("authentik_csrf"));
        }
        response = next.send().await.map_err(|_| ())?;
    }
    let status = response.status().as_u16();
    cookies.update(response.headers());
    let text = response.text().await.map_err(|_| ())?;
    let value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, value))
}

fn profile(value: &Value, group: &str) -> Outcome {
    let Some(user) = value.get("user") else { return Outcome::Invalid };
    if user.get("is_active").and_then(Value::as_bool) != Some(true) { return Outcome::Invalid; }
    let uid = user.get("uid").and_then(Value::as_str).unwrap_or("").trim();
    let email = user.get("email").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    if uid.is_empty() || !email.contains('@') { return Outcome::Invalid; }
    let member = user.get("groups").and_then(Value::as_array).is_some_and(|groups| {
        groups.iter().any(|g| g.get("name").and_then(Value::as_str) == Some(group))
    });
    if member { Outcome::Allowed(email) } else { Outcome::Forbidden }
}

pub async fn verify(username: &str, password: &str) -> Outcome {
    let raw = std::env::var("EVERID_AUTH_URL").unwrap_or_default();
    let Ok(url) = url::Url::parse(&raw) else { return Outcome::Unavailable };
    let local_test = cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
    if (url.scheme() != "https" && !local_test) || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Outcome::Unavailable;
    }
    verify_at(&url.origin().ascii_serialization(), username, password, &crate::config::everid_group()).await
}

async fn verify_at(base: &str, username: &str, password: &str, group: &str) -> Outcome {
    if username.trim().is_empty() || username.chars().count() > 150 || password.is_empty() || password.len() > 1024 {
        return Outcome::Invalid;
    }
    let Ok(client) = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(15)).build() else {
        return Outcome::Unavailable;
    };
    let mut cookies = Cookies::default();
    let brand = format!("{base}/api/v3/core/brands/current/");
    let Ok((200, brand)) = request(&client, &mut cookies, &brand, None).await else { return Outcome::Unavailable };
    let Some(slug) = brand.get("flow_authentication").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 128 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')) else {
        return Outcome::Unavailable;
    };
    let flow = format!("{base}/api/v3/flows/executor/{slug}/?query=");
    let Ok((200, mut challenge)) = request(&client, &mut cookies, &flow, None).await else { return Outcome::Unavailable };
    let mut sent = Vec::new();
    loop {
        let Some(component) = challenge.get("component").and_then(Value::as_str) else { return Outcome::Unavailable };
        if component == "xak-flow-redirect" {
            if !sent.iter().any(|s| s == "ak-stage-identification") { return Outcome::Invalid; }
            break;
        }
        if sent.contains(&component.to_owned()) { return Outcome::Invalid; }
        let body = match component {
            "ak-stage-identification" => json!({"component": component, "uid_field": username.trim(), "password": password}),
            "ak-stage-password" => json!({"component": component, "password": password}),
            _ => return Outcome::Invalid,
        };
        sent.push(component.to_owned());
        match request(&client, &mut cookies, &flow, Some(body)).await {
            Ok((200, value)) => challenge = value,
            Ok((400 | 401 | 403, _)) => return Outcome::Invalid,
            _ => return Outcome::Unavailable,
        }
    }
    // A redirect challenge alone is not proof: the provider must return an active authenticated user.
    let result = match request(&client, &mut cookies, &format!("{base}/api/v3/core/users/me/"), None).await {
        Ok((200, value)) => profile(&value, group),
        Ok((401 | 403, _)) => Outcome::Invalid,
        _ => Outcome::Unavailable,
    };
    let _ = request(&client, &mut cookies, &format!("{base}/api/v3/flows/executor/default-invalidation-flow/?query="), None).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn active_profile_and_exact_group_required() {
        let user = json!({"user":{"uid":"abc","email":"person@example.com","is_active":true,"groups":[{"name":"app-teams"}]}});
        assert_eq!(profile(&user, "app-teams"), Outcome::Allowed("person@example.com".into()));
        assert_eq!(profile(&user, "app-team"), Outcome::Forbidden);
        let mut inactive = user.clone();
        inactive["user"]["is_active"] = false.into();
        assert_eq!(profile(&inactive, "app-teams"), Outcome::Invalid);
        assert_eq!(profile(&json!({}), "app-teams"), Outcome::Invalid);
    }

    #[test]
    fn provider_cookies_are_retained_and_expired() {
        let mut headers = reqwest::header::HeaderMap::new();
        for value in ["authentik_csrf=one; Path=/", "authentik_session_variant=two; HttpOnly"] {
            headers.append(reqwest::header::SET_COOKIE, reqwest::header::HeaderValue::from_str(value).expect("header"));
        }
        let mut cookies = Cookies::default();
        cookies.update(&headers);
        assert_eq!(cookies.get("authentik_csrf"), "one");
        assert!(cookies.header().contains("authentik_session_variant=two"));
        let mut expired = reqwest::header::HeaderMap::new();
        expired.append(reqwest::header::SET_COOKIE, reqwest::header::HeaderValue::from_static("authentik_session_variant=; Max-Age=0"));
        cookies.update(&expired);
        assert!(!cookies.header().contains("authentik_session_variant"));
    }

    async fn mock(scenario: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            let mut posts = 0;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { break };
                let mut data = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).await.expect("read");
                    if n == 0 { break; }
                    data.extend_from_slice(&buf[..n]);
                    let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                    let headers = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                    let length = headers.lines().find_map(|line| line.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if data.len() < end + 4 + length { continue; }
                    break;
                }
                let text = String::from_utf8_lossy(&data);
                let (head, body) = text.split_once("\r\n\r\n").expect("request");
                let first = head.lines().next().expect("request line");
                let (status, response, extra) = if first.starts_with("GET /api/v3/core/brands/current/") {
                    let slug = if scenario == "bad-slug" { "../outside" } else { "accesso" };
                    (200, json!({"flow_authentication":slug}), "Set-Cookie: authentik_csrf=csrf; Path=/\r\nSet-Cookie: authentik_session_opaque=before; Path=/\r\n".to_string())
                } else if first.starts_with("GET /api/v3/core/users/me/") {
                    assert!(head.contains("authentik_session_opaque=after"));
                    let group = if scenario == "forbidden" { "app-other" } else { "app-teams" };
                    (200, json!({"user":{"uid":"abc","email":"user@example.com","is_active":true,"groups":[{"name":group}]}}), String::new())
                } else if first.starts_with("GET /api/v3/flows/executor/default-invalidation-flow/") {
                    (200, json!({"component":"xak-flow-redirect"}), String::new())
                } else if first.starts_with("GET /api/v3/flows/executor/accesso/") {
                    assert!(head.to_ascii_lowercase().contains("x-authentik-csrf: csrf"));
                    if posts > 0 { assert!(head.contains("authentik_session_opaque=after")); }
                    let component = if posts == 0 || scenario == "invalid" { "ak-stage-identification" }
                        else if scenario == "mfa" { "ak-stage-authenticator-validate" }
                        else if posts == 1 { "ak-stage-password" }
                        else { "xak-flow-redirect" };
                    if scenario == "redirect-loop" && posts > 0 {
                        (302, json!({}), "Location: /api/v3/flows/executor/accesso/?query=\r\n".to_string())
                    } else {
                        (200, json!({"component":component}), String::new())
                    }
                } else {
                    assert!(first.starts_with("POST /api/v3/flows/executor/accesso/"), "unexpected request");
                    assert!(head.to_ascii_lowercase().contains("x-authentik-csrf: csrf"));
                    assert!(head.contains("authentik_session_opaque="));
                    let sent: Value = serde_json::from_str(body).expect("body");
                    posts += 1;
                    if posts == 1 {
                        assert_eq!(sent["uid_field"], "alice");
                        assert_eq!(sent["password"], "test-only");
                    }
                    let target = match scenario {
                        "external" => "https://attacker.example/api/v3/flows/executor/accesso/?query=".to_string(),
                        "other-path" => "/api/v3/core/users/me/".to_string(),
                        "absolute" => format!("http://{addr}/api/v3/flows/executor/accesso/?query=next"),
                        "relative" => "?query=next".to_string(),
                        _ => "/api/v3/flows/executor/accesso/?query=".to_string(),
                    };
                    (302, json!({}), format!("Location: {target}\r\nSet-Cookie: authentik_session_opaque=after; Path=/\r\n"))
                };
                let response = response.to_string();
                let output = format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len());
                stream.write_all(output.as_bytes()).await.expect("write");
            }
        });
        (format!("http://{addr}"), task)
    }

    #[tokio::test]
    async fn executor_flow_accepts_only_valid_active_members() {
        for (scenario, expected) in [
            ("allowed", Outcome::Allowed("user@example.com".into())),
            ("absolute", Outcome::Allowed("user@example.com".into())),
            ("relative", Outcome::Allowed("user@example.com".into())),
            ("forbidden", Outcome::Forbidden),
            ("invalid", Outcome::Invalid),
            ("mfa", Outcome::Invalid),
            ("bad-slug", Outcome::Unavailable),
            ("external", Outcome::Unavailable),
            ("other-path", Outcome::Unavailable),
            ("redirect-loop", Outcome::Unavailable),
        ] {
            let (base, task) = mock(scenario).await;
            assert_eq!(verify_at(&base, "alice", "test-only", "app-teams").await, expected, "{scenario}");
            task.abort();
        }
    }

    #[tokio::test]
    async fn local_session_login_and_logout() {
        use crate::auth::{read_session, rows};
        use crate::web::{Req, now_ms};
        use hyper::header::{HeaderMap, HeaderValue};
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("tr-form-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let (base, task) = mock("allowed").await;
        for (k, v) in [
            ("APP_DB", dir.join("app.db").to_string_lossy().to_string()),
            ("APP_URL", "https://relay.example".into()),
            ("BETTER_AUTH_SECRET", "0123456789abcdef0123456789abcdef".into()),
            ("EVERID_MODE", "on".into()),
            ("EVERID_AUTH_URL", base),
            ("EVERID_ALIASES", "user@example.com=⟦RG:5f84b124⟧".into()),
            ("PASSWORD_LOGIN", "0".into()),
        ] {
            // SAFETY: this test holds the environment lock.
            unsafe { std::env::set_var(k, v) };
        }
        crate::appdb::with(|c| { crate::appdb::migrate(c)?; rows::migrate(c) }).expect("schema");
        let local = rows::create_user("⟦RG:5f84b124⟧", "Alice", "user", now_ms()).expect("user");
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert("origin", HeaderValue::from_static("https://relay.example"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("10.142.158.30"));
        let mut req = Req { method: "POST".into(), path: "/api/auth/sign-in/email".into(), query: String::new(), headers, body: json!({"email":"alice","password":"test-only"}).to_string().into(), params: vec![], stream_id: None };
        let response = crate::auth::routes::handle(&req).await;
        assert_eq!(response.status(), 200);
        let cookie = response.headers().get("set-cookie").expect("session cookie").to_str().expect("header").split(';').next().expect("pair").to_owned();
        let (_, user) = read_session(Some(&cookie), true).expect("read").expect("session");
        assert_eq!(user.id, local.id);
        req.path = "/api/auth/sign-out".into();
        req.body = "{}".into();
        req.headers.insert("cookie", HeaderValue::from_str(&cookie).expect("cookie"));
        assert_eq!(crate::auth::routes::handle(&req).await.status(), 200);
        assert!(read_session(Some(&cookie), true).expect("read").is_none());
        task.abort();
        for k in ["APP_DB", "APP_URL", "BETTER_AUTH_SECRET", "EVERID_MODE", "EVERID_AUTH_URL", "EVERID_ALIASES", "PASSWORD_LOGIN"] {
            // SAFETY: this test holds the environment lock.
            unsafe { std::env::remove_var(k) };
        }
    }
}
