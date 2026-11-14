//! Firebase Cloud Messaging, HTTP v1 API, for the phones that run the TeamsRelay app (src/agent/push/fcm.ts): one data
//! message per phone, its content sealed with the key of that phone. The access token of the service account comes
//! from the OAuth 2.0 JWT bearer grant, kept until five minutes before its end, as google-auth-library keeps it.

use super::http::{Http, Request, reqwest_http};
use super::notify::error_text;
use futures_util::future::LocalBoxFuture;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// The service account key of the Firebase project.
#[derive(Clone, Debug)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
}

/// None when the file is missing; Err when it is not a service account key file.
pub fn load_service_account(file: &str) -> Result<Option<ServiceAccount>, String> {
    if file.is_empty() || !std::path::Path::new(file).exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let json: Value = serde_json::from_str(&text).map_err(|e| format!("{file}: not JSON ({})", error_text(&e.to_string())))?;
    let field = |k: &str| json.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    match (field("project_id"), field("client_email"), field("private_key")) {
        (Some(project_id), Some(client_email), Some(private_key)) => Ok(Some(ServiceAccount { project_id, client_email, private_key })),
        _ => Err(format!("{file}: not a service account key file of Google Cloud")),
    }
}

pub const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// google-auth-library asks for a new token this long before the end of the one it has.
const EAGER_REFRESH_MS: i64 = 5 * 60 * 1000;

/// The outcome of one message: taken, or refused with its HTTP status; gone: the token does not exist any more, the
/// phone is to be removed.
#[derive(Clone, Debug, PartialEq)]
pub enum FcmResult {
    Ok,
    Failed { status: u16, gone: bool, retry_after: Option<String> },
}

/// What the notifier sends phones through: the FcmSender, or a fake in the tests. Err: no answer, or the access token
/// refused.
pub trait FcmSend {
    fn send<'a>(&'a self, token: &'a str, data: &'a Map<String, Value>, ttl: i64, high: bool) -> LocalBoxFuture<'a, Result<FcmResult, String>>;
}

pub struct FcmSender {
    sa: ServiceAccount,
    http: Http,
    clock: Rc<dyn Fn() -> i64>,
    /// access token and when it ends, ms
    token: RefCell<Option<(String, i64)>>,
}

impl FcmSender {
    pub fn new(sa: ServiceAccount) -> Self {
        FcmSender { sa, http: reqwest_http(), clock: Rc::new(crate::web::now_ms), token: RefCell::new(None) }
    }

    /// Requests through `http`, time from `clock` (ms): the tests answer for Google.
    pub fn with(mut self, http: Http, clock: Rc<dyn Fn() -> i64>) -> Self {
        self.http = http;
        self.clock = clock;
        self
    }

    /// The assertion of the JWT bearer grant: RS256 of {iss, scope, aud, exp, iat}, an hour long.
    pub fn assertion(&self, now_s: i64) -> Result<String, String> {
        let der = super::vapid::pem_der(&self.sa.private_key)?;
        let pair = RsaKeyPair::from_pkcs8(&der).map_err(|e| format!("private key of the service account refused: {e}"))?;
        let head = crate::js::base64_url(br#"{"alg":"RS256"}"#);
        let claims = json!({ "iss": self.sa.client_email, "scope": SCOPE, "aud": TOKEN_URL, "exp": now_s + 3600, "iat": now_s });
        let input = format!("{head}.{}", crate::js::base64_url(crate::js::stringify(&claims).as_bytes()));
        let mut sig = vec![0; pair.public().modulus_len()];
        pair.sign(&RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), input.as_bytes(), &mut sig).map_err(|_| "signing failed".to_string())?;
        Ok(format!("{input}.{}", crate::js::base64_url(&sig)))
    }

