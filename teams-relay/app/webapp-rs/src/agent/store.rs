//! The agent side of data/N/messages.db (src/agent/store/slot-store.ts): one connection for the life of the slot, the
//! web app reading and queueing commands through its own.

use super::now_s;
use crate::slotdb::{CALL_LOG_SIZE, HAS_TEAMS_ID, insert_row, open_store, parse_extra};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::Path;

/// The chats the list keeps (CHAT_LIMIT).
pub const CHAT_LIMIT: usize = 40;

/// A chat of the list as the agent keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatEntry {
    pub name: String,
    pub preview: String,
    pub time: String,
    pub unread: bool,
    pub mention: bool,
    pub muted: bool,
    pub av: String,
    pub presence: String,
    pub kind: String,
}

/// A message as the agent saves it; extra: the rich fields with a value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedMessage {
    pub mid: String,
    pub author: String,
    pub text: String,
    pub mine: bool,
    pub reacts: String,
    pub extra: Option<Map<String, Value>>,
}

/// A message as saved: its row with the rich fields spread (Message).
pub type MessageRow = Map<String, Value>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityEntry {
    pub id: String,
    pub kind: String,
    pub actor: String,
    pub title: String,
    pub emoji: String,
    pub preview: String,
    pub tm: String,
    pub chat: String,
    pub channel: bool,
    pub unread: bool,
    pub av: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingCommand {
    pub id: i64,
    pub kind: String,
    pub arg1: String,
    pub arg2: String,
    pub ts: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub id: String,
    pub actor: String,
    pub preview: String,
    pub chat: String,
    pub channel: bool,
    pub unread: bool,
}

/// The visible chats first, in Teams order, the others after (none when the list is complete); a picture or a kind this
/// read could not tell keeps the one known (mergeChats).
pub fn merge_chats(visible: &[ChatEntry], stored: &[ChatEntry], replace: bool) -> Vec<ChatEntry> {
    let known = |f: fn(&ChatEntry) -> &String, name: &str| stored.iter().find(|c| c.name == name && !f(c).is_empty()).map(|c| f(c).clone());
    let rows: Vec<ChatEntry> = visible
        .iter()
        .map(|c| {
            let mut c = c.clone();
            if c.av.is_empty()
                && let Some(av) = known(|x| &x.av, &c.name)
            {
                c.av = av;
            }
            if c.kind.is_empty()
                && let Some(kind) = known(|x| &x.kind, &c.name)
            {
                c.kind = kind;
            }
            c
        })
        .collect();
    if replace {
        return rows;
    }
    let others = stored.iter().filter(|c| !visible.iter().any(|v| v.name == c.name)).map(|c| ChatEntry { presence: String::new(), ..c.clone() });
    rows.into_iter().chain(others).take(CHAT_LIMIT).collect()
}

pub struct Store {
    c: Connection,
}

fn bit(b: bool) -> i64 {
    i64::from(b)
}

impl Store {
    pub fn open(file: &Path) -> crate::web::Res<Store> {
        Ok(Store { c: open_store(file)? })
    }

    pub fn get_state(&self, k: &str) -> String {
        self.c.query_row("SELECT v FROM state WHERE k=?", [k], |r| r.get::<_, Option<String>>(0)).optional().ok().flatten().flatten().unwrap_or_default()
    }

    pub fn set_state(&self, k: &str, v: &str) {
        let _ = self.c.execute("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)", params![k, v]);
    }

    pub fn last_notification_ts(&self) -> i64 {
        self.c
            .query_row("SELECT ts FROM messages ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, Option<i64>>(0))
            .optional()
            .ok()
            .flatten()
            .flatten()
            .unwrap_or(0)
    }

    pub fn chats(&self) -> Vec<ChatEntry> {
        let Ok(mut st) = self.c.prepare("SELECT name, preview, tm, unread, mention, muted, av, presence, kind FROM chats ORDER BY pos") else { return vec![] };
        let rows = st.query_map([], |r| {
            let s = |i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
            let b = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0) != 0);
            Ok(ChatEntry { name: s(0)?, preview: s(1)?, time: s(2)?, unread: b(3)?, mention: b(4)?, muted: b(5)?, av: s(6)?, presence: s(7)?, kind: s(8)? })
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// One transaction: the web app never sees the list empty.
    pub fn save_chats(&self, visible: &[ChatEntry], replace: bool) -> rusqlite::Result<()> {
        if visible.is_empty() {
            return Ok(());
        }
        let rows = merge_chats(visible, &self.chats(), replace);
        let tx = self.c.unchecked_transaction()?;
        let ts = now_s();
        tx.execute("DELETE FROM chats", [])?;
        let mut insert = tx.prepare(&insert_row("chats"))?;
        for (i, c) in rows.iter().enumerate() {
            insert.execute(params![c.name, c.preview, i as i64, ts, c.time, bit(c.unread), bit(c.mention), bit(c.muted), c.av, c.presence, c.kind])?;
        }
        drop(insert);
        tx.commit()
    }

    pub fn is_muted_chat(&self, name: &str) -> bool {
        self.c.query_row("SELECT 1 FROM chats WHERE name=? AND muted=1", [name], |_| Ok(())).optional().ok().flatten().is_some()
    }

    pub fn mark_chat_read(&self, name: &str) {
        let _ = self.c.execute("UPDATE chats SET unread=0, mention=0 WHERE name=?", [name]);
    }

    pub fn is_known_chat(&self, name: &str) -> bool {
        self.c.query_row("SELECT 1 FROM chats WHERE name=?", [name], |_| Ok(())).optional().ok().flatten().is_some()
    }

    pub fn save_chat_messages(&self, chat: &str, messages: &[SavedMessage]) -> rusqlite::Result<()> {
        let tx = self.c.unchecked_transaction()?;
        tx.execute("DELETE FROM chat_messages WHERE chat=?", [chat])?;
        let mut insert = tx.prepare(&insert_row("chat_messages"))?;
        for (i, m) in messages.iter().enumerate() {
            let extra = m.extra.as_ref().map(|e| crate::js::stringify(&Value::Object(e.clone()))).unwrap_or_default();
            insert.execute(params![chat, i as i64, m.mid, m.author, m.text, bit(m.mine), m.reacts, extra])?;
        }
        drop(insert);
        tx.commit()
    }

    /// Messages of a chat as saved, the rich fields spread.
    pub fn messages(&self, chat: &str) -> Vec<MessageRow> {
        let Ok(mut st) = self.c.prepare("SELECT mid, author, text, mine, reacts, extra FROM chat_messages WHERE chat=? ORDER BY idx") else { return vec![] };
        let rows = st.query_map([chat], |r| {
            let mut m = Map::new();
            for (i, k) in ["mid", "author", "text"].iter().enumerate() {
                m.insert((*k).into(), r.get::<_, Option<String>>(i)?.map_or(Value::Null, Value::String));
            }
            m.insert("mine".into(), r.get::<_, Option<i64>>(3)?.map_or(Value::Null, |v| json!(v)));
            m.insert("reacts".into(), r.get::<_, Option<String>>(4)?.map_or(Value::Null, Value::String));
            for (k, v) in parse_extra(r.get::<_, Option<String>>(5)?.as_deref()) {
                m.insert(k, v);
            }
            Ok(m)
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// The "Read by" of messages, by id: label and names.
    pub fn read_by_cache(&self, mids: &[String]) -> HashMap<String, Value> {
        if mids.is_empty() {
            return HashMap::new();
        }
        let sql = format!("SELECT mid, label, names FROM readby WHERE mid IN ({})", vec!["?"; mids.len()].join(","));
        let Ok(mut st) = self.c.prepare(&sql) else { return HashMap::new() };
        let rows = st.query_map(rusqlite::params_from_iter(mids), |r| {
            let names: Vec<String> = match serde_json::from_str::<Value>(&r.get::<_, Option<String>>(2)?.unwrap_or_else(|| "[]".into())) {
                Ok(Value::Array(a)) => a.iter().map(|x| crate::js::string(Some(x))).collect(),
                _ => vec![],
            };
            Ok((r.get::<_, String>(0)?, json!({ "label": r.get::<_, Option<String>>(1)?.unwrap_or_default(), "names": names })))
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn pending_commands(&self) -> Vec<PendingCommand> {
        let Ok(mut st) = self.c.prepare(
            "SELECT id, type, COALESCE(arg1, '') AS arg1, COALESCE(arg2, '') AS arg2, COALESCE(ts, 0) AS ts FROM commands WHERE status='pending' ORDER BY id",
        ) else {
            return vec![];
        };
        let rows = st.query_map([], |r| {
            Ok(PendingCommand { id: r.get(0)?, kind: r.get::<_, Option<String>>(1)?.unwrap_or_default(), arg1: r.get(2)?, arg2: r.get(3)?, ts: r.get(4)? })
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn command_status(&self, id: i64) -> Option<String> {
        self.c.query_row("SELECT status FROM commands WHERE id=?", [id], |r| r.get::<_, Option<String>>(0)).optional().ok().flatten().flatten()
    }

    /// The agent takes the command: from here on a stop of the agent leaves it running, never pending again.
    pub fn start_command(&self, id: i64) {
        let _ = self.c.execute("UPDATE commands SET status='running' WHERE id=? AND status='pending'", [id]);
    }

    pub fn finish_command(&self, id: i64, status: &str) {
        let _ = self.c.execute("UPDATE commands SET status=? WHERE id=?", params![status, id]);
    }

    /// Commands left running by an agent that stopped may have reached Teams: unconfirmed, never run again.
    pub fn interrupted_commands(&self) -> usize {
        self.c.execute("UPDATE commands SET status='unconfirmed' WHERE status='running'", []).unwrap_or(0)
    }

    /// Commands still waiting after `max_age` seconds end as failed.
    pub fn expire_pending_commands(&self, max_age: i64) -> usize {
        self.c.execute("UPDATE commands SET status='failed' WHERE status='pending' AND ts < ?", [now_s() - max_age]).unwrap_or(0)
    }

    #[cfg(test)]
    pub fn enqueue(&self, kind: &str, arg1: &str, arg2: &str) -> i64 {
        self.c.execute("INSERT INTO commands(ts, type, arg1, arg2, key) VALUES(?,?,?,?,NULL)", params![now_s(), kind, arg1, arg2]).expect("queued");
        self.c.last_insert_rowid()
    }

    /// A call that rang, for the call log of the web app: the last CALL_LOG_SIZE are kept.
    pub fn add_call(&self, caller: &str, since: i64, seconds: i64) {
        let run = || -> rusqlite::Result<()> {
            let tx = self.c.unchecked_transaction()?;
            tx.execute(&insert_row("calls"), params![since, caller, seconds])?;
            tx.execute("DELETE FROM calls WHERE id NOT IN (SELECT id FROM calls ORDER BY since DESC, id DESC LIMIT ?)", [CALL_LOG_SIZE])?;
            tx.commit()
        };
        let _ = run();
    }

    pub fn mentions(&self) -> Vec<Mention> {
        let sql = format!("SELECT id, actor, preview, chat, channel, unread FROM activity WHERE kind='mention' AND {HAS_TEAMS_ID} ORDER BY pos");
        let Ok(mut st) = self.c.prepare(&sql) else { return vec![] };
        let rows = st.query_map([], |r| {
            let s = |i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
            let b = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0) != 0);
            Ok(Mention { id: s(0)?, actor: s(1)?, preview: s(2)?, chat: s(3)?, channel: b(4)?, unread: b(5)? })
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// Every missed call of the feed that has its Teams id, in feed order: id, caller, the time Teams shows.
    pub fn missed_calls(&self) -> Vec<(String, String, String)> {
        let sql = format!("SELECT id, actor AS caller, tm AS time FROM activity WHERE kind='call' AND {HAS_TEAMS_ID} ORDER BY pos");
        let Ok(mut st) = self.c.prepare(&sql) else { return vec![] };
        let rows = st.query_map([], |r| {
            let s = |i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
            Ok((s(0)?, s(1)?, s(2)?))
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// The chat with yourself ("Name (You)"): the visible page can read nothing there on the user's behalf.
    pub fn self_chat(&self) -> String {
        self.c
            .query_row("SELECT name FROM chats WHERE name LIKE '%(You)%' ORDER BY pos LIMIT 1", [], |r| r.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub fn has_pending_commands(&self) -> bool {
        self.c.query_row("SELECT 1 FROM commands WHERE status='pending' LIMIT 1", [], |_| Ok(())).optional().ok().flatten().is_some()
    }

    /// Your messages of the chat with an id, the last first.
    pub fn own_recent_message_ids(&self, chat: &str, limit: usize) -> Vec<String> {
        let Ok(mut st) = self.c.prepare("SELECT mid FROM chat_messages WHERE chat=? AND mine=1 AND mid<>'' ORDER BY idx DESC LIMIT ?") else { return vec![] };
        st.query_map(params![chat, limit as i64], |r| r.get::<_, String>(0)).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// The "Read by" known of the messages of a chat: label and when it was read.
    pub fn read_by_of(&self, chat: &str) -> HashMap<String, (String, i64)> {
        let Ok(mut st) = self.c.prepare("SELECT mid, label, ts FROM readby WHERE chat=?") else { return HashMap::new() };
        let rows = st.query_map([chat], |r| {
            Ok((r.get::<_, String>(0)?, (r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0))))
        });
        rows.map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn save_read_by(&self, mid: &str, chat: &str, label: &str, names: &[String]) {
        let _ = self.c.execute(&insert_row("readby"), params![mid, chat, label, crate::js::stringify(&json!(names)), now_s()]);
    }

    /// Picture of each item of the Activity feed that has one, by id.
    pub fn activity_avatars(&self) -> HashMap<String, String> {
        let Ok(mut st) = self.c.prepare("SELECT id, av FROM activity WHERE av <> ''") else { return HashMap::new() };
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// Ids of every item of the Activity feed, in feed order.
    pub fn activity_ids(&self) -> Vec<String> {
        let Ok(mut st) = self.c.prepare("SELECT id FROM activity ORDER BY pos") else { return vec![] };
        st.query_map([], |r| r.get::<_, String>(0)).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// Ids of the items with their Teams id the feed shows unread, in feed order.
    pub fn unread_activity(&self) -> Vec<String> {
        let sql = format!("SELECT id FROM activity WHERE unread=1 AND {HAS_TEAMS_ID} ORDER BY pos");
        let Ok(mut st) = self.c.prepare(&sql) else { return vec![] };
        st.query_map([], |r| r.get::<_, String>(0)).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// Files of the media folder the rows name: pictures of chats, feed and account, images and pictures of the messages
    /// kept (mediaFilesOf).
    pub fn media_files(&self) -> std::collections::HashSet<String> {
        let mut files = std::collections::HashSet::new();
        let mut add = |f: Option<&str>| {
            if let Some(f) = f.filter(|f| !f.is_empty()) {
                files.insert(f.to_string());
            }
        };
        if let Ok(mut st) = self.c.prepare("SELECT av FROM chats UNION SELECT av FROM activity")
            && let Ok(rows) = st.query_map([], |r| r.get::<_, Option<String>>(0))
        {
            for av in rows.flatten() {
                add(av.as_deref());
            }
        }
        if let Ok(mut st) = self.c.prepare("SELECT extra FROM chat_messages")
            && let Ok(rows) = st.query_map([], |r| r.get::<_, Option<String>>(0))
        {
            for extra in rows.flatten() {
                let e = parse_extra(extra.as_deref());
                add(e.get("av").and_then(Value::as_str));
                for im in e.get("images").and_then(Value::as_array).into_iter().flatten() {
                    add(im.get("f").and_then(Value::as_str));
                }
            }
        }
        let me = crate::state::object(Some(&self.get_state(crate::state::key::ME))).unwrap_or_default();
        add(me.get("av").and_then(Value::as_str));
        add(me.get("logo").and_then(Value::as_str));
        files
    }

    pub fn save_activity(&self, items: &[ActivityEntry]) -> rusqlite::Result<()> {
        let tx = self.c.unchecked_transaction()?;
        let ts = now_s();
        tx.execute("DELETE FROM activity", [])?;
        let mut insert = tx.prepare(&insert_row("activity"))?;
        for (i, a) in items.iter().enumerate() {
            let id = if a.id.is_empty() { format!("x{i}") } else { a.id.clone() };
            insert.execute(params![id, i as i64, a.kind, a.actor, a.title, a.emoji, a.preview, a.tm, a.chat, bit(a.channel), bit(a.unread), ts, a.av])?;
        }
        drop(insert);
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::testing::temp_dir;

    fn chat(name: &str, av: &str, kind: &str) -> ChatEntry {
        ChatEntry { name: name.into(), av: av.into(), kind: kind.into(), presence: "busy".into(), ..Default::default() }
    }

    #[test]
    fn merges_the_list_keeping_known_pictures_and_kinds() {
        let stored = [chat("A", "a.png", "one"), chat("B", "b.png", "group")];
        let merged = merge_chats(&[chat("A", "", ""), chat("C", "", "")], &stored, false);
        assert_eq!(
            merged.iter().map(|c| (c.name.as_str(), c.av.as_str(), c.kind.as_str(), c.presence.as_str())).collect::<Vec<_>>(),
            [("A", "a.png", "one", "busy"), ("C", "", "", "busy"), ("B", "b.png", "group", "")]
        );
        assert_eq!(merge_chats(&[chat("A", "", "")], &stored, true).len(), 1);
    }

    #[test]
    fn commands_chats_messages_and_the_feed() {
        let s = Store::open(&temp_dir().join("1").join("messages.db")).expect("store");
        let id = s.enqueue("send", "Anna", "hi");
        assert_eq!(s.pending_commands().iter().map(|c| (c.id, c.kind.as_str())).collect::<Vec<_>>(), [(id, "send")]);
        s.start_command(id);
        assert_eq!(s.interrupted_commands(), 1);
        assert_eq!(s.command_status(id).as_deref(), Some("unconfirmed"));
        s.save_chats(&[chat("Anna", "", "one")], true).expect("chats");
        assert!(s.is_known_chat("Anna") && !s.is_muted_chat("Anna"));
        let extra = json!({"status": "Sent", "images": [{"f": "a.jpg", "w": 1, "h": 2}]}).as_object().cloned();
        s.save_chat_messages("Anna", &[SavedMessage { mid: "1".into(), author: "".into(), text: "hi".into(), mine: true, reacts: "".into(), extra }])
            .expect("messages");
        assert_eq!(
            Value::Object(s.messages("Anna")[0].clone()),
            json!({"mid": "1", "author": "", "text": "hi", "mine": 1, "reacts": "", "status": "Sent", "images": [{"f": "a.jpg", "w": 1, "h": 2}]})
        );
        s.save_activity(&[
            ActivityEntry { id: "7".into(), kind: "call".into(), actor: "Rossi".into(), tm: "9:00 AM".into(), ..Default::default() },
            ActivityEntry { kind: "mention".into(), ..Default::default() },
        ])
        .expect("feed");
        assert_eq!(s.missed_calls(), [("7".into(), "Rossi".into(), "9:00 AM".into())]);
        assert!(s.mentions().is_empty());
        s.add_call("Rossi", 5, 3);
        s.set_state("k", "v");
        assert_eq!((s.get_state("k").as_str(), s.get_state("none").as_str()), ("v", ""));
    }
}
