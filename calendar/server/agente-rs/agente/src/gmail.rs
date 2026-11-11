//! Gmail REST and mail-only OAuth storage. Calendar grants and calendar source lists remain unchanged.

use crate::diretto::{GOOGLE_OAUTH_CLIENT_ID, GOOGLE_OAUTH_CLIENT_SECRET, GOOGLE_OAUTH_REDIRECT_URI, GOOGLE_TOKEN_MAX_BYTES, GestoreToken, OpzioniToken};
use crate::fonte::Adesso;
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use oe_rete::errori::ErroreAccesso;
use oe_rete::google_web::ErroreGoogle;
use oe_rete::js::{lunghezza_utf16, numero_di, spazio_js, vero};
use oe_rete::outlook_rest::leggi_testo;
use oe_rete::sessione_web::{casuale_base64url, modulo, sha256_base64url};
use oe_rete::trasporto::{ErroreRete, Richiesta, Risposta, Trasporto};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/";
const SCOPES: &str = "https://www.googleapis.com/auth/gmail.modify openid email";
const MAX_BYTES: usize = 8 * 1024 * 1024;
const QUERY: [&str; 11] = ["maxResults", "pageToken", "q", "labelIds", "includeSpamTrash", "format", "metadataHeaders", "fields", "startHistoryId", "historyTypes", "labelId"];

fn errore(messaggio: impl Into<String>, status: u16) -> ErroreGoogle {
    ErroreGoogle::http(messaggio, status)
}

/// `/^[A-Za-z0-9_-]{1,2048}$/`
fn id_valido(v: &str) -> bool {
    (1..=2048).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn oggetto(v: Option<&Json>) -> Result<&Mappa, ErroreGoogle> {
    match v {
        Some(Json::Obj(m)) => Ok(m),
        _ => Err(errore("JSON object required", 400)),
    }
}

/// `/^[^\s@]+@[^\s@]+$/`, al massimo 320 caratteri, in minuscolo.
pub fn email(v: Option<&Json>) -> Result<String, ErroreGoogle> {
    let Some(Json::Str(s)) = v else { return Err(errore("exact account email required", 400)) };
    let vietato = |c: char| spazio_js(c) || c == '@';
    let forma = s.split_once('@').is_some_and(|(l, d)| !l.is_empty() && !d.is_empty() && !l.chars().any(vietato) && !d.chars().any(vietato));
    if lunghezza_utf16(s) > 320 || !forma {
        return Err(errore("exact account email required", 400));
    }
    Ok(s.to_lowercase())
}

fn identificativo(v: Option<&Json>) -> Result<String, ErroreGoogle> {
    match v {
        Some(Json::Str(s)) if id_valido(s) => Ok(s.clone()),
        _ => Err(errore("invalid Gmail id", 400)),
    }
}

fn solo_campi(data: &Mappa, ammessi: &[&str], messaggio: &str) -> Result<(), ErroreGoogle> {
    if data.keys().any(|k| !ammessi.contains(&k.as_str())) { Err(errore(messaggio, 400)) } else { Ok(()) }
}

pub fn gmail_read_path(v: Option<&Json>) -> Result<String, ErroreGoogle> {
    let non_valido = || errore("invalid Gmail path", 400);
    let Some(Json::Str(valore)) = v else { return Err(non_valido()) };
    if lunghezza_utf16(valore) > 8192 || valore.is_empty() || !valore.bytes().all(|b| (0x21..=0x7e).contains(&b)) || valore.contains(['#', '\\']) {
        return Err(non_valido());
    }
    let parti: Vec<&str> = valore.split('?').collect();
    if parti.len() > 2 {
        return Err(non_valido());
    }
    let p = parti[0];
    let segmenti: Vec<&str> = p.split('/').collect();
    let s = |i: usize| segmenti.get(i).copied().unwrap_or_default();
    let tipo = s(0);
    let valido = p == "profile"
        || p == "history"
        || (["labels", "messages", "threads", "drafts"].contains(&tipo)
            && (segmenti.len() == 1
                || (segmenti.len() == 2 && id_valido(s(1)))
                || (tipo == "messages" && segmenti.len() == 4 && id_valido(s(1)) && s(2) == "attachments" && id_valido(s(3)))));
    if !valido {
        return Err(errore("Gmail read path not allowed", 400));
    }
    let query: Vec<(String, String)> = form_urlencoded::parse(parti.get(1).copied().unwrap_or_default().as_bytes()).into_owned().collect();
    let mut visti: Vec<&str> = Vec::new();
    for (chiave, v) in &query {
        let ripetibile = ["labelIds", "metadataHeaders", "historyTypes"].contains(&chiave.as_str());
        if !QUERY.contains(&chiave.as_str()) || (visti.contains(&chiave.as_str()) && !ripetibile) {
            return Err(errore("Gmail query not allowed", 400));
        }
        visti.push(chiave);
        if chiave == "maxResults" && !((1..=3).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()) && v.parse::<u32>().is_ok_and(|n| (1..=500).contains(&n))) {
            return Err(errore("maxResults must be 1..500", 400));
        }
        if chiave == "format" && !["full", "minimal", "raw", "metadata"].contains(&v.as_str()) {
            return Err(errore("invalid Gmail format", 400));
        }
        if chiave == "includeSpamTrash" && v != "true" && v != "false" {
            return Err(errore("invalid includeSpamTrash", 400));
        }
        if v.chars().any(|c| (c as u32) < 0x20 || c == '\x7f') {
            return Err(errore("invalid Gmail query value", 400));
        }
    }
    if query.is_empty() {
        return Ok(p.to_string());
    }
    Ok(format!("{p}?{}", form_urlencoded::Serializer::new(String::new()).extend_pairs(&query).finish()))
}

