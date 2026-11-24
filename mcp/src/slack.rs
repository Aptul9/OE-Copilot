//! Slack over its Web API, straight from the slackcli workspace store (workspaces.json, written by the OE autologin and
//! read by Everflow too). Replaces the slackcli 0.19.1 subprocess: same calls, same JSON as its `--json` output, so the
//! tool answers do not change. Browser sessions (xoxc token + d cookie) go only to their https *.slack.com origin.

use crate::js::{Obj, base64_std, set, truthy, truthy_opt};
use crate::{Error, Result, http_client};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

const MAX_BYTES: usize = 5_000_000;
const FAILED: &str = "Slack request failed; write outcome may be unknown";
const CAPABILITIES: [&str; 8] = ["chat.list", "chat.read", "chat.search", "chat.send", "chat.edit", "chat.delete", "chat.react", "chat.attachment"];

/// Methods safe to repeat after an ambiguous failure (5xx, network error); anything else is retried on a 429 only.
const READ_METHODS: [&str; 11] = [
    "auth.test",
    "chat.getPermalink",
    "conversations.history",
    "conversations.list",
    "conversations.replies",
    "files.info",
    "messages.list",
    "search.messages",
    "users.conversations",
    "users.info",
    "conversations.info",
];

/// A conversation id as Slack writes it: C channel, G private channel or group DM, D direct message. On a Slack account
/// it stands for a conversation handle: the workspace session decides what it opens, as it does for slackcli.
pub fn native_conversation(id: &str) -> bool {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[CDG][A-Z0-9]{8,}$").expect("fixed"));
    ID.is_match(id)
}

fn failed() -> Error {
    Error::comm("PROVIDER_ERROR", FAILED)
}

fn invalid_resource(m: &str) -> Error {
    Error::comm("INVALID_RESOURCE", m)
}

/// Every Slack call goes through one limiter: two at a time, 200 ms between starts, as slackcli paced them (a burst
/// looks like scraping to Slack's anomaly detection and can sign the session out).
struct Limiter {
    slots: Semaphore,
    last: Mutex<Option<Instant>>,
}

static LIMITER: LazyLock<Limiter> = LazyLock::new(|| Limiter { slots: Semaphore::new(2), last: Mutex::new(None) });

#[derive(Clone)]
enum Auth {
    Browser { origin: String, xoxc: String, xoxd: String },
    Standard { token: String },
}

#[derive(Clone)]
struct Workspace {
    user_id: Option<String>,
    auth: Auth,
}

