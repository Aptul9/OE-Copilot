//! The browser of a slot as the agent drives it, over the DevTools client of crate::cdp, in place of Playwright: the tabs
//! with their addresses (Target events), a page with its frames (execution contexts, iframe sessions), scripts run in it
//! with an argument, real mouse and keyboard input (Input), the locators the agent uses (CSS with Playwright's :visible,
//! first or last, a click that waits until the element is visible and under the pointer), init scripts, navigation and
//! the cookies of the browser.

pub mod keys;
pub mod scripts;

use crate::cdp::Cdp;
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

pub type R<T> = Result<T, String>;

pub async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// True as soon as `check` is, asked `tries` times `ms` apart; a check that fails counts as false (until of page.ts).
pub async fn until<F, Fut>(check: F, tries: usize, ms: u64) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = R<bool>>,
{
    for _ in 0..tries {
        sleep(ms).await;
        if check().await.unwrap_or(false) {
            return true;
        }
    }
    false
}

#[derive(Clone, Debug, Default)]
struct Target {
    kind: String,
    url: String,
}

/// The browser, while the connection holds.
pub struct Browser {
    cdp: Rc<Cdp>,
    targets: Rc<RefCell<IndexMap<String, Target>>>,
    pages: RefCell<HashMap<String, Rc<Page>>>,
    /// scripts for every page and frame to come (Playwright's context.addInitScript)
    everywhere: RefCell<Vec<String>>,
    me: Weak<Browser>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Browser {
    /// The browser whose DevTools answer at `url` (CDP of the slot).
    pub async fn connect(url: &str) -> R<Rc<Browser>> {
        let cdp = Cdp::connect_url(url).await?;
        let mut events = cdp.subscribe();
        let targets: Rc<RefCell<IndexMap<String, Target>>> = Rc::default();
        let t = targets.clone();
        let task = tokio::task::spawn_local(async move {
            while let Some(e) = events.recv().await {
                if e.get("sessionId").is_some() {
                    continue;
                }
                let info = &e["params"]["targetInfo"];
                let id = info["targetId"].as_str().unwrap_or("").to_string();
                match e["method"].as_str().unwrap_or("") {
                    "Target.targetCreated" | "Target.targetInfoChanged" if !id.is_empty() => {
                        let target = Target { kind: info["type"].as_str().unwrap_or("").into(), url: info["url"].as_str().unwrap_or("").into() };
                        t.borrow_mut().insert(id, target);
                    }
                    "Target.targetDestroyed" => {
                        if let Some(id) = e["params"]["targetId"].as_str() {
                            t.borrow_mut().shift_remove(id);
                        }
                    }
                    _ => {}
                }
            }
        });
        let b = Rc::new_cyclic(|me| Browser { cdp, targets, pages: RefCell::default(), everywhere: RefCell::default(), me: me.clone(), task });
        b.cdp.send("Target.setDiscoverTargets", json!({ "discover": true }), None).await?;
        for (id, url) in b.cdp.pages().await? {
            b.targets.borrow_mut().entry(id).or_insert(Target { kind: "page".into(), url });
        }
        Ok(b)
    }

    pub fn connected(&self) -> bool {
        self.cdp.connected()
    }

    pub fn cdp(&self) -> &Rc<Cdp> {
        &self.cdp
    }

    /// The tabs, in the order they came: id and address.
    pub fn tabs(&self) -> Vec<(String, String)> {
        self.targets.borrow().iter().filter(|(_, t)| t.kind == "page").map(|(id, t)| (id.clone(), t.url.clone())).collect()
    }

    pub fn url_of(&self, target: &str) -> Option<String> {
        self.targets.borrow().get(target).map(|t| t.url.clone())
    }

    /// The page of a tab, attached once.
    pub async fn page(&self, target: &str) -> R<Rc<Page>> {
        if let Some(p) = self.pages.borrow().get(target).filter(|p| !p.is_closed()).cloned() {
            return Ok(p);
        }
        let p = Page::attach(self, target).await?;
        self.pages.borrow_mut().insert(target.to_string(), p.clone());
        Ok(p)
    }

    /// The pages of the tabs open now, attached.
    pub async fn pages(&self) -> Vec<Rc<Page>> {
        let mut out = vec![];
        for (id, _) in self.tabs() {
            if let Ok(p) = self.page(&id).await {
                out.push(p);
            }
        }
        out
    }

    /// A new tab on `url`.
    pub async fn new_page(&self, url: &str) -> R<Rc<Page>> {
        let r = self.cdp.send("Target.createTarget", json!({ "url": url }), None).await?;
        let id = r["targetId"].as_str().ok_or("no target")?.to_string();
        self.targets.borrow_mut().entry(id.clone()).or_insert(Target { kind: "page".into(), url: url.into() });
        self.page(&id).await
    }

    /// A script for every page and frame to come (installMicHook), and for those attached now.
    pub async fn add_init_script_everywhere(&self, source: &str) -> R<()> {
        self.everywhere.borrow_mut().push(source.to_string());
        let pages: Vec<Rc<Page>> = self.pages.borrow().values().cloned().collect();
        for p in pages {
            for s in p.sessions() {
                let _ = self.cdp.send("Page.addScriptToEvaluateOnNewDocument", json!({ "source": source }), Some(&s)).await;
            }
        }
        Ok(())
    }

    /// The cookies of the browser, as Playwright gave them.
    pub async fn cookies(&self) -> R<Vec<Value>> {
        self.cdp.cookies().await
    }

    /// Cookies set in the browser (Storage.setCookies params).
    pub async fn set_cookies(&self, cookies: Vec<Value>) -> R<()> {
        if !cookies.is_empty() {
            self.cdp.send("Storage.setCookies", json!({ "cookies": cookies }), None).await?;
        }
        Ok(())
    }

    /// Notifications only, in the default context: Browser.grantPermissions would deny every permission it does not list
    /// (the microphone of a call included); the grant lasts as long as this connection.
    pub async fn grant_notifications(&self) -> R<()> {
        self.cdp.send("Browser.setPermission", json!({ "permission": { "name": "notifications" }, "setting": "granted" }), None).await.map(drop)
    }

    pub async fn close_browser(&self) -> R<()> {
        self.cdp.send("Browser.close", json!({}), None).await.map(drop)
    }
}

/// Which of the elements of a selector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    First,
    Last,
}

