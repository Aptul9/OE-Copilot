//! Trouter, the push channel of Teams (cmd-trouter.js of Teams web): a socket.io 0.9 websocket that carries the
//! requests of the call controller to this client, and the registrar entry that tells Teams where this endpoint is.
//! Frames: "1::" connected, "2::" heartbeat (echoed), "5:id[+]::{name, args}" events (a + wants an ack "6:::id+[...]"),
//! "3:::{id, method, url, headers, body}" requests, answered with "3:::{id, status, headers, body}", "0::" close.

use super::{Fail, R};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// The version the Teams web build of the pinned artifacts reports (tc of the connect URL).
const CLIENT_VERSION: &str = "2026.40.01.1";

/// A request of the service to this client, with what its answer needs.
#[derive(Clone, Debug)]
pub struct Request {
    pub id: u64,
    pub method: String,
    pub url: String,
    pub headers: Map<String, Value>,
    /// the body, gunzipped where X-Microsoft-Skype-Content-Encoding says gzip
    pub body: String,
    received: Instant,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.as_str())
    }

    /// The last named segment of the path (progress, acceptance, rosterUpdate...).
    pub fn name(&self) -> &str {
        self.url.split('?').next().unwrap_or("").trim_end_matches('/').rsplit('/').next().unwrap_or("")
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// The body of a request: base64 of gzip when the service says so.
pub fn decode_body(body: &str, gzip: bool) -> R<String> {
    if !gzip {
        return Ok(body.to_string());
    }
    use base64::Engine;
    use std::io::Read;
    let bytes = base64::engine::general_purpose::STANDARD.decode(body.trim()).map_err(|e| Fail::Other(format!("trouter body: {e}")))?;
    let mut out = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).take(4 << 20).read_to_string(&mut out).map_err(|e| Fail::Other(format!("trouter body: {e}")))?;
    Ok(out)
}

/// The answer to a request: the request id, its status, MS-CV one level down, the trouter-request echoed, the time it
/// took (trouter-client cd, ms).
pub fn answer_frame(req: &Request, status: u16, body: &str) -> String {
    let mut headers = Map::new();
    if let Some(cv) = req.header("MS-CV") {
        headers.insert("MS-CV".into(), json!(format!("{cv}.0")));
    }
    if let Some(tr) = req.header("trouter-request") {
        headers.insert("trouter-request".into(), json!(tr));
    }
    headers.insert("trouter-client".into(), json!(json!({ "cd": req.received.elapsed().as_millis() as u64 }).to_string()));
    format!("3:::{}", json!({ "id": req.id, "status": status, "headers": headers, "body": body }))
}

/// What a frame of the socket is.
#[derive(Debug, PartialEq)]
pub enum Frame {
    Connect,
    Heartbeat,
    Close,
    /// an event: its id when it wants an ack, its name, its first argument
    Event {
        ack: Option<String>,
        name: String,
        arg: Value,
    },
    Request(Value),
    Ack,
    Other,
}

pub fn parse_frame(data: &str) -> Frame {
    let mut parts = data.splitn(4, ':');
    let kind = parts.next().unwrap_or("");
    let id = parts.next().unwrap_or("");
    let _endpoint = parts.next();
    let payload = parts.next().unwrap_or("");
    match kind {
        "0" => Frame::Close,
        "1" => Frame::Connect,
        "2" => Frame::Heartbeat,
        "3" => serde_json::from_str(payload).map(Frame::Request).unwrap_or(Frame::Other),
        "5" => {
            let v: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
            let name = v["name"].as_str().unwrap_or("").to_string();
            let arg = v["args"].as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
            Frame::Event { ack: id.strip_suffix('+').map(String::from), name, arg }
        }
        "6" => Frame::Ack,
        _ => Frame::Other,
    }
}

/// The connection to Trouter: what was learned when it connected, the frames to send.
pub struct Trouter {
    out: mpsc::UnboundedSender<String>,
    next: Arc<AtomicU64>,
    /// the base of the callback URLs of this endpoint (surl)
    pub surl: String,
    pub connected: Value,
}

