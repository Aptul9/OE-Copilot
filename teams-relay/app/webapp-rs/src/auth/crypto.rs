//! The cryptography of Better Auth 1.7, the same bytes: passwords (scrypt N=16384 r=16 p=1, 64 bytes, a 16-byte salt
//! written in hex and used as text), cookie signatures (HMAC-SHA256 in base64) and the random ids. An app.db written
//! by Better Auth reads here, and the other way round. Port of the calendar gateway (agente-rs/gateway/src/auth/cripto.rs).

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;

type HmacSha256 = Hmac<Sha256>;

pub fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("system random generator");
    b
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// createRandomStringGenerator: characters picked with the bytes that would skew the distribution dropped.
pub fn random_string(len: usize, alphabet: &str) -> String {
    let chars: Vec<char> = alphabet.chars().collect();
    let n = chars.len();
    let max = (256 / n) * n;
    let mut out = String::with_capacity(len);
    while out.chars().count() < len {
        for b in random(len * 2) {
            if (b as usize) < max {
                out.push(chars[b as usize % n]);
                if out.chars().count() == len {
                    break;
                }
            }
        }
    }
    out
}

/// generateId: 32 characters a-z, A-Z, 0-9 (row ids and session tokens).
pub fn generate_id() -> String {
    random_string(32, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")
}

fn scrypt_key(password: &str, salt: &str) -> Vec<u8> {
    let normalized: String = password.nfkc().collect();
    let params = scrypt::Params::new(14, 16, 1, 64).expect("fixed parameters");
    let mut out = vec![0u8; 64];
    scrypt::scrypt(normalized.as_bytes(), salt.as_bytes(), &params, &mut out).expect("fixed length");
    out
}

/// hashPassword: "<salt hex>:<key hex>". 32 MB and tens of milliseconds: off the server thread (hash_password_async).
pub fn hash_password(password: &str) -> String {
    let salt = hex(&random(16));
    let key = scrypt_key(password, &salt);
    format!("{salt}:{}", hex(&key))
}

/// verifyPassword: Err when the hash is not "salt:key".
pub fn verify_password(hash: &str, password: &str) -> Result<bool, String> {
    let mut parts = hash.split(':');
    let (Some(salt), Some(key)) = (parts.next().filter(|s| !s.is_empty()), parts.next().filter(|s| !s.is_empty())) else {
        return Err("Invalid password hash".to_string());
    };
    let mine = hex(&scrypt_key(password, salt));
    Ok(mine.len() == key.len() && mine.bytes().zip(key.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0)
}

pub async fn hash_password_async(password: String) -> String {
    tokio::task::spawn_blocking(move || hash_password(&password)).await.unwrap_or_default()
}

pub async fn verify_password_async(hash: String, password: String) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || verify_password(&hash, &password)).await.unwrap_or_else(|e| Err(e.to_string()))
}

/// The signature of a cookie: HMAC-SHA256 of the value with the secret, base64 with padding (44 characters).
pub fn sign(value: &str, secret: &str) -> String {
    let mut m = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    m.update(value.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
}

/// The signature of a cookie read: compared in constant time.
pub fn signature_valid(value: &str, sig_b64: &str, secret: &str) -> bool {
    let Ok(data) = base64::engine::general_purpose::STANDARD.decode(sig_b64) else { return false };
    let mut m = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).expect("any key length");
    m.update(value.as_bytes());
    m.verify_slice(&data).is_ok()
}

pub fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    hex(&Sha256::digest(s.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // from a database written by Better Auth 1.7 (the calendar gateway's test vectors)
    const SECRET: &str = "5a086d287157b50af22394627fd09e1f29189dcf7684814c";

    #[test]
    fn better_auth_password() {
        let hash = "4f8c51c767a8891ea8cad9c2ef2aa3c9:b5db7e10e29250584b3272e8fdb5091a1877c49a0f514239001bff1650a5a996b804a3319deaaf81ba636db0f8efdc9cf6872842fc2be1d6cd77d3567ae6d6e3";
        assert_eq!(verify_password(hash, "dcd7c680f0fbec96ecd7cdc8"), Ok(true));
        assert_eq!(verify_password(hash, "dcd7c680f0fbec96ecd7cdc9"), Ok(false));
        assert!(verify_password("only", "x").is_err());
        assert_eq!(verify_password(&hash_password("a password"), "a password"), Ok(true));
    }

    #[test]
    fn cookie_signature() {
        assert_eq!(sign("2Q3MUFMbiFJMBwvYnWdDCEtShJbdeDTW", SECRET), "C+M07+YCg8017knRmOytMy+Ey0K1LpV2tb35dtHseUg=");
        assert!(signature_valid("2Q3MUFMbiFJMBwvYnWdDCEtShJbdeDTW", "C+M07+YCg8017knRmOytMy+Ey0K1LpV2tb35dtHseUg=", SECRET));
        assert!(!signature_valid("2Q3MUFMbiFJMBwvYnWdDCEtShJbdeDTX", "C+M07+YCg8017knRmOytMy+Ey0K1LpV2tb35dtHseUg=", SECRET));
        assert_eq!(generate_id().len(), 32);
    }
}
