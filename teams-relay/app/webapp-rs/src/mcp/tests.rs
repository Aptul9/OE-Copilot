//! /mcp end to end on a temporary app.db and slot database: the answers of test/mcp-route.test.ts,
//! test/mcp-tools.test.ts and test/mcp-access.test.ts, and the wire answers recorded from the TypeScript server. One
//! test: the cases share the process environment (APP_DB, MCP_TOKEN, ADMIN_EMAIL).

use super::tools::{self, Wait, error_text};
use crate::slotdb::{open_store, slot_db_path, slot_dir};
use crate::web::{Req, Resp, error_response, now_s};
use bytes::Bytes;
use http_body_util::BodyExt;
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FAST: Wait = Wait { timeout_ms: 1000, poll_ms: 20 };

fn set_env(k: &str, v: &str) {
    // SAFETY: the tests that set the environment hold TEST_ENV
    unsafe { std::env::set_var(k, v) };
}

fn request(method: &str, headers: &[(&str, &str)], body: &str) -> Req {
    let mut h = hyper::header::HeaderMap::new();
    for (k, v) in headers {
        h.append(hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), hyper::header::HeaderValue::from_str(v).unwrap());
    }
    Req { method: method.into(), path: "/mcp".into(), query: String::new(), headers: h, body: Bytes::from(body.to_string()), params: vec![], stream_id: None }
}

async fn text_of(r: Resp) -> (u16, String) {
    let status = r.status().as_u16();
    let body = r.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn post_with(headers: &[(&str, &str)], body: &str) -> Resp {
    super::post(request("POST", headers, body)).await.unwrap_or_else(error_response)
}

const ACCEPT: (&str, &str) = ("accept", "application/json, text/event-stream");
const JSON: (&str, &str) = ("content-type", "application/json");

/// A JSON-RPC message posted with the token, the status and the body.
async fn mcp(message: Value) -> (u16, String) {
    let auth = format!("Bearer {TOKEN}");
    let r = post_with(&[("authorization", &auth), ACCEPT, JSON, ("mcp-protocol-version", "2025-06-18")], &message.to_string()).await;
    text_of(r).await
}

/// The one answer of an SSE body.
fn event(body: &str) -> Value {
    let data = body.strip_prefix("event: message\ndata: ").and_then(|b| b.strip_suffix("\n\n")).unwrap_or_else(|| panic!("not one event: {body}"));
    serde_json::from_str(data).unwrap()
}

async fn call(name: &str, args: Value) -> Value {
    let (status, body) = mcp(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}})).await;
    assert_eq!(status, 200, "{body}");
    event(&body)["result"].clone()
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn fail_text<T: std::fmt::Debug>(r: crate::web::Res<T>) -> String {
    error_text(r.unwrap_err())
}

struct Env {
    mine: i64,
    other: i64,
    empty: i64,
    db: Connection,
}

impl Env {
    fn state(&self, k: &str, v: &str) {
        self.db.execute("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)", params![k, v]).unwrap();
    }

    fn health(&self, teams: &str) {
        self.state("health", &json!({"cdp": "ok", "ts": now_s(), "teams": teams, "overall": "green"}).to_string());
    }

