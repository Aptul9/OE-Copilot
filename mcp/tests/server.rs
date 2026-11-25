//! The whole server over HTTP against fake backends on 127.0.0.1 (OE Copilot broker and agent, Outlook REST, Slack,
//! TeamsRelay /mcp): the MCP wire answers recorded from the TypeScript server, and what each backend is asked.

use bytes::Bytes;
use communication_mcp::communication::Communication;
use communication_mcp::gateway::Gateway;
use communication_mcp::http::{App, serve};
use communication_mcp::outlook::Outlook;
use communication_mcp::slack::Slack;
use communication_mcp::teams::Teams;
use communication_mcp::tokens::Tokens;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const TOKEN: &str = "0123456789abcdef0123456789abcdef-test";

/// What each backend was asked: method, path and query, body, and the Authorization and Cookie headers.
type Log = Arc<Mutex<Vec<(String, String, String, String)>>>;

const EVERFLOW: &str = "everflow-token-0123456789abcdef0123456789";

fn jwt(upn: &str) -> String {
    use base64::Engine;
    let c = json!({"upn": upn, "aud": "https://outlook.office.com", "scp": "Mail.ReadWrite", "exp": 4_000_000_000u64});
    format!("h.{}.s", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(c.to_string()))
}

/// The fake backends: one server, answers chosen by path.
async fn backend(log: Log) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::task::spawn_local(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::task::spawn_local(async move {
                let svc = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                    let log = log.clone();
                    async move {
                        let method = req.method().to_string();
                        let pq = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default();
                        let sized = req.headers().contains_key("content-length") || req.headers().contains_key("transfer-encoding");
                        let h = |k: &str| req.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                        let credentials = format!("authorization={}|cookie={}", h("authorization"), h("cookie"));
                        let body = String::from_utf8_lossy(&req.into_body().collect().await.unwrap().to_bytes()).to_string();
                        log.lock().unwrap().push((method.clone(), pq.clone(), body.clone(), credentials.clone()));
                        if pq.starts_with("/api/everflow/attachment?") {
                            return Ok::<_, std::convert::Infallible>(relay_attachment(&pq, &credentials));
                        }
                        // like Outlook: a POST must say how long it is
                        let (status, ctype, out) = if method == "POST" && !sized {
                            (411, "text/html", "Length Required".into())
                        } else {
                            answer(port, &method, &pq, &body, &credentials)
                        };
                        Ok::<_, std::convert::Infallible>(
                            Response::builder().status(status).header("content-type", ctype).body(Full::new(Bytes::from(out))).unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(s), svc).await;
            });
        }
    });
    port
}