impl Trouter {
    /// Connects as endpoint `endpoint` of the account of the Skype token; requests come out of the receiver, which ends
    /// when the connection does. trouter_url: calling_trouterUrl of the discovery (https://go-eu.trouter.../v4/a).
    pub async fn connect(trouter_url: &str, endpoint: &str, token: &str) -> R<(Trouter, mpsc::UnboundedReceiver<Request>)> {
        let base = url::Url::parse(trouter_url).map_err(|e| Fail::Other(e.to_string()))?;
        let host = base.host_str().ok_or_else(|| Fail::Other("trouter host".into()))?;
        let tc = json!({ "cv": CLIENT_VERSION, "ua": "SkypeSpaces", "hr": "", "v": CLIENT_VERSION }).to_string();
        let con_num = format!("{}_1", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let query = format!(
            "tc={}&timeout=40&epid={endpoint}&ccid=&cor_id=undefined&con_num={con_num}",
            tc.replace('"', "%22").replace(' ', "%20").replace('{', "%7B").replace('}', "%7D")
        );
        let ws_url = format!("wss://{host}/v4/c?{query}");
        let (socket, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(ws_url.as_str()))
            .await
            .map_err(|_| Fail::Other("trouter connect timed out".into()))?
            .map_err(|e| Fail::Other(format!("trouter connect: {e}")))?;
        let (mut write, mut read) = socket.split();
        // 1:: first, then the authentication, then trouter.connected
        let auth = json!({ "name": "user.authenticate", "args": [{ "headers": { "X-Skypetoken": token, "X-MS-Migration": "True" } }] });
        let mut connected = None;
        let deadline = Instant::now() + Duration::from_secs(20);
        while connected.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            let frame = tokio::time::timeout(left, read.next()).await.map_err(|_| Fail::Other("trouter did not connect in 20 s".into()))?;
            let Some(Ok(Message::Text(t))) = frame else {
                return Err(Fail::Other("trouter closed while connecting".into()));
            };
            match parse_frame(t.as_str()) {
                Frame::Connect => write.send(Message::Text(format!("5:::{auth}").into())).await.map_err(|e| Fail::Other(e.to_string()))?,
                Frame::Event { name, arg, .. } if name == "trouter.connected" => connected = Some(arg),
                Frame::Event { name, .. } if name == "trouter.unauthorized" || name == "trouter.error" => return Err(Fail::Other(format!("trouter: {name}"))),
                _ => {}
            }
        }
        let connected = connected.unwrap_or(Value::Null);
        let surl =
            connected["surl"].as_str().filter(|s| s.starts_with("https://")).ok_or_else(|| Fail::Other("trouter.connected without surl".into()))?.to_string();
        let (out, mut queue) = mpsc::unbounded_channel::<String>();
        let (tx, requests) = mpsc::unbounded_channel::<Request>();
        let next = Arc::new(AtomicU64::new(1));
        tokio::spawn(async move {
            while let Some(f) = queue.recv().await {
                let close = f == "0::";
                if write.send(Message::Text(f.into())).await.is_err() || close {
                    break;
                }
            }
            let _ = write.close().await;
        });
        let (o, n) = (out.clone(), next.clone());
        tokio::spawn(async move {
            while let Some(Ok(m)) = read.next().await {
                let Message::Text(t) = m else { continue };
                match parse_frame(t.as_str()) {
                    Frame::Heartbeat => {
                        let _ = o.send("2::".into());
                    }
                    Frame::Close => break,
                    Frame::Event { ack, name, arg } => {
                        if name == "trouter.message_loss" {
                            let id = n.fetch_add(1, Ordering::Relaxed);
                            let _ = o.send(format!("5:{id}+::{}", json!({ "name": "trouter.processed_message_loss", "args": [arg] })));
                        }
                        if let Some(id) = ack {
                            let _ = o.send(format!("6:::{id}+[]"));
                        }
                    }
                    Frame::Request(v) => {
                        let headers = v["headers"].as_object().cloned().unwrap_or_default();
                        let gzip = headers.iter().any(|(k, h)| {
                            k.eq_ignore_ascii_case("X-Microsoft-Skype-Content-Encoding") && h.as_str().is_some_and(|s| s.eq_ignore_ascii_case("gzip"))
                        });
                        let body = decode_body(v["body"].as_str().unwrap_or(""), gzip).unwrap_or_default();
                        let req = Request {
                            id: v["id"].as_u64().unwrap_or(0),
                            method: v["method"].as_str().unwrap_or("").into(),
                            url: v["url"].as_str().unwrap_or("").into(),
                            headers,
                            body,
                            received: Instant::now(),
                        };
                        if tx.send(req).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });
        // a ping keeps the connection and its idle timeout of 40 s alive
        let (o, n) = (out.clone(), next.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let id = n.fetch_add(1, Ordering::Relaxed);
                if o.send(format!("5:{id}+::{}", json!({ "name": "ping" }))).is_err() {
                    break;
                }
            }
        });
        Ok((Trouter { out, next, surl, connected }, requests))
    }

    /// The answer to a request of the service.
    pub fn answer(&self, req: &Request, status: u16, body: &str) {
        let _ = self.out.send(answer_frame(req, status, body));
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let _ = self.out.send(format!("5:{id}+::{}", json!({ "name": "ping" })));
    }

    pub fn close(&self) {
        let _ = self.out.send("0::".into());
    }

    pub fn alive(&self) -> bool {
        !self.out.is_closed()
    }
}

/// The registrar entry of this endpoint: where the call controller finds it (the Trouter URL), for an hour.
pub fn registration(endpoint: &str, surl: &str) -> Value {
    json!({
        "clientDescription": { "appId": "SkypeSpacesWeb", "aesKey": "", "languageId": "en-US", "platform": "Chrome", "templateKey": "SkypeSpacesWeb_2.6", "platformUIVersion": "" },
        "registrationId": endpoint,
        "nodeId": "",
        "transports": { "TROUTER": [{ "context": "", "path": surl, "ttl": 3600 }] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_of_the_socket() {
        assert_eq!(parse_frame("1::"), Frame::Connect);
        assert_eq!(parse_frame("2::"), Frame::Heartbeat);
        let e = parse_frame(r#"5:1::{"name":"trouter.connected","args":[{"surl":"https://x/v4/f/a/"}]}"#);
        assert_eq!(e, Frame::Event { ack: None, name: "trouter.connected".into(), arg: json!({ "surl": "https://x/v4/f/a/" }) });
        let e = parse_frame(r#"5:2+::{"name":"trouter.message_loss","args":[{"droppedIndicators":[]}]}"#);
        assert!(matches!(e, Frame::Event { ack: Some(ref id), .. } if id == "2"));
        assert_eq!(parse_frame(r#"6:::1+[]"#), Frame::Ack);
        assert!(matches!(parse_frame(r#"3:::{"id":7,"method":"POST","url":"/v4/f/a/x/call/progress/","headers":{},"body":"{}"}"#), Frame::Request(_)));
    }

    #[test]
    fn requests_answers_and_gzip_bodies() {
        use base64::Engine;
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(br#"{"callAcceptance":{}}"#).expect("gzip");
        let b64 = base64::engine::general_purpose::STANDARD.encode(gz.finish().expect("gzip"));
        assert_eq!(decode_body(&b64, true).expect("body"), r#"{"callAcceptance":{}}"#);
        let mut headers = Map::new();
        headers.insert("MS-CV".into(), json!("abc.1"));
        headers.insert("trouter-request".into(), json!(r#"{"id":"r"}"#));
        let req = Request {
            id: 9,
            method: "POST".into(),
            url: "/v4/f/a/callAgent/b/80ffddae/call/acceptance/".into(),
            headers,
            body: "{}".into(),
            received: Instant::now(),
        };
        assert_eq!(req.name(), "acceptance");
        let f: Value = serde_json::from_str(answer_frame(&req, 200, "x").strip_prefix("3:::").expect("frame")).expect("json");
        assert_eq!((f["id"].clone(), f["status"].clone(), f["body"].clone()), (json!(9), json!(200), json!("x")));
        assert_eq!(f["headers"]["MS-CV"], "abc.1.0");
        assert!(f["headers"]["trouter-client"].as_str().expect("cd").starts_with(r#"{"cd":"#));
        assert_eq!(registration("e", "https://s/")["transports"]["TROUTER"][0]["path"], "https://s/");
    }
}
