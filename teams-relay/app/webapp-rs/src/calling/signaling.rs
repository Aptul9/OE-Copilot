//! The signaling of a Teams call as the Teams web client does it (calling-pluginless.js, traced 2026-11-15): the
//! endpoint registered on Trouter, the relay of the tenant (ECS MDN_TRAP and its token service), a call created on the
//! conversation service with the offer, the call controller's callbacks on Trouter (progress, acceptance with the
//! answer, end), mute through updateEndpointState, hang-up through leave.

use super::trouter::{Request, Trouter, registration};
use super::{Fail, R};
use serde_json::{Map, Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

/// The client the calls say they come from (x-microsoft-skype-client of Teams web, the pinned calling build).
pub const CLIENT: &str = "SkypeSpaces/1415/26.1.0.0/os=windows; osVer=10; deviceType=computer; browser=chrome; browserVer=132.0.0.0/TsCallingVersion=2026.40.01.8/Ovb=1d7b6785828ef72ea1ae3da94082435119a87860";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";
/// Microsoft's Teams test-call service (config-prod teamsEchoBotMri): records what it hears, then plays it back
pub const ECHO_BOT: &str = "28:cf28171e-fcfd-47e4-a1d6-79460b0b3ca0";

pub fn uuid() -> String {
    crate::agent::random_uuid()
}

fn hex8() -> String {
    uuid().replace('-', "")[..8].to_string()
}

fn other(e: impl std::fmt::Display) -> Fail {
    Fail::Other(e.to_string())
}

/// The account a worker calls as: the Skype token and the service addresses of the discovery (authz of Teams).
#[derive(Clone, Debug)]
pub struct Account {
    pub token: String,
    pub oid: String,
    pub name: String,
    pub tenant: String,
    pub region: String,
    pub partition: String,
    pub gtms: Map<String, Value>,
}

impl Account {
    pub fn of(discovery: &Value, name: &str) -> R<Account> {
        let token = discovery["tokens"]["skypeToken"].as_str().filter(|t| t.len() > 100).ok_or_else(|| other("no Skype token"))?.to_string();
        let claims = crate::agent::claims(&token);
        let c = |k: &str| claims.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let oid = if c("oid").is_empty() { c("skypeid").rsplit(':').next().unwrap_or("").to_string() } else { c("oid") };
        let s = |k: &str| discovery[k].as_str().unwrap_or("").to_string();
        Ok(Account {
            token,
            oid,
            name: name.into(),
            tenant: c("tid"),
            region: s("region"),
            partition: s("partition"),
            gtms: discovery["regionGtms"].as_object().cloned().unwrap_or_default(),
        })
    }

    pub fn gtms(&self, key: &str) -> R<String> {
        self.gtms
            .get(key)
            .and_then(Value::as_str)
            .filter(|u| u.starts_with("https://"))
            .map(String::from)
            .ok_or_else(|| other(format!("no {key} in the discovery")))
    }

    pub fn mri(&self) -> String {
        format!("8:orgid:{}", self.oid)
    }
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP client")
}

async fn body_of(r: reqwest::Response) -> (u16, String) {
    let status = r.status().as_u16();
    (status, r.text().await.unwrap_or_default())
}

/// The TURN relay of the tenant and its credentials: fqdn, UDP port, TCP port, username, password.
pub struct Relay {
    pub fqdn: String,
    pub udp: u16,
    pub tcp: u16,
    pub username: String,
    pub credential: String,
}

/// The relay as Teams web finds it: the anonymous ECS settings (MDN_TRAP), then a token of its token service.
pub async fn relay(http: &reqwest::Client, a: &Account) -> R<Relay> {
    let ecs = format!(
        "https://config.teams.microsoft.com/config/v1/Skype/1415_1.0.0.0?tenantId={}&aadUserId={}&userId={}&region={}&agents=MDN_TRAP&ECSCanary=1",
        a.tenant, a.oid, a.oid, a.region
    );
    let r = http
        .get(&ecs)
        .header(
            "X-ECS-Client-Last-Telemetry-Events",
            "ecs_client_library_name=@1js/ecs-client,ecs_client_app_name=SkypeEcsConfigAdapter,ecs_client_version=1.2.0",
        )
        .send()
        .await
        .map_err(other)?;
    let (status, text) = body_of(r).await;
    if status != 200 {
        return Err(other(format!("ECS relay config {status}")));
    }
    let cfg: Value = serde_json::from_str(&text).map_err(other)?;
    let trap = &cfg["MDN_TRAP"];
    let token_url = trap["Service"]["tokenUrl"].as_str().filter(|u| u.starts_with("https://")).ok_or_else(|| other("no relay token service in ECS"))?;
    let turn = &trap["Relay"]["Turn"];
    let fqdn = turn["fqdns"][0].as_str().ok_or_else(|| other("no TURN relay in ECS"))?.to_string();
    let port = |k: &str, d: u16| turn[k].as_u64().map(|p| p as u16).unwrap_or(d);
    let r = http
        .get(token_url)
        .header("api-version", "2")
        .header("Content-Type", "application/json")
        .header("X-Skypetoken", &a.token)
        .header("X-MS-Migration", "True")
        .send()
        .await
        .map_err(other)?;
    let (status, text) = body_of(r).await;
    if status != 200 {
        return Err(other(format!("relay token {status}")));
    }
    let t: Value = serde_json::from_str(&text).map_err(other)?;
    let tok = &t["tokens"][0];
    Ok(Relay {
        fqdn,
        udp: port("udpPort", 3478),
        tcp: port("tcpPort", 443),
        username: tok["username"].as_str().unwrap_or("").into(),
        credential: tok["password"].as_str().unwrap_or("").into(),
    })
}

/// This client on Trouter and in the registrar: the callbacks of its calls arrive on `requests`.
pub struct Endpoint {
    pub http: reqwest::Client,
    pub account: Account,
    /// the registration on the registrar (epid of Trouter)
    pub registration: String,
    /// the endpoint of this client in its calls (endpointId of the participant)
    pub endpoint_id: String,
    /// the call agent of the callback URLs
    agent: String,
    pub trouter: Trouter,
    registrar: String,
    /// the Skype token now: a worker refreshes it while a call outlives it
    token: std::sync::RwLock<String>,
}

impl Endpoint {
    pub async fn start(account: Account) -> R<(Endpoint, mpsc::UnboundedReceiver<Request>)> {
        let http = http();
        let registration = uuid();
        let (trouter, requests) = Trouter::connect(&account.gtms("calling_trouterUrl")?, &registration, &account.token).await?;
        let registrar = account.gtms("calling_registrarUrl")?;
        let r = http
            .post(&registrar)
            .header("Accept", "application/json, text/javascript")
            .header("Content-Type", "application/json")
            .header("X-Skypetoken", &account.token)
            .header("X-MS-Migration", "True")
            .body(registration_body(&registration, &trouter.surl))
            .send()
            .await
            .map_err(other)?;
        if !r.status().is_success() {
            trouter.close();
            return Err(other(format!("registrar {}", r.status().as_u16())));
        }
        let token = std::sync::RwLock::new(account.token.clone());
        Ok((Endpoint { http, account, registration, endpoint_id: uuid(), agent: uuid(), trouter, registrar, token }, requests))
    }

    pub fn token(&self) -> String {
        self.token.read().map(|t| t.clone()).unwrap_or_default()
    }

    pub fn set_token(&self, token: &str) {
        if let Ok(mut t) = self.token.write() {
            *t = token.to_string();
        }
    }

    /// The registrar address, for the ownership marker of a worker.
    pub fn registrar(&self) -> &str {
        &self.registrar
    }

    /// A callback URL of this client for `kind` (call/progress, conversation/rosterUpdate...).
    pub fn link(&self, kind: &str) -> String {
        format!("{}callAgent/{}/{}/{kind}/", self.trouter.surl, self.agent, hex8())
    }

    /// Off the registrar and off Trouter.
    pub async fn stop(&self) {
        let _ = self
            .http
            .delete(format!("{}/{}", self.registrar, self.registration))
            .header("Accept", "application/json")
            .header("X-Skypetoken", self.token())
            .header("X-MS-Migration", "True")
            .send()
            .await;
        self.trouter.close();
    }

    /// The participant this client is in a call.
    pub fn me(&self, participant: &str) -> Value {
        json!({ "id": self.account.mri(), "displayName": self.account.name, "endpointId": self.endpoint_id, "participantId": participant, "languageId": "en-US" })
    }
}

fn registration_body(id: &str, surl: &str) -> String {
    registration(id, surl).to_string()
}

/// The links a call answers on, by what the far end sends there.
pub struct CallLinks {
    pub conversation: Map<String, Value>,
    pub invitation: Map<String, Value>,
    pub media_controller: Map<String, Value>,
}

impl CallLinks {
    pub fn new(e: &Endpoint) -> CallLinks {
        let map =
            |kinds: &[&str], prefix: &str| -> Map<String, Value> { kinds.iter().map(|k| (k.to_string(), json!(e.link(&format!("{prefix}/{k}"))))).collect() };
        CallLinks {
            conversation: map(
                &[
                    "conversationEnd",
                    "conversationUpdate",
                    "localParticipantUpdate",
                    "addParticipantSuccess",
                    "addParticipantFailure",
                    "addModalitySuccess",
                    "addModalityFailure",
                    "confirmUnmute",
                    "receiveMessage",
                ],
                "conversation",
            ),
            invitation: map(&["progress", "mediaAnswer", "acceptance", "redirection", "end"], "call"),
            media_controller: map(&["controlVideoStreaming", "csrcInfo"], "call"),
        }
    }

    /// The links the answer to an acceptance hands back (callAcceptanceAcknowledgement).
    pub fn acknowledgement(e: &Endpoint) -> Value {
        let links: Map<String, Value> =
            ["mediaRenegotiation", "transfer", "replacement", "balanceUpdate", "retargetCompletion", "controlVideoStreaming", "updateMediaDescriptions"]
                .iter()
                .map(|k| (k.to_string(), json!(e.link(&format!("call/{k}")))))
                .collect();
        json!({ "callAcceptanceAcknowledgement": { "links": links } })
    }
}

/// The body of a call created on the conversation service: the people called, the offer, the links of this client.
pub fn create_body(e: &Endpoint, links: &CallLinks, participant: &str, to: &[Value], offer: &str, media_leg: &str) -> Value {
    let roster = e.link("conversation/rosterUpdate");
    json!({
        "conversationRequest": {
            "conversationType": null,
            "subject": null,
            "suppressDialout": true,
            "roster": { "type": "Delta", "rosterUpdate": roster },
            "properties": { "allowConversationWithoutHost": true, "enableGroupCallEventMessages": true, "enableGroupCallUpgradeMessage": false, "enableGroupCallMeetupGeneration": false },
            "links": links.conversation,
        },
        "participants": { "from": e.me(participant), "to": to },
        "contentSharing": null,
        "capabilities": null,
        "endpointCapabilities": 1255,
        "clientEndpointCapabilities": null,
        "endpointMetadata": {},
        "groupContext": null,
        "groupChat": null,
        "meetingInfo": null,
        "meetingData": null,
        "callInvitation": {
            "callModalities": ["Audio"],
            "replaces": null,
            "transferor": null,
            "clientTransferContext": null,
            "customContext": null,
            "links": links.invitation,
            "clientContentForMediaController": links.media_controller,
            "pstnContent": { "emergencyCallCountry": "", "platformName": CLIENT, "publicApiCall": false },
            "emergencyContent": null,
            "mediaContent": {
                "blob": offer,
                "contentType": "application/sdp-ngc-1.0",
                "requiredFeatures": "nonByPass",
                "applyChannelParameters": { "multiChannelParameter": { "mids": ["*"], "mediaParameter": "{\"sendSideBWSeed\":{\"seedValueBitsPerSec\":600000}}" } },
                "mediaLegId": media_leg,
            },
            "voicemailSettings": {},
            "locationContent": null,
            "networkContent": null,
            "areaContent": null,
        }
    })
}

/// A call of this client on the conversation service: its chain id, the links the service gave it.
pub struct Conversation {
    pub chain: String,
    pub participant: String,
    pub links: Map<String, Value>,
    sequence: std::sync::atomic::AtomicU64,
}

impl Conversation {
    fn headers(&self, e: &Endpoint, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header("Content-Type", "application/json")
            .header("ms-teams-partition", &e.account.partition)
            .header("ms-teams-region", &e.account.region)
            .header("x-microsoft-skype-chain-id", &self.chain)
            .header("x-microsoft-skype-client", CLIENT)
            .header("x-microsoft-skype-message-id", uuid())
            .header("x-skypetoken", e.token())
            .header("User-Agent", USER_AGENT)
    }

    /// A call created with `to` (MRIs with their participant ids) and the offer: the conversation the service made.
    pub async fn create(e: &Endpoint, links: &CallLinks, to: &[Value], offer: &str) -> R<Conversation> {
        let mut c = Conversation { chain: uuid(), participant: uuid(), links: Map::new(), sequence: std::sync::atomic::AtomicU64::new(1) };
        let leg = uuid().replace('-', "").to_uppercase();
        let body = create_body(e, links, &c.participant, to, offer, &leg);
        let r = c.headers(e, e.http.post(e.account.gtms("calling_conversationServiceUrl")?)).body(body.to_string()).send().await.map_err(other)?;
        let (status, text) = body_of(r).await;
        if status != 201 && status != 200 {
            return Err(other(format!("call create {status}: {}", crate::agent::cut(&text, 200))));
        }
        let v: Value = serde_json::from_str(&text).map_err(other)?;
        c.links = v["links"].as_object().cloned().unwrap_or_default();
        Ok(c)
    }

    fn link(&self, k: &str) -> R<String> {
        self.links.get(k).and_then(Value::as_str).filter(|u| u.starts_with("https://")).map(String::from).ok_or_else(|| other(format!("no {k} link")))
    }

    async fn post(&self, e: &Endpoint, link: &str, body: Value) -> R<u16> {
        let r = self.headers(e, e.http.post(self.link(link)?)).body(body.to_string()).send().await.map_err(other)?;
        Ok(r.status().as_u16())
    }

    /// Muted or not, as the service shows it to the others (server mute).
    pub async fn set_muted(&self, e: &Endpoint, on: bool) -> R<()> {
        let n = self.sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 2;
        let status = self
            .post(
                e,
                "updateEndpointState",
                json!({ "from": e.me(&self.participant), "endpointState": { "state": { "isMuted": on }, "endpointStateSequenceNumber": n } }),
            )
            .await?;
        if (200..300).contains(&status) { Ok(()) } else { Err(other(format!("mute {status}"))) }
    }

    /// This client leaves the call (a local hang-up).
    pub async fn leave(&self, e: &Endpoint) -> R<()> {
        let body = json!({
            "participants": { "from": e.me(&self.participant) },
            "conversationTransactionEnd": { "reason": "noError", "code": 0, "phrase": "ConversationEndNoModalityConnected" },
            "callTransactionEnd": { "code": 0, "subCode": 0, "phrase": "CallEndReasonLocalUserInitiated", "resultCategories": ["Success"] },
        });
        let status = self.post(e, "leave", body).await?;
        if (200..300).contains(&status) { Ok(()) } else { Err(other(format!("leave {status}"))) }
    }
}

/// The links of the conversation of a call, for its keepalive: the roster link and those of `CallLinks`.
fn conversation_keepalive(e: &Endpoint, c: &Conversation, links: &CallLinks) -> Value {
    json!({ "participants": { "from": e.me(&c.participant) }, "roster": { "type": "Delta", "rosterUpdate": e.link("conversation/rosterUpdate") }, "links": links.conversation })
}

impl Conversation {
    /// A conversation joined from elsewhere (an incoming call): its links and the ids of this client in it.
    pub fn joined(chain: &str, participant: &str, links: Map<String, Value>) -> Conversation {
        Conversation { chain: chain.into(), participant: participant.into(), links, sequence: std::sync::atomic::AtomicU64::new(1) }
    }

    /// The keepalives of a call, every 35 minutes (the service drops a call after 2700 s without them): the call leg
    /// (POST callParticipantUpdate) and the conversation (POST notificationLinks).
    pub async fn keep_alive(&self, e: &Endpoint, call_leg: Option<&str>, links: &CallLinks) {
        if let Some(leg) = call_leg {
            let _ = self.headers(e, e.http.post(leg)).body(json!({ "callParticipantUpdate": {} }).to_string()).send().await;
        }
        if let Ok(url) = self.link("notificationLinks") {
            let _ = self.headers(e, e.http.post(url)).body(conversation_keepalive(e, self, links).to_string()).send().await;
        }
    }
}

/// A call that rings this client (event 107 of Trouter): who calls, the links of the call controller, the offer.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub call_id: String,
    pub caller: String,
    pub caller_name: String,
    /// this client's participant id in the call (to.participantId)
    pub participant: String,
    pub links: Map<String, Value>,
    pub offer: String,
    pub media_leg: String,
    pub conversation: String,
}

