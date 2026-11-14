use super::stats::*;
use super::*;

const H: i64 = 3600;
const DAY: i64 = 86400;
const GAP: i64 = 180;

fn rome() -> jiff::tz::TimeZone {
    zone("Europe/Rome")
}

/// 2026-11-03 00:00 in Rome
fn d() -> i64 {
    "2026-11-02T23:00:00Z".parse::<jiff::Timestamp>().expect("ts").as_second()
}

fn at(h: i64, m: i64) -> i64 {
    d() + h * H + m * 60
}

fn st(a: i64, b: i64, s: &str) -> Stretch {
    (a, b, s.to_string())
}

/// A day of the past (now two days after it), with its stretches.
fn past(s: &[Stretch], day: &str) -> Value {
    day_stats(s, day, &rome(), at(48, 0), GAP, true)
}

fn hours(v: &Value, k: &str) -> f64 {
    v["hours"][k].as_f64().expect("hours")
}

fn sum(v: &Value) -> f64 {
    v["hours"].as_object().expect("hours").values().map(|x| x.as_f64().expect("number")).sum()
}

#[test]
fn days_start_at_midnight_in_the_zone_and_have_23_or_25_hours_on_a_dst_change() {
    let z = rome();
    assert_eq!(day_start("2026-11-03", &z), d());
    assert_eq!(day_of(d(), &z), "2026-11-03");
    assert_eq!(day_of(d() - 1, &z), "2026-11-02");
    assert_eq!(next_day("2026-12-31"), "2027-01-01");
    // Date.UTC carries a month or day out of range over
    assert_eq!(next_day("2026-13-45"), "2027-02-15");
    assert_eq!(hour_bounds("2026-11-03", &z).len(), 25);
    assert_eq!(hour_bounds("2026-03-29", &z).len(), 24);
    assert_eq!(hour_bounds("2026-10-25", &z).len(), 26);
    assert_eq!(clock(at(9, 5), &z), "09:05");
}

#[test]
fn stretches_hold_until_the_next_within_the_gap_and_run_to_now_while_fresh() {
    let s = [st(at(9, 0), at(10, 0), "busy"), st(at(10, 1), at(11, 0), "available"), st(at(12, 0), at(13, 0), "away")];
    assert_eq!(join_stretches(&s, GAP, at(13, 2)), [st(at(9, 0), at(10, 1), "busy"), st(at(10, 1), at(11, 0), "available"), st(at(12, 0), at(13, 2), "away")]);
    assert_eq!(join_stretches(&s, GAP, at(14, 0)), [st(at(9, 0), at(10, 1), "busy"), st(at(10, 1), at(11, 0), "available"), st(at(12, 0), at(13, 0), "away")]);
}

