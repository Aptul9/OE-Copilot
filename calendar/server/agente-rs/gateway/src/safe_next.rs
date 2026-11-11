//! Dove andare dopo l'accesso, restando su questo sito (era lib/safe-next.ts). `?next=` è quello che c'era nella barra degli indirizzi: non
//! basta controllare che inizi con `/`, perché i browser leggono `/\evil.com` come `//evil.com` e scartano tab, CR e LF. La destinazione si
//! risolve contro un'origine finta e deve restare lì.

use crate::auth::cookie::codifica_componente;

pub fn safe_next(next: Option<&str>) -> String {
    let Some(n) = next.filter(|n| n.starts_with('/') && !n.starts_with("//")) else { return "/".to_string() };
    if n.chars().any(|c| (c as u32) < 0x20 || c == '\\') {
        return "/".to_string();
    }
    let Ok(base) = url::Url::parse("http://gateway.invalid") else { return "/".to_string() };
    match base.join(n) {
        Ok(u) if u.origin() == base.origin() => {
            let mut s = u.path().to_string();
            if let Some(q) = u.query().filter(|q| !q.is_empty()) {
                s.push('?');
                s.push_str(q);
            }
            if let Some(f) = u.fragment().filter(|f| !f.is_empty()) {
                s.push('#');
                s.push_str(f);
            }
            s
        }
        _ => "/".to_string(),
    }
}

/// La pagina di accesso, che poi riporta su `next`.
pub fn login_url(next: &str) -> String {
    format!("/login?next={}", codifica_componente(next))
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn solo_questo_sito() {
        assert_eq!(safe_next(Some("/calendario?x=1#y")), "/calendario?x=1#y");
        for no in ["//evil.com", "/\\evil.com", "/\tevil", "https://evil.com", "evil", ""] {
            assert_eq!(safe_next(Some(no)), "/", "{no}");
        }
        assert_eq!(safe_next(Some("/a/../b")), "/b");
        assert_eq!(login_url("/x y"), "/login?next=%2Fx%20y");
    }
}
