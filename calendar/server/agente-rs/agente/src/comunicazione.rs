//! Gateway di sola lettura su posta e calendari Outlook per chi non deve avere i token (l'MCP della posta): l'agente tiene le credenziali,
//! le rinnova e fa la richiesta; fuori escono solo la risposta di Outlook e i nomi delle caselle.
//!   GET  /api/communication/accounts  {"accounts": [{"upn", "status", "message"}]}: le caselle Microsoft con accesso, senza le rimosse
//!   POST /api/communication/read      {"account": casella esatta, "path": percorso REST relativo[, "prefer": 'outlook.timezone="Zona"']}
//! Solo GET verso https://outlook.office.com/api/v2.0, solo i percorsi di lettura di posta e calendari elencati qui sotto, solo i parametri
//! OData elencati, nessuna intestazione oltre al Prefer del fuso.

use crate::diretto::GestoreToken;
use oe_nucleo::json::{Json, Mappa};
use oe_rete::errori::ErroreAccesso;
use oe_rete::js::{lunghezza_utf16, primi_utf16, spazio_js, stringa_di, vero};
use oe_rete::outlook_rest::{ErroreDyn, ErroreRest, FonteToken, FutureToken, OUTLOOK_REST, OpzioniRest, claims_jwt, get_rest, upn_di};
use oe_rete::trasporto::{ErroreRete, Intestazioni};
use regex::Regex;
use std::fmt;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

pub const MAX_PERCORSO: usize = 4096;
/// 8 MiB, come il trasporto dell'MCP (HTTP e SSH): un allegato più grande torna 502 e si legge da Outlook stesso.
pub const MAX_RISPOSTA: usize = 8 * 1024 * 1024;
const MAX_TOP: u32 = 1000;

/// Errore con il codice HTTP da dare a chi chiama; il messaggio va in {"errore": ...}.
#[derive(Debug, Clone, PartialEq)]
pub struct ErroreComunicazione {
    pub codice: u16,
    pub messaggio: String,
    pub intestazioni: Vec<(String, String)>,
}

impl fmt::Display for ErroreComunicazione {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.messaggio)
    }
}

impl std::error::Error for ErroreComunicazione {}

fn errore(codice: u16, m: impl Into<String>) -> ErroreComunicazione {
    ErroreComunicazione { codice, messaggio: m.into(), intestazioni: Vec::new() }
}

fn rifiuta<T>(motivo: impl Into<String>) -> Result<T, ErroreComunicazione> {
    Err(errore(400, motivo))
}

// Le forme dei percorsi ammessi: segmenti fissi (minuscoli, il confronto ignora le maiuscole) e "*" per un identificativo.
const FORME: [&[&str]; 20] = [
    &["me", "mailfolders"],
    &["me", "mailfolders", "*"],
    &["me", "mailfolders", "*", "childfolders"],
    &["me", "mailfolders", "*", "messages"],
    &["me", "messages"],
    &["me", "messages", "*"],
    &["me", "messages", "*", "attachments"],
    &["me", "messages", "*", "attachments", "*"],
    &["me", "calendars"],
    &["me", "calendars", "*"],
    &["me", "calendars", "*", "calendarview"],
    &["me", "calendars", "*", "events"],
    &["me", "calendar"],
    &["me", "calendar", "calendarview"],
    &["me", "calendar", "events"],
    &["me", "calendarview"],
    &["me", "events"],
    &["me", "events", "*"],
    &["me", "events", "*", "instances"],
    &["me", "events", "*", "attachments"],
];

const PARAMETRI: [&str; 11] = ["$top", "$skip", "$select", "$filter", "$orderby", "$search", "$expand", "$count", "$skiptoken", "startdatetime", "enddatetime"];
// Gli @odata.nextLink di Outlook sono indirizzi interi: si accettano solo con questa base e si chiedono sempre a OUTLOOK_REST.
const BASI_ASSOLUTE: [&str; 2] = ["https://outlook.office.com/api/v2.0/", "https://outlook.office365.com/api/v2.0/"];
static IDENTIFICATIVO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[A-Za-z0-9._~!$&'()*+,;=:@-]|%[0-9A-Fa-f]{2})+$").expect("valida"));
// La chiave OData di un nextLink, MailFolders('AAMk...='): diventa MailFolders/AAMk...= prima del confronto con le forme.
static CHIAVE_ODATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z]+)\('((?:[A-Za-z0-9._~!$&*+,;=:@-]|%[0-9A-Fa-f]{2})+)'\)$").expect("valida"));
static PREFER_FUSO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^outlook\.timezone=\x22([A-Za-z0-9 _+./-]{1,64})\x22$").expect("valida"));
static CALENDARIO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^me/(?:calendars?|calendarview|events)(?:[/(?]|$)").expect("valida"));
static CODICE_OUTLOOK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_.]{0,63}$").expect("valida"));

