//! I cookie di Better Auth: i nomi (con __Secure- quando APP_URL è https), gli attributi (HttpOnly, SameSite=Lax, Path=/), la scrittura di
//! Set-Cookie come better-call e la lettura dei cookie firmati (valore.firma, firma HMAC di 44 caratteri).

use super::cripto::{firma, firma_valida};

pub const SESSIONE: &str = "session_token";
pub const DATI_SESSIONE: &str = "session_data";
pub const NON_RICORDARE: &str = "dont_remember";
pub const DUE_FATTORI: &str = "two_factor";
pub const DISPOSITIVO_FIDATO: &str = "trust_device";
pub const SFIDA_PASSKEY: &str = "better-auth-passkey";

/// I giorni di una sessione e quanto vale il suo cookie.
pub const SCADENZA_S: i64 = 60 * 60 * 24 * 30;

/// Un cookie da scrivere: nome completo, valore già pronto per la riga, durata (None: fino alla chiusura del browser).
#[derive(Clone, Debug)]
pub struct Biscotto {
    pub nome: String,
    pub max_age: Option<i64>,
}

/// Se i cookie hanno il prefisso __Secure- e l'attributo Secure: APP_URL in https.
pub fn sicuri() -> bool {
    crate::config::app_url().starts_with("https://")
}

/// Il nome completo di un cookie: __Secure-better-auth.<nome>.
pub fn nome(cookie: &str) -> String {
    format!("{}better-auth.{cookie}", if sicuri() { "__Secure-" } else { "" })
}

/// encodeURIComponent di JavaScript.
pub fn codifica_componente(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// decodeURIComponent di JavaScript: None se una sequenza % è rotta o non è UTF-8.
pub fn decodifica_componente(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(h, 16).ok().filter(|_| h.bytes().all(|c| c.is_ascii_hexdigit()))?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// tryDecode: decodeURIComponent se c'è un %, e se non riesce il testo com'è.
pub fn prova_a_decodificare(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    decodifica_componente(s).unwrap_or_else(|| s.to_string())
}

/// La riga Set-Cookie: nome=valore; Max-Age; Path; HttpOnly; Secure; SameSite=Lax.
fn riga(nome: &str, valore: &str, max_age: Option<i64>) -> String {
    let mut c = format!("{nome}={valore}");
    if let Some(m) = max_age.filter(|m| *m >= 0) {
        c.push_str(&format!("; Max-Age={m}"));
    }
    c.push_str("; Path=/; HttpOnly");
    if sicuri() || nome.starts_with("__Secure-") {
        c.push_str("; Secure");
    }
    c.push_str("; SameSite=Lax");
    c
}

/// serializeCookie: il valore passa da encodeURIComponent.
pub fn scrivi(nome: &str, valore: &str, max_age: Option<i64>) -> String {
    riga(nome, &codifica_componente(valore), max_age)
}

/// serializeSignedCookie: valore.firma, poi encodeURIComponent di tutto.
pub fn scrivi_firmato(nome: &str, valore: &str, segreto: &str, max_age: Option<i64>) -> String {
    riga(nome, &codifica_componente(&format!("{valore}.{}", firma(valore, segreto))), max_age)
}

/// Un cookie che scade adesso (expireCookie): stesso nome e attributi, Max-Age=0.
pub fn scaduto(nome: &str) -> String {
    riga(nome, "", Some(0))
}

/// parseCookies di better-call: chiave=valore separati da ";", vale la prima di due chiavi uguali, le virgolette intorno si tolgono, il
/// valore si decodifica se ha un %.
pub fn leggi_tutti(intestazione: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    let s = intestazione;
    while i < s.len() {
        let Some(uguale) = s[i..].find('=').map(|x| i + x) else { break };
        let fine = match s[i..].find(';').map(|x| i + x) {
            None => s.len(),
            Some(f) if f < uguale => {
                i = s[..uguale].rfind(';').map_or(0, |x| x + 1);
                continue;
            }
            Some(f) => f,
        };
        let chiave = crate::py::js_trim(&s[i..uguale]).to_string();
        if !out.iter().any(|(k, _)| *k == chiave) {
            let mut valore = crate::py::js_trim(&s[uguale + 1..fine]).to_string();
            if valore.starts_with('"') {
                let v: Vec<char> = valore.chars().collect();
                valore = v.get(1..v.len().saturating_sub(1).max(1)).map(|x| x.iter().collect()).unwrap_or_default();
            }
            out.push((chiave, prova_a_decodificare(&valore)));
        }
        i = fine + 1;
    }
    out
}

pub fn leggi(intestazione: Option<&str>, nome: &str) -> Option<String> {
    leggi_tutti(intestazione?).into_iter().find(|(k, _)| k == nome).map(|(_, v)| v)
}

/// Il valore di un cookie firmato: None se manca o non ha la forma di un valore firmato, Some(None) se la firma non torna (false di
/// better-call), Some(Some(valore)) se è buono.
pub fn leggi_firmato(intestazione: Option<&str>, nome: &str, segreto: &str) -> Option<Option<String>> {
    let v = leggi(intestazione, nome).filter(|v| !v.is_empty())?;
    let punto = v.rfind('.')?;
    if punto < 1 {
        return None;
    }
    let (valore, firma_b64) = (&v[..punto], &v[punto + 1..]);
    if firma_b64.len() != 44 || !firma_b64.ends_with('=') {
        return None;
    }
    Some(firma_valida(valore, firma_b64, segreto).then(|| valore.to_string()))
}

/// Il valore buono di un cookie firmato, o None.
pub fn firmato(intestazione: Option<&str>, nome: &str, segreto: &str) -> Option<String> {
    leggi_firmato(intestazione, nome, segreto).flatten()
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn lettura_come_better_call() {
        let c = leggi_tutti("a=1; b=\"x y\"; a=2;c=%41%; d=%E2%82%AC");
        assert_eq!(c, vec![("a".into(), "1".into()), ("b".into(), "x y".into()), ("c".into(), "%41%".into()), ("d".into(), "€".into())]);
        assert_eq!(codifica_componente("a+b/c=."), "a%2Bb%2Fc%3D.");
    }

    #[test]
    fn un_cookie_firmato() {
        let s = "s".repeat(48);
        let riga = scrivi_firmato("x", "tok", &s, Some(10));
        let valore = riga.split(';').next().unwrap().trim_start_matches("x=");
        assert_eq!(firmato(Some(&format!("x={valore}")), "x", &s), Some("tok".into()));
        assert_eq!(leggi_firmato(Some("x=tok.inventato"), "x", &s), None);
        let falsa = format!("x={}", codifica_componente(&format!("tok.{}", firma("altro", &s))));
        assert_eq!(leggi_firmato(Some(&falsa), "x", &s), Some(None));
    }
}
