//! WebAuthn per le passkey, come @simplewebauthn/server 13: la verifica di una registrazione (attestazione "none", o "packed" firmata dalla
//! credenziale stessa) e di un accesso (firma ES256/384/512, EdDSA, RSA PKCS#1 e PSS sulla chiave COSE salvata). Le attestazioni con un
//! certificato (packed con x5c, tpm, android-key, apple, fido-u2f) non si accettano: con `attestation: "none"` i browser non le mandano.

use super::cbor::{primo, Cbor};
use crate::b64::{codifica_url, decodifica_node};
use oe_nucleo::json::Json;
use sha2::{Digest, Sha256, Sha384, Sha512};

/// Gli algoritmi COSE che una registrazione può avere (supportedCOSEAlgorithmIdentifiers).
const ALGORITMI_REGISTRAZIONE: [i128; 10] = [-8, -7, -36, -37, -38, -39, -257, -258, -259, -65535];
const ALGORITMI_COSE: [i128; 12] = [-7, -8, -35, -36, -37, -38, -39, -47, -257, -258, -259, -65535];

/// Una sfida: 32 byte casuali in base64url.
pub fn sfida() -> String {
    codifica_url(&super::cripto::casuali(32))
}

/// isoBase64URL.isBase64URL: senza i "=" in fondo, solo l'alfabeto di base64url.
pub fn e_base64url(s: &str) -> bool {
    senza_riempimento(s).bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// trimPadding
pub fn senza_riempimento(s: &str) -> String {
    s.replace('=', "")
}

fn testo<'a>(v: &'a Json, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Json::as_str)
}

struct DatiAutenticatore {
    rp_id_hash: Vec<u8>,
    presente: bool,
    backup_possibile: bool,
    backup_fatto: bool,
    contatore: u32,
    aaguid: Option<Vec<u8>>,
    id_credenziale: Option<Vec<u8>>,
    chiave: Option<Vec<u8>>,
}

fn leggi_dati_autenticatore(dati: &[u8]) -> Result<DatiAutenticatore, String> {
    if dati.len() < 37 {
        return Err(format!("Authenticator data was {} bytes, expected at least 37 bytes", dati.len()));
    }
    let flag = dati[32];
    let mut p = 37;
    let mut out = DatiAutenticatore {
        rp_id_hash: dati[..32].to_vec(),
        presente: flag & 1 != 0,
        backup_possibile: flag & 8 != 0,
        backup_fatto: flag & 16 != 0,
        contatore: u32::from_be_bytes([dati[33], dati[34], dati[35], dati[36]]),
        aaguid: None,
        id_credenziale: None,
        chiave: None,
    };
    if flag & 64 != 0 {
        out.aaguid = Some(dati.get(p..p + 16).ok_or("authData troncato")?.to_vec());
        p += 16;
        let l = dati.get(p..p + 2).ok_or("authData troncato")?;
        let n = usize::from(u16::from_be_bytes([l[0], l[1]]));
        p += 2;
        out.id_credenziale = Some(dati.get(p..p + n).ok_or("authData troncato")?.to_vec());
        p += n;
        // Firefox 117 scriveva male la chiave EdDSA (mappa di 3 voci invece di 4): si corregge il byte per leggerla
        const ROTTA: [u8; 17] = [0xa3, 0x01, 0x63, 0x4f, 0x4b, 0x50, 0x03, 0x27, 0x20, 0x67, 0x45, 0x64, 0x32, 0x35, 0x35, 0x31, 0x39];
        let mut resto = dati[p..].to_vec();
        if resto.starts_with(&ROTTA) {
            resto[0] = 0xa4;
        }
        let (_, n) = primo(&resto)?;
        out.chiave = Some(resto[..n].to_vec());
        p += n;
    }
    if flag & 128 != 0 {
        let (_, n) = primo(&dati[p..])?;
        p += n;
    }
    if dati.len() > p {
        return Err("Leftover bytes detected while parsing authenticator data".to_string());
    }
    Ok(out)
}

/// parseBackupFlags: (tipo, salvata). Una credenziale di un dispositivo solo che dice di essere salvata è impossibile.
fn tipo_dispositivo(d: &DatiAutenticatore) -> Result<(&'static str, bool), String> {
    let tipo = if d.backup_possibile { "multiDevice" } else { "singleDevice" };
    if tipo == "singleDevice" && d.backup_fatto {
        return Err("Single-device credential indicated that it was backed up, which should be impossible.".to_string());
    }
    Ok((tipo, d.backup_fatto))
}

