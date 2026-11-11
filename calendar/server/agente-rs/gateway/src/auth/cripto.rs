//! La crittografia di Better Auth 1.7, gli stessi byte: le password (scrypt N=16384 r=16 p=1, 64 byte, sale di 16 byte scritto in esadecimale
//! e usato come testo), la firma dei cookie (HMAC-SHA256 in base64), la cifratura dei segreti del TOTP e dei codici di recupero (XChaCha20-
//! Poly1305 con la chiave SHA-256 del segreto, nonce davanti, tutto in esadecimale), i codici TOTP (RFC 6238, HMAC-SHA1, 6 cifre, 30 secondi)
//! e i testi casuali degli identificativi. Un gateway.db scritto da Better Auth si legge qui, e viceversa.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

type HmacSha256 = Hmac<Sha256>;
type HmacSha1 = Hmac<Sha1>;

pub fn casuali(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("generatore casuale del sistema");
    b
}

pub fn esadecimale(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn da_esadecimale(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

const MINUSCOLE: &str = "abcdefghijklmnopqrstuvwxyz";
const MAIUSCOLE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const CIFRE: &str = "0123456789";

/// createRandomStringGenerator: lettere e cifre scelte scartando i byte che sbilancerebbero la distribuzione.
pub fn testo_casuale(lunghezza: usize, alfabeto: &str) -> String {
    let caratteri: Vec<char> = alfabeto.chars().collect();
    let n = caratteri.len();
    let massimo = (256 / n) * n;
    let mut out = String::with_capacity(lunghezza);
    while out.chars().count() < lunghezza {
        for b in casuali(lunghezza * 2) {
            if (b as usize) < massimo {
                out.push(caratteri[b as usize % n]);
                if out.chars().count() == lunghezza {
                    break;
                }
            }
        }
    }
    out
}

/// generateRandomString di better-auth/crypto: a-z, 0-9, A-Z, -_
pub fn testo_casuale_auth(lunghezza: usize) -> String {
    testo_casuale(lunghezza, &format!("{MINUSCOLE}{CIFRE}{MAIUSCOLE}-_"))
}

/// generateRandomString(n, "a-z", "0-9", "A-Z")
pub fn testo_casuale_alfanumerico(lunghezza: usize) -> String {
    testo_casuale(lunghezza, &format!("{MINUSCOLE}{CIFRE}{MAIUSCOLE}"))
}

/// generateRandomString(n, "a-z", "0-9")
pub fn testo_casuale_minuscolo(lunghezza: usize) -> String {
    testo_casuale(lunghezza, &format!("{MINUSCOLE}{CIFRE}"))
}

/// generateId: 32 caratteri a-z, A-Z, 0-9 (gli identificativi delle righe e i token delle sessioni).
pub fn genera_id() -> String {
    testo_casuale(32, &format!("{MINUSCOLE}{MAIUSCOLE}{CIFRE}"))
}

fn chiave_scrypt(password: &str, sale: &str) -> Vec<u8> {
    let normalizzata: String = password.nfkc().collect();
    let parametri = scrypt::Params::new(14, 16, 1, 64).expect("parametri fissi");
    let mut out = vec![0u8; 64];
    scrypt::scrypt(normalizzata.as_bytes(), sale.as_bytes(), &parametri, &mut out).expect("lunghezza fissa");
    out
}

/// hashPassword: "<sale esadecimale>:<chiave esadecimale>". Costa 32 MB e qualche decina di millisecondi: va fuori dal thread del server.
pub fn hash_password(password: &str) -> String {
    let sale = esadecimale(&casuali(16));
    let chiave = chiave_scrypt(password, &sale);
    format!("{sale}:{}", esadecimale(&chiave))
}

/// verifyPassword: Err se l'hash non ha la forma "sale:chiave".
pub fn verifica_password(hash: &str, password: &str) -> Result<bool, String> {
    let mut parti = hash.split(':');
    let (Some(sale), Some(chiave)) = (parti.next().filter(|s| !s.is_empty()), parti.next().filter(|s| !s.is_empty())) else { return Err("Invalid password hash".to_string()) };
    Ok(esadecimale(&chiave_scrypt(password, sale)) == chiave)
}

/// La firma di un cookie: HMAC-SHA256 del valore con il segreto, in base64 con il riempimento (44 caratteri).
pub fn firma(valore: &str, segreto: &str) -> String {
    let mut m = <HmacSha256 as Mac>::new_from_slice(segreto.as_bytes()).expect("chiave di qualunque lunghezza");
    m.update(valore.as_bytes());
    crate::b64::codifica(&m.finalize().into_bytes())
}

/// La firma di un cookie letto: confronto in tempo costante della firma in base64 con quella giusta.
pub fn firma_valida(valore: &str, firma_b64: &str, segreto: &str) -> bool {
    use base64::Engine;
    let Ok(data) = base64::engine::general_purpose::STANDARD.decode(firma_b64) else { return false };
    let mut m = <HmacSha256 as Mac>::new_from_slice(segreto.as_bytes()).expect("chiave di qualunque lunghezza");
    m.update(valore.as_bytes());
    m.verify_slice(&data).is_ok()
}

/// createHMAC("SHA-256", "base64urlnopad").sign(segreto, dati)
pub fn hmac_base64url(segreto: &str, dati: &str) -> String {
    let mut m = <HmacSha256 as Mac>::new_from_slice(segreto.as_bytes()).expect("chiave di qualunque lunghezza");
    m.update(dati.as_bytes());
    crate::b64::codifica_url(&m.finalize().into_bytes())
}

fn cifratore(segreto: &str) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(&Sha256::digest(segreto.as_bytes())).expect("32 byte")
}

/// symmetricEncrypt con una chiave di testo: nonce (24 byte), testo cifrato e etichetta, in esadecimale.
pub fn cifra(segreto: &str, dati: &str) -> String {
    let nonce = casuali(24);
    let cifrato = cifratore(segreto).encrypt(XNonce::from_slice(&nonce), dati.as_bytes()).expect("cifratura in memoria");
    esadecimale(&[nonce, cifrato].concat())
}

/// symmetricDecrypt: Err se il testo non è esadecimale, è corto o non è stato cifrato con questo segreto.
pub fn decifra(segreto: &str, esa: &str) -> Result<String, String> {
    let dati = da_esadecimale(esa).ok_or("hex non valido")?;
    if dati.len() < 24 + 16 {
        return Err("invalid ciphertext length".to_string());
    }
    let (nonce, resto) = dati.split_at(24);
    let chiaro = cifratore(segreto).decrypt(XNonce::from_slice(nonce), resto).map_err(|_| "invalid tag".to_string())?;
    Ok(String::from_utf8_lossy(&chiaro).into_owned())
}

/// HOTP (RFC 4226) con la chiave in byte: le cifre del contatore.
pub fn hotp(chiave: &[u8], contatore: u64, cifre: u32) -> String {
    let mut m = <HmacSha1 as Mac>::new_from_slice(chiave).expect("chiave di qualunque lunghezza");
    m.update(&contatore.to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[h.len() - 1] & 15) as usize;
    let n = (u32::from(h[o] & 127) << 24) | (u32::from(h[o + 1]) << 16) | (u32::from(h[o + 2]) << 8) | u32::from(h[o + 3]);
    format!("{:0width$}", n % 10u32.pow(cifre), width = cifre as usize)
}

/// verifyTOTP: il codice del passo di adesso o di quelli accanto (finestra 1), confrontati tutti in tempo costante.
pub fn verifica_totp(codice: &str, segreto: &str, adesso_ms: i64) -> bool {
    let contatore = adesso_ms.div_euclid(30_000);
    let mut trovato = false;
    for d in -1..=1i64 {
        let atteso = hotp(segreto.as_bytes(), (contatore + d) as u64, 6);
        let mut diversi = (codice.encode_utf16().count() ^ atteso.len()) as u32;
        let c: Vec<u16> = codice.encode_utf16().collect();
        for (i, a) in atteso.bytes().enumerate() {
            diversi |= u32::from(c.get(i).copied().unwrap_or(0) ^ u16::from(a));
        }
        trovato = (diversi == 0) || trovato;
    }
    trovato
}

/// base32 di RFC 4648, maiuscolo, senza "=".
pub fn base32(dati: &[u8]) -> String {
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut accumulo = 0u32;
    let mut bit = 0;
    for &b in dati {
        accumulo = (accumulo << 8) | u32::from(b);
        bit += 8;
        while bit >= 5 {
            bit -= 5;
            out.push(A[((accumulo >> bit) & 31) as usize] as char);
        }
        accumulo &= (1 << bit) - 1;
    }
    if bit > 0 {
        out.push(A[((accumulo << (5 - bit)) & 31) as usize] as char);
    }
    out
}

pub fn sha256(dati: &[u8]) -> Vec<u8> {
    Sha256::digest(dati).to_vec()
}

#[cfg(test)]
mod prove {
    use super::*;

    // Da un gateway.db scritto dal gateway TypeScript (Better Auth 1.7.7)
    const SEGRETO: &str = "5a086d287157b50af22394627fd09e1f29189dcf7684814c";

    #[test]
    fn la_password_di_better_auth() {
        let hash = "4f8c51c767a8891ea8cad9c2ef2aa3c9:b5db7e10e29250584b3272e8fdb5091a1877c49a0f514239001bff1650a5a996b804a3319deaaf81ba636db0f8efdc9cf6872842fc2be1d6cd77d3567ae6d6e3";
        assert_eq!(verifica_password(hash, "dcd7c680f0fbec96ecd7cdc8"), Ok(true));
        assert_eq!(verifica_password(hash, "dcd7c680f0fbec96ecd7cdc9"), Ok(false));
        assert!(verifica_password("solo", "x").is_err());
        let nuovo = hash_password("una password");
        assert_eq!(verifica_password(&nuovo, "una password"), Ok(true));
    }

    #[test]
    fn la_firma_del_cookie() {
        assert_eq!(firma("2Q3MUFMbiFJMBwvYnWdDCEtShJbdeDTW", SEGRETO), "C+M07+YCg8017knRmOytMy+Ey0K1LpV2tb35dtHseUg=");
        assert!(firma_valida("2Q3MUFMbiFJMBwvYnWdDCEtShJbdeDTW", "C+M07+YCg8017knRmOytMy+Ey0K1LpV2tb35dtHseUg=", SEGRETO));
        assert!(!firma_valida("2Q3MUFMbiFJMBwvYnWdDCEtShJbdeDTX", "C+M07+YCg8017knRmOytMy+Ey0K1LpV2tb35dtHseUg=", SEGRETO));
    }

    #[test]
    fn il_segreto_del_totp_cifrato() {
        let cifrato = "2e84a63bc8353f8183f5a7a2f379910995c9d1cf1c3bcd0b8fc520d23ce51a6025bb5256d47648dd0da3e57c5e5d88d47c8049bf0da60688150434a941f3ac79be15ddc4e1203179";
        let chiaro = decifra(SEGRETO, cifrato).unwrap();
        assert_eq!(base32(chiaro.as_bytes()), "GRGVKRTJGJDVULKNGJLG4U32JFVEGVSJJZ3HG6KBPAWVMRKSIE2A");
        assert!(decifra("altro segreto lungo almeno trentadue caratteri", cifrato).is_err());
        assert_eq!(decifra(SEGRETO, &cifra(SEGRETO, "x")).unwrap(), "x");
    }

    #[test]
    fn hotp_di_rfc_4226() {
        let k = b"12345678901234567890";
        let attesi = ["755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871", "520489"];
        for (i, a) in attesi.iter().enumerate() {
            assert_eq!(hotp(k, i as u64, 6), *a);
        }
    }
}
