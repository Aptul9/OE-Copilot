//! The environment of the web app, read at call time like src/lib/config.ts: tests and the server see the variables they
//! run with.

use std::path::PathBuf;

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// `process.env.X || fallback`: an empty variable counts as unset.
fn or(name: &str, fallback: &str) -> String {
    let v = var(name);
    if v.is_empty() { fallback.to_string() } else { v }
}

fn positive_int(name: &str) -> Option<i64> {
    // Number.parseInt: leading digits, the rest ignored
    let v = var(name);
    let t = v.trim_start();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    let d: String = digits.chars().take_while(char::is_ascii_digit).collect();
    let n = d.parse::<i64>().ok()? * sign;
    (n > 0).then_some(n)
}

pub fn app_db() -> String {
    or("APP_DB", "/data/app.db")
}

pub fn data_dir() -> PathBuf {
    PathBuf::from(app_db()).parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// The unix socket of the supervisor of the browsers container.
pub fn control_socket() -> String {
    or("CONTROL_SOCKET", "/run/teamsrelay/control.sock")
}

/// Accounts one user may add, None for no cap.
pub fn accounts_per_user() -> Option<i64> {
    positive_int("ACCOUNTS_PER_USER")
}

pub const DEFAULT_DESKTOP_URL: &str = "/api/desktop/{n}";

pub fn desktop_url() -> String {
    or("DESKTOP_URL", DEFAULT_DESKTOP_URL)
}

pub fn desktop_url_of(slot: i64) -> String {
    desktop_url().replace("{n}", &slot.to_string())
}

/// RELAY_QUOTA_MB, 2048 by default: room of an account on another computer for its files on this server.
pub fn relay_quota_bytes() -> f64 {
    let mb = crate::js::number(Some(&serde_json::Value::String(var("RELAY_QUOTA_MB"))));
    let mb = if std::env::var("RELAY_QUOTA_MB").is_err() { f64::NAN } else { mb };
    (if mb.is_finite() && mb > 0.0 { mb } else { 2048.0 }) * 1048576.0
}

/// Public URL of the app: APP_URL, otherwise https://DOMAIN (DOMAIN may already carry a scheme), no trailing slash.
pub fn app_url() -> String {
    let explicit = var("APP_URL");
    let raw = if !explicit.is_empty() {
        explicit
    } else {
        let domain = or("DOMAIN", "localhost:8090");
        if domain.starts_with("http://") || domain.starts_with("https://") { domain } else { format!("https://{domain}") }
    };
    raw.trim_end_matches('/').to_string()
}

/// BETTER_AUTH_SECRET, or SESSION_SECRET of the previous release.
pub fn auth_secret() -> String {
    let s = var("BETTER_AUTH_SECRET");
    if s.is_empty() { var("SESSION_SECRET") } else { s }
}

/// Bearer token of /mcp; empty turns the endpoint off.
pub fn mcp_token() -> String {
    var("MCP_TOKEN")
}

pub fn shared_accounts_owner() -> String {
    let s = var("SHARED_ACCOUNTS_OWNER");
    if s.is_empty() { var("ADMIN_EMAIL") } else { s }
}

pub fn shared_accounts_url() -> String {
    var("SHARED_ACCOUNTS_URL")
}

/// The administrator of .env, lowercase, "" when none.
pub fn env_admin_email() -> String {
    var("ADMIN_EMAIL").trim().to_lowercase()
}

pub fn is_env_admin(email: Option<&str>) -> bool {
    let env = env_admin_email();
    !env.is_empty() && email.unwrap_or("").trim().to_lowercase() == env
}

/// EVERID_MODE=on: Caddy sends the pages through the EverID outpost and they sign in from X-Authentik-Email (everid.rs).
pub fn everid_on() -> bool {
    var("EVERID_MODE").trim() == "on"
}

/// The EverID group that may use this app, app-teams by default.
pub fn everid_group() -> String {
    or("EVERID_GROUP", "app-teams")
}

/// EVERID_PROXY_HOST: the only peer whose X-Authentik-* headers count, by name on the compose network (everid::screen).
pub fn everid_proxy_host() -> String {
    or("EVERID_PROXY_HOST", "caddy")
}

/// EVERID_ALIASES: `everid-email=local-email` pairs, comma separated, for users whose EverID address is not the one of
/// their account here.
pub fn everid_aliases() -> Vec<(String, String)> {
    var("EVERID_ALIASES")
        .split(',')
        .filter_map(|p| p.split_once('='))
        .map(|(a, b)| (a.trim().to_lowercase(), b.trim().to_lowercase()))
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .collect()
}

/// The password sign-in: always with EverID off, so that turning EverID off never locks anyone out; with EverID on
/// until PASSWORD_LOGIN=0.
pub fn password_login() -> bool {
    !everid_on() || var("PASSWORD_LOGIN").trim() != "0"
}

pub const ENV_PASSWORD_MESSAGE: &str = "The password of this administrator is set by ADMIN_PASSWORD in .env";

/// The exported pages and public files of the UI (`next build` with TR_EXPORT=1).
pub fn ui_dir() -> PathBuf {
    PathBuf::from(or("UI_DIR", "/ui"))
}

pub fn port() -> u16 {
    var("PORT").parse::<u16>().ok().filter(|p| *p > 0).unwrap_or(3000)
}

/// HOSTNAME as Next.js standalone reads it: the address to listen on.
pub fn bind_host() -> String {
    or("HOSTNAME", "0.0.0.0")
}

/// How the notifications of the accounts on another computer go out (src/shared/env.ts, PushEnv).
#[derive(Clone, Debug)]
pub struct Push {
    pub vapid_private_key_file: String,
    pub vapid_app_key_file: String,
    pub vapid_subject: String,
    pub fcm_credentials: String,
    pub ntfy: Option<(String, String)>,
}

/// PushEnv parsed: Err with zod's text of the first problems.
pub fn push() -> Result<Push, String> {
    push_of(&var)
}

/// PushEnv of an environment given as a lookup (a light slot brings its own); an empty value counts as unset.
pub fn push_of(get: &dyn Fn(&str) -> String) -> Result<Push, String> {
    let or = |name: &str, fallback: &str| {
        let v = get(name);
        if v.is_empty() { fallback.to_string() } else { v }
    };
    let var = |name: &str| get(name);
    let mut problems = vec![];
    let subject = or("VAPID_SUBJECT", "mailto:admin@example.com");
    if !(subject.starts_with("mailto:") || subject.starts_with("https://")) {
        problems.push("VAPID_SUBJECT: must be a mailto: or https:// URL".to_string());
    }
    let enabled = or("NTFY_ENABLED", "0");
    if enabled != "0" && enabled != "1" {
        problems.push("NTFY_ENABLED: Invalid option: expected one of \"0\"|\"1\"".to_string());
    }
    let ntfy_url = or("NTFY_URL", "https://ntfy.sh");
    if !url::Url::parse(&ntfy_url).is_ok_and(|u| u.scheme() == "http" || u.scheme() == "https") {
        problems.push("NTFY_URL: Invalid URL".to_string());
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let topic = var("NTFY_TOPIC");
    Ok(Push {
        vapid_private_key_file: or("VAPID_PRIVATE", "/vapid/private_key.pem"),
        vapid_app_key_file: or("VAPID_APPKEY", "/vapid/appkey.txt"),
        vapid_subject: subject,
        fcm_credentials: or("FCM_CREDENTIALS", "/fcm/service-account.json"),
        ntfy: (enabled == "1" && !topic.is_empty()).then_some((ntfy_url, topic)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_like_node() {
        // SAFETY: test-only variable, read by this test alone
        unsafe { std::env::set_var("TR_TEST_INT", "12abc") };
        assert_eq!(positive_int("TR_TEST_INT"), Some(12));
        unsafe { std::env::set_var("TR_TEST_INT", "0") };
        assert_eq!(positive_int("TR_TEST_INT"), None);
        unsafe { std::env::set_var("TR_TEST_INT", "x") };
        assert_eq!(positive_int("TR_TEST_INT"), None);
    }
}
