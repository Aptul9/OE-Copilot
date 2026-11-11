//! L'accesso a Outlook web attraverso il proxy di sicurezza di Microsoft (Defender for Cloud Apps): la sessione del proxy importata dal
//! browser, i suoi moduli e passaggi seguiti senza browser fino al codice, poi lo scambio del codice. Ogni passo è controllato: una pagina o
//! una destinazione inattesa ferma l'accesso.

use crate::errori::{ErroreAccesso, EsitoAccesso};
use crate::mcas_auth_forms::{bootstrap_url, configuration, mcas_continuation, mcas_form, mcas_moved, mcas_restore};
use crate::outlook_rest::leggi_testo;
use crate::sessione_web::{CookieSessione, campo, casuale_base64url, data_cookie, modulo, nome_cookie_valido, nuovo_cookie, scadenza, sha256_base64url, valore_cookie_valido};
use crate::trasporto::{Richiesta, Risposta, Trasporto};
use oe_nucleo::json::{Json, Mappa};
use regex::Regex;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub struct ProxySession {
    pub bootstrap_url: String,
    pub cookies: Vec<CookieSessione>,
}

impl ProxySession {
    pub fn in_json(&self) -> Json {
        let mut m = Mappa::new();
        m.insert("bootstrapUrl", self.bootstrap_url.as_str());
        m.insert("cookies", Json::Arr(self.cookies.iter().cloned().map(Json::Obj).collect()));
        Json::Obj(m)
    }
}

const LOGIN: &str = "https://login.microsoftonline.com";
const PROXY: &str = "https://login.microsoftonline.com.mcas.ms";
const APP: &str = "https://outlook.cloud.microsoft";
const REDIRECT: &str = "https://outlook.cloud.microsoft/mail/";
const CLIENT: &str = "9199bf20-a13f-4107-85dc-02114787ef48";
const SCOPE: &str = "https://outlook.office.com/.default offline_access openid profile";

pub fn proxy_cookie(c: &CookieSessione) -> bool {
    let stringhe = ["name", "value", "domain", "path"].iter().all(|k| matches!(c.get(k), Some(Json::Str(_))));
    if !stringhe || !scadenza(c).is_finite() || !nome_cookie_valido(campo(c, "name")) || !valore_cookie_valido(campo(c, "value")) || !campo(c, "path").starts_with('/') {
        return false;
    }
    let d = campo(c, "domain");
    let dominio = d.strip_prefix('.').unwrap_or(d).to_lowercase();
    let nome = campo(c, "name");
    dominio == "login.microsoftonline.com"
        || dominio == "login.microsoftonline.com.mcas.ms"
        || (dominio == "mcas.ms" && nome.strip_prefix("AdallomAuth").is_some_and(|r| r.bytes().all(|b| b.is_ascii_digit())))
        || (dominio.strip_suffix(".access.mcas.ms").is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')) && nome == "CAACSkipDisclaimer")
}

fn per_host(c: &CookieSessione, host: &str) -> bool {
    let d = campo(c, "domain");
    let dominio = d.strip_prefix('.').unwrap_or(d).to_lowercase();
    host == dominio || (d.starts_with('.') && host.ends_with(&format!(".{dominio}")))
}

fn adesso_s() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Un errore che non è del proxy diventa uno solo, senza dettagli: il testo di una pagina non esce.
pub async fn authorize_proxy(session: &mut ProxySession, account: &str, user_agent: &str, trasporto: &Rc<dyn Trasporto>) -> EsitoAccesso<Mappa> {
    match proxy_flow(session, account, user_agent, trasporto).await {
        Ok(m) => Ok(m),
        Err(e @ ErroreAccesso::ProxyBrowser(_)) => Err(e),
        Err(ErroreAccesso::Altro(m)) if m.starts_with("Microsoft security proxy") || m.starts_with("Unsupported Microsoft") => Err(ErroreAccesso::Altro(m)),
        Err(_) => Err(ErroreAccesso::altro("Microsoft security proxy returned an unsupported response")),
    }
}

fn altro(m: impl Into<String>) -> ErroreAccesso {
    ErroreAccesso::Altro(m.into())
}

fn url(v: &str) -> EsitoAccesso<Url> {
    Url::parse(v).map_err(|_| altro("Invalid URL"))
}

fn origine(u: &Url) -> String {
    u.origin().ascii_serialization()
}

fn bootstrap_valido(u: &Url) -> bool {
    u.scheme() == "https" && u.host_str() == Some("mcas-proxyweb.mcas.ms") && u.port().is_none() && u.path() == "/js-bootstrap.js" && u.username().is_empty() && u.password().is_none()
}

fn forme(e: String) -> ErroreAccesso {
    altro(e)
}

/// `searchParams.set`: il primo prende il valore, gli altri con lo stesso nome spariscono, tutto si riscrive come un modulo.
fn imposta_parametro(u: &mut Url, nome: &str, valore: &str) {
    let mut coppie: Vec<(String, String)> = u.query_pairs().into_owned().collect();
    match coppie.iter().position(|(k, _)| k == nome) {
        Some(i) => {
            coppie[i].1 = valore.to_string();
            let mut visto = false;
            coppie.retain(|(k, _)| {
                if k != nome {
                    return true;
                }
                let tieni = !visto;
                visto = true;
                tieni
            });
        }
        None => coppie.push((nome.to_string(), valore.to_string())),
    }
    u.query_pairs_mut().clear().extend_pairs(coppie);
}

struct Flusso<'a> {
    session: &'a mut ProxySession,
    user_agent: &'a str,
    trasporto: &'a Rc<dyn Trasporto>,
    gateway: String,
    scadenza: Instant,
}