fn controlla_client(dati_client: &str, tipo_atteso: &str, sfida: &str, origine: &str, registrazione: bool) -> Result<(), String> {
    let testo_client = String::from_utf8_lossy(&decodifica_node(dati_client)).into_owned();
    let c = Json::analizza(&testo_client).map_err(|e| e.messaggio)?;
    let tipo = c.get("type").and_then(Json::as_str).unwrap_or("");
    if tipo != tipo_atteso {
        return Err(format!("Unexpected {} response type: {tipo}", if registrazione { "registration" } else { "authentication" }));
    }
    if c.get("challenge").and_then(Json::as_str) != Some(sfida) {
        return Err("Unexpected response challenge".to_string());
    }
    if c.get("origin").and_then(Json::as_str) != Some(origine) {
        return Err("Unexpected response origin".to_string());
    }
    if let Some(tb) = c.get("tokenBinding").filter(|t| crate::py::py_bool(Some(t))) {
        let ammessi: &[&str] = if registrazione { &["present", "supported", "not-supported"] } else { &["present", "supported", "notSupported"] };
        if !matches!(tb, Json::Obj(_)) || !ammessi.contains(&tb.get("status").and_then(Json::as_str).unwrap_or("")) {
            return Err("Unexpected tokenBinding".to_string());
        }
    }
    Ok(())
}

pub struct Registrata {
    pub id_credenziale: String,
    pub chiave: Vec<u8>,
    pub contatore: u32,
    pub aaguid: String,
    pub tipo: &'static str,
    pub salvata: bool,
}

