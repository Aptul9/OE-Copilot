//! Il limite di richieste di Better Auth, in memoria: per indirizzo e percorso, una finestra che scorre. Le regole: 100 al minuto di norma,
//! 3 ogni 10 secondi su /sign-in, /sign-up, /change-password, /change-email e su /two-factor/*, e sopra quelle di auth.ts (30 al minuto
//! sull'accesso con la password e sui codici). Dietro sslh e docker-proxy ogni client ha lo stesso indirizzo: il freno vero è il blocco per
//! account (lockout.rs).

use crate::config::Modo2fa;
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;

const VOCI_MAX: usize = 100_000;

#[derive(Clone, Copy)]
struct Voce {
    conto: u32,
    ultima: i64,
    scade: i64,
}

thread_local! {
    static MEMORIA: RefCell<HashMap<String, Voce>> = RefCell::new(HashMap::new());
}

/// Dimentica tutto (per le prove).
pub fn azzera() {
    MEMORIA.with(|m| m.borrow_mut().clear());
}

/// getIP: X-Forwarded-For con un indirizzo solo e valido (senza proxy fidati), IPv6 ridotto alla sua /64. None se non si sa.
pub fn ip(xff: Option<&str>) -> Option<String> {
    let valori: Vec<&str> = xff?.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let [solo] = valori.as_slice() else { return None };
    let ip: IpAddr = solo.parse().ok()?;
    Some(match ip {
        IpAddr::V4(v) => v.to_string(),
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let g = v.segments();
                (0..8).map(|i| format!("{:04x}", if i < 4 { g[i] } else { 0 })).collect::<Vec<_>>().join(":")
            }
        },
    })
}

/// (finestra in secondi, massimo) per un percorso di /api/auth.
fn regola(percorso: &str, modo: Modo2fa) -> (i64, u32) {
    let mut r = (60, 100);
    if percorso.starts_with("/sign-in") || percorso.starts_with("/sign-up") || percorso.starts_with("/change-password") || percorso.starts_with("/change-email") {
        r = (10, 3);
    }
    if modo != Modo2fa::Off && percorso.starts_with("/two-factor/") {
        r = (10, 3);
    }
    if percorso == "/sign-in/email" || (modo != Modo2fa::Off && (percorso == "/two-factor/verify-totp" || percorso == "/two-factor/verify-backup-code")) {
        r = (60, 30);
    }
    r
}

/// normalizePathname: senza /api/auth davanti e senza le barre in fondo.
pub fn percorso_normalizzato(pathname: &str) -> String {
    let p = pathname.trim_end_matches('/');
    let p = if p.is_empty() { "/" } else { p };
    if p == "/api/auth" {
        return "/".to_string();
    }
    match p.strip_prefix("/api/auth/") {
        Some(resto) => {
            let r = format!("/{resto}");
            let r = r.trim_end_matches('/');
            if r.is_empty() { "/".to_string() } else { r.to_string() }
        }
        None => p.to_string(),
    }
}

/// Un passo del limite: None se la richiesta passa, Some(secondi da aspettare) se no.
pub fn consuma(pathname: &str, xff: Option<&str>, modo: Modo2fa, ora: i64) -> Option<i64> {
    let percorso = percorso_normalizzato(pathname);
    let chiave = format!("{}|{percorso}", ip(xff).unwrap_or_else(|| "no-trusted-ip".to_string()));
    let (finestra, massimo) = regola(&percorso, modo);
    MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|_, v| ora < v.scade);
        if m.len() > VOCI_MAX {
            let troppe = m.len() - VOCI_MAX;
            let chiavi: Vec<String> = m.keys().take(troppe).cloned().collect();
            for k in chiavi {
                m.remove(&k);
            }
        }
        let prima = m.get(&chiave).copied().filter(|v| ora < v.scade);
        let (conto, permesso, attesa) = match prima {
            None => (1, true, None),
            Some(v) if ora - v.ultima >= finestra * 1000 => (1, true, None),
            Some(v) if v.conto >= massimo => (v.conto, false, Some(((v.ultima + finestra * 1000 - ora) as f64 / 1000.0).ceil() as i64)),
            Some(v) => (v.conto + 1, true, None),
        };
        if permesso {
            m.insert(chiave, Voce { conto, ultima: ora, scade: ora + finestra * 1000 });
        }
        attesa
    })
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn indirizzo_da_x_forwarded_for() {
        assert_eq!(ip(Some("172.18.0.1")), Some("172.18.0.1".into()));
        assert_eq!(ip(Some("1.2.3.4, 5.6.7.8")), None);
        assert_eq!(ip(Some("::ffff:1.2.3.4")), Some("1.2.3.4".into()));
        assert_eq!(ip(Some("2001:DB8::1")), Some("2001:0db8:0000:0000:0000:0000:0000:0000".into()));
        assert_eq!(ip(Some("01.2.3.4")), None);
        assert_eq!(ip(None), None);
    }

    #[test]
    fn tre_ogni_dieci_secondi_sui_codici() {
        azzera();
        for _ in 0..3 {
            assert_eq!(consuma("/api/auth/two-factor/enable", Some("1.1.1.1"), Modo2fa::Totp, 1000), None);
        }
        assert_eq!(consuma("/api/auth/two-factor/enable", Some("1.1.1.1"), Modo2fa::Totp, 2000), Some(9));
        assert_eq!(consuma("/api/auth/two-factor/enable", Some("1.1.1.1"), Modo2fa::Totp, 11_000), None);
        for _ in 0..30 {
            assert_eq!(consuma("/api/auth/sign-in/email", None, Modo2fa::Off, 1000), None);
        }
        assert!(consuma("/api/auth/sign-in/email", None, Modo2fa::Off, 1000).is_some());
    }
}
