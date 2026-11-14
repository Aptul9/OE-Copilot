//! New messages from the chat list, independent of the notifications of Teams (src/agent/logic/new-messages.ts). Main
//! signal: preview or time of a chat change to an incoming text the chat did not show before; fallback: the chat turns
//! unread. The first list only primes the state; the chat with yourself and muted chats never notify, a muted chat with
//! something new is noted apart (Teams mutes its messages, not the @mentions of you).

use super::js_trim;
use super::store::ChatEntry;
use regex::Regex;
use std::collections::HashMap;

/// Last messages of a chat the list showed, kept to tell one it shows again from a new one.
const SEEN_PER_CHAT: usize = 8;

/// The chat with yourself ("Name (You)").
pub fn is_self_chat(name: &str) -> bool {
    name.to_lowercase().contains("(you)")
}

#[derive(Default)]
pub struct Detector {
    primed: bool,
    last: HashMap<String, (String, String)>,
    seen: HashMap<String, Vec<(String, String)>>,
    unread: HashMap<String, bool>,
    last_notified: HashMap<String, String>,
    muted_news: Vec<String>,
}

impl Detector {
    /// The new messages of this list: chat and body.
    pub fn scan(&mut self, chats: &[ChatEntry]) -> Vec<(String, String)> {
        let outbound = Regex::new("(?i)^(you|tu):").expect("fixed");
        let mut out = vec![];
        for ch in chats {
            if ch.name.is_empty() {
                continue;
            }
            let preview = js_trim(&ch.preview).to_string();
            let sig = format!("{preview}|{}", ch.time);
            if self.primed && !is_self_chat(&ch.name) {
                let inbound = !preview.is_empty() && !outbound.is_match(&preview);
                let changed = inbound && self.last.contains_key(&ch.name) && !self.shown(&ch.name, &preview, &ch.time);
                let became_unread = self.unread.get(&ch.name) != Some(&true) && ch.unread;
                if changed || became_unread {
                    let key = format!("{sig}|{}", if ch.unread { "u" } else { "r" });
                    if self.last_notified.get(&ch.name) != Some(&key) {
                        if ch.muted {
                            self.muted_news.push(ch.name.clone());
                        } else {
                            out.push((ch.name.clone(), if inbound { preview.clone() } else { format!("New message from {}", ch.name) }));
                        }
                        self.last_notified.insert(ch.name.clone(), key);
                    }
                }
            }
            self.last.insert(ch.name.clone(), (preview.clone(), ch.time.clone()));
            if !preview.is_empty() {
                let seen = self.seen.entry(ch.name.clone()).or_default();
                seen.retain(|s| s.0 != preview || s.1 != ch.time);
                seen.push((preview, ch.time.clone()));
                if seen.len() > SEEN_PER_CHAT {
                    seen.remove(0);
                }
            }
            self.unread.insert(ch.name.clone(), ch.unread);
        }
        self.primed = true;
        out
    }

