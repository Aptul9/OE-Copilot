//! The actions on Teams (src/agent/teams/actions.ts, mentions.ts): each one first through chatsvc with the token of the
//! page, else on the page as a person does it, and checked on the page that Teams applied it.

use super::Agent;
use crate::agent::browser::scripts::{s, sel, st, table};
use crate::agent::browser::{Pick, Point, R, sleep, until};
use crate::agent::chatsvc::Part;
use crate::agent::teams::message_selector;
use crate::agent::{info, js_trim, warn};
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};

/// What became of a message the agent sent: sent, failed (it never left the compose box), or unconfirmed (Enter went,
/// Teams did not show it sent in time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SendResult {
    Sent,
    Failed,
    Unconfirmed,
}

/// What a send runs as soon as the message went: the chat saved, so the web app sees it at once.
pub type AfterPress<'a> = Option<Box<dyn Fn() -> LocalBoxFuture<'a, ()> + 'a>>;

async fn after_press(sent: &AfterPress<'_>) {
    if let Some(f) = sent {
        f().await;
    }
}

/// The end of every send, once the message went: `sent` at once, then Teams watched until `shown` says it has it.
async fn confirm_sent(
    a: &Agent,
    area: &str,
    shown_script: &str,
    arg: Value,
    tries: usize,
    sent: &AfterPress<'_>,
    fields: Vec<(&str, crate::agent::Field)>,
) -> SendResult {
    after_press(sent).await;
    let Some(page) = a.page() else { return SendResult::Unconfirmed };
    if until(|| async { Ok(page.evaluate(shown_script, Some(&arg)).await? == Value::Bool(true)) }, tries, 300).await {
        return SendResult::Sent;
    }
    warn(area, "not shown sent on Teams: unconfirmed", &fields);
    SendResult::Unconfirmed
}

/// The start of `text` is in the compose box.
async fn typed(a: &Agent, text: &str) -> R<bool> {
    let page = a.page().ok_or("no page")?;
    let box_text = page.evaluate("message-actions.composerText", Some(&sel())).await?;
    Ok(box_text.as_str().unwrap_or("").contains(&crate::agent::cut(text, 20)))
}

async fn message_ids(a: &Agent) -> R<Value> {
    a.page().ok_or("no page")?.evaluate("compose.messageIds", Some(&sel())).await
}

async fn composer_left(a: &Agent) -> R<bool> {
    Ok(a.page().ok_or("no page")?.evaluate("compose.composerLeft", Some(&sel())).await? == Value::Bool(true))
}

/// A press that threw may have sent the message or not: the text still in the box and no new message, it never left.
async fn after_failed_press(a: &Agent, text: &str, before: &Value) -> SendResult {
    sleep(1000).await;
    let check = async {
        let left = typed(a, text).await?;
        let now = message_ids(a).await?;
        let fresh = now.as_array().into_iter().flatten().any(|m| !before.as_array().is_some_and(|b| b.contains(m)));
        Ok::<bool, String>(left && !fresh)
    };
    if check.await.unwrap_or(false) {
        if let Some(tp) = a.tp() {
            tp.empty_compose_box().await;
        }
        return SendResult::Failed;
    }
    SendResult::Unconfirmed
}

/// The chatsvc token of the page and the conversation of the chat, for a fast path.
async fn fast(a: &Agent, chat: &str, mid: Option<&str>) -> Option<(crate::agent::api::TeamsAuth, String)> {
    let auth = a.page_auth().await?;
    let conv = a.conversation(&auth, chat, mid).await?;
    Some((auth, conv))
}

