//! The chat list of a light account (src/agent/light/list.ts), made from the aggregator as the browser agent reads it
//! from the page: the same names, previews and times, so a chat keeps its messages and its place when an account
//! changes mode.

use super::api::{CsaChat, Person, PersonPresence};
use super::chatsvc::parse_teams_message_html;
use super::store::{CHAT_LIMIT, ChatEntry};
use super::{cut, js_trim, local_zone, parse_date};
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::HashSet;

/// "9:05 AM" on the day of `now`, "11/2" before, in the time zone of the process (TZ); "" for no time.
pub fn list_time(at_ms: f64, now_ms: f64) -> String {
    let tz = local_zone();
    let zoned = |ms: f64| jiff::Timestamp::from_millisecond(ms as i64).ok().map(|t| t.to_zoned(tz.clone()));
    let (Some(at), Some(now)) = (zoned(at_ms), zoned(now_ms)) else { return String::new() };
    if at.date() == now.date() {
        let h = at.hour();
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        return format!("{h12}:{:02} {}", at.minute(), if h < 12 { "AM" } else { "PM" });
    }
    format!("{}/{}", at.month(), at.day())
}

/// A presence of the API as one of the words of the app.
pub fn presence_word(p: Option<&PersonPresence>) -> &'static str {
    let Some(p) = p else { return "" };
    if p.activity.to_lowercase().contains("outofoffice") {
        return "ooo";
    }
    match p.availability.as_str() {
        "Available" | "AvailableIdle" => "available",
        "Busy" | "BusyIdle" => {
            let a = p.activity.to_lowercase();
            if a.contains("presenting") || a.contains("focusing") { "dnd" } else { "busy" }
        }
        "DoNotDisturb" => "dnd",
        "Away" | "BeRightBack" => "away",
        "Offline" => "offline",
        _ => "",
    }
}

fn s<'a>(c: &'a Value, path: &[&str]) -> &'a str {
    let mut v = c;
    for k in path {
        match v.get(*k) {
            Some(x) => v = x,
            None => return "",
        }
    }
    v.as_str().unwrap_or("")
}

fn flag(c: &Value, k: &str) -> Option<bool> {
    c.get(k).and_then(Value::as_bool)
}

/// When the last message of a chat came, ms (0 unknown).
fn arrival(c: &Value) -> f64 {
    let t = [s(c, &["lastMessage", "originalArrivalTime"]), s(c, &["lastMessage", "composeTime"]), s(c, &["createdAt"])]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or("");
    parse_date(t).unwrap_or(0.0)
}

/// The chats the list shows: with a message, not deleted, the newest first, as many as the page list keeps.
pub fn listed_chats(chats: &[CsaChat], limit: usize) -> Vec<CsaChat> {
    let mut out: Vec<CsaChat> = chats
        .iter()
        .filter(|c| flag(c, "isConversationDeleted") != Some(true) && c.get("lastMessage").and_then(|m| m.get("id")).is_some_and(crate::js::truthy))
        .cloned()
        .collect();
    out.sort_by(|x, y| arrival(y).partial_cmp(&arrival(x)).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    out
}

/// The other people of a chat (mri), you left out.
pub fn others_of(c: &CsaChat, me: &str) -> Vec<String> {
    let me = me.to_lowercase();
    c.get("members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| s(m, &["mri"]).to_string())
        .filter(|mri| !mri.is_empty() && !mri.to_lowercase().contains(&me))
        .collect()
}

fn title(c: &CsaChat) -> &str {
    js_trim(s(c, &["title"]))
}

/// People whose name a chat needs and nobody gave yet.
pub fn unnamed_people(chats: &[CsaChat], me: &str, names: &IndexMap<String, Person>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for c in chats {
        if !title(c).is_empty() {
            continue;
        }
        let n = if flag(c, "isOneOnOne") == Some(true) { 1 } else { 4 };
        for mri in others_of(c, me).into_iter().take(n) {
            if !names.contains_key(&mri) && !out.contains(&mri) {
                out.push(mri);
            }
        }
    }
    out
}

/// The chats a name of a person is still missing for.
pub fn unnamed_chats(chats: &[CsaChat], me: &str, names: &IndexMap<String, Person>) -> Vec<CsaChat> {
    chats.iter().filter(|c| chat_name(c, me, names).is_empty()).cloned().collect()
}

/// How the list names a chat; "" while a name it needs is unknown.
pub fn chat_name(c: &CsaChat, me: &str, names: &IndexMap<String, Person>) -> String {
    if !title(c).is_empty() {
        return title(c).to_string();
    }
    let others = others_of(c, me);
    if flag(c, "isOneOnOne") == Some(true) {
        if let Some(p) = others.first().and_then(|o| names.get(o)) {
            return p.display_name.clone();
        }
        return if flag(c, "isLastMessageFromMe") != Some(true) { js_trim(s(c, &["lastMessage", "imDisplayName"])).to_string() } else { String::new() };
    }
    let shown: Vec<Option<&String>> = others.iter().take(4).map(|m| names.get(m).map(|p| &p.given_name)).collect();
    if shown.is_empty() || shown.iter().any(|n| n.is_none_or(|n| n.is_empty())) {
        return String::new();
    }
    let more = others.len() as i64 - shown.len() as i64;
    let joined = shown.into_iter().map(|n| n.cloned().unwrap_or_default()).collect::<Vec<_>>().join(", ");
    if more > 0 { format!("{joined}, +{more}") } else { joined }
}

pub fn chat_kind(c: &CsaChat) -> &'static str {
    if flag(c, "isOneOnOne") == Some(true) {
        "one"
    } else if s(c, &["chatType"]) == "meeting" {
        "meeting"
    } else {
        "group"
    }
}

/// The first line of a message, short.
fn first_line(content: &str) -> String {
    js_trim(parse_teams_message_html(content, None).text.split('\n').next().unwrap_or("")).to_string()
}

/// The text of the last message as the list previews it: first line, "You: " for your own.
pub fn preview_of(c: &CsaChat) -> String {
    let text = first_line(s(c, &["lastMessage", "content"]));
    if text.is_empty() {
        return String::new();
    }
    cut(&if flag(c, "isLastMessageFromMe") == Some(true) { format!("You: {text}") } else { text }, 200)
}

pub struct ListInput<'a> {
    pub me: &'a str,
    pub names: &'a IndexMap<String, Person>,
    pub presence: &'a IndexMap<String, PersonPresence>,
    /// conversations with a mention of you not read yet
    pub mentioned: &'a HashSet<String>,
    pub now_ms: f64,
    /// the chat with yourself, which the aggregator does not list: its name, last message and time
    pub notes: Option<(String, String, String)>,
}

