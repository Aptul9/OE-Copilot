//! Notifications of the accounts on another computer, sent by the web app (src/agent/push: notifier.ts, fcm.ts, seal.ts,
//! vapid.ts, src/agent/store/app-store.ts as the web app uses them): Web Push to browsers, sealed FCM data messages to
//! the phones of the Android app, ntfy when enabled, and the history of the notified messages.

pub(crate) mod app_store;
pub mod fcm;
pub mod http;
pub mod notifier;
pub mod notify;
pub mod seal;
pub mod vapid;
pub mod webpush;

pub use app_store::AppStore;
pub use fcm::{FcmSender, ServiceAccount, load_service_account};
pub use notifier::{Notifier, NotifierOptions};
pub use notify::account_label;
pub use vapid::{VapidKeys, load_vapid_keys};

use crate::state::Identity;

/// A device of the owner: the endpoint and the subscription JSON of push_subscriptions.
#[derive(Clone, Debug, PartialEq)]
pub struct PushTarget {
    pub endpoint: String,
    pub sub: String,
}

/// Where the notifications of an account go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Audience {
    pub mobile: bool,
    pub desktop: bool,
}

impl Audience {
    pub const ALL: Audience = Audience { mobile: true, desktop: true };

    fn has(&self, side: &str) -> bool {
        if side == "mobile" { self.mobile } else { self.desktop }
    }
}

/// A phone of the Android app, registered through /api/push/fcm: FCM, not Web Push.
pub fn is_phone(endpoint: &str) -> bool {
    endpoint.starts_with("fcm:")
}

/// The side a device is on: the phones of the Android app and the browsers that subscribed from a phone or tablet are
/// mobile, every other browser is desktop.
pub fn device_side(t: &PushTarget) -> &'static str {
    if is_phone(&t.endpoint) {
        return "mobile";
    }
    match serde_json::from_str::<serde_json::Value>(&t.sub) {
        Ok(v) if v.get("kind").and_then(serde_json::Value::as_str) == Some("mobile") => "mobile",
        _ => "desktop",
    }
}

/// The devices a push goes to (PushDevices).
pub trait PushDevices {
    fn targets(&self) -> Vec<PushTarget>;
    /// a subscription the push service reports as gone
    fn remove(&self, endpoint: &str);
    /// the slot of the account (acc, the app opens it) and its name after the title when the owner has more
    fn account(&self, _me: &Identity) -> Option<(i64, String)> {
        None
    }
    /// where the notifications of the account go; None, everywhere
    fn audience(&self) -> Option<Audience> {
        None
    }
}

/// Where the devices of a notifier come from: AppStore while the account still holds its slot.
pub struct Devices {
    pub store: AppStore,
    /// the account that took the slot at `added` still holds it
    pub holds: Box<dyn Fn() -> bool>,
}

impl PushDevices for Devices {
    /// A notification still on its way when the account is removed goes to nobody, even once the slot is someone
    /// else's.
    fn targets(&self) -> Vec<PushTarget> {
        if (self.holds)() { self.store.targets() } else { vec![] }
    }

    fn remove(&self, endpoint: &str) {
        self.store.remove(endpoint)
    }

    fn account(&self, me: &Identity) -> Option<(i64, String)> {
        Some(self.store.account(me))
    }

    fn audience(&self) -> Option<Audience> {
        Some(self.store.audience())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_of_a_device() {
        let web = |kind: &str| PushTarget {
            endpoint: "https://push.test/x".into(),
            sub: serde_json::json!({ "endpoint": "https://push.test/x", "kind": kind }).to_string(),
        };
        let phone = PushTarget { endpoint: "fcm:token-1".into(), sub: r#"{"fcm":{"token":"token-1"}}"#.into() };
        let old = PushTarget { endpoint: "https://push.test/old".into(), sub: r#"{"endpoint":"https://push.test/old"}"#.into() };
        let sides: Vec<_> = [phone, web("mobile"), web("desktop"), old].iter().map(device_side).collect();
        assert_eq!(sides, ["mobile", "mobile", "desktop", "desktop"]);
        assert_eq!(device_side(&PushTarget { endpoint: "https://x".into(), sub: "not json".into() }), "desktop");
    }
}