/// The text typed in the compose box and sent, as a person does; first through chatsvc.
pub async fn send_text(a: &Agent, chat: &str, raw: &str, sent: AfterPress<'_>) -> SendResult {
    let text = js_trim(raw).to_string();
    if text.is_empty() {
        return SendResult::Failed;
    }
    if let Some((auth, conv)) = fast(a, chat, None).await {
        match a.chatsvc.send_text(&a.http, &auth, &conv, &text).await {
            Ok(Some(_)) => {
                info("send", "sent via chatsvc API", &[("chat", chat.into()), ("convId", conv.as_str().into())]);
                return SendResult::Sent;
            }
            Ok(None) => {}
            Err(e) => warn("send", &format!("chatsvc fast path failed: {}, falling back to DOM", crate::agent::text_of(&e)), &[("chat", chat.into())]),
        }
    }
    let Some(tp) = a.tp() else { return SendResult::Failed };
    if !tp.to_chat(chat, true).await {
        return SendResult::Failed;
    }
    if composer_left(a).await.unwrap_or(true) {
        warn("send", "compose box not empty", &[("chat", chat.into())]);
        return SendResult::Failed;
    }
    let Ok(before) = message_ids(a).await else { return SendResult::Failed };
    let page = &tp.page;
    let mut pressed = false;
    let attempt = async {
        let editor = if page.count(&s("editor")).await? > 0 { s("editor") } else { s("textbox") };
        page.click(&editor, Pick::First, 30_000).await?;
        sleep(200).await;
        page.insert_text(&text).await?;
        sleep(300).await;
        if !typed(a, &text).await? {
            return Err("text not in the compose box".to_string());
        }
        pressed = true;
        if page.count(&s("sendButton")).await? > 0 { page.click(&s("sendButton"), Pick::First, 30_000).await } else { page.press("Enter").await }
    };
    if let Err(e) = attempt.await {
        warn("send", &e, &[("chat", chat.into())]);
        if pressed {
            return after_failed_press(a, &text, &before).await;
        }
        tp.empty_compose_box().await;
        return SendResult::Failed;
    }
    confirm_sent(a, "send", "compose.ownMessageSent", st(json!({ "before": before })), 50, &sent, vec![("chat", chat.into())]).await
}

/// The image pasted in, then the caption, and Enter sends both; first through AMS and chatsvc.
pub async fn send_image(a: &Agent, chat: &str, name: &str, mime: &str, data: &[u8], caption: &str, sent: AfterPress<'_>) -> SendResult {
    if let Some((auth, conv)) = fast(a, chat, None).await {
        match a.chatsvc.send_image(&a.http, &auth, &conv, data, mime, caption).await {
            Ok(true) => {
                info("image", "sent via AMS/chatsvc API", &[("chat", chat.into()), ("convId", conv.as_str().into())]);
                return SendResult::Sent;
            }
            Ok(false) => {}
            Err(e) => warn("image", &format!("AMS fast path failed: {}, falling back to DOM", crate::agent::text_of(&e)), &[("chat", chat.into())]),
        }
    }
    let Some(tp) = a.tp() else { return SendResult::Failed };
    if !tp.to_chat(chat, true).await {
        return SendResult::Failed;
    }
    if composer_left(a).await.unwrap_or(true) {
        warn("image", "compose box not empty", &[("chat", chat.into())]);
        return SendResult::Failed;
    }
    let Ok(before) = message_ids(a).await else { return SendResult::Failed };
    let page = &tp.page;
    let mut pressed = false;
    let attempt = async {
        use base64::Engine;
        let arg = json!({ "s": sel(), "name": name, "type": mime, "data": base64::engine::general_purpose::STANDARD.encode(data) });
        if page.evaluate("compose.pasteImage", Some(&arg)).await? != Value::Bool(true) {
            return Err("the compose box did not take the image".to_string());
        }
        page.wait_for_function("compose.composerImages", Some(&sel()), 5000).await?;
        if !js_trim(caption).is_empty() {
            page.insert_text(caption).await?;
        }
        sleep(300).await;
        pressed = true;
        page.press("Enter").await
    };
    if let Err(e) = attempt.await {
        warn("image", &e, &[("chat", chat.into())]);
        if pressed {
            return SendResult::Unconfirmed;
        }
        // what was pasted must not go out with the next message
        tp.empty_compose_box().await;
        return SendResult::Failed;
    }
    confirm_sent(a, "image", "compose.ownMessageSent", st(json!({ "before": before, "image": true })), 100, &sent, vec![("chat", chat.into())]).await
}

