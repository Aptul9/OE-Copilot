//! Web Push as the npm package web-push 3.6.7 sends it (sendNotification): the payload encrypted for the device
//! (RFC 8291, aes128gcm of RFC 8188, records of 4096 bytes, no padding), the VAPID JWT of RFC 8292 in
//! "Authorization: vapid t=<jwt>, k=<public key>", TTL and Urgency.

use super::http::{Http, Request};
use super::vapid::VapidKeys;
use ring::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{ECDH_P256, EphemeralPrivateKey, UnparsedPublicKey, agree_ephemeral};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};

/// The options of a send, as the notifier gives them to web-push.
#[derive(Clone, Debug)]
pub struct Options {
    pub subject: String,
    pub vapid: VapidKeys,
    pub ttl: i64,
    pub urgency: String,
    pub timeout_ms: u64,
}

/// Why a send failed, in the shapes the notifier tells apart.
#[derive(Clone, Debug, PartialEq)]
pub enum SendError {
    /// refused before sending (the subscription or the keys): the same every time
    Refused(String),
    /// the push service did not answer (network error, timeout)
    NoAnswer(String),
    /// the push service answered outside 2xx
    Status { status: u16, retry_after: Option<String> },
}

impl SendError {
    pub fn text(&self) -> String {
        match self {
            SendError::Refused(t) | SendError::NoAnswer(t) => t.clone(),
            SendError::Status { .. } => "Received unexpected response code".into(),
        }
    }
}

const RS: usize = 4096;
const TAG: usize = 16;

fn hmac256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data);
    tag.as_ref().try_into().expect("32 bytes")
}

/// aes128gcm of `plain` once the ECDH secret is known: header (salt, rs, keyid = as_public) and the records.
pub fn encrypt_with(ecdh_secret: &[u8], ua_public: &[u8], as_public: &[u8], auth: &[u8], salt: &[u8; 16], plain: &[u8]) -> Result<Vec<u8>, String> {
    let prk_key = hmac256(auth, ecdh_secret);
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    key_info.push(1);
    let ikm = hmac256(&prk_key, &key_info);
    let prk = hmac256(salt, &ikm);
    let cek = &hmac256(&prk, b"Content-Encoding: aes128gcm\0\x01")[..16];
    let nonce_base: [u8; 12] = hmac256(&prk, b"Content-Encoding: nonce\0\x01")[..12].try_into().expect("12 bytes");
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, cek).map_err(|_| "bad key".to_string())?);
    let mut out = salt.to_vec();
    out.extend_from_slice(&(RS as u32).to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(as_public);
    // records of rs bytes: the data, the delimiter (2 on the last, 1 before) and the tag
    let per = RS - TAG - 1;
    let mut start = 0;
    let mut counter: u64 = 0;
    loop {
        let end = (start + per).min(plain.len());
        let last = start + per >= plain.len();
        let mut record = plain[start..end].to_vec();
        record.push(if last { 2 } else { 1 });
        let mut nonce = nonce_base;
        for (i, b) in counter.to_be_bytes()[2..].iter().enumerate() {
            nonce[6 + i] ^= b;
        }
        key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut record).map_err(|_| "encryption failed".to_string())?;
        out.extend_from_slice(&record);
        if last {
            return Ok(out);
        }
        start = end;
        counter += 1;
    }
}

/// The payload encrypted for the device of `p256dh` and `auth` with a new key pair and salt of this send.
pub fn encrypt(p256dh: &[u8], auth: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    let rng = SystemRandom::new();
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| "no random bytes".to_string())?;
    let private = EphemeralPrivateKey::generate(&ECDH_P256, &rng).map_err(|_| "no key pair".to_string())?;
    let public = private.compute_public_key().map_err(|_| "no key pair".to_string())?;
    let as_public = public.as_ref().to_vec();
    agree_ephemeral(private, &UnparsedPublicKey::new(&ECDH_P256, p256dh), |secret| encrypt_with(secret, p256dh, &as_public, auth, &salt, plain))
        .map_err(|_| "The subscription p256dh value is not a P-256 public key.".to_string())?
}

/// The VAPID JWT: ES256 of {aud, exp, sub}, exp 12 hours from `now_s`.
pub fn vapid_jwt(audience: &str, subject: &str, keys: &VapidKeys, now_s: i64) -> Result<String, String> {
    let pair = super::vapid::key_pair(keys)?;
    let head = crate::js::base64_url(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = crate::js::base64_url(crate::js::stringify(&json!({ "aud": audience, "exp": now_s + 12 * 3600, "sub": subject })).as_bytes());
    let input = format!("{head}.{claims}");
    let sig = pair.sign(&SystemRandom::new(), input.as_bytes()).map_err(|_| "signing failed".to_string())?;
    Ok(format!("{input}.{}", crate::js::base64_url(sig.as_ref())))
}

fn key_of<'a>(sub: &'a Value, k: &str) -> Option<&'a str> {
    sub.get("keys")?.get(k)?.as_str().filter(|s| !s.is_empty())
}

