use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicUsize, Ordering};

const OWNER: &str = "owner-id";

/// The directory and the supervisor as the tests answer them, every call recorded.
#[derive(Default)]
struct Fake {
    /// the list; None: no broker (404)
    list: RefCell<Option<Value>>,
    lists: Cell<usize>,
    /// the next answers of set, in order; then {ok, account, shared}
    set_answers: RefCell<Vec<Option<Value>>>,
    sets: RefCell<Vec<Value>>,
    reports: RefCell<Vec<Value>>,
    connect: RefCell<Value>,
    connects: RefCell<Vec<Value>>,
    stop_fails: Cell<bool>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn new(list: Value) -> Rc<Fake> {
        let f = Fake { list: RefCell::new(Some(list)), connect: RefCell::new(json!({"ok": false, "interaction": true})), ..Default::default() };
        Rc::new(f)
    }

    fn act(&self, what: &str, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        let failed = what == "stop" && self.stop_fails.get();
        if !failed {
            self.calls.borrow_mut().push(format!("{what} {n}"));
        }
        Box::pin(async move { if failed { Err(err(502, "supervisor busy")) } else { Ok(()) } })
    }

    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }
}

impl Broker for Fake {
    fn list(&self) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        self.lists.set(self.lists.get() + 1);
        let v = self.list.borrow().clone();
        Box::pin(async move { Ok(v) })
    }
    fn set(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        self.sets.borrow_mut().push(body.clone());
        let waiting = !self.set_answers.borrow().is_empty();
        let queued = waiting.then(|| self.set_answers.borrow_mut().remove(0));
        let answer = queued.unwrap_or_else(|| {
            Some(json!({ "ok": true, "account": body["account"], "shared": { "capabilities": body.get("capabilities"), "origin": "mobile" } }))
        });
        Box::pin(async move { Ok(answer) })
    }
    fn report(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        self.reports.borrow_mut().push(body);
        Box::pin(async { Ok(Some(json!({"ok": true}))) })
    }
    fn connect(&self, body: Value) -> LocalBoxFuture<'_, Res<Option<Value>>> {
        self.connects.borrow_mut().push(body);
        let v = self.connect.borrow().clone();
        Box::pin(async move { Ok(Some(v)) })
    }
    fn start(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        self.act("start", n)
    }
    fn light(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        self.act("light", n)
    }
    fn stop(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        self.act("stop", n)
    }
    fn signin(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        self.act("signin", n)
    }
    fn wipe(&self, n: i64) -> LocalBoxFuture<'_, Res<()>> {
        self.act("wipe", n)
    }
}

static DIRS: AtomicUsize = AtomicUsize::new(0);

/// The environment is the test's while the guard lives: one per test, before its worlds.
fn env() -> MutexGuard<'static, ()> {
    crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner())
}

/// A fresh app.db and data folder, under the guard of env().
struct World {
    dir: std::path::PathBuf,
    identities: Rc<RefCell<HashMap<i64, String>>>,
}

fn set_env(k: &str, v: &str) {
    // SAFETY: the tests that set the environment hold TEST_ENV
    unsafe { std::env::set_var(k, v) };
}

