//! Server-side identity verification for the calendar login form.

use oe_nucleo::json::Json;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

pub fn attivo() -> bool {
    crate::config::var("EVERID_ACCESSO")
        .or_else(|| crate::config::var("EVERID"))
        .is_some_and(|v| crate::py::js_trim(&v).eq_ignore_ascii_case("on"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Esito {
    Dentro(String),
    NonValide,
    Fuori,
    Guasto,
}

fn base() -> Option<Url> {
    let value = crate::config::var("EVERID_URL").unwrap_or_else(|| "https://id.everbright.it".to_string());
    let u = Url::parse(value.trim()).ok()?;
    let allowed_scheme = u.scheme() == "https" || (cfg!(feature = "prove") && u.scheme() == "http" && u.host_str() == Some("127.0.0.1"));
    if !allowed_scheme || u.host_str().is_none() || !u.username().is_empty() || u.password().is_some() || u.path() != "/" || u.query().is_some() || u.fragment().is_some() {
        return None;
    }
    Some(u)
}

fn campo<'a>(j: &'a Json, k: &str) -> &'a str {
    j.get(k).and_then(Json::as_str).unwrap_or("")
}

async fn chiama(client: &reqwest::Client, jar: &reqwest::cookie::Jar, base: &Url, path: &str, body: Option<&Json>) -> Result<Json, ()> {
    use reqwest::cookie::CookieStore;
    let url = base.join(path).map_err(|_| ())?;
    if url.origin() != base.origin() { return Err(()); }
    let request = if let Some(body) = body {
        let mut r = client.post(url.clone()).header("content-type", "application/json").body(body.in_js());
        if let Some(cookies) = jar.cookies(&url).and_then(|v| v.to_str().ok().map(str::to_owned)) {
            if let Some(csrf) = cookies.split(';').map(str::trim).find_map(|c| c.strip_prefix("authentik_csrf=")) {
                r = r.header("x-authentik-csrf", csrf);
            }
        }
        r
    } else { client.get(url) };
    let mut response = request.header("accept", "application/json").send().await.map_err(|_| ())?;
    for _ in 0..5 {
        if !matches!(response.status().as_u16(), 301 | 302 | 303) { break; }
        let location = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or(())?;
        let next = response.url().join(location).map_err(|_| ())?;
        if next.origin() != base.origin() || !next.path().starts_with("/api/v3/flows/executor/") { return Err(()); }
        response = client.get(next).header("accept", "application/json").send().await.map_err(|_| ())?;
    }
    if !response.status().is_success() { return Err(()); }
    let text = response.text().await.map_err(|_| ())?;
    Json::analizza(&text).map_err(|_| ())
}

pub async fn controlla(utente: &str, password: &str) -> Esito {
    let Some(base) = base() else { return Esito::Guasto };
    let jar = Arc::new(reqwest::cookie::Jar::default());
    let Ok(client) = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .cookie_provider(jar.clone()).timeout(Duration::from_secs(15)).build() else { return Esito::Guasto };
    let Ok(brand) = chiama(&client, &jar, &base, "/api/v3/core/brands/current/", None).await else { return Esito::Guasto };
    let flow = campo(&brand, "flow_authentication");
    if flow.is_empty() || !flow.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') { return Esito::Guasto; }
    let executor = format!("/api/v3/flows/executor/{flow}/?query=");
    let Ok(mut response) = chiama(&client, &jar, &base, &executor, None).await else { return Esito::Guasto };
    let mut sent = Vec::new();
    let mut sent_password = false;
    loop {
        if response.get("response_errors").is_some() { return Esito::NonValide; }
        let step = campo(&response, "component");
        let body = match step {
            "xak-flow-redirect" if sent_password => break,
            "ak-stage-access-denied" => return Esito::Fuori,
            "ak-stage-identification" if response.get("password_fields") == Some(&Json::Bool(true)) => {
                sent_password = true;
                oe_nucleo::ogg! { "component": step, "uid_field": utente, "password": password }
            },
            "ak-stage-identification" => oe_nucleo::ogg! { "component": step, "uid_field": utente },
            "ak-stage-password" => {
                sent_password = true;
                oe_nucleo::ogg! { "component": step, "password": password }
            },
            "" => return Esito::Guasto,
            _ => return Esito::NonValide,
        };
        if sent.contains(&step.to_string()) { return Esito::NonValide; }
        sent.push(step.to_string());
        response = match chiama(&client, &jar, &base, &executor, Some(&body)).await {
            Ok(value) => value,
            Err(()) => return Esito::Guasto,
        };
    }
    let Ok(me) = chiama(&client, &jar, &base, "/api/v3/core/users/me/", None).await else { return Esito::Guasto };
    let Some(user) = me.get("user") else { return Esito::Guasto };
    if user.get("is_active") != Some(&Json::Bool(true)) { return Esito::NonValide; }
    let group = crate::config::var("EVERID_GRUPPO").filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "app-calendar".to_string());
    let member = match user.get("groups") {
        Some(Json::Arr(groups)) => groups.iter().any(|g| campo(g, "name") == group.trim()),
        _ => false,
    };
    if !member { return Esito::Fuori; }
    let email = campo(user, "email").trim().to_lowercase();
    if email.is_empty() { Esito::NonValide } else { Esito::Dentro(email) }
}