    /// The rows of beforeEach of test/mcp-tools.test.ts.
    fn reset(&self) {
        crate::appdb::set_slot_stopped(self.mine, false).unwrap();
        self.db.execute_batch("DELETE FROM chats; DELETE FROM chat_messages; DELETE FROM activity; DELETE FROM commands; DELETE FROM state").unwrap();
        self.health("ok");
        self.state("me", r#"{"name":"ROSSI Anna","email":"anna@contoso.example","tenant":"Contoso","av":""}"#);
        let chat = "INSERT INTO chats(name, preview, pos, tm, unread, mention, muted) VALUES(?,?,?,?,?,?,?)";
        self.db.execute(chat, params!["BIANCHI Luca", "See you at 3", 0, "14:07", 1, 0, 0]).unwrap();
        self.db.execute(chat, params!["Cloud team", "Deploy done", 1, "Yesterday", 0, 1, 0]).unwrap();
        self.db.execute(chat, params!["ROSSI Anna (You)", "note", 2, "9/22", 0, 0, 1]).unwrap();
        let msg = "INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES(?,?,?,?,?,?,?,?)";
        let extra = json!({"reactions": [{"e": "like", "n": 2, "mine": true}], "images": [{"f": "a.webp", "w": 10, "h": 10}], "mentionsMe": true});
        self.db.execute(msg, params!["BIANCHI Luca", 0, "1790431027228", "BIANCHI Luca", "Can you check the pipeline?", 0, "", extra.to_string()]).unwrap();
        let extra = json!({"quote": {"author": "BIANCHI Luca", "text": "Can you check the pipeline?"}, "files": [{"name": "report.xlsx", "url": "https://contoso.sharepoint.com/x"}], "edited": true});
        self.db.execute(msg, params!["BIANCHI Luca", 1, "1790431664072", "ROSSI Anna", "Done", 1, "", extra.to_string()]).unwrap();
        self.db.execute(msg, params!["BIANCHI Luca", 2, "temp-1", "BIANCHI Luca", "", 0, "", r#"{"deleted":true}"#]).unwrap();
        let act = "INSERT INTO activity(id, pos, kind, actor, title, emoji, preview, tm, chat, unread, channel) VALUES(?,?,?,?,?,?,?,?,?,?,?)";
        self.db.execute(act, params!["a1", 0, "mention", "BIANCHI Luca", "mentioned you", "", "@Anna see this", "14:00", "Cloud team", 1, 0]).unwrap();
        self.db
            .execute(act, params!["a2", 1, "reaction", "VERDI Carla", "reacted to your message", "heart", "Done", "Yesterday", "BIANCHI Luca", 0, 0])
            .unwrap();
        self.state("activity_ts", "1790431000");
    }

    fn queued(&self) -> Vec<Value> {
        crate::slotdb::rows(&self.db, "SELECT type, arg1, arg2 FROM commands ORDER BY id", []).unwrap().into_iter().map(Value::Object).collect()
    }
}

/// The agent of the account: runs every pending command with this outcome; a message sent becomes a message of the
/// account in the chat, a download writes its file, an open (when it opens) makes the chat the active one with a new
/// message.
fn agent(slot: i64, outcome: &'static str) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let db = Connection::open(slot_db_path(slot)).unwrap();
        loop {
            let cmds = crate::slotdb::rows(&db, "SELECT id, type, arg1 FROM commands WHERE status='pending'", []).unwrap();
            for c in cmds {
                let id = c["id"].as_i64().unwrap();
                let kind = c["type"].as_str().unwrap();
                let arg1 = c["arg1"].as_str().unwrap();
                let ok = outcome != "failed";
                if ok && ["send", "sendmentions", "reply", "sendimage"].contains(&kind) {
                    let idx: i64 = db.query_row("SELECT COUNT(*) FROM chat_messages WHERE chat=?", [arg1], |r| r.get(0)).unwrap();
                    db.execute(
                        "INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES(?,?,?,?,?,?,?,?)",
                        params![arg1, idx, "1790433000000", "ROSSI Anna", "sent", 1, "", ""],
                    )
                    .unwrap();
                }
                if ok && kind == "download" {
                    std::fs::create_dir_all(slot_dir(slot).join("files")).unwrap();
                    std::fs::write(slot_dir(slot).join("files").join("0123456789abcdef.csv"), "a,b\n1,2\n").unwrap();
                    db.execute("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)", params![format!("cmd_result:{id}"), r#"{"f":"0123456789abcdef.csv"}"#])
                        .unwrap();
                }
                if kind == "open" {
                    db.execute("INSERT OR REPLACE INTO state(k, v) VALUES('viewing', ?)", [json!({"chat": arg1, "ts": now_s()}).to_string()]).unwrap();
                    if outcome == "opens" {
                        db.execute("INSERT OR REPLACE INTO state(k, v) VALUES('active_chat', ?)", [arg1]).unwrap();
                        db.execute(
                            "INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES(?,?,?,?,?,?,?,?)",
                            params![arg1, 0, "1790432000000", "VERDI Carla", "New here", 0, "", ""],
                        )
                        .unwrap();
                    }
                }
                db.execute("UPDATE commands SET status=? WHERE id=?", params![if ok { "done" } else { "failed" }, id]).unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
}

#[test]
fn mcp_endpoint() {
    let _env = crate::TEST_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let dir = std::env::temp_dir().join(format!("trweb-mcp-{}-{}", std::process::id(), crate::web::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    set_env("APP_DB", dir.join("app.db").to_str().unwrap());
    set_env("ADMIN_EMAIL", "admin@teamsrelay.test");
    set_env("MCP_TOKEN", TOKEN);
    set_env("APP_URL", "https://teams.example");
    crate::appdb::with(crate::appdb::migrate).unwrap();
    crate::appdb::with(|c| {
        c.execute_batch(r#"CREATE TABLE IF NOT EXISTS "user"(id TEXT PRIMARY KEY, email TEXT NOT NULL); INSERT INTO "user"(id, email) VALUES('u1', 'admin@teamsrelay.test')"#)
    })
    .unwrap();
    let mine = crate::appdb::claim_slot("u1", Some(4)).unwrap();
    let other = crate::appdb::claim_slot("u2", Some(4)).unwrap();
    let empty = crate::appdb::claim_slot("u1", Some(4)).unwrap();
    let env = Env { mine, other, empty, db: open_store(&slot_db_path(mine)).unwrap() };
    env.reset();

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    tokio::task::LocalSet::new().block_on(&rt, async {
        access(&env).await;
        wire(&env).await;
        reads(&env);
        refresh(&env).await;
        commands(&env).await;
        attachments(&env).await;
    });
    let _ = std::fs::remove_dir_all(dir);
}

async fn access(env: &Env) {
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}).to_string();
    let bearer = format!("Bearer {TOKEN}");
    let post = |h: Vec<(&'static str, String)>| {
        let init = init.clone();
        async move {
            let mut all: Vec<(&str, &str)> = vec![ACCEPT, JSON];
            all.extend(h.iter().map(|(k, v)| (*k, v.as_str())));
            post_with(&all, &init).await
        }
    };

    // the token: 401 with a challenge, session cookies ignored; any case of the scheme
    for h in [
        vec![],
        vec![("authorization", "Bearer wrong".to_string())],
        vec![("authorization", TOKEN.to_string())],
        vec![("cookie", "better-auth.session_token=x".to_string())],
    ] {
        let r = post(h).await;
        assert_eq!(r.headers().get("www-authenticate").unwrap(), "Bearer realm=\"teamsrelay\"");
        assert_eq!(text_of(r).await, (401, r#"{"detail":"Missing or wrong token"}"#.into()));
    }
    assert_eq!(post(vec![("authorization", format!("bearer {TOKEN}"))]).await.status(), 200);
    assert!(super::token_matches(Some(&bearer)));
    for h in [
        None,
        Some(""),
        Some(TOKEN),
        Some(&format!("Bearer {TOKEN}x")[..]),
        Some(&format!("Bearer {}", &TOKEN[1..])[..]),
        Some(&format!("Basic {TOKEN}")[..]),
        Some("Bearer "),
    ] {
        assert!(!super::token_matches(h), "{h:?}");
    }

    // requests from web pages
    let r = post(vec![("authorization", bearer.clone()), ("origin", "https://example.com".into())]).await;
    assert_eq!(text_of(r).await, (403, r#"{"detail":"Requests from web pages are not accepted"}"#.into()));

    // the administrator of .env, whatever the case of the address; gone: 503
    set_env("ADMIN_EMAIL", "Admin@TeamsRelay.test");
    assert_eq!(super::mcp_user_id().unwrap().as_deref(), Some("u1"));
    set_env("ADMIN_EMAIL", "gone@teamsrelay.test");
    let r = post(vec![("authorization", bearer.clone())]).await;
    assert_eq!(text_of(r).await, (503, r#"{"detail":"The administrator of .env does not exist"}"#.into()));
    set_env("ADMIN_EMAIL", "admin@teamsrelay.test");

    // off without MCP_TOKEN
    set_env("MCP_TOKEN", "");
    assert!(super::config_error().is_none());
    let r = post(vec![("authorization", bearer.clone())]).await;
    assert_eq!(text_of(r).await, (404, r#"{"detail":"Not found"}"#.into()));
    set_env("MCP_TOKEN", "short");
    assert!(super::config_error().unwrap().contains("at least 32 characters"));
    set_env("MCP_TOKEN", TOKEN);
    assert!(super::config_error().is_none());

    // GET and DELETE: the 405 of the stateless transport
    for m in ["GET", "DELETE"] {
        let route = super::routes().into_iter().find(|r| r.method == m).unwrap();
        let r = (route.handler)(request(m, &[], "")).await.unwrap();
        assert_eq!(r.headers().get("allow").unwrap(), "POST");
        assert_eq!(text_of(r).await, (405, r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}"#.into()));
    }
    let _ = env;
}

async fn wire(env: &Env) {
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
    let (status, body) = mcp(init.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        "event: message\ndata: {\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{\"tools\":{\"listChanged\":true}},\"serverInfo\":{\"name\":\"teamsrelay\",\"version\":\"1.0.0\"}},\"jsonrpc\":\"2.0\",\"id\":1}\n\n"
    );
    let r = post_with(&[("authorization", &format!("Bearer {TOKEN}")), ACCEPT, JSON], &init.to_string()).await;
    assert_eq!(r.headers().get("content-type").unwrap(), "text/event-stream");
    assert_eq!(r.headers().get("x-accel-buffering").unwrap(), "no");

    let (_, body) = mcp(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})).await;
    assert_eq!(body, format!("event: message\ndata: {{\"result\":{{\"tools\":{}}},\"jsonrpc\":\"2.0\",\"id\":2}}\n\n", include_str!("tools.json").trim()));
    let tools = event(&body)["result"]["tools"].as_array().unwrap().clone();
    let readers = ["get_attachment", "list_accounts", "list_activity", "list_chats", "read_chat"];
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], json!(readers.contains(&name)), "{name}");
    }

    assert_eq!(
        mcp(json!({"jsonrpc": "2.0", "id": 3, "method": "ping"})).await,
        (200, "event: message\ndata: {\"result\":{},\"jsonrpc\":\"2.0\",\"id\":3}\n\n".into())
    );
    assert_eq!(mcp(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await, (202, String::new()));
    assert_eq!(
        mcp(json!({"jsonrpc": "2.0", "id": 4, "method": "foo"})).await.1,
        "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}\n\n"
    );
    assert_eq!(
        mcp(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "nope", "arguments": {}}})).await.1,
        "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":5,\"error\":{\"code\":-32602,\"message\":\"Tool nope not found\"}}\n\n"
    );

    // the validation errors recorded from the TypeScript server
    let (_, body) = mcp(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_chats", "arguments": {"account": 0}}})).await;
    assert_eq!(
        body,
        "event: message\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Input validation error: Invalid arguments for tool list_chats: account: Too small: expected number to be >=1\"}],\"isError\":true},\"jsonrpc\":\"2.0\",\"id\":3}\n\n"
    );
    let (_, body) = mcp(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "read_chat", "arguments": {"chat": " "}}})).await;
    assert_eq!(
        body,
        "event: message\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Input validation error: Invalid arguments for tool read_chat: chat: Invalid string: must match pattern /\\\\S/\"}],\"isError\":true},\"jsonrpc\":\"2.0\",\"id\":4}\n\n"
    );

    // the transport's checks
    let auth = format!("Bearer {TOKEN}");
    let ping = json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}).to_string();
    assert_eq!(
        text_of(post_with(&[("authorization", &auth), ("accept", "application/json"), JSON], &ping).await).await,
        (
            406,
            r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Not Acceptable: Client must accept both application/json and text/event-stream"},"id":null}"#
                .into()
        )
    );
    assert_eq!(
        text_of(post_with(&[("authorization", &auth), ACCEPT, ("content-type", "text/plain")], &ping).await).await,
        (415, r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Unsupported Media Type: Content-Type must be application/json"},"id":null}"#.into())
    );
    assert_eq!(
        text_of(post_with(&[("authorization", &auth), ACCEPT, JSON, ("mcp-protocol-version", "1999-01-01")], &ping).await).await,
        (400, r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Bad Request: Unsupported protocol version: 1999-01-01 (supported versions: 2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05, 2024-10-07)"},"id":null}"#.into())
    );
    assert_eq!(
        text_of(post_with(&[("authorization", &auth), ACCEPT, JSON], "{bad").await).await,
        (400, r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"Parse error: Invalid JSON"},"id":null}"#.into())
    );
    assert_eq!(
        text_of(post_with(&[("authorization", &auth), ACCEPT, JSON], "[]").await).await,
        (400, r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Bad Request: empty JSON-RPC batch"},"id":null}"#.into())
    );
    let (_, body) = mcp(json!([{"jsonrpc": "2.0", "id": 2, "method": "ping"}, {"jsonrpc": "2.0", "method": "notifications/initialized"}])).await;
    assert_eq!(body, "event: message\ndata: {\"result\":{},\"jsonrpc\":\"2.0\",\"id\":2}\n\n");

    // the chats of the administrator of .env, as structured content and as its JSON text
    let r = call("read_chat", json!({"chat": "BIANCHI Luca"})).await;
    assert_eq!(r.get("isError"), None);
    assert_eq!(r["structuredContent"]["account"], json!(env.mine));
    assert_eq!(r["structuredContent"]["messages"][1]["time"], json!("2026-09-26T14:07:44.072Z"));
    assert_eq!(serde_json::from_str::<Value>(r["content"][0]["text"].as_str().unwrap()).unwrap(), r["structuredContent"]);
    // the account of another user: an error result
    assert_eq!(call("list_chats", json!({"account": env.other})).await, json!({"content": [{"type": "text", "text": "Account not found"}], "isError": true}));
    assert_eq!(call("list_accounts", json!({})).await["structuredContent"]["accounts"].as_array().unwrap().len(), 2);
}

fn reads(env: &Env) {
    let mine = env.mine;
    let accounts = tools::list_accounts("u1").unwrap()["accounts"].clone();
    assert_eq!(
        accounts[0],
        json!({"slot": mine, "name": "ROSSI Anna", "email": "anna@contoso.example", "tenant": "Contoso", "teams": "ok", "overall": "green", "stopped": false, "unread": 1})
    );
    assert_eq!(accounts[1]["slot"], json!(env.empty));
    assert_eq!(accounts[1]["teams"], json!("starting"));

    assert_eq!(
        tools::list_chats("u1", &args(json!({}))).unwrap(),
        json!({"account": mine, "chats": [
            {"name": "BIANCHI Luca", "preview": "See you at 3", "last": "14:07", "unread": true},
            {"name": "Cloud team", "preview": "Deploy done", "last": "Yesterday", "unread": false, "mention": true},
            {"name": "ROSSI Anna (You)", "preview": "note", "last": "9/22", "unread": false, "muted": true},
        ]})
    );
    assert_eq!(tools::list_chats("u1", &args(json!({"unread_only": true}))).unwrap()["chats"].as_array().unwrap().len(), 1);

    // open: in use within 30 s and not left, agent and Teams working
    let open = || -> Vec<Value> {
        tools::list_chats("u1", &args(json!({}))).unwrap()["chats"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c.get("open").is_some())
            .map(|c| c["name"].clone())
            .collect()
    };
    env.state("active_chat", "Cloud team");
    env.state("viewing", &json!({"chat": "Cloud team", "ts": now_s()}).to_string());
    assert_eq!(open(), vec![json!("Cloud team")]);
    env.state("viewing", &json!({"chat": "Cloud team", "ts": now_s() - 31}).to_string());
    assert!(open().is_empty());
    env.state("viewing", &json!({"chat": "", "ts": now_s()}).to_string());
    assert!(open().is_empty());
    env.state("viewing", &json!({"chat": "Cloud team", "ts": now_s()}).to_string());
    env.health("login");
    assert!(open().is_empty());
    env.reset();

    assert_eq!(
        tools::read_chat("u1", &args(json!({"chat": "BIANCHI Luca"}))).unwrap(),
        json!({"account": mine, "chat": "BIANCHI Luca", "live": false, "messages": [
            {"id": "1790431027228", "time": "2026-09-26T13:57:07.228Z", "author": "BIANCHI Luca", "mine": false, "text": "Can you check the pipeline?", "reactions": [{"emoji": "like", "count": 2, "mine": true}], "images": 1, "mentions_me": true},
            {"id": "1790431664072", "time": "2026-09-26T14:07:44.072Z", "author": "ROSSI Anna", "mine": true, "text": "Done", "quote": {"author": "BIANCHI Luca", "text": "Can you check the pipeline?"}, "files": ["report.xlsx"], "edited": true},
            {"id": "temp-1", "author": "BIANCHI Luca", "mine": false, "text": "", "deleted": true},
        ]})
    );
    env.state("active_chat", "BIANCHI Luca");
    env.state("viewing", &json!({"chat": "BIANCHI Luca", "ts": now_s()}).to_string());
    assert_eq!(tools::read_chat("u1", &args(json!({"chat": "BIANCHI Luca"}))).unwrap()["live"], json!(true));
    env.reset();
    let never = tools::read_chat("u1", &args(json!({"chat": "Cloud team"}))).unwrap();
    assert_eq!(never["messages"], json!([]));
    assert_eq!(never["note"], json!("No messages saved for this chat yet: refresh_chat opens it in Teams and reads them"));
    assert_eq!(fail_text(tools::read_chat("u1", &args(json!({"chat": "Nobody"})))), "No chat with this name: use a name as list_chats gives it");
    assert_eq!(fail_text(tools::read_chat("u1", &args(json!({"account": env.other, "chat": "BIANCHI Luca"})))), "Account not found");
    assert_eq!(fail_text(tools::read_chat("u1", &args(json!({"account": 99, "chat": "BIANCHI Luca"})))), "Account not found");
    assert_eq!(fail_text(tools::read_chat("u1", &args(json!({"account": env.empty, "chat": "BIANCHI Luca"})))), "Account not ready yet");

    assert_eq!(
        tools::list_activity("u1", &args(json!({}))).unwrap(),
        json!({"account": mine, "read_at": "2026-09-26T13:56:40.000Z", "items": [
            {"kind": "mention", "actor": "BIANCHI Luca", "title": "mentioned you", "preview": "@Anna see this", "time": "14:00", "chat": "Cloud team", "unread": true},
            {"kind": "reaction", "actor": "VERDI Carla", "title": "reacted to your message", "emoji": "heart", "preview": "Done", "time": "Yesterday", "chat": "BIANCHI Luca", "unread": false},
        ]})
    );
    assert_eq!(tools::list_activity("u1", &args(json!({"unread_only": true}))).unwrap()["items"].as_array().unwrap().len(), 1);
}

async fn refresh(env: &Env) {
    let chat = args(json!({"chat": "Cloud team"}));
    let a = agent(env.mine, "opens");
    let r = tools::refresh_chat("u1", &chat, FAST).await.unwrap();
    a.abort();
    assert_eq!(r["live"], json!(true));
    assert_eq!(r["messages"], json!([{"id": "1790432000000", "time": "2026-09-26T14:13:20.000Z", "author": "VERDI Carla", "mine": false, "text": "New here"}]));
    assert_eq!(env.queued(), vec![json!({"type": "open", "arg1": "Cloud team", "arg2": ""})]);
    env.reset();

    env.state("active_chat", "BIANCHI Luca");
    let a = agent(env.mine, "does-not-open");
    assert_eq!(fail_text(tools::refresh_chat("u1", &chat, FAST).await), "Teams did not open this chat");
    a.abort();
    env.reset();
    let a = agent(env.mine, "failed");
    assert_eq!(fail_text(tools::refresh_chat("u1", &chat, FAST).await), "Teams could not open this chat");
    a.abort();
    env.reset();
    assert_eq!(
        fail_text(tools::refresh_chat("u1", &chat, Wait { timeout_ms: 100, poll_ms: 20 }).await),
        "Teams did not open this chat within 0.1 s: see list_accounts"
    );
    env.reset();

    // nothing queued for an unknown chat, a Teams not working or a stopped account
    assert!(fail_text(tools::refresh_chat("u1", &args(json!({"chat": "Nobody"})), FAST).await).contains("list_chats"));
    env.health("login");
    assert_eq!(fail_text(tools::refresh_chat("u1", &chat, FAST).await), "Teams is not working on this account (login): see list_accounts");
    env.health("ok");
    crate::appdb::set_slot_stopped(env.mine, true).unwrap();
    assert!(fail_text(tools::refresh_chat("u1", &chat, FAST).await).contains("stopped"));
    assert!(env.queued().is_empty());
    env.reset();
}

async fn commands(env: &Env) {
    let png: Vec<u8> = [vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], vec![0; 16]].concat();
    let a = agent(env.mine, "done");
    let r = tools::send_message("u1", &args(json!({"chat": "BIANCHI Luca", "text": "Ciao 👋"})), FAST).await.unwrap();
    assert_eq!(r["account"], json!(env.mine));
    assert_eq!(r["status"], json!("done"));
    assert_eq!(r["message"], json!({"id": "1790433000000", "time": "2026-09-26T14:30:00.000Z", "author": "ROSSI Anna", "mine": true, "text": "sent"}));
    assert_eq!(env.queued(), vec![json!({"type": "send", "arg1": "BIANCHI Luca", "arg2": "Ciao 👋"})]);
    env.reset();

    tools::send_message("u1", &args(json!({"chat": "Cloud team", "text": "@VERDI Carla look", "mentions": ["VERDI Carla"]})), FAST).await.unwrap();
    tools::send_message("u1", &args(json!({"chat": "BIANCHI Luca", "text": "On it", "reply_to": "1790431027228"})), FAST).await.unwrap();
    let data_url = format!("data:image/png;base64,{}", crate::js::base64_std(&png));
    tools::send_message("u1", &args(json!({"chat": "BIANCHI Luca", "text": "screen", "image": data_url})), FAST).await.unwrap();
    let q = env.queued();
    assert_eq!(q[0], json!({"type": "sendmentions", "arg1": "Cloud team", "arg2": r#"{"parts":[{"mention":"VERDI Carla"},{"text":" look"}]}"#}));
    assert_eq!(q[1], json!({"type": "reply", "arg1": "BIANCHI Luca", "arg2": r#"{"mid":"1790431027228","text":"On it"}"#}));
    assert_eq!(q[2]["type"], json!("sendimage"));
    let image: Value = serde_json::from_str(q[2]["arg2"].as_str().unwrap()).unwrap();
    assert_eq!(image["text"], json!("screen"));
    let file = image["file"].as_str().unwrap();
    assert!(regex::Regex::new(r"^[0-9a-f]{16}\.png$").unwrap().is_match(file), "{file}");
    assert_eq!(std::fs::read(slot_dir(env.mine).join("uploads").join(file)).unwrap(), png);
    env.reset();

    let r = tools::react_message("u1", &args(json!({"chat": "BIANCHI Luca", "message_id": "1790431027228", "reaction": "Heart"})), FAST).await.unwrap();
    assert_eq!(r["status"], json!("done"));
    assert_eq!(r["message"]["id"], json!("1790431027228"));
    assert_eq!(r["message"]["author"], json!("BIANCHI Luca"));
    tools::react_message("u1", &args(json!({"chat": "BIANCHI Luca", "message_id": "1790431027228", "reaction": "👍"})), FAST).await.unwrap();
    tools::edit_message("u1", &args(json!({"chat": "BIANCHI Luca", "message_id": "1790431664072", "text": "Done ✅"})), FAST).await.unwrap();
    let mref = args(json!({"chat": "BIANCHI Luca", "message_id": "1790431664072"}));
    tools::message_command("u1", &mref, "delete", "the delete", FAST).await.unwrap();
    tools::message_command("u1", &mref, "undodelete", "the message was brought back", FAST).await.unwrap();
    assert_eq!(
        env.queued(),
        vec![
            json!({"type": "react", "arg1": "BIANCHI Luca", "arg2": r#"{"mid":"1790431027228","emoji":"heart"}"#}),
            json!({"type": "react", "arg1": "BIANCHI Luca", "arg2": r#"{"mid":"1790431027228","pill":"👍"}"#}),
            json!({"type": "edit", "arg1": "BIANCHI Luca", "arg2": r#"{"mid":"1790431664072","text":"Done ✅"}"#}),
            json!({"type": "delete", "arg1": "BIANCHI Luca", "arg2": r#"{"mid":"1790431664072"}"#}),
            json!({"type": "undodelete", "arg1": "BIANCHI Luca", "arg2": r#"{"mid":"1790431664072"}"#}),
        ]
    );
    assert_eq!(
        fail_text(tools::edit_message("u1", &args(json!({"chat": "BIANCHI Luca", "message_id": "1790431664072", "text": " "})), FAST).await),
        "Empty text"
    );
    // a tool through the wire waits for the agent as well
    let r = call("delete_message", json!({"chat": "BIANCHI Luca", "message_id": "1790431664072"})).await;
    assert_eq!(r["structuredContent"]["status"], json!("done"));
    a.abort();
    env.reset();

    // nothing queued for what cannot go
    let send = |v: Value| async move { fail_text(tools::send_message("u1", &args(v), FAST).await) };
    assert_eq!(send(json!({"chat": "BIANCHI Luca", "text": " "})).await, "text is required (or image)");
    assert_eq!(
        send(json!({"chat": "BIANCHI Luca", "image": crate::js::base64_std(&png), "reply_to": "1"})).await,
        "An image goes alone or with a caption in text: no reply_to or mentions with it"
    );
    assert!(send(json!({"chat": "BIANCHI Luca", "image": crate::js::base64_std(b"not an image")})).await.contains("PNG, JPEG"));
    assert!(send(json!({"chat": "Nobody", "text": "hi"})).await.contains("list_chats"));
    assert_eq!(
        send(json!({"chat": "BIANCHI Luca", "text": "hi", "reply_to": "1", "mentions": ["A"]})).await,
        "A reply cannot tag people: send a message with mentions instead"
    );
    env.health("login");
    assert!(send(json!({"chat": "BIANCHI Luca", "text": "hi"})).await.contains("not working"));
    assert!(env.queued().is_empty());
    env.reset();

    // Teams did not confirm; the command still waits
    let a = agent(env.mine, "failed");
    assert_eq!(
        send(json!({"chat": "BIANCHI Luca", "text": "hi"})).await,
        "Teams did not confirm the message was sent: read_chat (or refresh_chat) shows whether it happened, check before trying again"
    );
    a.abort();
    let r = tools::send_message("u1", &args(json!({"chat": "BIANCHI Luca", "text": "hi"})), Wait { timeout_ms: 60, poll_ms: 20 }).await.unwrap();
    assert_eq!(r["status"], json!("pending"));
    assert_eq!(r["note"], json!("Queued, not run yet: the agent runs it within 2 minutes or drops it. read_chat shows the outcome later"));
    assert_eq!(r.as_object().unwrap().keys().collect::<Vec<_>>(), ["account", "chat", "command", "status", "note"]);
    env.reset();
}

async fn attachments(env: &Env) {
    let png: Vec<u8> = [vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], vec![0; 16]].concat();
    let media = slot_dir(env.mine).join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("00112233445566aa.png"), &png).unwrap();
    let extra = json!({"images": [{"f": "00112233445566aa.png"}], "files": [{"name": "data.csv", "url": "https://contoso.sharepoint.com/sites/x/data.csv"}]});
    env.db.execute("UPDATE chat_messages SET extra=? WHERE mid='1790431027228'", [extra.to_string()]).unwrap();
    let a = agent(env.mine, "done");
    let blocks = tools::get_attachment("u1", &args(json!({"chat": "BIANCHI Luca", "message_id": "1790431027228"})), FAST).await.unwrap();
    a.abort();
    let summary: Value = serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(summary["account"], json!(env.mine));
    assert_eq!(
        summary["attachments"],
        json!([
            {"index": 1, "kind": "image", "shown": true},
            {"index": 2, "kind": "file", "name": "data.csv", "shown": true, "link": format!("https://teams.example/files/0123456789abcdef.csv?a={}&name=data.csv", env.mine)},
        ])
    );
    assert_eq!(blocks[1], json!({"type": "image", "data": crate::js::base64_std(&png), "mimeType": "image/png"}));
    assert_eq!(blocks[2], json!({"type": "text", "text": "--- data.csv ---\na,b\n1,2\n"}));
    assert_eq!(env.queued(), vec![json!({"type": "download", "arg1": "https://contoso.sharepoint.com/sites/x/data.csv", "arg2": r#"{"name":"data.csv"}"#})]);
    env.reset();

    let extra =
        json!({"images": [{"url": "https://eu-api.asm.skype.com/v1/objects/x/views/imgo"}], "files": [{"name": "a.pdf", "url": "https://example.com/a.pdf"}]});
    env.db.execute("UPDATE chat_messages SET extra=? WHERE mid='1790431027228'", [extra.to_string()]).unwrap();
    let get = |v: Value| async move { tools::get_attachment("u1", &args(v), FAST).await };
    let one = get(json!({"chat": "BIANCHI Luca", "message_id": "1790431027228", "index": 1})).await.unwrap();
    assert_eq!(one.len(), 1);
    let summary: Value = serde_json::from_str(one[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        summary["attachments"],
        json!([{"index": 1, "kind": "image", "shown": false, "note": "Teams keeps this image behind the sign-in of the account and it is not saved yet: refresh_chat saves it", "link": "https://eu-api.asm.skype.com/v1/objects/x/views/imgo"}])
    );
    let two = get(json!({"chat": "BIANCHI Luca", "message_id": "1790431027228", "index": 2})).await.unwrap();
    let summary: Value = serde_json::from_str(two[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        summary["attachments"][0],
        json!({"index": 2, "kind": "file", "name": "a.pdf", "shown": false, "note": "not on SharePoint or OneDrive: not downloaded", "link": "https://example.com/a.pdf"})
    );
    assert_eq!(
        fail_text(get(json!({"chat": "BIANCHI Luca", "message_id": "1790431027228", "index": 3})).await),
        "index out of range: the message has 2 attachment(s)"
    );
    assert!(fail_text(get(json!({"chat": "BIANCHI Luca", "message_id": "nope"})).await).contains("read_chat"));
    assert_eq!(fail_text(get(json!({"chat": "BIANCHI Luca", "message_id": "temp-1"})).await), "This message has no image or file");
    assert!(env.queued().is_empty());
    // as content blocks through the wire: no structured content
    let r = call("get_attachment", json!({"chat": "BIANCHI Luca", "message_id": "1790431027228", "index": 2})).await;
    assert_eq!(r.as_object().unwrap().keys().collect::<Vec<_>>(), ["content"]);
    env.reset();
}