impl World {
    fn new() -> World {
        let dir = std::env::temp_dir().join(format!("trweb-shared-{}-{}", std::process::id(), DIRS.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        set_env("APP_DB", dir.join("app.db").to_str().expect("path"));
        set_env("APP_URL", "https://relay.example");
        set_env("SHARED_ACCOUNTS_URL", "https://calendar.example");
        set_env("SHARED_ACCOUNTS_OWNER", "owner@example.com");
        appdb::with(|c| {
            appdb::migrate(c)?;
            crate::auth::rows::migrate(c)
        })
        .expect("schema");
        World { dir, identities: Rc::default() }
    }

    fn deps(&self, broker: &Rc<Fake>) -> Deps {
        let ids = self.identities.clone();
        Deps {
            broker: broker.clone(),
            per_user: None,
            owner: Some(OWNER.into()),
            identity: Rc::new(move |s: &Slot| ids.borrow().get(&s.slot).cloned().unwrap_or_default()),
        }
    }

    fn id(&self, slot: i64, address: &str) {
        self.identities.borrow_mut().insert(slot, address.into());
    }

    fn sync(&self, broker: &Rc<Fake>) -> Res<SyncResult> {
        run(sync_shared_accounts(&self.deps(broker)))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
    tokio::task::LocalSet::new().block_on(&rt, f)
}

fn shared(account: &str, caps: &[&str]) -> Value {
    json!({ "account": account, "shared": { "capabilities": caps, "origin": "mobile" } })
}

fn list(accounts: Vec<Value>) -> Value {
    json!({ "ok": true, "accounts": accounts })
}

fn row(n: i64) -> Option<Slot> {
    appdb::slot_row(n).expect("row")
}

fn claim(owner: &str) -> i64 {
    appdb::claim_slot(owner, None).expect("slot")
}

fn now() -> i64 {
    crate::web::now_s()
}

#[test]
fn the_list_of_the_agent() {
    let raw = json!({ "ok": true, "accounts": [
        { "account": "A@example.com", "shared": null, "status": "ok" },
        { "account": "b@example.com", "shared": { "capabilities": ["chat", "admin"] }, "status": "errore", "message": "Accesso scaduto" },
        { "account": "not-an-address", "shared": null },
        null,
    ]});
    let got: Vec<Value> = parse_directory(&raw).expect("list").iter().map(|a| Value::Object(a.json())).collect();
    assert_eq!(
        got,
        [
            json!({"account": "a@example.com", "capabilities": ["mail", "calendar"], "shared": false, "status": "ok", "message": ""}),
            json!({"account": "b@example.com", "capabilities": ["chat"], "shared": true, "status": "error", "message": "Accesso scaduto"}),
        ]
    );
    let e = parse_directory(&json!({ "ok": false, "errore": "gestore token non configurato" })).unwrap_err();
    assert_eq!(message(&e), "Shared accounts not read: gestore token non configurato");
    assert_eq!(message(&parse_directory(&json!({ "accounts": "x" })).unwrap_err()), "Shared accounts not read: unexpected answer");
}

#[test]
fn teams_on_gives_a_new_bound_slot_off_stops_it_on_again_starts_it() {
    let _env = env();
    let w = World::new();
    let b = Fake::new(list(vec![
        shared("Mario@Example.com", &["mail", "chat"]),
        shared("anna@example.com", &["mail", "calendar"]),
        json!({"account": "legacy@example.com", "shared": null}),
    ]));
    let r = w.sync(&b).expect("pass");
    assert!(r.enabled && r.errors.is_empty());
    assert_eq!(b.calls(), ["stop 1", "wipe 1", "light 1"]);
    let s = row(1).expect("slot 1");
    assert_eq!((s.owner_id.as_str(), s.light, s.stopped, s.shared_account.as_str(), s.shared_region.as_str()), (OWNER, 1, 0, "mario@example.com", "emea"));
    assert!(row(2).is_none());

    // a second pass changes nothing
    w.sync(&b).expect("pass");
    assert!(b.calls().is_empty());

    *b.list.borrow_mut() = Some(list(vec![shared("mario@example.com", &["mail"])]));
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), ["stop 1"]);
    assert_eq!(row(1).expect("kept").stopped, 1);

    *b.list.borrow_mut() = Some(list(vec![shared("mario@example.com", &["mail", "chat"])]));
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), ["light 1"]);
    assert_eq!(row(1).expect("kept").stopped, 0);

    // a slot the owner switched off starts again only with Teams on
    appdb::set_slot_stopped(1, true).expect("stop");
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), ["light 1"]);

    // gone from the calendar: stopped, kept
    *b.list.borrow_mut() = Some(list(vec![]));
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), ["stop 1"]);
    assert_eq!(row(1).expect("kept").stopped, 1);
}

#[test]
fn a_failed_stop_is_an_error_of_its_account_and_is_tried_again() {
    let _env = env();
    let w = World::new();
    let b = Fake::new(list(vec![shared("mario@example.com", &["chat"])]));
    w.sync(&b).expect("pass");
    b.calls();
    *b.list.borrow_mut() = Some(list(vec![shared("mario@example.com", &["mail"])]));
    b.stop_fails.set(true);
    let r = w.sync(&b).expect("pass");
    assert_eq!(r.errors["mario@example.com"], "supervisor busy");
    assert_eq!(row(1).expect("slot").stopped, 0);
    b.stop_fails.set(false);
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), ["stop 1"]);
    assert_eq!(row(1).expect("slot").stopped, 1);
}

