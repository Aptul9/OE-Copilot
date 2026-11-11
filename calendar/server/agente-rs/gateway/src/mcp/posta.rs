//! Gli strumenti MCP della posta (era lib/mcp/posta.ts): le caselle Outlook con il token che l'agente dà (/api/token-posta), l'API REST di
//! Outlook o Graph per leggere, cercare, mandare, inoltrare e sistemare i messaggi, e il corpo HTML riportato a Markdown. I testi che escono
//! sono quelli del gateway TypeScript, compresi gli errori; l'elenco degli strumenti è strumenti.json, lo stesso JSON di MCP_TOOLS.

use crate::auth::cookie::codifica_componente as enc;
use crate::caselle::{dopo_js, SPAZI};
use crate::orologio::ora_vera;
use crate::py::{js_trim, js_trim_end, lunghezza_js, numero, string_js, taglia_js, Errore, Esito};
use crate::stato::confronta_utf16;
use oe_nucleo::json::{numero_js, Json, Mappa};
use oe_nucleo::ogg;
use regex::{Captures, NoExpand, Regex, RegexBuilder};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;

const CACHE_TOKEN_MS: i64 = 30_000;

/// L'elenco degli strumenti, come lo scrive JSON.stringify(MCP_TOOLS).
pub static STRUMENTI: LazyLock<Json> = LazyLock::new(|| Json::analizza(include_str!("strumenti.json")).expect("strumenti.json è JSON"));

// ---- Le regole di JavaScript che servono qui --------------------------------------------------------------------------------
/// Un valore vero per JavaScript (`if (v)`, `Boolean(v)`).
fn vero(v: Option<&Json>) -> bool {
    match v {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(Json::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(Json::Str(s)) => !s.is_empty(),
        _ => true,
    }
}

/// `a ?? b`
fn o<'a>(a: Option<&'a Json>, b: Option<&'a Json>) -> Option<&'a Json> {
    match a {
        None | Some(Json::Null) => b,
        x => x,
    }
}

/// `String(v ?? "")`
fn testo(v: Option<&Json>) -> String {
    match v {
        None | Some(Json::Null) => String::new(),
        x => string_js(x),
    }
}

/// `typeof v === "object" && v !== null`
fn oggetto(v: Option<&Json>) -> bool {
    matches!(v, Some(Json::Obj(_) | Json::Arr(_)))
}

/// Un campo di un oggetto di Outlook: il nome com'è, con l'iniziale minuscola (Graph), o senza distinzione fra maiuscole e minuscole.
fn campo<'a>(obj: Option<&'a Json>, nome: &str) -> Option<&'a Json> {
    let Some(Json::Obj(m)) = obj else { return None };
    if let Some(v) = m.get(nome) {
        return Some(v);
    }
    let mut c = nome.chars();
    let cammello = c.next().map(|p| p.to_lowercase().collect::<String>() + c.as_str()).unwrap_or_default();
    if let Some(v) = m.get(&cammello) {
        return Some(v);
    }
    let basso = nome.to_lowercase();
    m.iter().find(|(k, _)| k.to_lowercase() == basso).map(|(_, v)| v)
}

/// Le voci di `value`, vuoto se non è una lista.
fn valori(json: &Json) -> Vec<Json> {
    crate::py::lista(json.get("value")).to_vec()
}

fn arg<'a>(args: &'a Mappa, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Json::as_str)
}

fn errore(m: impl Into<String>) -> Errore {
    Errore::altro(m)
}

fn errore_rete() -> Errore {
    Errore::tipo("fetch failed")
}

fn analizza(t: &str) -> Esito<Json> {
    Json::analizza(t.strip_prefix('\u{feff}').unwrap_or(t)).map_err(|e| Errore::Altro { nome: "SyntaxError", messaggio: e.messaggio })
}

// ---- I token delle caselle ---------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Account {
    pub upn: String,
    token: String,
    exp: f64,
    endpoint_type: String,
    base_url: String,
    phone_status: Option<Json>,
    phone_msg: Option<Json>,
}

thread_local! {
    static CACHE: RefCell<Option<(i64, Vec<Account>)>> = const { RefCell::new(None) };
}

/// Le caselle con un token ancora valido, dall'agente; per 30 secondi si tiene l'ultima lista (se non è vuota).
pub async fn carica_account_posta(forza: bool) -> Esito<Vec<Account>> {
    let adesso = ora_vera();
    if !forza {
        if let Some(a) = CACHE.with(|c| c.borrow().clone()).filter(|(q, a)| adesso - q < CACHE_TOKEN_MS && !a.is_empty()).map(|(_, a)| a) {
            return Ok(a);
        }
    }
    let r = crate::agente::client()
        .get(format!("{}/api/token-posta", crate::config::agente_url()))
        .header("Authorization", format!("Bearer {}", crate::config::agente_chiave()))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| errore_rete())?;
    if !r.status().is_success() {
        return Err(errore(format!("Agente non raggiungibile per i token della posta (HTTP {})", r.status().as_u16())));
    }
    let dati = analizza(&r.text().await.map_err(|_| errore_rete())?)?;
    let adesso_s = adesso as f64 / 1000.0;
    let validi: Vec<Account> = crate::py::lista(dati.get("account"))
        .iter()
        .filter(|a| matches!(a.get("upn"), Some(Json::Str(_))) && matches!(a.get("token"), Some(Json::Str(_))) && numero(a.get("exp")) > adesso_s)
        .map(|a| Account {
            upn: string_js(a.get("upn")),
            token: string_js(a.get("token")),
            exp: numero(a.get("exp")),
            endpoint_type: string_js(a.get("endpoint_type")),
            base_url: string_js(a.get("base_url")),
            phone_status: a.get("phone_status").cloned(),
            phone_msg: a.get("phone_msg").cloned(),
        })
        .collect();
    CACHE.with(|c| *c.borrow_mut() = Some((adesso, validi.clone())));
    Ok(validi)
}

/// La casella che contiene `filtro` nell'indirizzo, o la prima; se non c'è si rilegge la lista prima di dire di no.
pub async fn scegli_account(filtro: Option<&str>, forza: bool) -> Esito<Account> {
    let mut lista = carica_account_posta(forza).await?;
    if lista.is_empty() && !forza {
        lista = carica_account_posta(true).await?;
    }
    if lista.is_empty() {
        return Err(errore("Nessuna casella Outlook con token valido trovata sul telefono di OE Copilot."));
    }
    let Some(filtro) = filtro.filter(|f| !js_trim(f).is_empty()) else { return Ok(lista.swap_remove(0)) };
    let f = js_trim(filtro).to_lowercase();
    if let Some(a) = lista.iter().find(|a| a.upn.to_lowercase().contains(&f)) {
        return Ok(a.clone());
    }
    if !forza {
        lista = carica_account_posta(true).await?;
        if let Some(a) = lista.iter().find(|a| a.upn.to_lowercase().contains(&f)) {
            return Ok(a.clone());
        }
    }
    Err(errore(format!("Casella '{filtro}' non trovata tra quelle attive sul telefono: {}", lista.iter().map(|a| a.upn.as_str()).collect::<Vec<_>>().join(", "))))
}

fn client_outlook() -> reqwest::Client {
    thread_local! {
        static C: reqwest::Client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(300)).build().expect("client HTTP");
    }
    C.with(Clone::clone)
}

/// Una richiesta all'API di Outlook della casella scelta: con un 401 si rilegge il token e si riprova una volta.
async fn richiesta_outlook(account: Option<&str>, percorso: &str, metodo: &str, dati: Option<&Json>) -> Esito<(Account, Json)> {
    let mut riprova = true;
    loop {
        let info = scegli_account(account, !riprova).await?;
        let url = format!("{}/{}", info.base_url.trim_end_matches('/'), percorso.trim_start_matches('/'));
        let m = reqwest::Method::from_bytes(metodo.as_bytes()).map_err(|e| errore(e.to_string()))?;
        let mut r = client_outlook()
            .request(m, url)
            .header("Authorization", format!("Bearer {}", info.token))
            .header("Accept", "application/json")
            .header("User-Agent", "OECopilot-MailMCP/1.0");
        if let Some(d) = dati {
            r = r.header("Content-Type", "application/json; charset=utf-8").body(d.in_js());
        }
        let res = r.send().await.map_err(|_| errore_rete())?;
        let stato = res.status();
        if stato.as_u16() == 401 && riprova {
            carica_account_posta(true).await?;
            riprova = false;
            continue;
        }
        if !stato.is_success() {
            let frase = res
                .extensions()
                .get::<hyper::ext::ReasonPhrase>()
                .map(|f| String::from_utf8_lossy(f.as_bytes()).into_owned())
                .unwrap_or_else(|| stato.canonical_reason().unwrap_or("").to_string());
            let corpo = res.text().await.unwrap_or_default();
            return Err(errore(format!("Errore API Outlook {} su {metodo} {percorso}: {}", stato.as_u16(), if corpo.is_empty() { frase } else { corpo })));
        }
        if stato.as_u16() == 204 {
            return Ok((info, Json::obj()));
        }
        let t = res.text().await.map_err(|_| errore_rete())?;
        return Ok((info, if t.is_empty() { Json::obj() } else { analizza(&t)? }));
    }
}