/// The request of a send, or why web-push refuses it before sending.
pub fn request_for(sub: &Value, payload: &str, o: &Options, now_s: i64) -> Result<Request, String> {
    let endpoint = sub.get("endpoint").filter(|e| crate::js::truthy(e)).ok_or("You must pass in a subscription with at least an endpoint.")?;
    let endpoint = endpoint.as_str().filter(|e| !e.is_empty()).ok_or("The subscription endpoint must be a string with a valid URL.")?;
    let mut headers = vec![("TTL".to_string(), o.ttl.to_string())];
    let mut body = vec![];
    if !payload.is_empty() {
        let (Some(p256dh), Some(auth)) = (key_of(sub, "p256dh"), key_of(sub, "auth")) else {
            return Err("To send a message with a payload, the subscription must have 'auth' and 'p256dh' keys.".into());
        };
        let p256dh = crate::js::base64_decode_lenient(p256dh);
        if p256dh.len() != 65 {
            return Err("The subscription p256dh value should be 65 bytes long.".into());
        }
        let auth = crate::js::base64_decode_lenient(auth);
        if auth.len() < 16 {
            return Err("The subscription auth key should be at least 16 bytes long".into());
        }
        body = encrypt(&p256dh, &auth, payload.as_bytes())?;
        headers.push(("Content-Length".into(), body.len().to_string()));
        headers.push(("Content-Type".into(), "application/octet-stream".into()));
        headers.push(("Content-Encoding".into(), "aes128gcm".into()));
    } else {
        headers.push(("Content-Length".into(), "0".into()));
    }
    if endpoint.starts_with("https://android.googleapis.com/gcm/send") {
        eprintln!("Attempt to send push notification to GCM endpoint, but no GCM key is defined. Please use setGCMApiKey() or add 'gcmAPIKey' as an option.");
    } else {
        let url = url::Url::parse(endpoint).map_err(|_| format!("VAPID audience is not a url. {endpoint}"))?;
        let audience = url.origin().ascii_serialization();
        let jwt = vapid_jwt(&audience, &o.subject, &o.vapid, now_s)?;
        headers.push(("Authorization".into(), format!("vapid t={jwt}, k={}", o.vapid.public_key)));
    }
    if !["very-low", "low", "normal", "high"].contains(&o.urgency.as_str()) {
        return Err("Unsupported urgency specified.".into());
    }
    headers.push(("Urgency".into(), o.urgency.clone()));
    Ok(Request::post(endpoint, headers, body, o.timeout_ms))
}

