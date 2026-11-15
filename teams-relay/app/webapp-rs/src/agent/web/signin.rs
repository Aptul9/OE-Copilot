//! A slot signed out, and the sign-in of a light account (src/agent/jobs/sign-in.ts, fresh-sign-in.ts, light-token.ts,
//! teams/sign-in-actions.ts, teams/mcas.ts): the one press of Sign in that often signs Teams in again by itself, the
//! data of Teams cleared for a new Microsoft sign-in, the refresh token and the Microsoft session captured for the light
//! host, and the monitored-access notice of the proxy of the tenant continued.

use super::Agent;
use super::jobs::{SIGN_IN_TRY_AFTER, SIGN_IN_TRY_EVERY, SIGN_IN_TRY_WAIT, host_of, is_login_url, is_teams_url, sign_in_try, watch_of};
use super::media::behind_mcas;
use crate::agent::browser::scripts::{self, st};
use crate::agent::browser::{Page, Point};
use crate::agent::mcas::{McasSession, mcas_session_file};
use crate::agent::session::{MicrosoftSession, TEAMS_WEB_CLIENT, session_file};
use crate::agent::teams::auth::region_of;
use crate::agent::token::StoredToken;
use crate::agent::{info, now_ms, now_s, warn};
use crate::state::{LightSignIn, key};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const SIGN_IN_TRY: &str = "sign_in_try";
const FRESH_SIGN_IN: &str = "fresh_sign_in";
/// A sign-in holding less than this many seconds is the one to replace (MIN_LIFE of src/lib/light.ts)
const ENDING: i64 = 600;
/// Without an expiry in the page, the refresh token of a single-page app is taken to hold this long (Microsoft: 24 h)
const ASSUMED_LIFE: i64 = 20 * 3600;

/// What was logged for the sign-out that started at `since`: the pages whose buttons were listed, the attempt skipped
/// for being too close to the last one, the pages listed once the owner was told.
#[derive(Default)]
pub struct Told {
    since: i64,
    pages: HashSet<String>,
    skipped: bool,
    listed: bool,
}

fn told_of(a: &Agent, since: i64) {
    let mut t = a.sign_in.borrow_mut();
    if t.since != since {
        *t = Told { since, ..Told::default() };
    }
}

/// An MSAL entry of the page (readMsalEntries): secret, client, home account, expiry (Unix seconds, 0 when none).
#[derive(Clone, Debug, PartialEq)]
pub struct MsalEntry {
    pub secret: String,
    pub client_id: String,
    pub home_account_id: String,
    pub expires_on: i64,
}

fn msal_entries(v: &Value) -> Vec<MsalEntry> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let s = |k: &str| e[k].as_str().unwrap_or("").to_string();
            let exp = crate::js::number(e.get("expiresOn"));
            MsalEntry {
                secret: s("secret"),
                client_id: s("clientId"),
                home_account_id: s("homeAccountId"),
                expires_on: if exp.is_finite() { exp as i64 } else { 0 },
            }
        })
        .collect()
}

/// The refresh token of Teams web among those the page keeps, else the first one.
pub fn teams_refresh_token(entries: &[MsalEntry]) -> Option<MsalEntry> {
    entries.iter().find(|e| e.client_id == TEAMS_WEB_CLIENT).or(entries.first()).cloned()
}

async fn refresh_tokens(page: &Page) -> Vec<MsalEntry> {
    page.evaluate("tokens.readMsalEntries", Some(&json!(["refreshtoken"]))).await.map(|v| msal_entries(&v)).unwrap_or_default()
}

/// The sign-in the light agent runs on, as the state row says it.
pub fn light_sign_in(a: &Agent) -> LightSignIn {
    LightSignIn::parse_value(&serde_json::from_str(&a.store.get_state(key::LIGHT_TOKEN)).unwrap_or(Value::Null))
}