    /// Muted chats with something new since the last call.
    pub fn take_muted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.muted_news)
    }

    /// The chat showed this message before: same text at the same time, or at the time it had before it turned into a
    /// date.
    fn shown(&self, chat: &str, preview: &str, time: &str) -> bool {
        let clock = Regex::new(r"(?i)^\d{1,2}:\d{2}\s?(AM|PM)?$").expect("fixed");
        let date = Regex::new(r"^\d{1,2}/\d{1,2}$").expect("fixed");
        self.seen.get(chat).into_iter().flatten().any(|(p, t)| p == preview && (t == time || (clock.is_match(t) && date.is_match(time))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, preview: &str, time: &str, unread: bool, muted: bool) -> ChatEntry {
        ChatEntry { name: name.into(), preview: preview.into(), time: time.into(), unread, muted, ..Default::default() }
    }

    fn r(name: &str, preview: &str, time: &str) -> ChatEntry {
        row(name, preview, time, false, false)
    }

    fn primed(chats: &[ChatEntry]) -> Detector {
        let mut d = Detector::default();
        assert!(d.scan(chats).is_empty());
        d
    }

    fn new(chat: &str, body: &str) -> (String, String) {
        (chat.into(), body.into())
    }

    #[test]
    fn notifies_incoming_previews_and_chats_turning_unread() {
        assert!(Detector::default().scan(&[row("Anna Rossi", "ciao", "10:30", true, false)]).is_empty());
        let mut d = primed(&[r("Anna Rossi", "ciao", "10:30")]);
        assert_eq!(d.scan(&[r("Anna Rossi", "are you there?", "10:31")]), [new("Anna Rossi", "are you there?")]);
        assert!(d.scan(&[r("Anna Rossi", "are you there?", "10:31")]).is_empty());
        let mut d = primed(&[r("Anna Rossi", "ciao", "10:30"), r("Luca Bianchi", "ok", "9:00")]);
        assert!(d.scan(&[r("Anna Rossi", "You: on my way", "10:32"), r("Luca Bianchi", "Tu: va bene", "10:33")]).is_empty());
        let mut d = primed(&[r("Project Alpha", "You: deployed", "9:58")]);
        assert_eq!(d.scan(&[row("Project Alpha", "You: deployed", "9:58", true, false)]), [new("Project Alpha", "New message from Project Alpha")]);
        let mut d = primed(&[r("Anna Rossi", "ciao", "10:30")]);
        assert_eq!(d.scan(&[row("Marco Neri", "hello", "10:40", true, false), r("Sara Gialli", "hi", "10:41")]), [new("Marco Neri", "hello")]);
    }

    #[test]
    fn never_the_self_chat_and_a_muted_chat_apart_once() {
        let mut d = primed(&[r("Anna Rossi (You)", "notes", "9:00"), row("Release notes", "build 1.4.1", "9:00", false, true)]);
        assert!(d.take_muted().is_empty());
        assert!(d.scan(&[row("Anna Rossi (You)", "more notes", "9:05", true, false), row("Release notes", "build 1.4.2", "9:06", true, true)]).is_empty());
        assert_eq!(d.take_muted(), ["Release notes"]);
        assert!(d.take_muted().is_empty());
        d.scan(&[row("Release notes", "build 1.4.2", "9:06", true, true)]);
        assert!(d.take_muted().is_empty());
    }

    #[test]
    fn a_message_shown_again_is_not_new() {
        let mut d = primed(&[r("Anna Rossi", "ciao", "10:30")]);
        assert_eq!(d.scan(&[row("Anna Rossi", "ciao", "10:30", true, false)]).len(), 1);
        assert!(d.scan(&[r("Anna Rossi", "ciao", "10:30")]).is_empty());
        assert!(d.scan(&[row("Anna Rossi", "ciao", "10:30", true, false)]).is_empty());
        let mut d = primed(&[r("Anna Rossi", "are you there?", "6:18 PM"), r("Luca Bianchi", "deploy is green", "18:05")]);
        assert!(d.scan(&[r("Anna Rossi", "are you there?", "9/26"), r("Luca Bianchi", "deploy is green", "9/26")]).is_empty());
        let mut d = primed(&[r("MOSSO Diego", "ciao", "2:20 PM")]);
        assert_eq!(d.scan(&[row("MOSSO Diego", "cosa intendi?", "2:22 PM", true, false)]), [new("MOSSO Diego", "cosa intendi?")]);
        assert!(d.scan(&[r("MOSSO Diego", "cosa intendi?", "2:22 PM")]).is_empty());
        assert!(d.scan(&[r("MOSSO Diego", "", "")]).is_empty());
        assert!(d.scan(&[r("MOSSO Diego", "cosa intendi?", "2:22 PM")]).is_empty());
        let mut d = primed(&[r("Anna Rossi", "ciao", "10:30")]);
        assert_eq!(d.scan(&[r("Anna Rossi", "are you there?", "10:31")]).len(), 1);
        assert!(d.scan(&[r("Anna Rossi", "ciao", "10:30")]).is_empty());
        let mut d = primed(&[r("Anna Rossi", "ciao", "10:30")]);
        assert!(d.scan(&[r("Anna Rossi", "", "")]).is_empty());
        assert_eq!(d.scan(&[r("Anna Rossi", "are you there?", "10:31")]), [new("Anna Rossi", "are you there?")]);
        let mut d = primed(&[r("Anna Rossi", "ok", "9/26"), r("Luca Bianchi", "ok", "10:02 AM")]);
        assert_eq!(d.scan(&[r("Anna Rossi", "ok", "10:05 AM"), r("Luca Bianchi", "ok", "10:07 AM")]), [new("Anna Rossi", "ok"), new("Luca Bianchi", "ok")]);
    }
}