/// `decodeURIComponent`: ogni %XX è un byte, i byte devono essere UTF-8.
pub fn decodifica_uri(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1..i + 3).filter(|h| h.iter().all(u8::is_ascii_hexdigit))?;
            out.push(u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn decodifica(s: &str) -> Result<String, ErroreComunicazione> {
    decodifica_uri(s).map_or_else(|| rifiuta("codifica % non valida"), Ok)
}

/// Un identificativo di Outlook in un solo segmento. Gli Id sono base64 e possono avere "/" (mandato come %2F, come fa encodeURIComponent):
/// si accetta, ma nessun pezzo tra due "/" può essere "." o "..", né iniziare con "$" ($value, $count, $ref non sono letture JSON).
fn identificativo_valido(s: &str) -> Result<bool, ErroreComunicazione> {
    if !IDENTIFICATIVO.is_match(s) {
        return Ok(false);
    }
    let chiaro = decodifica(s)?;
    if chiaro.starts_with('$') || chiaro.chars().any(|c| (c as u32) < 0x20 || c == '\x7f' || c == '\\' || c == '?' || c == '#') {
        return Ok(false);
    }
    Ok(chiaro.split('/').all(|p| p != "." && p != ".."))
}

/// Il percorso relativo da chiedere a OUTLOOK_REST, se è una lettura ammessa; altrimenti ErroreComunicazione 400 con il motivo.
pub fn percorso_lettura(dato: Option<&Json>) -> Result<String, ErroreComunicazione> {
    let Some(Json::Str(dato)) = dato.filter(|d| vero(d)) else { return rifiuta("manca path") };
    if lunghezza_utf16(dato) > MAX_PERCORSO {
        return rifiuta(format!("path oltre {MAX_PERCORSO} caratteri"));
    }
    let mut p: &str = dato;
    let base = BASI_ASSOLUTE.iter().find(|b| p.get(..b.len()).is_some_and(|x| x.to_lowercase() == **b));
    p = match base {
        Some(b) => &p[b.len()..],
        None => p.trim_start_matches('/'),
    };
    if p.is_empty() || !p.bytes().all(|b| (0x21..=0x7e).contains(&b)) || p.contains('#') || p.contains('\\') {
        return rifiuta("path con caratteri non ammessi");
    }
    let (parte_p, query) = match p.find('?') {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => (p, ""),
    };
    // gli @odata.nextLink hanno a volte "/" prima di "?": si toglie un solo "/" finale, mai segmenti vuoti in mezzo
    let mut segmenti: Vec<String> = Vec::new();
    for s in parte_p.strip_suffix('/').unwrap_or(parte_p).split('/') {
        match CHIAVE_ODATA.captures(s) {
            Some(c) => {
                segmenti.push(c[1].to_string());
                segmenti.push(c[2].to_string());
            }
            None => segmenti.push(s.to_string()),
        }
    }
    let mut ammessa = false;
    for f in FORME.iter().filter(|f| f.len() == segmenti.len()) {
        let mut ok = true;
        for (atteso, s) in f.iter().zip(&segmenti) {
            let giusto = if *atteso == "*" { identificativo_valido(s)? } else { s.to_lowercase() == *atteso };
            if !giusto {
                ok = false;
                break;
            }
        }
        if ok {
            ammessa = true;
            break;
        }
    }
    if !ammessa {
        return rifiuta("path non è una lettura di posta o calendario ammessa");
    }
    let mut visti: Vec<String> = Vec::new();
    for campo in if query.is_empty() { Vec::new() } else { query.split('&').collect() } {
        let Some(uguale) = campo.find('=').filter(|u| *u >= 1) else { return rifiuta("parametro della query senza nome o valore") };
        let nome = decodifica(&campo[..uguale])?.to_lowercase();
        let valore = &campo[uguale + 1..];
        if !PARAMETRI.contains(&nome.as_str()) {
            return rifiuta(format!("parametro non ammesso: {}", primi_utf16(&nome, 40)));
        }
        if visti.contains(&nome) {
            return rifiuta(format!("parametro ripetuto: {nome}"));
        }
        visti.push(nome.clone());
        if !valore.bytes().all(|b| b == 0x21 || b == 0x22 || (0x24..=0x7e).contains(&b)) {
            return rifiuta(format!("valore non ammesso per {nome}"));
        }
        let chiaro = decodifica(&valore.replace('+', " "))?;
        let cifre = |max: usize| !chiaro.is_empty() && chiaro.len() <= max && chiaro.bytes().all(|b| b.is_ascii_digit());
        if nome == "$top" && !(cifre(4) && chiaro.parse::<u32>().is_ok_and(|n| (1..=MAX_TOP).contains(&n))) {
            return rifiuta(format!("$top va da 1 a {MAX_TOP}"));
        }
        if nome == "$skip" && !cifre(7) {
            return rifiuta("$skip non valido");
        }
    }
    let percorso = segmenti.join("/");
    let relativo = if query.is_empty() { percorso.clone() } else { format!("{percorso}?{query}") };
    // il parser degli URL non deve cambiare il percorso (punti, codifiche): si chiede quello che si è controllato
    let base0 = url::Url::parse(OUTLOOK_REST).map_err(|_| errore(400, "path non ammesso"))?;
    let u = url::Url::parse(&format!("{OUTLOOK_REST}/{relativo}")).map_err(|_| errore(400, "path non ammesso"))?;
    if u.origin() != base0.origin() || u.path() != format!("{}/{percorso}", base0.path()) || u.fragment().is_some() {
        return rifiuta("path non ammesso");
    }
    Ok(relativo)
}

/// L'intestazione Prefer ammessa: solo il fuso delle date di Outlook. Nessun'altra intestazione passa.
pub fn prefer_fuso(dato: Option<&Json>) -> Result<Option<String>, ErroreComunicazione> {
    match dato {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Str(s)) if PREFER_FUSO.is_match(s) && !s.contains("..") => Ok(Some(s.clone())),
        _ => rifiuta("prefer ammette solo outlook.timezone=\"Zona\""),
    }
}

