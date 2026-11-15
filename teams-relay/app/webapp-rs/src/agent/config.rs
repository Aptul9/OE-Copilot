//! The environment of one light slot (src/agent/config.ts, the part a light agent reads), set by the supervisor. Checked
//! once at start: a wrong value stops the slot with the reason.

use super::{Fail, R};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Env = BTreeMap<String, String>;

/// A slot on a shared Microsoft account (src/supervisor/broker.ts): the calendar agent gives the tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct Broker {
    pub url: String,
    pub key: String,
    pub account: String,
    /// chat service region of the account (amer, emea, apac, ...): no browser cookie to read it from
    pub region: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub slot: i64,
    pub db_path: PathBuf,
    pub app_db: PathBuf,
    /// images of messages (data/N/media), downloaded attachments, images the web app queued to send
    pub media_dir: PathBuf,
    pub files_dir: PathBuf,
    pub uploads_dir: PathBuf,
    pub shared_broker: Option<Broker>,
    pub light_token_file: PathBuf,
    /// seconds between two reads of the chat list
    pub light_poll: i64,
    /// the status history of the status pages, next to app.db
    pub status_db: PathBuf,
    /// the environment this was read from: the calling worker starts on it
    pub env: Env,
    pub native_calls: bool,
    pub calling_host: String,
    pub call_audio_url: String,
    pub push: crate::config::Push,
    /// the DevTools of the slot's browser, for the browser agent (127.0.0.1: Chromium binds CDP on IPv4 only)
    pub cdp: String,
    /// the remote desktop of the browsers container, watched for the owner; None for a slot run by hand
    pub desktop: Option<DesktopWatch>,
    /// how to sign in again, and which browser does not start: the first sentence of those pushes
    pub alert_sign_in: String,
    pub alert_browser_down: String,
}

/// The websocket of the remote desktop (its viewers are connections to that port) and how to ask its compositor which
/// window is in front: as the desktop user, in its session.
#[derive(Clone, Debug, PartialEq)]
pub struct DesktopWatch {
    pub port: u16,
    pub uid: u32,
    pub gid: u32,
    pub app_id: String,
    pub session: Env,
}

/// An empty value counts as unset, so its default applies (parseEnv).
fn given<'a>(env: &'a Env, k: &str) -> Option<&'a str> {
    env.get(k).map(String::as_str).filter(|v| !v.is_empty())
}

/// z.coerce.number().int() with bounds: Number(v) of JavaScript.
fn int(env: &Env, k: &str, default: i64, min: i64, max: i64, problems: &mut Vec<String>) -> i64 {
    let Some(v) = given(env, k) else { return default };
    let n = crate::js::number(Some(&serde_json::Value::String(v.to_string())));
    if !n.is_finite() || n.fract() != 0.0 {
        problems.push(format!("{k}: Invalid input: expected int, received number"));
        return default;
    }
    if n < min as f64 {
        problems.push(format!("{k}: Too small: expected number to be >={min}"));
    } else if n > max as f64 {
        problems.push(format!("{k}: Too big: expected number to be <={max}"));
    }
    n as i64
}

fn http_url(v: &str) -> bool {
    url::Url::parse(v).is_ok_and(|u| u.scheme() == "http" || u.scheme() == "https")
}

