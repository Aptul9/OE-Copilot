//! Le pagine del proxy di sicurezza di Microsoft (Defender for Cloud Apps, *.mcas.ms) e di login: il modulo con l'id_token, la pagina
//! di accesso monitorato, il ripristino del contesto, "Object Moved", la configurazione JSON nella pagina, l'indirizzo del bootstrap. Lo
//! stesso controllo di `teams-relay/app/scripts/mcas-auth-forms.mjs`: ogni forma inattesa è un errore.

use oe_nucleo::json::Json;
use regex::Regex;
use std::sync::LazyLock;
use url::Url;

pub type Esito<T> = Result<T, String>;

fn verifica(cond: bool, messaggio: &str) -> Esito<()> {
    if cond { Ok(()) } else { Err(messaggio.to_string()) }
}

static ENTITA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)&(?:#x([0-9a-f]+)|#([0-9]+)|(amp|quot|apos|lt|gt));").expect("valida"));
static ATTRIBUTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?-u:\b)([A-Za-z0-9_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).expect("valida"));
static INPUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<input(?-u:\b)[^>]*>").expect("valida"));
static FORM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<form(?-u:\b)[^>]*>(.*?)</form>").expect("valida"));
static CONTESTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)var context\s*=\s*(\{.*?\});").expect("valida"));
static OBJECT_MOVED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<title>\s*Object Moved\s*</title>").expect("valida"));
static SOSTITUISCI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"window\.location\.replace\(\s*("(?:\\.|[^"\\])*")\s*\)"#).expect("valida"));
static SCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<!--.*?-->|(<script(?-u:\b)(?:[^"'<>]|"[^"]*"|'[^']*')*>)(?:.*?</script\s*>|$)"#).expect("valida"));
static ATTRIBUTO_SCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\s+([A-Za-z0-9_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'`=<>]+))"#).expect("valida"));

/// Le entità che il modulo decodifica: &amp; &quot; &apos; &lt; &gt; e i riferimenti numerici.
fn decodifica(s: &str) -> Esito<String> {
    let mut errore = None;
    let out = ENTITA.replace_all(s, |c: &regex::Captures<'_>| {
        if let Some(nome) = c.get(3) {
            return match nome.as_str().to_ascii_lowercase().as_str() {
                "amp" => "&",
                "quot" => "\"",
                "apos" => "'",
                "lt" => "<",
                _ => ">",
            }
            .to_string();
        }
        let (testo, base) = match c.get(1) {
            Some(h) => (h.as_str(), 16),
            None => (c.get(2).map_or("", |d| d.as_str()), 10),
        };
        match u32::from_str_radix(testo, base).ok().and_then(char::from_u32) {
            Some(ch) => ch.to_string(),
            None => {
                errore.get_or_insert_with(|| format!("Invalid code point {testo}"));
                String::new()
            }
        }
    });
    match errore {
        Some(e) => Err(e),
        None => Ok(out.into_owned()),
    }
}

/// Gli attributi di un tag, per nome in minuscolo (di due uguali vale l'ultimo).
fn attributi(tag: &str) -> Esito<indexmap::IndexMap<String, String>> {
    let mut out = indexmap::IndexMap::new();
    for m in ATTRIBUTO.captures_iter(tag) {
        let valore = m.get(2).or_else(|| m.get(3)).map_or("", |v| v.as_str());
        out.insert(m[1].to_lowercase(), decodifica(valore)?);
    }
    Ok(out)
}

fn input_nascosti(html: &str) -> Esito<Vec<indexmap::IndexMap<String, String>>> {
    let mut out = Vec::new();
    for m in INPUT.find_iter(html) {
        let a = attributi(m.as_str())?;
        if a.get("type").is_some_and(|t| t.to_lowercase() == "hidden") {
            out.push(a);
        }
    }
    Ok(out)
}

fn url(valore: &str) -> Esito<Url> {
    Url::parse(valore).map_err(|_| "Invalid URL".to_string())
}

fn gateway_action(valore: &str) -> Esito<String> {
    let a = url(valore)?;
    let host = a.host_str().unwrap_or_default();
    let gateway = host.strip_suffix(".access.mcas.ms").is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    verifica(a.scheme() == "https" && a.username().is_empty() && a.password().is_none() && a.port().is_none() && gateway && a.path() == "/aad_login", "unexpected MCAS form destination")?;
    Ok(a.to_string())
}

pub struct Modulo {
    pub action: String,
    pub id_token: String,
}

pub fn mcas_form(html: &str) -> Esito<Modulo> {
    let forme: Vec<regex::Captures<'_>> = FORM.captures_iter(html).collect();
    verifica(forme.len() == 1, "one MCAS form required")?;
    let tutto = forme[0].get(0).map_or("", |m| m.as_str());
    let form = attributi(&tutto[..tutto.find('>').map_or(0, |i| i + 1)])?;
    verifica(form.get("method").is_some_and(|m| m.to_lowercase() == "post"), "MCAS form must use POST")?;
    let action = gateway_action(form.get("action").map_or("", String::as_str))?;
    let token: Vec<_> = input_nascosti(&forme[0][1])?.into_iter().filter(|i| i.get("name").map(String::as_str) == Some("id_token")).collect();
    let valore = token.first().and_then(|t| t.get("value")).cloned().unwrap_or_default();
    verifica(token.len() == 1 && !valore.is_empty() && valore.encode_utf16().count() < 30000, "one MCAS id_token required")?;
    Ok(Modulo { action, id_token: valore })
}

pub struct Passo {
    pub action: String,
    pub campi: Vec<(String, String)>,
}

pub fn mcas_continuation(html: &str) -> Esito<Passo> {
    let invalido = || "invalid MCAS continuation context".to_string();
    let testo = CONTESTO.captures(html).and_then(|c| c.get(1)).map_or("", |m| m.as_str());
    let c = Json::analizza(testo).map_err(|_| invalido())?;
    let falso = |k: &str| !c.get(k).is_some_and(vero);
    verifica(
        c["TITLE_ID"].as_str() == Some("PROXYWEB_MONITORED_ACCESS_MESSAGE")
            && c["DESCRIPTION_ID"].as_str() == Some("PROXYWEB_MONITORED_ACCESS_DESCRIPTION")
            && c["DISPLAY_DISCLAIMER"] == Json::Bool(true)
            && falso("CUSTOMIZED_TITLE")
            && falso("CUSTOMIZED_BODY")
            && falso("WAIT_FOR_APP_ONBOARDING")
            && falso("APP_ONBOARDING_SHOW_CONTINUE")
            && falso("BYPASS_NON_SUFFIX_SUPPORTED_APP_FOR_ADMIN")
            && falso("SECOND_LOGIN_INFORMATION_PAGE_ENABLED"),
        "unsupported MCAS interstitial",
    )?;
    verifica(c["REDIRECT_TO_SUFFIX_METHOD"].as_str() == Some("POST"), "MCAS continuation must use POST")?;
    let nascosti = match c.get("ADDITIONAL_HIDDEN_FIELDS").filter(|v| !v.is_null()) {
        Some(Json::Str(s)) => input_nascosti(s)?,
        Some(_) => return Err("html.matchAll is not a function".to_string()),
        None => Vec::new(),
    };
    let nome = |i: &indexmap::IndexMap<String, String>| i.get("name").cloned().unwrap_or_default();
    verifica(!nascosti.is_empty() && nascosti.iter().all(|i| ["id_token", "appLoginCookies", "contextData"].contains(&nome(i).as_str())), "unsupported MCAS continuation field")?;
    let mut campi: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    for i in &nascosti {
        campi.insert(nome(i), i.get("value").cloned().unwrap_or_default());
    }
    verifica(campi.get("id_token").is_some_and(|t| !t.is_empty()) && campi.len() == nascosti.len(), "one MCAS id_token required")?;
    let action = gateway_action(c["REDIRECT_TO_SUFFIX_URL"].as_str().unwrap_or_default())?;
    let mut campi: Vec<(String, String)> = campi.into_iter().collect();
    campi.extend([("continueToSessionControl".to_string(), "true".to_string()), ("isTopFrame".to_string(), "true".to_string()), ("jsNativeBrokerMainWindowDomain".to_string(), String::new())]);
    Ok(Passo { action, campi })
}

fn vero(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Num(n) => *n != 0.0 && !n.is_nan(),
        Json::Str(s) => !s.is_empty(),
        _ => true,
    }
}

fn parametri(s: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(s.as_bytes()).into_owned().collect()
}

pub fn mcas_restore(valore: &str, gateway: &str) -> Esito<Passo> {
    let u = url(valore)?;
    verifica(
        u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.port().is_none() && u.host_str() == Some("inline.cdn.mcas.ms") && u.path() == "/proxyweb/1.67.98/html/session-context-restore.html",
        "unexpected MCAS context-restorer",
    )?;
    let p = parametri(u.fragment().unwrap_or_default());
    let tutti = |k: &str| p.iter().filter(|(n, _)| n == k).map(|(_, v)| v.clone()).collect::<Vec<_>>();
    let (azioni, ritorni) = (tutti("action"), tutti("redirect_uri"));
    verifica(azioni.first().map(String::as_str) == Some("post") && azioni.len() == 1 && ritorni.len() == 1, "unsupported MCAS context-restorer action")?;
    let t = url(&ritorni[0])?;
    verifica(t.scheme() == "https" && t.username().is_empty() && t.password().is_none() && t.port().is_none() && t.host_str() == Some(gateway) && t.path() == "/finalize_login", "unexpected MCAS context-restorer target")?;
    Ok(Passo { action: t.to_string(), campi: vec![("contextData".to_string(), String::new())] })
}

pub fn mcas_moved(html: &str, app_origin: &str) -> Esito<String> {
    verifica(OBJECT_MOVED.is_match(html), "unexpected MCAS moved response")?;
    let trovati: Vec<regex::Captures<'_>> = SOSTITUISCI.captures_iter(html).collect();
    verifica(trovati.len() == 1, "one literal MCAS redirect required")?;
    let destinazione = Json::analizza(&trovati[0][1]).map_err(|e| e.messaggio)?;
    let u = url(destinazione.as_str().unwrap_or_default())?;
    let outlook = app_origin == "https://outlook.cloud.microsoft";
    let host_validi: &[&str] = if outlook { &["outlook.cloud.microsoft.mcas.ms"] } else { &["teams.cloud.microsoft.mcas.ms", "teams.microsoft.com.mcas.ms"] };
    verifica(
        u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.port().is_none() && host_validi.contains(&u.host_str().unwrap_or_default()) && (u.path() == "/" || (outlook && u.path() == "/mail/")),
        "unexpected MCAS moved destination",
    )?;
    Ok(u.to_string())
}

fn spazio_js(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// Dove inizia il valore dopo ogni `$Config =` (non `==` né `=>`) o `__JSWRAPPER.initWrapperFixer(` che non segue un identificatore.
fn marcatori(sorgente: &str, marcatore: &str) -> Vec<usize> {
    let (parola, segno) = if marcatore == "$Config" { ("$Config", '=') } else { ("__JSWRAPPER.initWrapperFixer", '(') };
    let mut out = Vec::new();
    for (i, _) in sorgente.match_indices(parola) {
        let prima = sorgente[..i].chars().next_back();
        if prima.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.') {
            continue;
        }
        let dopo = &sorgente[i + parola.len()..];
        let resto = dopo.trim_start_matches(spazio_js);
        if !resto.starts_with(segno) {
            continue;
        }
        let oltre = &resto[1..];
        if segno == '=' && (oltre.starts_with('=') || oltre.starts_with('>')) {
            continue;
        }
        out.push(sorgente.len() - oltre.len());
    }
    out
}

/// L'oggetto JSON di configurazione di una pagina (`$Config = {...}` di login, `__JSWRAPPER.initWrapperFixer({...})` del bootstrap).
pub fn configuration(sorgente: &str, marcatore: &str) -> Esito<Json> {
    verifica(sorgente.len() <= 2 * 1024 * 1024, "configuration source exceeds two MiB")?;
    verifica(marcatore == "$Config" || marcatore == "__JSWRAPPER.initWrapperFixer", "unsupported configuration marker")?;
    let trovati = marcatori(sorgente, marcatore);
    verifica(trovati.len() == 1, "one configuration marker required")?;
    let resto = &sorgente[trovati[0]..];
    let inizio = trovati[0] + (resto.len() - resto.trim_start_matches(spazio_js).len());
    verifica(sorgente[inizio..].starts_with('{'), "configuration must be a JSON object")?;
    let (mut livello, mut virgolette, mut escape) = (0usize, false, false);
    for (i, c) in sorgente[inizio..].char_indices() {
        if virgolette {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                virgolette = false;
            }
        } else if c == '"' {
            virgolette = true;
        } else if c == '{' {
            livello += 1;
        } else if c == '}' {
            livello -= 1;
            if livello == 0 {
                let fine = inizio + i + 1;
                let coda = sorgente[fine..].trim_start_matches(spazio_js);
                let script_chiuso = coda.len() >= 8 && coda[..8].eq_ignore_ascii_case("</script") && coda[8..].trim_start_matches(spazio_js).starts_with('>');
                let ok = if marcatore == "$Config" { coda.is_empty() || coda.starts_with(';') || script_chiuso } else { coda.starts_with(')') };
                verifica(ok, "configuration must be one JSON object")?;
                return Json::analizza(&sorgente[inizio..fine]).map_err(|e| e.messaggio);
            }
        }
    }
    Err("invalid configuration JSON object".to_string())
}

/// L'indirizzo dello script di bootstrap del proxy nella pagina, se c'è (uno solo).
pub fn bootstrap_url(html: &str, origine: &str) -> Esito<Option<String>> {
    let base = url(origine)?;
    let mut trovati: Vec<String> = Vec::new();
    for m in SCRIPT.captures_iter(html) {
        let Some(tag) = m.get(1) else { continue };
        let src = ATTRIBUTO_SCRIPT.captures_iter(tag.as_str()).find(|a| a[1].to_lowercase() == "src");
        let src = match src {
            Some(a) => decodifica(a.get(2).or_else(|| a.get(3)).or_else(|| a.get(4)).map_or("", |v| v.as_str()))?,
            None => String::new(),
        };
        if src.is_empty() {
            continue;
        }
        let u = base.join(&src).map_err(|_| "Invalid URL".to_string())?;
        if !u.path().ends_with("/js-bootstrap.js") && u.host_str() != Some("mcas-proxyweb.mcas.ms") {
            continue;
        }
        verifica(
            u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.port().is_none() && u.host_str() == Some("mcas-proxyweb.mcas.ms") && u.path() == "/js-bootstrap.js",
            "unexpected MCAS bootstrap destination",
        )?;
        if !trovati.contains(&u.to_string()) {
            trovati.push(u.to_string());
        }
    }
    verifica(trovati.len() <= 1, "one MCAS bootstrap URL required")?;
    Ok(trovati.pop())
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn configurazione_e_moduli() {
        let c = configuration("x = 1; $Config={\"a\":\"}\",\"b\":{\"c\":1}};</script>", "$Config").unwrap();
        assert_eq!(c.in_js(), r#"{"a":"}","b":{"c":1}}"#);
        assert!(configuration("a.$Config = {}; $Config == 1", "$Config").is_err());
        assert_eq!(configuration("__JSWRAPPER.initWrapperFixer( {\"x\":true})", "__JSWRAPPER.initWrapperFixer").unwrap().in_js(), r#"{"x":true}"#);
        let f = mcas_form(r#"<form method="POST" action="https://a-1.access.mcas.ms/aad_login"><input type="hidden" name="id_token" value="t&amp;k"></form>"#).unwrap();
        assert_eq!((f.action.as_str(), f.id_token.as_str()), ("https://a-1.access.mcas.ms/aad_login", "t&k"));
        assert_eq!(mcas_form("<form method=post action='https://x.com/aad_login'></form>").err().unwrap(), "MCAS form must use POST");
        assert_eq!(mcas_form("<form method='post' action='https://x.com/aad_login'></form>").err().unwrap(), "unexpected MCAS form destination");
        assert_eq!(bootstrap_url(r#"<script src="https://mcas-proxyweb.mcas.ms/js-bootstrap.js?x=1"></script>"#, "https://outlook.cloud.microsoft.mcas.ms").unwrap().unwrap(), "https://mcas-proxyweb.mcas.ms/js-bootstrap.js?x=1");
        let moved = r#"<title>Object Moved</title><script>window.location.replace("https://outlook.cloud.microsoft.mcas.ms/mail/")</script>"#;
        assert_eq!(mcas_moved(moved, "https://outlook.cloud.microsoft").unwrap(), "https://outlook.cloud.microsoft.mcas.ms/mail/");
    }
}