/// The canonical origin of a browser workspace URL: https, a slack.com host other than app.slack.com, nothing else.
fn workspace_origin(candidate: &str) -> Option<String> {
    let u = url::Url::parse(candidate).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.port().is_some() {
        return None;
    }
    if u.path() != "/" || u.query().is_some() || u.fragment().is_some() {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    if host == "app.slack.com" || (host != "slack.com" && !host.ends_with(".slack.com")) {
        return None;
    }
    Some(u.origin().ascii_serialization())
}

fn api_base(auth: &Auth) -> String {
    #[cfg(feature = "test-hosts")]
    if let Ok(v) = std::env::var("SLACK_TEST_BASE") {
        return v;
    }
    match auth {
        Auth::Browser { origin, .. } => origin.clone(),
        Auth::Standard { .. } => "https://slack.com".into(),
    }
}

fn is_slack_host(u: &url::Url) -> bool {
    #[cfg(feature = "test-hosts")]
    if std::env::var("SLACK_TEST_BASE").is_ok() {
        return true;
    }
    u.host_str().is_some_and(|h| h.to_lowercase().ends_with(".slack.com"))
}

/// `compareTs`: seconds as integers, fractions digit by digit, never as floats.
fn compare_ts(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |t: &str| {
        let (s, f) = t.split_once('.').unwrap_or((t, ""));
        (s.trim_start_matches('0').to_string(), f.to_string())
    };
    let (sa, fa) = split(a);
    let (sb, fb) = split(b);
    sa.len().cmp(&sb.len()).then(sa.cmp(&sb)).then_with(|| {
        let w = fa.len().max(fb.len()).max(6);
        format!("{fa:0<w$}").cmp(&format!("{fb:0<w$}"))
    })
}

/// The JSON fields slackcli copies, in its order; missing ones stay out, as undefined does in JSON.stringify.
fn pick(src: &Value, keys: &[&str]) -> Value {
    let mut o = Obj::new();
    for k in keys {
        set(&mut o, k, src.get(*k).cloned());
    }
    Value::Object(o)
}

fn message_json(msg: &Value) -> Value {
    let mut o = match pick(msg, &["ts", "thread_ts", "user", "text", "type", "reply_count", "reactions", "bot_id", "blocks", "attachments"]) {
        Value::Object(o) => o,
        _ => Obj::new(),
    };
    if let Some(files) = msg.get("files").and_then(Value::as_array).filter(|f| !f.is_empty()) {
        let list = files.iter().map(|f| pick(f, &["id", "name", "title", "mimetype", "filetype", "size", "url_private", "permalink", "mode"])).collect();
        o.insert("files".into(), Value::Array(list));
    }
    Value::Object(o)
}

fn user_json(u: &Value) -> Value {
    let mut o = match pick(u, &["id", "name", "real_name"]) {
        Value::Object(o) => o,
        _ => Obj::new(),
    };
    set(&mut o, "email", u.get("profile").and_then(|p| p.get("email")).cloned());
    Value::Object(o)
}

/// What a call needs; the fields of the tool call, natives already taken out of their handles.
#[derive(Default)]
pub struct Args {
    pub channel: Option<String>,
    pub ts: Option<String>,
    pub file_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: i64,
    pub query: Option<String>,
    pub text: Option<String>,
    pub emoji: Option<String>,
    pub remove: bool,
    pub files: Vec<(String, String)>,
}

pub struct Slack {
    path: String,
}

/// A workspace session read from the store, for `Slack::raw`.
pub struct Session(Workspace);

pub enum Lookup {
    Missing,
    Unusable,
    Ready(Session),
}

enum Fail {
    /// Slack answered ok:false with this error code
    Slack(String),
    Other,
}

impl Slack {
    pub fn new(path: &str) -> Self {
        Slack { path: path.to_string() }
    }

    fn store(&self) -> Value {
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!({"workspaces": {}}))
    }

    pub fn accounts(&self) -> Vec<Value> {
        let store = self.store();
        let Some(ws) = store.get("workspaces").and_then(Value::as_object) else { return vec![] };
        ws.iter()
            .map(|(key, c)| {
                let mut o = Obj::new();
                o.insert("account_id".into(), json!(format!("slack:{key}")));
                o.insert("provider".into(), json!("slack"));
                o.insert("native".into(), json!(key));
                set(&mut o, "name", c.get("workspace_name").cloned());
                o.insert("capabilities".into(), json!(CAPABILITIES));
                Value::Object(o)
            })
            .collect()
    }

    /// Exact profile key, then the profile field, the workspace id, the workspace name: each only when unambiguous.
    fn workspace(&self, profile: &str) -> Result<Workspace> {
        Self::session(&self.config(profile).ok_or_else(failed)?)
    }

    /// The store entry of a workspace, found as `workspace` says.
    fn config(&self, profile: &str) -> Option<Value> {
        let store = self.store();
        let ws = store.get("workspaces").and_then(Value::as_object).cloned().unwrap_or_default();
        ws.get(profile).cloned().or_else(|| {
            for field in ["profile", "workspace_id", "workspace_name"] {
                let hits: Vec<&Value> = ws.values().filter(|c| c.get(field).and_then(Value::as_str) == Some(profile)).collect();
                match hits.len() {
                    1 => return Some(hits[0].clone()),
                    0 => continue,
                    _ => return None,
                }
            }
            None
        })
    }

    /// The session of a store entry: browser (https *.slack.com origin, xoxc token, d cookie) or standard token.
    fn session(c: &Value) -> Result<Workspace> {
        let s = |k: &str| c.get(k).and_then(Value::as_str).map(String::from);
        let auth = match s("auth_type").as_deref() {
            Some("browser") => Auth::Browser {
                origin: workspace_origin(&s("workspace_url").unwrap_or_default()).ok_or_else(failed)?,
                xoxc: s("xoxc_token").ok_or_else(failed)?,
                xoxd: s("xoxd_token").ok_or_else(failed)?,
            },
            Some("standard") => Auth::Standard { token: s("token").ok_or_else(failed)? },
            _ => return Err(failed()),
        };
        Ok(Workspace { user_id: s("user_id"), auth })
    }

    /// A workspace for Everflow (everflow.rs), read from the store once per call: absent, present with a session that
    /// cannot be used, or ready.
    pub fn lookup(&self, workspace: &str) -> Lookup {
        match self.config(workspace) {
            None => Lookup::Missing,
            Some(c) => Self::session(&c).map_or(Lookup::Unusable, |w| Lookup::Ready(Session(w))),
        }
    }

    /// One call for Everflow (everflow.rs): the same session and headers as `attempt`, through the same limiter, no
    /// retry; the status and the body as text, at most `max_bytes`. The caller checks the method and the params. Not
    /// reaching Slack is UNREACHABLE.
    pub async fn raw(&self, session: &Session, method: &str, params: &[(String, String)], max_bytes: usize) -> Result<(u16, String)> {
        let w = &session.0;
        let _slot = LIMITER.slots.acquire().await.expect("limiter");
        {
            let mut last = LIMITER.last.lock().await;
            if let Some(t) = *last {
                tokio::time::sleep(Duration::from_millis(200).saturating_sub(t.elapsed())).await;
            }
            *last = Some(Instant::now());
        }
        let mut form = form_urlencoded::Serializer::new(String::new());
        if let Auth::Browser { xoxc, .. } = &w.auth {
            form.append_pair("token", xoxc);
        }
        for (k, v) in params {
            form.append_pair(k, v);
        }
        let mut req = http_client().post(format!("{}/api/{method}", api_base(&w.auth))).timeout(Duration::from_secs(30));
        match &w.auth {
            Auth::Browser { xoxd, .. } => {
                req = req
                    .header("Cookie", format!("d={}", crate::js::encode_uri_component(xoxd)))
                    .header("Origin", "https://app.slack.com")
                    .header("User-Agent", "Mozilla/5.0 (compatible; SlackCLI/0.1.0)");
            }
            Auth::Standard { token } => req = req.header("Authorization", format!("Bearer {token}")),
        }
        let r = req
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form.finish())
            .send()
            .await
            .map_err(|_| Error::comm("UNREACHABLE", "Slack unreachable"))?;
        let status = r.status().as_u16();
        Ok((status, crate::text_body(r, max_bytes).await?))
    }

    async fn attempt(&self, w: &Workspace, method: &str, params: &[(&str, String)]) -> std::result::Result<Value, (Option<u16>, Option<u64>, bool, Fail)> {
        let _slot = LIMITER.slots.acquire().await.expect("limiter");
        {
            let mut last = LIMITER.last.lock().await;
            if let Some(t) = *last {
                let wait = Duration::from_millis(200).saturating_sub(t.elapsed());
                tokio::time::sleep(wait).await;
            }
            *last = Some(Instant::now());
        }
        let url = format!("{}/api/{method}", api_base(&w.auth));
        let body = {
            let mut form = form_urlencoded::Serializer::new(String::new());
            if let Auth::Browser { xoxc, .. } = &w.auth {
                form.append_pair("token", xoxc);
            }
            for (k, v) in params {
                form.append_pair(k, v);
            }
            form.finish()
        };
        let mut req = http_client().post(&url).timeout(Duration::from_secs(45));
        match &w.auth {
            Auth::Browser { xoxd, .. } => {
                req = req
                    .header("Cookie", format!("d={}", crate::js::encode_uri_component(xoxd)))
                    .header("Origin", "https://app.slack.com")
                    .header("User-Agent", "Mozilla/5.0 (compatible; SlackCLI/0.1.0)");
            }
            Auth::Standard { token } => req = req.header("Authorization", format!("Bearer {token}")),
        }
        let r = req.header("Content-Type", "application/x-www-form-urlencoded").body(body).send().await;
        let r = match r {
            Ok(r) => r,
            Err(_) => return Err((None, None, true, Fail::Other)),
        };
        let status = r.status().as_u16();
        if !(200..300).contains(&status) {
            let retry_after = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
            return Err((Some(status), retry_after, false, Fail::Other));
        }
        let data: Value = crate::json_body(r).await.ok_or((None, None, false, Fail::Other))?;
        if data.get("ok") != Some(&Value::Bool(true)) {
            let code = data.get("error").and_then(Value::as_str).unwrap_or("").to_string();
            return Err((None, None, false, Fail::Slack(code)));
        }
        Ok(data)
    }

    /// One Slack call: a 429 is retried for any method, a 5xx or network failure only for reads, at most three
    /// retries within two minutes.
    async fn request(&self, w: &Workspace, method: &str, params: &[(&str, String)]) -> std::result::Result<Value, Fail> {
        let mut waited = 0u64;
        for attempt in 1.. {
            match self.attempt(w, method, params).await {
                Ok(v) => return Ok(v),
                Err((status, retry_after, network, fail)) => {
                    let retryable = status == Some(429) || ((network || status.is_some_and(|s| s >= 500)) && READ_METHODS.contains(&method));
                    if !retryable || attempt > 3 {
                        return Err(fail);
                    }
                    let backoff = 1000u64 * (1 << (attempt - 1));
                    let wait = match (status, retry_after) {
                        (Some(429), Some(s)) if s > 0 => s * 1000,
                        _ => backoff * 3 / 4,
                    }
                    .min(60_000);
                    if waited + wait > 120_000 {
                        return Err(fail);
                    }
                    tokio::time::sleep(Duration::from_millis(wait)).await;
                    waited += wait;
                }
            }
        }
        Err(Fail::Other)
    }

    async fn ok(&self, w: &Workspace, method: &str, params: &[(&str, String)]) -> Result<Value> {
        self.request(w, method, params).await.map_err(|_| failed())
    }

    /// users.info for each id, in order; one that fails is left out.
    async fn users(&self, w: &Workspace, ids: Vec<String>) -> Vec<Value> {
        let mut seen = std::collections::HashSet::new();
        let ids: Vec<String> = ids.into_iter().filter(|i| seen.insert(i.clone())).collect();
        let answers = futures_util::future::join_all(ids.iter().map(|id| async move { self.request(w, "users.info", &[("user", id.clone())]).await })).await;
        let mut out: Vec<Value> = vec![];
        for a in answers.into_iter().flatten() {
            if let Some(u) = a.get("user").filter(|u| truthy(u)) {
                let id = u.get("id").cloned();
                if let Some(pos) = out.iter().position(|x| x.get("id").cloned() == id) {
                    out[pos] = u.clone();
                } else {
                    out.push(u.clone());
                }
            }
        }
        out.iter().map(user_json).collect()
    }

    /// A user id is a direct message still to open.
    async fn open(&self, w: &Workspace, recipient: &str) -> Result<String> {
        if !recipient.starts_with('U') {
            return Ok(recipient.to_string());
        }
        let r = self.ok(w, "conversations.open", &[("users", recipient.to_string())]).await?;
        r.pointer("/channel/id").and_then(Value::as_str).map(String::from).ok_or_else(failed)
    }

    pub async fn call(&self, profile: &str, action: &str, a: &Args) -> Result<Value> {
        static PROFILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,99}$").expect("fixed"));
        static CHANNEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[CDGU][A-Z0-9]+$").expect("fixed"));
        static TS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]+[.][0-9]+$").expect("fixed"));
        static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^F[A-Z0-9]+$").expect("fixed"));
        static B64: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+/]*={0,2}$").expect("fixed"));
        if !PROFILE.is_match(profile) {
            return Err(Error::comm("INVALID_ACCOUNT", "Invalid workspace profile"));
        }
        let channel = || a.channel.clone().filter(|c| CHANNEL.is_match(c)).ok_or_else(|| invalid_resource("Invalid Slack conversation"));
        let ts = || a.ts.clone().filter(|t| TS.is_match(t)).ok_or_else(|| invalid_resource("Invalid Slack timestamp"));
        if action == "send" && !a.files.is_empty() {
            let ch = channel()?;
            let thread = a.ts.as_ref().map(|_| ts()).transpose()?;
            let mut files = vec![];
            let mut total = 0;
            for (name, data) in &a.files {
                if !B64.is_match(data) {
                    return Err(Error::comm("INVALID_INPUT", "Invalid base64 file"));
                }
                let bytes = crate::js::base64_decode_lenient(data);
                total += bytes.len();
                if total > MAX_BYTES {
                    return Err(Error::comm("LIMIT_EXCEEDED", "Files exceed 5 MB"));
                }
                let clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(100).collect();
                files.push((clean, bytes));
            }
            let w = self.workspace(profile)?;
            return self.upload(&w, &ch, files, a.text.as_deref().unwrap_or(""), thread).await;
        }
        if action == "attachment" {
            let id = a.file_id.clone().filter(|f| FILE.is_match(f)).ok_or_else(|| invalid_resource("Invalid Slack file"))?;
            let w = self.workspace(profile)?;
            return self.attachment(&w, &id).await;
        }
        match action {
            "list" | "read" | "search" | "send" | "edit" | "delete" | "react" => {}
            _ => return Err(Error::comm("UNSUPPORTED_CAPABILITY", "Slack operation unsupported")),
        }
        let limit = a.limit.to_string();
        match action {
            "list" => {
                let w = self.workspace(profile)?;
                let mut p = vec![("types", "public_channel,private_channel,mpim,im".to_string()), ("limit", limit), ("exclude_archived", "false".into())];
                if let Some(c) = a.cursor.clone().filter(|c| !c.is_empty()) {
                    p.push(("cursor", c));
                }
                // the user's own conversations: conversations.list pages every public channel of the workspace before
                // the first DM (tens of thousands at ITV)
                let r = self.ok(&w, "users.conversations", &p).await?;
                let channels = r.get("channels").and_then(Value::as_array).cloned().unwrap_or_default();
                let ims: Vec<String> =
                    channels.iter().filter(|c| truthy_opt(c.get("is_im")) && truthy_opt(c.get("user"))).map(|c| crate::js::string(c.get("user"))).collect();
                let users = if ims.is_empty() { vec![] } else { self.users(&w, ims).await };
                let conversations: Vec<Value> = channels
                    .iter()
                    .map(|ch| {
                        let mut o = match pick(
                            ch,
                            &["id", "name", "user", "is_channel", "is_group", "is_im", "is_mpim", "is_private", "is_archived", "is_member", "num_members"],
                        ) {
                            Value::Object(o) => o,
                            _ => Obj::new(),
                        };
                        set(&mut o, "topic", ch.get("topic").filter(|t| !t.is_null()).and_then(|t| t.get("value")).cloned());
                        set(&mut o, "purpose", ch.get("purpose").filter(|t| !t.is_null()).and_then(|t| t.get("value")).cloned());
                        Value::Object(o)
                    })
                    .collect();
                let next = r.pointer("/response_metadata/next_cursor").filter(|c| truthy(c)).cloned().unwrap_or(Value::Null);
                Ok(json!({"conversation_count": channels.len(), "conversations": conversations, "users": users, "next_cursor": next}))
            }
            "read" => {
                let ch = channel()?;
                let thread = a.ts.as_ref().map(|_| ts()).transpose()?;
                let w = self.workspace(profile)?;
                let r = match &thread {
                    Some(t) => {
                        let first = self.ok(&w, "conversations.replies", &[("channel", ch.clone()), ("ts", t.clone()), ("limit", limit.clone())]).await?;
                        // a reply's ts reads its whole thread, from the parent
                        let parent = first
                            .pointer("/messages/0")
                            .filter(|m| m.get("ts").and_then(Value::as_str) == Some(t))
                            .and_then(|m| m.get("thread_ts"))
                            .and_then(Value::as_str)
                            .filter(|p| !p.is_empty() && p != t);
                        match parent {
                            Some(p) => {
                                self.ok(&w, "conversations.replies", &[("channel", ch.clone()), ("ts", p.to_string()), ("limit", limit.clone())]).await?
                            }
                            None => first,
                        }
                    }
                    None => self.ok(&w, "conversations.history", &[("channel", ch.clone()), ("limit", limit.clone())]).await?,
                };
                let mut messages = r.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
                let mut next_oldest: Option<String> = None;
                for m in &messages {
                    if let Some(t) = m.get("ts").and_then(Value::as_str).filter(|t| !t.is_empty())
                        && next_oldest.as_deref().is_none_or(|c| compare_ts(t, c).is_gt())
                    {
                        next_oldest = Some(t.to_string());
                    }
                }
                if thread.is_none() {
                    messages.reverse();
                }
                let ids: Vec<String> = messages.iter().filter(|m| truthy_opt(m.get("user"))).map(|m| crate::js::string(m.get("user"))).collect();
                let users = if ids.is_empty() { vec![] } else { self.users(&w, ids).await };
                Ok(json!({
                    "channel_id": ch,
                    "message_count": messages.len(),
                    "next_oldest": next_oldest,
                    "has_more": truthy_opt(r.get("has_more")),
                    "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
                    "users": users,
                }))
            }
            "search" => {
                let query = a.query.as_deref().unwrap_or("").trim().to_string();
                if query.is_empty() {
                    return Err(failed());
                }
                let w = self.workspace(profile)?;
                let r = self
                    .ok(
                        &w,
                        "search.messages",
                        &[("query", query.clone()), ("count", limit), ("page", "1".into()), ("sort", "timestamp".into()), ("sort_dir", "desc".into())],
                    )
                    .await?;
                let mut matches = r.pointer("/messages/matches").and_then(Value::as_array).cloned().unwrap_or_default();
                // a reply's match carries its thread only in the permalink
                for m in &mut matches {
                    let from_link = m
                        .get("permalink")
                        .and_then(Value::as_str)
                        .and_then(|p| url::Url::parse(p).ok())
                        .and_then(|u| u.query_pairs().find(|(k, _)| k == "thread_ts").map(|(_, v)| v.into_owned()))
                        .filter(|t| TS.is_match(t));
                    if let (Some(t), Value::Object(o)) = (from_link, &mut *m)
                        && !truthy_opt(o.get("thread_ts"))
                    {
                        o.insert("thread_ts".into(), json!(t));
                    }
                }
                if matches.is_empty() {
                    // slackcli prints nothing for no match
                    return Ok(json!({}));
                }
                let total = r.pointer("/messages/total").filter(|t| truthy(t)).cloned().unwrap_or(json!(0));
                let page = r.pointer("/messages/pagination/page").filter(|t| truthy(t)).cloned().unwrap_or(json!(1));
                let pages = r.pointer("/messages/pagination/page_count").filter(|t| truthy(t)).cloned().unwrap_or(json!(1));
                Ok(json!({"query": query, "total": total, "page": page, "pages": pages, "matches": matches}))
            }
            "send" => {
                let ch = channel()?;
                let thread = a.ts.as_ref().map(|_| ts()).transpose()?;
                let w = self.workspace(profile)?;
                let target = self.open(&w, &ch).await?;
                let mut p = vec![("channel", target.clone()), ("text", a.text.clone().unwrap_or_default())];
                if let Some(t) = thread {
                    p.push(("thread_ts", t));
                }
                let r = self.ok(&w, "chat.postMessage", &p).await?;
                let posted = r.get("ts").cloned();
                let mut o = Obj::new();
                o.insert("channel_id".into(), json!(target));
                set(&mut o, "ts", posted.clone());
                let link = self.request(&w, "chat.getPermalink", &[("channel", target), ("message_ts", crate::js::string(posted.as_ref()))]).await.ok();
                set(&mut o, "permalink", link.and_then(|l| l.get("permalink").filter(|p| truthy(p)).cloned()));
                Ok(Value::Object(o))
            }
            "edit" | "delete" => {
                let ch = channel()?;
                let t = ts()?;
                let w = self.workspace(profile)?;
                let me = self.ok(&w, "auth.test", &[]).await?;
                let user_id = me.get("user_id").and_then(Value::as_str).filter(|u| !u.is_empty()).map(String::from).or(w.user_id.clone());
                let original = self.message(&w, &ch, &t).await?;
                if user_id.is_none() || original.get("user").and_then(Value::as_str) != user_id.as_deref() {
                    return Err(Error::comm("OWNERSHIP_REQUIRED", "Only own Slack messages can be changed"));
                }
                if action == "edit" {
                    let r = self
                        .ok(&w, "chat.update", &[("channel", ch.clone()), ("ts", t), ("text", a.text.clone().unwrap_or_default()), ("parse", "none".into())])
                        .await?;
                    let mut o = Obj::new();
                    o.insert("channel_id".into(), json!(ch));
                    set(&mut o, "ts", r.get("ts").cloned());
                    return Ok(Value::Object(o));
                }
                match self.request(&w, "chat.delete", &[("channel", ch.clone()), ("ts", t.clone())]).await {
                    Ok(_) => Ok(json!({"channel_id": ch, "ts": t, "deleted": true, "already_deleted": false})),
                    Err(Fail::Slack(code)) if code == "message_not_found" => Ok(json!({"channel_id": ch, "ts": t, "deleted": false, "already_deleted": true})),
                    Err(_) => Err(failed()),
                }
            }
            "react" => {
                let ch = channel()?;
                let t = ts()?;
                let emoji = a.emoji.clone().unwrap_or_default();
                let w = self.workspace(profile)?;
                let p = [("channel", ch.clone()), ("timestamp", t.clone()), ("name", emoji.clone())];
                if !a.remove {
                    self.ok(&w, "reactions.add", &p).await?;
                    return Ok(json!({"channel_id": ch, "ts": t, "emoji": emoji}));
                }
                match self.request(&w, "reactions.remove", &p).await {
                    Ok(_) => Ok(json!({"channel_id": ch, "ts": t, "emoji": emoji, "removed": true, "already_removed": false})),
                    Err(Fail::Slack(code)) if code == "no_reaction" => {
                        Ok(json!({"channel_id": ch, "ts": t, "emoji": emoji, "removed": false, "already_removed": true}))
                    }
                    Err(_) => Err(failed()),
                }
            }
            _ => Err(Error::comm("UNSUPPORTED_CAPABILITY", "Slack operation unsupported")),
        }
    }

    /// One message by channel and ts: messages.list for a browser session (replies too), history for an app token.
    async fn message(&self, w: &Workspace, channel: &str, ts: &str) -> Result<Value> {
        let found = match &w.auth {
            Auth::Browser { .. } => {
                let ids = json!([{"channel": channel, "timestamps": [ts]}]).to_string();
                let r = self.ok(w, "messages.list", &[("message_ids", ids)]).await?;
                r.get("messages").and_then(|m| m.get(channel)).and_then(|l| l.get(0)).cloned()
            }
            Auth::Standard { .. } => {
                let r = self
                    .ok(
                        w,
                        "conversations.history",
                        &[
                            ("channel", channel.to_string()),
                            ("latest", ts.to_string()),
                            ("oldest", ts.to_string()),
                            ("inclusive", "true".into()),
                            ("limit", "1".into()),
                        ],
                    )
                    .await?;
                r.pointer("/messages/0").cloned()
            }
        };
        found.ok_or_else(failed)
    }

    async fn upload(&self, w: &Workspace, channel: &str, files: Vec<(String, Vec<u8>)>, comment: &str, thread: Option<String>) -> Result<Value> {
        let target = self.open(w, channel).await?;
        if files.iter().any(|(_, b)| b.is_empty()) {
            return Err(failed());
        }
        let mut uploaded = vec![];
        for (name, bytes) in files {
            let r = self.ok(w, "files.getUploadURLExternal", &[("filename", name.clone()), ("length", bytes.len().to_string())]).await?;
            let (Some(url), Some(id)) = (r.get("upload_url").and_then(Value::as_str), r.get("file_id").and_then(Value::as_str)) else { return Err(failed()) };
            let u = url::Url::parse(url).map_err(|_| failed())?;
            if u.scheme() != "https" && !cfg!(feature = "test-hosts") {
                return Err(failed());
            }
            let put = http_client()
                .post(u)
                .header("Content-Type", "application/octet-stream")
                .body(bytes)
                .timeout(Duration::from_secs(45))
                .send()
                .await
                .map_err(|_| failed())?;
            if !put.status().is_success() {
                return Err(failed());
            }
            uploaded.push(json!({"id": id, "title": name}));
        }
        let mut p = vec![("files", Value::Array(uploaded).to_string()), ("channel_id", target.clone())];
        if !comment.is_empty() {
            p.push(("initial_comment", comment.to_string()));
        }
        if let Some(t) = thread {
            p.push(("thread_ts", t));
        }
        let r = self.ok(w, "files.completeUploadExternal", &p).await?;
        let ids: Vec<Value> =
            r.get("files").and_then(Value::as_array).map(|f| f.iter().map(|x| x.get("id").cloned().unwrap_or(Value::Null)).collect()).unwrap_or_default();
        let mut o = Obj::new();
        o.insert("channel_id".into(), json!(target));
        set(&mut o, "file_id", ids.first().cloned().filter(|v| !v.is_null()));
        o.insert("file_ids".into(), Value::Array(ids));
        Ok(Value::Object(o))
    }

    async fn attachment(&self, w: &Workspace, id: &str) -> Result<Value> {
        let r = self.ok(w, "files.info", &[("file", id.to_string())]).await?;
        let info = r.get("file").filter(|f| truthy(f)).cloned().ok_or_else(failed)?;
        let inner = info.get("file");
        let size = inner.and_then(|f| crate::js::present(f.get("size"))).or(info.get("size"));
        if size.and_then(Value::as_f64).is_none_or(|s| s > MAX_BYTES as f64) {
            return Err(Error::comm("LIMIT_EXCEEDED", "Attachment size unavailable or exceeds 5 MB"));
        }
        let mime = inner
            .and_then(|f| f.get("mimetype"))
            .filter(|m| truthy(m))
            .or(info.get("mimetype").filter(|m| truthy(m)))
            .map(|m| crate::js::string(Some(m)))
            .unwrap_or_else(|| "application/octet-stream".into());
        let url = info.get("url_private_download").filter(|u| truthy(u)).or(info.get("url_private").filter(|u| truthy(u))).and_then(Value::as_str);
        let download_failed = || Error::comm("PROVIDER_ERROR", "Slack attachment download failed");
        let mut current = url::Url::parse(url.ok_or_else(download_failed)?).map_err(|_| download_failed())?;
        if (current.scheme() != "https" && !cfg!(feature = "test-hosts")) || !is_slack_host(&current) {
            return Err(download_failed());
        }
        let mut redirects = 0;
        let resp = loop {
            let mut req = http_client().get(current.clone()).timeout(Duration::from_secs(45));
            // credentials go only to Slack hosts, hop by hop
            if is_slack_host(&current) {
                req = match &w.auth {
                    Auth::Browser { xoxd, .. } => {
                        req.header("Cookie", format!("d={}", crate::js::encode_uri_component(xoxd))).header("Origin", "https://app.slack.com")
                    }
                    Auth::Standard { token } => req.header("Authorization", format!("Bearer {token}")),
                };
            }
            let r = req.send().await.map_err(|_| download_failed())?;
            if [301, 302, 303, 307, 308].contains(&r.status().as_u16()) {
                let loc = r.headers().get("location").and_then(|v| v.to_str().ok()).ok_or_else(download_failed)?;
                let next = current.join(loc).map_err(|_| download_failed())?;
                if next.scheme() != "https" && !cfg!(feature = "test-hosts") {
                    return Err(download_failed());
                }
                redirects += 1;
                if redirects > 5 {
                    return Err(download_failed());
                }
                current = next;
                continue;
            }
            if !r.status().is_success() {
                return Err(download_failed());
            }
            break r;
        };
        let signin = resp.url().path().to_lowercase();
        let html = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.to_lowercase().contains("text/html"));
        let mut bytes = vec![];
        let mut resp = resp;
        while let Some(chunk) = resp.chunk().await.map_err(|_| download_failed())? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_BYTES {
                return Err(Error::comm("LIMIT_EXCEEDED", "Attachment exceeds 5 MB"));
            }
        }
        static AUTH_PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)<form[^>]*signin|data-qa="signin|<title>[^<]*Sign\s*in"#).expect("fixed"));
        let signin_url = Regex::new(r"/(?:signin|ssb/signin)(?:/|$)").expect("fixed").is_match(&signin);
        if signin_url || (html && mime.to_lowercase() != "text/html" && AUTH_PAGE.is_match(&String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]))) {
            return Err(download_failed());
        }
        Ok(json!({"data": base64_std(&bytes), "mime_type": mime}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_and_ts() {
        assert_eq!(workspace_origin("https://itv.slack.com/").as_deref(), Some("https://itv.slack.com"));
        assert_eq!(workspace_origin("https://itv.slack.com"), Some("https://itv.slack.com".into()));
        assert_eq!(workspace_origin("https://app.slack.com/"), None);
        assert_eq!(workspace_origin("https://slack.com.evil.net/"), None);
        assert_eq!(workspace_origin("http://itv.slack.com/"), None);
        assert_eq!(workspace_origin("https://itv.slack.com/x"), None);
        assert!(compare_ts("1712345678.000010", "1712345678.00001").is_eq());
        assert!(compare_ts("1712345679.1", "1712345678.9").is_gt());
        assert!(compare_ts("999.5", "1000.1").is_lt());
    }

    #[test]
    fn json_shapes() {
        let m = json!({"ts": "1.2", "user": "U1", "text": "hi", "files": [{"id": "F1", "name": "a", "extra": 1}], "junk": true});
        assert_eq!(message_json(&m).to_string(), r#"{"ts":"1.2","user":"U1","text":"hi","files":[{"id":"F1","name":"a"}]}"#);
        assert_eq!(user_json(&json!({"id": "U1", "name": "n", "profile": {"email": "e@x"}})).to_string(), r#"{"id":"U1","name":"n","email":"e@x"}"#);
    }
}
