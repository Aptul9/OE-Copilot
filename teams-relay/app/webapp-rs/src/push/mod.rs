//! Notifications of the accounts on another computer, sent by the web app (src/agent/push: notifier.ts, fcm.ts, seal.ts,
//! vapid.ts, src/agent/store/app-store.ts as the web app uses them): Web Push to browsers, sealed FCM data messages to
//! the phones of the Android app, ntfy when enabled, and the history of the notified messages.
//!
//! STUB: the interface the relay uses. Implemented in full by the push port.

use crate::state::Identity;

/// The Web Push keys of vapid/: raw private scalar and uncompressed public point, both base64url.
#[derive(Clone, Debug)]
pub struct VapidKeys {
    pub public_key: String,
    pub private_key: String,
}

/// None without a private key; Err when the key is not P-256 or does not match appkey.txt.
pub fn load_vapid_keys(_private_key_file: &str, _app_key_file: &str) -> Result<Option<VapidKeys>, String> {
    Ok(None)
}

/// The service account key of the Firebase project.
#[derive(Clone, Debug)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
}

/// None when the file is missing; Err when it is not a service account key file.
pub fn load_service_account(_file: &str) -> Result<Option<ServiceAccount>, String> {
    Ok(None)
}

pub struct FcmSender {}

impl FcmSender {
    pub fn new(_sa: ServiceAccount) -> Self {
        FcmSender {}
    }
}

/// The devices of the owner of a slot in app.db, as the agents read them (AppStore).
pub struct AppStore {
    pub slot: i64,
}

impl AppStore {
    pub fn new(slot: i64) -> Self {
        AppStore { slot }
    }
}

/// Where the devices of a notifier come from: AppStore while the account still holds its slot.
pub struct Devices {
    pub store: AppStore,
    /// the account that took the slot at `added` still holds it
    pub holds: Box<dyn Fn() -> bool>,
}

pub struct NotifierOptions {
    /// data/N/messages.db of the slot (history of the notified messages, known chats, identity)
    pub store_file: std::path::PathBuf,
    pub devices: Devices,
    pub vapid: Option<VapidKeys>,
    pub subject: String,
    pub ntfy: Option<(String, String)>,
    pub fcm: Option<FcmSender>,
    pub answerable: bool,
}

pub struct Notifier {
    _o: NotifierOptions,
}

impl Notifier {
    pub fn new(o: NotifierOptions) -> Self {
        Notifier { _o: o }
    }
    pub async fn message(&self, _title: &str, _body: &str, _chat: &str) {}
    pub async fn alert(&self, _title: &str, _body: &str, _urgency: &str) -> usize {
        0
    }
    pub async fn notice(&self, _title: &str, _body: &str, _urgency: &str) -> usize {
        0
    }
    pub async fn call(&self, _caller: &str, _state: &str, _since: i64, _seconds: i64, _answered: bool) -> usize {
        0
    }
    pub async fn missed_call(&self, _caller: &str, _time: &str) -> usize {
        0
    }
    pub fn device_count(&self) -> usize {
        0
    }
}

/// accountLabel: the organization, otherwise the email, when the owner has more accounts.
pub fn account_label(owner_has_many: bool, me: &Identity, slot: i64) -> String {
    if !owner_has_many {
        return String::new();
    }
    if !me.tenant.is_empty() {
        me.tenant.clone()
    } else if !me.email.is_empty() {
        me.email.clone()
    } else {
        format!("account {slot}")
    }
}