#[test]
fn a_browser_slot_of_the_owner_is_kept_another_users_never_taken_a_busy_one_waits() {
    let _env = env();
    let w = World::new();
    let first = claim(OWNER);
    let own = claim(OWNER);
    w.id(first, "anna@example.com");
    w.id(own, "mario@example.com");
    let b = Fake::new(list(vec![shared("mario@example.com", &["chat"])]));
    let r = w.sync(&b).expect("pass");
    assert_eq!(r.errors["mario@example.com"], BROWSER_PRESERVED);
    assert!(b.calls().is_empty());
    assert_eq!(row(own).expect("own").shared_account, "");

    // busy with a check: no error, the next pass decides
    appdb::begin_check(own, 5).expect("check");
    assert!(w.sync(&b).expect("pass").errors.is_empty());
    assert!(b.calls().is_empty());

    // a slot of another user on the same address: a new slot of the owner
    let w = World::new();
    let other = claim("someone-else");
    w.id(other, "mario@example.com");
    w.sync(&b).expect("pass");
    assert_eq!(row(other).expect("other").shared_account, "");
    assert!(b.calls().contains(&"light 2".to_string()));
}

#[test]
fn the_account_limit_is_an_error_of_that_account_and_nothing_changes_without_the_list() {
    let _env = env();
    let w = World::new();
    claim(OWNER);
    let b = Fake::new(list(vec![shared("mario@example.com", &["chat"])]));
    let r = run(sync_shared_accounts(&Deps { per_user: Some(1), ..w.deps(&b) })).expect("pass");
    assert!(r.errors["mario@example.com"].as_str().expect("error").contains("at most 1"), "{:?}", r.errors);
    assert!(b.calls().is_empty());

    *b.list.borrow_mut() = None;
    assert!(!w.sync(&b).expect("pass").enabled);
    assert!(!run(sync_shared_accounts(&Deps { owner: None, ..w.deps(&b) })).expect("pass").enabled);
    *b.list.borrow_mut() = Some(
        json!({ "ok": false, "errore": "gestore token non configurato", "commands": [{ "id": "bad", "account": "a@example.com", "operation": "remove" }] }),
    );
    let own = claim(OWNER);
    w.id(own, "a@example.com");
    assert!(message(&w.sync(&b).err().expect("refused")).contains("gestore token"));
    assert!(row(own).is_some());
    assert!(b.calls().is_empty());
}

#[test]
fn the_pass_reads_the_list_inside_the_queue() {
    let _env = env();
    let w = World::new();
    let b = Fake::new(list(vec![shared("mario@example.com", &["chat"])]));
    let d = w.deps(&b);
    run(async {
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let queue = tokio::task::spawn_local(crate::slots::exclusive(async {
            let _ = held.await;
        }));
        tokio::task::yield_now().await;
        let pass = sync_shared_accounts(&d);
        // Teams turned off from a page while the pass waits
        *b.list.borrow_mut() = Some(list(vec![shared("mario@example.com", &["mail"])]));
        let _ = release.send(());
        queue.await.expect("queue");
        pass.await.expect("pass");
    });
    assert!(b.calls().is_empty());
    assert!(row(1).is_none());
}

fn signins(jobs: Value) -> Value {
    json!({ "ok": true, "accounts": [], "signins": jobs })
}

fn progress_of(b: &Fake) -> Vec<(Value, Value)> {
    b.sets.borrow().iter().filter(|s| s["operation"] == "signin-progress").map(|s| (s["slot"].clone(), s["phase"].clone())).collect()
}

