//! The Activity feed of a light account as the page feed saves it (src/agent/light/activity.ts): its ids are the
//! activity ids of Teams, so the missed calls and mentions told before a change of mode stay told.

use super::api::Notification;
use super::chatsvc::reaction_emoji;
use super::list::list_time;
use super::store::ActivityEntry;
use super::{cut, js_trim, parse_date};
use regex::Regex;
use serde_json::Value;

const FEED_LIMIT: usize = 40;

/// The kind the page feed gives the item.
pub fn activity_kind(kind: &str, subtype: &str) -> &'static str {
    let has = |p: &str, s: &str| Regex::new(&format!("(?i){p}")).expect("fixed").is_match(s);
    if kind == "call" {
        "call"
    } else if kind == "mentionInChat" || kind == "mentions" || has("mention", kind) {
        "mention"
    } else if has("reaction", kind) {
        "reaction"
    } else if has("reply", kind) {
        "reply"
    } else if has("task", subtype) {
        "task"
    } else if has("meeting", subtype) || has("meeting", kind) {
        "meeting"
    } else if has("team|channel", kind) {
        "team"
    } else {
        "message"
    }
}

fn title(kind: &str, actor: &str) -> String {
    let who = if actor.is_empty() { "Someone" } else { actor };
    match kind {
        "call" => {
            if actor.is_empty() {
                "Missed call".into()
            } else {
                format!("Missed call from {actor}")
            }
        }
        "mention" => format!("{who} mentioned you"),
        "reaction" => format!("{who} reacted to your message"),
        "reply" => format!("{who} replied"),
        _ => who.to_string(),
    }
}

/// When an item came, ms.
pub fn at_of(n: &Notification) -> f64 {
    parse_date(n.activity.get("activityTimestamp").and_then(Value::as_str).unwrap_or("")).unwrap_or(0.0)
}

/// The feed of the page from the items of the service, the newest first; a chat goes by the name the chat list gives
/// it, a 1:1 chat and a call by the person, a channel by its topic.
pub fn light_feed(items: &[Notification], chat_of: &dyn Fn(&str) -> String, now_ms: f64) -> Vec<ActivityEntry> {
    let mut items: Vec<&Notification> = items.iter().filter(|n| n.activity.get("activityId").is_some_and(|v| !v.is_null() && v != &Value::from(""))).collect();
    items.sort_by(|x, y| at_of(y).partial_cmp(&at_of(x)).unwrap_or(std::cmp::Ordering::Equal));
    items
        .into_iter()
        .take(FEED_LIMIT)
        .map(|n| {
            let a = &n.activity;
            let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let kind = activity_kind(&s("activityType"), &s("activitySubtype"));
            let actor = js_trim(&s("sourceUserImDisplayName")).to_string();
            let channel = Regex::new("(?i)topic|space|channel").expect("fixed").is_match(&s("sourceThreadType"));
            let thread = s("sourceThreadId");
            let listed = if thread.is_empty() { String::new() } else { chat_of(&thread) };
            let chat = if kind == "call" {
                actor.clone()
            } else if channel {
                s("sourceThreadTopic")
            } else if !listed.is_empty() {
                listed
            } else if thread.contains("@unq.gbl.spaces") {
                actor.clone()
            } else {
                s("sourceThreadTopic")
            };
            let sub = s("activitySubtype");
            let emoji = if kind == "reaction" { reaction_emoji(if sub == "sad" { "cry" } else { &sub }).unwrap_or("").to_string() } else { String::new() };
            ActivityEntry {
                id: crate::js::string(a.get("activityId")),
                kind: kind.into(),
                title: title(kind, &actor),
                actor,
                emoji,
                preview: cut(&s("messagePreview"), 300),
                tm: list_time(at_of(n), now_ms),
                chat,
                channel,
                unread: !n.read,
                av: String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_the_ids_of_teams_the_newest_first() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: the tests that set the environment hold TEST_ENV
        unsafe { std::env::set_var("TZ", "UTC") };
        let n = |a: Value, read: bool| Notification { activity: a.as_object().cloned().expect("object"), read };
        let feed = light_feed(
            &[
                n(
                    json!({ "activityId": 1001, "activityType": "reactionInChat", "activitySubtype": "like", "activityTimestamp": "2026-11-03T09:00:00Z", "sourceThreadId": "19:g@thread.v2", "sourceUserImDisplayName": "Rossi Anna", "messagePreview": "ok" }),
                    true,
                ),
                n(
                    json!({ "activityId": 1002, "activityType": "call", "activitySubtype": "missedCall", "activityTimestamp": "2026-11-03T10:00:00Z", "sourceThreadId": "19:a@unq.gbl.spaces", "sourceUserImDisplayName": "Rossi Anna" }),
                    false,
                ),
                n(
                    json!({ "activityId": 1003, "activityType": "mentionInChat", "activitySubtype": "person", "activityTimestamp": "2026-11-03T11:00:00Z", "sourceThreadId": "19:t@thread.tacv2", "sourceThreadType": "topic", "sourceThreadTopic": "General", "sourceUserImDisplayName": "Bianchi Marco", "messagePreview": "@you look" }),
                    false,
                ),
                n(json!({ "activityType": "msGraph" }), true),
            ],
            &|id| if id == "19:g@thread.v2" { "Group chat".into() } else { String::new() },
            parse_date("2026-11-03T15:00:00Z").expect("now"),
        );
        assert_eq!(feed.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["1003", "1002", "1001"]);
        assert_eq!(
            (feed[0].kind.as_str(), feed[0].chat.as_str(), feed[0].channel, feed[0].unread, feed[0].title.as_str()),
            ("mention", "General", true, true, "Bianchi Marco mentioned you")
        );
        assert_eq!(
            (feed[1].kind.as_str(), feed[1].chat.as_str(), feed[1].title.as_str(), feed[1].unread, feed[1].tm.as_str()),
            ("call", "Rossi Anna", "Missed call from Rossi Anna", true, "10:00 AM")
        );
        assert_eq!((feed[2].kind.as_str(), feed[2].chat.as_str(), feed[2].emoji.as_str(), feed[2].unread), ("reaction", "Group chat", "\u{1f44d}", false));
        unsafe { std::env::remove_var("TZ") };
    }
}
