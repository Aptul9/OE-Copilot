//! The Teams page of a slot and the moves every action shares (src/agent/teams/page.ts): opening a chat, closing menus,
//! the real mouse hover that makes the action bar of a message appear, a click on the side bar. Which chat Teams shows
//! is read from its header (src/agent/logic/chats.ts).

pub mod auth;

use crate::agent::browser::scripts::{s, st};
use crate::agent::browser::{Page, Pick, Point, R, sleep};
use crate::agent::store::Store;
use crate::agent::{info, now_ms, warn};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The header of the open chat (page script openChatHeader): its title as the list names it, every person a group chat
/// without a name shows, the "+N" of the others, the participant count (None where the header shows none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenHeader {
    pub title: String,
    pub people: Vec<String>,
    pub more: i64,
    pub count: Option<i64>,
}

impl OpenHeader {
    pub fn of(v: &Value) -> OpenHeader {
        OpenHeader {
            title: v["title"].as_str().unwrap_or("").to_string(),
            people: v["people"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect(),
            more: v["more"].as_i64().unwrap_or(0),
            count: v["count"].as_i64(),
        }
    }
}

/// The open title can differ from the list name (cut at 60 characters, suffixes such as "(External)"): a prefix still
/// matches, unless the title is another chat of the list (sameChat).
pub fn same_chat(title: &str, name: &str, known: &dyn Fn(&str) -> bool) -> bool {
    if title.is_empty() || name.is_empty() {
        return false;
    }
    if title == name {
        return true;
    }
    if !title.starts_with(name) && !name.starts_with(title) {
        return false;
    }
    !known(title)
}

fn header_total(h: &OpenHeader) -> i64 {
    h.count.unwrap_or(h.people.len() as i64 + if h.more != 0 { h.more } else { 1 })
}

fn fits_group_header(h: &OpenHeader, name: &str) -> bool {
    if h.more == 0 && h.people.join(", ") == name {
        return true;
    }
    let Some(c) = regex::Regex::new(r"^(.+), \+(\d+)$").expect("fixed").captures(name) else { return false };
    let named = &c[1];
    let others: i64 = c[2].parse().unwrap_or(0);
    for j in 1..=h.people.len() {
        if h.people[..j].join(", ") == named {
            return header_total(h) == j as i64 + others;
        }
    }
    named.starts_with(&format!("{}, ", h.people.join(", "))) && header_total(h) >= h.people.len() as i64 + 1 + others
}

/// Whether the open chat is `name` of the list (isOpenChat): a header of one person by its title, a group chat without
/// a name person by person against its "+N", never one two chats of the list fit.
pub fn is_open_chat(h: &OpenHeader, name: &str, known: &[String]) -> bool {
    if h.people.len() < 2 && h.more == 0 {
        return same_chat(&h.title, name, &|n| known.iter().any(|k| k == n));
    }
    if !fits_group_header(h, name) {
        return false;
    }
    !known.iter().any(|other| other != name && fits_group_header(h, other))
}

/// A message by id, for the locators.
pub fn message_selector(mid: &str) -> String {
    let escaped: String = mid.chars().flat_map(|c| if c == '"' || c == '\\' { vec!['\\', c] } else { vec![c] }).collect();
    format!(r#"{}[data-mid="{escaped}"]"#, s("message"))
}

/// Why a chat did not open (OpenProblem).
pub type OpenProblem = &'static str;

pub struct TeamsPage {
    pub page: Rc<Page>,
    store: Rc<Store>,
    /// chats a sweep of the whole list did not find, and when
    missed: RefCell<HashMap<String, i64>>,
}

impl TeamsPage {
    pub fn new(page: Rc<Page>, store: Rc<Store>) -> TeamsPage {
        TeamsPage { page, store, missed: RefCell::default() }
    }

    pub async fn is_open(&self, name: &str) -> R<bool> {
        let header = self.page.evaluate("chat-list.openChatHeader", Some(&st(json!({})))).await?;
        let known: Vec<String> = self.store.chats().into_iter().map(|c| c.name).collect();
        Ok(is_open_chat(&OpenHeader::of(&header), name, &known))
    }

    pub async fn open_chat(&self, name: &str) -> bool {
        self.show_chat(name).await.is_none()
    }

    /// Menus closed, then the chat open; `typing`: asked once more right before something is typed in it.
    pub async fn to_chat(&self, name: &str, typing: bool) -> bool {
        self.clear_overlays().await && self.open_chat(name).await && (!typing || self.is_open(name).await.unwrap_or(false))
    }

    /// Opens the chat unless Teams shows it already: its row clicked, Teams waited for, the list back to the top. None once
    /// Teams shows it, otherwise why not.
    pub async fn show_chat(&self, name: &str) -> Option<OpenProblem> {
        if self.is_open(name).await.unwrap_or(false) {
            return None;
        }
        let clicked = self.click_row(name).await;
        let mut open = false;
        for _ in 0..24 {
            if !clicked || open {
                break;
            }
            open = self.is_open(name).await.unwrap_or(false);
            if !open {
                sleep(250).await;
            }
        }
        let _ = self.scroll_list("top").await;
        if !clicked {
            return Some("not-listed");
        }
        if !open {
            warn("open", "chat did not open", &[("chat", name.into())]);
            return Some("not-shown");
        }
        let _ = self.page.wait_for(&s("message"), Pick::First, 3000).await;
        sleep(400).await;
        None
    }

    async fn scroll_list(&self, to: &str) -> R<bool> {
        Ok(self.page.evaluate("chat-list.scrollList", Some(&json!({ "item": s("anyChatRow"), "to": to }))).await? == Value::Bool(true))
    }

    /// The row of that exact name, the virtualized list scrolled down from the top to reach it; a name that only starts the
    /// same as a last resort. A name a sweep missed is not swept for again within 30 s.
    async fn click_row(&self, name: &str) -> bool {
        let click = |exact: bool| async move {
            self.page.evaluate("chat-list.clickChatRow", Some(&st(json!({ "name": name, "exact": exact })))).await.is_ok_and(|v| v == Value::Bool(true))
        };
        if click(true).await {
            return true;
        }
        if now_ms() - self.missed.borrow().get(name).copied().unwrap_or(0) < 30_000 {
            return false;
        }
        let _ = self.scroll_list("top").await;
        for _ in 0..9 {
            sleep(400).await;
            if click(true).await {
                self.missed.borrow_mut().remove(name);
                return true;
            }
            if !self.scroll_list("down").await.unwrap_or(false) {
                break;
            }
        }
        self.missed.borrow_mut().insert(name.to_string(), now_ms());
        let _ = self.scroll_list("top").await;
        sleep(400).await;
        if click(false).await {
            return true;
        }
        warn("open", "chat not in the list", &[("chat", name.into())]);
        false
    }

    /// Menus or dialogs left open over the chat: Escape, up to three times; one still open is named in the log.
    pub async fn clear_overlays(&self) -> bool {
        for i in 0.. {
            let open = match self.page.evaluate("message-actions.openOverlayNames", Some(&crate::agent::browser::scripts::sel())).await {
                Ok(v) => v.as_array().cloned().unwrap_or_default(),
                Err(_) => return false,
            };
            if open.is_empty() {
                return true;
            }
            if i == 3 {
                let names: Vec<String> = open.iter().map(|v| crate::js::string(Some(v))).collect();
                warn("page", "overlay still open after Escape", &[("overlays", names.join(", ").into())]);
                return false;
            }
            if self.page.press("Escape").await.is_err() {
                return false;
            }
            sleep(400).await;
        }
        false
    }

    /// Empties the compose box after a send that went wrong: what is left would go out with the next message.
    pub async fn empty_compose_box(&self) -> bool {
        let _ = self.page.press("Escape").await;
        for _ in 0..3 {
            let attempt = async {
                // focus, not a click: the click could land on an image or a person tagged, which open their menus
                self.page.focus(&s("editor"), Pick::Last, 2000).await?;
                self.page.press("Control+A").await?;
                self.page.press("Delete").await?;
                sleep(300).await;
                Ok::<bool, String>(self.page.evaluate("compose.composerLeft", Some(&crate::agent::browser::scripts::sel())).await? != Value::Bool(true))
            };
            if attempt.await.unwrap_or(false) {
                return true;
            }
        }
        warn("compose", "compose box not emptied: the next message would carry what is left", &[]);
        false
    }

    /// The action bar shows up only with a real mouse over the message: synthetic events are ignored.
    pub async fn hover_message(&self, mid: &str) -> bool {
        let m = message_selector(mid);
        if self.page.count(&m).await.unwrap_or(0) == 0 {
            return false;
        }
        let bar = format!("{}:visible", s("actionBar"));
        for _ in 0..4 {
            let attempt = async {
                self.page.evaluate_on(&m, Pick::First, "message-actions.centerElement", None).await?;
                let Some((x, y, w, h)) = self.page.bounding_box(&m, Pick::First).await? else { return Ok(false) };
                // straight mouse moves: Playwright's hover() waited for the Teams animations, up to 10 s
                self.page.mouse_move(2.0, 2.0, 1).await?;
                sleep(100).await;
                self.page.mouse_move(x + w / 2.0, y + (h / 2.0).min(20.0), 3).await?;
                self.page.wait_for(&bar, Pick::First, 1500).await?;
                Ok::<bool, String>(true)
            };
            if attempt.await.unwrap_or(false) {
                return true;
            }
            sleep(300).await;
        }
        false
    }

    /// Hover, then a click on button `tid` of the bar by its coordinates: the mouse jumps there without crossing other
    /// messages.
    pub async fn click_bar_button(&self, mid: &str, tid: &str) -> bool {
        for _ in 0..3 {
            if !self.hover_message(mid).await {
                return false;
            }
            let point = self
                .page
                .evaluate("message-actions.barButtonPoint", Some(&json!({ "s": crate::agent::browser::scripts::sel(), "mid": mid, "tid": tid })))
                .await
                .ok()
                .and_then(|v| Point::of(&v));
            if let Some(p) = point {
                return self.page.mouse_click(p.x, p.y).await.is_ok();
            }
            sleep(300).await;
        }
        false
    }

    pub async fn mouse_away(&self) {
        let _ = self.page.mouse_move(2.0, 2.0, 1).await;
    }

    /// A button of the Teams rail (Activity, Chat), clicked on a part of it nothing covers (a tooltip can sit over its
    /// middle); while Teams starts, up to `wait_ms` for a free point.
    pub async fn click_rail(&self, selector: &str, wait_ms: u64) -> R<()> {
        let until = now_ms() + wait_ms as i64;
        let mut point = self.rail_point(selector).await;
        while point.is_none() && now_ms() < until && !self.page.is_closed() {
            sleep(250).await;
            point = self.rail_point(selector).await;
        }
        if let Some(p) = point {
            return self.page.mouse_click(p.x, p.y).await;
        }
        self.page.click(&format!("{selector}:visible"), Pick::First, 4000).await
    }

    async fn rail_point(&self, selector: &str) -> Option<Point> {
        self.page.evaluate("page-state.uncoveredPoint", Some(&json!(selector))).await.ok().and_then(|v| Point::of(&v))
    }

    /// Logged once a chat shows (park).
    pub fn note_show(&self, chat: &str) {
        info("show", chat, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // test/agent/open-header.test.ts: names written "Surname, Name", as on the Tesco tenant
    const LIST: [&str; 7] =
        ["Newson, Geoffrey", "Newson, Geoffrey, +2", "Mohamed, Faraz", "Mohamed, Faraz, P, Pravin, +4", "Page, Duncan", "Anna Rossi", "Anna Rossi, +2"];

    fn header(people: &[&str], more: i64, count: Option<i64>) -> OpenHeader {
        let title = match people.first() {
            None => String::new(),
            Some(p) if more != 0 && people.len() == 1 => format!("{p}, +{more}"),
            Some(p) => p.to_string(),
        };
        OpenHeader { title, people: people.iter().map(|p| p.to_string()).collect(), more, count }
    }

    fn open(h: &OpenHeader, name: &str, extra: &[&str]) -> bool {
        let known: Vec<String> = LIST.iter().chain(extra).map(|x| x.to_string()).collect();
        is_open_chat(h, name, &known)
    }

    #[test]
    fn which_chat_the_header_shows() {
        let newson_group = header(&["Newson, Geoffrey", "Page, Duncan"], 0, Some(3));
        let mohamed_group = header(&["Mohamed, Faraz", "P, Pravin", "Renton, John"], 3, Some(6));
        let newson_one = header(&["Newson, Geoffrey"], 0, None);
        let mohamed_narrow = header(&["Mohamed, Faraz"], 5, None);
        assert!(open(&newson_group, "Newson, Geoffrey, +2", &[]));
        assert!(open(&mohamed_group, "Mohamed, Faraz, P, Pravin, +4", &[]));
        assert!(open(&mohamed_narrow, "Mohamed, Faraz, P, Pravin, +4", &[]));
        assert!(!open(&mohamed_narrow, "Mohamed, Faraz", &[]));
        assert!(!open(&mohamed_narrow, "Mohamed, Faraz, P, Pravin, +4", &["Mohamed, Faraz, Q, Quinn, +3"]));
        assert!(!open(&newson_group, "Newson, Geoffrey", &[]));
        assert!(!open(&mohamed_group, "Mohamed, Faraz", &[]));
        assert!(open(&newson_one, "Newson, Geoffrey", &[]));
        assert!(!open(&newson_one, "Newson, Geoffrey, +2", &[]));
        assert!(!open(&header(&["Newson, Geoffrey", "Page, Duncan"], 0, Some(5)), "Newson, Geoffrey, +2", &[]));
        assert!(!open(&mohamed_group, "Mohamed, Faraz, +4", &[]));
        assert!(open(&header(&["Newson, Geoffrey", "Page, Duncan"], 0, None), "Newson, Geoffrey, +2", &[]));
        assert!(!open(&header(&["Page, Duncan", "Newson, Geoffrey"], 0, Some(3)), "Newson, Geoffrey, +2", &[]));
        assert!(open(&header(&["Anna Rossi"], 2, None), "Anna Rossi, +2", &[]));
        assert!(!open(&header(&["Anna Rossi"], 2, None), "Anna Rossi", &[]));
        // a title cut or with a suffix still matches, another chat of the list does not
        let known = ["Luca Bianchi".to_string(), "Luca Bianchini".to_string()];
        assert!(!is_open_chat(&header(&["Luca Bianchini"], 0, None), "Luca Bianchi", &known));
        assert!(is_open_chat(&OpenHeader { title: "Anna (External)".into(), ..Default::default() }, "Anna", &known));
        assert_eq!(message_selector(r#"1"2"#), r#"[data-tid="chat-pane-message"][data-mid="1\"2"]"#);
    }
}
