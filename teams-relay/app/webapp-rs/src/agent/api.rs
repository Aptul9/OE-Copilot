//! The Teams services a light account reads without a page (src/agent/light/api.ts), each with the access token of its
//! audience. Shapes as read on prod (2026-11-03), only the fields used.

use super::http::{Http, json, send};
use super::token::{CHATSVCAGG, IC3, PRESENCE, SPACES, Token};
use super::{Fail, R, random_uuid};
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;

const SERVER: &str = "https://teams.microsoft.com";

/// A chat of the aggregator: the raw object, read through accessors.
pub type CsaChat = Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub display_name: String,
    pub given_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersonPresence {
    pub availability: String,
    pub activity: String,
}

/// An item of the Activity feed, the id of its message in 48:notifications and whether it is read.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub activity: serde_json::Map<String, Value>,
    pub mid: String,
    pub read: bool,
}

/// What the chatsvc functions take (TeamsAuth of src/agent/teams/chatsvc.ts).
#[derive(Clone, Debug, Default)]
pub struct TeamsAuth {
    pub token: String,
    pub region: String,
    pub oid: Option<String>,
    pub upn: Option<String>,
    pub name: Option<String>,
}

/// The Activity feed is the conversation 48:notifications of chatsvc.
pub fn notifications_url(region: &str) -> String {
    format!(
        "{SERVER}/api/chatsvc/{region}/v1/users/ME/conversations/{}/messages?view=msnp24Equivalent&pageSize=50",
        crate::js::encode_uri_component("48:notifications")
    )
}

pub fn notifications_of(j: &Value) -> Vec<Notification> {
    let mut out = vec![];
    for m in j.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let props = m.get("properties");
        let raw = props.and_then(|p| p.get("activity"));
        let activity = match raw {
            Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok(),
            other => other.cloned(),
        };
        let isread = props.and_then(|p| p.get("isread"));
        if let Some(Value::Object(activity)) = activity {
            let read = !(isread == Some(&Value::Bool(false)) || isread == Some(&json!("false")));
            let mid = m.get("id").filter(|v| crate::js::truthy(v)).map(|v| crate::js::string(Some(v))).unwrap_or_default();
            out.push(Notification { activity, mid, read });
        }
    }
    out
}

pub struct Api {
    http: Http,
    token: Rc<Token>,
    endpoint: String,
    registered: Cell<bool>,
}

impl Api {
    pub fn new(token: Rc<Token>, http: Http) -> Self {
        Api { http, token, endpoint: random_uuid(), registered: Cell::new(false) }
    }

    fn region(&self) -> String {
        self.token.region()
    }

    async fn get(&self, scope: &str, url: &str, what: &str) -> R<Value> {
        let token = self.token.access_token(scope).await?;
        let r = send(&self.http, "GET", url, vec![("Accept", "application/json".into()), ("Authorization", format!("Bearer {token}"))], vec![], 20_000).await?;
        if !r.ok() {
            return Err(Fail::Status { status: r.status, what: what.into() });
        }
        json(&r)
    }

    async fn post(&self, scope: &str, url: &str, body: &Value, what: &str) -> R<Value> {
        let token = self.token.access_token(scope).await?;
        let headers = vec![("Accept", "application/json".into()), ("Content-Type", "application/json".into()), ("Authorization", format!("Bearer {token}"))];
        let r = send(&self.http, "POST", url, headers, crate::js::stringify(body).into_bytes(), 20_000).await?;
        if !r.ok() {
            return Err(Fail::Status { status: r.status, what: what.into() });
        }
        json(&r)
    }

