//! The chat service with the sign-in of the Teams page (the page half of src/agent/teams/chatsvc.ts): the IC3 token
//! from the cookies of the browser or the MSAL cache of the page, the region from the authtoken cookie, and chat names
//! resolved to conversation ids through the IndexedDB of Teams, else the conversation directory of the chat service.

use crate::agent::api::TeamsAuth;
use crate::agent::browser::Page;
use crate::agent::chatsvc::ChatSvc;
use crate::agent::http::{Http, json, send};
use crate::agent::{now_ms, text_of, warn};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

const SERVER: &str = "https://teams.microsoft.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";
const UNRESOLVED_MS: i64 = 60_000;
const FORCED_SYNC_GAP_MS: i64 = 5_000;
const TITLE_TTL_MS: i64 = 15 * 60_000;

/// The conversations and the messages of the IndexedDB of Teams (syncIndexedDbMappings): id, topics, titles, people of
/// each conversation, and the conversation of each message id.
const IDB_READ: &str = r#"(async () => {
  if (typeof indexedDB === "undefined" || typeof indexedDB.databases !== "function") return null;
  const dbs = await indexedDB.databases();
  const convs = [];
  const mids = [];
  for (const info of dbs) {
    const dbName = info.name || "";
    if (!dbName.includes(":conversation-manager:") && !dbName.includes(":replychain-manager:")) continue;
    try {
      const db = await new Promise((resolve, reject) => { const req = indexedDB.open(dbName); req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error); });
      const all = (store) => new Promise((resolve) => { const req = db.transaction(store, "readonly").objectStore(store).getAll(); req.onsuccess = () => resolve(req.result || []); req.onerror = () => resolve([]); });
      if (dbName.includes(":conversation-manager:") && db.objectStoreNames.contains("conversations")) {
        const texts = (o, keys) => keys.map((k) => (typeof o[k] === "string" ? o[k].trim() : "")).filter(Boolean);
        for (const r of await all("conversations")) {
          const id = typeof r?.id === "string" ? r.id : "";
          if (!id) continue;
          const ct = r.chatTitle || {};
          const users = Array.isArray(ct.avatarUsersInfo) ? ct.avatarUsersInfo : [];
          convs.push({ id, topics: texts(r.threadProperties || {}, ["topic", "spaceThreadTopic", "topicThreadTopic"]), chatTitles: texts(ct, ["shortTitle", "longTitle"]), people: users.flatMap((u) => texts(u, ["displayName"])) });
        }
      }
      if (dbName.includes(":replychain-manager:") && db.objectStoreNames.contains("replychains-2")) {
        for (const rc of await all("replychains-2")) {
          for (const m of Object.values(rc?.messageMap || {})) {
            const mid = typeof m?.id === "string" ? m.id : typeof m?.dedupeKey === "string" ? m.dedupeKey : "";
            const cid = typeof m?.conversationId === "string" ? m.conversationId : "";
            if (mid && cid) mids.push({ mid, cid });
          }
        }
      }
      db.close();
    } catch {}
  }
  return { convs, mids };
})()"#;

/// Region of the chat service: from the path of the authtoken or McasPartialCookie*authtoken cookie
/// (/api/mt/part/emea-03/...), "emea" when there is none.
pub fn region_of(cookies: &[Value]) -> String {
    let s = |c: &Value, k: &str| c[k].as_str().unwrap_or("").to_string();
    let auth = cookies.iter().find(|c| {
        let (name, domain) = (s(c, "name"), s(c, "domain"));
        (name == "authtoken" || name.ends_with("authtoken")) && (domain.contains("teams.microsoft.com") || domain.contains("teams.cloud.microsoft"))
    });
    let Some(path) = auth.map(|c| s(c, "path")).filter(|p| !p.is_empty()) else { return "emea".into() };
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let Some(at) = parts.iter().position(|p| *p == "mt").filter(|i| i + 1 < parts.len()) else { return "emea".into() };
    let mut r = parts[at + 1];
    if r == "part" && at + 2 < parts.len() {
        r = parts[at + 2];
    }
    r.split('-').next().filter(|x| !x.is_empty()).unwrap_or("emea").to_string()
}