fn email_of(a: &Agent) -> String {
    crate::state::object(Some(&a.store.get_state(key::ME))).and_then(|m| m.get("email").and_then(Value::as_str).map(String::from)).unwrap_or_default()
}

// ---- one press of Sign in (jobs/sign-in.ts, teams/sign-in-actions.ts)

/// The pages on Microsoft's sign-in host: the popup Teams' Sign in opened, or the Teams tab itself sent there.
async fn login_pages(a: &Agent) -> Vec<Rc<Page>> {
    let Some(b) = a.page().and_then(|p| p.browser()) else { return vec![] };
    b.pages().await.into_iter().filter(|p| !p.is_closed() && is_login_url(&p.url())).collect()
}

async fn microsoft_page(a: &Agent) -> Option<Rc<Page>> {
    login_pages(a).await.pop()
}

async fn sign_in_page(page: &Page, email: Option<&str>) -> Option<Value> {
    let mut more = json!({});
    if let Some(e) = email {
        more["email"] = json!(e);
    }
    page.evaluate("sign-in.signInPage", Some(&st(more))).await.ok().filter(Value::is_object)
}

/// The buttons of a page of the sign-out, once per sign-out and address (without its query).
async fn log_buttons(a: &Agent, page: &Page) {
    let url = page.url();
    let where_ = url.split('?').next().unwrap_or("").to_string();
    if a.sign_in.borrow().pages.contains(&where_) {
        return;
    }
    let Some(buttons) = sign_in_page(page, None).await.and_then(|v| v["buttons"].as_array().cloned()) else { return };
    a.sign_in.borrow_mut().pages.insert(where_);
    let list: Vec<String> = buttons.iter().filter_map(Value::as_str).map(String::from).collect();
    info("SESSION", "sign-in page buttons", &[("host", host_of(&url).into()), ("buttons", list.join(" | ").into())]);
}

fn save_try(a: &Agent, at: i64, pressed: &[String], microsoft: bool) {
    a.store.set_state(SIGN_IN_TRY, &crate::js::stringify(&json!({ "at": at, "pressed": pressed, "microsoft": microsoft })));
}

/// Teams' own Sign in while Teams says to sign in again: one real click (its popup opens only for a trusted click).
async fn press_teams_sign_in(page: &Page) -> bool {
    page.with_input(async {
        let Some(at) = sign_in_page(page, None).await.and_then(|v| Point::of(&v["teams"]["at"])) else { return false };
        page.mouse_click(at.x, at.y).await.is_ok()
    })
    .await
}

/// Microsoft's sign-in page: the tile of this account, else its one Sign in or Continue, with one real click, at a look
/// after the first and only with no input of the owner meanwhile. "asks" (a field to type in), "owner", "account" or
/// "button"; None while there is nothing to press.
async fn press_microsoft(page: &Page, email: &str) -> Option<&'static str> {
    if page.evaluate("page-state.watchInput", None).await.ok()? != "already" {
        return None;
    }
    page.with_input(async {
        if !page.owner_input().await.unwrap_or_default().is_empty() {
            return Some("owner");
        }
        let r = sign_in_page(page, Some(email)).await?["microsoft"].clone();
        if !r.is_object() {
            return None;
        }
        if r["asks"] == Value::Bool(true) {
            return Some("asks");
        }
        let (account, button) = (Point::of(&r["account"]), Point::of(&r["button"]));
        let at = account.or(button)?;
        page.mouse_click(at.x, at.y).await.ok()?;
        Some(if account.is_some() { "account" } else { "button" })
    })
    .await
}

