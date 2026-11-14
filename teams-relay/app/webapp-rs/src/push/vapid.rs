//! The Web Push keys of vapid/ (src/agent/push/vapid.ts): private_key.pem (PKCS#8, P-256) and appkey.txt, the public
//! key the devices subscribed with. Web Push takes the raw private scalar and the uncompressed public point, base64url.

use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};

/// The Web Push keys of vapid/: raw private scalar and uncompressed public point, both base64url.
#[derive(Clone, Debug)]
pub struct VapidKeys {
    pub public_key: String,
    pub private_key: String,
}

const NOT_P256: &str = "VAPID private key is not a P-256 key";
const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

/// The DER of the first block of a PEM.
pub fn pem_der(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem.lines().skip_while(|l| !l.starts_with("-----BEGIN")).skip(1).take_while(|l| !l.starts_with("-----END")).collect();
    use base64::Engine;
    let der = base64::engine::general_purpose::STANDARD.decode(body.trim()).map_err(|_| "not a PEM private key".to_string())?;
    if der.is_empty() {
        return Err("not a PEM private key".into());
    }
    Ok(der)
}

/// One DER element: tag, content, the rest after it.
fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, b) = b.split_first()?;
    let (&first, mut b) = b.split_first()?;
    let len = if first < 0x80 {
        usize::from(first)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || b.len() < n {
            return None;
        }
        let len = b[..n].iter().fold(0usize, |a, x| (a << 8) | usize::from(*x));
        b = &b[n..];
        len
    };
    (b.len() >= len).then(|| (tag, &b[..len], &b[len..]))
}

/// Scalar and public point of a PKCS#8 P-256 key; the public point is required (Node and Python always write it).
fn p256_of_pkcs8(der: &[u8]) -> Result<([u8; 32], Vec<u8>), String> {
    let bad = || "not a PKCS#8 private key".to_string();
    let (0x30, info, _) = tlv(der).ok_or_else(bad)? else { return Err(bad()) };
    let (0x02, _, rest) = tlv(info).ok_or_else(bad)? else { return Err(bad()) };
    let (0x30, alg, rest) = tlv(rest).ok_or_else(bad)? else { return Err(bad()) };
    let (0x04, key, _) = tlv(rest).ok_or_else(bad)? else { return Err(bad()) };
    let (oid_tag, oid, params) = tlv(alg).ok_or_else(bad)?;
    if oid_tag != 0x06 || oid != EC_PUBLIC_KEY {
        return Err(NOT_P256.into());
    }
    match tlv(params) {
        Some((0x06, curve, _)) if curve == PRIME256V1 => {}
        _ => return Err(NOT_P256.into()),
    }
    let (0x30, ec, _) = tlv(key).ok_or_else(bad)? else { return Err(bad()) };
    let (0x02, _, mut rest) = tlv(ec).ok_or_else(bad)? else { return Err(bad()) };
    let (0x04, d, more) = tlv(rest).ok_or_else(bad)? else { return Err(bad()) };
    if d.len() > 32 {
        return Err(NOT_P256.into());
    }
    let mut scalar = [0u8; 32];
    scalar[32 - d.len()..].copy_from_slice(d);
    rest = more;
    let mut public = None;
    while let Some((tag, content, more)) = tlv(rest) {
        if tag == 0xa1
            && let Some((0x03, bits, _)) = tlv(content)
            && bits.len() == 66
            && bits[0] == 0
            && bits[1] == 4
        {
            public = Some(bits[1..].to_vec());
        }
        rest = more;
    }
    Ok((scalar, public.ok_or_else(|| "VAPID private key has no public key".to_string())?))
}

fn vapid_keys_from_pem(pem: &str) -> Result<VapidKeys, String> {
    let (scalar, public) = p256_of_pkcs8(&pem_der(pem)?)?;
    let rng = ring::rand::SystemRandom::new();
    EcdsaKeyPair::from_private_key_and_public_key(&ECDSA_P256_SHA256_FIXED_SIGNING, &scalar, &public, &rng)
        .map_err(|e| format!("VAPID private key refused: {e}"))?;
    Ok(VapidKeys { public_key: crate::js::base64_url(&public), private_key: crate::js::base64_url(&scalar) })
}

/// None without a private key: push stays off. A key that does not match appkey.txt would sign pushes every subscribed
/// device rejects: Err.
pub fn load_vapid_keys(private_key_file: &str, app_key_file: &str) -> Result<Option<VapidKeys>, String> {
    if !std::path::Path::new(private_key_file).exists() {
        return Ok(None);
    }
    let pem = std::fs::read_to_string(private_key_file).map_err(|e| e.to_string())?;
    let keys = vapid_keys_from_pem(&pem)?;
    if std::path::Path::new(app_key_file).exists() {
        let app_key = std::fs::read_to_string(app_key_file).map_err(|e| e.to_string())?;
        if app_key.trim() != keys.public_key {
            return Err(format!("VAPID_PRIVATE does not match the public key in {app_key_file}"));
        }
    }
    Ok(Some(keys))
}

/// The signing key of the keys, checked when they were loaded.
pub fn key_pair(keys: &VapidKeys) -> Result<EcdsaKeyPair, String> {
    let d = crate::js::base64_decode_lenient(&keys.private_key);
    let p = crate::js::base64_decode_lenient(&keys.public_key);
    if p.len() != 65 {
        return Err("Vapid public key should be 65 bytes long when decoded.".into());
    }
    if d.len() != 32 {
        return Err("Vapid private key should be 32 bytes long when decoded.".into());
    }
    EcdsaKeyPair::from_private_key_and_public_key(&ECDSA_P256_SHA256_FIXED_SIGNING, &d, &p, &ring::rand::SystemRandom::new())
        .map_err(|e| format!("VAPID key refused: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/push/testdata");

    #[test]
    fn read_back_with_the_public_key_of_appkey_txt() {
        let pem = format!("{DIR}/vapid-test.pem");
        let app = format!("{DIR}/vapid-test-appkey.txt");
        let keys = load_vapid_keys(&pem, &app).expect("loaded").expect("keys");
        assert_eq!(keys.public_key, "BJXDcTMFlHY6onShpth5k0Q1StzcMhCL6VcpgtVDcsduXRDnCR7fe3EUHuM8RasMkIByB1rC2XgEj0fTwO8ezsc");
        assert_eq!(keys.private_key, "Dy2zH-6gm82MoZx_Tb4ygz5v9IamQTPc3vUeGEqnRkY");
        assert!(key_pair(&keys).is_ok());
        assert!(load_vapid_keys(&format!("{DIR}/missing.pem"), &app).expect("none").is_none());
        // appkey.txt absent: nothing to check against
        assert!(load_vapid_keys(&pem, &format!("{DIR}/missing.txt")).expect("loaded").is_some());
    }

    #[test]
    fn refuse_another_curve_and_another_appkey() {
        let other = std::env::temp_dir().join(format!("trweb-push-appkey-{}", std::process::id()));
        std::fs::write(&other, "BPublic").expect("write");
        let e = load_vapid_keys(&format!("{DIR}/vapid-test.pem"), other.to_str().expect("path")).unwrap_err();
        assert!(e.starts_with("VAPID_PRIVATE does not match the public key in "), "{e}");
        let _ = std::fs::remove_file(other);
        assert_eq!(load_vapid_keys(&format!("{DIR}/p384-test.pem"), "").unwrap_err(), NOT_P256);
        assert_eq!(load_vapid_keys(&format!("{DIR}/rsa-test.pem"), "").unwrap_err(), NOT_P256);
    }
}