    /// Kept until five minutes before its end; a refused key gives Google's reason.
    async fn access_token(&self) -> Result<String, String> {
        let now = (self.clock)();
        if let Some((t, ends)) = self.token.borrow().as_ref()
            && *ends > now + EAGER_REFRESH_MS
        {
            return Ok(t.clone());
        }
        let form = crate::js::search_params(&[("grant_type", GRANT_TYPE), ("assertion", &self.assertion(now.div_euclid(1000))?)]);
        let headers = vec![("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string())];
        let res = (self.http)(Request::post(TOKEN_URL, headers, form.into_bytes(), 15_000)).await?;
        let body: Value = serde_json::from_slice(&res.body).unwrap_or(Value::Null);
        if !res.ok() {
            return Err(match body.get("error").and_then(Value::as_str) {
                Some(e) => format!("{e}: {}", crate::js::string(body.get("error_description"))),
                None => format!("Request failed with status code {}", res.status),
            });
        }
        let token = body.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("FCM access token refused")?.to_string();
        let expires_in = body.get("expires_in").and_then(Value::as_f64).unwrap_or(0.0);
        *self.token.borrow_mut() = Some((token.clone(), now + (expires_in * 1000.0) as i64));
        Ok(token)
    }

    /// A data message to one phone: priority high wakes a phone in Doze at once; ttl, seconds the service keeps it.
    pub async fn send(&self, token: &str, data: &Map<String, Value>, ttl: i64, high: bool) -> Result<FcmResult, String> {
        let body = crate::js::stringify(
            &json!({ "message": { "token": token, "data": data, "android": { "priority": if high { "HIGH" } else { "NORMAL" }, "ttl": format!("{ttl}s") } } }),
        );
        let url = format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", self.sa.project_id);
        for attempt in 0.. {
            let headers = vec![
                ("Authorization".to_string(), format!("Bearer {}", self.access_token().await?)),
                ("Content-Type".to_string(), "application/json".to_string()),
            ];
            let res = (self.http)(Request::post(&url, headers, body.clone().into_bytes(), 15_000)).await?;
            if res.ok() {
                return Ok(FcmResult::Ok);
            }
            // an access token refused before its end: once more with a new one
            if res.status == 401 && attempt == 0 {
                *self.token.borrow_mut() = None;
                continue;
            }
            let detail: Value = serde_json::from_slice(&res.body).unwrap_or(json!({}));
            return Ok(FcmResult::Failed {
                status: res.status,
                gone: token_gone(res.status, &detail),
                retry_after: res.header("retry-after").map(String::from),
            });
        }
        unreachable!("the loop returns")
    }
}

impl FcmSend for FcmSender {
    fn send<'a>(&'a self, token: &'a str, data: &'a Map<String, Value>, ttl: i64, high: bool) -> LocalBoxFuture<'a, Result<FcmResult, String>> {
        Box::pin(FcmSender::send(self, token, data, ttl, high))
    }
}