struct Attesa {
    verifier: String,
    account: Option<String>,
    creato: i64,
}

pub struct GmailGateway {
    pub file: PathBuf,
    pub tokens: GestoreToken,
    trasporto: Rc<dyn Trasporto>,
    adesso: Adesso,
    pending: RefCell<IndexMap<String, Attesa>>,
}

pub struct Inizio {
    pub url: String,
    pub state: String,
}

impl GmailGateway {
    pub fn nuovo(file: PathBuf, trasporto: Rc<dyn Trasporto>, adesso: Option<Adesso>) -> Self {
        let adesso = adesso.unwrap_or_else(|| Rc::new(crate::fonte::adesso_vero));
        let tokens = GestoreToken::nuovo(file.clone(), OpzioniToken { token_posta: None, adesso: Some(adesso.clone()), trasporto: trasporto.clone(), strict: true });
        GmailGateway { file, tokens, trasporto, adesso, pending: RefCell::new(IndexMap::new()) }
    }

    pub fn accounts(&self) -> Result<Vec<Json>, ErroreGoogle> {
        self.tokens.carica().map_err(|e| ErroreGoogle::Altro(e.to_string()))?;
        let mut out = Vec::new();
        for (account, voce) in self.tokens.elenco_account() {
            let v = voce.borrow();
            if v.get("provider").and_then(Json::as_str) != Some("google") || self.tokens.rimosso(&account) {
                continue;
            }
            let errore = v.get("errore").is_some_and(vero);
            let mut m = Mappa::new();
            m.insert("account", account.as_str());
            m.insert("provider", "gmail");
            m.insert("status", if errore { "errore" } else { "ok" });
            m.insert("message", if errore { "Gmail sign-in required" } else { "" });
            out.push(Json::Obj(m));
        }
        Ok(out)
    }

    fn configured(&self, account: &str) -> Result<(), ErroreGoogle> {
        if self.accounts()?.iter().any(|a| a["account"].as_str() == Some(account)) { Ok(()) } else { Err(errore("Gmail account not found", 404)) }
    }

    async fn token(&self, account: &str, force: bool) -> Result<String, ErroreGoogle> {
        match self.tokens.ottieni_access_token(account, force).await {
            Ok(v) => {
                self.configured(account)?;
                Ok(v)
            }
            Err(ErroreAccesso::Scaduto(_)) => Err(errore("Gmail sign-in required", 409)),
            Err(_) => Err(errore("Gmail token refresh failed", 502)),
        }
    }

