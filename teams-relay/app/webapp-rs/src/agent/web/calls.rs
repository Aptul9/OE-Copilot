//! Incoming calls in the browser (src/agent/jobs/calls.ts, logic/calls.ts, teams/call-actions.ts): the toast Teams
//! shows, pushed as soon as it rings and followed until it stops, on a timer of its own beside the loop; the answers,
//! hang-ups and mutes the app asks for; the call in progress while a Teams page records from the microphone.

use super::{Agent, FeedAfterCalls};
use crate::agent::browser::scripts::{s, sel, st};
use crate::agent::browser::{Page, Point, sleep};
use crate::agent::{info, now_ms, warn};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

pub const CALL_RING_EVERY: f64 = 5.0;
pub const CALL_RING_FOR: f64 = 60.0;
pub const CALL_END_AFTER: f64 = 2.0;
/// Milliseconds between two looks for a call and for the commands of calls
pub const CALL_LOOK_MS: u64 = 200;
const CONFIRM_TRIES: usize = 20;
const CONFIRM_EVERY: u64 = 250;
const SHORTCUT_AFTER: usize = 6;
pub const MUTE_KEY_TRIES: usize = 6;
pub const MIC_LOOK_CALL: f64 = 1.0;
pub const MIC_LOOK_EVERY: f64 = 5.0;
pub const ANSWERED_WATCH: f64 = 60.0;
/// Seconds between two writes of a call still ringing or in progress (CALL_SEEN_EVERY of the state)
const CALL_SEEN_EVERY: f64 = 5.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Ended {
    pub caller: String,
    pub since: i64,
    pub seconds: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CallEvent {
    Ringing { caller: String, since: i64, again: bool, replaced: Option<Ended> },
    Ended(Ended),
}

struct Tracked {
    caller: String,
    since: i64,
    start: f64,
    rang: f64,
    seen: f64,
    gone: f64,
}

/// An incoming call as the agent follows it from the toast: durations on a clock that only goes forward (ms).
pub struct CallTracker {
    call: Option<Tracked>,
    clock: Box<dyn Fn() -> f64>,
    wall: Box<dyn Fn() -> i64>,
}

impl CallTracker {
    pub fn new(clock: Box<dyn Fn() -> f64>, wall: Box<dyn Fn() -> i64>) -> CallTracker {
        CallTracker { call: None, clock, wall }
    }

    /// What to push for what the page shows now: the caller of the call ringing, or none.
    pub fn update(&mut self, shown: Option<&str>) -> Option<CallEvent> {
        let now = (self.clock)();
        let secs = |c: &Tracked| ((c.seen - c.start) / 1000.0).round() as i64;
        if let Some(caller) = shown {
            let other = self.call.as_ref().is_some_and(|c| !c.caller.is_empty() && !caller.is_empty() && c.caller != caller);
            if self.call.is_none() || other {
                let since = (self.wall)();
                let replaced = self.call.as_ref().map(|c| Ended { caller: c.caller.clone(), since: c.since, seconds: secs(c) });
                self.call = Some(Tracked { caller: caller.into(), since, start: now, rang: now, seen: now, gone: -1.0 });
                return Some(CallEvent::Ringing { caller: caller.into(), since, again: false, replaced });
            }
            let c = self.call.as_mut()?;
            if c.caller.is_empty() {
                c.caller = caller.into();
            }
            c.seen = now;
            c.gone = -1.0;
            if now - c.rang < CALL_RING_EVERY * 1000.0 || now - c.start >= CALL_RING_FOR * 1000.0 {
                return None;
            }
            c.rang = now;
            return Some(CallEvent::Ringing { caller: c.caller.clone(), since: c.since, again: true, replaced: None });
        }
        let c = self.call.as_mut()?;
        if c.gone < 0.0 {
            c.gone = now;
        }
        if now - c.gone < CALL_END_AFTER * 1000.0 {
            return None;
        }
        let c = self.call.take()?;
        Some(CallEvent::Ended(Ended { caller: c.caller.clone(), since: c.since, seconds: secs(&c) }))
    }

    /// The call whose toast shows now, for as long as it is pushed.
    pub fn current(&self) -> Option<(String, i64)> {
        let c = self.call.as_ref()?;
        (c.gone < 0.0 && (self.clock)() - c.start < CALL_RING_FOR * 1000.0).then(|| (c.caller.clone(), c.since))
    }
}

/// The Teams pages of the browser: the one the agent drives and the others (a call window of its own).
async fn teams_pages(page: &Rc<Page>) -> Vec<Rc<Page>> {
    let Some(b) = page.browser() else { return vec![page.clone()] };
    let mut out = vec![];
    for p in b.pages().await {
        if Rc::ptr_eq(&p, page) || (!p.is_closed() && super::jobs::is_teams_url(&p.url())) {
            out.push(p);
        }
    }
    if !out.iter().any(|p| Rc::ptr_eq(p, page)) {
        out.insert(0, page.clone());
    }
    out
}

/// The Teams page that records from the microphone now, in any of its frames: a call is in progress.
pub async fn recording_page(page: &Rc<Page>) -> Option<Rc<Page>> {
    for p in teams_pages(page).await {
        for f in p.frames() {
            if p.evaluate_in(&f, "calls.micLive", None).await.is_ok_and(|v| v == Value::Bool(true)) {
                return Some(p);
            }
        }
    }
    None
}

/// Teams' own mute from the microphone button on screen in any frame of any Teams page; None where none shows one, or
/// where two read differently.
async fn call_mic(page: &Rc<Page>) -> Option<(Rc<Page>, bool)> {
    let mut found: Option<(Rc<Page>, bool)> = None;
    for p in teams_pages(page).await {
        for f in p.frames() {
            let Some(muted) = p.evaluate_in(&f, "calls.micMuted", Some(&sel())).await.ok().and_then(|v| v.as_bool()) else { continue };
            if found.as_ref().is_some_and(|x| x.1 != muted) {
                return None;
            }
            if found.is_none() {
                found = Some((p.clone(), muted));
            }
        }
    }
    found
}

/// A real click on the first element of `sel` on screen, at a point nothing covers; false when none shows.
async fn real_click(page: &Page, selector: &str) -> bool {
    page.with_input(async {
        let Some(at) = page.evaluate("page-state.uncoveredPoint", Some(&json!(selector))).await.ok().and_then(|v| Point::of(&v)) else { return false };
        page.mouse_click(at.x, at.y).await.is_ok()
    })
    .await
}

async fn keys(page: &Page, combo: &str) {
    let _ = page.with_input(page.press(combo)).await;
}

/// A call asked from the app: Teams web's shortcut for an audio call from the open chat (Alt+Shift+A), pressed only when
/// no call rings, read inside the input lock right before the keys (the same keys accept a ringing call as video).
pub async fn start_audio_call(a: &Agent, page: &Page) -> bool {
    page.with_input(async {
        let ringing = page.evaluate("calls.readIncomingCall", Some(&st(json!({})))).await.map(|v| !v.is_null()).unwrap_or(true);
        if a.ringing.get() || a.in_call.get() || ringing {
            return false;
        }
        page.press("Alt+Shift+KeyA").await.is_ok()
    })
    .await
}

struct InCall {
    caller: String,
    since: i64,
    written: f64,
    muted: Option<bool>,
    held: bool,
}

/// The watch of calls: one look at a time, every CALL_LOOK_MS.
pub struct CallWatch {
    a: Rc<Agent>,
    tracker: RefCell<CallTracker>,
    busy: Cell<bool>,
    warned: Cell<f64>,
    written: Cell<f64>,
    last: RefCell<Option<(String, i64)>>,
    answered: Cell<i64>,
    answered_at: Cell<f64>,
    mic_looked: Cell<f64>,
    in_call: RefCell<Option<InCall>>,
    ended: Rc<FeedAfterCalls>,
    clock: Rc<dyn Fn() -> f64>,
}

impl CallWatch {
    pub fn start(a: Rc<Agent>, ended: Rc<FeedAfterCalls>) {
        let origin = Instant::now();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || origin.elapsed().as_secs_f64() * 1000.0);
        let c = clock.clone();
        let w = Rc::new(CallWatch {
            a,
            tracker: RefCell::new(CallTracker::new(Box::new(move || c()), Box::new(now_ms))),
            busy: Cell::new(false),
            warned: Cell::new(0.0),
            written: Cell::new(f64::MIN),
            last: RefCell::new(None),
            answered: Cell::new(0),
            answered_at: Cell::new(f64::MIN),
            mic_looked: Cell::new(f64::MIN),
            in_call: RefCell::new(None),
            ended,
            clock,
        });
        tokio::task::spawn_local(async move {
            let mut t = tokio::time::interval(std::time::Duration::from_millis(CALL_LOOK_MS));
            loop {
                t.tick().await;
                let w = w.clone();
                tokio::task::spawn_local(async move { w.tick().await });
            }
        });
    }

    fn now(&self) -> f64 {
        (self.clock)()
    }

    async fn tick(&self) {
        let Some(page) = self.a.page() else { return };
        if self.busy.get() || page.is_closed() || !super::jobs::is_teams_url(&page.url()) {
            return;
        }
        self.busy.set(true);
        let r = async {
            self.look_at_toast(&page).await?;
            self.run_call_commands(&page).await;
            if self.microphone_due() {
                self.watch_microphone(&page).await;
            }
            Ok::<(), String>(())
        };
        if let Err(e) = r.await
            && (self.warned.get() == 0.0 || self.now() - self.warned.get() > 60_000.0)
        {
            warn("call", &e, &[]);
            self.warned.set(self.now());
        }
        self.busy.set(false);
    }

    async fn read_toast(page: &Page) -> Result<Option<String>, String> {
        let v = page.evaluate("calls.readIncomingCall", Some(&st(json!({})))).await?;
        Ok((!v.is_null()).then(|| v["caller"].as_str().unwrap_or("").to_string()))
    }

    async fn look_at_toast(&self, page: &Page) -> Result<(), String> {
        let shown = Self::read_toast(page).await?;
        let event = self.tracker.borrow_mut().update(shown.as_deref());
        let current = self.tracker.borrow().current();
        let taken = current.as_ref().is_some_and(|c| c.1 == self.answered.get());
        // the loop keeps off the page while a call rings: an answer clicks there
        self.a.ringing.set(current.is_some() && !taken);
        match &event {
            Some(CallEvent::Ringing { caller, since, again, replaced }) => {
                if let Some(r) = replaced {
                    self.a.store.add_call(&r.caller, r.since, r.seconds);
                    self.ended.call_ended();
                }
                if !again {
                    info("call", "ringing", &[("caller", caller.as_str().into())]);
                    self.keep(caller, *since, true);
                }
                if !(*again && *since == self.answered.get()) {
                    self.push(caller.clone(), if *again { "again" } else { "ringing" }, *since, 0, false);
                }
            }
            Some(CallEvent::Ended(e)) => {
                let answered = e.since == self.answered.get();
                let mut fields = vec![("caller", e.caller.as_str().into()), ("seconds", e.seconds.into())];
                if answered {
                    fields.push(("answered", true.into()));
                }
                info("call", "ended", &fields);
                self.keep(&e.caller, e.since, false);
                self.a.store.add_call(&e.caller, e.since, e.seconds);
                self.push(e.caller.clone(), "ended", e.since, e.seconds, answered);
                self.ended.call_ended();
            }
            None => {}
        }
        if let Some(c) = &current {
            *self.last.borrow_mut() = Some(c.clone());
            if !taken && !matches!(event, Some(CallEvent::Ended(_))) && self.now() - self.written.get() >= CALL_SEEN_EVERY * 1000.0 {
                self.keep(&c.0, c.1, true);
            }
        }
        Ok(())
    }

    /// The answers, hang-ups and mutes the app queued, oldest first; never marked running.
    async fn run_call_commands(&self, page: &Rc<Page>) {
        for cmd in self.a.store.pending_commands() {
            if !["answer", "hangup", "mute"].contains(&cmd.kind.as_str()) {
                continue;
            }
            let mut fields = vec![("id", cmd.id.into())];
            if !cmd.arg1.is_empty() {
                fields.push(("arg", cmd.arg1.as_str().into()));
            }
            info("CMD", &cmd.kind, &fields);
            let args = crate::state::object(Some(&cmd.arg2)).unwrap_or_default();
            let outcome = match cmd.kind.as_str() {
                "answer" => self.answer(page, args.get("since").and_then(Value::as_i64).unwrap_or(0)).await,
                "hangup" => self.hang_up(page).await,
                _ => self.set_mute(page, args.get("on").and_then(Value::as_bool)).await,
            };
            self.a.store.finish_command(cmd.id, outcome);
        }
    }

    /// Only the call of that since, only while its toast shows: a real click on Accept with audio, Teams' Accept shortcut
    /// where none can be clicked or the toast stays; answered once the toast is gone or a page records.
    async fn answer(&self, page: &Rc<Page>, since: i64) -> &'static str {
        let ringing = self.tracker.borrow().current();
        let Some((caller, ring_since)) = ringing.filter(|r| since != 0 && r.1 == since) else {
            warn("call", "answer: that call no longer rings", &[("since", since.into())]);
            return "failed";
        };
        let mut shortcut = !real_click(page, &s("callAccept")).await;
        if shortcut {
            warn("call", "answer: no part of Accept to click, pressing the shortcut", &[("caller", caller.as_str().into())]);
            keys(page, "Alt+Shift+KeyS").await;
        }
        for i in 0..CONFIRM_TRIES {
            if Self::read_toast(page).await.is_ok_and(|t| t.is_none()) || recording_page(page).await.is_some() {
                self.answered.set(ring_since);
                self.answered_at.set(self.now());
                info("call", "answered", &[("caller", caller.as_str().into()), ("by", if shortcut { "shortcut" } else { "click" }.into())]);
                self.keep(&caller, since, false);
                self.a.ringing.set(false);
                self.mic_looked.set(f64::MIN);
                return "done";
            }
            if !shortcut && i + 1 == SHORTCUT_AFTER {
                warn("call", "answer: the toast stayed after the click, pressing the shortcut", &[("caller", caller.as_str().into())]);
                keys(page, "Alt+Shift+KeyS").await;
                shortcut = true;
            }
            sleep(CONFIRM_EVERY).await;
        }
        warn("call", "answer: the call still rings after the click and the shortcut", &[("caller", caller.as_str().into())]);
        "failed"
    }

    /// Teams' hang-up shortcut on the page that records; done once it stops.
    async fn hang_up(&self, page: &Rc<Page>) -> &'static str {
        let recording = recording_page(page).await;
        let target = match &recording {
            Some(p) => Some(p.clone()),
            None if self.in_call.borrow().is_some() => call_mic(page).await.map(|m| m.0),
            None => None,
        };
        let Some(target) = target else {
            warn("call", "hangup: no call in progress", &[]);
            return "failed";
        };
        keys(&target, "Control+Shift+H").await;
        for _ in 0..CONFIRM_TRIES {
            let over = if recording.is_some() { recording_page(page).await.is_none() } else { call_mic(page).await.is_none() };
            if over {
                info("call", "hung up", &[]);
                self.mic_looked.set(f64::MIN);
                return "done";
            }
            sleep(CONFIRM_EVERY).await;
        }
        warn(
            "call",
            if recording.is_some() { "hangup: the microphone stayed on after the shortcut" } else { "hangup: the call stayed on screen after the shortcut" },
            &[],
        );
        "failed"
    }

    async fn mute_shows(&self, page: &Rc<Page>, on: bool, tries: usize) -> bool {
        for _ in 0..tries {
            sleep(CONFIRM_EVERY).await;
            if call_mic(page).await.is_some_and(|m| m.1 == on) {
                return true;
            }
        }
        false
    }

    fn mute_shown(&self, on: bool, by: &str) -> &'static str {
        info("call", if on { "muted in Teams" } else { "unmuted in Teams" }, &[("by", by.into())]);
        let mut c = self.in_call.borrow_mut();
        if let Some(c) = c.as_mut() {
            c.muted = Some(on);
            c.written = self.now();
            self.keep_in_call(&c.caller, c.since, true, Some(on));
        }
        "done"
    }

    /// Teams' own mute as the app asks for it: the shortcut only when Teams shows the other state, the microphone button
    /// clicked where the shortcut changed nothing; nothing pressed on a state that cannot be read.
    async fn set_mute(&self, page: &Rc<Page>, on: Option<bool>) -> &'static str {
        let Some(on) = on else {
            warn("call", "mute: no state asked for, nothing pressed", &[]);
            return "failed";
        };
        if self.in_call.borrow().is_none() {
            warn("call", "mute: no call in progress", &[("on", on.into())]);
            return "failed";
        }
        let Some((mic_page, muted)) = call_mic(page).await else {
            warn("call", "mute: Teams' microphone button cannot be read, nothing pressed", &[("on", on.into())]);
            return "failed";
        };
        if muted == on {
            return self.mute_shown(on, "already");
        }
        keys(&mic_page, "Control+Shift+M").await;
        if self.mute_shows(page, on, MUTE_KEY_TRIES).await {
            return self.mute_shown(on, "shortcut");
        }
        let Some((again_page, again)) = call_mic(page).await else {
            warn("call", "mute: Teams' microphone button cannot be read after the shortcut, nothing more pressed", &[("on", on.into())]);
            return "failed";
        };
        if again == on {
            return self.mute_shown(on, "shortcut");
        }
        warn("call", "mute: the shortcut changed nothing, clicking the microphone button", &[("on", on.into())]);
        if !real_click(&again_page, &s("callMic")).await {
            warn("call", "mute: no part of the microphone button to click", &[("on", on.into())]);
            return "failed";
        }
        if self.mute_shows(page, on, CONFIRM_TRIES - MUTE_KEY_TRIES).await {
            return self.mute_shown(on, "click");
        }
        warn("call", "mute: Teams' microphone button stayed as it was after the shortcut and the click", &[("on", on.into())]);
        "failed"
    }

    /// Every frame read every MIC_LOOK_CALL seconds while a call rings, was just answered or is in progress, otherwise
    /// every MIC_LOOK_EVERY; a call being placed from the app at every look.
    fn microphone_due(&self) -> bool {
        let now = self.now();
        let placing_old = self.a.outgoing.borrow().as_ref().is_some_and(|o| now_ms() - o.1 >= (ANSWERED_WATCH * 1000.0) as i64);
        if placing_old && self.in_call.borrow().is_none() {
            *self.a.outgoing.borrow_mut() = None;
        }
        let placing = self.a.outgoing.borrow().is_some() && self.in_call.borrow().is_none();
        let watching = self.in_call.borrow().is_some() || self.tracker.borrow().current().is_some() || now - self.answered_at.get() < ANSWERED_WATCH * 1000.0;
        if !placing && now - self.mic_looked.get() < if watching { MIC_LOOK_CALL } else { MIC_LOOK_EVERY } * 1000.0 {
            return false;
        }
        self.mic_looked.set(now);
        true
    }

    /// The call in progress is named after the one placed from the app when it came after the last one that rang.
    fn named(&self) -> Option<(String, i64)> {
        let out = self.a.outgoing.borrow().clone();
        let last = self.last.borrow().clone();
        match out {
            Some((callee, since)) if since > last.as_ref().map_or(0, |l| l.1) => Some((callee, since)),
            _ => last,
        }
    }

    async fn watch_microphone(&self, page: &Rc<Page>) {
        let recording = recording_page(page).await.is_some();
        let was = self.in_call.borrow().is_some();
        let mic = if recording || was { call_mic(page).await } else { None };
        let live = recording || (was && mic.as_ref().is_some_and(|m| m.1));
        self.a.in_call.set(live);
        if live {
            let muted = mic.map(|m| m.1);
            let from = self.named();
            if !was {
                let mute = match muted {
                    None => "unreadable",
                    Some(true) => "on",
                    Some(false) => "off",
                };
                let mut fields = vec![("mute", mute.into())];
                if let Some((c, _)) = from.as_ref().filter(|f| !f.0.is_empty()) {
                    fields.insert(0, ("caller", c.as_str().into()));
                }
                info("call", "in progress", &fields);
                *self.in_call.borrow_mut() = Some(InCall {
                    caller: from.as_ref().map(|f| f.0.clone()).unwrap_or_default(),
                    since: from.as_ref().map_or(now_ms(), |f| f.1),
                    written: f64::MIN,
                    muted,
                    held: false,
                });
            }
            let mut slot = self.in_call.borrow_mut();
            let Some(c) = slot.as_mut() else { return };
            if !recording && !c.held {
                c.held = true;
                info("call", "no microphone track while muted in Teams: the call stays in progress", &[]);
            }
            let changed = c.muted != muted;
            if changed {
                match muted {
                    None => warn("call", "Teams' microphone button cannot be read", &[]),
                    Some(m) => info("call", if m { "muted in Teams" } else { "unmuted in Teams" }, &[]),
                }
            }
            c.muted = muted;
            if !changed && self.now() - c.written < CALL_SEEN_EVERY * 1000.0 {
                return;
            }
            c.written = self.now();
            self.keep_in_call(&c.caller, c.since, true, muted);
        } else if was {
            let Some(c) = self.in_call.borrow_mut().take() else { return };
            self.a.call_over_at.set(Some(now_ms()));
            *self.a.outgoing.borrow_mut() = None;
            let mut fields = vec![];
            if !c.caller.is_empty() {
                fields.push(("caller", c.caller.as_str().into()));
            }
            info("call", "over", &fields);
            self.keep_in_call(&c.caller, c.since, false, None);
        }
    }

    /// The pushes of a call, one after the other in the background: a slow push service holds up neither the looks nor the
    /// web app.
    fn push(&self, caller: String, state: &'static str, since: i64, seconds: i64, answered: bool) {
        let n = self.a.notifier.clone();
        tokio::task::spawn_local(async move {
            n.call(&caller, state, since, seconds, answered).await;
        });
    }

    fn keep(&self, caller: &str, since: i64, ringing: bool) {
        self.written.set(self.now());
        let call = json!({ "caller": caller, "since": since, "seen": now_ms(), "ringing": ringing });
        self.a.store.set_state(crate::state::key::CALL, &crate::js::stringify(&call));
    }

    fn keep_in_call(&self, caller: &str, since: i64, active: bool, muted: Option<bool>) {
        let mut c = json!({ "caller": caller, "since": since, "seen": now_ms(), "active": active });
        if let Some(m) = muted {
            c["muted"] = json!(m);
        }
        self.a.store.set_state(crate::state::key::IN_CALL, &crate::js::stringify(&c));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_rings_rings_again_and_ends() {
        let t = Rc::new(Cell::new(0.0));
        let c = t.clone();
        let mut tr = CallTracker::new(Box::new(move || c.get()), Box::new(|| 1000));
        assert!(matches!(tr.update(Some("Anna")), Some(CallEvent::Ringing { again: false, .. })));
        t.set(3000.0);
        assert_eq!(tr.update(Some("")), None);
        t.set(5000.0);
        assert!(matches!(tr.update(Some("Anna")), Some(CallEvent::Ringing { again: true, .. })));
        t.set(6000.0);
        assert_eq!(tr.update(None), None);
        assert_eq!(tr.current(), None);
        t.set(8000.0);
        assert_eq!(tr.update(None), Some(CallEvent::Ended(Ended { caller: "Anna".into(), since: 1000, seconds: 5 })));
        assert!(matches!(tr.update(Some("Luca")), Some(CallEvent::Ringing { replaced: None, .. })));
        t.set(9000.0);
        assert!(matches!(tr.update(Some("Marco")), Some(CallEvent::Ringing { replaced: Some(Ended { seconds: 0, .. }), .. })));
    }
}