#[test]
fn guided_sign_ins() {
    let _env = env();
    // a job whose slot was removed: a new slot, its progress saved first
    let w = World::new();
    let b = Fake::new(signins(json!([{ "id": "retry", "account": "new@example.com", "capabilities": ["mail"], "phase": "queued", "slot": 9 }])));
    w.sync(&b).expect("pass");
    assert_eq!(row(1).expect("slot").owner_id, OWNER);
    assert_eq!(progress_of(&b)[0], (json!(1), json!("browser")));
    let first = b.sets.borrow()[0].clone();
    assert_eq!(
        first,
        json!({"operation": "signin-progress", "account": "new@example.com", "id": "retry", "slot": 1, "phase": "browser", "url": "https://relay.example/api/desktop/1"})
    );
    // mail only: a sign-in browser, Teams stopped, light
    assert_eq!(b.calls(), ["stop 1", "wipe 1", "signin 1", "signin 1"]);
    let s = row(1).expect("slot");
    assert_eq!((s.stopped, s.light), (1, 1));

    // an unfinished mail-only profile resumes without a wipe or Teams
    let w = World::new();
    let slot = claim(OWNER);
    appdb::set_slot_stopped(slot, true).expect("stop");
    let b = Fake::new(signins(json!([{ "id": "retry", "account": "new@example.com", "capabilities": ["mail"], "phase": "queued", "slot": slot }])));
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), [format!("signin {slot}")]);
    assert_eq!(row(slot).expect("slot").stopped, 1);

    // the cleanup of an old blocked job does not stop a new profile on the same slot
    let b = Fake::new(signins(json!([
        { "id": "old", "account": "old@example.com", "phase": "blocked", "slot": slot, "capabilities": ["mail"] },
        { "id": "new", "account": "new@example.com", "phase": "browser", "slot": slot, "capabilities": ["mail"] },
    ])));
    w.sync(&b).expect("pass");
    assert!(!b.calls().contains(&format!("stop {slot}")));

    // a slot claimed for a sign-in that stopped before anyone signed in: given back
    let w = World::new();
    let slot = claim(OWNER);
    let b = Fake::new(signins(
        json!([{ "id": "bip", "account": "new@example.com", "capabilities": ["mail", "chat"], "phase": "blocked", "slot": slot, "created": now() - 60, "ended": now() + 5 }]),
    ));
    w.sync(&b).expect("pass");
    assert_eq!(b.calls(), [format!("stop {slot}"), format!("wipe {slot}")]);
    assert!(row(slot).is_none());

    // never one claimed after the sign-in ended, nor one signed in to an account
    let w = World::new();
    let later = claim(OWNER);
    let signed_in = claim(OWNER);
    w.id(signed_in, "new@example.com");
    let b = Fake::new(signins(json!([
        { "id": "old", "account": "old@example.com", "capabilities": ["mail"], "phase": "blocked", "slot": later, "created": now() - 1000, "ended": now() - 700 },
        { "id": "new", "account": "new@example.com", "capabilities": ["mail"], "phase": "blocked", "slot": signed_in, "created": now() - 60, "ended": now() + 5 },
    ])));
    w.sync(&b).expect("pass");
    assert!(b.calls().is_empty());
    assert!(row(later).is_some() && row(signed_in).is_some());

    // Teams and mail: a browser started, light from the start as a sign-in run, one wipe
    let w = World::new();
    let b = Fake::new(signins(json!([{ "id": "job", "account": "new@example.com", "capabilities": ["mail", "chat"], "phase": "queued" }])));
    w.sync(&b).expect("pass");
    let calls = b.calls();
    assert!(calls.contains(&"start 1".to_string()), "{calls:?}");
    assert_eq!(calls.iter().filter(|c| c.starts_with("wipe")).count(), 1);
    let s = row(1).expect("slot");
    assert_eq!((s.light, s.refresh_result.as_str()), (1, "asked"));
    assert_eq!(*b.connects.borrow(), [json!({"id": "job", "account": "new@example.com", "slot": 1})]);

    // a different address signed in: blocked with the reason
    let w = World::new();
    let slot = claim(OWNER);
    w.id(slot, "someone@example.com");
    let b = Fake::new(signins(json!([{ "id": "job", "account": "new@example.com", "capabilities": ["mail"], "phase": "browser", "slot": slot }])));
    w.sync(&b).expect("pass");
    assert_eq!(b.sets.borrow().last().expect("progress")["error"], "Sign-in slot ownership changed");
}