#[test]
fn the_stats_of_a_day() {
    let z = rome();
    // a past day adds up to its length, no data included, unknown counted as no data
    let v = past(&[st(at(8, 0), at(12, 0), "available"), st(at(12, 1), at(14, 0), "unknown"), st(at(14, 1), at(18, 0), "busy")], "2026-11-03");
    assert!((sum(&v) - 24.0).abs() < 0.05);
    assert_eq!((hours(&v, "available"), hours(&v, "busy"), hours(&v, "offline")), (4.02, 3.98, 0.0));
    assert!((hours(&v, "nodata") - 16.0).abs() < 0.05);
    assert_eq!(v["hours"].as_object().expect("hours").keys().collect::<Vec<_>>(), ["available", "busy", "dnd", "brb", "away", "offline", "nodata"]);

    // today adds up to the time elapsed so far
    let v = day_stats(&[st(at(8, 0), at(10, 0), "available")], "2026-11-03", &z, at(10, 30), GAP, true);
    assert!((sum(&v) - 10.5).abs() < 0.05);
    assert_eq!(hours(&v, "available"), 2.0);
    assert_eq!(v["byHour"].as_array().expect("by hour").len(), 24);

    // a stretch across midnight, clipped to each day
    let s = [st(at(-1, 0), at(1, 0), "dnd")];
    assert_eq!(hours(&past(&s, "2026-11-03"), "dnd"), 1.0);
    assert_eq!(hours(&past(&s, "2026-11-02"), "dnd"), 1.0);
    assert_eq!(past(&s, "2026-11-03")["stretches"], json!([[at(0, 0), at(1, 0), "dnd"]]));

    // the longest Available streak breaks at a gap, not at a change of poll
    let s = [st(at(9, 0), at(10, 0), "available"), st(at(10, 1), at(11, 0), "available"), st(at(11, 30), at(13, 0), "available")];
    assert_eq!(past(&s, "2026-11-03")["longestAvailable"], 120);

    // first and last online from statuses other than offline and unknown
    let s = [
        st(at(6, 0), at(8, 47), "offline"),
        st(at(8, 47), at(9, 0), "away"),
        st(at(9, 0), at(18, 12), "busy"),
        st(at(18, 12), at(23, 0), "offline"),
        st(at(23, 0), at(23, 30), "unknown"),
    ];
    let v = past(&s, "2026-11-03");
    assert_eq!((v["firstOnline"].as_str(), v["lastOnline"].as_str()), (Some("08:47"), Some("18:12")));
    let v = past(&[st(at(6, 0), at(8, 0), "offline")], "2026-11-03");
    assert_eq!((&v["firstOnline"], &v["lastOnline"], &v["green"], &v["longestAvailable"]), (&json!(""), &json!(""), &Value::Null, &json!(0)));

    // the green share: available over online hours
    let v = past(&[st(at(9, 0), at(12, 0), "available"), st(at(12, 0), at(13, 0), "busy"), st(at(13, 0), at(20, 0), "offline")], "2026-11-03");
    assert_eq!(v["green"], 0.75);
    assert_eq!(past(&[st(at(9, 0), at(12, 0), "away")], "2026-11-03")["green"], 0);

    // by hour: the status that held most of it, no data where the gap holds most
    let v = past(&[st(at(9, 0), at(9, 40), "available"), st(at(9, 40), at(10, 10), "busy")], "2026-11-03");
    assert_eq!(v["byHour"].as_array().expect("by hour")[8..11], [json!("nodata"), json!("available"), json!("nodata")]);

    // the DST days: 23 and 25 hours, by hour too
    let spring = day_start("2026-03-29", &z);
    let autumn = day_start("2026-10-25", &z);
    let a = day_stats(&[st(spring, spring + 23 * H, "available")], "2026-03-29", &z, at(0, 0), GAP, true);
    let b = day_stats(&[st(autumn, autumn + 25 * H, "offline")], "2026-10-25", &z, at(0, 0), GAP, true);
    assert_eq!((a["byHour"].as_array().expect("by hour").len(), sum(&a), hours(&a, "available")), (23, 23.0, 23.0));
    assert_eq!((b["byHour"].as_array().expect("by hour").len(), sum(&b), hours(&b, "offline")), (25, 25.0, 25.0));

    // 24:00 for a last online at the end of the day; no stretches when the timeline does not draw them
    let v = day_stats(&[st(at(20, 0), at(26, 0), "busy")], "2026-11-03", &z, at(48, 0), GAP, false);
    assert_eq!(v["lastOnline"], "24:00");
    assert!(v.get("stretches").is_none());
}

fn set_env(k: &str, v: &str) {
    // SAFETY: the tests that set the environment hold TEST_ENV
    unsafe { std::env::set_var(k, v) };
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().build().expect("runtime").block_on(f)
}

fn request(method: &str, slug: &str) -> Req {
    Req {
        method: method.into(),
        path: format!("/api/status/{slug}"),
        query: String::new(),
        headers: hyper::header::HeaderMap::new(),
        body: bytes::Bytes::new(),
        params: vec![("slug".into(), slug.into())],
        stream_id: None,
    }
}