/// The payload of event 107: gp, base64 JSON or the object itself; cp, base64 deflate (decodeIncomingPayload).
pub fn notification_payload(body: &Value) -> R<Value> {
    use base64::Engine;
    use std::io::Read;
    if body["gp"].is_object() {
        return Ok(body["gp"].clone());
    }
    let (source, deflated) = match (body["gp"].as_str(), body["cp"].as_str()) {
        (Some(gp), _) => (gp, false),
        (None, Some(cp)) => (cp, true),
        _ => return Err(other("no call payload")),
    };
    if source.len() > 1_400_000 {
        return Err(other("call payload too large"));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(source).map_err(other)?;
    let text = if deflated {
        let mut s = String::new();
        flate2::read::ZlibDecoder::new(&bytes[..]).take(1 << 20).read_to_string(&mut s).map_err(other)?;
        s
    } else {
        String::from_utf8(bytes).map_err(other)?
    };
    serde_json::from_str(&text).map_err(other)
}

impl Incoming {
    /// The call a 107 event announces; None for another event.
    pub fn of(body: &Value) -> Option<Incoming> {
        if body["evt"].as_i64() != Some(107) {
            return None;
        }
        let p = notification_payload(body).ok()?;
        let n = &p["callNotification"];
        let s = |v: &Value| v.as_str().unwrap_or("").to_string();
        let links = n["links"].as_object().cloned()?;
        Some(Incoming {
            call_id: s(&p["debugContent"]["callId"]),
            caller: s(&n["from"]["id"]),
            caller_name: s(&n["from"]["displayName"]),
            participant: s(&n["to"]["participantId"]),
            links,
            offer: s(&n["mediaContent"]["blob"]),
            media_leg: s(&n["mediaContent"]["mediaLegId"]),
            conversation: s(&p["conversationInvitation"]["conversationController"]),
        })
    }

    fn link(&self, k: &str) -> R<String> {
        self.links.get(k).and_then(Value::as_str).filter(|u| u.starts_with("https://")).map(String::from).ok_or_else(|| other(format!("no {k} link")))
    }
}

/// An incoming call this client attached to: the links of the call controller for it, the conversation joined.
pub struct Attached {
    pub call: Map<String, Value>,
    pub conversation: Conversation,
}

impl Attached {
    fn link(&self, k: &str) -> R<String> {
        self.call.get(k).and_then(Value::as_str).filter(|u| u.starts_with("https://")).map(String::from).ok_or_else(|| other(format!("no {k} link")))
    }
}

/// Attaches to an incoming call and joins its conversation in the same request (additionalActions join), as Teams web
/// does on a 107: the call controller answers with the links of the call, the conversation with its own.
pub async fn attach(e: &Endpoint, inc: &Incoming, links: &CallLinks) -> R<Attached> {
    let conv = Conversation::joined(&inc.call_id, &inc.participant, Map::new());
    let body = json!({
        "attach": { "requireMediaContent": false, "links": { "end": e.link("call/end") }, "locationContent": null, "networkContent": null, "areaContent": null },
        "capabilities": null,
        "endpointCapabilities": 1255,
        "additionalActions": [{
            "input": {
                "capabilities": null,
                "endpointCapabilities": 1255,
                "conversationRequest": { "roster": { "type": "Delta", "rosterUpdate": e.link("conversation/rosterUpdate") }, "links": links.conversation },
                "endpointMetadata": {},
                "participants": { "from": e.me(&inc.participant) },
            },
            "name": "join",
            "url": inc.conversation,
            "waitForResponse": true,
        }],
    });
    let r = conv.headers(e, e.http.post(inc.link("attach")?)).body(body.to_string()).send().await.map_err(other)?;
    let (status, text) = body_of(r).await;
    if status != 200 {
        return Err(other(format!("attach {status}: {}", crate::agent::cut(&text, 200))));
    }
    let v: Value = serde_json::from_str(&text).map_err(other)?;
    let call = v["callInvitation"]["links"].as_object().cloned().unwrap_or_default();
    let joined = v["additionalActionResponses"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["output"]["links"].is_object()))
        .map(|x| x["output"]["links"].as_object().cloned().unwrap_or_default())
        .unwrap_or_default();
    Ok(Attached { call, conversation: Conversation::joined(&inc.call_id, &inc.participant, joined) })
}