impl Flusso<'_> {
    fn permesso(&self, valore: &str) -> EsitoAccesso<Url> {
        let u = url(valore)?;
        let host = u.host_str().unwrap_or_default();
        let ammessi = ["mcas-proxyweb.mcas.ms", "login.microsoftonline.com", "login.microsoftonline.com.mcas.ms", "outlook.cloud.microsoft", "outlook.cloud.microsoft.mcas.ms", "inline.cdn.mcas.ms", self.gateway.as_str()];
        if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.port().is_some() || !ammessi.contains(&host) {
            return Err(altro("Unsupported Microsoft security proxy destination"));
        }
        Ok(u)
    }

    async fn invia(&mut self, valore: &str, campi: Option<&[(String, String)]>, intestazioni: &[(&str, String)]) -> EsitoAccesso<Risposta> {
        let u = self.permesso(valore)?;
        let host = u.host_str().unwrap_or_default().to_string();
        let ora = adesso_s();
        let percorso = u.path().to_string();
        let mut adatti: Vec<&CookieSessione> = self
            .session
            .cookies
            .iter()
            .filter(|c| {
                let e = scadenza(c);
                let p = campo(c, "path");
                per_host(c, &host) && (e == -1.0 || e > ora as f64) && (p == "/" || percorso == p || percorso.starts_with(&if p.ends_with('/') { p.to_string() } else { format!("{p}/") }))
            })
            .collect();
        adatti.sort_by_key(|c| std::cmp::Reverse(campo(c, "path").encode_utf16().count()));
        let cookie = adatti.iter().map(|c| format!("{}={}", campo(c, "name"), campo(c, "value"))).collect::<Vec<_>>().join("; ");
        let resta = self.scadenza.saturating_duration_since(Instant::now());
        if resta.is_zero() {
            return Err(altro("Microsoft security proxy timed out"));
        }
        let mut r = match campi {
            Some(c) => Richiesta::post(u.as_str(), form_urlencoded::Serializer::new(String::new()).extend_pairs(c).finish()),
            None => Richiesta::get(u.as_str()),
        };
        r = r.intestazione("User-Agent", self.user_agent.to_string());
        if !cookie.is_empty() {
            r = r.intestazione("Cookie", cookie);
        }
        if campi.is_some() {
            r = r.intestazione("Content-Type", "application/x-www-form-urlencoded");
        }
        for (k, v) in intestazioni {
            r = r.intestazione(k, v.clone());
        }
        let risposta = self.trasporto.invia(r.senza_redirect().scadenza(resta.min(Duration::from_millis(15_000)))).await?;
        for raw in risposta.intestazioni.tutti("set-cookie") {
            let mut pezzi = raw.split(';');
            let coppia = pezzi.next().unwrap_or_default();
            let Some(uguale) = coppia.find('=').filter(|i| *i >= 1) else { continue };
            let (mut domain, mut path, mut expires) = (host.clone(), "/".to_string(), -1.0);
            let mut max_age: Option<i64> = None;
            for attr in pezzi {
                let mut kv = attr.trim().split('=');
                let nome = kv.next().unwrap_or_default().to_lowercase();
                let v = kv.collect::<Vec<_>>().join("=");
                match nome.as_str() {
                    "domain" => domain = format!(".{}", v.strip_prefix('.').unwrap_or(&v)),
                    "path" => path = v,
                    "expires" => {
                        if let Some(ms) = data_cookie(&v) {
                            expires = ms / 1000.0;
                        }
                    }
                    "max-age" if !v.is_empty() && v != "-" && v.strip_prefix('-').unwrap_or(&v).bytes().all(|b| b.is_ascii_digit()) => max_age = v.parse().ok(),
                    _ => {}
                }
            }
            if let Some(m) = max_age {
                expires = (ora + m) as f64;
            }
            let c = nuovo_cookie(coppia[..uguale].trim(), coppia[uguale + 1..].trim(), &domain, &path, expires);
            let mut con_punto = c.clone();
            con_punto.insert("domain", format!(".{}", domain.strip_prefix('.').unwrap_or(&domain)));
            if !proxy_cookie(&c) || !per_host(&con_punto, &host) {
                continue;
            }
            let nudo = |d: &str| d.strip_prefix('.').unwrap_or(d).to_string();
            self.session.cookies.retain(|v| campo(v, "name") != campo(&c, "name") || nudo(campo(v, "domain")) != nudo(&domain) || campo(v, "path") != path);
            if expires == -1.0 || expires > ora as f64 {
                self.session.cookies.push(c);
            }
        }
        if risposta.stato == 429 || risposta.stato >= 500 {
            return Err(altro(format!("Microsoft security proxy HTTP {}", risposta.stato)));
        }
        Ok(risposta)
    }

    fn testo(r: &Risposta) -> EsitoAccesso<String> {
        if !r.ok() {
            return Err(ErroreAccesso::ProxyBrowser("Microsoft security proxy session requires browser sign-in".to_string()));
        }
        Ok(leggi_testo(r, Some(2 * 1024 * 1024))?)
    }

    async fn leggi_impostazioni(&mut self, indirizzo: &str) -> EsitoAccesso<Json> {
        let b = url(indirizzo)?;
        if !bootstrap_valido(&b) {
            return Err(altro("Unsupported Microsoft security proxy bootstrap"));
        }
        let r = self.invia(b.as_str(), None, &[]).await?;
        let s = configuration(&Self::testo(&r)?, "__JSWRAPPER.initWrapperFixer").map_err(forme)?;
        let mut mancanti = Vec::new();
        if s.get("urlIntegrityEnforcementEnabled") != Some(&Json::Bool(false)) {
            mancanti.push("integrity");
        }
        if s.get("suffixDomainName").and_then(Json::as_str) != Some("mcas.ms") {
            mancanti.push("suffix");
        }
        if s.get("niptuckSubDomainName").and_then(Json::as_str) != Some("mcas-proxyweb") {
            mancanti.push("subdomain");
        }
        if !matches!(s.get("authUserData"), Some(Json::Str(_))) {
            mancanti.push("auth context");
        }
        if !matches!(s.get("topFrameSaasId"), Some(Json::Str(_) | Json::Num(_))) {
            mancanti.push("app id");
        }
        if !mancanti.is_empty() {
            return Err(ErroreAccesso::ProxyBrowser(format!("Microsoft security proxy context unavailable: {}", mancanti.join(", "))));
        }
        Ok(s)
    }

    async fn rinnova_bootstrap(&mut self) -> EsitoAccesso<()> {
        let radice = format!("{APP}.mcas.ms/mail/");
        let mut r = self.invia(&radice, None, &[]).await?;
        let mut i = 0;
        while i < 2 && (300..400).contains(&r.stato) {
            let base = url(&radice)?;
            let dopo = base.join(&r.intestazioni.ottieni("location").unwrap_or_default()).map_err(|_| altro("Invalid URL"))?;
            let dopo = self.permesso(dopo.as_str())?;
            if origine(&dopo) != format!("{APP}.mcas.ms") {
                return Err(altro("Microsoft security proxy app context changed"));
            }
            r = self.invia(dopo.as_str(), None, &[]).await?;
            i += 1;
        }
        if let Some(b) = bootstrap_url(&Self::testo(&r)?, &format!("{APP}.mcas.ms")).map_err(forme)? {
            self.session.bootstrap_url = b;
        }
        Ok(())
    }
}