fn stretch(name: &str, since: i64, until: i64, status: &str) {
    with_status_db(|c| {
        c.execute("INSERT INTO presence_log(name, mri, since, until, status) VALUES(?, '8:orgid:x', ?, ?, ?)", rusqlite::params![name, since, until, status])
    })
    .expect("stretch");
}

fn page(slug: &str, name: &str, added: i64, ends: i64) {
    add_page(&StatusPage { slug: slug.into(), slot: 4, name: name.into(), added, ends }).expect("page");
}

fn detail(r: Res<impl std::fmt::Debug>) -> (u16, String) {
    match r {
        Err(Error::Http(s, d, _)) => (s, d),
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

fn body(v: Value) -> Map<String, Value> {
    v.as_object().cloned().expect("object")
}

/// The pages, their answers and the admin card, on an app.db, a status.db and slot databases of a temp folder.
#[test]
fn pages_answers_and_the_admin_card() {
    let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let dir = std::env::temp_dir().join(format!("trweb-status-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ui = dir.join("ui");
    std::fs::create_dir_all(ui.join("static")).expect("ui");
    for f in ["index.html", "login.html", "status.html", "sw.js", "manifest.webmanifest"] {
        std::fs::write(ui.join(f), "").expect("ui file");
    }
    set_env("APP_DB", dir.join("app.db").to_str().expect("path"));
    set_env("TZ", "Europe/Rome");
    set_env("UI_DIR", ui.to_str().expect("path"));
    appdb::with(appdb::migrate).expect("schema");
    let z = rome();

    // the answer of a page: the status now, the days from the one it was added, stretches for today and yesterday
    let d = day_start("2026-11-04", &z);
    let now = d + 10 * H;
    page("kamila", "Camilla Marini", d - 2 * DAY + 9 * H, 0);
    stretch("Camilla Marini", d - 2 * DAY + 9 * H, d - 2 * DAY + 12 * H, "available");
    stretch("Camilla Marini", d + 8 * H, d + 10 * H - 60, "busy");
    let a = status_answer("kamila", now).expect("answer").expect("page");
    assert_eq!(body(a.clone()).keys().collect::<Vec<_>>(), ["name", "now", "days", "ends", "updated", "zone"]);
    assert_eq!(
        (&a["name"], &a["now"], &a["ends"], &a["updated"], &a["zone"]),
        (&json!("Camilla Marini"), &json!({"status": "busy", "since": d + 8 * H, "fresh": true}), &json!(0), &json!(d + 10 * H - 60), &json!("Europe/Rome"))
    );
    let days = a["days"].as_array().expect("days");
    assert_eq!(days.iter().map(|x| x["day"].as_str().expect("day")).collect::<Vec<_>>(), ["2026-11-02", "2026-11-03", "2026-11-04"]);
    assert_eq!(days[0]["hours"]["available"], 3);
    assert!(days[0].get("stretches").is_none());
    assert_eq!(days[1]["stretches"], json!([]));
    assert_eq!(days[2]["stretches"], json!([[d + 8 * H, now, "busy"]]));
    assert_eq!(days[2]["byHour"].as_array().expect("by hour").len(), 24);
    assert_eq!(
        body(days[2].clone()).keys().collect::<Vec<_>>(),
        ["day", "hours", "firstOnline", "lastOnline", "longestAvailable", "green", "byHour", "stretches"]
    );

    // a status no longer fresh
    page("damiano", "Damiano Bogni", d, 0);
    stretch("Damiano Bogni", d + 9 * H, d + 9 * H, "available");
    assert_eq!(status_answer("damiano", now).expect("answer").expect("page")["now"], json!({"status": "available", "since": d + 9 * H, "fresh": false}));
    assert!(delete_page("damiano").expect("deleted"));

    // gone with its data 7 days after the end of its challenge
    page("ended", "Ended Person", d - 20 * DAY, d - 7 * DAY);
    page("recent", "Recent Person", d - 20 * DAY, d - 2 * DAY);
    stretch("Ended Person", d - 10 * DAY, d - 10 * DAY + H, "available");
    stretch("Recent Person", d - 10 * DAY, d - 10 * DAY + H, "available");
    assert!(status_answer("ended", now).expect("answer").is_none());
    assert!(!list_pages().expect("pages").iter().any(|p| p.slug == "ended"));
    assert!(with_status_db(|c| stretches_of(c, "Ended Person", 0, now)).expect("read").is_empty());
    let recent = status_answer("recent", now).expect("answer").expect("page");
    assert_eq!(recent["days"].as_array().expect("days").last().expect("day")["day"], "2026-11-01");
    assert_eq!(recent["now"]["fresh"], false);
    assert!(delete_page("recent").expect("deleted"));

    // the public route: no session, 404 for a page that is not there, cached 30 s
    for slug in ["nobody", "Bad Slug!"] {
        assert_eq!(detail(run(public_get(request("GET", slug)))), (404, "Page not found".into()));
    }
    let r = run(public_get(request("GET", "kamila"))).expect("answer");
    assert_eq!((r.status().as_u16(), r.headers().get("cache-control").and_then(|v| v.to_str().ok())), (200, Some("public, max-age=30")));
    assert!(page_answer("Bad Slug!").expect("answer").is_none());

    // the admin card is for administrators
    let admin_routes: Vec<_> = routes().into_iter().filter(|r| r.pattern == "/api/admin/status-pages").collect();
    assert_eq!(admin_routes.iter().map(|r| r.method).collect::<Vec<_>>(), ["GET", "POST", "PATCH", "DELETE"]);
    for r in admin_routes {
        assert_eq!(detail(run((r.handler)(request(r.method, "")))).0, 401, "{}", r.method);
    }
    // 2026-00-00 is the day before 2026-01-01, as Date.UTC has it
    assert_eq!(next_day("2026-00-00"), "2025-12-01");

    // the slugs of the app: the exported UI, with and without extension, and the paths the server owns
    let reserved = reserved_slugs();
    for s in [
        "admin",
        "api",
        "consent",
        "files",
        "healthz",
        "login",
        "login.html",
        "mcp",
        "media",
        "remote",
        "settings",
        "static",
        "sw.js",
        "manifest.webmanifest",
        "desktop",
        "status",
        "index",
    ] {
        assert!(reserved.contains(s), "{s}");
    }

    // accounts of the administrator: one light with an identity, one always on
    let light = appdb::claim_slot("admin", Some(9)).expect("slot");
    appdb::set_light(light, true).expect("light");
    let always = appdb::claim_slot("admin", Some(9)).expect("slot");
    let slot_db = crate::slotdb::open_store(&crate::slotdb::slot_db_path(light)).expect("slot db");
    slot_db.execute("INSERT INTO state(k, v) VALUES('me', ?)", [r#"{"name":"Owner Agap","email":"","tenant":"AGAP2","av":"","logo":""}"#]).expect("me");
    let names = |v: &str| {
        slot_db.execute("INSERT OR REPLACE INTO state(k, v) VALUES('status_names', ?)", [v]).expect("names");
    };
    let add = |b: Value| admin_add("admin", &body(b), now);

    // reserved and taken paths, a bad path, an account that is not light, no name
    assert_eq!(detail(add(json!({"slug": "admin", "slot": light, "name": "Camilla Marini"}))), (409, "/admin is a page of TeamsRelay".into()));
    assert_eq!(detail(add(json!({"slug": "desktop", "slot": light, "name": "Camilla Marini"}))).0, 409);
    assert_eq!(detail(add(json!({"slug": "Kamila!", "slot": light, "name": "Camilla Marini"}))), (400, "Path: 2 to 32 lowercase letters, digits or -".into()));
    assert_eq!(
        detail(add(json!({"slug": "camilla", "slot": always, "name": "Camilla Marini"}))),
        (400, format!("Account {always} is not a light account of yours"))
    );
    assert_eq!(detail(add(json!({"slug": "camilla", "name": "Camilla Marini"}))), (400, "Account undefined is not a light account of yours".into()));
    assert_eq!(detail(add(json!({"slug": "camilla", "slot": light, "name": " "}))), (400, "Missing name".into()));
    assert_eq!(detail(add(json!({"slug": "kamila", "slot": light, "name": "Camilla Marini"}))), (409, "/kamila is already a status page".into()));
    add(json!({"slug": "damian", "slot": light, "name": "Damiano Bogni"})).expect("added");

    // the list: whether each page collects and why not, the last status seen, the accounts to pick from
    let damian = || {
        let list = admin_list("admin", now).expect("list");
        list["pages"].as_array().expect("pages").iter().find(|p| p["slug"] == "damian").cloned().expect("damian")
    };
    let list = admin_list("admin", now).expect("list");
    assert_eq!(
        list["accounts"],
        json!([{"slot": light, "label": "Owner Agap (AGAP2)", "light": true}, {"slot": always, "label": format!("Account {always}"), "light": false}])
    );
    assert_eq!(list["zone"], "Europe/Rome");
    let p = damian();
    assert_eq!(body(p.clone()).keys().collect::<Vec<_>>(), ["slug", "slot", "name", "added", "ends", "tenant", "collecting", "reason", "last"]);
    assert_eq!((&p["tenant"], &p["collecting"], &p["reason"], &p["last"]), (&json!("AGAP2"), &json!(false), &json!("waiting for the agent"), &Value::Null));
    // the page of a slot that is gone
    assert_eq!(list["pages"].as_array().expect("pages").iter().find(|p| p["slug"] == "kamila").expect("kamila")["reason"], "account removed");
    names(r#"{"Damiano Bogni":""}"#);
    assert_eq!(damian()["reason"], "not found in AGAP2");
    names(r#"{"Damiano Bogni":"8:orgid:dam","Other":5}"#);
    assert_eq!(damian()["reason"], "waiting for the agent");
    names(r#"{"Damiano Bogni":"8:orgid:dam"}"#);
    stretch("Damiano Bogni", 1_793_700_000, 1_793_700_000, "busy");
    let p = damian();
    assert_eq!(
        (&p["collecting"], &p["reason"], &p["last"]),
        (&json!(true), &json!(""), &json!({"since": 1_793_700_000, "until": 1_793_700_000, "status": "busy"}))
    );
    appdb::set_light(light, false).expect("always on");
    assert_eq!(damian()["reason"], "account not light");
    appdb::set_light(light, true).expect("light");

    // the end of the challenge: the midnight after the day given, or none
    let end = |b: Value| admin_set_end(&body(b));
    assert_eq!(end(json!({"slug": "kamila", "ends": "2026-11-30"})).expect("set"), day_start("2026-12-01", &z));
    assert_eq!(page_of("kamila").expect("page").expect("kamila").ends, day_start("2026-12-01", &z));
    assert_eq!(detail(end(json!({"slug": "kamila", "ends": "30/11/2026"}))), (400, "End date: YYYY-MM-DD".into()));
    assert_eq!(detail(end(json!({"slug": "nobody", "ends": ""}))), (404, "Page not found".into()));
    assert_eq!(end(json!({"slug": "kamila", "ends": ""})).expect("cleared"), 0);
    assert_eq!(page_of("kamila").expect("page").expect("kamila").ends, 0);

    // a delete takes the data with it, unless another page tracks the same person
    page("damian-2", "Damiano Bogni", d, 0);
    assert!(delete_page("damian").expect("deleted"));
    assert_eq!(with_status_db(|c| stretches_of(c, "Damiano Bogni", 0, 2_000_000_000)).expect("read").len(), 1);
    assert!(delete_page("damian-2").expect("deleted"));
    assert!(with_status_db(|c| stretches_of(c, "Damiano Bogni", 0, 2_000_000_000)).expect("read").is_empty());
    assert_eq!(list_pages().expect("pages").iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(), ["kamila"]);
    assert!(!delete_page("damian").expect("none"));

    drop(slot_db);
    let _ = std::fs::remove_dir_all(&dir);
}