/// Often a sign-out needs only a press of Teams' own Sign in: the session the browser still holds signs in again by
/// itself. Once per sign-out, SIGN_IN_TRY_AFTER seconds into it, once Teams shows its Sign in or a Microsoft sign-in
/// page shows: Teams' Sign in, then for SIGN_IN_TRY_WAIT seconds one press on Microsoft's page, never where it asks for
/// something to type. Never for an account never signed in, once the owner was told, or within SIGN_IN_TRY_EVERY
/// seconds of an attempt that pressed something (trySignIn).
pub async fn try_sign_in(a: &Agent) {
    let now = now_s();
    let (since, alerted) = watch_of(&a.store.get_state(key::LOGIN_WATCH));
    let me = a.store.get_state(key::ME);
    if since == 0 || me.is_empty() {
        return;
    }
    told_of(a, since);
    let Some(page) = a.page() else { return };
    let (mut at, mut pressed, mut microsoft) = sign_in_try(a);
    if at < since {
        if alerted {
            if !a.sign_in.borrow().listed {
                let mut pages = vec![page.clone()];
                pages.extend(login_pages(a).await.into_iter().filter(|p| !Rc::ptr_eq(p, &page)));
                for p in pages {
                    log_buttons(a, &p).await;
                }
            }
            a.sign_in.borrow_mut().listed = true;
            return;
        }
        if now - since < SIGN_IN_TRY_AFTER {
            return;
        }
        if at != 0 && !pressed.is_empty() && now - at < SIGN_IN_TRY_EVERY {
            if !a.sign_in.borrow().skipped {
                info("SESSION", &format!("signed out again within {} min of the last Sign in attempt: nothing pressed", SIGN_IN_TRY_EVERY / 60), &[]);
            }
            a.sign_in.borrow_mut().skipped = true;
            return;
        }
        let on_teams = is_teams_url(&page.url());
        let button = on_teams && sign_in_page(&page, None).await.is_some_and(|v| Point::of(&v["teams"]["at"]).is_some());
        if !button && microsoft_page(a).await.is_none() {
            return;
        }
        (at, pressed, microsoft) = (now, vec![], false);
        save_try(a, at, &pressed, microsoft);
        info("SESSION", "signed out: one attempt to sign in again", &[]);
        if on_teams {
            log_buttons(a, &page).await;
            if button && press_teams_sign_in(&page).await {
                pressed.push("teams".into());
                save_try(a, at, &pressed, microsoft);
                info("SESSION", "Sign in pressed in Teams", &[]);
            } else {
                info("SESSION", "no single Sign in of Teams to press: nothing pressed", &[]);
            }
        }
    }
    if alerted || microsoft || now - at >= SIGN_IN_TRY_WAIT {
        return;
    }
    let Some(ms) = microsoft_page(a).await else { return };
    log_buttons(a, &ms).await;
    let Some(done) = press_microsoft(&ms, &email_of(a)).await else { return };
    if done == "account" || done == "button" {
        pressed.push(done.into());
    }
    save_try(a, at, &pressed, true);
    let host = host_of(&ms.url());
    let what = match done {
        "asks" => "Microsoft asks for something to type: nothing pressed",
        "owner" => "the owner is on the Microsoft sign-in page: nothing pressed",
        "account" => "this account pressed on the Microsoft sign-in page",
        _ => "Sign in or Continue pressed on the Microsoft sign-in page",
    };
    info("SESSION", what, &[("host", host.into())]);
}

// ---- the sign-in of a light account (jobs/fresh-sign-in.ts, jobs/light-token.ts)

