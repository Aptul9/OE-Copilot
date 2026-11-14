//! Better Auth cookies: names (__Secure- when APP_URL is https), attributes (HttpOnly, SameSite=Lax, Path=/), Set-Cookie
//! written as better-call writes it, and signed cookies read (value.signature, a 44-character HMAC).

use super::crypto::{sign, signature_valid};
use crate::js::{decode_uri_component, encode_uri_component};

pub const SESSION: &str = "session_token";
pub const SESSION_DATA: &str = "session_data";
pub const DONT_REMEMBER: &str = "dont_remember";

/// 30 days: a session and its cookie (session.expiresIn of src/lib/auth.ts).
pub const EXPIRES_S: i64 = 60 * 60 * 24 * 30;

pub fn secure() -> bool {
    crate::config::app_url().starts_with("https://")
}

/// __Secure-better-auth.<name>
pub fn name(cookie: &str) -> String {
    format!("{}better-auth.{cookie}", if secure() { "__Secure-" } else { "" })
}

/// String.prototype.trim
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// tryDecode: decodeURIComponent when there is a %, the text as it is when that fails.
fn try_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    decode_uri_component(s).unwrap_or_else(|| s.to_string())
}

fn line(name: &str, value: &str, max_age: Option<i64>) -> String {
    let mut c = format!("{name}={value}");
    if let Some(m) = max_age.filter(|m| *m >= 0) {
        c.push_str(&format!("; Max-Age={m}"));
    }
    c.push_str("; Path=/; HttpOnly");
    if secure() || name.starts_with("__Secure-") {
        c.push_str("; Secure");
    }
    c.push_str("; SameSite=Lax");
    c
}

/// serializeSignedCookie: value.signature, then encodeURIComponent of the whole.
pub fn write_signed(name: &str, value: &str, secret: &str, max_age: Option<i64>) -> String {
    line(name, &encode_uri_component(&format!("{value}.{}", sign(value, secret))), max_age)
}

/// expireCookie: same name and attributes, Max-Age=0.
pub fn expired(name: &str) -> String {
    line(name, "", Some(0))
}

/// parseCookies of better-call: key=value separated by ";", the first of two equal keys wins, surrounding quotes go,
/// the value is decoded when it holds a %.
pub fn parse_all(header: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let s = header;
    let mut i = 0;
    while i < s.len() {
        let Some(eq) = s[i..].find('=').map(|x| i + x) else { break };
        let end = match s[i..].find(';').map(|x| i + x) {
            None => s.len(),
            Some(f) if f < eq => {
                i = s[..eq].rfind(';').map_or(0, |x| x + 1);
                continue;
            }
            Some(f) => f,
        };
        let k = js_trim(&s[i..eq]).to_string();
        if !out.iter().any(|(x, _)| *x == k) {
            let mut v = js_trim(&s[eq + 1..end]).to_string();
            if v.starts_with('"') {
                let c: Vec<char> = v.chars().collect();
                v = c.get(1..c.len().saturating_sub(1).max(1)).map(|x| x.iter().collect()).unwrap_or_default();
            }
            out.push((k, try_decode(&v)));
        }
        i = end + 1;
    }
    out
}

pub fn get(header: Option<&str>, name: &str) -> Option<String> {
    parse_all(header?).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// A signed cookie: None when missing or not shaped as a signed value, Some(None) when the signature is wrong,
/// Some(Some(value)) when good.
pub fn get_signed(header: Option<&str>, name: &str, secret: &str) -> Option<Option<String>> {
    let v = get(header, name).filter(|v| !v.is_empty())?;
    let dot = v.rfind('.')?;
    if dot < 1 {
        return None;
    }
    let (value, sig) = (&v[..dot], &v[dot + 1..]);
    if sig.len() != 44 || !sig.ends_with('=') {
        return None;
    }
    Some(signature_valid(value, sig, secret).then(|| value.to_string()))
}

pub fn signed(header: Option<&str>, name: &str, secret: &str) -> Option<String> {
    get_signed(header, name, secret).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_like_better_call() {
        let c = parse_all("a=1; b=\"x y\"; a=2;c=%41%; d=%E2%82%AC");
        assert_eq!(c, vec![("a".into(), "1".into()), ("b".into(), "x y".into()), ("c".into(), "%41%".into()), ("d".into(), "€".into())]);
    }

    #[test]
    fn signed_cookie() {
        let s = "s".repeat(48);
        let l = write_signed("x", "tok", &s, Some(10));
        let v = l.split(';').next().unwrap().trim_start_matches("x=");
        assert_eq!(signed(Some(&format!("x={v}")), "x", &s), Some("tok".into()));
        assert_eq!(get_signed(Some("x=tok.made-up"), "x", &s), None);
        let fake = format!("x={}", encode_uri_component(&format!("tok.{}", sign("other", &s))));
        assert_eq!(get_signed(Some(&fake), "x", &s), Some(None));
    }
}
