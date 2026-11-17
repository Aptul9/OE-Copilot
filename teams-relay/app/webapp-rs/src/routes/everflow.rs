//! /api/everflow/saved: saved Teams messages of the administrator of .env since a time, for Everflow through the
//! communication service (src/app/api/everflow/saved/route.ts, src/lib/everflow-saved.ts). Bearer MCP_TOKEN only,
//! like /mcp: no session cookie, no web page (Origin).

use crate::route;
use crate::server::Route;
use crate::web::{Error, Req, Res, http, json};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

pub fn routes() -> Vec<Route> {
    vec![route!("GET", "/api/everflow/saved", get)]
}

/// The query Everflow ran on each slot store over SSH (storeQuery in everflow src/watchers/relay.ts): one JSON text with
/// the account (state me, last_scan_ts, state), the muted chats and the saved messages with a numeric id at or after
/// `since`, oldest first, text cut at 2000 characters. One row more than Everflow's cap, so Everflow sees it was hit.
const STORE_QUERY: &str = "SELECT json_object('state',json((SELECT json_group_object(k,v) FROM state WHERE k IN ('me','last_scan_ts','state'))),'muted',json((SELECT json_group_array(name) FROM chats WHERE muted=1)),'messages',json((SELECT json_group_array(json_object('chat',chat,'mid',mid,'author',author,'text',text,'mine',mine,'extra',extra)) FROM (SELECT chat,mid,author,substr(text,1,2000) AS text,mine,extra FROM chat_messages WHERE mid GLOB '[0-9]*' AND mid NOT GLOB '*[^0-9]*' AND CAST(mid AS INTEGER)>=?1 ORDER BY CAST(mid AS INTEGER),chat LIMIT ?2)))) AS snapshot";

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

fn integer(v: Option<String>, min: i64, max: i64) -> Option<i64> {
    let v = v?;
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse::<i64>().ok().filter(|n| (min..=max).contains(n))
}

/// ?since=<ms>&max_rows=<n>: {"slots": [{"slot", "snapshot"} | {"slot", "error"}]}, active slots in slot order.
async fn get(req: Req) -> Res {
    if crate::config::mcp_token().is_empty() {
        return http(404, "Not found");
    }
    if req.headers.contains_key("origin") {
        return http(403, "Requests from web pages are not accepted");
    }
    if !crate::mcp::token_matches(req.header("authorization").as_deref()) {
        return Err(Error::Http(401, "Missing or wrong token".into(), vec![("WWW-Authenticate".into(), "Bearer realm=\"teamsrelay\"".into())]));
    }
    let since = integer(req.query_param("since"), 0, MAX_SAFE_INTEGER);
    let max_rows = integer(req.query_param("max_rows"), 1, 100_000);
    let (Some(since), Some(max_rows)) = (since, max_rows) else { return http(400, "since (ms) and max_rows (1 to 100000) are required") };
    let Some(user) = crate::mcp::mcp_user_id()? else { return http(503, "The administrator of .env does not exist") };
    // the stores are read on a blocking thread: the server is one thread, and a store an agent holds locked waits
    let saved = tokio::task::spawn_blocking(move || saved_since(&user, since, max_rows)).await.map_err(|e| Error::Internal(format!("everflow saved: {e}")))?;
    json(saved?)
}

/// Saved messages of every active Teams account of a user; a store that cannot be read is that slot's error, the
/// others are still read.
pub fn saved_since(user: &str, since: i64, max_rows: i64) -> Res<Value> {
    let ids: Vec<i64> = crate::appdb::with(|c| {
        let mut s = c.prepare("SELECT slot FROM teams_accounts WHERE owner_id=? AND stopped=0 ORDER BY slot LIMIT 1025")?;
        s.query_map([user], |r| r.get(0))?.collect()
    })?;
    let slots: Vec<Value> = ids
        .into_iter()
        .map(|slot| match read_store(slot, since, max_rows + 1) {
            Some(snapshot) => json!({ "slot": slot, "snapshot": snapshot }),
            None => json!({ "slot": slot, "error": "store-unavailable" }),
        })
        .collect();
    Ok(json!({ "slots": slots }))
}

fn read_store(slot: i64, since: i64, limit: i64) -> Option<String> {
    let file = crate::slotdb::slot_db_path(slot);
    if !file.exists() {
        return None;
    }
    let c = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    // short: every active slot is read in turn within the 60 s the communication service waits
    c.busy_timeout(std::time::Duration::from_millis(STORE_BUSY_MS)).ok()?;
    c.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;").ok()?;
    // bytes that are not UTF-8 become U+FFFD, as the sqlite3 output read over SSH was decoded, rather than the whole slot failing
    c.query_row(STORE_QUERY, [since, limit], |r| Ok(String::from_utf8_lossy(r.get_ref(0)?.as_bytes()?).into_owned())).ok()
}