async fn outlook(account: Option<&str>, percorso: &str) -> Esito<(Account, Json)> {
    richiesta_outlook(account, percorso, "GET", None).await
}

// ---- HTML e Markdown ---------------------------------------------------------------------------------------------------------
/// Un'espressione di JavaScript con i flag `gi` ([\s\S] si scrive `.`): \s diventa la classe degli spazi di JavaScript.
fn re(src: &str) -> Regex {
    RegexBuilder::new(&src.replace(r"\s", SPAZI)).case_insensitive(true).dot_matches_new_line(true).build().expect("espressione fissa")
}

macro_rules! regex {
    ($nome:ident, $src:expr) => {
        static $nome: LazyLock<Regex> = LazyLock::new(|| re($src));
    };
}

regex!(TAG, r"<[^>]+>");
regex!(ENT_DEC, r"&#([0-9]+);");
regex!(ENT_HEX, r"&#x([0-9a-f]+);");
regex!(ENT_NOME, r"&[a-z0-9]+;");
regex!(STILI, r"<(style|script|head|title|xml)\b[^>]*>");
regex!(CONDIZIONALI, r"<!\[if[^\]]*\]>.*?<!\[endif\]>");
regex!(COMMENTI, r"<!--.*?-->");
regex!(PRE, r"<pre\b[^>]*>(.*?)</pre>");
regex!(CODE, r"<code\b[^>]*>(.*?)</code>");
regex!(GRASSETTO, r"<(b|strong)(?:\s[^>]*)?>");
regex!(CORSIVO, r"<(i|em)(?:\s[^>]*)?>");
regex!(LINK, r#"<a\s+[^>]*href=["']([^"']+)["'][^>]*>(.*?)</a>"#);
regex!(TABELLA, r"<table\b[^>]*>(.*?)</table>");
regex!(RIGA_TAB, r"<tr\b[^>]*>(.*?)</tr>");
regex!(CELLA, r"<(?:td|th)\b[^>]*>(.*?)</(?:td|th)>");
regex!(BR, r"<br\s*/?>");
regex!(CITAZIONE, r"<blockquote\b[^>]*>(.*?)</blockquote>");
regex!(LI, r"<li\b[^>]*>");
regex!(LI_FINE, r"</li>");
regex!(BLOCCHI_FINE, r"</(p|div|tr|ul|ol|table)>");
regex!(HR, r"<hr\b[^>]*");
regex!(SPAZI_RIGA, r"[ \t]+");
regex!(A_CAPO, r" *\n *");
regex!(TROPPI_A_CAPO, r"\n{3,}");
regex!(SPAZI_TUTTI, r"\s+");
regex!(TAG_HTML, r"<\s*(?:p|div|span|b|strong|i|em|u|mark|br|hr|ul|ol|li|table|thead|tbody|tr|th|td|h[1-6]|a|blockquote|pre|code|font|img)\b");

fn carattere(code: u64, originale: &str) -> String {
    if code == 0 || code > 0x10ffff {
        return originale.to_string();
    }
    char::from_u32(code as u32).map_or_else(|| "\u{fffd}".to_string(), |c| c.to_string())
}

fn decodifica_entita_html(testo: &str) -> String {
    let s = ENT_DEC.replace_all(testo, |c: &Captures| carattere(c[1].parse::<u64>().unwrap_or(u64::MAX), &c[0]));
    let s = ENT_HEX.replace_all(&s, |c: &Captures| carattere(u64::from_str_radix(&c[1], 16).unwrap_or(u64::MAX), &c[0]));
    ENT_NOME
        .replace_all(&s, |c: &Captures| {
            match c[0].to_lowercase().as_str() {
                "&nbsp;" => " ",
                "&amp;" => "&",
                "&lt;" => "<",
                "&gt;" => ">",
                "&quot;" => "\"",
                "&#39;" | "&apos;" | "&rsquo;" | "&lsquo;" => "'",
                "&ndash;" | "&mdash;" => "-",
                "&euro;" => "€",
                "&ldquo;" | "&rdquo;" => "\"",
                "&hellip;" => "...",
                _ => return c[0].to_string(),
            }
            .to_string()
        })
        .into_owned()
}

fn senza_tag(s: &str) -> String {
    TAG.replace_all(s, "").into_owned()
}

fn pulisci_safe_link(url: &str) -> String {
    if let Ok(u) = url::Url::parse(url) {
        if u.host_str().is_some_and(|h| h.to_lowercase().contains("safelinks.protection.outlook.com")) {
            if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "url") {
                if !v.is_empty() {
                    return v.into_owned();
                }
            }
        }
    }
    url.to_string()
}