/// The browser of a light account runs because its sign-in ended or Microsoft refused it. Teams web there holds the same
/// refresh token, and its own Sign in keeps none: so, once per SIGN_IN_TRY_EVERY seconds, the data of Teams cleared and
/// Teams opened again, which sends the tab to Microsoft's sign-in page as a first sign-in does. True when Teams was
/// sent there (freshSignIn).
pub async fn fresh_sign_in(a: &Agent) -> bool {
    let now = now_s();
    let Some(page) = a.page() else { return false };
    let sign = light_sign_in(a);
    let browser = sign.browser == Some(true) || sign.mcas == Some(true) || behind_mcas(&page.url());
    if sign.at != 0.0 && !sign.refused && ((sign.session == Some(true) && !browser) || sign.expires_on - now as f64 >= ENDING as f64) {
        return false;
    }
    let email = email_of(a);
    if email.is_empty() {
        return false;
    }
    if now - a.store.get_state(FRESH_SIGN_IN).parse::<i64>().unwrap_or(0) < SIGN_IN_TRY_EVERY {
        return false;
    }
    // a refresh token other than the stored one is a new sign-in already: the light-token job takes it
    let rt = teams_refresh_token(&refresh_tokens(&page).await);
    let stored = StoredToken::load(&a.config.light_token_file).map(|t| t.refresh_token);
    if !browser && rt.is_some_and(|r| Some(r.secret) != stored) {
        return false;
    }
    a.store.set_state(FRESH_SIGN_IN, &now.to_string());
    let url = page.url();
    let here = if is_teams_url(&url) {
        url::Url::parse(&url).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
    } else {
        "https://teams.cloud.microsoft".into()
    };
    let mut origins = vec![here.clone()];
    origins.extend(scripts::list("teams_hosts").iter().map(|h| format!("https://{h}")));
    if browser {
        origins.extend(["https://teams.microsoft.com.mcas.ms".to_string(), "https://teams.cloud.microsoft.mcas.ms".to_string()]);
    }
    let mut seen = HashSet::new();
    origins.retain(|o| seen.insert(o.clone()));
    let run = async {
        for origin in &origins {
            page.command("Storage.clearDataForOrigin", json!({ "origin": origin, "storageTypes": "local_storage,indexeddb,cache_storage,service_workers" }))
                .await?;
        }
        page.goto(&format!("{here}/?loginHint={}", crate::js::encode_uri_component(&email))).await
    };
    match run.await {
        Ok(()) => {
            info("SESSION", "light sign-in: Teams data cleared, Teams sent to a new Microsoft sign-in", &[]);
            true
        }
        Err(e) => {
            warn("SESSION", &format!("light sign-in: {e}"), &[]);
            false
        }
    }
}

/// The refresh token and the Microsoft session for the light host: a new sign-in of the page, or the session cookies
/// that let the light host renew the saved token over HTTP; behind the proxy of the tenant its session too, validated
/// by one renewal (captureLightToken).
pub async fn capture_light_token(a: &Agent) -> bool {
    match capture(a).await {
        Ok(done) => done,
        Err(e) => {
            warn("light", &format!("capture: {e}"), &[]);
            false
        }
    }
}