/// Reply with quote: on the bar for other people's messages, in More options for yours; first through chatsvc.
pub async fn reply_with_quote(a: &Agent, chat: &str, mid: &str, raw: &str, sent: AfterPress<'_>) -> SendResult {
    let text = js_trim(raw).to_string();
    if text.is_empty() {
        return SendResult::Failed;
    }
    if let Some((auth, conv)) = fast(a, chat, Some(mid)).await {
        match a.chatsvc.reply_text(&a.http, &auth, &conv, mid, &text).await {
            Ok(Some(_)) => {
                info("reply", "replied via chatsvc API", &[("chat", chat.into()), ("convId", conv.as_str().into()), ("mid", mid.into())]);
                return SendResult::Sent;
            }
            Ok(None) => {}
            Err(e) => warn(
                "reply",
                &format!("chatsvc fast path failed: {}, falling back to DOM", crate::agent::text_of(&e)),
                &[("chat", chat.into()), ("mid", mid.into())],
            ),
        }
    }
    let Some(tp) = a.tp() else { return SendResult::Failed };
    if !tp.to_chat(chat, true).await {
        return SendResult::Failed;
    }
    if composer_left(a).await.unwrap_or(true) {
        warn("reply", "compose box not empty", &[("chat", chat.into())]);
        return SendResult::Failed;
    }
    let page = &tp.page;
    let Ok(before) = page.evaluate("message-actions.messageCount", Some(&sel())).await else { return SendResult::Failed };
    let mut pressed = false;
    let attempt = async {
        let mine = page.evaluate("message-actions.isOwnMessage", Some(&json!({ "s": sel(), "mid": mid }))).await? == Value::Bool(true);
        if mine || !tp.click_bar_button(mid, &table("actions", "quotedReply").unwrap_or_default()).await {
            if !tp.click_bar_button(mid, &table("actions", "more").unwrap_or_default()).await {
                return Err("action bar not found".to_string());
            }
            let entry = format!(r#"{} [data-tid="{}"]:visible"#, s("menu"), table("actions", "quotedReply").unwrap_or_default());
            page.click(&entry, Pick::First, 4000).await?;
        }
        page.wait_for_function("message-actions.quoteBoxReady", Some(&sel()), 4000).await?;
        // Teams already put the cursor after the quote: a click on the box would land on the quote
        sleep(300).await;
        page.insert_text(&text).await?;
        sleep(300).await;
        if !typed(a, &text).await? {
            return Err("text not in the compose box".to_string());
        }
        pressed = true;
        page.press("Enter").await
    };
    let outcome = attempt.await;
    tp.mouse_away().await;
    if let Err(e) = outcome {
        warn("reply", &e, &[("mid", mid.into())]);
        if pressed {
            return SendResult::Unconfirmed;
        }
        // no quote left behind in the compose box
        let _ = page.click(&format!("{}:visible", s("closeQuote")), Pick::First, 1500).await;
        return SendResult::Failed;
    }
    let arg = json!({ "s": sel(), "before": before, "text": crate::agent::cut(&text, 40) });
    confirm_sent(a, "reply", "message-actions.lastMessageQuotes", arg, 20, &sent, vec![("mid", mid.into())]).await
}

async fn deleted_state(a: &Agent, mid: &str) -> R<String> {
    let v = a.page().ok_or("no page")?.evaluate("message-actions.deletedState", Some(&json!({ "s": sel(), "mid": mid }))).await?;
    Ok(v.as_str().unwrap_or("").to_string())
}

pub async fn delete_message(a: &Agent, chat: &str, mid: &str) -> bool {
    if let Some((auth, conv)) = fast(a, chat, Some(mid)).await {
        match a.chatsvc.delete_message(&a.http, &auth, &conv, mid).await {
            Ok(true) => {
                info("delete", "deleted via chatsvc API", &[("chat", chat.into()), ("convId", conv.as_str().into()), ("mid", mid.into())]);
                return true;
            }
            Ok(false) => {}
            Err(e) => warn(
                "delete",
                &format!("chatsvc fast path failed: {}, falling back to DOM", crate::agent::text_of(&e)),
                &[("chat", chat.into()), ("mid", mid.into())],
            ),
        }
    }
    let Some(tp) = a.tp() else { return false };
    if !tp.to_chat(chat, false).await {
        return false;
    }
    let attempt = async {
        if !tp.click_bar_button(mid, &table("actions", "more").unwrap_or_default()).await {
            return Ok(false);
        }
        tp.page.click(&format!(r#"{} [data-tid="{}"]:visible"#, s("menu"), table("actions", "delete").unwrap_or_default()), Pick::First, 4000).await?;
        Ok::<bool, String>(true)
    };
    let clicked = attempt.await;
    tp.mouse_away().await;
    match clicked {
        Ok(true) => {}
        Ok(false) => return false,
        Err(e) => {
            warn("delete", &e, &[("mid", mid.into())]);
            tp.clear_overlays().await;
            return false;
        }
    }
    // Teams deletes at once and leaves "Undo" for a few seconds
    if until(|| async { Ok(deleted_state(a, mid).await? == "deleted") }, 32, 250).await {
        return true;
    }
    warn("delete", "no change on Teams", &[("mid", mid.into())]);
    false
}

pub async fn undo_delete(a: &Agent, chat: &str, mid: &str) -> bool {
    if let Some((auth, conv)) = fast(a, chat, Some(mid)).await
        && a.chatsvc.undelete_message(&a.http, &auth, &conv, mid).await.unwrap_or(false)
    {
        info("undodelete", "restored via chatsvc API", &[("chat", chat.into()), ("convId", conv.as_str().into()), ("mid", mid.into())]);
        return true;
    }
    let Some(tp) = a.tp() else { return false };
    if !tp.to_chat(chat, false).await {
        return false;
    }
    let Some(point) = tp.page.evaluate("message-actions.undoButtonPoint", Some(&json!({ "s": sel(), "mid": mid }))).await.ok().and_then(|v| Point::of(&v))
    else {
        return false;
    };
    if tp.page.mouse_click(point.x, point.y).await.is_err() {
        return false;
    }
    tp.mouse_away().await;
    until(|| async { Ok(deleted_state(a, mid).await? == "present") }, 16, 250).await
}

async fn own_reactions(a: &Agent, mid: &str) -> R<String> {
    let v = a.page().ok_or("no page")?.evaluate("message-actions.ownReactions", Some(&json!({ "s": sel(), "mid": mid }))).await?;
    Ok(crate::js::stringify(&v))
}

/// Adds a reaction, or removes it when it is already yours. True when your reactions on Teams changed.
pub async fn react(a: &Agent, chat: &str, mid: &str, emoji: &str) -> bool {
    let bar = table("bar_reactions", emoji);
    let picker = table("picker_reactions", emoji);
    if bar.is_none() && picker.is_none() {
        return false;
    }
    if let Some((auth, conv)) = fast(a, chat, Some(mid)).await
        && a.chatsvc.toggle_reaction(&a.http, &auth, &conv, mid, emoji).await
    {
        info(
            "react",
            "toggled reaction via chatsvc API",
            &[("chat", chat.into()), ("convId", conv.as_str().into()), ("mid", mid.into()), ("emoji", emoji.into())],
        );
        return true;
    }
    let Some(tp) = a.tp() else { return false };
    if !tp.to_chat(chat, false).await {
        return false;
    }
    let Ok(before) = own_reactions(a, mid).await else { return false };
    let attempt = async {
        let button = bar.clone().unwrap_or_else(|| table("actions", "picker").unwrap_or_default());
        if !tp.click_bar_button(mid, &button).await {
            warn("react", "button not found", &[("mid", mid.into()), ("emoji", emoji.into())]);
            return Ok(false);
        }
        if let Some(p) = &picker {
            tp.page.click(&format!(r#"[data-tid="{p}"]:visible"#), Pick::First, 4000).await?;
        }
        Ok::<bool, String>(true)
    };
    let outcome = attempt.await;
    tp.mouse_away().await;
    match outcome {
        Ok(true) => {}
        Ok(false) => return false,
        Err(e) => {
            warn("react", &e, &[("mid", mid.into()), ("emoji", emoji.into())]);
            let _ = tp.page.press("Escape").await;
            return false;
        }
    }
    if until(|| async { Ok(own_reactions(a, mid).await? != before) }, 12, 250).await {
        return true;
    }
    warn("react", "no change on Teams", &[("mid", mid.into()), ("emoji", emoji.into())]);
    false
}

/// A click on the pill of reaction `emoji` under the message, like in Teams: removed if yours, added otherwise.
pub async fn toggle_pill(a: &Agent, chat: &str, mid: &str, emoji: &str) -> bool {
    if emoji.is_empty() {
        return false;
    }
    if let Some((auth, conv)) = fast(a, chat, Some(mid)).await
        && a.chatsvc.toggle_reaction(&a.http, &auth, &conv, mid, emoji).await
    {
        info(
            "pill",
            "toggled reaction pill via chatsvc API",
            &[("chat", chat.into()), ("convId", conv.as_str().into()), ("mid", mid.into()), ("emoji", emoji.into())],
        );
        return true;
    }
    let Some(tp) = a.tp() else { return false };
    if !tp.to_chat(chat, false).await {
        return false;
    }
    let m = message_selector(mid);
    if tp.page.count(&m).await.unwrap_or(0) == 0 {
        return false;
    }
    let _ = tp.page.evaluate_on(&m, Pick::First, "message-actions.centerElement", None).await;
    sleep(400).await;
    let pill = || async { tp.page.evaluate("message-actions.reactionPill", Some(&json!({ "s": sel(), "mid": mid, "emoji": emoji }))).await };
    let was = pill().await.unwrap_or(Value::Null);
    if was["found"] != Value::Bool(true) {
        warn("pill", "reaction not found", &[("mid", mid.into()), ("emoji", emoji.into())]);
        return false;
    }
    if was["covered"] == Value::Bool(true) {
        warn("pill", "reaction covered by something else on the page (a call?): not clicked", &[("mid", mid.into()), ("emoji", emoji.into())]);
        return false;
    }
    let Some(at) = Point::of(&was) else { return false };
    if tp.page.mouse_click(at.x, at.y).await.is_err() {
        return false;
    }
    tp.mouse_away().await;
    let pressed = was["pressed"].clone();
    if until(
        || async {
            let now = pill().await?;
            Ok(!now.is_null() && (now["found"] != Value::Bool(true) || now["pressed"] != pressed))
        },
        12,
        250,
    )
    .await
    {
        return true;
    }
    tp.clear_overlays().await;
    warn("pill", "no change on Teams", &[("mid", mid.into()), ("emoji", emoji.into())]);
    false
}

/// Edits one of your messages. True when the text on Teams is the new one.
pub async fn edit_message(a: &Agent, chat: &str, mid: &str, raw: &str) -> bool {
    let text = js_trim(raw).to_string();
    if text.is_empty() {
        return false;
    }
    if let Some((auth, conv)) = fast(a, chat, Some(mid)).await {
        match a.chatsvc.edit_text(&a.http, &auth, &conv, mid, &text).await {
            Ok(true) => {
                info("edit", "edited via chatsvc API", &[("chat", chat.into()), ("convId", conv.as_str().into()), ("mid", mid.into())]);
                return true;
            }
            Ok(false) => {}
            Err(e) => warn(
                "edit",
                &format!("chatsvc fast path failed: {}, falling back to DOM", crate::agent::text_of(&e)),
                &[("chat", chat.into()), ("mid", mid.into())],
            ),
        }
    }
    let Some(tp) = a.tp() else { return false };
    if !tp.to_chat(chat, false).await {
        return false;
    }
    let item = format!("{}:has({})", s("item"), message_selector(mid));
    let page = &tp.page;
    let attempt = async {
        if !tp.click_bar_button(mid, &table("actions", "edit").unwrap_or_default()).await {
            warn("edit", "button not found", &[("mid", mid.into())]);
            return Ok(false);
        }
        let editor = format!("{item} {}", s("editor"));
        page.wait_for(&editor, Pick::First, 4000).await?;
        page.click(&editor, Pick::First, 4000).await?;
        page.press("Control+A").await?;
        page.press("Delete").await?;
        page.insert_text(&text).await?;
        sleep(200).await;
        page.click(&format!("{item} {}", s("editDone")), Pick::First, 3000).await?;
        Ok::<bool, String>(true)
    };
    let outcome = attempt.await;
    match outcome {
        Ok(true) => {}
        Ok(false) => {
            tp.mouse_away().await;
            return false;
        }
        Err(e) => {
            warn("edit", &e, &[("mid", mid.into())]);
            // the editor left open is discarded, the message stays as it was
            if page.click(&format!("{item} {}", s("editDiscard")), Pick::First, 2000).await.is_ok() {
                let _ = page.click(&s("discardConfirm"), Pick::First, 2000).await;
            }
            tp.mouse_away().await;
            return false;
        }
    }
    tp.mouse_away().await;
    let shown = || async {
        let v = page.evaluate("message-actions.messageBodyText", Some(&json!(mid))).await?;
        Ok(v.as_str().map(|t| js_trim(&t.replace('\u{fffd}', " ")).to_string()) == Some(text.clone()))
    };
    if until(shown, 16, 250).await {
        return true;
    }
    warn("edit", "text not updated on Teams", &[("mid", mid.into())]);
    false
}

/// Who read one of your messages: "Read by X of Y" in More options and the names of its submenu. A menu without that
/// entry is a 1:1 chat (label ""); None when the menu could not be read.
pub async fn read_receipts(a: &Agent, chat: &str, mid: &str) -> Option<(String, Vec<String>)> {
    let tp = a.tp()?;
    if !tp.to_chat(chat, false).await {
        return None;
    }
    let page = &tp.page;
    let entry = format!(r#"[data-tid="{}"]:visible"#, table("actions", "readReceipt").unwrap_or_default());
    let menu = format!("{}:visible", s("menu"));
    let more = table("actions", "more").unwrap_or_default();
    let run = async {
        let (mut found, mut menu_seen) = (false, false);
        for _ in 0..2 {
            if found {
                break;
            }
            if !tp.click_bar_button(mid, &more).await {
                if menu_seen {
                    break;
                }
                return Ok(None);
            }
            if page.wait_for(&menu, Pick::First, 3000).await.is_err() {
                tp.clear_overlays().await;
                continue;
            }
            menu_seen = true;
            if page.wait_for(&entry, Pick::First, 1500).await.is_ok() {
                found = true;
            } else {
                tp.clear_overlays().await;
                sleep(500).await;
            }
        }
        if !found {
            return Ok(menu_seen.then(|| (String::new(), vec![])));
        }
        let label = js_trim(&page.inner_text(&entry, Pick::First).await?).to_string();
        page.hover(&entry, Pick::First, 4000).await?;
        sleep(1000).await;
        let names = page.evaluate("message-actions.readReceiptNames", Some(&json!({ "s": sel(), "entry": table("actions", "readReceipt") }))).await?;
        Ok::<Option<(String, Vec<String>)>, String>(Some((label, names.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect())))
    };
    let out = match run.await {
        Ok(v) => v,
        Err(e) => {
            warn("readby", &e, &[("mid", mid.into())]);
            None
        }
    };
    tp.clear_overlays().await;
    tp.mouse_away().await;
    out
}

/// People of a chat as Teams names them: in a group chat the list its participant count opens, read and closed with
/// Escape (nothing in it is clicked); in the other chats the header. None when the chat or the list did not show.
pub async fn read_members(a: &Agent, chat: &str) -> Option<Vec<String>> {
    let tp = a.tp()?;
    if !tp.to_chat(chat, true).await {
        return None;
    }
    let page = &tp.page;
    let count = format!("{}:visible", s("participantCount"));
    let names = |v: Value| v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect::<Vec<_>>();
    if page.count(&count).await.unwrap_or(0) == 0 {
        return page.evaluate("members.topicNames", Some(&st(json!({})))).await.ok().map(names);
    }
    let run = async {
        page.click(&count, Pick::First, 3000).await?;
        page.wait_for(&s("rosterName"), Pick::First, 5000).await?;
        page.evaluate("members.rosterNames", Some(&sel())).await
    };
    let out = match run.await {
        Ok(v) => Some(names(v)),
        Err(e) => {
            warn("members", &e, &[("chat", chat.into())]);
            None
        }
    };
    tp.clear_overlays().await;
    out
}

async fn tagged_names(a: &Agent) -> R<Vec<String>> {
    let v = a.page().ok_or("no page")?.evaluate("mentions.composerMentionNames", Some(&sel())).await?;
    Ok(v.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect())
}

/// @ and the name word by word until Teams lists exactly that person, then a real click on the entry. True once the
/// compose box holds one more person tagged.
async fn tag_person(a: &Agent, name: &str) -> R<bool> {
    let page = a.page().ok_or("no page")?;
    let tagged = tagged_names(a).await?.len();
    let words: Vec<&str> = name.split_whitespace().collect();
    for i in 0..=words.len() {
        let piece = if i == 0 { "@".to_string() } else { format!("{}{}", if i > 1 { " " } else { "" }, words[i - 1]) };
        page.type_text(&piece, 40).await?;
        for _ in 0..8 {
            sleep(300).await;
            let point = page.evaluate("mentions.mentionOptionPoint", Some(&json!({ "s": sel(), "name": name }))).await?;
            let Some(p) = Point::of(&point) else { continue };
            page.mouse_click(p.x, p.y).await?;
            return Ok(until(|| async { Ok(tagged_names(a).await?.len() > tagged) }, 10, 200).await);
        }
    }
    warn("mention", "not in the list of Teams", &[("name", name.into())]);
    Ok(false)
}

/// The message with people tagged, as a person writes it; Enter once every person is tagged.
pub async fn send_with_mentions(a: &Agent, chat: &str, parts: &[Part], sent: AfterPress<'_>) -> SendResult {
    let Some(tp) = a.tp() else { return SendResult::Failed };
    if !tp.to_chat(chat, true).await {
        return SendResult::Failed;
    }
    if composer_left(a).await.unwrap_or(true) {
        warn("mention", "compose box not empty", &[("chat", chat.into())]);
        return SendResult::Failed;
    }
    let names: Vec<String> = parts.iter().filter_map(|p| if let Part::Mention(n) = p { Some(n.clone()) } else { None }).collect();
    let Ok(before) = message_ids(a).await else { return SendResult::Failed };
    let page = &tp.page;
    let mut pressed = false;
    let attempt = async {
        page.focus(&s("editor"), Pick::Last, 2000).await?;
        for part in parts {
            match part {
                Part::Text(t) => page.insert_text(t).await?,
                Part::Mention(n) => {
                    if !tag_person(a, n).await? {
                        return Err("message not composed".to_string());
                    }
                }
            }
        }
        let tagged = tagged_names(a).await?;
        if !names.iter().all(|n| tagged.contains(n)) {
            return Err("message not composed".to_string());
        }
        sleep(300).await;
        pressed = true;
        page.press("Enter").await
    };
    if let Err(e) = attempt.await {
        warn("mention", &e, &[("chat", chat.into())]);
        if pressed {
            return SendResult::Unconfirmed;
        }
        tp.empty_compose_box().await;
        return SendResult::Failed;
    }
    confirm_sent(a, "mention", "compose.ownMessageSent", st(json!({ "before": before, "names": names })), 40, &sent, vec![("chat", chat.into())]).await
}