    async fn request(&self, account: &str, relativo: &str, metodo: &str, corpo: Option<Json>) -> Result<Mappa, ErroreGoogle> {
        self.configured(account)?;
        for force in [false, true] {
            let token = self.token(account, force).await?;
            let mut r = Richiesta::get(format!("{BASE}{relativo}")).metodo(metodo).intestazione("Authorization", format!("Bearer {token}")).intestazione("Accept", "application/json");
            if let Some(c) = &corpo {
                r = r.intestazione("Content-Type", "application/json");
                r.corpo = Some(c.in_js().into_bytes());
            }
            let risposta = self
                .trasporto
                .invia(r.senza_redirect().scadenza(Duration::from_millis(30_000)).max_byte(Some(MAX_BYTES)))
                .await
                .map_err(|_| errore("Gmail transport failed; write outcome may be unknown", 502))?;
            // A rejected 401 is safe to retry. Transport failures, 5xx and decode failures never replay a send.
            if risposta.stato == 401 && !force {
                continue;
            }
            return parse(&risposta, MAX_BYTES);
        }
        Err(errore("Gmail sign-in required", 409))
    }

    pub async fn read(&self, input: &Json) -> Result<Json, ErroreGoogle> {
        let data = oggetto(Some(input))?;
        solo_campi(data, &["account", "path"], "unexpected Gmail read field")?;
        let account = email(data.get("account"))?;
        let percorso = gmail_read_path(data.get("path"))?;
        Ok(Json::Obj(self.request(&account, &percorso, "GET", None).await?))
    }

    pub async fn write(&self, input: &Json) -> Result<Json, ErroreGoogle> {
        let data = oggetto(Some(input))?;
        if data.get("confirm") != Some(&Json::Bool(true)) {
            return Err(errore("confirm: true required", 400));
        }
        let account = email(data.get("account"))?;
        let azione = data.get("action").and_then(Json::as_str).unwrap_or_default();
        let fatto = match azione {
            "send" | "draft" | "update_draft" => {
                let mut campi = vec!["account", "action", "confirm", "raw", "thread_id"];
                if azione == "update_draft" {
                    campi.push("draft_id");
                }
                solo_campi(data, &campi, "unexpected Gmail send field")?;
                let raw = match data.get("raw") {
                    Some(Json::Str(r)) if lunghezza_utf16(r) <= MAX_BYTES && base64url_mime(r) => r.clone(),
                    _ => return Err(errore("raw must be base64url RFC2822 MIME within size limit", 400)),
                };
                let mut messaggio = Mappa::new();
                messaggio.insert("raw", raw);
                if data.get("thread_id").is_some_and(vero) {
                    messaggio.insert("threadId", identificativo(data.get("thread_id"))?);
                }
                if azione == "update_draft" {
                    let id = identificativo(data.get("draft_id"))?;
                    let mut corpo = Mappa::new();
                    corpo.insert("id", id.as_str());
                    corpo.insert("message", Json::Obj(messaggio));
                    self.request(&account, &format!("drafts/{id}"), "PUT", Some(Json::Obj(corpo))).await?
                } else if azione == "send" {
                    self.request(&account, "messages/send", "POST", Some(Json::Obj(messaggio))).await?
                } else {
                    let mut corpo = Mappa::new();
                    corpo.insert("message", Json::Obj(messaggio));
                    self.request(&account, "drafts", "POST", Some(Json::Obj(corpo))).await?
                }
            }
            "send_draft" | "delete_draft" => {
                solo_campi(data, &["account", "action", "confirm", "draft_id"], "unexpected draft field")?;
                let id = identificativo(data.get("draft_id"))?;
                if azione == "send_draft" {
                    let mut corpo = Mappa::new();
                    corpo.insert("id", id);
                    self.request(&account, "drafts/send", "POST", Some(Json::Obj(corpo))).await?
                } else {
                    self.request(&account, &format!("drafts/{id}"), "DELETE", None).await?
                }
            }
            "modify" => {
                solo_campi(data, &["account", "action", "confirm", "message_id", "add_labels", "remove_labels"], "unexpected label field")?;
                let etichette = |v: Option<&Json>| -> Result<Vec<String>, ErroreGoogle> {
                    match v {
                        None => Ok(Vec::new()),
                        Some(Json::Arr(a)) if a.len() <= 100 && a.iter().all(|x| x.as_str().is_some_and(id_valido)) => Ok(a.iter().filter_map(Json::as_str).map(str::to_string).collect()),
                        _ => Err(errore("invalid Gmail labels", 400)),
                    }
                };
                let mut corpo = Mappa::new();
                corpo.insert("addLabelIds", etichette(data.get("add_labels"))?);
                corpo.insert("removeLabelIds", etichette(data.get("remove_labels"))?);
                let id = identificativo(data.get("message_id"))?;
                self.request(&account, &format!("messages/{id}/modify"), "POST", Some(Json::Obj(corpo))).await?
            }
            "trash" | "untrash" => {
                solo_campi(data, &["account", "action", "confirm", "message_id"], "unexpected Gmail action field")?;
                let id = identificativo(data.get("message_id"))?;
                self.request(&account, &format!("messages/{id}/{azione}"), "POST", Some(Json::obj())).await?
            }
            _ => return Err(errore("Gmail action not available", 400)),
        };
        Ok(Json::Obj(fatto))
    }

