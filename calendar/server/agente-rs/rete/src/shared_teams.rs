//! Lo stato degli account Teams che TeamsRelay manda all'agente ogni minuto (POST /api/auth/shared/teams), perché il calendario mostri
//! accanto a ogni casella se Teams la usa e come sta, e gli account Teams che hanno solo il loro accesso del browser. Sta in memoria: un
//! riavvio lo perde fino al giro dopo. Un rapporto più vecchio di STANTIO_S non si mostra.
//!   {"accounts": [{"account", "slot", "state", "detail", "calls"?}], "others": [{"email", "slot", "state", "detail", "calls"?}]}
//! "calls" (true or false) is there only for an account without a browser, whose calls can be switched; no field for the others.

use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use regex::Regex;
use std::sync::LazyLock;

pub const STANTIO_S: i64 = 600;
const MAX_VOCI: usize = 64;
static STATO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z-]{0,23}$").expect("valida"));

#[derive(Clone, Debug, PartialEq)]
pub struct StatoTeams {
    pub slot: i64,
    pub state: String,
    pub detail: String,
    pub calls: Option<bool>,
}

impl StatoTeams {
    pub fn in_json(&self, email: Option<&str>) -> Json {
        let mut m = Mappa::new();
        if let Some(e) = email {
            m.insert("email", e);
        }
        m.insert("slot", self.slot);
        m.insert("state", self.state.as_str());
        m.insert("detail", self.detail.as_str());
        if let Some(c) = self.calls {
            m.insert("calls", c);
        }
        Json::Obj(m)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RapportoTeams {
    pub at: i64,
    pub accounts: IndexMap<String, StatoTeams>,
    pub others: Vec<(String, StatoTeams)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErroreRapportoTeams(pub String);

impl std::fmt::Display for ErroreRapportoTeams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn rifiuta<T>(motivo: impl Into<String>) -> Result<T, ErroreRapportoTeams> {
    Err(ErroreRapportoTeams(motivo.into()))
}

/// `String.prototype.slice(0, n)` in unità UTF-16.
fn primi_utf16(s: &str, n: usize) -> String {
    let mut usate = 0;
    s.chars()
        .take_while(|c| {
            usate += c.len_utf16();
            usate <= n
        })
        .collect()
}

fn intero(v: Option<&Json>) -> Option<i64> {
    match v? {
        Json::Num(n) if n.is_finite() && n.fract() == 0.0 => Some(*n as i64),
        _ => None,
    }
}

fn voce(dato: &Json, campo: &str) -> Result<(String, StatoTeams), ErroreRapportoTeams> {
    let Json::Obj(r) = dato else { return rifiuta("ogni voce è un oggetto") };
    let email = match r.get(campo) {
        Some(Json::Str(s)) => s.trim_matches(spazio_js).to_lowercase(),
        _ => String::new(),
    };
    // un account Teams senza identità ancora letta (appena aggiunto) non ha indirizzo: va bene solo tra gli altri
    if !(campo == "email" && email.is_empty()) && !email_valida(&email) {
        return rifiuta(format!("{campo} non valido"));
    }
    let slot = match intero(r.get("slot")) {
        Some(s) if (1..=9999).contains(&s) => s,
        _ => return rifiuta("slot non valido"),
    };
    let state = match r.get("state") {
        Some(Json::Str(s)) if STATO.is_match(s) => s.clone(),
        _ => return rifiuta("state non valido"),
    };
    let detail = match r.get("detail") {
        Some(Json::Str(s)) => primi_utf16(&s.chars().map(|c| if (c as u32) < 0x20 || c == '\x7f' { ' ' } else { c }).collect::<String>(), 200),
        _ => String::new(),
    };
    Ok((email, StatoTeams { slot, state, detail, calls: r.get("calls").and_then(Json::as_bool) }))
}

/// `\s` di JavaScript nella classe di EMAIL: gli spazi di trim più nessun altro.
fn spazio_js(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// `/^[^\s@]{1,128}@[^\s@]{1,253}$/`: le lunghezze in unità UTF-16, gli spazi quelli di JavaScript.
fn email_valida(email: &str) -> bool {
    let Some((locale, dominio)) = email.split_once('@') else { return false };
    let conta = |s: &str| s.encode_utf16().count();
    !email.chars().any(spazio_js) && !dominio.contains('@') && (1..=128).contains(&conta(locale)) && (1..=253).contains(&conta(dominio))
}

/// Il corpo del rapporto, controllato: elenchi brevi, indirizzi, slot, stati come parole. Nessun altro campo.
pub fn leggi_rapporto_teams(corpo: &Json, adesso_s: i64) -> Result<RapportoTeams, ErroreRapportoTeams> {
    let Json::Obj(r) = corpo else { return rifiuta("serve {\"accounts\": [...], \"others\": [...]}") };
    let estranei: Vec<&str> = r.keys().map(String::as_str).filter(|k| *k != "accounts" && *k != "others").collect();
    if !estranei.is_empty() {
        return rifiuta(format!("campi non ammessi: {}", primi_utf16(&estranei.join(", "), 80)));
    }
    let elenco = |k: &str| -> Result<Vec<Json>, ErroreRapportoTeams> {
        match r.get(k) {
            None | Some(Json::Null) => Ok(Vec::new()),
            Some(Json::Arr(v)) if v.len() <= MAX_VOCI => Ok(v.clone()),
            _ => rifiuta(format!("{k}: un elenco di al massimo {MAX_VOCI} voci")),
        }
    };
    let mut accounts = IndexMap::new();
    for v in elenco("accounts")? {
        let (email, stato) = voce(&v, "account")?;
        accounts.insert(email, stato);
    }
    let others = elenco("others")?.iter().map(|v| voce(v, "email")).collect::<Result<Vec<_>, _>>()?;
    Ok(RapportoTeams { at: adesso_s, accounts, others })
}

/// Il rapporto se è recente, altrimenti nessuno: un TeamsRelay fermo non lascia stati vecchi sulla pagina.
pub fn rapporto_valido(r: Option<&RapportoTeams>, adesso_s: i64) -> Option<&RapportoTeams> {
    r.filter(|r| adesso_s - r.at <= STANTIO_S)
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn rapporto_controllato() {
        let j = |t: &str| Json::analizza(t).unwrap();
        let r = leggi_rapporto_teams(&j(r#"{"accounts":[{"account":" A@B.it ","slot":2,"state":"ok","detail":"x\ny","calls":true}],"others":[{"email":"","slot":3,"state":"signed-out"}]}"#), 100).unwrap();
        assert_eq!(r.accounts["a@b.it"], StatoTeams { slot: 2, state: "ok".into(), detail: "x y".into(), calls: Some(true) });
        assert_eq!(r.others[0].0, "");
        assert_eq!(leggi_rapporto_teams(&j(r#"{"x":1}"#), 0).unwrap_err().0, "campi non ammessi: x");
        assert_eq!(leggi_rapporto_teams(&j(r#"{"accounts":[{"account":"","slot":1,"state":"ok"}]}"#), 0).unwrap_err().0, "account non valido");
        assert_eq!(leggi_rapporto_teams(&j(r#"{"others":[{"email":"a@b","slot":0,"state":"ok"}]}"#), 0).unwrap_err().0, "slot non valido");
        assert_eq!(leggi_rapporto_teams(&j(r#"[]"#), 0).unwrap_err().0, "serve {\"accounts\": [...], \"others\": [...]}");
        assert!(rapporto_valido(Some(&r), 700).is_some() && rapporto_valido(Some(&r), 701).is_none());
    }
}