/// The names the chat list shows for a conversation: the topic when it has one, otherwise the titles Teams made, and the
/// other person only for a 1:1 chat (conversationTitles).
pub fn conversation_titles(id: &str, topics: &[String], chat_titles: &[String], people: &[String]) -> Vec<String> {
    if !topics.is_empty() {
        return topics.to_vec();
    }
    let one = id.ends_with("@unq.gbl.spaces") || id == "48:notes";
    let person = if one && people.len() == 1 { people[0].clone() } else { String::new() };
    let mut out = chat_titles.to_vec();
    if !person.is_empty() {
        out.push(person.clone());
        if id == "48:notes" {
            out.push(format!("{person} (You)"));
        }
    }
    out
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

/// What a slot keeps between two uses of the page's sign-in: the token for a minute, chat names and message ids to
/// conversations, the names not to look up again for a while.
#[derive(Default)]
pub struct PageAuth {
    cached: RefCell<Option<(TeamsAuth, i64)>>,
    titles: RefCell<HashMap<String, (String, i64)>>,
    mids: RefCell<HashMap<String, String>>,
    unresolved: RefCell<HashMap<String, i64>>,
    synced: Cell<i64>,
}

impl PageAuth {
    /// The IC3 token and region with the sign-in of the page; None while the page has none the chat service takes.
    pub async fn auth(&self, page: &Page, svc: &ChatSvc) -> Option<TeamsAuth> {
        match self.read(page, svc).await {
            Ok(a) => a,
            Err(e) => {
                warn("chatsvc", &format!("getTeamsAuth failed: {e}"), &[]);
                None
            }
        }
    }

    async fn read(&self, page: &Page, svc: &ChatSvc) -> Result<Option<TeamsAuth>, String> {
        if let Some((a, until)) = self.cached.borrow().clone()
            && now_ms() < until
            && !svc.refused(&a.token)
        {
            return Ok(Some(a));
        }
        let Some(browser) = page.browser() else { return Ok(None) };
        let cookies = browser.cookies().await?;
        let region = region_of(&cookies);
        let s = |c: &Value, k: &str| c[k].as_str().unwrap_or("").to_string();
        // behind Defender for Cloud Apps the proxy keeps the cookies of the session in its own encoding: refused (911)
        let proxied = |c: &Value| s(c, "domain").ends_with(".mcas.ms");
        let asm = cookies
            .iter()
            .find(|c| {
                let d = s(c, "domain");
                s(c, "name") == "authtoken_asm"
                    && !proxied(c)
                    && (d.contains("asyncgw.teams.microsoft.com") || d.contains("teams.microsoft.com") || d.contains("teams.cloud.microsoft"))
            })
            .or_else(|| cookies.iter().find(|c| s(c, "name") == "authtoken_asm" && !proxied(c)));
        let mut token = String::new();
        if let Some(c) = asm {
            let v = s(c, "value");
            let low = v.to_lowercase();
            let v = if low.starts_with("bearer%20") {
                &v[9..]
            } else if low.starts_with("bearer ") || low.starts_with("bearer=") {
                &v[7..]
            } else {
                &v[..]
            };
            token = form_urlencoded::parse(format!("x={}", v.replace('+', "%2B")).as_bytes()).next().map(|(_, x)| x.into_owned()).unwrap_or_default();
        }
        if token.is_empty() || svc.refused(&token) {
            let entries = page.evaluate("tokens.readMsalEntries", Some(&json!(["accesstoken", "ic3.teams.office.com"]))).await.unwrap_or(Value::Null);
            if let Some(t) = entries.as_array().into_iter().flatten().filter_map(|e| e["secret"].as_str()).find(|t| t.encode_utf16().count() > 50) {
                token = t.to_string();
            }
        }
        if token.is_empty() || svc.refused(&token) {
            return Ok(None);
        }
        let c = crate::agent::claims(&token);
        let claim = |k: &str| c.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from);
        let a = TeamsAuth { region, oid: claim("oid"), upn: claim("upn").or_else(|| claim("unique_name")), name: claim("name"), token };
        *self.cached.borrow_mut() = Some((a.clone(), now_ms() + 60_000));
        Ok(Some(a))
    }

    fn cache_title(&self, title: &str, id: &str) {
        let clean = crate::agent::js_trim(title);
        if clean.is_empty() || id.is_empty() {
            return;
        }
        let until = now_ms() + TITLE_TTL_MS;
        let mut t = self.titles.borrow_mut();
        t.insert(clean.to_string(), (id.to_string(), until));
        t.insert(clean.to_lowercase(), (id.to_string(), until));
    }

    /// The conversation of a chat name already known, without asking Teams.
    pub fn cached(&self, chat: &str) -> Option<String> {
        let t = self.titles.borrow();
        let hit = t.get(chat).or_else(|| t.get(&crate::agent::js_trim(chat).to_lowercase()))?;
        (now_ms() < hit.1).then(|| hit.0.clone())
    }

    /// The chat names and message ids of the IndexedDB of Teams, at most every 20 s (5 s when forced).
    pub async fn sync(&self, page: &Page, force: bool) {
        if now_ms() - self.synced.get() < if force { FORCED_SYNC_GAP_MS } else { 20_000 } {
            return;
        }
        self.synced.set(now_ms());
        let Ok(res) = page.eval_js(IDB_READ).await else { return };
        let convs: Vec<(String, Vec<String>)> = res["convs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                let id = c["id"].as_str().unwrap_or("").to_string();
                let titles = conversation_titles(&id, &strings(&c["topics"]), &strings(&c["chatTitles"]), &strings(&c["people"]));
                (id, titles)
            })
            .collect();
        let mut ids_of: HashMap<String, HashSet<String>> = HashMap::new();
        for (id, titles) in &convs {
            for t in titles {
                ids_of.entry(crate::agent::js_trim(t).to_lowercase()).or_default().insert(id.clone());
            }
        }
        for (id, titles) in &convs {
            for t in titles {
                let low = crate::agent::js_trim(t).to_lowercase();
                if ids_of.get(&low).map_or(0, HashSet::len) == 1 {
                    self.cache_title(t, id);
                } else {
                    // two conversations of one name: a message must not reach the wrong one
                    let mut titles = self.titles.borrow_mut();
                    titles.remove(crate::agent::js_trim(t));
                    titles.remove(&low);
                    self.unresolved.borrow_mut().insert(low, now_ms() + 15 * 60_000);
                }
            }
        }
        for m in res["mids"].as_array().into_iter().flatten() {
            if let (Some(mid), Some(cid)) = (m["mid"].as_str(), m["cid"].as_str()) {
                self.mids.borrow_mut().insert(mid.into(), cid.into());
            }
        }
    }

    fn by_mid(&self, chat: &str, mid: Option<&str>) -> Option<String> {
        let cid = self.mids.borrow().get(mid?).cloned()?;
        self.cache_title(chat, &cid);
        Some(cid)
    }

    fn unresolved(&self, low: &str) -> bool {
        self.unresolved.borrow().get(low).is_some_and(|until| *until > now_ms())
    }

    /// The conversation of a chat of the list (resolveConversationId); None when none is known for sure.
    pub async fn conversation(&self, http: &Http, svc: &ChatSvc, auth: &TeamsAuth, page: &Page, chat: &str, mid: Option<&str>) -> Option<String> {
        if chat.starts_with("19:") || chat.starts_with("48:") {
            return Some(chat.into());
        }
        if regex::Regex::new(r"(?i)\(You\)$").expect("fixed").is_match(crate::agent::js_trim(chat)) {
            return Some("48:notes".into());
        }
        if let Some(c) = self.by_mid(chat, mid).or_else(|| self.cached(chat)) {
            return Some(c);
        }
        let low = crate::agent::js_trim(chat).to_lowercase();
        if self.unresolved(&low) {
            return None;
        }
        self.sync(page, false).await;
        if let Some(c) = self.by_mid(chat, mid).or_else(|| self.cached(chat)) {
            return Some(c);
        }
        self.sync(page, true).await;
        if self.unresolved(&low) {
            return None;
        }
        if let Some(c) = self.cached(chat) {
            return Some(c);
        }
        match self.directory(http, svc, auth, &low).await {
            Ok(Some(c)) => return Some(c),
            Ok(None) => {}
            Err(e) => warn("chatsvc", &format!("resolveConversationId failed: {}", text_of(&e)), &[("chat", chat.into())]),
        }
        self.unresolved.borrow_mut().insert(low, now_ms() + UNRESOLVED_MS);
        None
    }

    /// The conversation directory of the chat service: group and meeting topics, the sender of the last message of a
    /// 1:1 chat.
    async fn directory(&self, http: &Http, svc: &ChatSvc, auth: &TeamsAuth, low: &str) -> crate::agent::R<Option<String>> {
        let url = format!("{SERVER}/api/chatsvc/{}/v1/users/ME/conversations?view=msnp24Equivalent&pageSize=200&startTime=0", auth.region);
        let headers = vec![
            ("Accept", "application/json".to_string()),
            ("User-Agent", USER_AGENT.to_string()),
            ("Origin", SERVER.to_string()),
            ("Referer", format!("{SERVER}/v2/")),
            ("Authorization", format!("Bearer {}", auth.token)),
        ];
        let r = send(http, "GET", &url, headers, vec![], 10_000).await?;
        if r.status == 401 {
            svc.note_refused(&auth.token);
        }
        if !r.ok() {
            return Ok(None);
        }
        let data = json(&r)?;
        let mut matched = None;
        for c in data["conversations"].as_array().into_iter().flatten() {
            let Some(id) = c["id"].as_str().filter(|i| !i.is_empty()) else { continue };
            let tp = &c["threadProperties"];
            let title = ["topic", "spaceThreadTopic", "threadTopic"].iter().filter_map(|k| tp[*k].as_str()).find(|t| !t.is_empty());
            if let Some(title) = title {
                self.cache_title(title, id);
                if crate::agent::js_trim(title).to_lowercase() == low && matched.is_none() {
                    matched = Some(id.to_string());
                }
            } else if id.contains("@unq.gbl.spaces") && c["lastMessage"].is_object() {
                let lm = &c["lastMessage"];
                let sender = crate::agent::js_trim(
                    [lm["imdisplayname"].as_str(), lm["fromDisplayNameInToken"].as_str()].into_iter().flatten().find(|x| !x.is_empty()).unwrap_or(""),
                )
                .to_string();
                let from = lm["from"].as_str().unwrap_or("").to_lowercase();
                let me = auth.oid.as_deref().is_some_and(|o| !o.is_empty() && from.contains(&o.to_lowercase()));
                if !sender.is_empty() && !me {
                    self.cache_title(&sender, id);
                    if sender.to_lowercase() == low && matched.is_none() {
                        matched = Some(id.to_string());
                    }
                }
            }
        }
        Ok(matched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_and_titles() {
        let c = |name: &str, domain: &str, path: &str| json!({ "name": name, "domain": domain, "path": path });
        assert_eq!(region_of(&[c("authtoken", ".teams.microsoft.com", "/api/mt/part/amer-02/beta")]), "amer");
        assert_eq!(region_of(&[c("McasPartialCookie1authtoken", "teams.cloud.microsoft", "/api/mt/apac/x")]), "apac");
        assert_eq!(region_of(&[c("other", ".teams.microsoft.com", "/api/mt/amer")]), "emea");
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(conversation_titles("19:a@thread.v2", &s(&["Release"]), &s(&["x"]), &[]), ["Release"]);
        assert_eq!(conversation_titles("19:a@unq.gbl.spaces", &[], &s(&["Rossi Anna"]), &s(&["Rossi Anna"])), ["Rossi Anna", "Rossi Anna"]);
        assert_eq!(conversation_titles("19:m@thread.v2", &[], &s(&["Meeting"]), &s(&["Rossi Anna"])), ["Meeting"]);
        assert_eq!(conversation_titles("48:notes", &[], &[], &s(&["Me"])), ["Me", "Me (You)"]);
    }
}