impl Pick {
    fn js(self) -> &'static str {
        match self {
            Pick::First => "0",
            Pick::Last => "-1",
        }
    }
}

/// A point of the page, in CSS pixels of the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    /// {x, y} of a page script; None for null.
    pub fn of(v: &Value) -> Option<Point> {
        Some(Point { x: v.get("x")?.as_f64()?, y: v.get("y")?.as_f64()? })
    }
}

/// The elements of a selector in the page: CSS, with Playwright's :visible at its end (a box of some size and not
/// hidden), and the one of `pick`.
const FIND: &str = r#"(sel, pick) => {
  let visible = false;
  if (sel.endsWith(":visible")) { visible = true; sel = sel.slice(0, -8); }
  let all = [...document.querySelectorAll(sel)];
  if (visible) all = all.filter((e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== "hidden"; });
  return pick === null ? all : all.at(pick) ?? null;
}"#;

type Contexts = Rc<RefCell<HashMap<i64, String>>>;

/// The page of a tab: its own session, with the frames of the page in it and the iframes in sessions of their own.
pub struct Page {
    browser: Weak<Browser>,
    cdp: Rc<Cdp>,
    target: String,
    session: String,
    /// execution contexts of frames, default world: id to frame id
    contexts: Contexts,
    /// sessions of iframes in other processes
    children: Rc<RefCell<Vec<String>>>,
    closed: Rc<Cell<bool>>,
    mouse: Cell<(f64, f64)>,
    /// one sequence of real input at a time (withInput of src/agent/teams/input.ts)
    input: tokio::sync::Mutex<()>,
    /// when the agent sent input (asAgent): id, from, to (ms), kept KEEP_MS
    spans: RefCell<Vec<(u64, i64, i64)>>,
    span_seq: Cell<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Page {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Page {
    async fn attach(b: &Browser, target: &str) -> R<Rc<Page>> {
        let cdp = b.cdp.clone();
        let attached = cdp.send("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await?;
        let session = attached["sessionId"].as_str().ok_or("no session")?.to_string();
        let mut events = cdp.subscribe();
        let contexts: Contexts = Rc::default();
        let children: Rc<RefCell<Vec<String>>> = Rc::default();
        let closed: Rc<Cell<bool>> = Rc::default();
        let (c, k, d, s, t, cd) = (contexts.clone(), children.clone(), closed.clone(), session.clone(), target.to_string(), cdp.clone());
        let everywhere = b.everywhere.borrow().clone();
        let task = tokio::task::spawn_local(async move {
            while let Some(e) = events.recv().await {
                let from = e["sessionId"].as_str().unwrap_or("");
                let p = &e["params"];
                match e["method"].as_str().unwrap_or("") {
                    "Runtime.executionContextCreated" if from == s => {
                        let ctx = &p["context"];
                        if ctx["auxData"]["isDefault"] == Value::Bool(true)
                            && let Some(id) = ctx["id"].as_i64()
                        {
                            c.borrow_mut().insert(id, ctx["auxData"]["frameId"].as_str().unwrap_or("").to_string());
                        }
                    }
                    "Runtime.executionContextDestroyed" if from == s => {
                        if let Some(id) = p["executionContextId"].as_i64() {
                            c.borrow_mut().remove(&id);
                        }
                    }
                    "Runtime.executionContextsCleared" if from == s => c.borrow_mut().clear(),
                    "Target.attachedToTarget" if from == s && p["targetInfo"]["type"] == "iframe" => {
                        let child = p["sessionId"].as_str().unwrap_or("").to_string();
                        for src in &everywhere {
                            let _ = cd.send("Page.addScriptToEvaluateOnNewDocument", json!({ "source": src }), Some(&child)).await;
                        }
                        let _ = cd.send("Runtime.runIfWaitingForDebugger", json!({}), Some(&child)).await;
                        k.borrow_mut().push(child);
                    }
                    "Target.detachedFromTarget" => {
                        let gone = p["sessionId"].as_str().unwrap_or("");
                        if gone == s {
                            d.set(true);
                        } else {
                            k.borrow_mut().retain(|x| x != gone);
                        }
                    }
                    "Target.targetDestroyed" if p["targetId"].as_str() == Some(t.as_str()) => d.set(true),
                    _ => {}
                }
            }
            d.set(true);
        });
        let page = Rc::new(Page {
            browser: b.me.clone(),
            cdp: cdp.clone(),
            target: target.to_string(),
            session: session.clone(),
            contexts,
            children,
            closed,
            mouse: Cell::new((0.0, 0.0)),
            input: tokio::sync::Mutex::new(()),
            spans: RefCell::default(),
            span_seq: Cell::new(0),
            task,
        });
        cdp.send("Runtime.enable", json!({}), Some(&session)).await?;
        cdp.send("Target.setAutoAttach", json!({ "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true }), Some(&session)).await?;
        let everywhere = b.everywhere.borrow().clone();
        for src in &everywhere {
            cdp.send("Page.addScriptToEvaluateOnNewDocument", json!({ "source": src }), Some(&session)).await?;
        }
        Ok(page)
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn is_closed(&self) -> bool {
        self.closed.get() || !self.cdp.connected()
    }

    /// The sessions of the page: its own, and those of its iframes in other processes.
    fn sessions(&self) -> Vec<String> {
        std::iter::once(self.session.clone()).chain(self.children.borrow().iter().cloned()).collect()
    }

    pub fn browser(&self) -> Option<Rc<Browser>> {
        self.browser.upgrade()
    }

    /// The address of the page now ("" when unknown).
    pub fn url(&self) -> String {
        self.browser.upgrade().and_then(|b| b.url_of(&self.target)).unwrap_or_default()
    }

    async fn send(&self, method: &str, params: Value) -> R<Value> {
        self.cdp.send(method, params, Some(&self.session)).await
    }

    async fn run(&self, expression: &str, session: &str, context: Option<i64>) -> R<Value> {
        let mut params = json!({ "expression": expression, "returnByValue": true, "awaitPromise": true });
        if let Some(c) = context {
            params["contextId"] = json!(c);
        }
        let r = self.cdp.send("Runtime.evaluate", params, Some(session)).await?;
        if let Some(x) = r.get("exceptionDetails") {
            let why = x["exception"]["description"].as_str().or(x["text"].as_str()).unwrap_or("exception");
            return Err(why.lines().next().unwrap_or(why).to_string());
        }
        Ok(r["result"]["value"].clone())
    }

    /// An expression in the main frame, its value.
    pub async fn eval_js(&self, expression: &str) -> R<Value> {
        self.run(expression, &self.session, None).await
    }

    /// Script `name` of page-scripts.json with `arg`, in the main frame.
    pub async fn evaluate(&self, name: &str, arg: Option<&Value>) -> R<Value> {
        self.eval_js(&scripts::call(name, arg)).await
    }

    /// The frames of the page, the main one included: where an evaluate in every frame runs.
    pub fn frames(&self) -> Vec<Frame> {
        let mut out: Vec<Frame> = self.contexts.borrow().keys().map(|c| Frame { session: self.session.clone(), context: Some(*c) }).collect();
        if out.is_empty() {
            out.push(Frame { session: self.session.clone(), context: None });
        }
        out.extend(self.children.borrow().iter().map(|s| Frame { session: s.clone(), context: None }));
        out
    }

    /// Script `name` in one frame.
    pub async fn evaluate_in(&self, frame: &Frame, name: &str, arg: Option<&Value>) -> R<Value> {
        self.run(&scripts::call(name, arg), &frame.session, frame.context).await
    }

    /// A script for the documents to come in this page (Playwright's page.addInitScript).
    pub async fn add_init_script(&self, name: &str) -> R<()> {
        self.send("Page.addScriptToEvaluateOnNewDocument", json!({ "source": scripts::call(name, None) })).await.map(drop)
    }

    /// Polls `expression` until it is truthy, `timeout_ms` at most (page.waitForFunction).
    pub async fn wait_for_js(&self, expression: &str, timeout_ms: u64) -> R<()> {
        let end = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if self.eval_js(expression).await.is_ok_and(|v| crate::js::truthy(&v)) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= end {
                return Err(format!("Timeout {timeout_ms}ms exceeded."));
            }
            sleep(100).await;
        }
    }

    /// Script `name` with `arg` truthy within `timeout_ms`.
    pub async fn wait_for_function(&self, name: &str, arg: Option<&Value>, timeout_ms: u64) -> R<()> {
        self.wait_for_js(&scripts::call(name, arg), timeout_ms).await
    }

    fn find(sel: &str, pick: Option<Pick>) -> String {
        format!("({FIND})({}, {})", crate::js::stringify(&json!(sel)), pick.map_or("null", Pick::js))
    }

    /// How many elements the selector has.
    pub async fn count(&self, sel: &str) -> R<usize> {
        Ok(self.eval_js(&format!("{}.length", Self::find(sel, None))).await?.as_u64().unwrap_or(0) as usize)
    }

    /// The element of the selector shows (Playwright's isVisible of the first match).
    pub async fn is_visible(&self, sel: &str) -> R<bool> {
        let js = format!(
            "(() => {{ const e = {}; if (!e) return false; const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; }})()",
            Self::find(sel, Some(Pick::First))
        );
        Ok(self.eval_js(&js).await? == Value::Bool(true))
    }

    /// Waits until the element shows, `timeout_ms` at most (locator.waitFor).
    pub async fn wait_for(&self, sel: &str, pick: Pick, timeout_ms: u64) -> R<()> {
        let js = format!(
            "(() => {{ const e = {}; if (!e) return false; const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; }})()",
            Self::find(sel, Some(pick))
        );
        self.wait_for_js(&js, timeout_ms).await.map_err(|_| format!("Timeout {timeout_ms}ms exceeded waiting for {sel}"))
    }

    /// A script on the element (locator.evaluate): Err when there is none.
    pub async fn evaluate_on(&self, sel: &str, pick: Pick, name: &str, arg: Option<&Value>) -> R<Value> {
        let js = format!(
            "(async () => {{ const e = {}; if (!e) throw new Error('no element'); return await ({})(e); }})()",
            Self::find(sel, Some(pick)),
            scripts::on_element(name, arg)
        );
        self.eval_js(&js).await
    }

    /// The box of the element in the viewport, None when it has none.
    pub async fn bounding_box(&self, sel: &str, pick: Pick) -> R<Option<(f64, f64, f64, f64)>> {
        let js = format!(
            "(() => {{ const e = {}; if (!e) return null; const r = e.getBoundingClientRect(); return r.width || r.height ? [r.x, r.y, r.width, r.height] : null; }})()",
            Self::find(sel, Some(pick))
        );
        let v = self.eval_js(&js).await?;
        Ok(v.as_array().map(|a| (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0), a[2].as_f64().unwrap_or(0.0), a[3].as_f64().unwrap_or(0.0))))
    }

    pub async fn inner_text(&self, sel: &str, pick: Pick) -> R<String> {
        let js = format!("(() => {{ const e = {}; if (!e) throw new Error('no element'); return e.innerText; }})()", Self::find(sel, Some(pick)));
        Ok(self.eval_js(&js).await?.as_str().unwrap_or("").to_string())
    }

    /// The point a click on the element lands on, once it shows, is enabled, holds still in view and nothing covers its
    /// middle (Playwright's actionability checks); None while it does not.
    async fn actionable(&self, sel: &str, pick: Pick) -> R<Option<Point>> {
        let js = format!(
            "(async () => {{ const e = {}; if (!e) return null; const box = () => e.getBoundingClientRect(); let r = box(); if (!r.width || !r.height || getComputedStyle(e).visibility === 'hidden' || e.disabled || e.getAttribute('aria-disabled') === 'true') return null; e.scrollIntoView({{ block: 'nearest', inline: 'nearest' }}); await new Promise((f) => requestAnimationFrame(() => requestAnimationFrame(f))); const s = box(); if (Math.abs(s.x - box().x) > 0.5) return null; const x = s.x + s.width / 2, y = s.y + s.height / 2; const hit = document.elementFromPoint(x, y); return hit && (hit === e || e.contains(hit)) ? {{ x, y }} : null; }})()",
            Self::find(sel, Some(pick))
        );
        Ok(Point::of(&self.eval_js(&js).await?))
    }

    /// A real click on the element, waited for as Playwright waits, `timeout_ms` at most.
    pub async fn click(&self, sel: &str, pick: Pick, timeout_ms: u64) -> R<()> {
        let at = self.wait_actionable(sel, pick, timeout_ms).await?;
        self.mouse_click(at.x, at.y).await
    }

    /// The mouse over the middle of the element (locator.hover).
    pub async fn hover(&self, sel: &str, pick: Pick, timeout_ms: u64) -> R<()> {
        let at = self.wait_actionable(sel, pick, timeout_ms).await?;
        self.mouse_move(at.x, at.y, 1).await
    }

    async fn wait_actionable(&self, sel: &str, pick: Pick, timeout_ms: u64) -> R<Point> {
        let end = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Ok(Some(p)) = self.actionable(sel, pick).await {
                return Ok(p);
            }
            if tokio::time::Instant::now() >= end {
                return Err(format!("Timeout {timeout_ms}ms exceeded waiting for {sel} to be clickable"));
            }
            sleep(100).await;
        }
    }

    /// Focus on the element, once it is there (locator.focus).
    pub async fn focus(&self, sel: &str, pick: Pick, timeout_ms: u64) -> R<()> {
        let js = format!("(() => {{ const e = {}; if (!e) return false; e.focus(); return true; }})()", Self::find(sel, Some(pick)));
        self.wait_for_js(&js, timeout_ms).await.map_err(|_| format!("Timeout {timeout_ms}ms exceeded focusing {sel}"))
    }

    /// The mouse to (x, y), in `steps` moves from where it is.
    pub async fn mouse_move(&self, x: f64, y: f64, steps: u32) -> R<()> {
        let (fx, fy) = self.mouse.get();
        let steps = steps.max(1);
        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            let (px, py) = (fx + (x - fx) * t, fy + (y - fy) * t);
            self.send("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": px, "y": py, "button": "none" })).await?;
        }
        self.mouse.set((x, y));
        Ok(())
    }

    /// A real click at a point: the move there, press and release (the page takes it as trusted, with user activation).
    pub async fn mouse_click(&self, x: f64, y: f64) -> R<()> {
        self.mouse_move(x, y, 1).await?;
        self.send("Input.dispatchMouseEvent", json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 })).await?;
        self.send("Input.dispatchMouseEvent", json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1 })).await?;
        Ok(())
    }

    async fn key_event(&self, kind: &str, k: &keys::Key, modifiers: i64) -> R<()> {
        let mut p = json!({ "type": kind, "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.key_code, "nativeVirtualKeyCode": k.key_code, "modifiers": modifiers });
        if kind == "keyDown"
            && let Some(t) = &k.text
            && modifiers & !8 == 0
        {
            p["type"] = json!("keyDown");
            p["text"] = json!(t);
            p["unmodifiedText"] = json!(t);
        } else if kind == "keyDown" {
            p["type"] = json!("rawKeyDown");
        }
        self.send("Input.dispatchKeyEvent", p).await.map(drop)
    }

    /// keyboard.press("Control+Shift+H"): the modifiers down, the key down and up, the modifiers up.
    pub async fn press(&self, combo: &str) -> R<()> {
        let (held, k) = keys::combo(combo)?;
        let mut mods = 0;
        for m in &held {
            mods |= keys::modifier(&m.key);
            self.key_event("keyDown", m, mods).await?;
        }
        self.key_event("keyDown", &k, mods).await?;
        self.key_event("keyUp", &k, mods).await?;
        for m in held.iter().rev() {
            mods &= !keys::modifier(&m.key);
            self.key_event("keyUp", m, mods).await?;
        }
        Ok(())
    }

    /// keyboard.insertText: the text as one input, no key events.
    pub async fn insert_text(&self, text: &str) -> R<()> {
        self.send("Input.insertText", json!({ "text": text })).await.map(drop)
    }

    /// keyboard.type: a key per character where the layout has it, else the character inserted; `delay_ms` between.
    pub async fn type_text(&self, text: &str, delay_ms: u64) -> R<()> {
        for c in text.chars() {
            let name = c.to_string();
            match keys::key(&name) {
                Some(k) => {
                    let k = keys::Key { key: name.clone(), text: Some(name.clone()), ..k };
                    self.key_event("keyDown", &k, 0).await?;
                    self.key_event("keyUp", &k, 0).await?;
                }
                None => self.insert_text(&name).await?,
            }
            if delay_ms > 0 {
                sleep(delay_ms).await;
            }
        }
        Ok(())
    }

    /// page.goto with waitUntil commit: the navigation started and answered.
    pub async fn goto(&self, url: &str) -> R<()> {
        let r = self.send("Page.navigate", json!({ "url": url })).await?;
        match r["errorText"].as_str().filter(|e| !e.is_empty()) {
            Some(e) => Err(format!("navigation: {e}")),
            None => Ok(()),
        }
    }

    /// page.reload with waitUntil domcontentloaded: a new document, past loading, `timeout_ms` at most.
    pub async fn reload(&self, timeout_ms: u64) -> R<()> {
        let before = self.eval_js("performance.timeOrigin").await.ok();
        self.send("Page.reload", json!({})).await?;
        let check = format!("performance.timeOrigin !== {} && document.readyState !== 'loading'", before.map_or("0".into(), |v| v.to_string()));
        self.wait_for_js(&check, timeout_ms).await
    }

    /// A command of the page's own session (Storage.clearDataForOrigin...).
    pub async fn command(&self, method: &str, params: Value) -> R<Value> {
        self.send(method, params).await
    }

    /// `f` as the agent's own input: the page sees it as trusted input, like the owner's in the remote desktop, so the
    /// spans of time the agent sends input are kept, and input then, or up to TAIL_MS after, is its own (asAgent).
    pub async fn as_agent<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        let now = crate::agent::now_ms();
        let id = self.span_seq.get() + 1;
        self.span_seq.set(id);
        {
            let mut spans = self.spans.borrow_mut();
            spans.retain(|(_, _, to)| now - to < KEEP_MS);
            spans.push((id, now, i64::MAX));
        }
        let out = f.await;
        if let Some(s) = self.spans.borrow_mut().iter_mut().find(|s| s.0 == id) {
            s.2 = crate::agent::now_ms();
        }
        out
    }

    /// One sequence of real input at a time on this page, as the agent's own (withInput): a click of the call watch must
    /// not land where the presence keeper left the mouse.
    pub async fn with_input<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        let _turn = self.input.lock().await;
        self.as_agent(f).await
    }

    /// Input at `at` (ms) came from the agent.
    pub fn by_agent(&self, at: i64) -> bool {
        self.spans.borrow().iter().any(|(_, from, to)| at >= *from && at <= to.saturating_add(TAIL_MS))
    }

    /// The owner's input (ms) the page recorded since the last call, the agent's own left out (ownerInput).
    pub async fn owner_input(&self) -> R<Vec<i64>> {
        let times = self.evaluate("page-state.drainInput", None).await?;
        Ok(times.as_array().into_iter().flatten().filter_map(Value::as_f64).map(|t| t as i64).filter(|t| !self.by_agent(*t)).collect())
    }
}

