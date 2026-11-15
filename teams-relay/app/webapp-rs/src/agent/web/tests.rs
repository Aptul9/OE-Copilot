//! The browser agent in a real Chrome (CDP_CHROME), its pages served by the test on their real addresses: the sign-in
//! of a light account (test/agent/fresh-sign-in.test.ts, the capture of jobs/light-token.ts), the one press of Sign in
//! (sign-in-job.test.ts), the monitored-access notice of the proxy (page-mcas.test.ts).

use super::*;
use crate::agent::browser::testing::{chrome, open, route};
use crate::agent::config::{Env, load};
use crate::agent::session::{MicrosoftSession, TEAMS_WEB_CLIENT, session_file};
use crate::agent::store::ChatEntry;
use crate::agent::testing::{run, temp_dir};
use crate::agent::token::StoredToken;
use crate::state::key;
use futures_util::future::LocalBoxFuture;
use serde_json::json;

const EMAIL: &str = "test.user@contoso.example";

#[derive(Default)]
struct FakeNotify {
    log: RefCell<Vec<(String, String)>>,
}

impl Notify for FakeNotify {
    fn message<'a>(&'a self, title: &'a str, _: &'a str, _: &'a str) -> LocalBoxFuture<'a, ()> {
        self.log.borrow_mut().push(("message".into(), title.into()));
        Box::pin(async {})
    }
    fn alert<'a>(&'a self, title: &'a str, _: &'a str) -> LocalBoxFuture<'a, usize> {
        self.log.borrow_mut().push(("alert".into(), title.into()));
        Box::pin(async { 1 })
    }
    fn notice<'a>(&'a self, title: &'a str, _: &'a str) -> LocalBoxFuture<'a, usize> {
        self.log.borrow_mut().push(("notice".into(), title.into()));
        Box::pin(async { 1 })
    }
    fn call<'a>(&'a self, caller: &'a str, _: &'a str, _: i64, _: i64, _: bool) -> LocalBoxFuture<'a, usize> {
        self.log.borrow_mut().push(("call".into(), caller.into()));
        Box::pin(async { 1 })
    }
    fn device_count(&self) -> usize {
        1
    }
}