fn aaguid_testo(b: &[u8]) -> String {
    let h = super::cripto::esadecimale(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// verifyRegistrationResponse con requireUserVerification false. Ok(None): la verifica dice no; Err: la risposta non si legge.
pub fn verifica_registrazione(r: &Json, sfida: &str, origine: &str, rp_id: &str) -> Result<Option<Registrata>, String> {
    let id = testo(r, "id").filter(|s| !s.is_empty()).ok_or("Missing credential ID")?;
    if r.get("rawId").and_then(Json::as_str) != Some(id) {
        return Err("Credential ID was not base64url-encoded".to_string());
    }
    if testo(r, "type") != Some("public-key") {
        return Err("Unexpected credential type".to_string());
    }
    let risposta = r.get("response").ok_or("Missing response")?;
    let client = testo(risposta, "clientDataJSON").ok_or("Missing clientDataJSON")?;
    controlla_client(client, "webauthn.create", sfida, origine, true)?;
    let attestazione = decodifica_node(testo(risposta, "attestationObject").ok_or("Missing attestationObject")?);
    let (oggetto, _) = primo(&attestazione)?;
    let formato = match oggetto.prendi_testo("fmt") {
        Some(Cbor::Testo(f)) => f.clone(),
        _ => String::new(),
    };
    let dati = oggetto.prendi_testo("authData").and_then(Cbor::byte).ok_or("authData mancante")?.to_vec();
    let dichiarazione = oggetto.prendi_testo("attStmt").cloned().unwrap_or(Cbor::Mappa(vec![]));
    let a = leggi_dati_autenticatore(&dati)?;
    if a.rp_id_hash != super::cripto::sha256(rp_id.as_bytes()) {
        return Err("Unexpected RP ID hash".to_string());
    }
    if !a.presente {
        return Err("User presence was required, but user was not present".to_string());
    }
    let id_credenziale = a.id_credenziale.clone().ok_or("No credential ID was provided by authenticator")?;
    let chiave = a.chiave.clone().ok_or("No public key was provided by authenticator")?;
    let aaguid = a.aaguid.clone().ok_or("No AAGUID was present during registration")?;
    let (cose, _) = primo(&chiave)?;
    let alg = cose.prendi_intero(3).and_then(Cbor::intero).ok_or("Credential public key was missing numeric alg")?;
    if !ALGORITMI_REGISTRAZIONE.contains(&alg) {
        return Err(format!("Unexpected public key alg \"{alg}\""));
    }
    let hash_client = super::cripto::sha256(&decodifica_node(client));
    let verificata = match formato.as_str() {
        "none" => {
            if !matches!(&dichiarazione, Cbor::Mappa(v) if v.is_empty()) {
                return Err("None attestation had unexpected attestation statement".to_string());
            }
            true
        }
        "packed" => {
            let firma = dichiarazione.prendi_testo("sig").and_then(Cbor::byte).ok_or("No attestation signature provided in attestation statement (Packed)")?;
            let alg_dich = dichiarazione.prendi_testo("alg").and_then(Cbor::intero).ok_or("Attestation statement did not contain alg (Packed)")?;
            if !ALGORITMI_COSE.contains(&alg_dich) {
                return Err("Attestation statement contained invalid alg (Packed)".to_string());
            }
            if dichiarazione.prendi_testo("x5c").is_some() {
                return Err("Attestazione con certificato non ammessa (Packed|Full)".to_string());
            }
            verifica_firma(&cose, firma, &[dati.as_slice(), hash_client.as_slice()].concat(), Some(alg_dich))?
        }
        altro => return Err(format!("Unsupported Attestation Format: {altro}")),
    };
    if !verificata {
        return Ok(None);
    }
    let (tipo, salvata) = tipo_dispositivo(&a)?;
    Ok(Some(Registrata { id_credenziale: codifica_url(&id_credenziale), chiave, contatore: a.contatore, aaguid: aaguid_testo(&aaguid), tipo, salvata }))
}

/// verifyAuthenticationResponse con requireUserVerification false: (firma buona, nuovo contatore). Err: la risposta non si legge o non va.
pub fn verifica_accesso(r: &Json, sfida: &str, origine: &str, rp_id: &str, chiave: &[u8], contatore: u32) -> Result<(bool, u32), String> {
    let id = testo(r, "id").filter(|s| !s.is_empty()).ok_or("Missing credential ID")?;
    if r.get("rawId").and_then(Json::as_str) != Some(id) {
        return Err("Credential ID was not base64url-encoded".to_string());
    }
    if testo(r, "type") != Some("public-key") {
        return Err("Unexpected credential type".to_string());
    }
    let risposta = r.get("response").ok_or("Credential missing response")?;
    let client = testo(risposta, "clientDataJSON").ok_or("Credential response clientDataJSON was not a string")?;
    controlla_client(client, "webauthn.get", sfida, origine, false)?;
    let dati_b64 = testo(risposta, "authenticatorData").filter(|s| e_base64url(s)).ok_or("Credential response authenticatorData was not a base64url string")?;
    let firma_b64 = testo(risposta, "signature").filter(|s| e_base64url(s)).ok_or("Credential response signature was not a base64url string")?;
    if let Some(u) = risposta.get("userHandle") {
        if crate::py::py_bool(Some(u)) && u.as_str().is_none() {
            return Err("Credential response userHandle was not a string".to_string());
        }
    }
    let dati = decodifica_node(dati_b64);
    let a = leggi_dati_autenticatore(&dati)?;
    if a.rp_id_hash != super::cripto::sha256(rp_id.as_bytes()) {
        return Err("Unexpected RP ID hash".to_string());
    }
    if !a.presente {
        return Err("User not present during authentication".to_string());
    }
    if (a.contatore > 0 || contatore > 0) && a.contatore <= contatore {
        return Err(format!("Response counter value {} was lower than expected {contatore}", a.contatore));
    }
    tipo_dispositivo(&a)?;
    let hash_client = super::cripto::sha256(&decodifica_node(client));
    let (cose, _) = primo(chiave)?;
    let buona = verifica_firma(&cose, &decodifica_node(firma_b64), &[dati.as_slice(), hash_client.as_slice()].concat(), None)?;
    Ok((buona, a.contatore))
}

enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

/// mapCoseAlgToWebCryptoAlg
fn hash_di(alg: i128) -> Result<Hash, String> {
    match alg {
        -257 | -7 | -37 => Ok(Hash::Sha256),
        -258 | -35 | -38 => Ok(Hash::Sha384),
        -259 | -36 | -39 => Ok(Hash::Sha512),
        -65535 => Ok(Hash::Sha1),
        _ => Err(format!("Could not map COSE alg value of {alg} to a WebCrypto alg")),
    }
}

fn riassunto(h: &Hash, dati: &[u8]) -> Vec<u8> {
    match h {
        Hash::Sha1 => sha1::Sha1::digest(dati).to_vec(),
        Hash::Sha256 => Sha256::digest(dati).to_vec(),
        Hash::Sha384 => Sha384::digest(dati).to_vec(),
        Hash::Sha512 => Sha512::digest(dati).to_vec(),
    }
}

/// La firma ECDSA DER come r e s di `n` byte ciascuno (unwrapEC2Signature).
fn firma_ec(der: &[u8], n: usize) -> Result<Vec<u8>, String> {
    let errore = || "firma ECDSA non valida".to_string();
    let mut i = 0;
    let leggi_lunghezza = |i: &mut usize| -> Result<usize, String> {
        let b = *der.get(*i).ok_or_else(errore)?;
        *i += 1;
        if b < 0x80 {
            return Ok(usize::from(b));
        }
        let k = usize::from(b & 0x7f);
        if k == 0 || k > 2 {
            return Err(errore());
        }
        let mut l = 0;
        for _ in 0..k {
            l = (l << 8) | usize::from(*der.get(*i).ok_or_else(errore)?);
            *i += 1;
        }
        Ok(l)
    };
    if der.first() != Some(&0x30) {
        return Err(errore());
    }
    i += 1;
    leggi_lunghezza(&mut i)?;
    let pezzo = |i: &mut usize| -> Result<Vec<u8>, String> {
        if der.get(*i) != Some(&0x02) {
            return Err(errore());
        }
        *i += 1;
        let l = leggi_lunghezza(i)?;
        let v = der.get(*i..*i + l).ok_or_else(errore)?.to_vec();
        *i += l;
        Ok(v)
    };
    let r = pezzo(&mut i)?;
    let s = pezzo(&mut i)?;
    let normalizza = |b: Vec<u8>| -> Result<Vec<u8>, String> {
        if b.len() < n {
            let mut v = vec![0u8; n - b.len()];
            v.extend(b);
            Ok(v)
        } else if b.len() == n {
            Ok(b)
        } else if b.len() == n + 1 && b[0] == 0 && b[1] & 0x80 != 0 {
            Ok(b[1..].to_vec())
        } else {
            Err(format!("Invalid signature component length {}, expected {n}", b.len()))
        }
    };
    Ok([normalizza(r)?, normalizza(s)?].concat())
}

fn byte_di<'a>(cose: &'a Cbor, k: i128) -> Option<&'a [u8]> {
    cose.prendi_intero(k).and_then(Cbor::byte).filter(|b| !b.is_empty())
}