fn testo_js(v: &Json) -> String {
    match v {
        Json::Str(s) => s.clone(),
        Json::Num(n) => oe_nucleo::json::numero_js(*n),
        Json::Bool(b) => b.to_string(),
        Json::Null => "null".to_string(),
        altro => altro.in_js(),
    }
}

fn vero(v: Option<&Json>) -> bool {
    match v {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(Json::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(Json::Str(s)) => !s.is_empty(),
        _ => true,
    }
}

static TITOLO_SPOSTATO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<title>\s*Object Moved\s*</title>").expect("valida"));
static FORM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<form(?-u:\b)").expect("valida"));
static RIPROCESSA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/(common|[a-f0-9-]{36})/reprocess$").expect("valida"));

async fn proxy_flow(session: &mut ProxySession, account: &str, user_agent: &str, trasporto: &Rc<dyn Trasporto>) -> EsitoAccesso<Mappa> {
    let bootstrap = url(&session.bootstrap_url)?;
    if !bootstrap_valido(&bootstrap) {
        return Err(altro("Unsupported Microsoft security proxy bootstrap"));
    }
    session.cookies.retain(proxy_cookie);
    let mut f = Flusso { session, user_agent, trasporto, gateway: String::new(), scadenza: Instant::now() + Duration::from_secs(90) };
    let impostazioni = f.leggi_impostazioni(bootstrap.as_str()).await?;
    let proxied = |valore: &str, xhr: bool| -> EsitoAccesso<String> {
        let mut u = url(valore)?;
        if vero(impostazioni.get("extendedTopFrameSaasIdParamEnabled")) || (!xhr && vero(impostazioni.get("topFrameSaasIdParamEnabled"))) {
            imposta_parametro(&mut u, "McasTsid", &testo_js(&impostazioni["topFrameSaasId"]));
        }
        if xhr || vero(impostazioni.get("resolveNavigationRequestByUrlEnabled")) {
            imposta_parametro(&mut u, "McasCtx", if xhr { "2" } else { "4" });
        }
        if xhr {
            imposta_parametro(&mut u, "McasUserAuth", &testo_js(&impostazioni["authUserData"]));
        }
        Ok(u.to_string())
    };
    let verifier = casuale_base64url(32);
    let state = casuale_base64url(24);
    let mut auth = url(&format!("{PROXY}/common/oauth2/v2.0/authorize"))?;
    let sfida = sha256_base64url(&verifier);
    auth.set_query(Some(&modulo(&[
        ("client_id", CLIENT),
        ("redirect_uri", REDIRECT),
        ("response_type", "code"),
        ("response_mode", "query"),
        ("scope", SCOPE),
        ("login_hint", account),
        ("state", &state),
        ("code_challenge", &sfida),
        ("code_challenge_method", "S256"),
    ])));
    let mut corrente = auth.to_string();
    let mut risposta = f.invia(&proxied(&corrente, false)?, None, &[]).await?;
    let mut code = String::new();
    let mut ripreso = false;
    for _ in 0..14 {
        let da = f.permesso(&corrente)?;
        if let Some(location) = risposta.intestazioni.ottieni("location").filter(|l| !l.is_empty()) {
            let dopo = f.permesso(da.join(&location).map_err(|_| altro("Invalid URL"))?.as_str())?;
            let p: Vec<(String, String)> = if dopo.query_pairs().any(|(k, _)| k == "state") {
                dopo.query_pairs().into_owned().collect()
            } else {
                form_urlencoded::parse(dopo.fragment().unwrap_or_default().as_bytes()).into_owned().collect()
            };
            let prendi = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            if prendi("code").is_some() || prendi("error").is_some() {
                let o = origine(&dopo);
                if o.strip_suffix(".mcas.ms").unwrap_or(&o) != APP || dopo.path() != "/mail/" || prendi("state").as_deref() != Some(state.as_str()) {
                    return Err(altro("Microsoft security proxy callback mismatch"));
                }
                if prendi("error").is_some() {
                    return Err(altro("Microsoft security proxy session requires sign-in"));
                }
                code = prendi("code").unwrap_or_default();
                break;
            }
            if dopo.host_str() == Some("inline.cdn.mcas.ms") {
                let passo = mcas_restore(dopo.as_str(), &f.gateway).map_err(forme)?;
                corrente = passo.action;
                risposta = f.invia(&corrente, Some(&passo.campi), &[("Origin", origine(&dopo))]).await?;
            } else {
                corrente = dopo.to_string();
                risposta = f.invia(&corrente, None, &[]).await?;
            }
            continue;
        }
        let html = Flusso::testo(&risposta)?;
        if da.host_str() == Some(f.gateway.as_str()) && da.path() == "/finalize_login" && TITOLO_SPOSTATO.is_match(&html) {
            corrente = mcas_moved(&html, APP).map_err(forme)?;
            risposta = f.invia(&corrente, None, &[]).await?;
            continue;
        }
        if html.contains("PROXYWEB_MONITORED_ACCESS_MESSAGE") {
            let passo = mcas_continuation(&html).map_err(forme)?;
            if url(&passo.action)?.host_str() != Some(f.gateway.as_str()) {
                return Err(altro("Microsoft security proxy gateway changed"));
            }
            corrente = passo.action;
            let o = origine(&da);
            risposta = f.invia(&corrente, Some(&passo.campi), &[("Origin", o.clone()), ("Referer", format!("{o}/"))]).await?;
            continue;
        }
        if FORM.is_match(&html) {
            if origine(&da) != LOGIN {
                return Err(altro("Unsupported Microsoft security proxy form issuer"));
            }
            let passo = mcas_form(&html).map_err(forme)?;
            let host = url(&passo.action)?.host_str().unwrap_or_default().to_string();
            if !f.gateway.is_empty() && f.gateway != host {
                return Err(altro("Microsoft security proxy gateway changed"));
            }
            f.gateway = host;
            corrente = passo.action;
            let o = origine(&da);
            risposta = f.invia(&corrente, Some(&[("id_token".to_string(), passo.id_token)]), &[("Origin", o.clone()), ("Referer", format!("{o}/"))]).await?;
            continue;
        }
        let c = configuration(&html, "$Config").map_err(forme)?;
        let sessioni: Vec<&Json> = c
            .get("arrSessions")
            .and_then(Json::as_arr)
            .map(|v| {
                v.iter()
                    .filter(|s| vero(s.get("isSignedIn")) && vero(s.get("matchesLoginHint")) && s.get("name").and_then(Json::as_str).is_some_and(|n| n.to_lowercase() == account) && vero(s.get("id")))
                    .collect()
            })
            .unwrap_or_default();
        if !ripreso && testo_js(c.get("sErrorCode").unwrap_or(&Json::Null)) == "50058" && c.get("fSsoFeasible") == Some(&Json::Bool(true)) && sessioni.len() == 1 {
            if let Some(Json::Str(refresh)) = c.get("urlRefresh") {
                let mut dopo = f.permesso(da.join(refresh).map_err(|_| altro("Invalid URL"))?.as_str())?;
                if origine(&dopo) != LOGIN || !RIPROCESSA.is_match(dopo.path()) {
                    return Err(altro("Unsupported Microsoft session selection"));
                }
                imposta_parametro(&mut dopo, "sessionid", &testo_js(&sessioni[0]["id"]));
                ripreso = true;
                corrente = dopo.to_string();
                risposta = f.invia(&corrente, None, &[]).await?;
                continue;
            }
        }
        return Err(altro("Microsoft security proxy requires interactive sign-in"));
    }
    if code.is_empty() {
        return Err(altro("Microsoft security proxy continuation limit reached"));
    }
    let mut destinazione = proxied(&format!("{PROXY}/common/oauth2/v2.0/token"), true)?;
    let mut o = format!("{APP}.mcas.ms");
    let campi: Vec<(String, String)> = [("client_id", CLIENT), ("redirect_uri", REDIRECT), ("grant_type", "authorization_code"), ("code", &code), ("code_verifier", &verifier), ("scope", SCOPE)]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for _ in 0..3 {
        risposta = f.invia(&destinazione, Some(&campi), &[("Origin", o.clone())]).await?;
        if risposta.stato != 307 {
            break;
        }
        let base = url(&destinazione)?;
        let dopo = f.permesso(base.join(&risposta.intestazioni.ottieni("location").unwrap_or_default()).map_err(|_| altro("Invalid URL"))?.as_str())?;
        if origine(&dopo) != LOGIN || dopo.path() != "/common/oauth2/v2.0/token" {
            return Err(altro("Unsupported Microsoft token redirect"));
        }
        destinazione = dopo.to_string();
        o = "null".to_string();
    }
    let risultato = match Json::analizza(&Flusso::testo(&risposta)?) {
        Ok(Json::Obj(m)) => m,
        Ok(_) => Mappa::new(),
        Err(e) => return Err(altro(e.messaggio)),
    };
    if !matches!(risultato.get("access_token"), Some(Json::Str(_))) || !matches!(risultato.get("refresh_token"), Some(Json::Str(_))) {
        return Err(altro("Microsoft security proxy token missing"));
    }
    f.rinnova_bootstrap().await?;
    Ok(risultato)
}