#[test]
fn a_guided_sign_in_reuses_the_owners_browser_and_completes_when_teams_runs() {
    let _env = env();
    let w = World::new();
    let foreign = claim("other");
    let own = claim(OWNER);
    w.id(foreign, "new@example.com");
    w.id(own, "new@example.com");
    let health = crate::slotdb::open_store(&crate::slotdb::slot_db_path(own)).expect("slot db");
    health.execute("INSERT INTO state(k, v) VALUES('health', ?)", [json!({ "ts": now(), "teams": "ok", "overall": "green" }).to_string()]).expect("health");
    let b = Fake::new(signins(json!([{ "id": "job", "account": "new@example.com", "capabilities": ["mail", "chat"], "phase": "queued" }])));
    *b.connect.borrow_mut() = json!({ "ok": true, "route": "outlook-web" });
    w.sync(&b).expect("pass");
    assert_eq!(*b.connects.borrow(), [json!({"id": "job", "account": "new@example.com", "slot": own})]);
    assert_eq!(progress_of(&b).last().expect("progress"), &(json!(own), json!("complete")));
    assert_eq!(row(own).expect("own").shared_account, "");
    assert!(!b.calls().contains(&format!("stop {foreign}")));

    // mail only through a route: the identity of the mailbox is kept for the profile
    let w = World::new();
    let b = Fake::new(signins(json!([{ "id": "mail-job", "account": "mail@example.com", "capabilities": ["mail"], "phase": "queued" }])));
    *b.connect.borrow_mut() = json!({ "ok": true, "route": "outlook-web" });
    run(sync_shared_accounts(&Deps { identity: Rc::new(email_of), ..w.deps(&b) })).expect("pass");
    assert_eq!(crate::accounts::account_summary(&row(1).expect("slot")).expect("summary")["email"], "mail@example.com");
    let others = shared_report(OWNER, &(Rc::new(email_of) as IdentityOf)).expect("report")["others"].clone();
    assert_eq!((&others[0]["email"], &others[0]["slot"], &others[0]["state"]), (&json!("mail@example.com"), &json!(1), &json!("stopped")));
}

#[test]
fn one_automatic_repair_of_an_expired_session() {
    let _env = env();
    let w = World::new();
    let account = json!({ "account": "expired@example.com", "shared": { "capabilities": ["mail"], "origin": "outlook-spa" }, "status": "errore", "message": "Sessione del browser scaduta" });
    let b = Fake::new(json!({ "ok": true, "accounts": [account.clone()], "signins": [] }));
    w.sync(&b).expect("pass");
    assert_eq!(*b.sets.borrow(), [json!({"operation": "connect", "account": "expired@example.com", "capabilities": ["mail"]})]);
    let b = Fake::new(json!({ "ok": true, "accounts": [account], "signins": [{ "account": "expired@example.com", "phase": "blocked" }] }));
    w.sync(&b).expect("pass");
    assert!(b.sets.borrow().is_empty());
}

fn commands(cmds: Value) -> Value {
    json!({ "ok": true, "accounts": [], "commands": cmds })
}