async fn capture(a: &Agent) -> Result<bool, String> {
    let file = &a.config.light_token_file;
    let page = a.page().ok_or("no page")?;
    let now = now_s();
    let sign = light_sign_in(a);
    let browser = sign.browser == Some(true) || sign.mcas == Some(true) || behind_mcas(&page.url());
    let fresh = a.store.get_state(FRESH_SIGN_IN).parse::<f64>().unwrap_or(0.0) > sign.at;
    let ending = browser && sign.at != 0.0 && (sign.refused || sign.expires_on - (now as f64) < ENDING as f64);
    if ending && !fresh {
        return Ok(false);
    }
    let previous = StoredToken::load(file);
    let rt = teams_refresh_token(&refresh_tokens(&page).await);
    let tenant_id = rt
        .as_ref()
        .and_then(|r| r.home_account_id.split('.').nth(1).filter(|t| !t.is_empty()).map(String::from))
        .or_else(|| previous.as_ref().map(|p| p.tenant_id.clone()).filter(|t| !t.is_empty()))
        .unwrap_or_default();
    if (rt.is_none() && previous.is_none()) || tenant_id.is_empty() {
        return Ok(false);
    }
    let unchanged = rt.as_ref().is_none_or(|r| previous.as_ref().is_some_and(|p| p.refresh_token == r.secret));
    if ending && unchanged {
        return Ok(false);
    }
    let session = session_file(file);
    let captured = a.captured.borrow().clone();
    let same = match &rt {
        Some(r) => captured.as_deref() == Some(r.secret.as_str()),
        None => captured.is_some(),
    };
    if previous.is_some() && MicrosoftSession::load(&session).is_some() && same {
        return Ok(false);
    }
    // an expired token of the page still carries the tenant and the client the HTTP renewal needs
    let expires_on = match (&rt, &previous) {
        (_, Some(p)) if unchanged || (sign.browser == Some(true) && !fresh) => p.expires_on,
        (_, None) if unchanged || (sign.browser == Some(true) && !fresh) => return Err("Cannot read properties of null (reading 'expiresOn')".into()),
        (Some(r), _) => {
            if r.expires_on != 0 {
                r.expires_on
            } else {
                now + ASSUMED_LIFE
            }
        }
        (None, Some(p)) => p.expires_on,
        (None, None) => return Ok(false),
    };
    let browser_of = page.browser().ok_or("no browser")?;
    let cookies = browser_of.cookies().await?;
    let region = region_of(&cookies);
    let login = regex::Regex::new(r"(?i)^\.?login\.microsoftonline\.com$").expect("fixed");
    let microsoft: Vec<Map<String, Value>> =
        cookies.iter().filter(|c| login.is_match(c["domain"].as_str().unwrap_or(""))).filter_map(|c| c.as_object().cloned()).collect();
    let user_agent = crate::js::string(Some(&page.eval_js("navigator.userAgent").await?));
    let login_hint = email_of(a);
    let estsauth = regex::Regex::new("(?i)^ESTSAUTH(?:PERSISTENT|LIGHT)?$").expect("fixed");
    if microsoft.iter().any(|c| estsauth.is_match(c.get("name").and_then(Value::as_str).unwrap_or(""))) {
        let s = MicrosoftSession { user_agent: user_agent.clone(), login_hint: login_hint.clone(), cookies: microsoft };
        MicrosoftSession::save(&session, &s).map_err(|e| crate::agent::text_of(&e))?;
    } else if unchanged {
        return Ok(false);
    }
    let mcas_file = mcas_session_file(file);
    let mut mcas = rt.is_some() && browser && crate::agent::mcas::capture(&mcas_file, &a.http, &page, &tenant_id, &login_hint, &user_agent).await;
    let pick = |own: Option<String>, old: Option<String>| own.or(old).ok_or_else(|| "Cannot read properties of null".to_string());
    let mut token = StoredToken {
        refresh_token: pick(rt.as_ref().map(|r| r.secret.clone()), previous.as_ref().map(|p| p.refresh_token.clone()))?,
        client_id: pick(rt.as_ref().map(|r| r.client_id.clone()), previous.as_ref().map(|p| p.client_id.clone()))?,
        tenant_id,
        region: if rt.is_some() { region } else { pick(None, previous.as_ref().map(|p| p.region.clone()))? },
        expires_on,
    };
    if mcas {
        // the cached tokens of the page may be refused already: a family just validated goes to the light host
        match McasSession::new(mcas_file, a.http.clone()).renew(&token).await {
            Ok(t) => token = t,
            Err(_) => {
                mcas = false;
                if let Some(p) = previous.as_ref().filter(|p| sign.mcas == Some(true) && !sign.refused && p.expires_on - now >= ENDING) {
                    token = p.clone();
                }
                warn("light", "MCAS token validation unavailable: browser renewal retained", &[]);
            }
        }
    }
    token.save(file).map_err(|e| crate::agent::text_of(&e))?;
    let mut row = json!({
        "at": if rt.is_some() { json!(now) } else { crate::web::num(sign.at) },
        "expiresOn": token.expires_on,
        "refused": rt.is_none() && sign.refused,
    });
    if MicrosoftSession::load(&session).is_some() {
        row["session"] = json!(true);
    }
    if mcas || (rt.is_none() && sign.mcas == Some(true)) {
        row["mcas"] = json!(true);
    } else if browser {
        row["browser"] = json!(true);
    }
    a.store.set_state(key::LIGHT_TOKEN, &crate::js::stringify(&row));
    if let Some(r) = &rt {
        *a.captured.borrow_mut() = Some(r.secret.clone());
    }
    info("light", "sign-in captured", &[("until", crate::auth::rows::iso_ms(token.expires_on * 1000).into())]);
    Ok(true)
}