/// Il codice d'errore di Outlook ({"error": {"code": "ErrorItemNotFound"}}), solo se ha la forma di un codice.
fn codice_errore_outlook(testo: &str) -> String {
    let Ok(j) = Json::analizza(testo) else { return String::new() };
    match j.get("error").filter(|e| !e.is_null()).and_then(|e| e.get("code")) {
        Some(Json::Str(c)) if CODICE_OUTLOOK.is_match(c) => c.clone(),
        _ => String::new(),
    }
}

fn indirizzo_esatto(a: &str) -> bool {
    a.split_once('@').is_some_and(|(l, d)| !l.is_empty() && !d.is_empty() && !d.contains('@') && !a.chars().any(spazio_js))
}

pub struct GatewayComunicazione {
    tokens: Rc<GestoreToken>,
}

struct Richiesta {
    account: String,
    percorso: String,
    prefer: Option<String>,
}

/// I token della casella chiesta, con i controlli del gateway: un errore di rinnovo non dice niente di Microsoft.
struct ConCasella<'a> {
    tokens: &'a GestoreToken,
    account: &'a str,
}

impl FonteToken for ConCasella<'_> {
    fn ottieni_access_token<'a>(&'a self, email: &'a str, forza: bool) -> FutureToken<'a> {
        Box::pin(async move {
            let token = match self.tokens.ottieni_access_token(email, forza).await {
                Ok(t) => t,
                // i testi di ErroreAccessoScaduto sono dell'agente (Accedi di nuovo...); gli altri errori di rinnovo possono citare Microsoft
                Err(ErroreAccesso::Scaduto(m)) => return Err(Box::new(errore(409, m)) as ErroreDyn),
                Err(_) => return Err(Box::new(errore(502, "rinnovo dell'accesso Microsoft non riuscito")) as ErroreDyn),
            };
            // un token che dice un'altra casella non esce verso Outlook (i token MSA non sono JWT e non dicono la casella)
            let upn = upn_di(&claims_jwt(&token));
            if !upn.is_empty() && upn != self.account {
                return Err(Box::new(errore(409, "il token salvato è di un'altra casella: accedi di nuovo")) as ErroreDyn);
            }
            if self.tokens.rimosso(self.account) || !self.tokens.ha_account(self.account) {
                return Err(Box::new(errore(404, "casella non trovata")) as ErroreDyn);
            }
            Ok(token)
        })
    }
}