#[test]
fn commands_of_the_directory() {
    let _env = env();
    // remove: every slot of the owner on the address, never another owner's
    let w = World::new();
    let own = claim(OWNER);
    let foreign = claim("other");
    w.id(own, "a@example.com");
    w.id(foreign, "a@example.com");
    let b = Fake::new(commands(json!([{ "id": "remove-1", "account": "a@example.com", "operation": "remove" }])));
    w.sync(&b).expect("pass");
    assert!(row(own).is_none() && row(foreign).is_some());
    assert_eq!(b.calls(), [format!("stop {own}"), format!("wipe {own}")]);
    assert_eq!(b.reports.borrow()[0]["results"], json!([{"id": "remove-1"}]));

    // a browser slot stopped and resumed without a wipe or a binding
    let w = World::new();
    let n = claim(OWNER);
    w.id(n, "a@example.com");
    for on in [false, true] {
        let b = Fake::new(commands(json!([{ "id": "chat", "account": "a@example.com", "operation": "chat", "on": on }])));
        w.sync(&b).expect("pass");
        assert_eq!(b.calls(), [format!("{} {n}", if on { "start" } else { "stop" })]);
        assert_eq!(row(n).expect("slot").stopped, i64::from(!on));
    }
    assert_eq!(row(n).expect("slot").shared_account, "");

    // calls of a slot without a browser, reported with the new state
    let w = World::new();
    let b = Fake::new(list(vec![shared("a@example.com", &["chat"])]));
    w.sync(&b).expect("pass");
    assert_eq!(shared_report(OWNER, &w.deps(&b).identity).expect("report")["accounts"][0]["calls"], true);
    let mut with_command = list(vec![shared("a@example.com", &["chat"])]);
    with_command["commands"] = json!([{ "id": "calls", "account": "a@example.com", "operation": "calls", "on": false }]);
    *b.list.borrow_mut() = Some(with_command);
    w.sync(&b).expect("pass");
    assert_eq!(row(1).expect("slot").calls, 0);
    let report = b.reports.borrow()[0].clone();
    assert_eq!(
        (&report["accounts"][0]["account"], &report["accounts"][0]["calls"], &report["results"]),
        (&json!("a@example.com"), &json!(false), &json!([{"id": "calls"}]))
    );

    // no calls switch for a browser slot, an address without a slot, or no on
    let w = World::new();
    let n = claim(OWNER);
    w.id(n, "b@example.com");
    let b = Fake::new(commands(json!([
        { "id": "browser", "account": "b@example.com", "operation": "calls", "on": false },
        { "id": "none", "account": "nobody@example.com", "operation": "calls", "on": true },
        { "id": "bad", "account": "b@example.com", "operation": "calls" },
    ])));
    w.sync(&b).expect("pass");
    assert_eq!(row(n).expect("slot").calls, 1);
    assert_eq!(
        b.reports.borrow()[0]["results"],
        json!([{"id": "browser", "error": NO_CALLS_SWITCH}, {"id": "none", "error": NO_CALLS_SWITCH}, {"id": "bad", "error": NO_CALLS_SWITCH}])
    );

    // a failure is reported, the slot kept
    let w = World::new();
    let n = claim(OWNER);
    w.id(n, "a@example.com");
    let b = Fake::new(commands(json!([{ "id": "remove", "account": "a@example.com", "operation": "remove" }])));
    b.stop_fails.set(true);
    w.sync(&b).expect("pass");
    assert!(row(n).is_some());
    assert_eq!(b.reports.borrow()[0]["results"], json!([{"id": "remove", "error": "supervisor busy"}]));

    // both the browser and the bound slot of one address go
    let w = World::new();
    let one = claim(OWNER);
    let two = claim(OWNER);
    w.id(two, "a@example.com");
    appdb::bind_shared(one, "a@example.com", "emea").expect("bind");
    let b = Fake::new(commands(json!([{ "id": "both", "account": "a@example.com", "operation": "remove" }])));
    w.sync(&b).expect("pass");
    assert!(row(one).is_none() && row(two).is_none());
    assert_eq!(b.calls(), [format!("stop {one}"), format!("wipe {one}"), format!("stop {two}"), format!("wipe {two}")]);

    // a mailbox-only removal touches no other slot
    let w = World::new();
    let n = claim(OWNER);
    w.id(n, "other@example.com");
    let b = Fake::new(commands(json!([{ "id": "mail", "account": "a@example.com", "operation": "remove" }])));
    w.sync(&b).expect("pass");
    assert!(row(n).is_some() && b.calls().is_empty());
    assert_eq!(b.reports.borrow()[0]["results"], json!([{"id": "mail"}]));
}