/// Input the page records up to this long after a span of the agent is still the agent's (events on their way).
pub const TAIL_MS: i64 = 1000;
const KEEP_MS: i64 = 120_000;

/// A frame of a page: its session and, for a frame in the page's process, its execution context.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    session: String,
    context: Option<i64>,
}

#[cfg(test)]
pub(crate) mod testing {
    //! A headless Chrome where CDP_CHROME names one, and the fixtures of the Node tests loaded in it.
    use super::*;

    pub struct Chrome {
        pub browser: Rc<Browser>,
        pub page: Rc<Page>,
        _process: tokio::process::Child,
        _profile: std::path::PathBuf,
    }

    /// The Chrome of CDP_CHROME, None (the test is skipped) without one.
    pub async fn chrome() -> Option<Chrome> {
        let exe = std::env::var("CDP_CHROME").ok().filter(|c| !c.is_empty())?;
        let profile = crate::agent::testing::temp_dir();
        static PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
        let port = 9500 + (std::process::id() % 300) as u16 + PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let process = tokio::process::Command::new(exe)
            .args(["--headless=new", "--no-sandbox", "--no-first-run", "--window-size=1280,900", "--user-agent=TeamsRelayTest/1.0"])
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("chrome");
        let mut browser = None;
        for _ in 0..100 {
            if let Ok(b) = Browser::connect(&format!("http://127.0.0.1:{port}")).await {
                browser = Some(b);
                break;
            }
            sleep(100).await;
        }
        let browser = browser.expect("DevTools up");
        let mut tab = None;
        for _ in 0..50 {
            tab = browser.tabs().into_iter().next();
            if tab.is_some() {
                break;
            }
            sleep(100).await;
        }
        let page = browser.page(&tab.expect("a tab").0).await.expect("page");
        Some(Chrome { browser, page, _process: process, _profile: profile })
    }