const STORE_BUSY_MS: u64 = 2000;

#[cfg(test)]
mod tests {
    //! The cases of test/everflow-saved.test.ts.

    use crate::slotdb::{open_store, slot_db_path};
    use crate::web::{Req, error_response};
    use bytes::Bytes;
    use http_body_util::BodyExt;
    use rusqlite::params;
    use serde_json::{Value, json};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn set_env(k: &str, v: &str) {
        // SAFETY: the tests that set the environment hold TEST_ENV
        unsafe { std::env::set_var(k, v) };
    }

    fn store(slot: i64, me: &str) {
        let db = open_store(&slot_db_path(slot)).unwrap();
        let state = "INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)";
        db.execute(state, params!["me", json!({"name": "ROSSI Anna", "email": me}).to_string()]).unwrap();
        db.execute(state, params!["last_scan_ts", "1790431900"]).unwrap();
        db.execute(state, params!["state", "ok"]).unwrap();
        db.execute(state, params!["health", "{}"]).unwrap();
        let chat = "INSERT INTO chats(name, preview, pos, tm, unread, mention, muted) VALUES(?, '', ?, '', 0, 0, ?)";
        db.execute(chat, params!["BIANCHI Luca", 0, 0]).unwrap();
        db.execute(chat, params!["Noisy group", 1, 1]).unwrap();
        let msg = "INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES(?, ?, ?, ?, ?, ?, '', ?)";
        db.execute(msg, params!["BIANCHI Luca", 0, "1790432000000", "BIANCHI Luca", "x".repeat(2001), 0, r#"{"mentionsMe":true}"#]).unwrap();
        db.execute(msg, params!["BIANCHI Luca", 1, "1790431000000", "ROSSI Anna", "Earlier", 1, ""]).unwrap();
        db.execute(msg, params!["BIANCHI Luca", 2, "abc", "BIANCHI Luca", "Not saved yet", 0, ""]).unwrap();
    }

    async fn get(query: &str, headers: &[(&str, &str)]) -> (u16, Value) {
        let mut h = hyper::header::HeaderMap::new();
        for (k, v) in headers {
            h.append(hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), hyper::header::HeaderValue::from_str(v).unwrap());
        }
        let req = Req {
            method: "GET".into(),
            path: "/api/everflow/saved".into(),
            query: query.into(),
            headers: h,
            body: Bytes::new(),
            params: vec![],
            stream_id: None,
        };
        let r = super::get(req).await.unwrap_or_else(error_response);
        let status = r.status().as_u16();
        let body = r.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn everflow_saved() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("trweb-everflow-{}-{}", std::process::id(), crate::web::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        set_env("APP_DB", dir.join("app.db").to_str().unwrap());
        set_env("ADMIN_EMAIL", "admin@teamsrelay.test");
        set_env("MCP_TOKEN", TOKEN);
        crate::appdb::with(crate::appdb::migrate).unwrap();
        crate::appdb::with(|c| c.execute_batch(r#"CREATE TABLE IF NOT EXISTS "user"(id TEXT PRIMARY KEY, email TEXT NOT NULL); INSERT INTO "user"(id, email) VALUES('admin-id', 'admin@teamsrelay.test')"#)).unwrap();
        let first = crate::appdb::claim_slot("admin-id", None).unwrap();
        let other = crate::appdb::claim_slot("someone-else", None).unwrap();
        let second = crate::appdb::claim_slot("admin-id", None).unwrap();
        let stopped = crate::appdb::claim_slot("admin-id", None).unwrap();
        crate::appdb::set_slot_stopped(stopped, true).unwrap();
        store(first, "anna@contoso.example");
        store(second, "anna@fabrikam.example");
        store(stopped, "anna@stopped.example");
        store(other, "luca@other.example");
        let auth = format!("Bearer {TOKEN}");
        let ok: &[(&str, &str)] = &[("authorization", &auth)];

        // refuses without the token
        for headers in [&[][..], &[("authorization", "Bearer wrong")][..], &[("authorization", TOKEN)][..], &[("cookie", "better-auth.session_token=x")][..]] {
            assert_eq!(get("since=0&max_rows=10", headers).await.0, 401, "{headers:?}");
        }
        assert_eq!(get("since=0&max_rows=10", &[("authorization", &auth), ("origin", "https://x")]).await.0, 403);

        // rejects bad parameters
        for q in
            ["since=-1&max_rows=10", "since=x&max_rows=10", "since=0&max_rows=0", "since=0&max_rows=100001", "max_rows=10", "since=1.5&max_rows=10", "since=0"]
        {
            assert_eq!(get(q, ok).await.0, 400, "{q}");
        }

        // returns the admin's active slots only, in slot order
        let (status, all) = get("since=0&max_rows=10", ok).await;
        assert_eq!(status, 200);
        let slots: Vec<i64> = all["slots"].as_array().unwrap().iter().map(|s| s["slot"].as_i64().unwrap()).collect();
        assert_eq!(slots, vec![first, second]);
        let saved: Value = serde_json::from_str(all["slots"][0]["snapshot"].as_str().unwrap()).unwrap();
        let mids: Vec<&str> = saved["messages"].as_array().unwrap().iter().map(|m| m["mid"].as_str().unwrap()).collect();
        assert_eq!(mids, vec!["1790431000000", "1790432000000"]);

        // snapshot matches the Everflow store query
        let (_, since) = get("since=1790431500000&max_rows=10", ok).await;
        let saved: Value = serde_json::from_str(since["slots"][0]["snapshot"].as_str().unwrap()).unwrap();
        let me: Value = serde_json::from_str(saved["state"]["me"].as_str().unwrap()).unwrap();
        assert_eq!(me["email"], "anna@contoso.example");
        assert_eq!(saved["state"]["last_scan_ts"], "1790431900");
        assert_eq!(saved["state"]["state"], "ok");
        assert!(saved["state"].get("health").is_none());
        assert_eq!(saved["muted"], json!(["Noisy group"]));
        assert_eq!(
            saved["messages"],
            json!([{"chat": "BIANCHI Luca", "mid": "1790432000000", "author": "BIANCHI Luca", "text": "x".repeat(2000), "mine": 0, "extra": r#"{"mentionsMe":true}"#}])
        );

        // max_rows + 1 rows
        let (_, capped) = get("since=0&max_rows=1", ok).await;
        let saved: Value = serde_json::from_str(capped["slots"][0]["snapshot"].as_str().unwrap()).unwrap();
        assert_eq!(saved["messages"].as_array().unwrap().len(), 2);

        // a WAL store with a writer holding an open transaction still reads its committed rows
        let writer = rusqlite::Connection::open(slot_db_path(first)).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        writer
            .execute("INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES('BIANCHI Luca', 9, '1790433000000', 'BIANCHI Luca', 'uncommitted', 0, '', '')", [])
            .unwrap();
        let (status, open) = get("since=0&max_rows=10", ok).await;
        assert_eq!(status, 200);
        let saved: Value = serde_json::from_str(open["slots"][0]["snapshot"].as_str().unwrap()).unwrap();
        assert_eq!(saved["messages"].as_array().unwrap().len(), 2);
        writer.execute_batch("ROLLBACK").unwrap();
        drop(writer);

        // text that is not UTF-8 is replaced, not the whole slot lost
        let db = rusqlite::Connection::open(slot_db_path(second)).unwrap();
        db.execute("INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES('BIANCHI Luca', 9, '1790433000000', 'BIANCHI Luca', CAST(x'ff41' AS TEXT), 0, '', '')", [])
            .unwrap();
        drop(db);
        let (_, bad) = get("since=1790433000000&max_rows=10", ok).await;
        let saved: Value = serde_json::from_str(bad["slots"][1]["snapshot"].as_str().expect("second slot keeps its snapshot")).unwrap();
        assert_eq!(saved["messages"][0]["text"], "\u{FFFD}A");

        // a store locked by its agent is that slot's error within a short wait, the others are still read
        let locker = rusqlite::Connection::open(slot_db_path(first)).unwrap();
        locker.execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; UPDATE state SET v=v WHERE k='state';").unwrap();
        let started = std::time::Instant::now();
        let (status, locked) = get("since=0&max_rows=10", ok).await;
        assert_eq!(status, 200);
        assert!(started.elapsed() < std::time::Duration::from_secs(4), "{:?}", started.elapsed());
        assert_eq!(locked["slots"][0], json!({"slot": first, "error": "store-unavailable"}));
        assert!(locked["slots"][1]["snapshot"].is_string());
        locker.execute_batch("ROLLBACK").unwrap();
        drop(locker);

        // missing store is that slot's error
        std::fs::remove_file(slot_db_path(second)).unwrap();
        let (status, missing) = get("since=0&max_rows=10", ok).await;
        assert_eq!(status, 200);
        assert!(missing["slots"][0]["snapshot"].is_string());
        assert_eq!(missing["slots"][1], json!({"slot": second, "error": "store-unavailable"}));

        // off without MCP_TOKEN
        set_env("MCP_TOKEN", "");
        assert_eq!(get("since=0&max_rows=10", ok).await.0, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