#[test]
fn teams_on_or_off_from_this_app() {
    let _env = env();
    let w = World::new();
    let b = Fake::new(list(vec![json!({"account": "legacy@example.com", "shared": null}), shared("mario@example.com", &["mail", "chat"])]));
    assert_eq!(run(set_shared_capability(b.as_ref(), "Legacy@Example.com", "chat", true)).expect("on"), ["mail", "calendar", "chat"]);
    assert_eq!(run(set_shared_capability(b.as_ref(), "mario@example.com", "chat", false)).expect("off"), ["mail"]);
    assert_eq!(
        *b.sets.borrow(),
        [
            json!({"account": "legacy@example.com", "capabilities": ["mail", "calendar", "chat"]}),
            json!({"account": "mario@example.com", "capabilities": ["mail"]})
        ]
    );
    let e = run(set_shared_capability(b.as_ref(), "nobody@example.com", "chat", true)).unwrap_err();
    assert!(matches!(&e, Error::Http(404, d, _) if d == "nobody@example.com is not in the shared accounts of the calendar"), "{e:?}");

    // nothing sent when it is already so; 404 without a list; a refusal is a 409 with its reason
    let b = Fake::new(list(vec![shared("mario@example.com", &["mail", "chat"])]));
    assert_eq!(run(set_shared_capability(b.as_ref(), "Mario@Example.com", "calendar", true)).expect("on"), ["mail", "calendar", "chat"]);
    assert_eq!(run(set_shared_capability(b.as_ref(), "mario@example.com", "mail", true)).expect("same"), ["mail", "chat"]);
    assert_eq!(b.sets.borrow().len(), 1);
    *b.list.borrow_mut() = None;
    assert!(matches!(run(set_shared_capability(b.as_ref(), "mario@example.com", "chat", false)), Err(Error::Http(404, _, _))));
    let b = Fake::new(list(vec![shared("mario@example.com", &["mail"])]));
    b.set_answers.borrow_mut().push(Some(json!({"ok": false, "errore": "Capacita chat non disponibile per questa casella (csa)"})));
    assert!(matches!(run(set_shared_capability(b.as_ref(), "mario@example.com", "chat", true)), Err(Error::Http(409, d, _)) if d.contains("csa")));
    drop(w);
}

#[test]
fn the_owner_by_address_and_the_report() {
    let _env = env();
    let w = World::new();
    let owner = crate::auth::rows::create_user("Owner@Example.com", "Owner", "admin", 0).expect("user");
    assert_eq!(owner_of("owner@example.com").expect("owner"), Some(owner.id.clone()));
    assert_eq!(shared_owner_id().expect("owner"), Some(owner.id));
    assert_eq!(owner_of("nobody@example.com").expect("none"), None);
    assert_eq!(owner_of("").expect("none"), None);

    let b = Fake::new(list(vec![shared("mario@example.com", &["chat"])]));
    w.sync(&b).expect("pass");
    let own = claim(OWNER);
    claim("someone-else");
    appdb::set_slot_stopped(own, true).expect("stop");
    w.id(own, "anna@example.com");
    let r = shared_report(OWNER, &w.deps(&b).identity).expect("report");
    let a = &r["accounts"][0];
    assert_eq!((&a["account"], &a["slot"], &a["calls"]), (&json!("mario@example.com"), &json!(1), &json!(true)));
    assert!(regex::Regex::new("^[a-z][a-z-]*$").expect("fixed").is_match(a["state"].as_str().expect("state")), "{a}");
    assert_eq!(r["others"], json!([{"email": "anna@example.com", "slot": own, "state": "stopped", "detail": ""}]));
}