    /// A fixture of test/agent/fixtures as the document of the page.
    pub async fn fixture(page: &Page, name: &str) {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test/agent/fixtures").join(name);
        let html = std::fs::read_to_string(&file).expect("fixture");
        set_content(page, &html).await;
    }

    /// Every request of the page answered with the HTML `serve` makes of its address, nothing reaching the network
    /// (Playwright's page.route with fulfill): a page on https://teams.cloud.microsoft/ or an MCAS gateway in a test.
    pub async fn route(page: &Page, serve: impl Fn(&str) -> String + 'static) {
        use base64::Engine;
        let mut events = page.cdp.subscribe();
        let (cdp, session) = (page.cdp.clone(), page.session.clone());
        tokio::task::spawn_local(async move {
            while let Some(e) = events.recv().await {
                if e["method"] != "Fetch.requestPaused" || e["sessionId"].as_str() != Some(session.as_str()) {
                    continue;
                }
                let p = &e["params"];
                let body = base64::engine::general_purpose::STANDARD.encode(serve(p["request"]["url"].as_str().unwrap_or("")));
                let answer = json!({ "requestId": p["requestId"], "responseCode": 200, "responseHeaders": [{ "name": "Content-Type", "value": "text/html; charset=utf-8" }], "body": body });
                let _ = cdp.send("Fetch.fulfillRequest", answer, Some(&session)).await;
            }
        });
        page.send("Fetch.enable", json!({ "patterns": [{ "urlPattern": "*" }] })).await.expect("Fetch.enable");
    }