    /// L'accesso PKCE di Gmail, con i soli permessi della posta.
    pub fn begin(&self, account: Option<&str>) -> Result<Inizio, ErroreGoogle> {
        let adesso = (self.adesso)();
        self.pending.borrow_mut().retain(|_, v| adesso - v.creato <= 900_000);
        if self.pending.borrow().len() >= 20 {
            return Err(errore("too many pending Gmail sign-ins", 429));
        }
        let account = match account.filter(|a| !a.is_empty()) {
            Some(a) => Some(email(Some(&Json::from(a)))?),
            None => None,
        };
        let verifier = casuale_base64url(32);
        let state = casuale_base64url(24);
        let sfida = sha256_base64url(&verifier);
        let mut campi = vec![
            ("client_id", GOOGLE_OAUTH_CLIENT_ID),
            ("redirect_uri", GOOGLE_OAUTH_REDIRECT_URI),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", sfida.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
        ];
        if let Some(a) = &account {
            campi.push(("login_hint", a));
        }
        let url = format!("https://accounts.google.com/o/oauth2/v2/auth?{}", modulo(&campi));
        self.pending.borrow_mut().insert(state.clone(), Attesa { verifier, account, creato: adesso });
        Ok(Inizio { url, state })
    }

    async fn exchange(&self, parametri: &[(&str, &str)]) -> Result<Mappa, ErroreGoogle> {
        let r = Richiesta::post("https://oauth2.googleapis.com/token", modulo(parametri))
            .intestazione("Content-Type", "application/x-www-form-urlencoded")
            .senza_redirect()
            .scadenza(Duration::from_millis(15_000))
            .max_byte(Some(GOOGLE_TOKEN_MAX_BYTES));
        let risposta = self.trasporto.invia(r).await.map_err(|_| errore("Gmail OAuth exchange failed", 502))?;
        let esito = parse(&risposta, GOOGLE_TOKEN_MAX_BYTES)?;
        if !matches!(esito.get("access_token"), Some(Json::Str(t)) if !t.is_empty()) {
            return Err(errore("Gmail OAuth returned no access token", 502));
        }
        Ok(esito)
    }