/// sendNotification: Ok once the push service took it (2xx).
pub async fn send_notification(http: &Http, sub: &Value, payload: &str, o: &Options) -> Result<(), SendError> {
    let req = request_for(sub, payload, o, crate::web::now_s()).map_err(SendError::Refused)?;
    let res = http(req).await.map_err(SendError::NoAnswer)?;
    if res.ok() {
        return Ok(());
    }
    Err(SendError::Status { status: res.status, retry_after: res.header("retry-after").map(String::from) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as Verify};

    fn b64(s: &str) -> Vec<u8> {
        crate::js::base64_decode_lenient(&s.replace(' ', ""))
    }

    #[test]
    fn rfc8291_example() {
        let ua_public = b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcx aOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let as_public = b64("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIg Dll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
        let secret = b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().expect("16");
        let out = encrypt_with(&secret, &ua_public, &as_public, &auth, &salt, b"When I grow up, I want to be a watermelon").expect("encrypted");
        let expected = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
        );
        // 86 of header, 41 + delimiter + tag (Appendix A); the Content-Length 145 of section 5 is a typo of the RFC
        assert_eq!(out.len(), 144);
        assert_eq!(out, expected);
    }

    /// What the device does: its key agrees with the keyid of the header, then the record opens.
    fn device_open(body: &[u8], ua: EphemeralPrivateKey, ua_public: &[u8], auth: &[u8]) -> Vec<u8> {
        let salt = &body[..16];
        assert_eq!(u32::from_be_bytes(body[16..20].try_into().expect("4")), 4096);
        let idlen = usize::from(body[20]);
        let as_public = body[21..21 + idlen].to_vec();
        let data = &body[21 + idlen..];
        agree_ephemeral(ua, &UnparsedPublicKey::new(&ECDH_P256, &as_public), |secret| {
            let prk_key = hmac256(auth, secret);
            let mut info = b"WebPush: info\0".to_vec();
            info.extend_from_slice(ua_public);
            info.extend_from_slice(&as_public);
            info.push(1);
            let prk = hmac256(salt, &hmac256(&prk_key, &info));
            let cek = &hmac256(&prk, b"Content-Encoding: aes128gcm\0\x01")[..16];
            let nonce: [u8; 12] = hmac256(&prk, b"Content-Encoding: nonce\0\x01")[..12].try_into().expect("12");
            let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, cek).expect("key"));
            let mut rec = data.to_vec();
            let plain = key.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut rec).expect("opens").to_vec();
            assert_eq!(plain.last(), Some(&2));
            plain[..plain.len() - 1].to_vec()
        })
        .expect("agreed")
    }

    fn vapid() -> VapidKeys {
        VapidKeys {
            public_key: "BJXDcTMFlHY6onShpth5k0Q1StzcMhCL6VcpgtVDcsduXRDnCR7fe3EUHuM8RasMkIByB1rC2XgEj0fTwO8ezsc".into(),
            private_key: "Dy2zH-6gm82MoZx_Tb4ygz5v9IamQTPc3vUeGEqnRkY".into(),
        }
    }

    fn opts() -> Options {
        Options { subject: "mailto:admin@example.com".into(), vapid: vapid(), ttl: 86_400, urgency: "high".into(), timeout_ms: 15_000 }
    }

    #[test]
    fn request_readable_by_the_device_only() {
        let rng = SystemRandom::new();
        let ua = EphemeralPrivateKey::generate(&ECDH_P256, &rng).expect("key");
        let ua_public = ua.compute_public_key().expect("public").as_ref().to_vec();
        let auth = [9u8; 16];
        let sub = json!({ "endpoint": "https://push.example.net:8443/push/abc?x=1", "keys": { "p256dh": crate::js::base64_url(&ua_public), "auth": crate::js::base64_url(&auth) } });
        let payload = r#"{"title":"Anna Rossi","body":"ciao"}"#;
        let r = request_for(&sub, payload, &opts(), 1_790_000_000).expect("request");
        assert_eq!(r.url, "https://push.example.net:8443/push/abc?x=1");
        let names: Vec<_> = r.headers.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["TTL", "Content-Length", "Content-Type", "Content-Encoding", "Authorization", "Urgency"]);
        assert_eq!(r.header("ttl"), Some("86400"));
        assert_eq!(r.header("urgency"), Some("high"));
        assert_eq!(r.header("content-encoding"), Some("aes128gcm"));
        assert_eq!(r.header("content-length"), Some(r.body.len().to_string().as_str()));
        assert_eq!(r.timeout.as_millis(), 15_000);
        assert_eq!(device_open(&r.body, ua, &ua_public, &auth), payload.as_bytes());

        // the VAPID header: a JWT for the origin of the endpoint, signed with the key of k
        let auth_header = r.header("authorization").expect("vapid");
        let re = regex::Regex::new(r"^vapid t=([^,\s]+),\s*k=(\S+)$").expect("fixed");
        let m = re.captures(auth_header).expect("form");
        assert_eq!(&m[2], vapid().public_key);
        let parts: Vec<&str> = m[1].split('.').collect();
        let input = format!("{}.{}", parts[0], parts[1]);
        let point = crate::js::base64_decode_lenient(&m[2]);
        Verify::new(&ECDSA_P256_SHA256_FIXED, &point).verify(input.as_bytes(), &crate::js::base64_decode_lenient(parts[2])).expect("signature");
        let head: Value = serde_json::from_slice(&crate::js::base64_decode_lenient(parts[0])).expect("json");
        assert_eq!(head, json!({"typ": "JWT", "alg": "ES256"}));
        let claims = String::from_utf8(crate::js::base64_decode_lenient(parts[1])).expect("utf8");
        assert_eq!(claims, r#"{"aud":"https://push.example.net:8443","exp":1790043200,"sub":"mailto:admin@example.com"}"#);
    }

    #[test]
    fn long_payload_in_records_of_4096_bytes() {
        let rng = SystemRandom::new();
        let ua = EphemeralPrivateKey::generate(&ECDH_P256, &rng).expect("key");
        let ua_public = ua.compute_public_key().expect("public").as_ref().to_vec();
        let out = encrypt(&ua_public, &[1; 16], &vec![b'x'; 5000]).expect("encrypted");
        // header 86, a full record of 4096, then 5000 - 4079 bytes, delimiter and tag
        assert_eq!(out.len(), 86 + 4096 + (5000 - 4079) + 1 + 16);
    }

    #[test]
    fn refused_before_sending() {
        let o = opts();
        assert_eq!(request_for(&json!({}), "x", &o, 0).unwrap_err(), "You must pass in a subscription with at least an endpoint.");
        assert_eq!(
            request_for(&json!({"endpoint": "https://push/x"}), "x", &o, 0).unwrap_err(),
            "To send a message with a payload, the subscription must have 'auth' and 'p256dh' keys."
        );
        assert_eq!(
            request_for(&json!({"endpoint": "https://push/x", "keys": {"p256dh": "k", "auth": "a"}}), "x", &o, 0).unwrap_err(),
            "The subscription p256dh value should be 65 bytes long."
        );
        let fake = Options { vapid: VapidKeys { public_key: "BPublic".into(), private_key: "private".into() }, ..opts() };
        let rng = SystemRandom::new();
        let ua = EphemeralPrivateKey::generate(&ECDH_P256, &rng).expect("key");
        let p = crate::js::base64_url(ua.compute_public_key().expect("public").as_ref());
        let sub = json!({"endpoint": "https://push/x", "keys": {"p256dh": p, "auth": crate::js::base64_url(&[1; 16])}});
        assert_eq!(request_for(&sub, "x", &fake, 0).unwrap_err(), "Vapid public key should be 65 bytes long when decoded.");
    }
}
