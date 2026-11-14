//! What the relay sends a phone of the Android app is sealed with the key of that phone, AES-256-GCM, so that Google
//! carries ciphertext only, as with Web Push (src/agent/push/seal.ts); the app opens it (Seal.kt).

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Map, Value};

/// The data of the FCM message for `content`: v 1, iv (12 random bytes), ct (ciphertext and 16-byte tag), base64url.
pub fn seal_for(key: &str, content: &Value) -> Result<Map<String, Value>, String> {
    let mut iv = [0u8; 12];
    SystemRandom::new().fill(&mut iv).map_err(|_| "no random bytes".to_string())?;
    seal_with_iv(key, content, iv)
}

fn seal_with_iv(key: &str, content: &Value, iv: [u8; 12]) -> Result<Map<String, Value>, String> {
    let key = crate::js::base64_decode_lenient(key);
    let key = UnboundKey::new(&AES_256_GCM, &key).map_err(|_| "Invalid key length".to_string())?;
    let mut ct = crate::js::stringify(content).into_bytes();
    LessSafeKey::new(key).seal_in_place_append_tag(Nonce::assume_unique_for_key(iv), Aad::empty(), &mut ct).map_err(|_| "seal failed".to_string())?;
    let mut m = Map::new();
    m.insert("v".into(), Value::from("1"));
    m.insert("iv".into(), Value::from(crate::js::base64_url(&iv)));
    m.insert("ct".into(), Value::from(crate::js::base64_url(&ct)));
    Ok(m)
}

#[cfg(test)]
pub(crate) fn open(key: &str, data: &Map<String, Value>) -> Option<Value> {
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &crate::js::base64_decode_lenient(key)).ok()?);
    let iv: [u8; 12] = crate::js::base64_decode_lenient(data.get("iv")?.as_str()?).try_into().ok()?;
    let mut ct = crate::js::base64_decode_lenient(data.get("ct")?.as_str()?);
    let plain = key.open_in_place(Nonce::assume_unique_for_key(iv), Aad::empty(), &mut ct).ok()?;
    serde_json::from_slice(plain).ok()
}

#[cfg(test)]
pub(crate) fn new_device_key() -> String {
    let mut k = [0u8; 32];
    SystemRandom::new().fill(&mut k).expect("random");
    crate::js::base64_url(&k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opens_with_the_key_of_the_phone_only() {
        let key = new_device_key();
        let data = seal_for(&key, &json!({"title": "Anna Rossi is calling", "acc": 2})).expect("sealed");
        assert_eq!(data.keys().collect::<Vec<_>>(), ["v", "iv", "ct"]);
        assert_eq!(data["v"], "1");
        assert_eq!(open(&key, &data), Some(json!({"title": "Anna Rossi is calling", "acc": 2})));
        assert_eq!(open(&new_device_key(), &data), None);
        assert_ne!(seal_for(&key, &json!({"a": 1})).expect("sealed")["iv"], seal_for(&key, &json!({"a": 1})).expect("sealed")["iv"]);
        assert_eq!(seal_for("c2hvcnQ", &json!({})).unwrap_err(), "Invalid key length");
        // ciphertext followed by the 16-byte tag
        let fixed = seal_with_iv(&key, &json!({"a": 1}), [7; 12]).expect("sealed");
        assert_eq!(crate::js::base64_decode_lenient(fixed["ct"].as_str().expect("ct")).len(), r#"{"a":1}"#.len() + 16);
    }
}