    pub async fn complete(&self, codice: &str, state: Option<&str>) -> Result<String, ErroreGoogle> {
        let mut code = codice.to_string();
        let mut state = state.filter(|s| !s.is_empty()).map(str::to_string);
        if code.contains("://") {
            let u = url::Url::parse(&code).map_err(|_| errore("invalid OAuth callback", 400))?;
            let p = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
            if p("error").is_some_and(|e| !e.is_empty()) {
                return Err(errore("Gmail consent rejected", 400));
            }
            let dato = p("state").filter(|s| !s.is_empty());
            match (&dato, &state) {
                (None, _) => return Err(errore("OAuth state mismatch", 400)),
                (Some(d), Some(s)) if d != s => return Err(errore("OAuth state mismatch", 400)),
                _ => {}
            }
            state = dato;
            code = p("code").unwrap_or_default();
        }
        let adesso = (self.adesso)();
        let valida = state.as_ref().and_then(|s| self.pending.borrow().get(s).map(|a| adesso - a.creato <= 900_000));
        if valida != Some(true) || code.is_empty() {
            return Err(errore("OAuth state missing or expired", 400));
        }
        let Some(attesa) = state.as_ref().and_then(|s| self.pending.borrow_mut().shift_remove(s)) else { return Err(errore("OAuth state missing or expired", 400)) };
        let token = self
            .exchange(&[
                ("client_id", GOOGLE_OAUTH_CLIENT_ID),
                ("client_secret", GOOGLE_OAUTH_CLIENT_SECRET),
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", GOOGLE_OAUTH_REDIRECT_URI),
                ("code_verifier", &attesa.verifier),
            ])
            .await?;
        let refresh = token.get("refresh_token").and_then(Json::as_str).map(str::to_string);
        self.save_verified(attesa.account.as_deref(), GOOGLE_OAUTH_CLIENT_ID, Some(GOOGLE_OAUTH_CLIENT_SECRET), refresh, &token).await
    }

    /// Le credenziali OAuth di gws (client_id, refresh_token e client_secret facoltativo), verificate con un rinnovo e il profilo.
    pub async fn import(&self, input: &Json) -> Result<String, ErroreGoogle> {
        let data = oggetto(Some(input))?;
        let account = email(data.get("account"))?;
        let credenziali = oggetto(data.get("credentials"))?;
        let mut presi: Vec<String> = Vec::new();
        for chiave in ["client_id", "refresh_token"] {
            match credenziali.get(chiave) {
                Some(Json::Str(s)) if !s.is_empty() && lunghezza_utf16(s) <= 8192 => presi.push(s.clone()),
                _ => return Err(errore(format!("credentials missing {chiave}"), 400)),
            }
        }
        let segreto = match credenziali.get("client_secret") {
            None => None,
            Some(Json::Str(s)) if lunghezza_utf16(s) <= 2048 => Some(s.clone()).filter(|s| !s.is_empty()),
            Some(_) => return Err(errore("invalid OAuth client secret", 400)),
        };
        let mut parametri: Vec<(&str, &str)> = vec![("client_id", &presi[0]), ("refresh_token", &presi[1]), ("grant_type", "refresh_token")];
        if let Some(s) = &segreto {
            parametri.push(("client_secret", s));
        }
        let token = self.exchange(&parametri).await?;
        let refresh = token.get("refresh_token").and_then(Json::as_str).map(str::to_string).or_else(|| Some(presi[1].clone()));
        self.save_verified(Some(&account), &presi[0], segreto.as_deref(), refresh, &token).await
    }

    async fn save_verified(&self, atteso: Option<&str>, client_id: &str, client_secret: Option<&str>, refresh: Option<String>, token: &Mappa) -> Result<String, ErroreGoogle> {
        let access = token.get("access_token").map_or_else(|| "undefined".to_string(), oe_rete::js::stringa_di);
        let r = Richiesta::get(format!("{BASE}profile"))
            .intestazione("Authorization", format!("Bearer {access}"))
            .intestazione("Accept", "application/json")
            .senza_redirect()
            .scadenza(Duration::from_millis(15_000))
            .max_byte(Some(MAX_BYTES));
        let risposta: Risposta = self.trasporto.invia(r).await.map_err(|e: ErroreRete| ErroreGoogle::Altro(e.to_string()))?;
        let profilo = parse(&risposta, MAX_BYTES)?;
        let account = email(profilo.get("emailAddress"))?;
        if atteso.is_some_and(|a| a != account) {
            return Err(errore("Gmail returned another account", 409));
        }
        let durata = token.get("expires_in").map_or(f64::NAN, numero_di);
        let precedenti = {
            self.tokens.carica().map_err(|e| ErroreGoogle::Altro(e.to_string()))?;
            self.tokens.elenco_account()
        };
        let refresh = match refresh.filter(|r| !r.is_empty()) {
            Some(r) => Json::from(r),
            None => precedenti.iter().find(|(k, _)| *k == account).and_then(|(_, v)| v.borrow().get("refresh_token").cloned()).unwrap_or(Json::Null),
        };
        if !vero(&refresh) {
            return Err(errore("Gmail returned no refresh token; consent required", 409));
        }
        let mut voce = Mappa::new();
        voce.insert("provider", "google");
        voce.insert("client_id", client_id);
        if let Some(s) = client_secret {
            voce.insert("client_secret", s);
        }
        voce.insert("refresh_token", refresh);
        voce.insert("access_token", access);
        voce.insert("expires_at", ((self.adesso)() as f64 / 1000.0).floor() + if durata.is_finite() && durata > 0.0 { durata } else { 3600.0 });
        voce.insert("scopes", token.get("scope").and_then(Json::as_str).unwrap_or(SCOPES));
        let mut accounts = Mappa::new();
        for (k, v) in &precedenti {
            accounts.insert(k.as_str(), Json::Obj(v.borrow().clone()));
        }
        accounts.insert(account.as_str(), Json::Obj(voce));
        let mut dati = Mappa::new();
        dati.insert("accounts", Json::Obj(accounts));
        self.scrivi(&Json::Obj(dati)).map_err(|e| ErroreGoogle::Altro(e.to_string()))?;
        Ok(account)
    }

