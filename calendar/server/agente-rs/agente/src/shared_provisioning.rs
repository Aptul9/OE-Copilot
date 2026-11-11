//! Attivazione esplicita di un account Microsoft condiviso: la casella, già collegata con il suo accesso, diventa la fonte unica di posta,
//! calendario e chat Teams per le capacità scelte. Tenant e object id vengono dai claims di un token appena rinnovato, mai da quello che
//! manda la pagina; l'origine dal client con cui la casella ha fatto l'accesso. Dopo il salvataggio ogni capacità scelta si prova con un
//! token vero: se una non passa, la casella torna com'era.
//!   GET  /api/auth/shared   {"accounts": [{"account", "shared": {"capabilities", "origin"} | null, "status", "message", "teams"}],
//!                            "teamsOnly": [{"email", "slot", "state", "detail"}], "teamsAt"}
//!   POST /api/auth/shared   {"account", "capabilities": ["mail", "calendar", "chat"]} attiva o cambia; {"account", "enabled": false} spegne
//! Una casella non condivisa vale posta e calendario, senza chat: è l'accesso di sempre.

use crate::diretto::{GestoreToken, Voce};
use oe_nucleo::json::{Json, Mappa};
use oe_rete::errori::{ErroreAccesso, EsitoAccesso};
use oe_rete::js::{js_trim, stringa_di, vero};
use oe_rete::outlook_rest::{claims_jwt, upn_di};
use oe_rete::sessione_web::OUTLOOK_WEB_CLIENT_ID;
use oe_rete::shared_teams::RapportoTeams;
use std::future::Future;
use std::pin::Pin;

pub const CAPABILITIES: [&str; 3] = ["mail", "calendar", "chat"];

pub type FutureAccesso<'a> = Pin<Box<dyn Future<Output = EsitoAccesso<String>> + 'a>>;

/// Quello che l'attivazione usa del gestore dei token: le caselle, un token rinnovato, la prova di una risorsa, la scrittura della
/// condivisione.
pub trait ArchivioCondiviso {
    fn voce_account(&self, email: &str) -> Option<Voce>;
    fn elenco_account(&self) -> Vec<(String, Voce)>;
    fn ottieni_access_token<'a>(&'a self, email: &'a str, forza: bool) -> FutureAccesso<'a>;
    fn ottieni_risorsa<'a>(&'a self, email: &'a str, risorsa: &'a str, forza: bool) -> FutureAccesso<'a>;
    fn imposta_shared(&self, email: &str, shared: Option<Json>) -> EsitoAccesso<()>;
}

impl ArchivioCondiviso for GestoreToken {
    fn voce_account(&self, email: &str) -> Option<Voce> {
        GestoreToken::voce_account(self, email)
    }

    fn elenco_account(&self) -> Vec<(String, Voce)> {
        GestoreToken::elenco_account(self)
    }

    fn ottieni_access_token<'a>(&'a self, email: &'a str, forza: bool) -> FutureAccesso<'a> {
        Box::pin(GestoreToken::ottieni_access_token(self, email, forza))
    }

    fn ottieni_risorsa<'a>(&'a self, email: &'a str, risorsa: &'a str, forza: bool) -> FutureAccesso<'a> {
        Box::pin(GestoreToken::ottieni_risorsa(self, email, risorsa, forza))
    }

    fn imposta_shared(&self, email: &str, shared: Option<Json>) -> EsitoAccesso<()> {
        GestoreToken::imposta_shared(self, email, shared)
    }
}

/// Le risorse che una capacità deve poter ottenere: la chat sono i quattro servizi che l'agente Teams senza browser usa.
fn prove(c: &str) -> &'static [&'static str] {
    match c {
        "chat" => &["csa", "ic3", "spaces", "presence"],
        _ => &["outlook"],
    }
}

const TEAMS_WEB_CLIENT_ID: &str = "5e3ce6c0-2b1f-4285-8d4b-75ee78787346";

#[derive(Debug, Clone)]
pub struct ErroreCondivisione {
    pub messaggio: String,
    /// L'errore di Microsoft che ha fermato la prova di una capacità.
    pub causa: Option<ErroreAccesso>,
}

impl std::fmt::Display for ErroreCondivisione {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.messaggio)
    }
}

fn rifiuta<T>(m: impl Into<String>) -> Result<T, ErroreCondivisione> {
    Err(ErroreCondivisione { messaggio: m.into(), causa: None })
}

impl From<ErroreAccesso> for ErroreCondivisione {
    fn from(e: ErroreAccesso) -> Self {
        ErroreCondivisione { messaggio: e.to_string(), causa: None }
    }
}

/// Le capacità chieste: un elenco di quelle note (anche vuoto), senza doppioni, nell'ordine fisso.
pub fn capacita_valide(valore: Option<&Json>) -> Result<Vec<&'static str>, ErroreCondivisione> {
    match valore {
        Some(Json::Arr(v)) if v.iter().all(|c| c.as_str().is_some_and(|c| CAPABILITIES.contains(&c))) => Ok(CAPABILITIES.into_iter().filter(|c| v.iter().any(|x| x.as_str() == Some(c))).collect()),
        _ => rifiuta("capabilities: un elenco tra mail, calendar, chat"),
    }
}