impl Attached {
    /// The call rings here (progress ringing).
    pub async fn ringing(&self, e: &Endpoint) -> R<()> {
        let c = &self.conversation;
        let body = json!({ "callProgress": { "sender": e.me(&c.participant), "status": "ringing", "phrase": "ringing" } });
        let r = c.headers(e, e.http.post(self.link("progress")?)).body(body.to_string()).send().await.map_err(other)?;
        if r.status().is_success() { Ok(()) } else { Err(other(format!("ringing {}", r.status().as_u16()))) }
    }

    /// Accepts with this client's answer: the links of the active call (callLeg for the keepalive) come back.
    pub async fn accept(&self, e: &Endpoint, answer: &str, media_leg: &str) -> R<Map<String, Value>> {
        let c = &self.conversation;
        let ack = CallLinks::acknowledgement(e);
        let body = json!({
            "callAcceptance": {
                "acceptedBy": e.me(&c.participant),
                "acceptedCallModalities": ["Audio"],
                "capabilities": null,
                "endpointCapabilities": 1255,
                "links": ack["callAcceptanceAcknowledgement"]["links"],
                "clientContentForMediaController": { "controlVideoStreaming": e.link("call/controlVideoStreaming"), "csrcInfo": e.link("call/csrcInfo") },
                "mediaContent": {
                    "blob": answer,
                    "contentType": "application/sdp-ngc-1.0",
                    "applyChannelParameters": { "multiChannelParameter": { "mids": ["*"], "mediaParameter": "{\"sendSideBWSeed\":{\"seedValueBitsPerSec\":600000}}" } },
                    "mediaLegId": media_leg,
                },
                "pstnContent": { "emergencyCallCountry": "", "platformName": CLIENT, "publicApiCall": false },
                "callKeepAliveInterval": null,
            }
        });
        let r = c.headers(e, e.http.post(self.link("acceptance")?)).body(body.to_string()).send().await.map_err(other)?;
        let (status, text) = body_of(r).await;
        if status != 200 {
            return Err(other(format!("accept {status}: {}", crate::agent::cut(&text, 200))));
        }
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(v["callAcceptanceAcknowledgement"]["links"].as_object().cloned().unwrap_or_default())
    }