/// UNREGISTERED (404): the app was removed or its data cleared; INVALID_ARGUMENT about the token (400): never valid;
/// SENDER_ID_MISMATCH (403): a token of another Firebase project.
fn token_gone(status: u16, d: &Value) -> bool {
    let error = d.get("error");
    let from_details = error
        .and_then(|e| e.get("details"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().filter_map(|x| x.get("errorCode")).find(|c| crate::js::truthy(c)))
        .and_then(Value::as_str);
    let code = from_details.or_else(|| error.and_then(|e| e.get("status")).and_then(Value::as_str));
    let message = error.and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("");
    status == 404
        || code == Some("UNREGISTERED")
        || code == Some("SENDER_ID_MISMATCH")
        || (status == 400 && regex::Regex::new(r"(?i)registration token").expect("fixed").is_match(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::http::Response;
    use std::cell::Cell;

    const PEM: &str = include_str!("testdata/rsa-test.pem");

    fn sa() -> ServiceAccount {
        ServiceAccount { project_id: "teamsrelay-test".into(), client_email: "relay@teamsrelay-test.iam.gserviceaccount.com".into(), private_key: PEM.into() }
    }

    type Sent = Rc<RefCell<Vec<(String, Option<String>, String)>>>;

    /// Google as the test answers it: the token endpoint, then FCM with the given answers in order (200 after them).
    /// status, body and headers of an answer of FCM
    type Answer = (u16, Value, Vec<(&'static str, &'static str)>);

    fn google(answers: Vec<Answer>) -> (Http, Sent) {
        let sent: Sent = Rc::new(RefCell::new(vec![]));
        let answers = Rc::new(RefCell::new(answers));
        let tokens = Rc::new(Cell::new(0));
        let log = sent.clone();
        let http: Http = Rc::new(move |r: Request| {
            log.borrow_mut().push((r.url.clone(), r.header("authorization").map(String::from), String::from_utf8(r.body.clone()).expect("utf8")));
            let res = if r.url == TOKEN_URL {
                tokens.set(tokens.get() + 1);
                Response {
                    status: 200,
                    headers: vec![],
                    body: json!({"access_token": format!("token-{}", tokens.get()), "expires_in": 3599, "token_type": "Bearer"}).to_string().into_bytes(),
                }
            } else {
                let mut a = answers.borrow_mut();
                let (status, body, headers) = if a.is_empty() { (200, json!({"name": "projects/teamsrelay-test/messages/1"}), vec![]) } else { a.remove(0) };
                Response { status, headers: headers.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: body.to_string().into_bytes() }
            };
            Box::pin(async move { Ok(res) })
        });
        (http, sent)
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().build().expect("runtime").block_on(f)
    }

    fn data() -> Map<String, Value> {
        json!({"v": "1"}).as_object().expect("object").clone()
    }

    #[test]
    fn signs_in_with_a_jwt_and_keeps_the_token_until_shortly_before_it_ends() {
        let now = Rc::new(Cell::new(1_790_000_000_000i64));
        let (http, sent) = google(vec![]);
        let clock = now.clone();
        let sender = FcmSender::new(sa()).with(http, Rc::new(move || clock.get()));
        run(async {
            sender.send("phone-token-1234567890", &data(), 60, true).await.expect("sent");
            sender.send("phone-token-1234567890", &data(), 60, true).await.expect("sent");
        });
        let token_calls: Vec<_> = sent.borrow().iter().filter(|s| s.0 == TOKEN_URL).cloned().collect();
        assert_eq!(token_calls.len(), 1);
        let form: Vec<(String, String)> = form_urlencoded::parse(token_calls[0].2.as_bytes()).into_owned().collect();
        assert_eq!(form[0], ("grant_type".into(), "urn:ietf:params:oauth:grant-type:jwt-bearer".into()));
        let parts: Vec<&str> = form[1].1.split('.').collect();
        assert_eq!(form[1].0, "assertion");
        let head: Value = serde_json::from_slice(&crate::js::base64_decode_lenient(parts[0])).expect("json");
        assert_eq!(head, json!({"alg": "RS256"}));
        let claims: Value = serde_json::from_slice(&crate::js::base64_decode_lenient(parts[1])).expect("json");
        assert_eq!(
            claims,
            json!({"iss": "relay@teamsrelay-test.iam.gserviceaccount.com", "scope": SCOPE, "aud": "https://oauth2.googleapis.com/token", "iat": 1_790_000_000, "exp": 1_790_003_600})
        );
        let der = crate::push::vapid::pem_der(PEM).expect("der");
        let pair = RsaKeyPair::from_pkcs8(&der).expect("key");
        ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, pair.public().as_ref())
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &crate::js::base64_decode_lenient(parts[2]))
            .expect("signature");
        now.set(1_790_000_000_000 + 3300 * 1000);
        run(async {
            sender.send("phone-token-1234567890", &data(), 60, true).await.expect("sent");
        });
        assert_eq!(sent.borrow().iter().filter(|s| s.0 == TOKEN_URL).count(), 2);
        let auths: Vec<_> = sent.borrow().iter().filter(|s| s.0.contains("fcm.googleapis.com")).map(|s| s.1.clone().unwrap_or_default()).collect();
        assert_eq!(auths, ["Bearer token-1", "Bearer token-1", "Bearer token-2"]);
    }

    #[test]
    fn sends_a_data_message_only() {
        let (http, sent) = google(vec![]);
        let sender = FcmSender::new(sa()).with(http, Rc::new(|| 0));
        let d = json!({"v": "1", "iv": "x", "ct": "y"}).as_object().expect("object").clone();
        assert_eq!(run(sender.send("phone-token-1234567890", &d, 86400, false)), Ok(FcmResult::Ok));
        let m = sent.borrow().iter().find(|s| s.0.contains("fcm")).cloned().expect("sent");
        assert_eq!(m.0, "https://fcm.googleapis.com/v1/projects/teamsrelay-test/messages:send");
        assert_eq!(m.2, r#"{"message":{"token":"phone-token-1234567890","data":{"v":"1","iv":"x","ct":"y"},"android":{"priority":"NORMAL","ttl":"86400s"}}}"#);
    }

    #[test]
    fn tells_a_gone_token_from_a_failure_worth_another_try() {
        let (http, _) = google(vec![
            (404, json!({"error": {"status": "NOT_FOUND", "details": [{"errorCode": "UNREGISTERED"}]}}), vec![]),
            (400, json!({"error": {"status": "INVALID_ARGUMENT", "message": "The registration token is not a valid FCM registration token"}}), vec![]),
            (429, json!({"error": {"status": "RESOURCE_EXHAUSTED"}}), vec![("Retry-After", "90")]),
            (503, json!({"error": {"status": "UNAVAILABLE"}}), vec![]),
            (403, json!({"error": {"status": "PERMISSION_DENIED", "details": [{"errorCode": "SENDER_ID_MISMATCH"}]}}), vec![]),
            (401, json!({"error": {"status": "UNAUTHENTICATED"}}), vec![]),
            (401, json!({"error": {"status": "UNAUTHENTICATED"}}), vec![]),
        ]);
        let sender = FcmSender::new(sa()).with(http, Rc::new(|| 0));
        let f = |status, gone, retry_after: Option<&str>| Ok(FcmResult::Failed { status, gone, retry_after: retry_after.map(String::from) });
        run(async {
            let d = data();
            let send = || sender.send("phone-token-1234567890", &d, 60, true);
            assert_eq!(send().await, f(404, true, None));
            assert_eq!(send().await, f(400, true, None));
            assert_eq!(send().await, f(429, false, Some("90")));
            assert_eq!(send().await, f(503, false, None));
            assert_eq!(send().await, f(403, true, None));
            // refused twice: once more only
            assert_eq!(send().await, f(401, false, None));
        });
    }

    #[test]
    fn asks_for_a_new_token_once_when_fcm_refuses_the_one_it_has() {
        let (http, sent) = google(vec![(401, json!({"error": {"status": "UNAUTHENTICATED"}}), vec![])]);
        let sender = FcmSender::new(sa()).with(http, Rc::new(|| 0));
        assert_eq!(run(sender.send("phone-token-1234567890", &data(), 60, true)), Ok(FcmResult::Ok));
        let auths: Vec<_> = sent.borrow().iter().filter(|s| s.0.contains("fcm")).map(|s| s.1.clone().unwrap_or_default()).collect();
        assert_eq!(auths, ["Bearer token-1", "Bearer token-2"]);
    }

    #[test]
    fn key_file() {
        let dir = std::env::temp_dir().join(format!("trweb-push-sa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("service-account.json");
        let f = file.to_str().expect("path");
        assert!(load_service_account(f).expect("none").is_none());
        assert!(load_service_account("").expect("none").is_none());
        std::fs::write(
            &file,
            json!({"type": "service_account", "project_id": "teamsrelay-test", "client_email": "a@b", "private_key": PEM, "private_key_id": "k1"}).to_string(),
        )
        .expect("write");
        let sa = load_service_account(f).expect("read").expect("some");
        assert_eq!((sa.project_id.as_str(), sa.client_email.as_str()), ("teamsrelay-test", "a@b"));
        std::fs::write(&file, r#"{"project_info":{}}"#).expect("write");
        assert_eq!(load_service_account(f).unwrap_err(), format!("{f}: not a service account key file of Google Cloud"));
        std::fs::write(&file, "").expect("write");
        assert!(load_service_account(f).unwrap_err().starts_with(&format!("{f}: not JSON (")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