impl GatewayComunicazione {
    pub fn nuovo(tokens: Rc<GestoreToken>) -> Self {
        GatewayComunicazione { tokens }
    }

    /// Le caselle Microsoft con un accesso salvato, senza quelle rimosse. Nessun token e nessun rinnovo: si legge solo l'archivio.
    pub fn caselle(&self) -> Vec<Json> {
        let mut out = Vec::new();
        for (upn, voce) in self.tokens.elenco_account() {
            let v = voce.borrow();
            if v.get("provider").and_then(Json::as_str) != Some("microsoft") || self.tokens.rimosso(&upn) {
                continue;
            }
            let mut m = Mappa::new();
            if let Some(sh) = v.get("shared").filter(|s| vero(s)) {
                m.insert("capabilities", sh.get("capabilities").cloned().unwrap_or_default());
            }
            m.insert("upn", upn.as_str());
            let errore = v.get("errore");
            m.insert("status", if errore.is_some_and(vero) { "errore" } else { "ok" });
            m.insert("message", errore.filter(|e| !e.is_null()).map_or_else(|| Json::from(""), |e| Json::from(stringa_di(e))));
            out.push(Json::Obj(m));
        }
        out
    }

    /// Il corpo della richiesta di lettura, controllato: casella esatta, percorso ammesso, Prefer del fuso, nessun altro campo.
    fn richiesta(&self, dati: &Json) -> Result<Richiesta, ErroreComunicazione> {
        let Json::Obj(r) = dati else { return rifiuta("serve {\"account\": ..., \"path\": ...}") };
        let estranei: Vec<&str> = r.keys().map(String::as_str).filter(|k| !["account", "path", "prefer"].contains(k)).collect();
        if !estranei.is_empty() {
            return rifiuta(format!("campi non ammessi: {}", primi_utf16(&estranei.join(", "), 80)));
        }
        let Some(Json::Str(account)) = r.get("account").filter(|a| a.as_str().is_some_and(indirizzo_esatto)) else {
            return rifiuta("account deve essere l'indirizzo esatto della casella");
        };
        let percorso = percorso_lettura(r.get("path"))?;
        let prefer = prefer_fuso(r.get("prefer"))?;
        let account = account.to_lowercase();
        if !self.caselle().iter().any(|c| c["upn"].as_str() == Some(account.as_str())) {
            return Err(errore(404, "casella non trovata"));
        }
        let shared = self.tokens.voce_account(&account).and_then(|v| v.borrow().get("shared").filter(|s| vero(s)).cloned());
        if let Some(sh) = shared {
            let serve = if CALENDARIO.is_match(&percorso) { "calendar" } else { "mail" };
            let ha = sh.get("capabilities").and_then(Json::as_arr).is_some_and(|c| c.iter().any(|x| x.as_str() == Some(serve)));
            if !ha {
                return Err(errore(403, "capability disabled"));
            }
        }
        Ok(Richiesta { account, percorso, prefer })
    }