    /// Declines the call while it rings (DELETE of the call leg with LocalDecline, as Teams web rejects).
    pub async fn decline(&self, e: &Endpoint) -> R<()> {
        let c = &self.conversation;
        let body = json!({ "callEnd": { "code": 486, "subCode": 4508, "phrase": "LocalDecline", "resultCategories": ["Success"] } });
        let r = c.headers(e, e.http.delete(self.link("callLeg")?)).body(body.to_string()).send().await.map_err(other)?;
        if r.status().is_success() || r.status().as_u16() == 404 { Ok(()) } else { Err(other(format!("decline {}", r.status().as_u16()))) }
    }
}

/// What a callback of the call controller says about the call.
#[derive(Debug, PartialEq)]
pub enum Event {
    Progress(String),
    /// accepted, with the answer of the far end
    Accepted {
        by: String,
        answer: String,
    },
    Ended(String),
    Other(String),
}

pub fn event_of(req: &Request) -> Event {
    let v = req.json();
    match req.name() {
        "progress" => Event::Progress(v["callProgress"]["status"].as_str().unwrap_or("").into()),
        "acceptance" => {
            let a = &v["callAcceptance"];
            Event::Accepted { by: a["acceptedBy"]["id"].as_str().unwrap_or("").into(), answer: a["mediaContent"]["blob"].as_str().unwrap_or("").into() }
        }
        "end" | "conversationEnd" => {
            let reason = v["callEnd"]["phrase"].as_str().or(v["conversationEnd"]["reason"].as_str()).unwrap_or("ended");
            Event::Ended(reason.into())
        }
        other => Event::Other(other.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_of_a_discovery() {
        use base64::Engine;
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"oid":"51f9","tid":"3d43"}"#);
        let token = format!("eyJhbGciOiJub25lIn0.{claims}.{}", "s".repeat(120));
        let d = json!({ "tokens": { "skypeToken": token }, "region": "emea", "partition": "emea03", "regionGtms": { "calling_trouterUrl": "https://go-eu.trouter.teams.microsoft.com/v3/c" } });
        let a = Account::of(&d, "Me").expect("account");
        assert_eq!((a.oid.as_str(), a.tenant.as_str(), a.partition.as_str()), ("51f9", "3d43", "emea03"));
        assert_eq!(a.mri(), "8:orgid:51f9");
        assert!(a.gtms("calling_trouterUrl").is_ok() && a.gtms("calling_registrarUrl").is_err());
    }
}