// ---- the monitored-access notice of the proxy (teams/mcas.ts)

thread_local! {
    /// per tab: the document whose Continue was pressed, and until when the owner's input keeps the notice theirs (ms)
    static HANDLED: RefCell<HashMap<String, f64>> = RefCell::default();
    static OWNER_UNTIL: RefCell<HashMap<String, i64>> = RefCell::default();
}

/// The notice exactly as observed, nothing customized.
const MONITORED_NOTICE: &str = r#"(() => {
  const c = window.context;
  return !!c && c.TITLE_ID === "PROXYWEB_MONITORED_ACCESS_MESSAGE" && c.DESCRIPTION_ID === "PROXYWEB_MONITORED_ACCESS_DESCRIPTION" && c.DISPLAY_DISCLAIMER === true && !c.CUSTOMIZED_TITLE && !c.CUSTOMIZED_BODY && !c.WAIT_FOR_APP_ONBOARDING && !c.APP_ONBOARDING_SHOW_CONTINUE && !c.BYPASS_NON_SUFFIX_SUPPORTED_APP_FOR_ADMIN && !c.SECOND_LOGIN_INFORMATION_PAGE_ENABLED && c.REDIRECT_TO_SUFFIX_METHOD === "POST";
})()"#;

/// Marks with data-relay-pick=`pick` the elements of `role` whose accessible name `name` matches, not hidden (Playwright's
/// getByRole), and answers how many; the one marked can then be clicked by the selector.
fn by_role(role: &str, name: &str, pick: &str) -> String {
    format!(
        r#"(() => {{
  const roles = {{ button: 'button, input[type="button"], input[type="submit"], [role="button"]', checkbox: 'input[type="checkbox"], [role="checkbox"]' }};
  const re = new RegExp({name}, "i");
  const text = (e) => (e.innerText || e.textContent || "").replace(/\s+/g, " ").trim();
  const nameOf = (e) => {{
    const by = e.getAttribute("aria-labelledby");
    if (by) return by.split(/\s+/).map((id) => document.getElementById(id)).filter(Boolean).map(text).join(" ");
    if (e.getAttribute("aria-label")) return e.getAttribute("aria-label").trim();
    if (e.tagName === "INPUT") {{
      const l = (e.id && document.querySelector(`label[for="${{CSS.escape(e.id)}}"]`)) || e.closest("label");
      return l ? text(l) : (e.value || "").trim();
    }}
    return text(e);
  }};
  const hidden = (e) => {{ const cs = getComputedStyle(e); return cs.display === "none" || cs.visibility === "hidden" || e.closest('[aria-hidden="true"]'); }};
  document.querySelectorAll("[data-relay-pick={pick}]").forEach((e) => e.removeAttribute("data-relay-pick"));
  const all = [...document.querySelectorAll(roles[{role}])].filter((e) => !hidden(e) && re.test(nameOf(e)));
  all.forEach((e) => e.setAttribute("data-relay-pick", "{pick}"));
  return all.length;
}})()"#,
        name = crate::js::stringify(&json!(name)),
        role = crate::js::stringify(&json!(role)),
        pick = pick,
    )
}

fn enabled(sel: &str) -> String {
    format!(
        "(() => {{ const e = document.querySelector({}); return !!e && !e.disabled && e.getAttribute('aria-disabled') !== 'true'; }})()",
        crate::js::stringify(&json!(sel))
    )
}

