//! The chat service of Teams over HTTP (the part of src/agent/teams/chatsvc.ts a light account uses): the messages of a
//! chat as the app shows them, and sending, replying, editing, deleting, reacting, images, read marks. What a slot
//! remembers between calls (the last message of each chat, the messages just sent) is in ChatSvc, one per slot.

use super::api::TeamsAuth;
use super::http::{Http, json, json_or_empty, send, text};
use super::store::SavedMessage;
use super::{Fail, R, cut, js_trim, now_ms, warn};
use regex::{Captures, Regex};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";
const AMS_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) SkypeSpaces/1410/0.0.0.0";
const SERVER: &str = "https://teams.microsoft.com";

/// The six quick reactions of Teams as it draws them (REACTION_EMOJI of src/shared/slot-db/rows.ts).
pub fn reaction_emoji(key: &str) -> Option<&'static str> {
    Some(match key {
        "like" => "\u{1f44d}",
        "heart" => "\u{2764}\u{fe0f}",
        "laugh" => "\u{1f606}",
        "surprised" => "\u{1f62e}",
        "cry" => "\u{1f622}",
        "angry" => "\u{1f620}",
        _ => return None,
    })
}

fn emoji_key(e: &str) -> Option<&'static str> {
    Some(match e {
        "like" | "\u{1f44d}" => "like",
        "heart" | "\u{2764}\u{fe0f}" | "\u{2764}" => "heart",
        "laugh" | "\u{1f606}" => "laugh",
        "surprised" | "\u{1f62e}" => "surprised",
        "cry" | "sad" | "\u{1f622}" => "sad",
        "angry" | "\u{1f620}" => "angry",
        _ => return None,
    })
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("fixed pattern")
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn unescape_html(s: &str) -> String {
    let mut s = s.to_string();
    for (p, to) in [("(?i)&nbsp;", " "), ("(?i)&amp;", "&"), ("(?i)&lt;", "<"), ("(?i)&gt;", ">"), ("(?i)&quot;", "\""), ("(?i)&#39;", "'")] {
        s = re(p).replace_all(&s, to).to_string();
    }
    s
}

fn strip_tags(s: &str) -> String {
    re("<[^>]+>").replace_all(s, "").to_string()
}

fn to_paragraphs(text: &str) -> String {
    text.split('\n').map(|line| format!("<p>{}</p>", if js_trim(line).is_empty() { "&nbsp;".into() } else { escape_html(line) })).collect()
}

fn client_message_id() -> String {
    let n = 100 + (u16::from_le_bytes([super::random_bytes(1)[0], super::random_bytes(1)[0]]) % 899);
    format!("{}{n}", now_ms())
}

/// Math.round of a number of the page.
fn round(v: &str) -> i64 {
    v.parse::<f64>().map(|f| (f + 0.5).floor() as i64).unwrap_or(0)
}

/// A message of Teams as the app shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub quote: Option<(String, String)>,
    pub text: String,
    pub html: String,
    pub images: Vec<(String, i64, i64)>,
    pub mentions_me: bool,
}

