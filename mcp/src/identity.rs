//! Resource handles: every id given to a client is signed and bound to one account and one kind, so a native id of
//! another account, or of another kind, never reaches a provider. Same format as the TypeScript server
//! (base64url of the JSON, a dot, base64url of its HMAC-SHA256), so handles stay valid across the switch.

use crate::js::{Obj, base64_decode_lenient, base64_url};
use crate::{Error, Result};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::sync::OnceLock;

static KEY: OnceLock<Vec<u8>> = OnceLock::new();

/// COMMUNICATION_ID_KEY, or a random key for this process (handles then die with it).
fn key() -> &'static [u8] {
    KEY.get_or_init(|| match std::env::var("COMMUNICATION_ID_KEY") {
        Ok(k) if !k.is_empty() => k.into_bytes(),
        _ => {
            let mut b = [0u8; 32];
            getrandom::fill(&mut b).expect("random key");
            b.iter().map(|x| format!("{x:02x}")).collect::<String>().into_bytes()
        }
    })
}

fn mac(body: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key()).expect("any key length");
    m.update(body.as_bytes());
    m.finalize().into_bytes().to_vec()
}

pub fn resource_id(account: &str, kind: &str, native: &str, context: Obj) -> String {
    let payload = json!({ "account": account, "kind": kind, "native": native, "context": Value::Object(context) });
    let body = base64_url(serde_json::to_string(&payload).unwrap_or_default().as_bytes());
    let sig = base64_url(&mac(&body));
    format!("{body}.{sig}")
}

pub fn handle(account: &str, kind: &str, native: &str) -> String {
    resource_id(account, kind, native, Obj::new())
}

/// The native id and context of a handle, refused unless signed here for this account and kind.
pub struct Resource {
    pub native: String,
    pub context: Obj,
}

impl Resource {
    pub fn ctx(&self, key: &str) -> Option<&Value> {
        self.context.get(key)
    }
    pub fn ctx_str(&self, key: &str) -> Option<String> {
        self.context.get(key).map(|v| crate::js::string(Some(v)))
    }
}

pub fn resource(account: &str, kind: &str, id: &str) -> Result<Resource> {
    let mut parts = id.split('.');
    let body = parts.next().unwrap_or("");
    let sig = parts.next().unwrap_or("");
    let extra = parts.next();
    let expected = mac(body);
    let got = base64_decode_lenient(sig);
    let same = got.len() == expected.len() && got.iter().zip(&expected).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0;
    if body.is_empty() || extra.is_some_and(|e| !e.is_empty()) || !same {
        return Err(Error::comm("INVALID_RESOURCE", "Invalid resource handle; list resources again"));
    }
    let value: Value = serde_json::from_slice(&base64_decode_lenient(body)).map_err(|_| Error::comm("INVALID_RESOURCE", "Invalid resource handle"))?;
    let ok = value.get("account").and_then(Value::as_str) == Some(account) && value.get("kind").and_then(Value::as_str) == Some(kind);
    let native = value.get("native").and_then(Value::as_str);
    match (ok, native) {
        (true, Some(n)) => Ok(Resource { native: n.to_string(), context: crate::js::obj(value.get("context").unwrap_or(&Value::Null)) }),
        _ => Err(Error::comm("ACCOUNT_MISMATCH", "Resource belongs to another account or resource type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_refusals() {
        let mut c = Obj::new();
        c.insert("message".into(), json!("m1"));
        let h = resource_id("outlook:a@b.c", "email_attachment", "att", c);
        let r = resource("outlook:a@b.c", "email_attachment", &h).ok().unwrap();
        assert_eq!((r.native.as_str(), r.ctx_str("message").as_deref()), ("att", Some("m1")));
        let code = |e: Result<Resource>| match e {
            Err(Error::Comm { code, .. }) => code,
            _ => "ok",
        };
        assert_eq!(code(resource("outlook:x@b.c", "email_attachment", &h)), "ACCOUNT_MISMATCH");
        assert_eq!(code(resource("outlook:a@b.c", "email", &h)), "ACCOUNT_MISMATCH");
        assert_eq!(code(resource("outlook:a@b.c", "email_attachment", "abc")), "INVALID_RESOURCE");
        assert_eq!(code(resource("outlook:a@b.c", "email_attachment", &format!("{h}.x"))), "INVALID_RESOURCE");
        let (body, _) = h.split_once('.').unwrap();
        assert_eq!(code(resource("outlook:a@b.c", "email_attachment", &format!("{body}.AAAA"))), "INVALID_RESOURCE");
    }

    #[test]
    fn same_format_as_typescript() {
        // the body is base64url of JSON.stringify({account, kind, native, context})
        let h = handle("teams:1", "conversation", "19:x");
        let (body, _) = h.split_once('.').unwrap();
        assert_eq!(String::from_utf8(base64_decode_lenient(body)).unwrap(), r#"{"account":"teams:1","kind":"conversation","native":"19:x","context":{}}"#);
    }
}