#[test]
fn the_route_of_the_one_list() {
    let _env = env();
    let w = World::new();
    let owner = crate::auth::rows::create_user("owner@example.com", "Owner", "admin", 0).expect("user").id;
    let other = crate::auth::rows::create_user("other@example.com", "Other", "user", 0).expect("user").id;
    let b = Fake::new(list(vec![shared("mario@example.com", &["mail", "chat"]), json!({"account": "legacy@example.com", "shared": null})]));
    let deps = Deps { owner: Some(owner.clone()), identity: Rc::new(email_of), ..w.deps(&b) };
    run(sync_shared_accounts(&deps)).expect("pass");
    let own = claim(&owner);
    b.calls();

    let r = run(list_for(b.as_ref(), &owner)).expect("list");
    assert_eq!((&r["enabled"], &r["addUrl"]), (&json!(true), &json!("https://calendar.example")));
    let accounts: Vec<Value> =
        r["accounts"].as_array().expect("accounts").iter().map(|a| json!([a["account"], a["capabilities"], a["shared"], a["slot"]])).collect();
    assert_eq!(accounts, [json!(["mario@example.com", ["mail", "chat"], true, 1]), json!(["legacy@example.com", ["mail", "calendar"], false, 0])]);
    assert_eq!(r["teamsOnly"].as_array().expect("teams only").iter().map(|t| t["slot"].clone()).collect::<Vec<_>>(), [json!(own)]);
    assert_eq!(
        Value::Object(r.as_object().expect("object").clone()).as_object().expect("o").keys().collect::<Vec<_>>(),
        ["enabled", "addUrl", "accounts", "teamsOnly", "commands", "signins", "labels"]
    );

    // anyone else sees nothing and changes nothing
    assert_eq!(run(list_for(b.as_ref(), &other)).expect("list"), json!({"enabled": false, "addUrl": "", "accounts": [], "teamsOnly": []}));
    let post = |user: &str, body: Value| run(change(b.clone(), user, body.as_object().expect("object")));
    assert!(
        matches!(post(&other, json!({"account": "mario@example.com", "capability": "chat", "on": false})), Err(Error::Http(404, d, _)) if d == "Not found")
    );
    assert!(b.sets.borrow().is_empty());

    // mail or calendar: the list only, no pass
    b.lists.set(0);
    post(&owner, json!({"account": "mario@example.com", "capability": "calendar", "on": true})).expect("changed");
    assert_eq!(*b.sets.borrow(), [json!({"account": "mario@example.com", "capabilities": ["mail", "calendar", "chat"]})]);
    assert_eq!(b.lists.get(), 2);
    assert!(b.calls().is_empty());
    b.sets.borrow_mut().clear();

    // Teams on for a mailbox: a new slot runs on it
    let r = post(&owner, json!({"account": "legacy@example.com", "capability": "chat", "on": true})).expect("changed");
    assert_eq!(*b.sets.borrow(), [json!({"account": "legacy@example.com", "capabilities": ["mail", "calendar", "chat"]})]);
    b.sets.borrow_mut().clear();
    // the fake keeps its list: the pass binds the new slot only once the list says so
    *b.list.borrow_mut() = Some(list(vec![shared("mario@example.com", &["mail", "chat"]), shared("legacy@example.com", &["mail", "calendar", "chat"])]));
    let r2 = post(&owner, json!({"account": "legacy@example.com", "capability": "chat", "on": true})).expect("same");
    assert!(r.get("error").is_none());
    let slot =
        r2["accounts"].as_array().expect("accounts").iter().find(|a| a["account"] == "legacy@example.com").expect("legacy")["slot"].as_i64().expect("slot");
    assert!(slot > own);
    let s = row(slot).expect("slot");
    assert_eq!((s.shared_account.as_str(), s.light, s.owner_id.as_str()), ("legacy@example.com", 1, owner.as_str()));
    assert_eq!(b.calls(), [format!("stop {slot}"), format!("wipe {slot}"), format!("light {slot}")]);

    // refused before anything is sent
    for body in [
        json!({"account": "mario@example.com", "capability": "admin", "on": true}),
        json!({"account": "mario@example.com", "capability": "chat", "on": "yes"}),
        json!({"account": "mario@example.com", "capabilities": ["chat"]}),
        json!({"account": "x", "capability": "chat", "on": true}),
        json!({"account": "mario@example.com", "operation": "connect", "capabilities": ["chat", "admin"]}),
        json!({"account": "mario@example.com", "operation": "signin-cancel"}),
    ] {
        assert!(matches!(post(&owner, body.clone()), Err(Error::Http(400, _, _))), "{body}");
    }
    assert!(b.sets.borrow().is_empty());

    // onboarding is never forwarded to an older agent, which would read it as a change of capabilities
    assert!(matches!(post(&owner, json!({"account": "legacy@example.com", "operation": "connect", "capabilities": ["mail"]})), Err(Error::Http(503, _, _))));

    // calls: of a slot without a browser; none for an address with a browser slot only
    let r = post(&owner, json!({"account": "mario@example.com", "operation": "calls", "on": false})).expect("calls");
    assert_eq!(r["accounts"][0]["calls"], false);
    assert_eq!(b.reports.borrow().last().expect("report")["accounts"][0]["calls"], false);
    assert!(
        matches!(post(&owner, json!({"account": "nobody@example.com", "operation": "calls", "on": true})), Err(Error::Http(409, d, _)) if d == NO_CALLS_SWITCH)
    );

    // a sign-in cancelled: the directory refuses with its reason
    b.set_answers.borrow_mut().push(Some(json!({"ok": false, "errore": "Not running"})));
    assert!(
        matches!(post(&owner, json!({"account": "mario@example.com", "operation": "signin-cancel", "id": "job"})), Err(Error::Http(409, d, _)) if d == "Not running")
    );
}