fn answer(port: u16, method: &str, pq: &str, body: &str, credentials: &str) -> (u16, &'static str, String) {
    let j = |v: Value| (200, "application/json", v.to_string());
    let path = pq.split('?').next().unwrap_or("");
    match (method, path) {
        ("GET", "/api/communication/accounts") => j(json!({"accounts": [{"upn": "a@x.it", "status": "ok"}]})),
        ("POST", "/api/communication/read") if body.contains("down@x.it") => (502, "application/json", r#"{"errore":"Outlook non raggiungibile"}"#.into()),
        ("POST", "/api/communication/read") => j(json!({"value": [{"Id": "m1"}]})),
        ("POST", "/api/conversations.history") => j(json!({"ok": true, "messages": [{"ts": "1700000000.000100", "text": "hi"}]})),
        ("GET", "/api/everflow/saved") if pq.contains("since=7&") => (200, "application/json", "x".repeat(64 * 1024 * 1024 + 1)),
        ("GET", "/api/everflow/saved") if credentials.starts_with("authorization=Bearer relay-token|") => {
            j(json!({"slots": [{"slot": 3, "snapshot": "{\"state\":{}}"}, {"slot": 7, "error": "store-unavailable"}], "asked": pq}))
        }
        ("GET", "/api/everflow/saved") => (401, "application/json", r#"{"detail":"Missing or wrong token"}"#.into()),
        ("GET", "/api/token-posta") => j(json!({"account": [{"upn": "a@x.it", "token": jwt("a@x.it")}]})),
        ("GET", "/api/communication/google/accounts") => j(json!({"accounts": [{"provider": "gmail", "account": "g@x.it", "status": "ok"}]})),
        ("POST", "/api/communication/gmail/write") => j(json!({"id": "d1", "message": {"id": "m1", "threadId": "t1"}})),
        ("GET", "/api/v2.0/me/MailFolders/Inbox/messages") if !pq.contains("skip") => j(json!({
            "value": [{"Id": "m1", "IsRead": true, "ConversationId": "c1"}, {"Id": "m2", "IsRead": false}],
            "@odata.nextLink": format!("http://127.0.0.1:{port}/api/v2.0/me/MailFolders('Inbox')/messages?$skip=2"),
        })),
        ("GET", "/api/v2.0/me/MailFolders('Inbox')/messages") => j(json!({"value": [{"Id": "m2", "IsRead": false}, {"Id": "m3", "IsRead": false}]})),
        ("GET", "/api/v2.0/me/MailFolders/SentItems/messages") => j(json!({"value": [{"Id": "s1", "IsRead": true}]})),
        ("GET", "/api/v2.0/me/messages/m1") => j(json!({"Id": "m1", "Subject": "Native", "HasAttachments": false})),
        ("POST", "/api/v2.0/me/sendmail") => (202, "application/json", String::new()),
        ("POST", "/api/v2.0/me/messages") => (201, "application/json", json!({"Id": "d1", "Subject": "s"}).to_string()),
        ("GET", "/api/v2.0/me/messages/d1") => j(json!({"IsDraft": true, "Subject": "s", "ToRecipients": [{"EmailAddress": {"Address": "b@x.it"}}]})),
        ("POST", "/api/v2.0/me/messages/d1/send") => (202, "application/json", String::new()),
        ("GET", "/api/v2.0/me/messages/m1/attachments") => j(json!({"value": [{"Id": "a1", "Name": "Q3 report.pdf", "Size": 5}]})),
        ("GET", "/api/v2.0/me/messages/m1/attachments/a1") => {
            j(json!({"Id": "a1", "Name": "Q3 report.pdf", "ContentType": "application/pdf", "ContentBytes": "aGVsbG8="}))
        }
        ("POST", "/api/users.conversations") => j(
            json!({"ok": true, "channels": [{"id": "D1", "is_im": true, "user": "U1"}, {"id": "C1", "name": "general", "topic": {"value": "t"}}], "response_metadata": {"next_cursor": ""}}),
        ),
        ("POST", "/api/conversations.replies") => j(json!({"ok": true, "messages": [{"ts": "1700000000.000100", "text": "parent", "reply_count": 1}]})),
        ("POST", "/api/search.messages") if body.contains("query=dm") => j(json!({"ok": true, "messages": {"total": 1, "matches": [{
            "iid": "i1", "ts": "1700000000.000200", "user": "U1", "text": "in the dm",
            "channel": {"id": "D1", "is_im": true, "name": "U1"},
            "permalink": "https://acme.slack.com/archives/D1/p1700000000000200?thread_ts=1700000000.000100&cid=D1",
        }]}})),
        ("POST", "/api/users.info") => j(json!({"ok": true, "user": {"id": "U1", "name": "me", "real_name": "Me", "profile": {"email": "me@x.it"}}})),
        ("POST", "/api/chat.postMessage") => j(json!({"ok": true, "ts": "1700000000.000100", "channel": "C1"})),
        ("POST", "/api/chat.getPermalink") => j(json!({"ok": false, "error": "no_permission"})),
        ("POST", "/api/search.messages") => j(json!({"ok": true, "messages": {"matches": [], "total": 0}})),
        ("POST", "/mcp") => {
            let req: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let id = req["id"].clone();
            let result = match req["params"]["name"].as_str() {
                Some("list_accounts") => json!({"content": [{"type": "text", "text": "{\"accounts\":[{\"slot\":3,\"name\":\"Work\",\"email\":\"w@x.it\"}]}"}]}),
                Some(read @ ("read_chat" | "refresh_chat")) if req["params"]["arguments"]["chat"] == "Anna" => {
                    json!({"content": [{"type": "text", "text": json!({"messages": [{"mid": "1", "text": read}]}).to_string()}]})
                }
                Some("send_message") if req["params"]["arguments"]["chat"] == "Anna" => json!({"content": [{"type": "text", "text": "{\"ok\":true}"}]}),
                _ => json!({"content": [{"type": "text", "text": "nope"}], "isError": true}),
            };
            (200, "text/event-stream", format!("event: message\ndata: {}\n\n", json!({"result": result, "jsonrpc": "2.0", "id": id})))
        }
        _ => (404, "application/json", "{}".into()),
    }
}

async fn post(port: u16, body: &str, headers: &[(&str, &str)]) -> (u16, String, String) {
    let auth = format!("Bearer {TOKEN}");
    let mut all: Vec<(&str, &str)> =
        vec![("Host", "mcp.test"), ("Authorization", &auth), ("Content-Type", "application/json"), ("Accept", "application/json, text/event-stream")];
    for (k, v) in headers {
        all.retain(|(x, _)| !x.eq_ignore_ascii_case(k));
        all.push((k, v));
    }
    let mut req = reqwest::Client::new().post(format!("http://127.0.0.1:{port}/mcp"));
    for (k, v) in all {
        req = req.header(k, v);
    }
    let r = req.body(body.to_string()).send().await.unwrap();
    let status = r.status().as_u16();
    let ctype = r.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    (status, ctype, r.text().await.unwrap())
}

fn data(sse: &str) -> Value {
    let line = sse.lines().find(|l| l.starts_with("data: ")).unwrap_or("data: null");
    serde_json::from_str(&line[6..]).unwrap()
}

async fn tool(port: u16, name: &str, args: Value) -> Value {
    let (_, _, body) =
        post(port, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string(), &[]).await;
    let r = data(&body)["result"].clone();
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn the_server_end_to_end() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    tokio::task::LocalSet::new().block_on(&rt, async {
        let log: Log = Arc::default();
        let b = backend(log.clone()).await;
        let dir = std::env::temp_dir().join(format!("cmcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("workspaces.json");
        std::fs::write(&store, json!({"workspaces": {"T1": {"workspace_id": "T1", "workspace_name": "Acme", "workspace_url": "https://acme.slack.com/", "auth_type": "browser", "user_id": "U1", "xoxc_token": "xoxc-1", "xoxd_token": "xoxd/1"}}}).to_string()).unwrap();
        // SAFETY: set once, before the first thread reads them
        unsafe {
            std::env::set_var("COMMUNICATION_ID_KEY", TOKEN);
            std::env::set_var("OUTLOOK_MCP_TEST_REST", format!("http://127.0.0.1:{b}/api/v2.0"));
            std::env::set_var("SLACK_TEST_BASE", format!("http://127.0.0.1:{b}"));
            std::env::set_var("MCP_PUBLIC_URL", "https://mcp.test/communication/");
        }
        let broker = format!("http://127.0.0.1:{b}/api/token-posta");
        let comm = Communication {
            outlook: Outlook::new(Tokens::new(&broker, "key").unwrap()),
            google: Some(Gateway::from_broker(&broker, "key").unwrap()),
            teams: Teams::from_env(Some(&format!("http://127.0.0.1:{b}/mcp")), Some("relay-token")).unwrap(),
            slack: Some(Slack::new(store.to_str().unwrap())),
        };
        let app = Arc::new(App { comm, token: TOKEN.into(), hosts: ["mcp.test".to_string()].into(), everflow: Some(EVERFLOW.into()) });
        let port = start(app).await;

        // the wire, as the TypeScript server answered
        let (s, t, body) = post(port, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#, &[]).await;
        assert_eq!((s, t.as_str()), (200, "text/event-stream"));
        assert!(body.starts_with("event: message\ndata: {\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{\"tools\":{\"listChanged\":true},\"resources\":{}},\"serverInfo\":{\"name\":\"communication\""));
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#, &[]).await.2, "event: message\ndata: {\"result\":{},\"jsonrpc\":\"2.0\",\"id\":3}\n\n");
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &[]).await.0, 202);
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","id":4,"method":"boh"}"#, &[]).await.2, "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}\n\n");
        assert_eq!(post(port, "{nope", &[]).await.2, r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"Parse error: Invalid JSON"},"id":null}"#);
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#, &[("Accept", "application/json")]).await.0, 406);
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#, &[("Content-Type", "text/plain")]).await.0, 415);
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#, &[("MCP-Protocol-Version", "1999-01-01")]).await.0, 400);
        assert_eq!(post(port, r#"{"id":1,"method":"ping"}"#, &[]).await.2, r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Bad Request: the request body is not a valid JSON-RPC message"},"id":1}"#);
        assert_eq!(post(port, "{}", &[("Authorization", "Bearer wrong")]).await.0, 401);
        assert_eq!(post(port, "{}", &[("Host", "evil")]).await.0, 421);
        assert_eq!(post(port, "{}", &[("Origin", "https://x")]).await.0, 403);
        let list = data(&post(port, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, &[]).await.2);
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 26);
        let v = data(&post(port, r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"email_list","arguments":{}}}"#, &[]).await.2);
        assert_eq!(v["result"]["content"][0]["text"], json!("Input validation error: Invalid arguments for tool email_list: account_id: Invalid input: expected string, received undefined"));
        // 2025-era clients read the skills as plain resources
        let res = data(&post(port, r#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#, &[]).await.2);
        let uris: Vec<&str> = res["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
        assert_eq!(uris, ["skill://accounts/SKILL.md", "skill://chats/SKILL.md", "skill://mail-sweep/SKILL.md", "skill://safe-send/SKILL.md"]);
        let read = data(&post(port, r#"{"jsonrpc":"2.0","id":8,"method":"resources/read","params":{"uri":"skill://chats/SKILL.md"}}"#, &[]).await.2);
        assert!(read["result"]["contents"][0]["text"].as_str().unwrap().starts_with("---\nname: chats\n"));
        assert!(read["result"].get("resultType").is_none());
        let gone = data(&post(port, r#"{"jsonrpc":"2.0","id":9,"method":"resources/read","params":{"uri":"skill://x/SKILL.md"}}"#, &[]).await.2);
        assert_eq!(gone["error"]["code"], json!(-32002));

        // 2026-07-28: no initialize, version and capabilities in every request's _meta, mirrored headers checked
        let modern = |id: u32, method: &str, params: Value| {
            let mut p = params;
            p["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientInfo": {"name": "t", "version": "1"}, "io.modelcontextprotocol/clientCapabilities": {}});
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": p}).to_string()
        };
        let h = |method: &'static str| vec![("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Method", method)];
        let (s, _, body) = post(port, &modern(10, "server/discover", json!({})), &h("server/discover")).await;
        let d = data(&body)["result"].clone();
        assert_eq!(s, 200);
        assert_eq!(d["resultType"], json!("complete"));
        assert_eq!(d["supportedVersions"][0], json!("2026-07-28"));
        assert_eq!(d["capabilities"]["extensions"], json!({"io.modelcontextprotocol/skills": {}}));
        assert_eq!(d["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], json!("communication"));
        assert_eq!((d["ttlMs"].as_u64(), d["cacheScope"].as_str()), (Some(300000), Some("private")));
        let sk = data(&post(port, &modern(11, "skills/list", json!({})), &h("skills/list")).await.2)["result"].clone();
        let entry = sk["skills"].as_array().unwrap().iter().find(|e| e["uri"] == "skill://mail-sweep/SKILL.md").unwrap().clone();
        assert_eq!(entry["frontmatter"]["name"], json!("mail-sweep"));
        assert!(entry["resources"][0]["digest"].as_str().unwrap().starts_with("sha256:"));
        let got = data(&post(port, &modern(12, "skills/get", json!({"uri": "skill://mail-sweep/SKILL.md"})), &h("skills/get")).await.2)["result"].clone();
        assert_eq!(got["skill"], entry);
        let unknown = data(&post(port, &modern(13, "skills/get", json!({"uri": "skill://nope/SKILL.md"})), &h("skills/get")).await.2);
        assert_eq!(unknown["error"]["code"], json!(-32602));
        let mut hr = h("resources/read");
        hr.push(("Mcp-Name", "skill://mail-sweep/SKILL.md"));
        let text = data(&post(port, &modern(14, "resources/read", json!({"uri": "skill://mail-sweep/SKILL.md"})), &hr).await.2)["result"]["contents"][0]["text"].clone();
        assert_eq!(text.as_str().unwrap().len() as u64, entry["resources"][0]["size"].as_u64().unwrap());
        let mut ht = h("tools/call");
        ht.push(("Mcp-Name", "=?base64?Y29tbXVuaWNhdGlvbl9hY2NvdW50cw==?="));
        let call = data(&post(port, &modern(15, "tools/call", json!({"name": "communication_accounts", "arguments": {}})), &ht).await.2)["result"].clone();
        assert_eq!((call["resultType"].as_str(), call["structuredContent"]["accounts"].as_array().map(Vec::len)), (Some("complete"), Some(4)));
        assert!(call.get("ttlMs").is_none());
        let tl = data(&post(port, &modern(16, "tools/list", json!({})), &h("tools/list")).await.2)["result"].clone();
        assert!(tl["tools"][0]["description"].as_str().unwrap().contains("outlook:<address>"));
        // refusals come as plain JSON with the status the transport names
        let (s, t, body) = post(port, &modern(17, "ping", json!({})), &h("ping")).await;
        assert_eq!((s, t.as_str(), serde_json::from_str::<Value>(&body).unwrap()["error"]["code"].clone()), (404, "application/json", json!(-32601)));
        let (s, _, body) = post(port, &modern(18, "tools/list", json!({})), &h("tools/call")).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"]["code"].clone()), (400, json!(-32020)));
        let mut bad_name = h("tools/call");
        bad_name.push(("Mcp-Name", "email_send"));
        let (s, _, _) = post(port, &modern(19, "tools/call", json!({"name": "communication_accounts", "arguments": {}})), &bad_name).await;
        assert_eq!(s, 400);
        let future = modern(20, "tools/list", json!({})).replace("2026-07-28", "2099-01-01");
        let (s, _, body) = post(port, &future, &[("MCP-Protocol-Version", "2099-01-01")]).await;
        let e = serde_json::from_str::<Value>(&body).unwrap()["error"].clone();
        assert_eq!((s, e["code"].clone(), e["data"]["supported"][0].clone(), e["data"]["requested"].clone()), (400, json!(-32022), json!("2026-07-28"), json!("2099-01-01")));
        let (s, _, body) = post(port, r#"{"jsonrpc":"2.0","id":21,"method":"tools/list"}"#, &[("MCP-Protocol-Version", "2026-07-28")]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"]["code"].clone()), (400, json!(-32020)));
        let batch = format!("[{},{}]", modern(22, "tools/list", json!({})), modern(23, "tools/list", json!({})));
        assert_eq!(post(port, &batch, &[]).await.0, 400);
        // left-out mirrored headers are let through
        assert_eq!(data(&post(port, &modern(24, "skills/list", json!({})), &[]).await.2)["result"]["resultType"], json!("complete"));

        let health = reqwest::get(format!("http://127.0.0.1:{port}/healthz")).await.unwrap();
        assert_eq!((health.status().as_u16(), health.text().await.unwrap()), (200, r#"{"ok":true}"#.to_string()));

        // accounts of every provider
        let acc = tool(port, "communication_accounts", json!({})).await;
        let ids: Vec<&str> = acc["accounts"].as_array().unwrap().iter().map(|a| a["account_id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["outlook:a@x.it", "gmail:g@x.it", "teams:3", "slack:T1"]);
        assert_eq!(acc["errors"], json!({}));
        assert!(acc["accounts"][0].get("native").is_none());

        // Outlook: unread list over two pages, duplicates dropped, the query the API gets
        log.lock().unwrap().clear();
        let l = tool(port, "email_list", json!({"account_id": "outlook:a@x.it", "limit": 2, "unread_only": true})).await;
        let items = l["data"]["items"].as_array().unwrap();
        assert_eq!(items.iter().map(|m| m["Id"].as_str().unwrap()).collect::<Vec<_>>(), ["m1", "m2"]);
        assert_eq!((l["data"]["stop_reason"].as_str(), l["data"]["has_more"].as_bool()), (Some("limit"), Some(true)));
        let asked = log.lock().unwrap().iter().find(|e| e.1.contains("/messages")).map(|e| e.1.clone()).unwrap();
        assert_eq!(asked, "/api/v2.0/me/MailFolders/Inbox/messages?$top=2&$select=Id,Subject,From,Sender,ToRecipients,CcRecipients,ReceivedDateTime,CreatedDateTime,BodyPreview,HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId,ParentFolderId&$filter=ReceivedDateTime%20ge%201900-01-01T00%3A00%3A00Z%20and%20IsRead%20eq%20false&$orderby=ReceivedDateTime%20desc");
        let l3 = tool(port, "email_list", json!({"account_id": "outlook:a@x.it", "limit": 3})).await;
        assert_eq!(l3["data"]["items"].as_array().unwrap().iter().map(|m| m["Id"].as_str().unwrap()).collect::<Vec<_>>(), ["m1", "m2", "m3"]);
        assert_eq!(l3["data"]["pages"], json!(2));
        // an id another system kept: a bare or differently written address names the one account of the tool's family
        for loose in ["a@x.it", "A@X.it", "OUTLOOK:a@x.it", "outlook:A@x.it"] {
            assert_eq!(tool(port, "email_list", json!({"account_id": loose, "limit": 1})).await["account_id"], json!("outlook:a@x.it"), "{loose}");
        }
        assert_eq!(tool(port, "chat_list", json!({"account_id": "teamsrelay:w@x.it"})).await["code"], json!("PROVIDER_ERROR"));
        for unknown in [("email_list", "w@x.it"), ("chat_list", "a@x.it"), ("chat_list", "nobody@x.it"), ("email_list", "gmail:a@x.it"), ("email_list", "a")] {
            assert_eq!(tool(port, unknown.0, json!({"account_id": unknown.1})).await["code"], json!("UNKNOWN_ACCOUNT"), "{unknown:?}");
        }
        // and a native Outlook id or well-known folder name; anything handle-shaped is still checked
        assert_eq!(tool(port, "email_read", json!({"account_id": "outlook:a@x.it", "message_id": "m1"})).await["data"]["Subject"], json!("Native"));
        let sent_items = tool(port, "email_list", json!({"account_id": "outlook:a@x.it", "folder_id": "sentitems"})).await;
        assert_eq!(sent_items["data"]["items"][0]["Id"], json!("s1"));
        assert_eq!(tool(port, "email_read", json!({"account_id": "outlook:a@x.it", "message_id": "m1.forged"})).await["code"], json!("INVALID_RESOURCE"));
        assert_eq!(tool(port, "email_read", json!({"account_id": "gmail:g@x.it", "message_id": "m1"})).await["code"], json!("INVALID_RESOURCE"));
        // a handle of this account works, the same handle on another account does not
        let read = tool(port, "email_read", json!({"account_id": "gmail:g@x.it", "message_id": items[0]["id"]})).await;
        assert_eq!(read["code"], json!("ACCOUNT_MISMATCH"));
        let sent = tool(port, "email_send", json!({"account_id": "outlook:a@x.it", "to": ["Bob <b@x.it>"], "subject": "s", "text": "<p>hi</p>"})).await;
        assert_eq!(sent["data"], json!({"status": "sent", "draft_id": null, "subject": "s", "to": ["b@x.it"], "cc": [], "bcc": []}));
        let mail = log.lock().unwrap().iter().find(|e| e.1.ends_with("/sendmail")).map(|e| e.2.clone()).unwrap();
        assert_eq!(mail, r#"{"Message":{"Subject":"s","Body":{"ContentType":"HTML","Content":"<p>hi</p>"},"ToRecipients":[{"EmailAddress":{"Address":"b@x.it","Name":"Bob"}}]},"SaveToSentItems":"true"}"#);
        // Outlook draft sent: the bodyless POST to /send
        let d = tool(port, "email_send", json!({"account_id": "outlook:a@x.it", "to": ["b@x.it"], "subject": "s", "text": "x", "draft": true})).await;
        let ds = tool(port, "email_draft_send", json!({"account_id": "outlook:a@x.it", "draft_id": d["data"]["draft_id"]})).await;
        assert_eq!((ds["data"]["status"].as_str(), ds["data"]["to"].clone()), (Some("sent"), json!(["b@x.it"])));

        // attachment download link: the link at once, the bytes on a GET without the MCP token
        let atts = tool(port, "email_attachments", json!({"account_id": "outlook:a@x.it", "message_id": items[0]["id"]})).await;
        let att = atts["data"]["items"][0]["id"].clone();
        log.lock().unwrap().clear();
        let dl = tool(port, "email_attachment", json!({"account_id": "outlook:a@x.it", "attachment_id": att, "download": true})).await;
        assert!(dl["data"]["expires_at"].as_str().unwrap().ends_with('Z'));
        let file_path = dl["data"]["download_url"].as_str().unwrap().strip_prefix("https://mcp.test/communication/files/").unwrap().to_string();
        assert!(!log.lock().unwrap().iter().any(|e| e.1.contains("/attachments")));
        let get = |path: String, headers: Vec<(&'static str, &'static str)>| async move {
            let mut req = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/files/{path}"));
            if !headers.iter().any(|(k, _)| *k == "Host") {
                req = req.header("Host", "mcp.test");
            }
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.send().await.unwrap()
        };
        let r = get(file_path.clone(), vec![]).await;
        assert_eq!(r.status().as_u16(), 200);
        let h = |k: &str| r.headers().get(k).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        assert_eq!(h("content-type"), "application/pdf");
        assert_eq!(h("content-disposition"), "attachment; filename=\"Q3 report.pdf\"; filename*=UTF-8''Q3%20report.pdf");
        assert_eq!(r.text().await.unwrap(), "hello");
        assert_eq!(get(file_path.clone(), vec![("Host", "evil")]).await.status().as_u16(), 421);
        assert_eq!(get(file_path.clone(), vec![("Origin", "https://x")]).await.status().as_u16(), 403);
        let (body, _) = file_path.split_once('.').unwrap();
        assert_eq!(get(format!("{body}.AAAA"), vec![]).await.status().as_u16(), 404);
        let old = communication_mcp::identity::ticket("email_attachment", &json!({"account_id": "outlook:a@x.it", "attachment_id": att}), 1);
        assert_eq!(get(old, vec![]).await.status().as_u16(), 410);
        let other = tool(port, "email_attachment", json!({"account_id": "gmail:g@x.it", "attachment_id": att, "download": true})).await;
        assert_eq!(other["code"], json!("ACCOUNT_MISMATCH"));

        // Gmail draft: the MIME the agent gets, handles in the answer
        let g = tool(port, "email_send", json!({"account_id": "gmail:g@x.it", "to": ["b@x.it"], "subject": "Caffè", "text": "x", "draft": true})).await;
        assert_eq!(g["data"]["status"], json!("draft"));
        assert!(g["data"]["draft_id"].as_str().unwrap().contains('.'));
        let write: Value = log.lock().unwrap().iter().rev().find(|e| e.1.ends_with("/gmail/write")).map(|e| serde_json::from_str(&e.2).unwrap()).unwrap();
        assert_eq!((write["account"].as_str(), write["confirm"].as_bool(), write["action"].as_str()), (Some("g@x.it"), Some(true), Some("draft")));

        // Teams through the relay
        let t = tool(port, "chat_list", json!({"account_id": "teams:3"})).await;
        assert_eq!(t["code"], json!("PROVIDER_ERROR"));
        // refresh asks the relay to open the chat in Teams first; Teams only
        let anna = communication_mcp::identity::handle("teams:3", "conversation", "Anna");
        for (refresh, relay_tool) in [(json!(false), "read_chat"), (json!(true), "refresh_chat")] {
            let r = tool(port, "chat_read", json!({"account_id": "teams:3", "conversation_id": anna, "refresh": refresh})).await;
            assert_eq!(r["data"]["messages"][0]["text"], json!(relay_tool));
        }
        assert_eq!(tool(port, "chat_read", json!({"account_id": "slack:T1", "conversation_id": communication_mcp::identity::handle("slack:T1", "conversation", "C1"), "refresh": true})).await["code"], json!("UNSUPPORTED_CAPABILITY"));
        // people tagged go to the relay as its mentions; Teams only
        let r = tool(port, "chat_send", json!({"account_id": "teams:3", "conversation_id": anna, "text": "@VERDI Carla look", "mentions": ["VERDI Carla"]})).await;
        assert_eq!(r["data"]["ok"], json!(true));
        let sent: Value = log.lock().unwrap().iter().rev().find(|e| e.2.contains("\"send_message\"")).map(|e| serde_json::from_str(&e.2).unwrap()).unwrap();
        assert_eq!(sent["params"]["arguments"]["mentions"], json!(["VERDI Carla"]));
        assert_eq!(
            tool(port, "chat_send", json!({"account_id": "slack:T1", "conversation_id": communication_mcp::identity::handle("slack:T1", "conversation", "C1"), "text": "@x", "mentions": ["x"]})).await["code"],
            json!("UNSUPPORTED_CAPABILITY")
        );

        // Slack: the conversations and users slackcli printed, the send it made
        log.lock().unwrap().clear();
        let s = tool(port, "chat_list", json!({"account_id": "slack:T1", "limit": 5})).await;
        assert_eq!(s["data"]["conversation_count"], json!(2));
        assert_eq!(s["data"]["users"], json!([{"id": "U1", "name": "me", "real_name": "Me", "email": "me@x.it"}]));
        assert!(s["data"].get("next_cursor").is_none());
        assert_eq!(s["data"]["conversations"][1]["topic"], json!("t"));
        // only the user's own conversations: never conversations.list, which pages the whole workspace before a DM
        let call = log.lock().unwrap().iter().find(|e| e.1 == "/api/users.conversations").map(|e| e.2.clone()).unwrap();
        assert_eq!(call, "token=xoxc-1&types=public_channel%2Cprivate_channel%2Cmpim%2Cim&limit=5&exclude_archived=false");
        assert!(!log.lock().unwrap().iter().any(|e| e.1 == "/api/conversations.list"));
        let dm = s["data"]["conversations"][0]["id"].as_str().unwrap().to_string();
        let conv = s["data"]["conversations"][1]["id"].as_str().unwrap().to_string();
        let posted = tool(port, "chat_send", json!({"account_id": "slack:T1", "conversation_id": conv, "text": "hello"})).await;
        assert_eq!((posted["data"]["channel_id"].as_str(), posted["data"]["ts"].as_str()), (Some("C1"), Some("1700000000.000100")));
        assert!(posted["data"].get("permalink").is_none());
        assert_eq!(tool(port, "chat_search", json!({"account_id": "slack:T1", "query": "none"})).await["data"], json!({}));
        assert_eq!(tool(port, "chat_send", json!({"account_id": "slack:T1", "conversation_id": conv, "text": "  "})).await["code"], json!("INVALID_INPUT"));
        // a search match names its conversation and thread: search, read, reply without listing
        let found = tool(port, "chat_search", json!({"account_id": "slack:T1", "query": "dm"})).await;
        let hit = &found["data"]["matches"][0];
        assert_eq!(hit["conversation_id"].as_str(), Some(dm.as_str()), "{found}");
        let thread = hit["thread_id"].as_str().expect("thread from the permalink").to_string();
        log.lock().unwrap().clear();
        let read = tool(port, "chat_read", json!({"account_id": "slack:T1", "conversation_id": hit["conversation_id"]})).await;
        assert_eq!((read["data"]["channel_id"].as_str(), read["data"]["messages"][0]["text"].as_str()), (Some("D1"), Some("hi")));
        let in_thread = tool(port, "chat_read", json!({"account_id": "slack:T1", "conversation_id": hit["conversation_id"], "thread_id": thread})).await;
        assert_eq!(in_thread["data"]["messages"][0]["text"], json!("parent"));
        let replied = tool(port, "chat_send", json!({"account_id": "slack:T1", "conversation_id": hit["conversation_id"], "thread_id": thread, "text": "ok"})).await;
        assert_eq!(replied["data"]["channel_id"], json!("D1"));
        let calls: Vec<(String, String)> = log.lock().unwrap().iter().map(|e| (e.1.clone(), e.2.clone())).collect();
        assert!(calls.contains(&("/api/conversations.history".into(), "token=xoxc-1&channel=D1&limit=20".into())), "{calls:?}");
        assert!(calls.contains(&("/api/conversations.replies".into(), "token=xoxc-1&channel=D1&ts=1700000000.000100&limit=20".into())), "{calls:?}");
        assert!(calls.contains(&("/api/chat.postMessage".into(), "token=xoxc-1&channel=D1&text=ok&thread_ts=1700000000.000100".into())), "{calls:?}");
        // the match's message handle fits its conversation handle; the fake has no auth.test, so the call stops there
        let mine = tool(port, "chat_delete", json!({"account_id": "slack:T1", "conversation_id": hit["conversation_id"], "message_id": hit["id"]})).await;
        assert_eq!(mine["code"], json!("PROVIDER_ERROR"));
        assert!(log.lock().unwrap().iter().any(|e| e.1 == "/api/auth.test"));
        // and a Slack id as Slack writes it, on the Slack account only
        log.lock().unwrap().clear();
        let raw = tool(port, "chat_read", json!({"account_id": "slack:T1", "conversation_id": "D0CANCD317C"})).await;
        assert_eq!(raw["data"]["channel_id"], json!("D0CANCD317C"));
        assert!(log.lock().unwrap().iter().any(|e| e.1 == "/api/conversations.history" && e.2.contains("channel=D0CANCD317C")));
        for refused in ["d0cancd317c", "U0BRH984NAC", "D1", "D0CANCD317C.x"] {
            assert_eq!(tool(port, "chat_read", json!({"account_id": "slack:T1", "conversation_id": refused})).await["code"], json!("INVALID_RESOURCE"), "{refused}");
        }
        assert_eq!(tool(port, "chat_read", json!({"account_id": "teams:3", "conversation_id": "D0CANCD317C"})).await["code"], json!("INVALID_RESOURCE"));

        // Everflow: everflow_routes_need_their_own_token
        let accounts = json!({"requests": [{"path": "/api/communication/accounts"}]}).to_string();
        let mcp_bearer = format!("Bearer {TOKEN}");
        let ef_bearer = format!("Bearer {EVERFLOW}");
        assert_eq!(everflow(port, "POST", "agent", &accounts, &[]).await.0, 200);
        for refused in [vec![("Authorization", "")], vec![("Authorization", mcp_bearer.as_str())], vec![("Authorization", "Bearer wrong")]] {
            assert_eq!(everflow(port, "POST", "agent", &accounts, &refused).await.0, 401, "{refused:?}");
        }
        assert_eq!(everflow(port, "POST", "agent", &accounts, &[("Origin", "https://x")]).await.0, 403);
        assert_eq!(everflow(port, "POST", "agent", &accounts, &[("Host", "evil")]).await.0, 421);
        assert_eq!(post(port, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#, &[("Authorization", ef_bearer.as_str())]).await.0, 401);
        assert_eq!(everflow(port, "POST", "nothing", &accounts, &[]).await.0, 404);
        assert_eq!(everflow(port, "GET", "agent", "", &[]).await.0, 405);

        // agent_passes_status_and_body_through
        log.lock().unwrap().clear();
        let two = json!({"requests": [
            {"path": "/api/communication/read", "body": {"account": "a@x.it", "path": "me/MailFolders/Inbox/messages"}},
            {"path": "/api/communication/read", "body": {"account": "down@x.it", "path": "me/MailFolders/Inbox/messages"}},
        ]});
        let (s, body) = everflow(port, "POST", "agent", &two.to_string(), &[]).await;
        assert_eq!(s, 200);
        let r: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(r, json!({"responses": [{"status": 200, "body": "{\"value\":[{\"Id\":\"m1\"}]}"}, {"status": 502, "body": "{\"errore\":\"Outlook non raggiungibile\"}"}]}));
        let asked: Vec<(String, String, String, String)> = log.lock().unwrap().clone();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].2, r#"{"account":"a@x.it","path":"me/MailFolders/Inbox/messages"}"#);
        assert!(asked[0].3.starts_with("authorization=Bearer key|"));
        let (s, body) = everflow(port, "POST", "agent", &accounts, &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["responses"][0]["status"].clone()), (200, json!(200)));

        // agent_refuses_paths_outside_the_list
        log.lock().unwrap().clear();
        for bad in ["/api/communication/gmail/write", "/api/x", "/api/communication/read?x=1", "/api/token-posta"] {
            let req = json!({"requests": [{"path": "/api/communication/accounts"}, {"path": bad, "body": {}}]});
            assert_eq!(everflow(port, "POST", "agent", &req.to_string(), &[]).await.0, 400, "{bad}");
        }
        assert_eq!(everflow(port, "POST", "agent", "{nope", &[]).await.0, 400);
        assert_eq!(everflow(port, "POST", "agent", r#"{"requests":[{"path":"/api/communication/read"}]}"#, &[]).await.0, 400);
        assert!(log.lock().unwrap().is_empty());

        // slack_reads_only_the_read_methods
        let req = |method: &str| json!({"workspace": "T1", "requests": [{"method": method, "params": {"channel": "C1", "limit": "200"}}]}).to_string();
        assert_eq!(everflow(port, "POST", "slack", &req("chat.postMessage"), &[]).await.0, 400);
        assert_eq!(everflow(port, "POST", "slack", &json!({"workspace": "../x", "requests": []}).to_string(), &[]).await.0, 400);
        assert_eq!(everflow(port, "POST", "slack", &json!({"workspace": "T1", "requests": [{"method": "auth.test", "params": {"n": 1}}]}).to_string(), &[]).await.0, 400);
        assert!(log.lock().unwrap().is_empty());
        let (s, body) = everflow(port, "POST", "slack", &req("conversations.history"), &[]).await;
        assert_eq!(s, 200);
        let r: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(r["responses"][0]["status"], json!(200));
        assert_eq!(serde_json::from_str::<Value>(r["responses"][0]["body"].as_str().unwrap()).unwrap()["messages"][0]["ts"], json!("1700000000.000100"));
        let sent = log.lock().unwrap().iter().find(|e| e.1 == "/api/conversations.history").cloned().unwrap();
        assert_eq!(sent.2, "token=xoxc-1&channel=C1&limit=200");
        assert!(sent.3.ends_with("|cookie=d=xoxd%2F1"), "{}", sent.3);
        assert_eq!(everflow(port, "POST", "slack", &json!({"workspace": "T9", "requests": [{"method": "auth.test"}]}).to_string(), &[]).await.0, 404);
        // a caller cannot bring its own Slack token next to the session's
        log.lock().unwrap().clear();
        for key in ["token", "TOKEN"] {
            let req = json!({"workspace": "T1", "requests": [{"method": "auth.test", "params": {key: "xoxp-other"}}]});
            assert_eq!(everflow(port, "POST", "slack", &req.to_string(), &[]).await.0, 400, "{key}");
        }
        assert!(log.lock().unwrap().is_empty());
        // a workspace in the store whose session cannot be used is said so, not reported as Slack unreachable
        let mut stored: Value = serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
        stored["workspaces"]["T2"] = json!({"workspace_id": "T2", "workspace_url": "https://two.slack.com/", "auth_type": "browser", "user_id": "U2"});
        std::fs::write(&store, stored.to_string()).unwrap();
        let (s, body) = everflow(port, "POST", "slack", &json!({"workspace": "T2", "requests": [{"method": "auth.test"}]}).to_string(), &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (502, Some("slack session unusable".into())));
        assert!(log.lock().unwrap().is_empty());

        // teams_forwards_saved
        let (s, body) = everflow(port, "GET", "teams?since=1790431000000&max_rows=10", "", &[]).await;
        assert_eq!(s, 200);
        let r: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(r["slots"], json!([{"slot": 3, "snapshot": "{\"state\":{}}"}, {"slot": 7, "error": "store-unavailable"}]));
        assert_eq!(r["asked"], json!("/api/everflow/saved?since=1790431000000&max_rows=10"));
        // an answer over the cap is its own status (507), never a cut body and never the 502 of a backend that is down
        let (s, body) = everflow(port, "GET", "teams?since=7&max_rows=10", "", &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (507, Some("answer too large".into())));
        assert_eq!(everflow(port, "GET", "teams?since=x&max_rows=10", "", &[]).await.0, 400);
        assert_eq!(everflow(port, "GET", "teams?since=1", "", &[]).await.0, 400);

        // teams_attachment_passes_bytes_through
        log.lock().unwrap().clear();
        let q = |index: &str| format!("slot=3&chat=BIANCHI%20Luca%20%26%20co&mid=1790432000000&index={index}");
        let (s, h, body) = attachment(port, &q("1"), &[]).await;
        assert_eq!((s, body.as_slice()), (200, IMAGE.as_slice()));
        assert_eq!(h, ["image/png", "inline; filename*=UTF-8''00112233445566aa.png", "private, max-age=86400"]);
        let asked = log.lock().unwrap().iter().find(|e| e.1.starts_with("/api/everflow/attachment")).cloned().unwrap();
        assert_eq!(asked.1, "/api/everflow/attachment?slot=3&chat=BIANCHI+Luca+%26+co&mid=1790432000000&index=1");
        assert!(asked.3.starts_with("authorization=Bearer relay-token|"));
        // the relay's refusals pass with their reason; a 200 without a type is octet-stream
        for (index, status, error) in [("2", 409, "runs without a browser"), ("3", 404, "index out of range"), ("4", 413, "too large")] {
            let (s, h, body) = attachment(port, &q(index), &[]).await;
            assert_eq!((s, h[0].as_str()), (status, "application/json"), "{index}");
            assert!(serde_json::from_slice::<Value>(&body).unwrap()["error"].as_str().unwrap().contains(error), "{index}");
        }
        let (s, body) = everflow(port, "GET", &format!("teams/attachment?{}", q("5")), "", &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (507, Some("answer too large".into())));
        let (s, h, body) = attachment(port, &q("6"), &[]).await;
        assert_eq!((s, h[0].as_str(), body.as_slice()), (200, "application/octet-stream", b"raw".as_slice()));
        let (s, body) = everflow(port, "GET", &format!("teams/attachment?{}", q("7")), "", &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (502, Some("teams relay HTTP 500".into())));
        // the guards of the other routes, and parameters checked before any call
        log.lock().unwrap().clear();
        assert_eq!(attachment(port, &q("1"), &[("Authorization", mcp_bearer.as_str())]).await.0, 401);
        assert_eq!(attachment(port, &q("1"), &[("Origin", "https://x")]).await.0, 403);
        assert_eq!(everflow(port, "POST", &format!("teams/attachment?{}", q("1")), "", &[]).await.0, 405);
        let long = "x".repeat(401);
        let emoji = "%F0%9F%98%80".repeat(201);
        for bad in [
            "chat=a&mid=1&index=1".to_string(),
            "slot=x&chat=a&mid=1&index=1".into(),
            "slot=-1&chat=a&mid=1&index=1".into(),
            "slot=99999999999&chat=a&mid=1&index=1".into(),
            "slot=3&mid=1&index=1".into(),
            "slot=3&chat=&mid=1&index=1".into(),
            format!("slot=3&chat={long}&mid=1&index=1"),
            format!("slot=3&chat={emoji}&mid=1&index=1"),
            "slot=3&chat=a&mid=abc&index=1".into(),
            "slot=3&chat=a&mid=123456789012345678901&index=1".into(),
            "slot=3&chat=a&mid=1&index=0".into(),
            "slot=3&chat=a&mid=1&index=1001".into(),
            "slot=3&chat=a&mid=1".into(),
        ] {
            assert_eq!(attachment(port, &bad, &[]).await.0, 400, "{bad}");
        }
        assert!(log.lock().unwrap().is_empty());

        // more_than_100_requests_is_refused
        let many: Vec<Value> = (0..101).map(|_| json!({"path": "/api/communication/accounts"})).collect();
        assert_eq!(everflow(port, "POST", "agent", &json!({"requests": many}).to_string(), &[]).await.0, 400);
        let many: Vec<Value> = (0..101).map(|_| json!({"method": "auth.test"})).collect();
        assert_eq!(everflow(port, "POST", "slack", &json!({"workspace": "T1", "requests": many}).to_string(), &[]).await.0, 400);

        // without EVERFLOW_TOKEN the routes are off; a relay that does not answer or refuses is 502 on teams
        let comm = Communication {
            outlook: Outlook::new(Tokens::new(&broker, "key").unwrap()),
            google: Some(Gateway::from_broker(&broker, "key").unwrap()),
            teams: Teams::from_env(Some("http://127.0.0.1:9/mcp"), Some("relay-token")).unwrap(),
            slack: None,
        };
        let off = start(Arc::new(App { comm, token: TOKEN.into(), hosts: ["mcp.test".to_string()].into(), everflow: None })).await;
        assert_eq!(everflow(off, "POST", "agent", &accounts, &[]).await.0, 404);
        let comm = Communication {
            outlook: Outlook::new(Tokens::new(&broker, "key").unwrap()),
            google: None,
            teams: Teams::from_env(Some("http://127.0.0.1:9/mcp"), Some("relay-token")).unwrap(),
            slack: None,
        };
        let down = start(Arc::new(App { comm, token: TOKEN.into(), hosts: ["mcp.test".to_string()].into(), everflow: Some(EVERFLOW.into()) })).await;
        let (s, body) = everflow(down, "GET", "teams?since=0&max_rows=10", "", &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (502, Some("teams relay unreachable".into())));
        let (s, _, body) = attachment(down, "slot=3&chat=a&mid=1&index=1", &[]).await;
        assert_eq!((s, serde_json::from_slice::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (502, Some("teams relay unreachable".into())));
        assert_eq!(everflow(down, "POST", "agent", &accounts, &[]).await.0, 404);
        assert_eq!(everflow(down, "POST", "slack", &req("auth.test"), &[]).await.0, 404);
        let comm = Communication {
            outlook: Outlook::new(Tokens::new(&broker, "key").unwrap()),
            google: None,
            teams: Teams::from_env(Some(&format!("http://127.0.0.1:{b}/mcp")), Some("not-the-relay-token")).unwrap(),
            slack: None,
        };
        let refused = start(Arc::new(App { comm, token: TOKEN.into(), hosts: ["mcp.test".to_string()].into(), everflow: Some(EVERFLOW.into()) })).await;
        let (s, body) = everflow(refused, "GET", "teams?since=0&max_rows=10", "", &[]).await;
        assert_eq!((s, serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (502, Some("teams relay HTTP 401".into())));
        let (s, _, body) = attachment(refused, "slot=3&chat=a&mid=1&index=1", &[]).await;
        assert_eq!((s, serde_json::from_slice::<Value>(&body).unwrap()["error"].as_str().map(String::from)), (502, Some("teams relay HTTP 401".into())));
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// Bytes that are not UTF-8: an attachment must come through as it is, never as text.
const IMAGE: [u8; 8] = [0x89, b'P', b'N', b'G', 0xff, 0x00, 0xfe, 0x80];

/// TeamsRelay GET /api/everflow/attachment: the answer chosen by index.
fn relay_attachment(pq: &str, credentials: &str) -> Response<Full<Bytes>> {
    let r = |status: u16| Response::builder().status(status);
    let j = |status: u16, error: &str| r(status).header("content-type", "application/json").body(Full::new(Bytes::from(json!({"error": error}).to_string()))).unwrap();
    if !credentials.starts_with("authorization=Bearer relay-token|") {
        return j(401, "Missing or wrong token");
    }
    let index = pq.split("index=").nth(1).unwrap_or("");
    match index {
        "1" => r(200)
            .header("content-type", "image/png")
            .header("content-disposition", "inline; filename*=UTF-8''00112233445566aa.png")
            .body(Full::new(Bytes::from_static(&IMAGE)))
            .unwrap(),
        "2" => j(409, "This Teams account runs without a browser: calls and downloads need Always on in Settings"),
        "3" => j(404, "index out of range: the message has 2 attachment(s)"),
        "4" => j(413, "21 MB, too large to carry here"),
        "5" => r(200).header("content-type", "application/pdf").body(Full::new(Bytes::from(vec![0u8; 20 * 1024 * 1024 + 1]))).unwrap(),
        "6" => r(200).body(Full::new(Bytes::from_static(b"raw"))).unwrap(),
        _ => j(500, "Internal error"),
    }
}

async fn start(app: Arc<App>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::task::spawn_local(serve(app, listener, std::future::pending()));
    port
}

/// A call to /everflow/<route> with EVERFLOW_TOKEN and the right Host; `headers` replace or add.
async fn everflow(port: u16, method: &str, route: &str, body: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let auth = format!("Bearer {EVERFLOW}");
    let mut all: Vec<(&str, &str)> = vec![("Host", "mcp.test"), ("Authorization", &auth), ("Content-Type", "application/json")];
    for (k, v) in headers {
        all.retain(|(x, _)| !x.eq_ignore_ascii_case(k));
        if !v.is_empty() {
            all.push((k, v));
        }
    }
    let url = format!("http://127.0.0.1:{port}/everflow/{route}");
    let mut req = if method == "GET" { reqwest::Client::new().get(url) } else { reqwest::Client::new().post(url).body(body.to_string()) };
    for (k, v) in all {
        req = req.header(k, v);
    }
    let r = req.send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

/// GET /everflow/teams/attachment with EVERFLOW_TOKEN: status, the headers named, the body as bytes.
async fn attachment(port: u16, query: &str, headers: &[(&str, &str)]) -> (u16, Vec<String>, Vec<u8>) {
    let auth = format!("Bearer {EVERFLOW}");
    let mut req = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/everflow/teams/attachment?{query}")).header("Host", "mcp.test");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
        req = req.header("Authorization", auth);
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let r = req.send().await.unwrap();
    let h = ["content-type", "content-disposition", "cache-control"].map(|k| r.headers().get(k).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default());
    (r.status().as_u16(), h.to_vec(), r.bytes().await.unwrap().to_vec())
}