/// An agent of slot 7 in a folder of its own, signed in as EMAIL, driving `page`.
fn agent(page: Rc<Page>) -> Rc<Agent> {
    let dir = temp_dir();
    let mut env: Env = [("ACCOUNT", "7"), ("DOMAIN", "relay.example")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.insert("DB_PATH".into(), dir.join("messages.db").display().to_string());
    env.insert("APP_DB".into(), dir.join("app.db").display().to_string());
    let config = load(&env).expect("config");
    let store = Rc::new(Store::open(&config.db_path).expect("store"));
    store.set_state(key::ME, &json!({ "name": "Test User", "email": EMAIL, "tenant": "Contoso", "av": "", "logo": "" }).to_string());
    let notifier: Rc<dyn Notify> = Rc::new(FakeNotify::default());
    let a = Agent::new(Parts {
        config,
        store: store.clone(),
        notifier,
        http: crate::agent::http::slot_http(),
        checked_only: Box::new(|| false),
        light_mode: Box::new(|| true),
    });
    a.set_tp(Rc::new(TeamsPage::new(page, store)));
    a
}

/// A refresh token of Teams web in the MSAL cache of the page (localStorage).
async fn seed_refresh_token(page: &Page, secret: &str, expires_on: i64) {
    let entry = json!({ "secret": secret, "clientId": TEAMS_WEB_CLIENT, "homeAccountId": "user.tenant", "expiresOn": expires_on }).to_string();
    let js = format!(
        "localStorage.setItem({}, {}); true",
        crate::js::stringify(&json!(format!("user.tenant-login.windows.net-refreshtoken-{TEAMS_WEB_CLIENT}----"))),
        crate::js::stringify(&json!(entry))
    );
    page.eval_js(&js).await.expect("seed");
}

fn stored(secret: &str, expires_on: i64) -> StoredToken {
    StoredToken { refresh_token: secret.into(), client_id: TEAMS_WEB_CLIENT.into(), tenant_id: "tenant".into(), region: "emea".into(), expires_on }
}

fn state(a: &Agent, k: &str) -> Value {
    serde_json::from_str(&a.store.get_state(k)).unwrap_or(Value::Null)
}

#[test]
fn every_page_script_is_a_function() {
    assert_eq!(check_scripts(), 48);
}

#[test]
fn the_monitored_access_notice_lives_only_on_the_plain_gateway_address() {
    assert!(signin::notice_url("https://tenant.access.mcas.ms/aad_login"));
    for url in [
        "http://tenant.access.mcas.ms/aad_login",
        "https://user@tenant.access.mcas.ms/aad_login",
        "https://tenant.access.mcas.ms.evil.test/aad_login",
        "https://tenant.access.mcas.ms:444/aad_login",
        "https://tenant.access.mcas.ms/other",
    ] {
        assert!(!signin::notice_url(url), "{url}");
    }
}

#[test]
fn a_light_accounts_browser_is_sent_to_a_new_microsoft_sign_in_once_in_a_while() {
    run(async {
        let Some(c) = chrome().await else {
            eprintln!("CDP_CHROME not set: the browser test is skipped");
            return;
        };
        route(&c.page, |_| "<html><body>Teams</body></html>".into()).await;
        open(&c.page, "https://teams.cloud.microsoft/").await;
        seed_refresh_token(&c.page, "old-rt", 0).await;
        let now = now_s();
        let a = agent(c.page.clone());
        a.store.set_state(key::LIGHT_TOKEN, &json!({ "at": now - 90_000, "expiresOn": now - 3600, "refused": true }).to_string());
        stored("old-rt", now - 3600).save(&a.config.light_token_file).expect("token");
        assert!(signin::fresh_sign_in(&a).await);
        c.page
            .wait_for_js("location.search === '?loginHint=test.user%40contoso.example' && document.readyState === 'complete'", 5000)
            .await
            .expect("sent to the sign-in");
        assert_eq!(c.page.eval_js("localStorage.length").await.expect("storage"), json!(0), "the data of Teams cleared");
        assert!(a.store.get_state("fresh_sign_in").parse::<i64>().expect("when") >= now);
        assert!(!signin::fresh_sign_in(&a).await, "once in a while only");

        // a sign-in just done: Teams holds a new refresh token, the light-token job takes it
        seed_refresh_token(&c.page, "new-rt", 0).await;
        let b = agent(c.page.clone());
        b.store.set_state(key::LIGHT_TOKEN, &json!({ "at": now - 90_000, "expiresOn": now - 3600, "refused": true }).to_string());
        stored("old-rt", now - 3600).save(&b.config.light_token_file).expect("token");
        assert!(!signin::fresh_sign_in(&b).await);
        assert_eq!(b.store.get_state("fresh_sign_in"), "");

        // the sign-in holds, or a captured Microsoft session renews it over HTTP
        b.store.set_state(key::LIGHT_TOKEN, &json!({ "at": now - 60, "expiresOn": now + 80_000, "refused": false }).to_string());
        assert!(!signin::fresh_sign_in(&b).await);
        b.store.set_state(key::LIGHT_TOKEN, &json!({ "at": now - 86_400, "expiresOn": 1, "refused": false, "session": true }).to_string());
        assert!(!signin::fresh_sign_in(&b).await);
        assert_eq!(b.store.get_state("fresh_sign_in"), "");
    });
}

#[test]
fn the_sign_in_of_the_page_goes_to_the_light_host_with_its_microsoft_session() {
    run(async {
        let Some(c) = chrome().await else {
            eprintln!("CDP_CHROME not set: the browser test is skipped");
            return;
        };
        route(&c.page, |_| "<html><body>Teams</body></html>".into()).await;
        open(&c.page, "https://teams.cloud.microsoft/").await;
        seed_refresh_token(&c.page, "new-rt", 0).await;
        let now = now_s();
        let cookie = json!({ "name": "ESTSAUTHPERSISTENT", "value": "ests", "domain": ".login.microsoftonline.com", "path": "/", "secure": true, "httpOnly": true, "expires": now + 86_400 });
        c.browser.set_cookies(vec![cookie]).await.expect("cookie");
        let a = agent(c.page.clone());
        let file = a.config.light_token_file.clone();
        assert!(signin::capture_light_token(&a).await);
        let token = StoredToken::load(&file).expect("token saved");
        assert_eq!(
            (token.refresh_token.as_str(), token.client_id.as_str(), token.tenant_id.as_str(), token.region.as_str()),
            ("new-rt", TEAMS_WEB_CLIENT, "tenant", "emea")
        );
        assert!((token.expires_on - (now + 20 * 3600)).abs() < 5, "assumed 20 h without an expiry: {}", token.expires_on);
        let session = MicrosoftSession::load(&session_file(&file)).expect("session saved");
        assert_eq!((session.login_hint.as_str(), session.user_agent.as_str(), session.cookies.len()), (EMAIL, "TeamsRelayTest/1.0", 1));
        let row = state(&a, key::LIGHT_TOKEN);
        assert_eq!(row["refused"], json!(false));
        assert_eq!(row["session"], json!(true));
        assert_eq!(row["expiresOn"], json!(token.expires_on));
        assert!(row.get("browser").is_none() && row.get("mcas").is_none(), "{row}");
        assert!((row["at"].as_i64().expect("at") - now).abs() < 5);
        assert!(!signin::capture_light_token(&a).await, "the same sign-in is captured once");

        // a browser renewal about to end is not taken from a page that was not sent to a new sign-in
        let b = agent(c.page.clone());
        b.store.set_state(key::LIGHT_TOKEN, &json!({ "at": now - 80_000, "expiresOn": now + 30, "refused": false, "browser": true }).to_string());
        assert!(!signin::capture_light_token(&b).await);
        assert!(StoredToken::load(&b.config.light_token_file).is_none());
    });
}

/// The page of the sign-in fixture `template` shown, every click recorded in window.clicks by the text clicked.
fn microsoft_page(template: &str) -> String {
    let html =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test/agent/fixtures/sign-in-microsoft.html")).expect("fixture");
    format!(
        "{html}<body><script>document.body.append(document.getElementById('{template}').content.cloneNode(true)); window.clicks = []; addEventListener('click', (e) => clicks.push(e.target.closest('[role=button],button,input')?.innerText?.trim() || e.target.value || ''), true);</script></body>"
    )
}

#[test]
fn one_press_of_sign_in_in_teams_then_on_microsofts_page() {
    run(async {
        let Some(c) = chrome().await else {
            eprintln!("CDP_CHROME not set: the browser test is skipped");
            return;
        };
        let teams =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test/agent/fixtures/sign-in-teams.html")).expect("fixture");
        let served = Rc::new(RefCell::new(teams));
        let s = served.clone();
        route(&c.page, move |_| {
            format!("{}<script>window.clicks = []; addEventListener('click', (e) => clicks.push(e.target.innerText.trim()), true);</script>", s.borrow())
        })
        .await;
        open(&c.page, "https://teams.cloud.microsoft/v2/").await;
        let a = agent(c.page.clone());
        let clicks = |p: Rc<Page>| async move { p.eval_js("clicks").await.expect("clicks") };
        let now = now_s();
        a.store.set_state(key::LOGIN_WATCH, &json!({ "since": now - 5, "alerted": false }).to_string());
        signin::try_sign_in(&a).await;
        assert_eq!(a.store.get_state("sign_in_try"), "", "nothing in the first 15 s");
        a.store.set_state(key::LOGIN_WATCH, &json!({ "since": now - 20, "alerted": false }).to_string());
        signin::try_sign_in(&a).await;
        assert_eq!(clicks(c.page.clone()).await, json!(["Sign in"]));
        let t = state(&a, "sign_in_try");
        assert_eq!((t["pressed"].clone(), t["microsoft"].clone()), (json!(["teams"]), json!(false)));
        signin::try_sign_in(&a).await;
        assert_eq!(clicks(c.page.clone()).await, json!(["Sign in"]), "one press per sign-out");

        // Teams' button opened Microsoft's page: this account's tile, at a look after the first
        let login = c.browser.new_page("about:blank").await.expect("tab");
        let shown = Rc::new(RefCell::new(microsoft_page("pick-account")));
        let s = shown.clone();
        route(&login, move |_| s.borrow().clone()).await;
        open(&login, "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=x").await;
        signin::try_sign_in(&a).await;
        assert_eq!(clicks(login.clone()).await, json!([]), "the first look only starts watching the owner");
        signin::try_sign_in(&a).await;
        let pressed = clicks(login.clone()).await;
        assert_eq!(pressed.as_array().map(Vec::len), Some(1));
        assert!(pressed[0].as_str().is_some_and(|t| t.contains(EMAIL)), "{pressed}");
        let t = state(&a, "sign_in_try");
        assert_eq!((t["pressed"].clone(), t["microsoft"].clone()), (json!(["teams", "account"]), json!(true)));
        signin::try_sign_in(&a).await;
        assert_eq!(clicks(login.clone()).await.as_array().map(Vec::len), Some(1), "one press on Microsoft's page");

        // a later sign-out whose Microsoft page asks for a password: nothing pressed, ever
        *served.borrow_mut() = "<html><body>Teams</body></html>".into();
        open(&c.page, "https://teams.cloud.microsoft/v2/").await;
        let b = agent(c.page.clone());
        b.store.set_state(key::LOGIN_WATCH, &json!({ "since": now - 20, "alerted": false }).to_string());
        *shown.borrow_mut() = microsoft_page("password");
        open(&login, "https://login.microsoftonline.com/common/login").await;
        for _ in 0..3 {
            signin::try_sign_in(&b).await;
        }
        assert_eq!(clicks(login.clone()).await, json!([]));
        let t = state(&b, "sign_in_try");
        assert_eq!((t["pressed"].clone(), t["microsoft"].clone()), (json!([]), json!(true)));
    });
}

// page-live-fixtures.test.ts and page-chat-list.test.ts through the Rust page layer: the captured chat list read and
// saved, and the header of a group chat without a name taken for its chat of the list
#[test]
fn the_captured_chat_list_saved_and_the_open_group_chat_recognized() {
    run(async {
        let Some(c) = chrome().await else {
            eprintln!("CDP_CHROME not set: the browser test is skipped");
            return;
        };
        crate::agent::browser::testing::fixture(&c.page, "chat-list.html").await;
        let a = agent(c.page.clone());
        let rows = jobs::read_list(&a).await.expect("list");
        assert_eq!(rows.len(), 40);
        assert_eq!(rows.iter().map(|r| r.entry.name.as_str()).collect::<HashSet<_>>().len(), 40);
        let presences: HashSet<&str> = rows.iter().map(|r| r.entry.presence.as_str()).filter(|p| !p.is_empty()).collect();
        assert_eq!(presences, ["available", "away", "offline", "ooo"].into_iter().collect::<HashSet<&str>>());
        assert!(rows.iter().filter(|r| !r.entry.presence.is_empty()).all(|r| r.entry.kind == "one"));
        assert!(jobs::read_chats(&a, false).await.expect("read"));
        assert_eq!(a.store.chats().len(), 40);
        assert!(a.store.get_state("last_scan_ts").parse::<i64>().is_ok_and(|t| t >= now_s() - 5));

        let person = |n: u32, name: &str| {
            format!(
                r#"<li role="listitem" data-tid="chat-topic-menu-list-item"><span data-tid="participant-8:orgid:{n}"><span id="chat-topic-person-8:orgid:{n}">{name}</span></span><div data-tid="presence"></div></li>"#
            )
        };
        let header = format!(
            r#"<h2 data-tid="chat-title"><div><ul role="list" data-tid="chat-topic-menu">{}{}{}</ul><span>+3</span></div></h2><button data-tid="chat-header-participant-count" aria-label="View and add participants, 6 participants">6</button>"#,
            person(1, "Mohamed, Faraz"),
            person(2, "P, Pravin"),
            person(3, "Renton, John")
        );
        crate::agent::browser::testing::set_content(&c.page, &header).await;
        let mohamed = ChatEntry { name: "Mohamed, Faraz, P, Pravin, +4".into(), ..ChatEntry::default() };
        let one = ChatEntry { name: "Mohamed, Faraz".into(), ..ChatEntry::default() };
        a.store.save_chats(&[mohamed, one], true).expect("chats");
        let tp = a.tp().expect("page");
        assert!(tp.is_open("Mohamed, Faraz, P, Pravin, +4").await.expect("header"));
        assert!(!tp.is_open("Mohamed, Faraz").await.expect("header"));
    });
}

const NORMAL: &str = r#"{"TITLE_ID":"PROXYWEB_MONITORED_ACCESS_MESSAGE","DESCRIPTION_ID":"PROXYWEB_MONITORED_ACCESS_DESCRIPTION","DISPLAY_DISCLAIMER":true,"REDIRECT_TO_SUFFIX_METHOD":"POST"}"#;
const GATEWAY: &str = "https://tenant.access.mcas.ms/aad_login";

fn notice(context: &str, buttons: usize) -> String {
    format!(
        "<script>window.context={context}</script><p>Access is monitored</p><label><input type=\"checkbox\">Don't show this message again for a week</label>{}",
        "<button onclick=\"document.body.dataset.continued='yes'\">Continue</button>".repeat(buttons)
    )
}

#[test]
fn the_monitored_access_notice_continued_once_per_document_never_under_the_owner() {
    run(async {
        let Some(c) = chrome().await else {
            eprintln!("CDP_CHROME not set: the browser test is skipped");
            return;
        };
        let p = c.page.clone();
        let shown = Rc::new(RefCell::new(notice(NORMAL, 1)));
        let s = shown.clone();
        route(&p, move |_| s.borrow().clone()).await;
        let continued = || async { p.eval_js("document.body.dataset.continued || null").await.expect("body") };
        open(&p, GATEWAY).await;
        assert!(!signin::continue_mcas(&p).await, "the first look starts watching the owner");
        assert!(signin::continue_mcas(&p).await);
        assert_eq!(continued().await, json!("yes"));
        assert_eq!(p.eval_js("document.querySelector('input[type=checkbox]').checked").await.expect("box"), json!(true));
        assert!(!signin::continue_mcas(&p).await, "once per document");

        for (url, context) in [
            (GATEWAY, NORMAL.replace("\"POST\"", "\"POST\",\"CUSTOMIZED_BODY\":\"terms\"")),
            ("https://tenant.access.mcas.ms.evil.test/aad_login", NORMAL.to_string()),
        ] {
            *shown.borrow_mut() = notice(&context, 1);
            open(&p, url).await;
            signin::continue_mcas(&p).await;
            assert!(!signin::continue_mcas(&p).await, "{url}");
            assert_eq!(continued().await, Value::Null, "{url}");
        }

        // two Continue: neither pressed
        *shown.borrow_mut() = notice(NORMAL, 2);
        open(&p, GATEWAY).await;
        signin::continue_mcas(&p).await;
        assert!(!signin::continue_mcas(&p).await);
        assert_eq!(continued().await, Value::Null);

        // the owner clicks on the notice: it is theirs
        *shown.borrow_mut() = notice(NORMAL, 1);
        open(&p, GATEWAY).await;
        signin::continue_mcas(&p).await;
        sleep(1100).await;
        p.mouse_click(5.0, 5.0).await.expect("owner click");
        assert!(!signin::continue_mcas(&p).await);
        assert_eq!(continued().await, Value::Null);
    });
}