    /// The page on `url`, loaded, and its address known to the browser.
    pub async fn open(page: &Page, url: &str) {
        let before = page.eval_js("performance.timeOrigin").await.unwrap_or(json!(0));
        page.goto(url).await.expect("navigation");
        let host = url::Url::parse(url).expect("url").host_str().unwrap_or("").to_string();
        let loaded = format!(
            "performance.timeOrigin !== {before} && location.hostname === {} && document.readyState === 'complete'",
            crate::js::stringify(&json!(host))
        );
        page.wait_for_js(&loaded, 5000).await.expect("loaded");
        for _ in 0..50 {
            if page.url().contains(&host) {
                return;
            }
            sleep(20).await;
        }
        panic!("the browser never told the address {url}");
    }

    pub async fn set_content(page: &Page, html: &str) {
        let js = format!("document.open(); document.write({}); document.close(); true", crate::js::stringify(&json!(html)));
        page.eval_js(&js).await.expect("content");
        sleep(50).await;
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{chrome, set_content};
    use super::*;
    use crate::agent::testing::run;

    #[test]
    fn scripts_selectors_input_and_frames_in_a_real_browser() {
        run(async {
            let Some(c) = chrome().await else {
                eprintln!("CDP_CHROME not set: the browser test is skipped");
                return;
            };
            let p = &c.page;
            set_content(
                p,
                r#"<html><body><button id="a" onclick="window.hits=(window.hits||0)+1">A</button><button id="h" style="visibility:hidden">H</button>
                <div data-tid="ckeditor" contenteditable="true"></div><input id="i"><iframe srcdoc="<p>in</p>"></iframe>
                <script>window.keys=[];addEventListener('keydown',e=>keys.push([e.key,e.ctrlKey,e.shiftKey,e.isTrusted]));</script></body></html>"#,
            )
            .await;
            assert_eq!(p.evaluate("page-state.makeVisible", None).await.expect("script"), json!("installed"));
            assert_eq!(p.evaluate("page-state.makeVisible", None).await.expect("script"), json!("already"));
            let at = p.evaluate("page-state.uncoveredPoint", Some(&json!("#a"))).await.expect("point");
            assert!(Point::of(&at).is_some());
            assert_eq!(p.count("button").await.expect("count"), 2);
            assert_eq!(p.count("button:visible").await.expect("count"), 1);
            assert!(!p.is_visible("#h").await.expect("visible"));
            p.click("#a", Pick::First, 2000).await.expect("click");
            assert_eq!(p.eval_js("window.hits").await.expect("hits"), json!(1));
            assert!(p.click("#h", Pick::First, 300).await.is_err());
            p.focus("#i", Pick::First, 1000).await.expect("focus");
            p.type_text("ab @1", 0).await.expect("type");
            p.press("Control+A").await.expect("press");
            p.insert_text("xy").await.expect("insert");
            assert_eq!(p.eval_js("document.querySelector('#i').value").await.expect("value"), json!("xy"));
            let keys = p.eval_js("keys").await.expect("keys");
            assert!(keys.as_array().expect("keys").iter().all(|k| k[3] == json!(true)));
            assert!(keys.as_array().expect("keys").contains(&json!(["A", true, false, true])));
            // the text of TEXTS and SEL as RegExps in the page
            let reduced = p.evaluate("page-state.probePage", Some(&scripts::st(json!({ "withPresence": false })))).await.expect("probe");
            assert_eq!(reduced, json!({ "reduced": false, "domReady": true, "hookInstalled": false }));
            sleep(200).await;
            assert!(p.frames().len() >= 2, "{:?}", p.frames());
            for f in p.frames() {
                p.evaluate_in(&f, "page-state.watchInput", None).await.expect("in every frame");
            }
            assert!(p.url().starts_with("about:blank"));
            let err = p.eval_js("(() => { throw new Error('boom') })()").await.unwrap_err();
            assert!(err.contains("boom"), "{err}");
        });
    }
}
