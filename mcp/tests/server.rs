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

type Log = Arc<Mutex<Vec<(String, String, String)>>>;

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
                        let body = String::from_utf8_lossy(&req.into_body().collect().await.unwrap().to_bytes()).to_string();
                        log.lock().unwrap().push((method.clone(), pq.clone(), body.clone()));
                        let (status, ctype, out) = answer(port, &method, &pq, &body);
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

fn answer(port: u16, method: &str, pq: &str, body: &str) -> (u16, &'static str, String) {
    let j = |v: Value| (200, "application/json", v.to_string());
    let path = pq.split('?').next().unwrap_or("");
    match (method, path) {
        ("GET", "/api/token-posta") => j(json!({"account": [{"upn": "a@x.it", "token": jwt("a@x.it")}]})),
        ("GET", "/api/communication/google/accounts") => j(json!({"accounts": [{"provider": "gmail", "account": "g@x.it", "status": "ok"}]})),
        ("POST", "/api/communication/gmail/write") => j(json!({"id": "d1", "message": {"id": "m1", "threadId": "t1"}})),
        ("GET", "/api/v2.0/me/MailFolders/Inbox/messages") if !pq.contains("skip") => j(json!({
            "value": [{"Id": "m1", "IsRead": true, "ConversationId": "c1"}, {"Id": "m2", "IsRead": false}],
            "@odata.nextLink": format!("http://127.0.0.1:{port}/api/v2.0/me/MailFolders('Inbox')/messages?$skip=2"),
        })),
        ("GET", "/api/v2.0/me/MailFolders('Inbox')/messages") => j(json!({"value": [{"Id": "m2", "IsRead": false}, {"Id": "m3", "IsRead": false}]})),
        ("POST", "/api/v2.0/me/sendmail") => (202, "application/json", String::new()),
        ("POST", "/api/conversations.list") => j(
            json!({"ok": true, "channels": [{"id": "D1", "is_im": true, "user": "U1"}, {"id": "C1", "name": "general", "topic": {"value": "t"}}], "response_metadata": {"next_cursor": ""}}),
        ),
        ("POST", "/api/users.info") => j(json!({"ok": true, "user": {"id": "U1", "name": "me", "real_name": "Me", "profile": {"email": "me@x.it"}}})),
        ("POST", "/api/chat.postMessage") => j(json!({"ok": true, "ts": "1700000000.000100", "channel": "C1"})),
        ("POST", "/api/chat.getPermalink") => j(json!({"ok": false, "error": "no_permission"})),
        ("POST", "/api/search.messages") => j(json!({"ok": true, "messages": {"matches": [], "total": 0}})),
        ("POST", "/mcp") => {
            let req: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let id = req["id"].clone();
            let result = match req["params"]["name"].as_str() {
                Some("list_accounts") => json!({"content": [{"type": "text", "text": "{\"accounts\":[{\"slot\":3,\"name\":\"Work\"}]}"}]}),
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
        }
        let broker = format!("http://127.0.0.1:{b}/api/token-posta");
        let comm = Communication {
            outlook: Outlook::new(Tokens::new(&broker, "key").unwrap()),
            google: Some(Gateway::from_broker(&broker, "key").unwrap()),
            teams: Teams::from_env(Some(&format!("http://127.0.0.1:{b}/mcp")), Some("relay-token")).unwrap(),
            slack: Some(Slack::new(store.to_str().unwrap())),
        };
        let app = Arc::new(App { comm, token: TOKEN.into(), hosts: ["mcp.test".to_string()].into() });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::task::spawn_local(serve(app, listener, std::future::pending()));

        // the wire, as the TypeScript server answered
        let (s, t, body) = post(port, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#, &[]).await;
        assert_eq!((s, t.as_str()), (200, "text/event-stream"));
        assert!(body.starts_with("event: message\ndata: {\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{\"tools\":{\"listChanged\":true}},\"serverInfo\":{\"name\":\"communication\""));
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
        // a handle of this account works, the same handle on another account does not
        let read = tool(port, "email_read", json!({"account_id": "gmail:g@x.it", "message_id": items[0]["id"]})).await;
        assert_eq!(read["code"], json!("ACCOUNT_MISMATCH"));
        let sent = tool(port, "email_send", json!({"account_id": "outlook:a@x.it", "to": ["Bob <b@x.it>"], "subject": "s", "text": "<p>hi</p>"})).await;
        assert_eq!(sent["data"], json!({"status": "sent", "draft_id": null, "subject": "s", "to": ["b@x.it"], "cc": [], "bcc": []}));
        let mail = log.lock().unwrap().iter().find(|e| e.1.ends_with("/sendmail")).map(|e| e.2.clone()).unwrap();
        assert_eq!(mail, r#"{"Message":{"Subject":"s","Body":{"ContentType":"HTML","Content":"<p>hi</p>"},"ToRecipients":[{"EmailAddress":{"Address":"b@x.it","Name":"Bob"}}]},"SaveToSentItems":"true"}"#);

        // Gmail draft: the MIME the agent gets, handles in the answer
        let g = tool(port, "email_send", json!({"account_id": "gmail:g@x.it", "to": ["b@x.it"], "subject": "Caffè", "text": "x", "draft": true})).await;
        assert_eq!(g["data"]["status"], json!("draft"));
        assert!(g["data"]["draft_id"].as_str().unwrap().contains('.'));
        let write: Value = log.lock().unwrap().iter().rev().find(|e| e.1.ends_with("/gmail/write")).map(|e| serde_json::from_str(&e.2).unwrap()).unwrap();
        assert_eq!((write["account"].as_str(), write["confirm"].as_bool(), write["action"].as_str()), (Some("g@x.it"), Some(true), Some("draft")));

        // Teams through the relay
        let t = tool(port, "chat_list", json!({"account_id": "teams:3"})).await;
        assert_eq!(t["code"], json!("PROVIDER_ERROR"));

        // Slack: the conversations and users slackcli printed, the send it made
        log.lock().unwrap().clear();
        let s = tool(port, "chat_list", json!({"account_id": "slack:T1", "limit": 5})).await;
        assert_eq!(s["data"]["conversation_count"], json!(2));
        assert_eq!(s["data"]["users"], json!([{"id": "U1", "name": "me", "real_name": "Me", "email": "me@x.it"}]));
        assert!(s["data"].get("next_cursor").is_none());
        assert_eq!(s["data"]["conversations"][1]["topic"], json!("t"));
        let call = log.lock().unwrap().iter().find(|e| e.1 == "/api/conversations.list").map(|e| e.2.clone()).unwrap();
        assert_eq!(call, "token=xoxc-1&types=public_channel%2Cprivate_channel%2Cmpim%2Cim&limit=5&exclude_archived=false");
        let conv = s["data"]["conversations"][1]["id"].as_str().unwrap().to_string();
        let posted = tool(port, "chat_send", json!({"account_id": "slack:T1", "conversation_id": conv, "text": "hello"})).await;
        assert_eq!((posted["data"]["channel_id"].as_str(), posted["data"]["ts"].as_str()), (Some("C1"), Some("1700000000.000100")));
        assert!(posted["data"].get("permalink").is_none());
        assert_eq!(tool(port, "chat_search", json!({"account_id": "slack:T1", "query": "none"})).await["data"], json!({}));
        assert_eq!(tool(port, "chat_send", json!({"account_id": "slack:T1", "conversation_id": conv, "text": "  "})).await["code"], json!("INVALID_INPUT"));
        let _ = std::fs::remove_dir_all(&dir);
    });
}
