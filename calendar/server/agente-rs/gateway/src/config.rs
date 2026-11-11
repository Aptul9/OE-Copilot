//! La configurazione dall'ambiente, letta al momento dell'uso come in lib/config.ts. Le prove possono mettere valori loro, per il solo thread
//! che le esegue (`imposta_per_prova`): le variabili del processo restano quelle di partenza.

use std::cell::RefCell;
use std::collections::HashMap;

/// Il secondo fattore dell'accesso: spento, codici TOTP con codici di recupero, oppure anche le passkey (che includono il TOTP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modo2fa {
    Off,
    Totp,
    Passkey,
}

impl Modo2fa {
    pub fn nome(self) -> &'static str {
        match self {
            Modo2fa::Off => "off",
            Modo2fa::Totp => "totp",
            Modo2fa::Passkey => "passkey",
        }
    }
}

pub const PASSWORD_MIN: usize = 12;
pub const PASSWORD_MAX: usize = 128;

thread_local! {
    static PROVA: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Solo per le prove: il valore di una variabile per questo thread (None la toglie).
pub fn imposta_per_prova(nome: &str, valore: Option<&str>) {
    PROVA.with(|p| p.borrow_mut().insert(nome.to_string(), valore.map(str::to_string)));
}

/// Solo per le prove: torna all'ambiente del processo.
pub fn azzera_prova() {
    PROVA.with(|p| p.borrow_mut().clear());
}

/// `process.env[nome]`: None se manca.
pub fn var(nome: &str) -> Option<String> {
    if let Some(v) = PROVA.with(|p| p.borrow().get(nome).cloned()) {
        return v;
    }
    std::env::var(nome).ok()
}

/// `process.env[nome] || predefinito`: un valore vuoto vale come mancante.
fn o(nome: &str, predefinito: &str) -> String {
    var(nome).filter(|v| !v.is_empty()).unwrap_or_else(|| predefinito.to_string())
}

/// AUTH_2FA: senza valore è off; un valore che non è uno dei tre è un errore, così un refuso non spegne il secondo fattore in silenzio.
pub fn modo2fa_da(valore: Option<&str>) -> Result<Modo2fa, String> {
    let v = crate::py::js_trim(valore.unwrap_or("")).to_lowercase();
    match v.as_str() {
        "" | "off" => Ok(Modo2fa::Off),
        "totp" => Ok(Modo2fa::Totp),
        "passkey" => Ok(Modo2fa::Passkey),
        _ => Err(format!("AUTH_2FA non valido: \"{}\". Valori ammessi: off, totp, passkey", valore.unwrap_or(""))),
    }
}

pub fn modo2fa() -> Result<Modo2fa, String> {
    modo2fa_da(var("AUTH_2FA").as_deref())
}

/// Il secondo fattore: all'avvio un valore sbagliato ferma il gateway, quindi qui è sempre uno dei tre.
pub fn auth2fa() -> Modo2fa {
    modo2fa().unwrap_or(Modo2fa::Off)
}

/// Indirizzo pubblico: APP_URL, altrimenti https://UI_HOST
pub fn app_url() -> String {
    if let Some(e) = var("APP_URL").filter(|v| !v.is_empty()) {
        return e.trim_end_matches('/').to_string();
    }
    let host = o("UI_HOST", "localhost:3000");
    let u = if host.starts_with("http://") || host.starts_with("https://") { host } else { format!("https://{host}") };
    u.trim_end_matches('/').to_string()
}

pub fn auth_secret() -> String {
    var("BETTER_AUTH_SECRET").unwrap_or_default()
}

/// Database delle sessioni: sta in ui-dati/, la cartella che sopravvive agli aggiornamenti
pub fn db() -> String {
    o("GATEWAY_DB", "/dati/gateway.db")
}

/// La cartella dei dati dell'app (config.json, personalizzazione.json, cache, eventi personali, loghi): ui-dati/ sul server
pub fn dati() -> String {
    o("OE_DATI", "/dati")
}

/// I loghi che porta l'immagine (personale.png, l'icona degli eventi personali): LOGHI_PREDEFINITI, altrimenti logos-predefiniti nella
/// cartella di lavoro.
pub fn loghi_predefiniti() -> String {
    var("LOGHI_PREDEFINITI").filter(|v| !v.is_empty()).unwrap_or_else(|| {
        let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".to_string());
        format!("{}/logos-predefiniti", cwd.trim_end_matches('/'))
    })
}

/// Il database dello stato dell'app (configurazione ed eventi personali): lo stesso delle sessioni; senza GATEWAY_DB, gateway.db nella cartella dei dati
pub fn db_stato() -> String {
    var("GATEWAY_DB").filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{}/gateway.db", dati().trim_end_matches('/')))
}

/// L'agente sulla rete di Docker e la sua chiave
pub fn agente_url() -> String {
    o("AGENTE_URL", "http://agent:8180").trim_end_matches('/').to_string()
}

pub fn agente_chiave() -> String {
    crate::py::js_trim(&o("AGENTE_CHIAVE", "")).to_string()
}

pub fn mcp_token() -> String {
    let v = var("MCP_TOKEN").filter(|v| !v.is_empty()).or_else(|| var("AGENTE_CHIAVE")).unwrap_or_default();
    crate::py::js_trim(&v).to_string()
}

pub fn admin_email() -> String {
    crate::py::js_trim(&o("ADMIN_EMAIL", "")).to_lowercase()
}

pub fn admin_password() -> String {
    var("ADMIN_PASSWORD").unwrap_or_default()
}

/// La cartella con le pagine già costruite (l'esportazione statica di web-ts) e i file di public/.
pub fn ui() -> String {
    o("UI_STATICA", "/ui")
}
