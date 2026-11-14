//! The environment of the supervisor (src/supervisor/config.ts): the container environment (s6 with-contenv), set by the
//! image and by docker-compose.yml. Checked once at start; an empty value counts as unset, so its default applies.

use super::broker::SharedBroker;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Env = BTreeMap<String, String>;

/// What the accounts run with (AccountsConfig).
#[derive(Clone, Debug)]
pub struct AccountsConfig {
    /// the OE calendar agent of the shared Microsoft accounts: a slot the web app binds to one runs only its light
    /// agent, on resource tokens of that account
    pub shared_broker: Option<SharedBroker>,
    pub profiles_dir: PathBuf,
    pub data_dir: PathBuf,
    pub vapid_dir: PathBuf,
    /// service account key of the Firebase project, for the Android app
    pub fcm_dir: PathBuf,
    /// owner of the profiles and of the desktop session (PUID/PGID of the image)
    pub uid: u32,
    pub gid: u32,
    pub chromium: String,
    pub agent_script: String,
    /// the agent of a light account, which runs without a browser (src/agent/light)
    pub light_script: String,
    pub node: String,
    /// DevTools of account N listen on 127.0.0.1:(cdp_base_port + N)
    pub cdp_base_port: u16,
    /// Wayland session of the desktop, where the browser windows open
    pub session: Env,
    /// port of the websocket of the remote desktop, whose viewers the agents count
    pub desktop_port: u16,
    pub browser_env: Env,
    pub agent_env: Env,
    pub wlrctl: String,
    /// every light slot in one process; false: one process per slot
    pub light_host: bool,
    /// the host in Rust (agent/host.rs) instead of node light_script --host
    pub light_bin: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DesktopSettings {
    pub on_demand: bool,
    pub socket: PathBuf,
    /// None where there is no s6 (tests, a supervisor started by hand): the desktop is taken as running
    pub s6rc: Option<String>,
    pub microphone: Option<String>,
    pub idle_ms: f64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub socket: String,
    pub accounts: AccountsConfig,
    pub desktop: DesktopSettings,
}

/// Passed on as they are, when set. PULSE_RUNTIME_PATH: where the image runs PulseAudio, the sound of the desktop and the
/// microphone Selkies forwards.
const BROWSER_ENV: [&str; 5] = ["TZ", "LANG", "LANGUAGE", "LC_ALL", "PULSE_RUNTIME_PATH"];
const AGENT_ENV: [&str; 8] = ["TZ", "VAPID_SUBJECT", "NTFY_ENABLED", "NTFY_URL", "NTFY_TOPIC", "LIGHT_POLL", "NATIVE_CALLS", "DOMAIN"];

/// The variables as given; None for an unset or empty one.
pub fn load(env: &dyn Fn(&str) -> Option<String>) -> Result<Config, String> {
    let given = |k: &str| env(k).filter(|v| !v.is_empty());
    let or = |k: &str, d: &str| given(k).unwrap_or_else(|| d.to_string());
    let mut problems = vec![];
    let mut flag = |k: &str, d: &str| {
        let v = or(k, d);
        if v != "0" && v != "1" {
            problems.push(format!("{k}: Invalid option: expected one of \"0\"|\"1\""));
        }
        v == "1"
    };
    let light_host = flag("LIGHT_HOST", "1");
    let light_rust = flag("LIGHT_RUST", "1");
    let on_demand = flag("DESKTOP_ON_DEMAND", "1");
    let mut int = |k: &str, d: i64, min: i64, max: i64| {
        let Some(v) = given(k) else { return d };
        let n = crate::js::number(Some(&serde_json::Value::String(v)));
        if !n.is_finite() || n.fract() != 0.0 {
            problems.push(format!("{k}: Invalid input: expected int, received number"));
            d
        } else if n < min as f64 {
            problems.push(format!("{k}: Too small: expected number to be >={min}"));
            d
        } else if n > max as f64 {
            problems.push(format!("{k}: Too big: expected number to be <={max}"));
            d
        } else {
            n as i64
        }
    };
    let uid = int("PUID", 1000, 1, u32::MAX as i64) as u32;
    let gid = int("PGID", 1000, 1, u32::MAX as i64) as u32;
    let idle_min = int("DESKTOP_IDLE_MIN", 10, 1, 1440);
    let url = given("SHARED_BROKER_URL");
    if url.as_deref().is_some_and(|u| !url::Url::parse(u).is_ok_and(|p| p.scheme() == "http" || p.scheme() == "https")) {
        problems.push("SHARED_BROKER_URL: Invalid URL".into());
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let key = given("SHARED_BROKER_KEY");
    if url.is_some() != key.is_some() {
        return Err("SHARED_BROKER_URL and SHARED_BROKER_KEY go together".into());
    }
    let pick = |names: &[&str]| names.iter().filter_map(|k| env(k).filter(|v| !v.is_empty()).map(|v| (k.to_string(), v))).collect::<Env>();
    let xdg = or("XDG_RUNTIME_DIR", "/config/.XDG");
    let session: Env = [("XDG_RUNTIME_DIR", xdg.clone()), ("WAYLAND_DISPLAY", "wayland-0".into()), ("DISPLAY", ":0".into())]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    Ok(Config {
        socket: or("CONTROL_SOCKET", "/root/run/control.sock"),
        accounts: AccountsConfig {
            shared_broker: url.zip(key).map(|(url, key)| SharedBroker { url, key }),
            profiles_dir: or("PROFILES_DIR", "/profiles").into(),
            data_dir: or("DATA_DIR", "/root/data").into(),
            vapid_dir: or("VAPID_DIR", "/root/vapid").into(),
            fcm_dir: or("FCM_DIR", "/root/fcm").into(),
            uid,
            gid,
            chromium: or("CHROMIUM", "/usr/bin/chromium"),
            agent_script: or("AGENT_SCRIPT", "/app/agent.cjs"),
            light_script: or("LIGHT_SCRIPT", "/app/light.cjs"),
            // the Node of the image, which runs the agents (process.execPath of the Node supervisor)
            node: or("NODE_BIN", "/usr/local/bin/node"),
            cdp_base_port: 9221,
            // Selkies of the image listens there, behind its nginx
            desktop_port: 8082,
            // the labwc session of the image: its clients connect to wayland-0
            session,
            browser_env: pick(&BROWSER_ENV),
            agent_env: pick(&AGENT_ENV),
            wlrctl: "wlrctl".into(),
            light_host,
            light_bin: light_rust.then(|| or("LIGHT_BIN", "/app/teamsrelay-light")),
        },
        desktop: DesktopSettings {
            on_demand,
            socket: PathBuf::from(&xdg).join("wayland-0"),
            // an empty S6_RC means none, not the default
            s6rc: match env("S6_RC") {
                Some(v) if v.is_empty() => None,
                Some(v) => Some(v),
                None => Some("/command/s6-rc".into()),
            },
            microphone: (given("SELKIES_MICROPHONE_ENABLED").as_deref() == Some("true")).then(|| "/usr/local/bin/desktop-microphone".to_string()),
            idle_ms: idle_min as f64 * 60_000.0,
        },
    })
}

/// The environment of this process.
pub fn from_process() -> Result<Config, String> {
    load(&|k| std::env::var(k).ok())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn of(pairs: &[(&str, &str)]) -> Result<Config, String> {
        let m: Env = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        load(&move |k| m.get(k).cloned())
    }

    #[test]
    fn the_paths_of_the_browsers_image_by_default() {
        let c = of(&[]).expect("config");
        let a = &c.accounts;
        assert_eq!(c.socket, "/root/run/control.sock");
        assert_eq!(
            (a.profiles_dir.to_str(), a.data_dir.to_str(), a.vapid_dir.to_str(), a.fcm_dir.to_str()),
            (Some("/profiles"), Some("/root/data"), Some("/root/vapid"), Some("/root/fcm"))
        );
        assert_eq!(
            (a.uid, a.gid, a.chromium.as_str(), a.agent_script.as_str(), a.light_script.as_str()),
            (1000, 1000, "/usr/bin/chromium", "/app/agent.cjs", "/app/light.cjs")
        );
        assert_eq!((a.cdp_base_port, a.desktop_port, a.node.as_str()), (9221, 8082, "/usr/local/bin/node"));
        assert_eq!(a.session["XDG_RUNTIME_DIR"], "/config/.XDG");
        assert!(a.shared_broker.is_none());
    }

    #[test]
    fn switches_of_the_light_host() {
        assert!(of(&[]).expect("c").accounts.light_host);
        assert!(!of(&[("LIGHT_HOST", "0")]).expect("c").accounts.light_host);
        assert!(of(&[("LIGHT_HOST", "yes")]).unwrap_err().contains("LIGHT_HOST"));
        assert_eq!(of(&[]).expect("c").accounts.light_bin.as_deref(), Some("/app/teamsrelay-light"));
        assert_eq!(of(&[("LIGHT_BIN", "/opt/light")]).expect("c").accounts.light_bin.as_deref(), Some("/opt/light"));
        assert_eq!(of(&[("LIGHT_RUST", "0")]).expect("c").accounts.light_bin, None);
        assert!(of(&[("LIGHT_RUST", "yes")]).unwrap_err().contains("LIGHT_RUST"));
    }

    #[test]
    fn the_desktop_and_the_broker() {
        let d = of(&[]).expect("c").desktop;
        assert_eq!(
            (d.on_demand, d.socket.to_str(), d.s6rc.as_deref(), d.microphone.as_deref(), d.idle_ms),
            (true, Some("/config/.XDG/wayland-0"), Some("/command/s6-rc"), None, 600_000.0)
        );
        let d = of(&[("S6_RC", ""), ("DESKTOP_ON_DEMAND", "0"), ("DESKTOP_IDLE_MIN", "3"), ("SELKIES_MICROPHONE_ENABLED", "true")]).expect("c").desktop;
        assert_eq!((d.on_demand, d.s6rc, d.idle_ms, d.microphone.as_deref()), (false, None, 180_000.0, Some("/usr/local/bin/desktop-microphone")));
        assert!(of(&[("DESKTOP_IDLE_MIN", "0")]).is_err());
        let b = of(&[("SHARED_BROKER_URL", "http://oe-copilot-agent:8180"), ("SHARED_BROKER_KEY", "k")]).expect("c").accounts.shared_broker.expect("broker");
        assert_eq!((b.url.as_str(), b.key.as_str()), ("http://oe-copilot-agent:8180", "k"));
        assert!(of(&[("SHARED_BROKER_URL", "http://x")]).is_err());
        assert!(of(&[("SHARED_BROKER_KEY", "k")]).is_err());
        assert!(of(&[("SHARED_BROKER_URL", "ftp://x"), ("SHARED_BROKER_KEY", "k")]).is_err());
        let a = of(&[("TZ", "Europe/Rome"), ("LANG", "en_US.UTF-8"), ("VAPID_SUBJECT", "mailto:me@example.com"), ("NTFY_ENABLED", "")]).expect("c").accounts;
        assert_eq!(a.browser_env.keys().collect::<Vec<_>>(), ["LANG", "TZ"]);
        assert_eq!(a.agent_env.keys().collect::<Vec<_>>(), ["TZ", "VAPID_SUBJECT"]);
    }
}