/// The HTML of a message of Teams as { quote, text, html, images } (parseTeamsMessageHtml).
pub fn parse_teams_message_html(raw: &str, my_name: Option<&str>) -> Parsed {
    let mut content = raw.to_string();
    let mut quote = None;
    if let Some(m) = re(r"(?i)<blockquote[^>]*schema\.skype\.com/Reply[^>]*>([\s\S]*?)</blockquote>").captures(&content) {
        let inner = m.get(1).map_or("", |x| x.as_str());
        let who = re(r"(?i)<strong[^>]*>([\s\S]*?)</strong>").captures(inner).map(|c| c[1].to_string()).unwrap_or_default();
        let prev = re(r#"(?i)<p[^>]*itemprop=["']preview["'][^>]*>([\s\S]*?)</p>"#).captures(inner).map(|c| c[1].to_string()).unwrap_or_default();
        let author = cut(js_trim(&unescape_html(&strip_tags(&who))), 60);
        let text = cut(js_trim(&unescape_html(&strip_tags(&prev))), 300);
        if !author.is_empty() || !text.is_empty() {
            quote = Some((author, text));
        }
        let whole = m.get(0).expect("match").as_str().to_string();
        content = content.replacen(&whole, "", 1);
    }
    let mut images = vec![];
    let with_emojis = re(r"(?i)<img\b[^>]*>")
        .replace_all(&content, |c: &Captures| {
            let tag = &c[0];
            let alt = re(r#"(?i)\balt=["']([^"']*)["']"#).captures(tag).map(|a| unescape_html(&a[1])).unwrap_or_default();
            if re(r"(?i)schema\.skype\.com/Emoji").is_match(tag) || re("(?i)emoticon").is_match(tag) {
                return alt;
            }
            let src = re(r#"(?i)\bsrc=["']([^"']+)["']"#).captures(tag).map(|a| unescape_html(&a[1])).unwrap_or_default();
            if !src.is_empty() && !re("(?i)avatar").is_match(tag) {
                let dim = |k: &str| re(&format!(r#"(?i)\b{k}=["'](\d+(?:\.\d+)?)["']"#)).captures(tag).map_or(0, |a| round(&a[1]));
                images.push((src, dim("width"), dim("height")));
            }
            String::new()
        })
        .to_string();
    let mut mentions_me = false;
    let with_mentions = re(r"(?i)<span\b[^>]*schema\.skype\.com/Mention[^>]*>([\s\S]*?)</span>")
        .replace_all(&with_emojis, |c: &Captures| {
            let clean = js_trim(&unescape_html(&strip_tags(&c[1]))).to_string();
            let me = my_name.is_some_and(|n| clean.to_lowercase() == n.to_lowercase());
            mentions_me |= me;
            format!(r#"<span class="mn{}">{}</span>"#, if me { " me" } else { "" }, escape_html(&clean))
        })
        .to_string();
    let raw_text = re(r"(?i)<br\s*/?>").replace_all(&with_emojis, "\n");
    let raw_text = re(r"(?i)</(p|div|li|tr|h[1-6]|pre|blockquote)>").replace_all(&raw_text, "\n");
    let text = unescape_html(&strip_tags(&raw_text)).replace('\u{a0}', " ");
    let text = re(r"[ \t]+\n").replace_all(&text, "\n");
    let text = re(r"\n{3,}").replace_all(&text, "\n\n");
    let text = cut(js_trim(&text), 2000);
    let html = re(r"(?i)^(\s|<div>|<p>[\s\x{a0}&nbsp;]*</p>)+").replace(&with_mentions, "");
    let html = re(r"(?i)(<p>[\s\x{a0}&nbsp;]*</p>|\s)+$").replace(&html, "");
    let html = cut(js_trim(&html), 20000);
    Parsed { quote, text, html, images, mentions_me }
}

const EXTRA_KEYS: [&str; 11] = ["quote", "images", "files", "reactions", "status", "edited", "readby", "html", "mentionsMe", "av", "deleted"];

/// chat_messages.extra: the rich fields of a message that have a value, None when none has (extraOf).
pub fn extra_of(m: &Map<String, Value>) -> Option<Map<String, Value>> {
    let has = |v: &Value| match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        v => crate::js::truthy(v),
    };
    let out: Map<String, Value> = EXTRA_KEYS.iter().filter_map(|k| m.get(*k).filter(|v| has(v)).map(|v| ((*k).to_string(), v.clone()))).collect();
    (!out.is_empty()).then_some(out)
}

#[derive(Default)]
struct Patch {
    added: Vec<(String, SavedMessage, i64)>,
    edited: HashMap<String, (String, String, i64)>,
    deleted: HashMap<String, i64>,
}

/// What a slot keeps between calls of the chat service.
#[derive(Default)]
pub struct ChatSvc {
    /// tokens chatsvc refused (401), logged once each
    refused: RefCell<HashSet<String>>,
    /// the last message of each conversation read: id and client message id
    last: RefCell<HashMap<String, (String, String)>>,
    /// messages just sent, edited or deleted, shown at once before the service has them
    optimistic: RefCell<HashMap<String, Patch>>,
}

fn headers(auth: &TeamsAuth, json_body: bool) -> Vec<(&'static str, String)> {
    let mut h = vec![("Accept", "application/json".to_string())];
    if json_body {
        h.push(("Content-Type", "application/json".to_string()));
    }
    h.extend([
        ("User-Agent", USER_AGENT.to_string()),
        ("Origin", SERVER.to_string()),
        ("Referer", format!("{SERVER}/v2/")),
        ("Authorization", format!("Bearer {}", auth.token)),
    ]);
    h
}

fn messages_url(auth: &TeamsAuth, conv: &str) -> String {
    format!("{SERVER}/api/chatsvc/{}/v1/users/ME/conversations/{}/messages", auth.region, crate::js::encode_uri_component(conv))
}

/// The id of what was sent: OriginalArrivalTime of the answer, else now.
fn sent_id(j: &Value) -> String {
    match j.get("OriginalArrivalTime").filter(|v| crate::js::truthy(v)) {
        Some(v) => crate::js::string(Some(v)),
        None => now_ms().to_string(),
    }
}

impl ChatSvc {
    #[allow(clippy::too_many_arguments)]
    async fn call(
        &self,
        http: &Http,
        auth: &TeamsAuth,
        method: &'static str,
        url: &str,
        json_body: bool,
        body: Vec<u8>,
        timeout_ms: u64,
    ) -> R<super::http::Response> {
        let r = send(http, method, url, headers(auth, json_body), body, timeout_ms).await?;
        if r.status == 401 {
            self.note_refused(&auth.token);
        }
        Ok(r)
    }

    /// A token the chat service refused: the browser agent takes the page's MSAL token next, else the Teams page serves
    /// the account until Teams renews it. Logged once per token.
    pub fn note_refused(&self, token: &str) {
        if self.refused.borrow_mut().insert(token.to_string()) {
            warn("chatsvc", "token refused: back to the Teams page", &[]);
        }
    }

    pub fn refused(&self, token: &str) -> bool {
        self.refused.borrow().contains(token)
    }

    fn mine(auth: &TeamsAuth, conv: &str, from: &str) -> bool {
        conv == "48:notes" || auth.oid.as_deref().is_some_and(|o| !o.is_empty() && from.to_lowercase().contains(&o.to_lowercase()))
    }

    /// The recent messages of a conversation as the app saves them; None when the service did not give them.
    pub async fn fetch_chat_messages(
        &self,
        http: &Http,
        auth: &TeamsAuth,
        conv: &str,
        read_by: &dyn Fn(&[String]) -> HashMap<String, Value>,
    ) -> Option<Vec<SavedMessage>> {
        match self.fetch(http, auth, conv, read_by).await {
            Ok(v) => v,
            Err(e) => {
                warn("chatsvc", &format!("fetchChatMessages failed: {}", super::text_of(&e)), &[("convId", conv.into())]);
                None
            }
        }
    }

    async fn fetch(&self, http: &Http, auth: &TeamsAuth, conv: &str, read_by: &dyn Fn(&[String]) -> HashMap<String, Value>) -> R<Option<Vec<SavedMessage>>> {
        let url = format!("{}?view=msnp24Equivalent&pageSize=50", messages_url(auth, conv));
        let r = self.call(http, auth, "GET", &url, false, vec![], 12_000).await?;
        if !r.ok() {
            return Ok(None);
        }
        let data = json(&r)?;
        let mut raw: Vec<Value> = data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                let mt = crate::js::string(Some(m.get("messagetype").filter(|v| crate::js::truthy(v)).unwrap_or(&json!(""))));
                !mt.starts_with("ThreadActivity") && mt != "RichText/Media_CallTranscript"
            })
            .collect();
        let order = |m: &Value| {
            let id = crate::js::number(m.get("id"));
            if id != 0.0 && !id.is_nan() {
                return id;
            }
            let t = crate::js::or(&[m.get("originalarrivaltime"), m.get("composetime")])
                .filter(|v| crate::js::truthy(v))
                .map(|v| crate::js::string(Some(v)))
                .unwrap_or_default();
            super::parse_date(&t).unwrap_or(0.0)
        };
        raw.sort_by(|a, b| order(a).partial_cmp(&order(b)).unwrap_or(std::cmp::Ordering::Equal));
        let recent: Vec<Value> = raw[raw.len().saturating_sub(40)..].to_vec();
        if let Some(last) = recent.last().filter(|m| m.get("id").is_some_and(crate::js::truthy)) {
            let id = crate::js::string(last.get("id"));
            let cmid = last.get("clientmessageid").filter(|v| crate::js::truthy(v)).map_or(id.clone(), |v| crate::js::string(Some(v)));
            self.last.borrow_mut().insert(conv.to_string(), (id, cmid));
        }
        let mid_of = |m: &Value| crate::js::string(Some(crate::js::or(&[m.get("id"), m.get("clientmessageid"), Some(&json!(""))]).unwrap_or(&json!(""))));
        let from_of = |m: &Value| crate::js::string(Some(m.get("from").filter(|v| crate::js::truthy(v)).unwrap_or(&json!(""))));
        let my_mids: Vec<String> = recent.iter().filter(|m| Self::mine(auth, conv, &from_of(m))).map(mid_of).filter(|m| !m.is_empty()).collect();
        let read = read_by(&my_mids);
        let now = now_ms();
        let mut optimistic = self.optimistic.borrow_mut();
        let opt = optimistic.entry(conv.to_string()).or_default();
        let mut out = vec![];
        for m in &recent {
            let mid = mid_of(m);
            let mine = Self::mine(auth, conv, &from_of(m));
            let name = crate::js::or(&[m.get("imdisplayname"), m.get("fromDisplayNameInToken"), Some(&json!(""))])
                .map(|v| crate::js::string(Some(v)))
                .unwrap_or_default();
            let author = if mine { String::new() } else { cut(js_trim(&name), 60) };
            let props = m.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
            let deleted = props.get("deletetime").is_some_and(crate::js::truthy) || opt.deleted.get(&mid).is_some_and(|t| now < *t);
            let edit = opt.edited.get(&mid).filter(|e| now < e.2);
            let edited = props.get("edittime").is_some_and(crate::js::truthy) || edit.is_some();
            let parsed = if deleted {
                Parsed::default()
            } else if let Some((text, html, _)) = edit {
                Parsed { text: text.clone(), html: html.clone(), ..Default::default() }
            } else {
                parse_teams_message_html(m.get("content").and_then(Value::as_str).unwrap_or(""), auth.name.as_deref())
            };
            let mut files: Vec<Value> = vec![];
            if let (false, Some(Value::String(f))) = (deleted, props.get("files"))
                && f.starts_with('[')
                && let Ok(Value::Array(list)) = serde_json::from_str::<Value>(f)
            {
                for x in list {
                    let s = |v: Option<&Value>| crate::js::string(Some(v.filter(|x| crate::js::truthy(x)).unwrap_or(&json!(""))));
                    let name = cut(js_trim(&s(crate::js::or(&[x.get("fileName"), x.get("title")]))), 160);
                    let info = x.get("fileInfo");
                    let link = s(crate::js::or(&[x.get("objectUrl"), info.and_then(|i| i.get("shareUrl")), info.and_then(|i| i.get("siteUrl"))]));
                    let link = js_trim(&link).to_string();
                    if !name.is_empty() && !files.iter().any(|f| f["url"] == json!(link)) {
                        files.push(json!({ "name": name, "url": link }));
                    }
                }
            }
            let emotions = match props.get("emotions") {
                Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(json!([])),
                Some(v) => v.clone(),
                None => Value::Null,
            };
            let mut reactions: Vec<Value> = vec![];
            for em in emotions.as_array().into_iter().flatten() {
                let key = crate::js::string(Some(em.get("key").filter(|v| crate::js::truthy(v)).unwrap_or(&json!("")))).to_lowercase();
                let users = em.get("users").and_then(Value::as_array).cloned().unwrap_or_default();
                let Some(emoji) = reaction_emoji(if key == "sad" { "cry" } else { &key }) else { continue };
                if users.is_empty() {
                    continue;
                }
                let oid = auth.oid.clone().unwrap_or_default().to_lowercase();
                let is_mine = !oid.is_empty()
                    && users
                        .iter()
                        .any(|u| crate::js::string(Some(u.get("mri").filter(|v| crate::js::truthy(v)).unwrap_or(&json!("")))).to_lowercase().contains(&oid));
                reactions.push(json!({ "e": emoji, "n": users.len(), "mine": is_mine }));
            }
            let reacts = reactions
                .iter()
                .map(|r| format!("{}{}", r["e"].as_str().unwrap_or(""), if r["n"].as_u64().unwrap_or(0) > 1 { r["n"].to_string() } else { String::new() }))
                .collect::<Vec<_>>()
                .join(" ");
            let mut fields = Map::new();
            if let Some((a, t)) = &parsed.quote {
                fields.insert("quote".into(), json!({ "author": a, "text": t }));
            }
            fields.insert("images".into(), Value::Array(parsed.images.iter().map(|(u, w, h)| json!({ "url": u, "w": w, "h": h })).collect()));
            fields.insert("files".into(), Value::Array(files));
            fields.insert("reactions".into(), Value::Array(reactions));
            fields.insert("status".into(), json!(if mine { "Sent" } else { "" }));
            fields.insert("edited".into(), json!(edited));
            if let Some(rb) = read.get(&mid) {
                fields.insert("readby".into(), rb.clone());
            }
            fields.insert("html".into(), json!(parsed.html));
            fields.insert("mentionsMe".into(), json!(parsed.mentions_me));
            fields.insert("deleted".into(), json!(deleted));
            out.push(SavedMessage { mid, author, text: parsed.text, mine, reacts, extra: extra_of(&fields) });
        }
        opt.added.retain(|(_, _, until)| now < *until);
        for (mid, msg, _) in &opt.added {
            if !out.iter().any(|x| x.mid == *mid) {
                out.push(msg.clone());
            }
        }
        Ok(Some(out))
    }

    fn added(&self, conv: &str, id: &str, msg: SavedMessage) {
        let mut o = self.optimistic.borrow_mut();
        let p = o.entry(conv.to_string()).or_default();
        p.added.retain(|(m, _, _)| m != id);
        p.added.push((id.to_string(), msg, now_ms() + 8_000));
    }

    async fn post_message(&self, http: &Http, auth: &TeamsAuth, conv: &str, body: Value, what: &str, extra: &[(&str, Field)]) -> R<Option<String>> {
        let r = self.call(http, auth, "POST", &messages_url(auth, conv), true, crate::js::stringify(&body).into_bytes(), 15_000).await?;
        if !r.ok() {
            let mut fields = vec![("error", Field::from(cut(&text(&r), 200))), ("convId", Field::from(conv))];
            fields.extend(extra.iter().cloned());
            warn("chatsvc", &format!("{what} HTTP {}", r.status), &fields);
            return Ok(None);
        }
        Ok(Some(sent_id(&json_or_empty(&r))))
    }

    pub async fn send_text(&self, http: &Http, auth: &TeamsAuth, conv: &str, text: &str) -> R<Option<String>> {
        let html = to_paragraphs(text);
        let body = json!({ "content": html, "messagetype": "RichText/Html", "contenttype": "Text", "clientmessageid": client_message_id() });
        let Some(id) = self.post_message(http, auth, conv, body, "sendTextMessage", &[]).await? else { return Ok(None) };
        let mut extra = Map::new();
        extra.insert("status".into(), json!("Sent"));
        extra.insert("html".into(), json!(html));
        self.added(
            conv,
            &id,
            SavedMessage { mid: id.clone(), author: String::new(), text: text.into(), mine: true, reacts: String::new(), extra: extra_of(&extra) },
        );
        Ok(Some(id))
    }

    /// A message with people tagged: each mention as the Mention span Teams writes, its person named in
    /// properties.mentions by mri; a mention whose person is unknown goes as plain "@Name".
    pub async fn send_mentions(&self, http: &Http, auth: &TeamsAuth, conv: &str, parts: &[Part], mri_of: &dyn Fn(&str) -> Option<String>) -> R<Option<String>> {
        let (mut mentions, mut html, mut plain) = (vec![], String::new(), String::new());
        for p in parts {
            match p {
                Part::Text(t) => {
                    html.push_str(&escape_html(t).replace('\n', "<br>"));
                    plain.push_str(t);
                }
                Part::Mention(name) => match mri_of(name) {
                    None => {
                        html.push_str(&escape_html(&format!("@{name}")));
                        plain.push_str(&format!("@{name}"));
                    }
                    Some(mri) => {
                        let itemid = mentions.len();
                        mentions.push(json!({ "itemid": itemid, "mri": mri, "mentionType": "person", "displayName": name }));
                        html.push_str(&format!(
                            r#"<span itemtype="http://schema.skype.com/Mention" itemscope="" itemid="{itemid}">{}</span>"#,
                            escape_html(name)
                        ));
                        plain.push_str(name);
                    }
                },
            }
        }
        if js_trim(&plain).is_empty() {
            return Ok(None);
        }
        let mut body =
            json!({ "content": format!("<p>{html}</p>"), "messagetype": "RichText/Html", "contenttype": "Text", "clientmessageid": client_message_id() });
        if !mentions.is_empty() {
            body["properties"] = json!({ "mentions": crate::js::stringify(&Value::Array(mentions)) });
        }
        self.post_message(http, auth, conv, body, "sendMentionsMessage", &[]).await
    }

    /// Author, preview and mri of a message, for the quote of a reply.
    async fn get_message(&self, http: &Http, auth: &TeamsAuth, conv: &str, mid: &str) -> Option<(String, String, String)> {
        let r = self.call(http, auth, "GET", &format!("{}/{mid}", messages_url(auth, conv)), false, vec![], 10_000).await.ok()?;
        if !r.ok() {
            return None;
        }
        let m = json(&r).ok()?;
        let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let author = [s("imdisplayname"), s("fromDisplayNameInToken")].into_iter().find(|x| !x.is_empty()).unwrap_or_else(|| "?".into());
        let from = s("from");
        let mri = if from.contains("/contacts/") { from.split("/contacts/").last().unwrap_or("").to_string() } else { String::new() };
        let preview = cut(js_trim(&strip_tags(&s("content"))), 150);
        Some((author, preview, mri))
    }

    pub async fn reply_text(&self, http: &Http, auth: &TeamsAuth, conv: &str, mid: &str, text: &str) -> R<Option<String>> {
        let (author, preview, mri) = self.get_message(http, auth, conv, mid).await.unwrap_or_else(|| ("?".into(), String::new(), String::new()));
        let quote = format!(
            r#"<blockquote itemscope itemtype="http://schema.skype.com/Reply" itemid="{mid}"><strong itemprop="mri" itemid="{}">{}</strong><span itemprop="time" itemid="{mid}"></span><p itemprop="preview">{}</p></blockquote>"#,
            escape_html(&mri),
            escape_html(&author),
            escape_html(&preview)
        );
        let text_html = to_paragraphs(text);
        let as_number = mid.parse::<f64>().ok().filter(|n| *n != 0.0).map(crate::web::num).unwrap_or_else(|| json!(mid));
        let body = json!({
            "content": format!("{quote}{text_html}"),
            "messagetype": "RichText/Html",
            "contenttype": "Text",
            "clientmessageid": client_message_id(),
            "properties": { "qtdMsgs": [{ "messageId": as_number, "sender": mri, "time": as_number, "message": null, "validationResult": "Valid", "sharedRefId": null, "replyChainId": null }] },
        });
        let Some(id) = self.post_message(http, auth, conv, body, "replyTextMessage", &[("mid", Field::from(mid))]).await? else { return Ok(None) };
        let mut extra = Map::new();
        extra.insert("quote".into(), json!({ "author": author, "text": preview }));
        extra.insert("status".into(), json!("Sent"));
        extra.insert("html".into(), json!(text_html));
        self.added(
            conv,
            &id,
            SavedMessage { mid: id.clone(), author: String::new(), text: text.into(), mine: true, reacts: String::new(), extra: extra_of(&extra) },
        );
        Ok(Some(id))
    }

    pub async fn edit_text(&self, http: &Http, auth: &TeamsAuth, conv: &str, mid: &str, text: &str) -> R<bool> {
        let html = to_paragraphs(text);
        let body = json!({ "content": html, "messagetype": "RichText/Html", "contenttype": "Text" });
        let r = self.call(http, auth, "PUT", &format!("{}/{mid}", messages_url(auth, conv)), true, crate::js::stringify(&body).into_bytes(), 15_000).await?;
        if !r.ok() {
            warn(
                "chatsvc",
                &format!("editTextMessage HTTP {}", r.status),
                &[("error", cut(&super::http::text(&r), 200).into()), ("convId", conv.into()), ("mid", mid.into())],
            );
            return Ok(false);
        }
        self.optimistic.borrow_mut().entry(conv.into()).or_default().edited.insert(mid.into(), (text.into(), html, now_ms() + 8_000));
        Ok(true)
    }

    pub async fn delete_message(&self, http: &Http, auth: &TeamsAuth, conv: &str, mid: &str) -> R<bool> {
        let r = self.call(http, auth, "DELETE", &format!("{}/{mid}", messages_url(auth, conv)), false, vec![], 15_000).await?;
        if !r.ok() {
            warn(
                "chatsvc",
                &format!("deleteRemoteMessage HTTP {}", r.status),
                &[("error", cut(&super::http::text(&r), 200).into()), ("convId", conv.into()), ("mid", mid.into())],
            );
            return Ok(false);
        }
        self.optimistic.borrow_mut().entry(conv.into()).or_default().deleted.insert(mid.into(), now_ms() + 8_000);
        Ok(true)
    }

    /// Undo of a delete, as Teams does it: the deletetime property goes.
    pub async fn undelete_message(&self, http: &Http, auth: &TeamsAuth, conv: &str, mid: &str) -> R<bool> {
        let r = self.call(http, auth, "DELETE", &format!("{}/{mid}/properties?name=deletetime", messages_url(auth, conv)), false, vec![], 15_000).await?;
        if !r.ok() {
            warn(
                "chatsvc",
                &format!("undeleteRemoteMessage HTTP {}", r.status),
                &[("error", cut(&super::http::text(&r), 200).into()), ("convId", conv.into()), ("mid", mid.into())],
            );
            return Ok(false);
        }
        self.optimistic.borrow_mut().entry(conv.into()).or_default().deleted.remove(mid);
        Ok(true)
    }

    pub async fn toggle_reaction(&self, http: &Http, auth: &TeamsAuth, conv: &str, mid: &str, emoji_or_name: &str) -> bool {
        let e = js_trim(emoji_or_name);
        let Some(key) = emoji_key(&e.to_lowercase()).or_else(|| emoji_key(e)) else { return false };
        let Some(oid) = auth.oid.clone().filter(|o| !o.is_empty()) else { return false };
        let msg_url = format!("{}/{mid}", messages_url(auth, conv));
        let run = async {
            let r = self.call(http, auth, "GET", &msg_url, false, vec![], 10_000).await?;
            if !r.ok() {
                return Ok(false);
            }
            let msg = json(&r)?;
            let emotions = match msg.get("properties").and_then(|p| p.get("emotions")) {
                Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(json!([])),
                Some(v) => v.clone(),
                None => json!([]),
            };
            let entry = emotions.as_array().into_iter().flatten().find(|x| crate::js::string(Some(x.get("key").unwrap_or(&json!("")))).to_lowercase() == key);
            let mine = entry
                .and_then(|x| x.get("users"))
                .and_then(Value::as_array)
                .is_some_and(|u| u.iter().any(|u| crate::js::string(Some(u.get("mri").unwrap_or(&json!("")))).to_lowercase().contains(&oid.to_lowercase())));
            let (method, body) =
                if mine { ("DELETE", json!({ "emotions": { "key": key } })) } else { ("PUT", json!({ "emotions": { "key": key, "value": now_ms() } })) };
            let r =
                self.call(http, auth, method, &format!("{msg_url}/properties?name=emotions"), true, crate::js::stringify(&body).into_bytes(), 12_000).await?;
            Ok::<bool, Fail>(r.ok())
        };
        match run.await {
            Ok(ok) => ok,
            Err(e) => {
                warn(
                    "chatsvc",
                    &format!("toggleMessageReaction failed: {}", super::text_of(&e)),
                    &[("convId", conv.into()), ("mid", mid.into()), ("key", key.into())],
                );
                false
            }
        }
    }

    async fn upload_ams_image(http: &Http, auth: &TeamsAuth, data: &[u8], mime: &str) -> Option<String> {
        let run = async {
            let h = |ct: &str| {
                vec![("User-Agent", AMS_USER_AGENT.to_string()), ("Authorization", format!("Bearer {}", auth.token)), ("Content-Type", ct.to_string())]
            };
            let meta = json!({ "type": "pish/image", "permissions": { "users/ME": ["read"] } });
            let r =
                send(http, "POST", "https://eu-api.asm.skype.com/v1/objects", h("application/json"), crate::js::stringify(&meta).into_bytes(), 15_000).await?;
            if !r.ok() {
                return Ok(None);
            }
            let id = crate::js::string(json(&r)?.get("id"));
            let r = send(http, "PUT", &format!("https://eu-api.asm.skype.com/v1/objects/{id}/content/imgpsh"), h(mime), data.to_vec(), 30_000).await?;
            Ok::<Option<String>, Fail>(r.ok().then_some(id))
        };
        match run.await {
            Ok(id) => id,
            Err(e) => {
                warn("chatsvc", &format!("uploadAmsImage failed: {}", super::text_of(&e)), &[]);
                None
            }
        }
    }

    pub async fn send_image(&self, http: &Http, auth: &TeamsAuth, conv: &str, data: &[u8], mime: &str, caption: &str) -> R<bool> {
        let Some(obj) = Self::upload_ams_image(http, auth, data, mime).await else { return Ok(false) };
        let ext = mime.split('/').next_back().filter(|e| !e.is_empty()).unwrap_or("png");
        let img = format!(
            r#"<p><img src="https://eu-api.asm.skype.com/v1/objects/{obj}/views/imgo" itemtype="http://schema.skype.com/AMSImage" itemscope="{ext}" alt="immagine" id="x_{obj}" itemid="{obj}"></p>"#
        );
        let cap = if js_trim(caption).is_empty() { String::new() } else { to_paragraphs(caption) };
        let body = json!({ "content": format!("{img}{cap}"), "messagetype": "RichText/Html", "contenttype": "Text", "clientmessageid": client_message_id(), "amsreferences": [obj] });
        let r = self.call(http, auth, "POST", &messages_url(auth, conv), true, crate::js::stringify(&body).into_bytes(), 15_000).await?;
        Ok(r.ok())
    }

    /// The read mark of a conversation (consumption horizon) at its last message.
    pub async fn mark_read(&self, http: &Http, auth: &TeamsAuth, conv: &str, fallback_mid: Option<&str>) -> bool {
        let run = async {
            let mut last = self.last.borrow().get(conv).cloned();
            if last.is_none() {
                let r = self.call(http, auth, "GET", &format!("{}?view=msnp24Equivalent&pageSize=1", messages_url(auth, conv)), false, vec![], 10_000).await?;
                if r.ok()
                    && let Some(m) = json(&r)?.get("messages").and_then(|m| m.get(0)).cloned()
                    && m.get("id").is_some_and(crate::js::truthy)
                {
                    let id = crate::js::string(m.get("id"));
                    let cmid = m.get("clientmessageid").filter(|v| crate::js::truthy(v)).map_or(id.clone(), |v| crate::js::string(Some(v)));
                    last = Some((id, cmid));
                }
            }
            let mid = last
                .as_ref()
                .map(|l| l.0.clone())
                .filter(|m| !m.is_empty())
                .or_else(|| fallback_mid.filter(|m| !m.is_empty()).map(String::from))
                .unwrap_or_else(|| now_ms().to_string());
            let cmid = last.map(|l| l.1).filter(|c| !c.is_empty()).unwrap_or_else(|| mid.clone());
            let body = json!({ "consumptionhorizon": format!("{mid};{};{cmid}", now_ms()) });
            let url = format!(
                "{SERVER}/api/chatsvc/{}/v1/users/ME/conversations/{}/properties?name=consumptionhorizon",
                auth.region,
                crate::js::encode_uri_component(conv)
            );
            let r = self.call(http, auth, "PUT", &url, true, crate::js::stringify(&body).into_bytes(), 10_000).await?;
            Ok::<bool, Fail>(r.ok())
        };
        match run.await {
            Ok(ok) => ok,
            Err(e) => {
                warn("chatsvc", &format!("markConversationRead failed: {}", super::text_of(&e)), &[("convId", conv.into())]);
                false
            }
        }
    }
}

use super::Field;

/// A part of a message with people tagged (MentionArgs).
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Mention(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::{Net, run};

    fn auth() -> TeamsAuth {
        TeamsAuth { token: "fake_token_123".into(), region: "emea".into(), ..Default::default() }
    }

    fn none(_: &[String]) -> HashMap<String, Value> {
        HashMap::new()
    }

    #[test]
    fn sends_replies_edits_deletes_and_undeletes() {
        let net = Net::new();
        let svc = ChatSvc::default();
        net.answer(|s| {
            if s.url.ends_with("/messages/12345") && s.method == "GET" {
                (200, r#"{"imdisplayname":"Mario","content":"<p>Original</p>","from":"contacts/8:orgid:mri-1"}"#.into())
            } else {
                (201, r#"{"OriginalArrivalTime":1793300000000}"#.into())
            }
        });
        assert_eq!(run(svc.send_text(&net.http(), &auth(), "19:test_chat", "Hello from test")).expect("sent").as_deref(), Some("1793300000000"));
        let s = &net.requests()[0];
        assert!(s.url.contains("/users/ME/conversations/19%3Atest_chat/messages"));
        assert_eq!(s.header("Authorization"), Some("Bearer fake_token_123"));
        assert!(s.text().contains("<p>Hello from test</p>"));
        net.clear();
        run(svc.reply_text(&net.http(), &auth(), "19:test_chat", "12345", "Risposta di test")).expect("replied");
        let body = net.requests()[1].text();
        assert!(body.contains("schema.skype.com/Reply") && body.contains("Original") && body.contains("Risposta di test"));
        assert_eq!(net.requests()[1].json()["properties"]["qtdMsgs"][0]["messageId"], json!(12345));
        net.clear();
        assert!(run(svc.edit_text(&net.http(), &auth(), "19:test_chat", "12345", "Modificato")).expect("edited"));
        assert_eq!(net.requests()[0].method, "PUT");
        assert!(run(svc.delete_message(&net.http(), &auth(), "19:test_chat", "12345")).expect("deleted"));
        assert!(run(svc.undelete_message(&net.http(), &auth(), "19:test_chat", "12345")).expect("back"));
        let last = net.requests().last().cloned().expect("request");
        assert_eq!(
            (last.method.as_str(), last.url.as_str()),
            ("DELETE", "https://teams.microsoft.com/api/chatsvc/emea/v1/users/ME/conversations/19%3Atest_chat/messages/12345/properties?name=deletetime")
        );
    }

    #[test]
    fn uploads_an_image_to_ams_and_sends_it() {
        let net = Net::new();
        net.answer(|s| {
            if s.url.ends_with("/objects") {
                (200, r#"{"id":"ams-obj-999"}"#.into())
            } else if s.url.contains("/content/imgpsh") {
                (201, String::new())
            } else {
                (201, r#"{"OriginalArrivalTime":1793300002000}"#.into())
            }
        });
        assert!(run(ChatSvc::default().send_image(&net.http(), &auth(), "19:test_chat", b"fake-png-data", "image/png", "Didascalia")).expect("sent"));
        assert_eq!(net.requests().len(), 3);
        let body = net.requests()[2].text();
        assert!(body.contains("ams-obj-999") && body.contains("Didascalia"));
    }

    #[test]
    fn reads_messages_with_quotes_emojis_reactions_edits_and_deletes() {
        let net = Net::new();
        let messages = json!({ "messages": [
            {
                "id": "1002", "messagetype": "RichText/Html", "imdisplayname": "Daniele Fortunio",
                "from": "https://teams.microsoft.com/api/chatsvc/emea/v1/users/ME/contacts/8:orgid:other-uid",
                "content": "<blockquote itemscope itemtype=\"http://schema.skype.com/Reply\" itemid=\"1001\"><strong itemprop=\"mri\" itemid=\"8:orgid:my-oid\">Samuele</strong><p itemprop=\"preview\">Ciao!</p></blockquote><p>Tutto bene <img itemtype=\"http://schema.skype.com/Emoji\" alt=\"\u{1f600}\"></p>",
                "properties": { "edittime": 1793000005000u64, "emotions": [{ "key": "like", "users": [{ "mri": "8:orgid:my-oid" }, { "mri": "8:orgid:other-uid" }] }] },
            },
            { "id": "1001", "messagetype": "RichText/Html", "imdisplayname": "Samuele Maritato", "from": "https://teams.microsoft.com/api/chatsvc/emea/v1/users/ME/contacts/8:orgid:my-oid", "content": "<p>Ciao!</p>", "properties": {} },
            { "id": "1003", "messagetype": "RichText/Html", "imdisplayname": "Samuele Maritato", "from": "https://teams.microsoft.com/api/chatsvc/emea/v1/users/ME/contacts/8:orgid:my-oid", "content": "", "properties": { "deletetime": 1793000009000u64 } },
            { "id": "1004", "messagetype": "ThreadActivity/AddMember", "content": "x" },
        ]});
        net.answer(move |_| (200, messages.to_string()));
        let a = TeamsAuth { oid: Some("my-oid".into()), ..auth() };
        let msgs = run(ChatSvc::default().fetch_chat_messages(&net.http(), &a, "19:daniele_1on1@unq.gbl.spaces", &none)).expect("messages");
        assert_eq!(msgs.len(), 3);
        assert_eq!((msgs[0].mid.as_str(), msgs[0].mine, msgs[0].author.as_str(), msgs[0].text.as_str()), ("1001", true, "", "Ciao!"));
        let m = &msgs[1];
        assert_eq!(
            (m.mid.as_str(), m.mine, m.author.as_str(), m.text.as_str(), m.reacts.as_str()),
            ("1002", false, "Daniele Fortunio", "Tutto bene \u{1f600}", "\u{1f44d}2")
        );
        let extra = m.extra.clone().expect("extra");
        assert_eq!((extra["quote"].clone(), extra["edited"].clone()), (json!({"author": "Samuele", "text": "Ciao!"}), json!(true)));
        assert_eq!(extra["reactions"], json!([{ "e": "\u{1f44d}", "n": 2, "mine": true }]));
        assert_eq!((msgs[2].mid.as_str(), msgs[2].mine, msgs[2].text.as_str()), ("1003", true, ""));
        assert_eq!(msgs[2].extra.as_ref().expect("extra")["deleted"], json!(true));
    }

    #[test]
    fn the_html_of_a_message() {
        let p = parse_teams_message_html(
            r#"<div><p>&nbsp;</p><p>Hi <span itemtype="http://schema.skype.com/Mention" itemid="0">Me Myself</span>,</p><p><img src="https://x/a.png" width="10.5" height="20"><img src="https://x/avatar.png"></p><p></p><p></p><p>end</p></div>"#,
            Some("me myself"),
        );
        assert!(p.mentions_me);
        assert_eq!(p.text, "Hi Me Myself,\n\nend");
        assert_eq!(p.images, [("https://x/a.png".to_string(), 11, 20)]);
        assert!(p.html.starts_with(r#"<p>Hi <span class="mn me">Me Myself</span>,</p>"#), "{}", p.html);
        assert_eq!(
            extra_of(&json!({"status": "", "images": [], "edited": false, "html": "x"}).as_object().cloned().expect("o")),
            Some(json!({"html": "x"}).as_object().cloned().expect("o"))
        );
    }
}