/// isoCrypto.verify: la firma di `dati` con la chiave COSE; `hash` sostituisce quello dell'algoritmo della chiave (attestazione packed).
fn verifica_firma(cose: &Cbor, firma: &[u8], dati: &[u8], hash: Option<i128>) -> Result<bool, String> {
    let kty = cose.prendi_intero(1).and_then(Cbor::intero);
    let alg = cose.prendi_intero(3).and_then(Cbor::intero);
    match kty {
        Some(2) => {
            let crv = cose.prendi_intero(-1).and_then(Cbor::intero).filter(|c| [1, 2, 3, 6, 8].contains(c)).ok_or("unknown COSE curve")?;
            let n = match crv {
                1 => 32,
                2 => 48,
                3 => 66,
                _ => return Err(format!("Unexpected COSE crv value of {crv} (EC2)")),
            };
            let rs = firma_ec(firma, n)?;
            let alg = alg.filter(|a| *a != 0).ok_or("Public key was missing alg (EC2)")?;
            let (x, y) = (byte_di(cose, -2).ok_or("Public key was missing x (EC2)")?, byte_di(cose, -3).ok_or("Public key was missing y (EC2)")?);
            let h = riassunto(&hash_di(hash.unwrap_or(alg))?, dati);
            let punto = [&[0x04u8][..], x, y].concat();
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            Ok(match crv {
                1 => {
                    let k = p256::ecdsa::VerifyingKey::from_sec1_bytes(&punto).map_err(|e| e.to_string())?;
                    p256::ecdsa::Signature::from_slice(&rs).map(|s| k.verify_prehash(&h, &s).is_ok()).unwrap_or(false)
                }
                2 => {
                    let k = p384::ecdsa::VerifyingKey::from_sec1_bytes(&punto).map_err(|e| e.to_string())?;
                    p384::ecdsa::Signature::from_slice(&rs).map(|s| k.verify_prehash(&h, &s).is_ok()).unwrap_or(false)
                }
                _ => {
                    let k = p521::ecdsa::VerifyingKey::from_sec1_bytes(&punto).map_err(|e| e.to_string())?;
                    p521::ecdsa::Signature::from_slice(&rs).map(|s| k.verify_prehash(&h, &s).is_ok()).unwrap_or(false)
                }
            })
        }
        Some(3) => {
            let alg = alg.filter(|a| *a != 0).ok_or("Public key was missing alg (RSA)")?;
            if !ALGORITMI_COSE.contains(&alg) {
                return Err(format!("Public key had invalid alg {alg} (RSA)"));
            }
            let (n, e) = (byte_di(cose, -1).ok_or("Public key was missing n (RSA)")?, byte_di(cose, -2).ok_or("Public key was missing e (RSA)")?);
            let chiave = rsa::RsaPublicKey::new(rsa::BigUint::from_bytes_be(n), rsa::BigUint::from_bytes_be(e)).map_err(|e| e.to_string())?;
            let h_alg = hash_di(hash.unwrap_or(alg))?;
            let h = riassunto(&h_alg, dati);
            let pss = match alg {
                -257 | -258 | -259 | -65535 => false,
                -37 | -38 | -39 => true,
                _ => return Err(format!("Unexpected RSA key algorithm {alg}")),
            };
            Ok(if pss {
                match h_alg {
                    Hash::Sha256 => chiave.verify(rsa::Pss::new_with_salt::<Sha256>(32), &h, firma).is_ok(),
                    Hash::Sha384 => chiave.verify(rsa::Pss::new_with_salt::<Sha384>(48), &h, firma).is_ok(),
                    Hash::Sha512 => chiave.verify(rsa::Pss::new_with_salt::<Sha512>(64), &h, firma).is_ok(),
                    Hash::Sha1 => chiave.verify(rsa::Pss::new_with_salt::<sha1::Sha1>(0), &h, firma).is_ok(),
                }
            } else {
                match h_alg {
                    Hash::Sha256 => chiave.verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &h, firma).is_ok(),
                    Hash::Sha384 => chiave.verify(rsa::Pkcs1v15Sign::new::<Sha384>(), &h, firma).is_ok(),
                    Hash::Sha512 => chiave.verify(rsa::Pkcs1v15Sign::new::<Sha512>(), &h, firma).is_ok(),
                    Hash::Sha1 => chiave.verify(rsa::Pkcs1v15Sign::new::<sha1::Sha1>(), &h, firma).is_ok(),
                }
            })
        }
        Some(1) => {
            let alg = alg.filter(|a| *a != 0).ok_or("Public key was missing alg (OKP)")?;
            if !ALGORITMI_COSE.contains(&alg) {
                return Err(format!("Public key had invalid alg {alg} (OKP)"));
            }
            let crv = cose.prendi_intero(-1).and_then(Cbor::intero).filter(|c| *c != 0).ok_or("Public key was missing crv (OKP)")?;
            let x = byte_di(cose, -2).ok_or("Public key was missing x (OKP)")?;
            if crv != 6 {
                return Err(format!("Unexpected COSE crv value of {crv} (OKP)"));
            }
            let chiave = ed25519_dalek::VerifyingKey::from_bytes(x.try_into().map_err(|_| "chiave Ed25519 non valida")?).map_err(|e| e.to_string())?;
            let Ok(f) = ed25519_dalek::Signature::from_slice(firma) else { return Ok(false) };
            Ok(ed25519_dalek::Verifier::verify(&chiave, dati, &f).is_ok())
        }
        k => Err(format!("Signature verification with public key of kty {} is not supported by this method", k.map_or("undefined".to_string(), |x| x.to_string()))),
    }
}