    /// La lettura: il testo JSON di Outlook così com'è. Il token resta qui; l'agente lo rinnova quando serve. Gli errori dicono solo il
    /// codice HTTP di Outlook e il suo codice d'errore: mai il testo della risposta, che può citare la posta, mai il messaggio di un errore
    /// di rete, che può citare l'indirizzo chiesto.
    pub async fn leggi(&self, dati: &Json) -> Result<String, ErroreComunicazione> {
        let Richiesta { account, percorso, prefer } = self.richiesta(dati)?;
        let con_casella = ConCasella { tokens: &self.tokens, account: &account };
        let mut headers = Intestazioni::new();
        if let Some(p) = &prefer {
            headers.imposta("Prefer", p.clone());
        }
        let o = OpzioniRest { headers, timeout: Duration::from_millis(30_000), max_byte: Some(MAX_RISPOSTA), segui_redirect: false };
        let r = match get_rest(&con_casella, &self.tokens.trasporto, &account, &format!("{OUTLOOK_REST}/{percorso}"), &o).await {
            Ok(r) => r,
            Err(ErroreRest::Token(e)) => {
                return Err(match e.downcast_ref::<ErroreComunicazione>() {
                    Some(c) => c.clone(),
                    None => errore(502, "Outlook non raggiungibile"),
                });
            }
            Err(ErroreRest::Rete(ErroreRete::RispostaGrande(_))) => return Err(errore(502, format!("risposta di Outlook oltre {MAX_RISPOSTA} byte"))),
            Err(ErroreRest::Rete(ErroreRete::Timeout)) => return Err(errore(504, "Outlook non ha risposto in tempo")),
            Err(_) => return Err(errore(502, "Outlook non raggiungibile")),
        };
        if !(200..=299).contains(&r.status) {
            // un 401 dopo il rinnovo, un redirect o un 5xx sono un problema di Outlook, non della richiesta: 502
            let codice = if [400, 403, 404, 429].contains(&r.status) { r.status } else { 502 };
            let attesa = r.headers.ottieni("retry-after");
            let codice_outlook = codice_errore_outlook(&r.testo);
            let mut e = errore(codice, format!("Outlook HTTP {}{}", r.status, if codice_outlook.is_empty() { String::new() } else { format!(" ({codice_outlook})") }));
            if let Some(a) = attesa.filter(|a| codice == 429 && !a.is_empty() && a.len() <= 6 && a.bytes().all(|b| b.is_ascii_digit())) {
                e.intestazioni.push(("Retry-After".to_string(), a));
            }
            return Err(e);
        }
        let testo = if r.testo.is_empty() { "{}".to_string() } else { r.testo };
        match Json::analizza(&testo) {
            Ok(Json::Obj(_)) => Ok(testo),
            Ok(_) => Err(errore(502, "Outlook ha risposto senza un oggetto JSON")),
            Err(_) => Err(errore(502, "Outlook ha risposto senza JSON")),
        }
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    fn p(s: &str) -> Result<String, String> {
        percorso_lettura(Some(&Json::from(s))).map_err(|e| e.messaggio)
    }

    #[test]
    fn percorsi_ammessi_e_rifiutati() {
        assert_eq!(p("me/messages?$top=10&$select=Subject").unwrap(), "me/messages?$top=10&$select=Subject");
        assert_eq!(p("https://outlook.office365.com/api/v2.0/me/MailFolders('AAMk%3D')/messages/?$skip=10").unwrap(), "me/MailFolders/AAMk%3D/messages?$skip=10");
        assert_eq!(p("me/messages/..%2F..").unwrap_err(), "path non è una lettura di posta o calendario ammessa");
        assert_eq!(p("me/sendmail").unwrap_err(), "path non è una lettura di posta o calendario ammessa");
        assert_eq!(p("me/messages?$top=0").unwrap_err(), "$top va da 1 a 1000");
        assert_eq!(p("me/messages?$top=1&$top=2").unwrap_err(), "parametro ripetuto: $top");
        assert_eq!(p("me/messages?x=1").unwrap_err(), "parametro non ammesso: x");
        assert_eq!(p("me/messages/%ZZ").unwrap_err(), "path non è una lettura di posta o calendario ammessa");
        assert_eq!(p("me/messages?$filter=%E0").unwrap_err(), "codifica % non valida");
        assert_eq!(p("").unwrap_err(), "manca path");
        assert_eq!(prefer_fuso(Some(&Json::from("outlook.timezone=\"Europe/Rome\""))).unwrap().as_deref(), Some("outlook.timezone=\"Europe/Rome\""));
        assert!(prefer_fuso(Some(&Json::from("outlook.timezone=\"../x\""))).is_err());
    }
}