    fn scrivi(&self, dati: &Json) -> std::io::Result<()> {
        if let Some(cartella) = self.file.parent() {
            let mut b = std::fs::DirBuilder::new();
            b.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            b.create(cartella)?;
        }
        let tmp = oe_rete::file::con_suffisso(&self.file, &format!(".{}.{}.tmp", std::process::id(), casuale_base64url(6)));
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        use std::io::Write;
        o.open(&tmp)?.write_all(format!("{}\n", dati.in_js_rientrato(2)).as_bytes())?;
        std::fs::rename(&tmp, &self.file)
    }
}

/// `/^[A-Za-z0-9_-]+={0,2}$/`
fn base64url_mime(r: &str) -> bool {
    let corpo = r.trim_end_matches('=');
    r.len() - corpo.len() <= 2 && !corpo.is_empty() && corpo.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn parse(r: &Risposta, max: usize) -> Result<Mappa, ErroreGoogle> {
    if !r.ok() {
        let status = if [400, 403, 404, 429].contains(&r.stato) { r.stato } else { 502 };
        return Err(errore(format!("Gmail HTTP {}", r.stato), status));
    }
    let invalido = || errore("Gmail returned invalid JSON; write outcome may be unknown", 502);
    match leggi_testo(r, Some(max)) {
        Err(ErroreRete::RispostaGrande(_)) => Err(errore("Gmail response exceeds size limit", 502)),
        Err(_) => Err(invalido()),
        Ok(t) if t.is_empty() => Ok(Mappa::new()),
        Ok(t) => match Json::analizza(&t) {
            Ok(Json::Obj(m)) => Ok(m),
            _ => Err(invalido()),
        },
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn percorsi_di_lettura() {
        let p = |s: &str| gmail_read_path(Some(&Json::from(s))).map_err(|e| e.to_string());
        assert_eq!(p("messages?maxResults=1&q=is%3Aunread").unwrap(), "messages?maxResults=1&q=is%3Aunread");
        assert_eq!(p("messages?pageToken=opaque%2Bpage%2F%3D").unwrap(), "messages?pageToken=opaque%2Bpage%2F%3D");
        assert_eq!(p("messages/m1/attachments/a1").unwrap(), "messages/m1/attachments/a1");
        assert_eq!(p("messages?labelIds=A&labelIds=B").unwrap(), "messages?labelIds=A&labelIds=B");
        assert_eq!(p("settings/filters").unwrap_err(), "Gmail read path not allowed");
        assert_eq!(p("messages?maxResults=1&maxResults=2").unwrap_err(), "Gmail query not allowed");
        assert_eq!(p("a?b?c").unwrap_err(), "invalid Gmail path");
        assert!(base64url_mime("aGk") && base64url_mime("aGk=") && !base64url_mime("invalid!") && !base64url_mime("==="));
    }
}