#[cfg(test)]
mod prove {
    use super::*;
    use p256::ecdsa::signature::Signer;

    fn cbor_mappa(voci: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0xa0 | voci.len() as u8];
        for (k, v) in voci {
            out.extend(k);
            out.extend(v);
        }
        out
    }

    fn cbor_byte(b: &[u8]) -> Vec<u8> {
        let mut out = if b.len() < 24 { vec![0x40 | b.len() as u8] } else if b.len() < 256 { vec![0x58, b.len() as u8] } else { vec![0x59, (b.len() >> 8) as u8, b.len() as u8] };
        out.extend(b);
        out
    }

    fn cbor_testo(s: &str) -> Vec<u8> {
        let mut out = vec![0x60 | s.len() as u8];
        out.extend(s.as_bytes());
        out
    }

    /// Un autenticatore in software (ES256, attestazione "none"), come test/autenticatore.ts.
    #[test]
    fn registrazione_e_accesso_di_un_autenticatore_es256() {
        let segreta = p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
        let punto = segreta.verifying_key().to_encoded_point(false);
        let cose = cbor_mappa(&[(vec![0x01], vec![0x02]), (vec![0x03], vec![0x26]), (vec![0x20], vec![0x01]), (vec![0x21], cbor_byte(punto.x().unwrap())), (vec![0x22], cbor_byte(punto.y().unwrap()))]);
        let id = [9u8; 32];
        let mut dati = super::super::cripto::sha256(b"oe.test");
        dati.push(0x01 | 0x04 | 0x40);
        dati.extend([0, 0, 0, 0]);
        dati.extend([0u8; 16]);
        dati.extend([0, 32]);
        dati.extend(id);
        dati.extend(&cose);
        let att = cbor_mappa(&[(cbor_testo("fmt"), cbor_testo("none")), (cbor_testo("attStmt"), vec![0xa0]), (cbor_testo("authData"), cbor_byte(&dati))]);
        let client = r#"{"type":"webauthn.create","challenge":"sfida","origin":"https://oe.test","crossOrigin":false}"#;
        let r = Json::analizza(&format!(
            r#"{{"id":"{0}","rawId":"{0}","type":"public-key","response":{{"clientDataJSON":"{1}","attestationObject":"{2}","transports":["internal"]}}}}"#,
            codifica_url(&id),
            codifica_url(client.as_bytes()),
            codifica_url(&att)
        ))
        .unwrap();
        let reg = verifica_registrazione(&r, "sfida", "https://oe.test", "oe.test").unwrap().unwrap();
        assert_eq!(reg.chiave, cose);
        assert_eq!(reg.tipo, "singleDevice");
        assert!(verifica_registrazione(&r, "altra", "https://oe.test", "oe.test").is_err());
        assert!(verifica_registrazione(&r, "sfida", "https://evil.test", "oe.test").is_err());
        assert!(verifica_registrazione(&r, "sfida", "https://oe.test", "evil.test").is_err());

        let mut dati = super::super::cripto::sha256(b"oe.test");
        dati.push(0x01 | 0x04);
        dati.extend([0, 0, 0, 1]);
        let client = r#"{"type":"webauthn.get","challenge":"sfida2","origin":"https://oe.test"}"#;
        let firma: p256::ecdsa::Signature = segreta.sign(&[dati.as_slice(), &super::super::cripto::sha256(client.as_bytes())].concat());
        let a = Json::analizza(&format!(
            r#"{{"id":"{0}","rawId":"{0}","type":"public-key","response":{{"clientDataJSON":"{1}","authenticatorData":"{2}","signature":"{3}"}}}}"#,
            codifica_url(&id),
            codifica_url(client.as_bytes()),
            codifica_url(&dati),
            codifica_url(firma.to_der().as_bytes())
        ))
        .unwrap();
        assert_eq!(verifica_accesso(&a, "sfida2", "https://oe.test", "oe.test", &reg.chiave, 0).unwrap(), (true, 1));
        // il contatore deve crescere: una credenziale copiata non passa
        assert!(verifica_accesso(&a, "sfida2", "https://oe.test", "oe.test", &reg.chiave, 1).is_err());
    }

    #[test]
    fn la_chiave_salvata_da_better_auth() {
        // la chiave pubblica di una passkey scritta dal gateway TypeScript (base64 della chiave COSE)
        let chiave = crate::b64::decodifica_node("pQECAyYgASFYIBMPqfMHK/yIBfP+CkzHNm6F7Ovln81HDO0h0OOgkvVOIlgg8I/d5HRxafqMAnbbtqs8PF3OZAAT28W4EIho0xGABn0=");
        let (cose, n) = primo(&chiave).unwrap();
        assert_eq!(n, chiave.len());
        assert_eq!(cose.prendi_intero(3).and_then(Cbor::intero), Some(-7));
    }
}