/// `<(a|b)...>([\s\S]*?)</\1>` di JavaScript, che il crate regex non sa scrivere: l'apertura dà il nome, la chiusura è la prima `</nome>`
/// dopo (senza distinzione fra maiuscole e minuscole). Un'apertura senza chiusura resta com'è e si cerca dalla posizione dopo.
fn coppie(s: &str, apertura: &Regex, mut f: impl FnMut(&str) -> String) -> String {
    let basso = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let (mut da, mut cerca) = (0, 0);
    while cerca <= s.len() {
        let Some(c) = apertura.captures_at(s, cerca) else { break };
        let (m, nome) = (c.get(0).expect("sempre"), c[1].to_ascii_lowercase());
        let chiusura = format!("</{nome}>");
        match basso[m.end()..].find(&chiusura) {
            Some(p) => {
                out.push_str(&s[da..m.start()]);
                out.push_str(&f(&s[m.end()..m.end() + p]));
                da = m.end() + p + chiusura.len();
                cerca = da;
            }
            None => cerca = m.start() + s[m.start()..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out.push_str(&s[da..]);
    out
}

/// Il corpo HTML di un messaggio come Markdown leggibile: titoli, grassetti, link (senza i Safe Links di Microsoft), tabelle, citazioni ed elenchi.
pub fn html_in_markdown(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }
    let mut s = coppie(html, &STILI, |_| String::new());
    s = CONDIZIONALI.replace_all(&s, "").into_owned();
    s = COMMENTI.replace_all(&s, "").into_owned();
    for lv in 1..=6 {
        let titolo = re(&format!(r"<h{lv}\b[^>]*>(.*?)</h{lv}>"));
        s = titolo.replace_all(&s, |c: &Captures| format!("\n\n{} {}\n\n", "#".repeat(lv), js_trim(&senza_tag(&c[1])))).into_owned();
    }
    s = PRE.replace_all(&s, |c: &Captures| format!("\n```\n{}\n```\n", decodifica_entita_html(js_trim(&senza_tag(&c[1]))))).into_owned();
    s = CODE.replace_all(&s, |c: &Captures| format!("`{}`", decodifica_entita_html(js_trim(&senza_tag(&c[1]))))).into_owned();
    s = coppie(&s, &GRASSETTO, |dentro| {
        let t = js_trim(dentro);
        if t.is_empty() { String::new() } else { format!("**{t}**") }
    });
    s = coppie(&s, &CORSIVO, |dentro| {
        let t = js_trim(dentro);
        if t.is_empty() { String::new() } else { format!("*{t}*") }
    });
    s = LINK
        .replace_all(&s, |c: &Captures| {
            let url = pulisci_safe_link(&decodifica_entita_html(js_trim(&c[1])));
            let txt = pulisci_safe_link(&decodifica_entita_html(js_trim(&senza_tag(&c[2]))));
            if txt.is_empty() || txt == url { url } else { format!("[{txt}]({url})") }
        })
        .into_owned();
    s = TABELLA
        .replace_all(&s, |c: &Captures| {
            let righe: Vec<String> = RIGA_TAB.captures_iter(&c[1]).map(|r| r[1].to_string()).collect();
            if righe.is_empty() {
                return c[0].to_string();
            }
            let mut md: Vec<String> = Vec::new();
            for r in righe {
                let celle: Vec<String> = CELLA
                    .captures_iter(&r)
                    .map(|x| {
                        let t = decodifica_entita_html(&TAG.replace_all(&BR.replace_all(&x[1], " "), " ")).replace('\u{a0}', " ");
                        js_trim(&SPAZI_TUTTI.replace_all(&t, " ")).replace('|', "\\|")
                    })
                    .collect();
                if !celle.is_empty() && celle.iter().any(|x| !x.is_empty()) {
                    md.push(format!("| {} |", celle.join(" | ")));
                    if md.len() == 1 {
                        md.push(format!("| {} |", celle.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")));
                    }
                }
            }
            if md.is_empty() { String::new() } else { format!("\n\n{}\n\n", md.join("\n")) }
        })
        .into_owned();
    s = CITAZIONE
        .replace_all(&s, |c: &Captures| {
            let q = html_in_markdown(&c[1]);
            format!("\n{}\n", q.split('\n').filter(|l| !js_trim(l).is_empty()).map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n"))
        })
        .into_owned();
    s = LI.replace_all(&s, NoExpand("\n* ")).into_owned();
    s = LI_FINE.replace_all(&s, "").into_owned();
    s = BR.replace_all(&s, NoExpand("\n")).into_owned();
    s = BLOCCHI_FINE.replace_all(&s, NoExpand("\n\n")).into_owned();
    s = HR.replace_all(&s, NoExpand("\n---\n")).into_owned();
    s = senza_tag(&s);
    s = decodifica_entita_html(&s);
    s = s.replace('\u{a0}', " ").replace('\u{200b}', "").replace("\r\n", "\n");
    s = SPAZI_RIGA.replace_all(&s, " ").into_owned();
    s = A_CAPO.replace_all(&s, NoExpand("\n")).into_owned();
    s = TROPPI_A_CAPO.replace_all(&s, NoExpand("\n\n")).into_owned();
    js_trim(&s).to_string()
}

pub fn sembra_html(testo: &str) -> bool {
    !testo.is_empty() && TAG_HTML.is_match(testo)
}

fn escape_html(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Il corpo per un messaggio HTML: un HTML resta com'è, un testo si protegge e va a capo con <br>.
pub fn formatta_corpo_per_html(corpo: &str, is_html: bool) -> String {
    if corpo.is_empty() {
        return String::new();
    }
    if is_html || sembra_html(corpo) {
        return corpo.to_string();
    }
    let esc = escape_html(corpo).replace("\r\n", "\n").replace('\n', "<br>\n");
    format!("<div style=\"font-family:Aptos,Calibri,sans-serif;font-size:11pt;\">{esc}</div>")
}

// ---- Destinatari e messaggi --------------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub struct Indirizzo {
    pub address: String,
    pub name: Option<String>,
}

impl Indirizzo {
    fn json(&self) -> Json {
        let mut ea = Mappa::new();
        ea.insert("Address", self.address.clone());
        if let Some(n) = &self.name {
            ea.insert("Name", n.clone());
        }
        ogg! { "EmailAddress": Json::Obj(ea) }
    }

    fn nuovo(address: String, name: String) -> Option<Indirizzo> {
        (!address.is_empty()).then(|| Indirizzo { address, name: (!name.is_empty()).then_some(name) })
    }
}

fn indirizzi_json(v: &[Indirizzo]) -> Json {
    Json::Arr(v.iter().map(Indirizzo::json).collect())
}

fn spezza(s: &str, parti: &mut Vec<Json>) {
    for p in s.split([',', ';']) {
        let t = js_trim(p);
        if !t.is_empty() {
            parti.push(Json::Str(t.to_string()));
        }
    }
}

/// I destinatari da un testo (a@x.it; Nome <b@y.it>), da una lista o dagli oggetti di Outlook, senza doppioni.
pub fn analizza_destinatari(val: Option<&Json>) -> Vec<Indirizzo> {
    if !vero(val) {
        return Vec::new();
    }
    let mut parti: Vec<Json> = Vec::new();
    match val {
        Some(Json::Str(s)) => spezza(s, &mut parti),
        Some(Json::Arr(a)) => {
            for x in a {
                match x {
                    Json::Str(s) => spezza(s, &mut parti),
                    x => parti.push(x.clone()),
                }
            }
        }
        x => parti.push(Json::Str(string_js(x))),
    }
    let mut out: Vec<Indirizzo> = Vec::new();
    let mut visti = HashSet::new();
    for p in &parti {
        let voce = match p {
            Json::Obj(_) | Json::Arr(_) => {
                let ea = p.get("EmailAddress").filter(|e| matches!(e, Json::Obj(_) | Json::Arr(_)));
                let fonte = ea.unwrap_or(p);
                let addr = js_trim(&testo(o(o(fonte.get("Address"), fonte.get("address")), None))).to_string();
                let name = js_trim(&testo(o(fonte.get("Name"), fonte.get("name")))).to_string();
                Indirizzo::nuovo(addr, name)
            }
            Json::Str(s) => {
                static TRA_PARENTESI: LazyLock<Regex> = LazyLock::new(|| Regex::new("<([^>]+)>").expect("fissa"));
                match TRA_PARENTESI.captures(s) {
                    Some(c) => {
                        let addr = js_trim(&c[1]).to_string();
                        let nome = js_trim(&s[..c.get(0).expect("sempre").start()]);
                        let nome = nome.strip_prefix(['"', '\'']).unwrap_or(nome);
                        let nome = nome.strip_suffix(['"', '\'']).unwrap_or(nome);
                        Indirizzo::nuovo(addr, nome.to_string())
                    }
                    None => Indirizzo::nuovo(js_trim(s).to_string(), String::new()),
                }
            }
            _ => None,
        };
        if let Some(v) = voce {
            if visti.insert(v.address.to_lowercase()) {
                out.push(v);
            }
        }
    }
    out
}

/// Una data di Outlook come la mostra l'app: AAAA-MM-GG HH:MM, ora di Roma.
fn formatta_data(iso: Option<&Json>) -> String {
    let Some(Json::Str(iso)) = iso.filter(|v| vero(Some(v))) else { return "-".to_string() };
    // senza scarto, come Date di Node in un container senza TZ: l'ora è UTC
    let ms = iso
        .parse::<jiff::Timestamp>()
        .map(|t| t.as_millisecond())
        .or_else(|_| iso.parse::<jiff::civil::DateTime>().map(|d| d.to_zoned(jiff::tz::TimeZone::UTC).map_or(0, |z| z.timestamp().as_millisecond())))
        .or_else(|_| iso.parse::<jiff::civil::Date>().map(|d| d.to_zoned(jiff::tz::TimeZone::UTC).map_or(0, |z| z.timestamp().as_millisecond())));
    match ms {
        Ok(ms) => {
            let n = crate::fuso::da_istante(ms, 0);
            format!("{:04}-{:02}-{:02} {:02}:{:02}", n.a, n.m, n.g, n.h, n.mi)
        }
        Err(_) => taglia_js(iso, 16).replacen('T', " ", 1),
    }
}

fn stringa_destinatario(rec: Option<&Json>) -> String {
    if oggetto(rec) {
        let ea = o(campo(rec, "EmailAddress"), rec);
        let name = js_trim(&testo(campo(ea, "Name"))).to_string();
        let addr = js_trim(&testo(campo(ea, "Address"))).to_string();
        if !name.is_empty() && !addr.is_empty() && name != addr {
            return format!("{name} <{addr}>");
        }
        return if !addr.is_empty() { addr } else if !name.is_empty() { name } else { "?".to_string() };
    }
    match rec {
        None | Some(Json::Null) => "?".to_string(),
        x => string_js(x),
    }
}

fn lista_destinatari(recs: Option<&Json>) -> String {
    match recs {
        Some(Json::Arr(a)) if !a.is_empty() => a.iter().map(|r| stringa_destinatario(Some(r))).collect::<Vec<_>>().join(", "),
        _ => "-".to_string(),
    }
}

fn lista_piena(v: Option<&Json>) -> bool {
    matches!(v, Some(Json::Arr(a)) if !a.is_empty())
}

/// `String(v || altrimenti)`
fn testo_o(v: Option<&Json>, altrimenti: &str) -> String {
    if vero(v) { string_js(v) } else { altrimenti.to_string() }
}

fn formatta_riepilogo_messaggio(m: &Json) -> String {
    let m = Some(m);
    let mid = testo(campo(m, "Id"));
    let cid = testo(campo(m, "ConversationId"));
    let dt = formatta_data(o(campo(m, "ReceivedDateTime"), campo(m, "CreatedDateTime")));
    let sender = stringa_destinatario(o(campo(m, "From"), campo(m, "Sender")));
    let (to, cc) = (campo(m, "ToRecipients"), campo(m, "CcRecipients"));
    let subj = testo_o(campo(m, "Subject"), "(No Subject)");
    let prev = js_trim(&testo(campo(m, "BodyPreview")).replace(['\r', '\n'], " ")).to_string();
    let unread = if campo(m, "IsRead") == Some(&Json::Bool(false)) { " [UNREAD]" } else { "" };
    let draft = if campo(m, "IsDraft") == Some(&Json::Bool(true)) { " [DRAFT]" } else { "" };
    let att = if vero(campo(m, "HasAttachments")) { " [📎]" } else { "" };
    let flagged = if testo(campo(campo(m, "Flag"), "FlagStatus")).to_lowercase() == "flagged" { " [🚩]" } else { "" };
    let imp = if testo(campo(m, "Importance")).to_lowercase() == "high" { " [❗HIGH]" } else { "" };
    let mut righe = vec![format!("[{dt}]{unread}{draft}{att}{flagged}{imp} From: {sender}")];
    if lista_piena(to) {
        righe.push(format!("To: {}", lista_destinatari(to)));
    }
    if lista_piena(cc) {
        righe.push(format!("Cc: {}", lista_destinatari(cc)));
    }
    righe.push(format!("Subject: {subj}"));
    righe.push(if lunghezza_js(&prev) > 140 { format!("Preview: {}...", taglia_js(&prev, 140)) } else { format!("Preview: {prev}") });
    righe.push(format!("Id: {mid}"));
    righe.push(format!("ThreadId: {cid}"));
    righe.join("\n")
}

fn formatta_messaggio_completo(msg: &Json) -> String {
    let m = Some(msg);
    let mid = testo(campo(m, "Id"));
    let cid = testo(campo(m, "ConversationId"));
    let dt = formatta_data(o(o(campo(m, "ReceivedDateTime"), campo(m, "SentDateTime")), campo(m, "CreatedDateTime")));
    let sender = stringa_destinatario(o(campo(m, "From"), campo(m, "Sender")));
    let to = lista_destinatari(campo(m, "ToRecipients"));
    let (cc, bcc, reply_to) = (campo(m, "CcRecipients"), campo(m, "BccRecipients"), campo(m, "ReplyTo"));
    let subj = testo_o(campo(m, "Subject"), "(No Subject)");
    let is_draft = campo(m, "IsDraft") == Some(&Json::Bool(true));
    let is_read = campo(m, "IsRead");
    let imp = testo_o(campo(m, "Importance"), "Normal");
    let corpo = campo(m, "Body");
    let grezzo = testo(o(campo(corpo, "Content"), campo(m, "BodyPreview")));
    let tipo = testo_o(campo(corpo, "ContentType"), "HTML").to_uppercase();
    let md = if tipo == "HTML" { html_in_markdown(&grezzo) } else { js_trim(&grezzo).to_string() };
    let mut tag = Vec::new();
    if is_draft {
        tag.push("DRAFT".to_string());
    }
    if is_read == Some(&Json::Bool(false)) {
        tag.push("UNREAD".to_string());
    }
    if imp.to_lowercase() != "normal" {
        tag.push(format!("Importance: {imp}"));
    }
    let mut h = vec!["==================================================".to_string(), format!("From: {sender}"), format!("To: {to}")];
    if lista_piena(cc) {
        h.push(format!("Cc: {}", lista_destinatari(cc)));
    }
    if lista_piena(bcc) {
        h.push(format!("Bcc: {}", lista_destinatari(bcc)));
    }
    if lista_piena(reply_to) {
        h.push(format!("Reply-To: {}", lista_destinatari(reply_to)));
    }
    h.push(format!("Date: {dt}"));
    if !tag.is_empty() {
        h.push(format!("Status: {}", tag.join(", ")));
    }
    h.push(format!("Subject: {subj}"));
    h.push(format!("Message ID: {mid}"));
    h.push(format!("Thread ID: {cid}"));
    if let Some(Json::Arr(atts)) = msg.get("AttachmentsList").filter(|a| lista_piena(Some(a))) {
        h.push("Attachments:".to_string());
        for (i, a) in atts.iter().enumerate() {
            let a = Some(a);
            let size = numero(o(campo(a, "Size"), Some(&Json::Num(0.0))));
            let kb = if size > 0.0 { (size / 1024.0).trunc().max(1.0) } else { 0.0 };
            let inline = if vero(campo(a, "IsInline")) { ", inline" } else { "" };
            h.push(format!(
                "  #{} 📎 {} ({} KB, {}{inline}) [attachment_id: {}]",
                i + 1,
                string_js(o(campo(a, "Name"), Some(&Json::Str("attachment".into())))),
                numero_js(kb),
                testo(campo(a, "ContentType")),
                testo(campo(a, "Id"))
            ));
        }
    }
    h.push("--------------------------------------------------".to_string());
    format!("{}\n\n{md}", h.join("\n"))
}

// ---- Cartelle ----------------------------------------------------------------------------------------------------------------
fn cartella_nota(nome: &str) -> Option<&'static str> {
    Some(match nome {
        "inbox" | "posta in arrivo" => "Inbox",
        "sent" | "sentitems" | "sent items" | "posta inviata" => "SentItems",
        "drafts" | "bozze" => "Drafts",
        "archive" | "archivio" => "Archive",
        "deleted" | "deleteditems" | "deleted items" | "trash" | "posta eliminata" => "DeletedItems",
        "junk" | "junkemail" | "spam" | "posta indesiderata" => "JunkEmail",
        "outbox" | "posta in uscita" => "Outbox",
        _ => return None,
    })
}

struct Cartella {
    id: String,
    name: String,
    display_name: String,
    total: f64,
    unread: f64,
}

fn cartella_da(f: &Json, nome: String, display: String) -> Cartella {
    let zero = Json::Num(0.0);
    Cartella {
        id: testo(campo(Some(f), "Id")),
        name: nome,
        display_name: display,
        total: numero(o(campo(Some(f), "TotalItemCount"), Some(&zero))),
        unread: numero(o(campo(Some(f), "UnreadItemCount"), Some(&zero))),
    }
}

async fn elenca_cartelle(account: Option<&str>, figlie: bool) -> Esito<(Account, Vec<Cartella>)> {
    let (info, json) = outlook(account, "me/MailFolders?$top=100").await?;
    let mut cartelle = Vec::new();
    for f in valori(&json) {
        let fid = testo(campo(Some(&f), "Id"));
        let fname = testo(campo(Some(&f), "DisplayName"));
        let figli = numero(o(campo(Some(&f), "ChildFolderCount"), Some(&Json::Num(0.0))));
        cartelle.push(cartella_da(&f, fname.clone(), fname.clone()));
        if figlie && figli > 0.0 && !fid.is_empty() {
            // un errore sulle sottocartelle non ferma l'elenco
            if let Ok((_, sub)) = outlook(account, &format!("me/MailFolders/{}/childfolders?$top=100", enc(&fid))).await {
                for cf in valori(&sub) {
                    let cname = testo(campo(Some(&cf), "DisplayName"));
                    cartelle.push(cartella_da(&cf, format!("{fname}/{cname}"), cname));
                }
            }
        }
    }
    Ok((info, cartelle))
}

async fn risolvi_id_cartella(account: Option<&str>, cartella: &str) -> Esito<Option<String>> {
    let c = js_trim(cartella);
    if c.is_empty() {
        return Ok(Some("Inbox".to_string()));
    }
    if let Some(n) = cartella_nota(&c.to_lowercase()) {
        return Ok(Some(n.to_string()));
    }
    if c.starts_with("AAMk") || c.starts_with("AQMk") {
        return Ok(Some(c.to_string()));
    }
    let (_, cartelle) = elenca_cartelle(account, true).await?;
    let basso = c.to_lowercase();
    if let Some(f) = cartelle.iter().find(|f| f.name.to_lowercase() == basso || f.display_name.to_lowercase() == basso) {
        return Ok(Some(f.id.clone()));
    }
    Ok(cartelle.iter().find(|f| f.name.to_lowercase().contains(&basso)).map(|f| f.id.clone()))
}

fn normalizza_data_iso(d: Option<&str>, fine_giorno: bool) -> Option<String> {
    let s = js_trim(d?);
    if s.is_empty() {
        return None;
    }
    static GIORNO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$").expect("fissa"));
    static SCARTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[+-][0-9]{2}:?[0-9]{2}$").expect("fissa"));
    if GIORNO.is_match(s) {
        return Some(format!("{s}T{}Z", if fine_giorno { "23:59:59" } else { "00:00:00" }));
    }
    if !s.ends_with('Z') && !SCARTO.is_match(dopo_js(s, 10)) {
        return Some(format!("{s}Z"));
    }
    Some(s.to_string())
}

// ---- Gli strumenti -----------------------------------------------------------------------------------------------------------
async fn tool_list_accounts() -> Esito<String> {
    let lista = carica_account_posta(false).await?;
    if lista.is_empty() {
        return Ok("Nessuna casella Outlook trovata sul telefono di OE Copilot.".to_string());
    }
    let adesso_s = ora_vera() as f64 / 1000.0;
    let mut righe = vec![format!("Found {} active Outlook account(s) on OE Copilot:", lista.len()), String::new()];
    for a in &lista {
        let minuti = ((a.exp - adesso_s) / 60.0).trunc().max(0.0);
        let st = if vero(a.phone_status.as_ref()) { format!(", phone sync: {}", string_js(a.phone_status.as_ref())) } else { String::new() };
        let msg = if vero(a.phone_msg.as_ref()) { format!(" ({})", string_js(a.phone_msg.as_ref())) } else { String::new() };
        righe.push(format!("- {} [oe-copilot, {}, token expires in {} min{st}]{msg}", a.upn, a.endpoint_type, numero_js(minuti)));
    }
    Ok(righe.join("\n"))
}

async fn tool_list_folders(args: &Mappa) -> Esito<String> {
    let figlie = args.get("include_child_folders") != Some(&Json::Bool(false));
    let (info, cartelle) = elenca_cartelle(arg(args, "account"), figlie).await?;
    let mut righe = vec![format!("Folders for {}:", info.upn), String::new()];
    for f in cartelle {
        let unread = if f.unread > 0.0 { format!(" ({} unread)", numero_js(f.unread)) } else { String::new() };
        righe.push(format!("- {} [{} items{unread}] (id: {})", f.name, numero_js(f.total), f.id));
    }
    Ok(righe.join("\n"))
}

#[derive(Default)]
struct Ricerca<'a> {
    account: Option<&'a str>,
    folder: Option<String>,
    query: Option<String>,
    da: Option<String>,
    non_letti: bool,
    allegati: Option<bool>,
    con_bandierina: bool,
    inizio: Option<String>,
    fine: Option<String>,
    limit: f64,
}

fn pieno(v: &Option<String>) -> bool {
    v.as_ref().is_some_and(|s| !s.is_empty())
}

/// Il numero di un messaggio, `Math.max(1, Math.min(Number(v || predefinito), 100))`, NaN compreso.
fn limite(v: Option<&Json>, predefinito: f64) -> f64 {
    let l = if vero(v) { numero(v) } else { predefinito };
    if l.is_nan() { l } else { l.min(100.0).max(1.0) }
}

async fn cerca_o_elenca(r: Ricerca<'_>) -> Esito<(Account, Vec<Json>)> {
    let limit = limite(Some(&Json::Num(r.limit)), 20.0);
    let folder = r.folder.as_deref().map(js_trim).filter(|f| !f.is_empty()).unwrap_or("Inbox").to_string();
    let base = if ["all", "*", "any"].contains(&folder.to_lowercase().as_str()) {
        "me/messages".to_string()
    } else {
        match risolvi_id_cartella(r.account, &folder).await? {
            Some(fid) => format!("me/MailFolders/{}/messages", enc(&fid)),
            None => "me/messages".to_string(),
        }
    };
    let locale = pieno(&r.da) || r.con_bandierina || (pieno(&r.query) && (r.non_letti || r.allegati.is_some() || pieno(&r.inizio) || pieno(&r.fine)));
    let top = if locale { (limit * 4.0).max(50.0).min(100.0) } else { limit };
    let campi = "Id,Subject,From,Sender,ToRecipients,CcRecipients,ReceivedDateTime,CreatedDateTime,BodyPreview,HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId,ParentFolderId";
    let mut params = vec![format!("$top={}", numero_js(top)), format!("$select={campi}")];
    let inizio = normalizza_data_iso(r.inizio.as_deref(), false);
    let fine = normalizza_data_iso(r.fine.as_deref(), true);
    if let Some(q) = r.query.as_ref().filter(|q| !q.is_empty()) {
        params.push(format!("$search={}", enc(&format!("\"{q}\""))));
    } else {
        let mut filtri = Vec::new();
        if r.non_letti {
            filtri.push("IsRead eq false".to_string());
        }
        match r.allegati {
            Some(true) => filtri.push("HasAttachments eq true".to_string()),
            Some(false) => filtri.push("HasAttachments eq false".to_string()),
            None => {}
        }
        if let Some(i) = &inizio {
            filtri.push(format!("ReceivedDateTime ge {i}"));
        }
        if let Some(f) = &fine {
            filtri.push(format!("ReceivedDateTime le {f}"));
        }
        if !filtri.is_empty() {
            params.push(format!("$filter={}", enc(&filtri.join(" and "))));
        }
        params.push(format!("$orderby={}", enc("ReceivedDateTime desc")));
    }
    let (info, json) = outlook(r.account, &format!("{base}?{}", params.join("&"))).await?;
    let mut items = valori(&json);
    if locale {
        items.retain(|m| {
            let m = Some(m);
            if r.non_letti && campo(m, "IsRead") != Some(&Json::Bool(false)) {
                return false;
            }
            if r.allegati == Some(true) && !vero(campo(m, "HasAttachments")) {
                return false;
            }
            if r.allegati == Some(false) && vero(campo(m, "HasAttachments")) {
                return false;
            }
            if r.con_bandierina && testo(campo(campo(m, "Flag"), "FlagStatus")).to_lowercase() != "flagged" {
                return false;
            }
            if let Some(da) = r.da.as_ref().filter(|d| !d.is_empty()) {
                if !stringa_destinatario(o(campo(m, "From"), campo(m, "Sender"))).to_lowercase().contains(&da.to_lowercase()) {
                    return false;
                }
            }
            let dt = testo(o(campo(m, "ReceivedDateTime"), campo(m, "CreatedDateTime")));
            if inizio.as_ref().is_some_and(|i| !dt.is_empty() && confronta_utf16(&dt, i).is_lt()) {
                return false;
            }
            if fine.as_ref().is_some_and(|f| !dt.is_empty() && confronta_utf16(&dt, f).is_gt()) {
                return false;
            }
            true
        });
    }
    items.truncate(limit as usize);
    Ok((info, items))
}

fn testo_arg(args: &Mappa, k: &str) -> Option<String> {
    arg(args, k).map(str::to_string)
}

fn limit_arg(args: &Mappa) -> f64 {
    args.get("limit").and_then(Json::as_f64).unwrap_or(20.0)
}

async fn tool_list_emails(args: &Mappa) -> Esito<String> {
    let folder = arg(args, "folder").map(js_trim).filter(|f| !f.is_empty()).unwrap_or("Inbox").to_string();
    let (info, messaggi) = cerca_o_elenca(Ricerca {
        account: arg(args, "account"),
        folder: Some(folder.clone()),
        query: testo_arg(args, "query"),
        da: testo_arg(args, "from_address"),
        non_letti: vero(args.get("unread_only")),
        allegati: args.get("has_attachments").and_then(Json::as_bool),
        con_bandierina: vero(args.get("flagged_only")),
        inizio: testo_arg(args, "start_date"),
        fine: testo_arg(args, "end_date"),
        limit: limit_arg(args),
    })
    .await?;
    if messaggi.is_empty() {
        return Ok(format!("No emails found in '{folder}' for {} matching criteria.", info.upn));
    }
    Ok(format!(
        "Account: {} | Folder: {folder} | Showing {} message(s) (newest first)\n\n{}",
        info.upn,
        messaggi.len(),
        messaggi.iter().map(formatta_riepilogo_messaggio).collect::<Vec<_>>().join("\n\n")
    ))
}

async fn tool_search_emails(args: &Mappa) -> Esito<String> {
    let query = arg(args, "query").map(|q| js_trim(q).to_string()).unwrap_or_default();
    let da = arg(args, "from_address").map(|q| js_trim(q).to_string()).unwrap_or_default();
    if query.is_empty() && da.is_empty() {
        return Ok("query or from_address is required".to_string());
    }
    let (info, messaggi) = cerca_o_elenca(Ricerca {
        account: arg(args, "account"),
        folder: Some(arg(args, "folder").map(js_trim).filter(|f| !f.is_empty()).unwrap_or("all").to_string()),
        query: (!query.is_empty()).then(|| query.clone()),
        da: (!da.is_empty()).then(|| da.clone()),
        non_letti: vero(args.get("unread_only")),
        allegati: args.get("has_attachments").and_then(Json::as_bool),
        inizio: testo_arg(args, "start_date"),
        fine: testo_arg(args, "end_date"),
        limit: limit_arg(args),
        ..Default::default()
    })
    .await?;
    let desc = if query.is_empty() { format!("from:{da}") } else { query };
    if messaggi.is_empty() {
        return Ok(format!("No emails matching '{desc}' for {}.", info.upn));
    }
    Ok(format!(
        "Search results for '{desc}' ({} hit(s), account: {}):\n\n{}",
        messaggi.len(),
        info.upn,
        messaggi.iter().map(formatta_riepilogo_messaggio).collect::<Vec<_>>().join("\n\n")
    ))
}

const CAMPI_ALLEGATI: &str = "attachments?$select=Id,Name,ContentType,Size,IsInline";

/// L'elenco degli allegati dentro il messaggio (AttachmentsList), vuoto se non si legge.
async fn con_allegati(account: Option<&str>, percorso: &str, m: &mut Json) {
    let lista = match outlook(account, &format!("{percorso}/{CAMPI_ALLEGATI}")).await {
        Ok((_, j)) => valori(&j),
        Err(_) => Vec::new(),
    };
    if let Some(o) = m.as_obj_mut() {
        o.insert("AttachmentsList", Json::Arr(lista));
    }
}

async fn tool_read_email(args: &Mappa) -> Esito<String> {
    let account = arg(args, "account");
    let id = arg(args, "message_id").map(js_trim).unwrap_or("");
    if id.is_empty() {
        return Ok("message_id is required".to_string());
    }
    let percorso = format!("me/messages/{}", enc(id));
    let (_, mut msg) = outlook(account, &percorso).await?;
    if vero(campo(Some(&msg), "HasAttachments")) {
        con_allegati(account, &percorso, &mut msg).await;
    }
    Ok(formatta_messaggio_completo(&msg))
}

async fn tool_read_thread(args: &Mappa) -> Esito<String> {
    let account = arg(args, "account");
    let thread_id = arg(args, "thread_id").map(js_trim).unwrap_or("").to_string();
    if thread_id.is_empty() {
        return Ok("thread_id (or message_id) is required".to_string());
    }
    let limit = limite(args.get("limit"), 50.0);
    let mut cid = thread_id.clone();
    if !thread_id.starts_with("AAQk") {
        if let Ok((_, m)) = outlook(account, &format!("me/messages/{}?$select=ConversationId", enc(&thread_id))).await {
            if let Some(Json::Str(t)) = campo(Some(&m), "ConversationId").filter(|t| vero(Some(t))) {
                cid = t.clone();
            }
        }
    }
    let filtro = enc(&format!("ConversationId eq '{}'", cid.replace('\'', "''")));
    let campi = "Id,Subject,From,Sender,ToRecipients,CcRecipients,BccRecipients,ReplyTo,ReceivedDateTime,SentDateTime,CreatedDateTime,Body,BodyPreview,HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId";
    let (info, json) = outlook(account, &format!("me/messages?$filter={filtro}&$top={}&$select={campi}", numero_js(limit))).await?;
    let mut messaggi = valori(&json);
    if messaggi.is_empty() {
        return Ok(format!("No messages found for thread '{thread_id}'"));
    }
    let quando = |m: &Json| testo(o(o(campo(Some(m), "ReceivedDateTime"), campo(Some(m), "SentDateTime")), campo(Some(m), "CreatedDateTime")));
    messaggi.sort_by(|a, b| confronta_utf16(&quando(a), &quando(b)));
    for m in messaggi.iter_mut() {
        if vero(campo(Some(m), "HasAttachments")) {
            let mid = testo(campo(Some(m), "Id"));
            con_allegati(account, &format!("me/messages/{}", enc(&mid)), m).await;
        }
    }
    let mut partecipanti: Vec<String> = Vec::new();
    let mut visti = HashSet::new();
    for m in &messaggi {
        let mut lista: Vec<Option<&Json>> = vec![o(campo(Some(m), "From"), campo(Some(m), "Sender"))];
        for k in ["ToRecipients", "CcRecipients"] {
            lista.extend(crate::py::lista(campo(Some(m), k)).iter().map(Some));
        }
        for r in lista.into_iter().filter(|r| vero(*r)) {
            let s = stringa_destinatario(r);
            if s != "?" && visti.insert(s.to_lowercase()) {
                partecipanti.push(s);
            }
        }
    }
    let subj = testo_o(campo(Some(&messaggi[0]), "Subject"), "(No Subject)");
    let n = messaggi.len();
    let mut out = vec![
        format!("Thread: {subj} ({n} message(s), oldest to newest)"),
        format!("Account: {} | ConversationId: {cid}", info.upn),
        format!("Participants: {}", partecipanti.join(", ")),
        String::new(),
    ];
    for (i, m) in messaggi.iter().enumerate() {
        out.push(format!("--- [Message {} of {n}] ---", i + 1));
        out.push(formatta_messaggio_completo(m));
        out.push(String::new());
    }
    Ok(out.join("\n"))
}

fn unisci_destinatari(esistenti: Option<&Json>, extra: &[Indirizzo]) -> Vec<Indirizzo> {
    let mut base = analizza_destinatari(esistenti);
    let mut visti: HashSet<String> = base.iter().map(|r| r.address.to_lowercase()).collect();
    for r in extra {
        if visti.insert(r.address.to_lowercase()) {
            base.push(r.clone());
        }
    }
    base
}

fn allegati_in_uscita(attachments: Option<&Json>) -> Vec<Json> {
    static DATA_URL: LazyLock<Regex> = LazyLock::new(|| RegexBuilder::new(r"^data:([^;]+);base64,(.+)$").case_insensitive(true).build().expect("fissa"));
    let mut out = Vec::new();
    for att in crate::py::lista(attachments) {
        if !matches!(att, Json::Obj(_) | Json::Arr(_)) {
            continue;
        }
        let nome = js_trim(&testo(o(o(o(att.get("name"), att.get("filename")), att.get("Name")), Some(&Json::Str("attachment.bin".into()))))).to_string();
        let mut b64 = js_trim(&testo(o(o(att.get("content_base64"), att.get("data")), att.get("ContentBytes")))).to_string();
        let mut tipo = att.get("content_type").and_then(Json::as_str).map(str::to_string);
        if let Some(c) = DATA_URL.captures(&b64.clone()) {
            tipo = tipo.or_else(|| Some(c[1].to_string()));
            b64 = c[2].to_string();
        }
        if !b64.is_empty() {
            let mut v = Mappa::new();
            v.insert("@odata.type", "#Microsoft.OutlookServices.FileAttachment");
            v.insert("Name", nome);
            v.insert("ContentBytes", b64);
            if let Some(t) = tipo {
                v.insert("ContentType", t);
            }
            out.push(Json::Obj(v));
        }
    }
    out
}

fn indirizzi(v: &[Indirizzo]) -> String {
    v.iter().map(|r| r.address.as_str()).collect::<Vec<_>>().join(", ")
}

fn importanza(args: &Mappa) -> Option<String> {
    arg(args, "importance").filter(|i| ["Low", "Normal", "High"].contains(i)).map(str::to_string)
}

async fn tool_send_email(args: &Mappa) -> Esito<String> {
    let account = arg(args, "account");
    let subject = arg(args, "subject").map(js_trim).unwrap_or("").to_string();
    let body = arg(args, "body").unwrap_or("").to_string();
    let is_html = vero(args.get("is_html"));
    let reply_to = arg(args, "reply_to_id").map(js_trim).unwrap_or("").to_string();
    let reply_all = vero(args.get("reply_all"));
    let forward_to = arg(args, "forward_to_id").map(js_trim).unwrap_or("").to_string();
    let replace_cc = vero(args.get("replace_cc"));
    let bozza = vero(args.get("save_as_draft"));
    let to = analizza_destinatari(args.get("to"));
    let cc = analizza_destinatari(args.get("cc"));
    let bcc = analizza_destinatari(args.get("bcc"));
    let esclusi: HashSet<String> = analizza_destinatari(args.get("exclude_recipients")).iter().map(|r| r.address.to_lowercase()).collect();
    let allegati = allegati_in_uscita(args.get("attachments"));

    let bersaglio = if reply_to.is_empty() { forward_to.clone() } else { reply_to.clone() };
    if !bersaglio.is_empty() {
        if !reply_to.is_empty() && js_trim(&body).is_empty() && !bozza {
            return Ok("body comment is required when replying".to_string());
        }
        if !forward_to.is_empty() && to.is_empty() && !bozza {
            return Ok("'to' recipient is required when forwarding an email".to_string());
        }
        let azione = if !reply_to.is_empty() { if reply_all { "createreplyall" } else { "createreply" } } else { "createforward" };
        let commento = formatta_corpo_per_html(&body, is_html);
        let (info, draft) = richiesta_outlook(account, &format!("me/messages/{}/{azione}", enc(&bersaglio)), "POST", Some(&ogg! { "Comment": commento })).await?;
        let draft_id = testo(campo(Some(&draft), "Id"));
        if draft_id.is_empty() {
            return Err(errore(format!("Impossibile creare la bozza ({azione})")));
        }
        let draft_path = format!("me/messages/{}", enc(&draft_id));
        let mut to_recs = if to.is_empty() { analizza_destinatari(campo(Some(&draft), "ToRecipients")) } else { to };
        let mut cc_recs = if replace_cc || azione != "createreplyall" {
            if !cc.is_empty() {
                cc
            } else if replace_cc {
                Vec::new()
            } else {
                analizza_destinatari(campo(Some(&draft), "CcRecipients"))
            }
        } else {
            unisci_destinatari(campo(Some(&draft), "CcRecipients"), &cc)
        };
        let mut bcc_recs = unisci_destinatari(campo(Some(&draft), "BccRecipients"), &bcc);
        if !esclusi.is_empty() {
            for v in [&mut to_recs, &mut cc_recs, &mut bcc_recs] {
                v.retain(|r| !esclusi.contains(&r.address.to_lowercase()));
            }
        }
        if to_recs.is_empty() && !bozza {
            let _ = richiesta_outlook(account, &draft_path, "DELETE", None).await;
            return Err(errore("Nessun destinatario 'To' rimasto: specifica esplicitamente il parametro 'to'."));
        }
        let mut patch = Mappa::new();
        patch.insert("ToRecipients", indirizzi_json(&to_recs));
        patch.insert("CcRecipients", indirizzi_json(&cc_recs));
        patch.insert("BccRecipients", indirizzi_json(&bcc_recs));
        if !subject.is_empty() {
            patch.insert("Subject", subject.clone());
        }
        if let Some(i) = importanza(args) {
            patch.insert("Importance", i);
        }
        let passi = async {
            richiesta_outlook(account, &draft_path, "PATCH", Some(&Json::Obj(patch))).await?;
            for att in &allegati {
                richiesta_outlook(account, &format!("{draft_path}/attachments"), "POST", Some(att)).await?;
            }
            if !bozza {
                richiesta_outlook(account, &format!("{draft_path}/send"), "POST", None).await?;
            }
            Ok::<(), Errore>(())
        };
        if let Err(e) = passi.await {
            if !bozza {
                let _ = richiesta_outlook(account, &draft_path, "DELETE", None).await;
            }
            return Err(e);
        }
        let etichetta = if !reply_to.is_empty() { format!("Reply ({})", if reply_all { "reply-all" } else { "selective/single reply" }) } else { "Forward".to_string() };
        let mut parti = vec![format!("{etichetta} {} from {}.", if bozza { "saved as draft" } else { "successfully sent" }, info.upn), format!("To: [{}]", indirizzi(&to_recs))];
        if !cc_recs.is_empty() {
            parti.push(format!("Cc: [{}]", indirizzi(&cc_recs)));
        }
        if !bcc_recs.is_empty() {
            parti.push(format!("Bcc: [{}]", indirizzi(&bcc_recs)));
        }
        if bozza {
            parti.push(format!("Draft ID: {draft_id}"));
        }
        return Ok(parti.join(" | "));
    }

    if to.is_empty() && !bozza {
        return Ok("'to' recipient is required for sending new email".to_string());
    }
    if js_trim(&body).is_empty() && subject.is_empty() {
        return Ok("Cannot send empty email (both subject and body are empty)".to_string());
    }
    let mut message = Mappa::new();
    message.insert("Subject", subject.clone());
    message.insert("Body", ogg! { "ContentType": if is_html || sembra_html(&body) { "HTML" } else { "Text" }, "Content": body.clone() });
    message.insert("ToRecipients", indirizzi_json(&to));
    if !cc.is_empty() {
        message.insert("CcRecipients", indirizzi_json(&cc));
    }
    if !bcc.is_empty() {
        message.insert("BccRecipients", indirizzi_json(&bcc));
    }
    if let Some(i) = importanza(args) {
        message.insert("Importance", i);
    }
    if bozza {
        let (info, draft) = richiesta_outlook(account, "me/messages", "POST", Some(&Json::Obj(message))).await?;
        let draft_id = testo(campo(Some(&draft), "Id"));
        for att in &allegati {
            richiesta_outlook(account, &format!("me/messages/{}/attachments", enc(&draft_id)), "POST", Some(att)).await?;
        }
        return Ok(format!("Email saved as draft from {}. | Subject: {subject} | To: [{}] | Draft ID: {draft_id}", info.upn, indirizzi(&to)));
    }
    if !allegati.is_empty() {
        message.insert("Attachments", Json::Arr(allegati));
    }
    let (info, _) = richiesta_outlook(account, "me/sendmail", "POST", Some(&ogg! { "Message": Json::Obj(message), "SaveToSentItems": "true" })).await?;
    let mut parti = vec![format!("Email '{subject}' successfully sent from {}.", info.upn), format!("To: [{}]", indirizzi(&to))];
    if !cc.is_empty() {
        parti.push(format!("Cc: [{}]", indirizzi(&cc)));
    }
    if !bcc.is_empty() {
        parti.push(format!("Bcc: [{}]", indirizzi(&bcc)));
    }
    Ok(parti.join(" | "))
}

async fn tool_forward_email(args: &Mappa) -> Esito<String> {
    let id = arg(args, "message_id").or_else(|| arg(args, "forward_to_id")).unwrap_or("").to_string();
    if id.is_empty() {
        return Ok("message_id is required to forward an email".to_string());
    }
    let mut a = args.clone();
    a.insert("forward_to_id", id);
    tool_send_email(&a).await
}

/// `(n / 1024).toFixed(1)` con gli interi: a metà strada si arrotonda in su, come toFixed.
fn kb_una_cifra(n: usize) -> String {
    let decimi = (n as u128 * 10 + 512) / 1024;
    format!("{}.{}", decimi / 10, decimi % 10)
}

fn blocco_testo(t: impl Into<String>) -> Json {
    ogg! { "type": "text", "text": t.into() }
}

async fn tool_get_attachment(args: &Mappa) -> Esito<Vec<Json>> {
    let account = arg(args, "account");
    let mid = arg(args, "message_id").map(js_trim).unwrap_or("").to_string();
    if mid.is_empty() {
        return Ok(vec![blocco_testo("message_id is required")]);
    }
    let (info, elenco) = outlook(account, &format!("me/messages/{}/{CAMPI_ALLEGATI}", enc(&mid))).await?;
    let atts = valori(&elenco);
    if atts.is_empty() {
        return Ok(vec![blocco_testo(format!("No attachments on message {mid}."))]);
    }
    let att_id = arg(args, "attachment_id").map(js_trim).unwrap_or("").to_string();
    let indice = args.get("index").and_then(Json::as_f64).map(f64::trunc);
    let scelti: Vec<(usize, &Json)> = if !att_id.is_empty() {
        let v: Vec<(usize, &Json)> = atts.iter().enumerate().filter(|(_, a)| testo(campo(Some(a), "Id")) == att_id).map(|(i, a)| (i + 1, a)).collect();
        if v.is_empty() {
            return Ok(vec![blocco_testo(format!("attachment_id not found on message {mid}; it has {} attachment(s).", atts.len()))]);
        }
        v
    } else if let Some(i) = indice {
        if i < 1.0 || i > atts.len() as f64 {
            return Ok(vec![blocco_testo(format!("index {} out of range: message {mid} has {} attachment(s).", numero_js(i), atts.len()))]);
        }
        vec![(i as usize, &atts[i as usize - 1])]
    } else {
        atts.iter().enumerate().map(|(i, a)| (i + 1, a)).collect()
    };
    let mut righe = vec![format!("{} attachment(s) on message {mid}:", atts.len())];
    let mut extra = Vec::new();
    for (idx, meta) in scelti {
        let aid = testo(campo(Some(meta), "Id"));
        let percorso = format!("me/messages/{}/attachments/{}", enc(&mid), enc(&aid));
        let (_, full) = outlook(account, &percorso).await?;
        let otipo = testo(o(full.get("@odata.type"), meta.get("@odata.type"))).to_lowercase();
        let nome = string_js(o(o(campo(Some(&full), "Name"), campo(Some(meta), "Name")), Some(&Json::Str(format!("attachment-{idx}")))));
        let mime_pieno = string_js(o(o(campo(Some(&full), "ContentType"), campo(Some(meta), "ContentType")), Some(&Json::Str("application/octet-stream".into()))));
        let mime = js_trim(mime_pieno.split(';').next().unwrap_or("")).to_string();
        if otipo.contains("itemattachment") {
            let espandi = if info.endpoint_type == "graph" { "$expand=microsoft.graph.itemAttachment/item" } else { "$expand=Microsoft.OutlookServices.ItemAttachment/Item" };
            if let Ok((_, exp)) = outlook(account, &format!("{percorso}?{espandi}")).await {
                if let Some(item) = campo(Some(&exp), "Item").filter(|i| oggetto(Some(i))) {
                    righe.push(format!("  #{idx} \"{nome}\": embedded Outlook item:\n{}", formatta_messaggio_completo(item)));
                    continue;
                }
            }
            righe.push(format!("  #{idx} \"{nome}\": embedded Outlook item."));
            continue;
        }
        if otipo.contains("referenceattachment") {
            let url = testo(campo(Some(&full), "SourceUrl"));
            righe.push(js_trim_end(&format!("  #{idx} \"{nome}\": cloud link (OneDrive/SharePoint): {url}")).to_string());
            continue;
        }
        let b64 = testo(campo(Some(&full), "ContentBytes"));
        if b64.is_empty() {
            righe.push(format!("  #{idx} \"{nome}\": no content bytes returned."));
            continue;
        }
        let dati = crate::b64::decodifica_node(&b64);
        righe.push(format!("  #{idx} \"{nome}\" ({} KB, {mime})", kb_una_cifra(dati.len())));
        if mime.starts_with("image/") {
            extra.push(ogg! { "type": "image", "data": b64, "mimeType": mime });
        } else if mime.starts_with("text/") || ["application/json", "application/xml", "application/csv"].contains(&mime.as_str()) {
            let t = String::from_utf8_lossy(&dati);
            extra.push(blocco_testo(format!("--- Content of {nome} ---\n{}", taglia_js(&t, 100_000))));
        } else if dati.len() <= 8 * 1024 * 1024 {
            extra.push(ogg! { "type": "resource", "resource": ogg! { "uri": format!("attachment:///{}/{}", enc(&mid), enc(&nome)), "mimeType": mime, "blob": b64 } });
        }
    }
    let mut out = vec![blocco_testo(righe.join("\n"))];
    out.extend(extra);
    Ok(out)
}

async fn tool_manage_email(args: &Mappa) -> Esito<String> {
    let account = arg(args, "account");
    let ids: Vec<String> = match args.get("message_id") {
        Some(Json::Str(s)) => vec![s.clone()],
        Some(Json::Arr(a)) => a.iter().map(|x| string_js(Some(x))).collect(),
        _ => Vec::new(),
    };
    let azione = arg(args, "action").map(|a| js_trim(a).to_lowercase()).unwrap_or_default();
    if ids.is_empty() || azione.is_empty() {
        return Ok("message_id and action are required".to_string());
    }
    let mut dest: Option<String> = None;
    match azione.as_str() {
        "archive" => dest = Some("Archive".to_string()),
        "delete" => dest = Some("DeletedItems".to_string()),
        "move" => {
            let cartella = arg(args, "destination_folder").unwrap_or("");
            if cartella.is_empty() {
                return Ok("destination_folder is required when action is 'move'".to_string());
            }
            dest = risolvi_id_cartella(account, cartella).await?;
            if dest.is_none() {
                return Ok(format!("Destination folder '{cartella}' not found"));
            }
        }
        "mark_read" | "mark_unread" | "flag" | "unflag" => {}
        _ => return Ok(format!("Unsupported action '{azione}'. Use: mark_read, mark_unread, flag, unflag, archive, delete, move.")),
    }
    let info = scegli_account(account, false).await?;
    let mut esiti = vec![format!("Executed '{azione}' on {} message(s) for {}:", ids.len(), info.upn)];
    for mid in &ids {
        let percorso = format!("me/messages/{}", enc(mid));
        let breve = taglia_js(mid, 32);
        let (patch, esito) = match azione.as_str() {
            "mark_read" => (Some(ogg! { "IsRead": true }), "marked_read".to_string()),
            "mark_unread" => (Some(ogg! { "IsRead": false }), "marked_unread".to_string()),
            "flag" => (Some(ogg! { "Flag": ogg! { "FlagStatus": "Flagged" } }), "flagged".to_string()),
            "unflag" => (Some(ogg! { "Flag": ogg! { "FlagStatus": "NotFlagged" } }), "unflagged".to_string()),
            _ => (None, String::new()),
        };
        if let Some(p) = patch {
            richiesta_outlook(account, &percorso, "PATCH", Some(&p)).await?;
            esiti.push(format!("- {breve}...: {esito}"));
        } else if let Some(d) = &dest {
            richiesta_outlook(account, &format!("{percorso}/move"), "POST", Some(&ogg! { "DestinationId": d.clone() })).await?;
            esiti.push(format!("- {breve}...: moved_to_{d}"));
        }
    }
    Ok(esiti.join("\n"))
}

/// Esegue uno strumento: i blocchi di contenuto della risposta MCP.
pub async fn esegui(nome: &str, args: &Mappa) -> Esito<Vec<Json>> {
    let t = match nome {
        "list_accounts" => tool_list_accounts().await,
        "list_folders" => tool_list_folders(args).await,
        "list_emails" => tool_list_emails(args).await,
        "read_email" => tool_read_email(args).await,
        "read_thread" => tool_read_thread(args).await,
        "search_emails" => tool_search_emails(args).await,
        "send_email" => tool_send_email(args).await,
        "forward_email" => tool_forward_email(args).await,
        "get_attachment" => return tool_get_attachment(args).await,
        "manage_email" => tool_manage_email(args).await,
        _ => return Err(errore(format!("Unknown tool: {nome}"))),
    }?;
    Ok(vec![blocco_testo(t)])
}

/// Azzera la copia dei token (per le prove).
pub fn azzera() {
    CACHE.with(|c| *c.borrow_mut() = None);
}

#[cfg(test)]
mod prove {
    use super::*;

    fn verita() -> Json {
        Json::analizza(include_str!("../../tests/dati/oe-mcp.json")).unwrap()
    }

    #[test]
    fn l_elenco_degli_strumenti_e_quello_di_typescript() {
        let ts = include_str!("strumenti.json");
        assert_eq!(STRUMENTI.in_js(), ts);
        assert_eq!(crate::py::lista(Some(&STRUMENTI)).len(), 10);
    }

    #[test]
    fn html_in_markdown_come_typescript() {
        let v = verita();
        let ingressi = [
            "<html><head><title>x</title><style>p{color:red}</style></head><body><h1>Titolo <b>uno</b></h1><p>Ciao <strong>Mario</strong>, <i>vedi</i> <a href=\"https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fesempio.it%2Fa%3Fb%3D1&amp;data=x\">qui</a> e <a href='https://x.it'>https://x.it</a>.</p><table><tr><th>A</th><th>B|C</th></tr><tr><td>1&nbsp;</td><td>2<br>3</td></tr></table><blockquote><p>citato</p><p>due</p></blockquote><ul><li>uno</li><li>due</li></ul><hr><pre>a &lt; b</pre><code>x&amp;y</code> &#8364; &#x20AC; &euro; &zzz; <!-- c --></body></html>",
            "<b><strong>doppio</strong></b> <b></b> <em>e</em>",
            "",
            "solo testo\r\n\r\n\r\nriga",
        ];
        for (i, h) in ingressi.iter().enumerate() {
            assert_eq!(html_in_markdown(h), v["md"][i].as_str().unwrap(), "ingresso {i}");
        }
        assert_eq!(html_in_markdown("<p>&#38;amp; &#0; &#1114112;</p>"), "& &#0; &#1114112;");
    }

    #[test]
    fn destinatari_e_corpo_come_typescript() {
        let v = verita();
        let ingressi = [
            Json::Str("a@x.it; B <b@y.it>, \"Carlo\" <c@z.it>,a@X.it".into()),
            Json::analizza(r#"["d@x.it,e@x.it",{"EmailAddress":{"Address":"f@x.it","Name":"F"}},{"address":"g@x.it"},5,null]"#).unwrap(),
            ogg! { "Address": "h@x.it" },
            Json::Num(7.0),
        ];
        for (i, d) in ingressi.iter().enumerate() {
            assert_eq!(indirizzi_json(&analizza_destinatari(Some(d))).in_js(), v["dest"][i].as_str().unwrap(), "destinatari {i}");
        }
        assert_eq!(formatta_corpo_per_html("a <b\n\"c\" & d", false), v["corpo"][0].as_str().unwrap());
        assert_eq!(formatta_corpo_per_html("<p>x</p>", false), v["corpo"][1].as_str().unwrap());
        assert!(sembra_html("< div>"));
        assert_eq!(formatta_corpo_per_html("a & b\nc", false), "<div style=\"font-family:Aptos,Calibri,sans-serif;font-size:11pt;\">a &amp; b<br>\nc</div>");
    }

    #[test]
    fn i_messaggi_formattati() {
        let m = Json::analizza(
            r#"{"Id":"AAMkX","ConversationId":"AAQkY","ReceivedDateTime":"2026-11-11T09:05:00Z","From":{"EmailAddress":{"Name":"Mario","Address":"m@x.it"}},
            "ToRecipients":[{"EmailAddress":{"Address":"a@y.it"}}],"CcRecipients":[],"Subject":"","BodyPreview":"riga\r\ndue","IsRead":false,"HasAttachments":true,
            "Flag":{"FlagStatus":"Flagged"},"Importance":"High","Body":{"ContentType":"html","Content":"<p>Ciao</p>"},
            "AttachmentsList":[{"Id":"A1","Name":"f.pdf","ContentType":"application/pdf","Size":3000,"IsInline":false}]}"#,
        )
        .unwrap();
        assert_eq!(
            formatta_riepilogo_messaggio(&m),
            "[2026-11-11 10:05] [UNREAD] [📎] [🚩] [❗HIGH] From: Mario <m@x.it>\nTo: a@y.it\nSubject: (No Subject)\nPreview: riga  due\nId: AAMkX\nThreadId: AAQkY"
        );
        let c = formatta_messaggio_completo(&m);
        assert!(c.starts_with("==================================================\nFrom: Mario <m@x.it>\nTo: a@y.it\nDate: 2026-11-11 10:05\nStatus: UNREAD, Importance: High\n"), "{c}");
        assert!(c.contains("  #1 📎 f.pdf (2 KB, application/pdf) [attachment_id: A1]\n--------------------------------------------------\n\nCiao"), "{c}");
        assert_eq!(formatta_data(Some(&Json::Str("2026-07-01T10:00:00.0000000Z".into()))), "2026-07-01 12:00");
        assert_eq!(formatta_data(Some(&Json::Str("boh-TT".into()))), "boh- T");
        assert_eq!(kb_una_cifra(256), "0.3");
        assert_eq!(normalizza_data_iso(Some("2026-11-11"), true).unwrap(), "2026-11-11T23:59:59Z");
        assert_eq!(normalizza_data_iso(Some("2026-11-11T10:00:00+0100"), false).unwrap(), "2026-11-11T10:00:00+0100");
        assert_eq!(normalizza_data_iso(Some("2026-11-11T10:00"), false).unwrap(), "2026-11-11T10:00Z");
    }
}