/// The rows of the list and, by chat name, its conversation id.
pub fn light_list(chats: &[CsaChat], o: &ListInput) -> (Vec<ChatEntry>, IndexMap<String, String>) {
    let mut ids: IndexMap<String, String> = IndexMap::new();
    let mut entries: Vec<(f64, ChatEntry)> = vec![];
    let limit = if o.notes.is_some() { CHAT_LIMIT - 1 } else { CHAT_LIMIT };
    for c in listed_chats(chats, limit) {
        let name = chat_name(&c, o.me, o.names);
        // a name another chat of the list already took: the first (newest) keeps it
        if name.is_empty() || ids.contains_key(&name) {
            continue;
        }
        ids.insert(name.clone(), s(&c, &["id"]).to_string());
        let other = if flag(&c, "isOneOnOne") == Some(true) { others_of(&c, o.me).into_iter().next() } else { None };
        let at = arrival(&c);
        entries.push((
            at,
            ChatEntry {
                name,
                preview: preview_of(&c),
                time: list_time(at, o.now_ms),
                unread: flag(&c, "isRead") == Some(false) && flag(&c, "isLastMessageFromMe") != Some(true),
                mention: o.mentioned.contains(s(&c, &["id"])),
                muted: flag(&c, "isMuted") == Some(true),
                av: String::new(),
                presence: other.map_or("", |m| presence_word(o.presence.get(&m))).to_string(),
                kind: chat_kind(&c).to_string(),
            },
        ));
    }
    if let Some((name, content, time)) = &o.notes {
        let at = parse_date(time).unwrap_or(0.0);
        ids.insert(name.clone(), "48:notes".into());
        let text = first_line(content);
        let preview = if text.is_empty() { String::new() } else { cut(&format!("You: {text}"), 200) };
        entries.push((at, ChatEntry { name: name.clone(), preview, time: list_time(at, o.now_ms), kind: "one".into(), ..Default::default() }));
    }
    entries.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    (entries.into_iter().map(|e| e.1).collect(), ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "aaaa-me";

    fn mri(oid: &str) -> String {
        format!("8:orgid:{oid}")
    }

    fn rome_ms(d: u32, h: i8, m: i8) -> f64 {
        jiff::civil::date(2026, 11, d as i8)
            .at(h, m, 0, 0)
            .to_zoned(jiff::tz::TimeZone::get("Europe/Rome").expect("zone"))
            .expect("zoned")
            .timestamp()
            .as_millisecond() as f64
    }

    fn iso(ms: f64) -> String {
        jiff::Timestamp::from_millisecond(ms as i64).expect("ts").to_string()
    }

    fn names() -> IndexMap<String, Person> {
        let p = |d: &str, g: &str| Person { display_name: d.into(), given_name: g.into() };
        IndexMap::from([(mri("anna"), p("Rossi Anna", "Anna")), (mri("marco"), p("Bianchi Marco", "Marco")), (mri("luca"), p("Verdi Luca", "Luca"))])
    }

    fn chat(extra: Value) -> Value {
        let mut c =
            json!({ "members": [{ "mri": mri(ME) }], "lastMessage": { "id": "1", "content": "<p>hi</p>", "originalArrivalTime": iso(rome_ms(3, 9, 0)) } });
        for (k, v) in extra.as_object().expect("object") {
            c[k] = v.clone();
        }
        c
    }

    #[test]
    fn times_and_presence() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: the tests that set the environment hold TEST_ENV
        unsafe { std::env::set_var("TZ", "Europe/Rome") };
        let now = rome_ms(3, 15, 0);
        assert_eq!(list_time(rome_ms(3, 9, 5), now), "9:05 AM");
        assert_eq!(list_time(rome_ms(3, 0, 30), now), "12:30 AM");
        assert_eq!(list_time(rome_ms(3, 14, 7), now), "2:07 PM");
        assert_eq!(list_time(rome_ms(2, 14, 7), now), "11/2");
        let p = |a: &str, x: &str| presence_word(Some(&PersonPresence { availability: a.into(), activity: x.into() }));
        assert_eq!(
            [
                p("Available", "Available"),
                p("Busy", "InACall"),
                p("Busy", "Presenting"),
                p("DoNotDisturb", ""),
                p("BeRightBack", ""),
                p("Offline", "OffWork"),
                p("Available", "OutOfOffice"),
                p("PresenceUnknown", "")
            ],
            ["available", "busy", "dnd", "dnd", "away", "offline", "ooo", ""]
        );

        let (rows, ids) = light_list(
            &[
                chat(
                    json!({ "id": "19:old@thread.v2", "title": "Old", "lastMessage": { "id": "1", "content": "<p>old</p>", "originalArrivalTime": iso(rome_ms(1, 10, 0)) } }),
                ),
                chat(
                    json!({ "id": "19:new@thread.v2", "title": "New", "isRead": false, "isMuted": true, "chatType": "meeting", "lastMessage": { "id": "2", "content": "<p>line one</p><p>line two</p>", "originalArrivalTime": iso(rome_ms(3, 14, 30)) } }),
                ),
                chat(
                    json!({ "id": "19:mine@thread.v2", "title": "Mine", "isRead": false, "isLastMessageFromMe": true, "lastMessage": { "id": "3", "content": "<p>sent</p>", "originalArrivalTime": iso(rome_ms(3, 11, 0)) } }),
                ),
                chat(
                    json!({ "id": "19:dup@thread.v2", "title": "New", "lastMessage": { "id": "4", "content": "x", "originalArrivalTime": iso(rome_ms(3, 8, 0)) } }),
                ),
                chat(json!({ "id": "19:gone@thread.v2", "title": "Gone", "isConversationDeleted": true })),
                chat(json!({ "id": "19:empty@thread.v2", "title": "Empty", "lastMessage": null })),
            ],
            &ListInput {
                me: ME,
                names: &names(),
                presence: &IndexMap::new(),
                mentioned: &HashSet::from(["19:new@thread.v2".to_string()]),
                now_ms: now,
                notes: Some(("Me Myself (You)".into(), "<p>note</p>".into(), iso(rome_ms(3, 12, 0)))),
            },
        );
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["New", "Me Myself (You)", "Mine", "Old"]);
        assert_eq!(
            (rows[0].preview.as_str(), rows[0].time.as_str(), rows[0].unread, rows[0].mention, rows[0].muted, rows[0].kind.as_str()),
            ("line one", "2:30 PM", true, true, true, "meeting")
        );
        assert_eq!((rows[1].preview.as_str(), rows[1].unread), ("You: note", false));
        assert_eq!((rows[2].preview.as_str(), rows[2].unread, rows[2].kind.as_str()), ("You: sent", false, "group"));
        assert_eq!(rows[3].time, "11/1");
        assert_eq!((ids["New"].as_str(), ids["Me Myself (You)"].as_str()), ("19:new@thread.v2", "48:notes"));
        unsafe { std::env::remove_var("TZ") };
    }

    #[test]
    fn names_of_chats() {
        let one = chat(
            json!({ "id": "19:a@unq.gbl.spaces", "isOneOnOne": true, "isLastMessageFromMe": true, "members": [{ "mri": mri(ME) }, { "mri": mri("anna") }] }),
        );
        assert_eq!(chat_name(&one, ME, &names()), "Rossi Anna");
        assert_eq!(chat_name(&one, ME, &IndexMap::new()), "");
        let mut from_other = one.clone();
        from_other["isLastMessageFromMe"] = json!(false);
        from_other["lastMessage"]["imDisplayName"] = json!("Rossi Anna");
        assert_eq!(chat_name(&from_other, ME, &IndexMap::new()), "Rossi Anna");
        let members: Vec<Value> = [ME, "anna", "marco", "luca", "x1", "x2"].iter().map(|m| json!({ "mri": mri(m) })).collect();
        let group = chat(json!({ "id": "19:g@thread.v2", "members": members }));
        assert_eq!(unnamed_people(std::slice::from_ref(&group), ME, &names()), [mri("x1")]);
        let mut more = names();
        more.insert(mri("x1"), Person { display_name: "X Uno".into(), given_name: "Uno".into() });
        assert_eq!(chat_name(&group, ME, &more), "Anna, Marco, Luca, Uno, +1");
        let mut titled = group;
        titled["title"] = json!(" Release ");
        assert_eq!(chat_name(&titled, ME, &IndexMap::new()), "Release");
    }
}