pub fn load(env: &Env) -> R<Config> {
    let mut problems = vec![];
    let slot = int(env, "ACCOUNT", 1, 1, i64::MAX, &mut problems);
    let light_poll = int(env, "LIGHT_POLL", 60, 15, 3600, &mut problems);
    let url = given(env, "SHARED_BROKER_URL");
    if url.is_some_and(|u| !http_url(u)) {
        problems.push("SHARED_BROKER_URL: Invalid URL".into());
    }
    let region = given(env, "SHARED_BROKER_REGION");
    if region.is_some_and(|r| !regex::Regex::new("^[a-z][a-z0-9]{1,15}$").expect("fixed").is_match(r)) {
        problems.push("SHARED_BROKER_REGION: Invalid string: must match pattern /^[a-z][a-z0-9]{1,15}$/".into());
    }
    let cdp = given(env, "CDP").unwrap_or("http://localhost:9222").to_string();
    if !http_url(&cdp) {
        problems.push("CDP: Invalid URL".into());
    }
    let desktop_port = given(env, "DESKTOP_PORT").map(|_| int(env, "DESKTOP_PORT", 0, 1, 65535, &mut problems));
    let desktop_uid = given(env, "DESKTOP_UID").map(|_| int(env, "DESKTOP_UID", 0, 0, u32::MAX as i64, &mut problems));
    let desktop_gid = given(env, "DESKTOP_GID").map(|_| int(env, "DESKTOP_GID", 0, 0, u32::MAX as i64, &mut problems));
    let push = crate::config::push_of(&|k| given(env, k).unwrap_or("").to_string());
    if let Err(e) = &push {
        problems.push(e.clone());
    }
    if !problems.is_empty() {
        return Err(Fail::Other(problems.join("; ")));
    }
    let broker = [url, given(env, "SHARED_BROKER_KEY"), given(env, "SHARED_BROKER_ACCOUNT"), region];
    if broker.iter().any(Option::is_some) && !broker.iter().all(Option::is_some) {
        return Err(Fail::Other("shared broker requires URL, key, account and region".into()));
    }
    let db_path = PathBuf::from(given(env, "DB_PATH").unwrap_or("/data/1/messages.db"));
    let app_db = PathBuf::from(given(env, "APP_DB").unwrap_or("/data/app.db"));
    let data_dir = db_path.parent().map(PathBuf::from).unwrap_or_default();
    let domain = given(env, "DOMAIN").unwrap_or("undefined");
    Ok(Config {
        slot,
        shared_broker: url.map(|u| Broker {
            url: u.to_string(),
            key: broker[1].unwrap_or("").to_string(),
            account: broker[2].unwrap_or("").to_string(),
            region: broker[3].unwrap_or("").to_string(),
        }),
        media_dir: data_dir.join("media"),
        files_dir: data_dir.join("files"),
        uploads_dir: data_dir.join("uploads"),
        light_token_file: data_dir.join("light-token.json"),
        light_poll,
        status_db: app_db.parent().map(PathBuf::from).unwrap_or_default().join("status.db"),
        env: env.clone(),
        native_calls: given(env, "NATIVE_CALLS") == Some("1"),
        // the calling worker in Rust (teamsrelay-calling) unless CALLS_RUST is 0: the Node host of Teams web's calling stack
        calling_host: given(env, "CALLING_HOST")
            .unwrap_or(if given(env, "CALLS_RUST") == Some("0") { "/app/calling-live-host.mjs" } else { "/app/teamsrelay-calling" })
            .to_string(),
        call_audio_url: given(env, "CALL_AUDIO_URL").map(String::from).unwrap_or_else(|| format!("wss://{domain}/api/call/audio/socket?a={slot}")),
        alert_sign_in: match given(env, "SHARED_BROKER_ACCOUNT") {
            Some(a) => format!("Sign in again to {a} in the Account window of the calendar, then check its Chat Teams capability"),
            None => "Open the remote desktop of the account and sign in again".into(),
        },
        alert_browser_down: format!("The browser of account {slot} does not start"),
        desktop: match (desktop_port, desktop_uid, desktop_gid, given(env, "DESKTOP_APP_ID"), given(env, "XDG_RUNTIME_DIR"), given(env, "WAYLAND_DISPLAY")) {
            (Some(port), Some(uid), Some(gid), Some(app), Some(xdg), Some(wayland)) => Some(DesktopWatch {
                port: port as u16,
                uid: uid as u32,
                gid: gid as u32,
                app_id: app.into(),
                session: [("XDG_RUNTIME_DIR".to_string(), xdg.to_string()), ("WAYLAND_DISPLAY".to_string(), wayland.to_string())].into_iter().collect(),
            }),
            _ => None,
        },
        cdp,
        db_path,
        app_db,
        push: push.map_err(Fail::Other)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_partial_broker_configuration_fails_closed() {
        let e = load(&env(&[("SHARED_BROKER_URL", "http://agent")])).unwrap_err();
        assert!(e.to_string().contains("requires"), "{e}");
        let e = load(&env(&[("SHARED_BROKER_URL", "http://agent"), ("SHARED_BROKER_KEY", "k"), ("SHARED_BROKER_ACCOUNT", "a@b.example")])).unwrap_err();
        assert!(e.to_string().contains("region"), "{e}");
        assert!(
            load(&env(&[
                ("SHARED_BROKER_URL", "http://agent"),
                ("SHARED_BROKER_KEY", "k"),
                ("SHARED_BROKER_ACCOUNT", "a@b.example"),
                ("SHARED_BROKER_REGION", "AMER/../x")
            ]))
            .is_err()
        );
        let c = load(&env(&[
            ("SHARED_BROKER_URL", "http://agent"),
            ("SHARED_BROKER_KEY", "k"),
            ("SHARED_BROKER_ACCOUNT", "a@b.example"),
            ("SHARED_BROKER_REGION", "amer"),
        ]))
        .expect("config");
        assert_eq!(c.shared_broker, Some(Broker { url: "http://agent".into(), key: "k".into(), account: "a@b.example".into(), region: "amer".into() }));
    }

    #[test]
    fn the_files_of_the_slot_and_its_defaults() {
        let c = load(&env(&[
            ("ACCOUNT", "4"),
            ("DB_PATH", "/root/data/4/messages.db"),
            ("APP_DB", "/root/data/app.db"),
            ("DOMAIN", "relay.example"),
            ("LIGHT_POLL", ""),
        ]))
        .expect("config");
        assert_eq!((c.slot, c.light_poll, c.native_calls), (4, 60, false));
        assert_eq!(c.light_token_file, PathBuf::from("/root/data/4/light-token.json"));
        assert_eq!(c.status_db, PathBuf::from("/root/data/status.db"));
        assert_eq!(c.call_audio_url, "wss://relay.example/api/call/audio/socket?a=4");
        assert_eq!(c.calling_host, "/app/teamsrelay-calling");
        assert_eq!(load(&env(&[("CALLS_RUST", "0")])).expect("config").calling_host, "/app/calling-live-host.mjs");
        assert_eq!(load(&env(&[("CALLS_RUST", "0"), ("CALLING_HOST", "/opt/h.mjs")])).expect("config").calling_host, "/opt/h.mjs");
        assert!(load(&env(&[("LIGHT_POLL", "5")])).is_err());
        assert!(load(&env(&[("ACCOUNT", "x")])).is_err());
    }
}