/// The aad_login page of a gateway of the proxy, on its plain address.
pub fn notice_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let host = regex::Regex::new(r"(?i)^[a-z0-9-]+\.access\.mcas\.ms$").expect("fixed");
    u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && host.is_match(u.host_str().unwrap_or(""))
        && u.port().is_none()
        && u.path() == "/aad_login"
}

/// The monitored-access notice of Defender for Cloud Apps on its aad_login page, exactly as observed: its \"don't show
/// for a week\" ticked and its one Continue pressed, once per document, never while the owner uses the page nor where
/// the page asks for something to type (continueMcas).
pub async fn continue_mcas(page: &Page) -> bool {
    if !notice_url(&page.url()) {
        return false;
    }
    if page.eval_js(MONITORED_NOTICE).await != Ok(Value::Bool(true))
        || page.evaluate("page-state.watchInput", None).await.ok().as_ref().and_then(Value::as_str) != Some("already")
    {
        return false;
    }
    let tab = page.target().to_string();
    page.with_input(async {
        let owner = page.owner_input().await.unwrap_or_default();
        if !owner.is_empty() {
            OWNER_UNTIL.with(|o| o.borrow_mut().insert(tab.clone(), now_ms() + 30_000));
        }
        if now_ms() < OWNER_UNTIL.with(|o| o.borrow().get(&tab).copied().unwrap_or(0)) {
            return false;
        }
        let Some(document_at) = page.eval_js("performance.timeOrigin").await.ok().and_then(|v| v.as_f64()) else { return false };
        if HANDLED.with(|h| h.borrow().get(&tab) == Some(&document_at)) {
            return false;
        }
        if page.count(r#"input[type="password"], input[type="email"], input[name="otc"]"#).await.unwrap_or(1) > 0 {
            return false;
        }
        let button = r#"[data-relay-pick="continue"]"#;
        if page.eval_js(&by_role("button", "^Continue$", "continue")).await != Ok(json!(1))
            || !page.is_visible(button).await.unwrap_or(false)
            || page.eval_js(&enabled(button)).await != Ok(Value::Bool(true))
        {
            return false;
        }
        let hide = r#"[data-relay-pick="hide"]"#;
        if page.eval_js(&by_role("checkbox", r"(?:don't|do not).*show.*week|hide.*week", "hide")).await == Ok(json!(1))
            && page.is_visible(hide).await.unwrap_or(false)
            && page.eval_js(&enabled(hide)).await == Ok(Value::Bool(true))
        {
            let checked = format!(
                "(() => {{ const e = document.querySelector({}); return !!e && (e.checked === true || e.getAttribute('aria-checked') === 'true'); }})()",
                crate::js::stringify(&json!(hide))
            );
            if page.eval_js(&checked).await != Ok(Value::Bool(true)) {
                if page.click(hide, crate::agent::browser::Pick::First, 2000).await.is_err() {
                    return false;
                }
                if page.eval_js(&checked).await != Ok(Value::Bool(true)) {
                    return false;
                }
            }
        }
        if page.click(button, crate::agent::browser::Pick::First, 3000).await.is_err() {
            return false;
        }
        HANDLED.with(|h| h.borrow_mut().insert(tab.clone(), document_at));
        true
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refresh_token_of_teams_web_first() {
        let e = |c: &str| MsalEntry { secret: format!("s-{c}"), client_id: c.into(), home_account_id: "oid.tid".into(), expires_on: 0 };
        assert_eq!(teams_refresh_token(&[e("other"), e(TEAMS_WEB_CLIENT)]).map(|r| r.secret), Some(format!("s-{TEAMS_WEB_CLIENT}")));
        assert_eq!(teams_refresh_token(&[e("other")]).map(|r| r.client_id), Some("other".into()));
        assert_eq!(teams_refresh_token(&[]), None);
        let parsed = msal_entries(&json!([{ "secret": "x", "clientId": "c", "homeAccountId": "o.t", "expiresOn": 1700000000 }]));
        assert_eq!(parsed[0].expires_on, 1_700_000_000);
    }
}