/// L'origine del refresh_token della casella, dal client con cui ha fatto l'accesso: decide come Microsoft lo rinnova.
pub fn origine_di(client_id: Option<&str>) -> &'static str {
    match client_id {
        Some(OUTLOOK_WEB_CLIENT_ID) => "outlook-spa",
        Some(TEAMS_WEB_CLIENT_ID) => "teams-spa",
        _ => "mobile",
    }
}

fn guid(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    p.len() == 5 && p.iter().zip([8, 4, 4, 4, 12]).all(|(x, n)| x.len() == n && x.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Tenant e object id dai claims del token della casella: devono essere GUID e il token deve essere proprio di quella casella.
pub fn identita_da_token(token: &str, casella: &str) -> Result<(String, String), ErroreCondivisione> {
    let claims = claims_jwt(token);
    let tid = claims.get("tid").and_then(Json::as_str).unwrap_or_default().to_string();
    let oid = claims.get("oid").and_then(Json::as_str).unwrap_or_default().to_string();
    if !guid(&tid) || !guid(&oid) {
        return rifiuta("il token della casella non dice tenant e object id");
    }
    if upn_di(&claims) != casella {
        return rifiuta("il token appartiene a un'altra casella");
    }
    Ok((tid, oid))
}

fn pubblico(s: Option<&Json>) -> Json {
    match s.filter(|s| vero(s)) {
        Some(s) => {
            let mut m = Mappa::new();
            m.insert("capabilities", s.get("capabilities").cloned().unwrap_or_default());
            m.insert("origin", s.get("origin").cloned().unwrap_or_default());
            Json::Obj(m)
        }
        None => Json::Null,
    }
}

/// L'elenco unico degli account: ogni casella Microsoft con le sue capacità, lo stato dell'accesso e, se TeamsRelay ha mandato un
/// rapporto recente, l'account Teams che la usa; poi gli account Teams con il solo accesso del browser, che il calendario non ha.
pub fn stato_condivisione(archivio: &dyn ArchivioCondiviso, teams: Option<&RapportoTeams>) -> Mappa {
    let mut accounts = Vec::new();
    for (account, v) in archivio.elenco_account() {
        let v = v.borrow();
        if v.get("provider").and_then(Json::as_str) != Some("microsoft") {
            continue;
        }
        let errore = v.get("errore");
        let mut m = Mappa::new();
        m.insert("account", account.as_str());
        m.insert("shared", pubblico(v.get("shared")));
        m.insert("status", if errore.is_some_and(vero) { "errore" } else { "ok" });
        m.insert("message", errore.filter(|e| !e.is_null()).map_or_else(|| Json::from(""), |e| Json::from(stringa_di(e))));
        m.insert("teams", teams.and_then(|t| t.accounts.get(&account)).map_or(Json::Null, |s| s.in_json(None)));
        accounts.push(Json::Obj(m));
    }
    let mut out = Mappa::new();
    out.insert("accounts", accounts);
    out.insert("teamsOnly", teams.map(|t| t.others.iter().map(|(e, s)| s.in_json(Some(e))).collect::<Vec<_>>()).unwrap_or_default());
    out.insert("teamsAt", teams.map_or(0, |t| t.at));
    out
}

fn condivisione(account: &str, shared: Json) -> Mappa {
    let mut m = Mappa::new();
    m.insert("account", account);
    m.insert("shared", shared);
    m
}

/// Attiva o cambia le capacità di una casella Microsoft già collegata. Una casella già condivisa tiene la sua identità; una nuova la prende
/// da un token rinnovato ora. Ogni risorsa delle capacità scelte si chiede a Microsoft con il nuovo stato: un rifiuto rimette lo stato di
/// prima e l'errore dice quale capacità non passa.
pub async fn attiva_condivisione(archivio: &dyn ArchivioCondiviso, account: &str, capabilities: Option<&Json>) -> Result<Mappa, ErroreCondivisione> {
    let casella = js_trim(account).to_lowercase();
    let caps = capacita_valide(capabilities)?;
    let Some(voce) = archivio.voce_account(&casella) else { return rifiuta("Casella non trovata") };
    let (provider, prima, refresh, errore, client_id) = {
        let v = voce.borrow();
        (v.get("provider").and_then(Json::as_str).map(str::to_string), v.get("shared").filter(|s| vero(s)).cloned(), v.get("refresh_token").is_some_and(vero), v.get("errore").filter(|e| vero(e)).map(stringa_di), v.get("client_id").and_then(Json::as_str).map(str::to_string))
    };
    if provider.as_deref() != Some("microsoft") {
        return rifiuta("Solo una casella Microsoft può essere condivisa");
    }
    let ha = |s: &Json, c: &str| s.get("capabilities").and_then(Json::as_arr).is_some_and(|v| v.iter().any(|x| x.as_str() == Some(c)));
    // togliere soltanto non chiede niente a Microsoft: si spegne Teams (o posta, o calendario) anche con l'accesso scaduto
    if let Some(p) = prima.as_ref().filter(|p| caps.iter().all(|c| ha(p, c))) {
        let mut meno = p.as_obj().cloned().unwrap_or_default();
        meno.insert("capabilities", caps.iter().map(|c| c.to_string()).collect::<Vec<_>>());
        let meno = Json::Obj(meno);
        archivio.imposta_shared(&casella, Some(meno.clone()))?;
        return Ok(condivisione(&casella, pubblico(Some(&meno))));
    }
    if !refresh {
        return rifiuta("La casella non ha un accesso rinnovabile: accedi di nuovo prima di condividerla");
    }
    if let Some(e) = errore {
        return rifiuta(format!("Accesso da rifare prima di condividere: {e}"));
    }
    let (tenant, oggetto) = match &prima {
        Some(p) => (p.get("tenantId").cloned().unwrap_or_default(), p.get("objectId").cloned().unwrap_or_default()),
        None => {
            let token = archivio.ottieni_access_token(&casella, true).await?;
            let (t, o) = identita_da_token(&token, &casella)?;
            (Json::from(t), Json::from(o))
        }
    };
    let mut nuovo = Mappa::new();
    nuovo.insert("tenantId", tenant);
    nuovo.insert("objectId", oggetto);
    nuovo.insert("capabilities", caps.iter().map(|c| c.to_string()).collect::<Vec<_>>());
    nuovo.insert("origin", prima.as_ref().and_then(|p| p.get("origin").filter(|o| !o.is_null()).cloned()).unwrap_or_else(|| Json::from(origine_di(client_id.as_deref()))));
    let nuovo = Json::Obj(nuovo);
    archivio.imposta_shared(&casella, Some(nuovo.clone()))?;
    // una risorsa sola anche quando due capacità la usano (posta e calendario: outlook)
    let mut risorse: Vec<&str> = Vec::new();
    for c in &caps {
        for r in prove(c) {
            if !risorse.contains(r) {
                risorse.push(r);
            }
        }
    }
    for risorsa in risorse {
        if let Err(e) = archivio.ottieni_risorsa(&casella, risorsa, true).await {
            archivio.imposta_shared(&casella, prima.clone())?;
            let per = caps.iter().filter(|c| prove(c).contains(&risorsa)).copied().collect::<Vec<_>>().join(", ");
            return Err(ErroreCondivisione { messaggio: format!("Capacità {per} non disponibile per questa casella ({risorsa}): {e}"), causa: Some(e) });
        }
    }
    Ok(condivisione(&casella, pubblico(Some(&nuovo))))
}

/// Spegne la condivisione: la casella torna all'accesso suo, posta e calendario come prima, nessun token per Teams.
pub fn disattiva_condivisione(archivio: &dyn ArchivioCondiviso, account: &str) -> Result<Mappa, ErroreCondivisione> {
    let casella = js_trim(account).to_lowercase();
    let Some(voce) = archivio.voce_account(&casella) else { return rifiuta("Casella non trovata") };
    let condivisa = voce.borrow().get("shared").is_some_and(vero);
    if condivisa {
        archivio.imposta_shared(&casella, None)?;
    }
    Ok(condivisione(&casella, Json::Null))
}

fn con_ok(ok: bool, resto: Mappa) -> Json {
    let mut m = Mappa::new();
    m.insert("ok", ok);
    for (k, v) in resto.iter() {
        m.insert(k.as_str(), v.clone());
    }
    Json::Obj(m)
}

/// La richiesta HTTP di /api/auth/shared, già autorizzata: il corpo della risposta, sempre 200 con ok come le altre di /api/auth.
pub async fn richiesta_condivisione(archivio: Option<&dyn ArchivioCondiviso>, post: Option<&Json>, teams: Option<&RapportoTeams>) -> Json {
    let Some(archivio) = archivio else {
        let mut m = Mappa::new();
        m.insert("ok", false);
        m.insert("errore", "gestore token non configurato");
        return Json::Obj(m);
    };
    let Some(corpo) = post else { return con_ok(true, stato_condivisione(archivio, teams)) };
    let esito = async {
        let vuoto = Json::obj();
        let r = if matches!(corpo, Json::Obj(_)) { corpo } else { &vuoto };
        let Some(Json::Str(account)) = r.get("account").filter(|a| a.as_str().is_some_and(|s| !js_trim(s).is_empty())) else { return rifiuta("account mancante") };
        if r.get("enabled") == Some(&Json::Bool(false)) {
            return disattiva_condivisione(archivio, account);
        }
        attiva_condivisione(archivio, account, r.get("capabilities")).await
    };
    match esito.await {
        Ok(m) => con_ok(true, m),
        Err(e) => {
            let mut m = Mappa::new();
            m.insert("ok", false);
            m.insert("errore", e.messaggio);
            Json::Obj(m)
        }
    }
}