    /// Every chat of the account, with its members and last message (the aggregator Teams web loads at start).
    pub async fn chats(&self) -> R<Vec<CsaChat>> {
        let url = format!("{SERVER}/api/csa/{}/api/v2/teams/users/me?isPrefetch=false&enableMembershipSummary=true&enableRC2Fetch=false", self.region());
        let j = self.get(CHATSVCAGG, &url, "chat list").await?;
        Ok(j.get("chats")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter(|c| c.get("id").is_some_and(Value::is_string)).cloned().collect())
            .unwrap_or_default())
    }

    /// Names of people (mri 8:orgid:...): of the organization, then of other organizations for the ones left.
    pub async fn people(&self, mris: &[String]) -> R<IndexMap<String, Person>> {
        let mut out = IndexMap::new();
        let wanted: Vec<String> = mris.iter().filter(|m| m.starts_with("8:orgid:")).cloned().collect();
        self.look_up("fetchShortProfile?isMailAddress=false&enableGuest=true&includeIBBarredUsers=true&skypeTeamsInfo=true", &wanted, &mut out).await?;
        let rest: Vec<String> = wanted.into_iter().filter(|m| !out.contains_key(m)).collect();
        if !rest.is_empty() {
            let _ = self.look_up("fetchFederated", &rest, &mut out).await;
        }
        Ok(out)
    }

    async fn look_up(&self, path: &str, wanted: &[String], out: &mut IndexMap<String, Person>) -> R<()> {
        for chunk in wanted.chunks(50) {
            let url = format!("{SERVER}/api/mt/{}/beta/users/{path}", self.region());
            let j = self.post(SPACES, &url, &json!(chunk), "people").await?;
            for p in j.get("value").and_then(Value::as_array).into_iter().flatten() {
                let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let mri = if !s("mri").is_empty() {
                    s("mri")
                } else if !s("objectId").is_empty() {
                    format!("8:orgid:{}", s("objectId"))
                } else {
                    String::new()
                };
                let name = s("displayName");
                if !mri.is_empty() && !name.is_empty() {
                    let given = if s("givenName").is_empty() { name.clone() } else { s("givenName") };
                    out.insert(mri, Person { display_name: name, given_name: given });
                }
            }
        }
        Ok(())
    }

    /// The people who wrote in a conversation, by the name their messages carry; the given name is its first word.
    pub async fn authors(&self, conversation: &str) -> R<IndexMap<String, Person>> {
        let url = format!(
            "{SERVER}/api/chatsvc/{}/v1/users/ME/conversations/{}/messages?view=msnp24Equivalent&pageSize=50",
            self.region(),
            crate::js::encode_uri_component(conversation)
        );
        let j = self.get(IC3, &url, "messages").await?;
        let mut out = IndexMap::new();
        let words = regex::Regex::new(r"[\s,]+").expect("fixed");
        for m in j.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let from = crate::js::string(m.get("from").filter(|v| !v.is_null()).or(Some(&json!(""))));
            let mri = from.split("/contacts/").last().unwrap_or("").to_string();
            let name = super::js_trim(m.get("imdisplayname").and_then(Value::as_str).unwrap_or("")).to_string();
            if mri.starts_with("8:") && !name.is_empty() && !out.contains_key(&mri) {
                let given = words.split(&name).next().filter(|g| !g.is_empty()).unwrap_or(&name).to_string();
                out.insert(mri, Person { display_name: name.clone(), given_name: given });
            }
        }
        Ok(out)
    }

    /// Availability and activity of each person.
    pub async fn presence(&self, mris: &[String]) -> R<IndexMap<String, PersonPresence>> {
        let mut out = IndexMap::new();
        for chunk in mris.chunks(100) {
            let body: Vec<Value> = chunk.iter().map(|m| json!({ "mri": m })).collect();
            let j = self.post(PRESENCE, "https://presence.teams.microsoft.com/v1/presence/getpresence/", &Value::Array(body), "presence").await?;
            for p in j.as_array().into_iter().flatten() {
                let Some(mri) = p.get("mri").and_then(Value::as_str).filter(|m| !m.is_empty()) else { continue };
                let pr = p.get("presence");
                let s = |k: &str| pr.and_then(|x| x.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
                out.insert(mri.to_string(), PersonPresence { availability: s("availability"), activity: s("activity") });
            }
        }
        Ok(out)
    }

    /// Keeps the account active in Teams: one endpoint registered, its activity reported.
    pub async fn keep_active(&self) -> R<()> {
        let token = self.token.access_token(PRESENCE).await?;
        let headers = || vec![("Content-Type", "application/json".to_string()), ("Authorization", format!("Bearer {token}"))];
        if !self.registered.get() {
            let body = json!({ "id": self.endpoint, "activity": "Available", "availability": "Available", "deviceType": "Web" });
            let r =
                send(&self.http, "PUT", "https://presence.teams.microsoft.com/v1/me/endpoints/", headers(), crate::js::stringify(&body).into_bytes(), 15_000)
                    .await?;
            if !r.ok() {
                return Err(Fail::Status { status: r.status, what: "presence endpoint".into() });
            }
            self.registered.set(true);
        }
        let body = json!({ "endpointId": self.endpoint, "isActive": true });
        let r = send(
            &self.http,
            "POST",
            "https://presence.teams.microsoft.com/v1/me/reportmyactivity/",
            headers(),
            crate::js::stringify(&body).into_bytes(),
            15_000,
        )
        .await?;
        if !r.ok() {
            self.registered.set(false);
            return Err(Fail::Status { status: r.status, what: "presence activity".into() });
        }
        Ok(())
    }

    /// The Activity feed, newest first as Teams keeps it.
    pub async fn notifications(&self) -> R<Vec<Notification>> {
        Ok(notifications_of(&self.get(IC3, &notifications_url(&self.region()), "activity").await?))
    }

    /// The last message of the chat with yourself, which the aggregator does not list: id, content, time.
    pub async fn notes_last(&self) -> R<Option<(String, String, String)>> {
        let url = format!(
            "{SERVER}/api/chatsvc/{}/v1/users/ME/conversations/{}/messages?view=msnp24Equivalent&pageSize=1",
            self.region(),
            crate::js::encode_uri_component("48:notes")
        );
        let j = self.get(IC3, &url, "notes").await?;
        let Some(m) = j.get("messages").and_then(|m| m.get(0)) else { return Ok(None) };
        if !m.get("id").is_some_and(crate::js::truthy) {
            return Ok(None);
        }
        let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let time = if s("originalarrivaltime").is_empty() { s("composetime") } else { s("originalarrivaltime") };
        Ok(Some((crate::js::string(m.get("id")), s("content"), time)))
    }

    /// What the chatsvc functions take: the IC3 token and who it is of; `me` (oid, name) of the agent when it knows it.
    pub async fn auth(&self, me: Option<(&str, &str)>) -> R<TeamsAuth> {
        let token = self.token.access_token(IC3).await?;
        let c = super::claims(&token);
        let s = |k: &str| c.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from);
        let claim = |k: &str| c.get(k).and_then(Value::as_str).map(String::from);
        Ok(TeamsAuth {
            region: self.region(),
            oid: me.map(|m| m.0.to_string()).or_else(|| claim("oid")),
            upn: s("upn").or_else(|| s("unique_name")),
            name: me.map(|m| m.1).filter(|n| !n.is_empty()).map(String::from).or_else(|| claim("name")),
            token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::Broker;
    use crate::agent::testing::{Net, run, temp_dir};

    fn mri(oid: &str) -> String {
        format!("8:orgid:{oid}")
    }

    /// An API whose token endpoint answers "t".
    fn api(net: &Net, broker: Option<Broker>) -> Api {
        let file = temp_dir().join("light-token.json");
        super::super::token::StoredToken { refresh_token: "rt".into(), client_id: "c".into(), tenant_id: "t".into(), region: "emea".into(), expires_on: 0 }
            .save(&file)
            .expect("token");
        Api::new(Rc::new(Token::new(file, broker, net.http())), net.http())
    }

    fn token_or(net: &Net, f: impl Fn(&crate::agent::testing::Seen) -> (u16, String) + 'static) {
        net.answer(move |s| {
            if s.url.contains("/oauth2/v2.0/token") || s.url.contains("/api/token-resource") {
                (200, r#"{"access_token":"t","token":"t"}"#.into())
            } else {
                f(s)
            }
        });
    }

    #[test]
    fn asks_the_federated_look_up_only_for_the_people_the_organization_did_not_name() {
        let net = Net::new();
        let a = api(&net, None);
        token_or(&net, |s| {
            let v = if s.url.contains("fetchShortProfile") {
                json!([{ "mri": mri("in"), "displayName": "Interno Anna", "givenName": "Anna" }])
            } else {
                json!([{ "objectId": "out", "displayName": "Esterno Luca", "givenName": "Luca" }])
            };
            (200, json!({ "value": v }).to_string())
        });
        let people = run(a.people(&[mri("in"), mri("out"), "4:+390000".into()])).expect("people");
        let bodies: Vec<(String, Value)> = net
            .requests()
            .iter()
            .filter(|s| s.url.contains("/beta/users/"))
            .map(|s| (s.url.split('/').next_back().unwrap_or("").split('?').next().unwrap_or("").to_string(), s.json()))
            .collect();
        assert_eq!(bodies, [("fetchShortProfile".into(), json!([mri("in"), mri("out")])), ("fetchFederated".into(), json!([mri("out")]))]);
        let got: Vec<(String, Person)> = people.into_iter().collect();
        assert_eq!(
            got,
            [
                (mri("in"), Person { display_name: "Interno Anna".into(), given_name: "Anna".into() }),
                (mri("out"), Person { display_name: "Esterno Luca".into(), given_name: "Luca".into() })
            ]
        );
    }

    #[test]
    fn reads_the_names_the_messages_of_a_chat_carry() {
        let net = Net::new();
        let a = api(&net, None);
        token_or(&net, |_| {
            let messages = json!([
                { "from": format!("https://emea.ng.msg.teams.microsoft.com/v1/users/ME/contacts/{}", mri("me")), "imdisplayname": "Me Myself" },
                { "from": format!("https://emea.ng.msg.teams.microsoft.com/v1/users/ME/contacts/{}", mri("ext")), "imdisplayname": "Rossi, Mario" },
                { "from": "", "imdisplayname": "System" },
            ]);
            (200, json!({ "messages": messages }).to_string())
        });
        let authors = run(a.authors("19:x@unq.gbl.spaces")).expect("authors");
        assert_eq!(authors.get(&mri("ext")), Some(&Person { display_name: "Rossi, Mario".into(), given_name: "Rossi".into() }));
        assert_eq!(authors.len(), 2);
    }

    #[test]
    fn registers_one_endpoint_and_reports_activity_again_after_a_forgotten_one() {
        let net = Net::new();
        let a = api(&net, None);
        let activity = Rc::new(Cell::new(0));
        let n = activity.clone();
        token_or(&net, move |s| {
            if s.url.contains("reportmyactivity") {
                n.set(n.get() + 1);
                return (if n.get() == 2 { 404 } else { 200 }, String::new());
            }
            (200, String::new())
        });
        run(a.keep_active()).expect("active");
        assert_eq!(run(a.keep_active()).unwrap_err().to_string(), "presence activity: HTTP 404");
        run(a.keep_active()).expect("active");
        let calls: Vec<(String, String)> = net
            .requests()
            .iter()
            .filter(|s| s.url.contains("presence.teams"))
            .map(|s| (url::Url::parse(&s.url).expect("url").path().to_string(), s.method.clone()))
            .collect();
        assert_eq!(
            calls,
            [
                ("/v1/me/endpoints/".into(), "PUT".into()),
                ("/v1/me/reportmyactivity/".into(), "POST".into()),
                ("/v1/me/reportmyactivity/".into(), "POST".into()),
                ("/v1/me/endpoints/".into(), "PUT".into()),
                ("/v1/me/reportmyactivity/".into(), "POST".into()),
            ]
        );
        let reqs = net.requests();
        let presence: Vec<_> = reqs.iter().filter(|s| s.url.contains("presence.teams")).collect();
        let endpoint = presence[0].json();
        assert_eq!((&endpoint["activity"], &endpoint["availability"], &endpoint["deviceType"]), (&json!("Available"), &json!("Available"), &json!("Web")));
        assert_eq!(presence[1].json(), json!({ "endpointId": endpoint["id"], "isActive": true }));
    }

    #[test]
    fn a_shared_account_outside_emea_reads_its_own_region() {
        let net = Net::new();
        let broker = Broker { url: "http://agent".into(), key: "k".into(), account: "a@b.example".into(), region: "amer".into() };
        let a = Api::new(Rc::new(Token::new("missing".into(), Some(broker), net.http())), net.http());
        token_or(&net, |_| (200, r#"{"messages":[]}"#.into()));
        run(a.notifications()).expect("feed");
        let urls: Vec<String> = net.requests().iter().map(|s| s.url.clone()).collect();
        assert_eq!(urls.last(), Some(&notifications_url("amer")));
        assert!(!urls.iter().any(|u| u.contains("/emea/")));
    }

    #[test]
    fn the_feed_and_its_read_flags() {
        let j = json!({ "messages": [
            { "id": "1795600000001", "properties": { "isread": "false", "activity": r#"{"activityId":1,"activityType":"call"}"# } },
            { "properties": { "isread": true, "activity": { "activityId": 2 } } },
            { "properties": { "activity": "broken" } },
            {},
        ]});
        let n = notifications_of(&j);
        assert_eq!(
            n.iter().map(|x| (x.activity["activityId"].clone(), x.mid.as_str(), x.read)).collect::<Vec<_>>(),
            [(json!(1), "1795600000001", false), (json!(2), "", true)]
        );
    }
}
