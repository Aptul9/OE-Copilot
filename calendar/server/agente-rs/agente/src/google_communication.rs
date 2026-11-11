//! Gmail e Google Chat con le credenziali dell'agente, separate dagli accessi ai calendari: gmail.json e google-web.json nella cartella
//! GOOGLE_STATE. Un client di Chat per casella, rifatto quando la sessione salvata cambia; un 409 di Chat segna la sessione da reimportare.

use crate::fonte::{Adesso, adesso_vero};
use crate::gmail::{GmailGateway, email};
use oe_nucleo::json::{Json, Mappa};
use oe_rete::gchat::{GoogleChat, OpzioniChat};
use oe_rete::google_web::{ArchivioGoogleWeb, ErroreGoogle, SessioneCondivisa, SessioneGoogleWeb, leggi_sessione_importata};
use oe_rete::trasporto::Trasporto;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

const DA_REIMPORTARE: &str = "Google Chat session import required";

fn errore(messaggio: &str, status: u16) -> ErroreGoogle {
    ErroreGoogle::http(messaggio, status)
}

/// La sessione è ancora quella che l'archivio tiene per la sua casella.
fn attuale(archivio: &ArchivioGoogleWeb, s: &SessioneCondivisa) -> Result<bool, ErroreGoogle> {
    let account = s.borrow().account.clone();
    Ok(archivio.sessione(&account).map_err(ErroreGoogle::Altro)?.is_some_and(|x| Rc::ptr_eq(&x, s)))
}

pub struct GoogleCommunication {
    pub gmail: GmailGateway,
    pub sessions: Rc<ArchivioGoogleWeb>,
    trasporto: Rc<dyn Trasporto>,
    now: Option<Adesso>,
    clients: RefCell<HashMap<String, Rc<GoogleChat>>>,
}

impl GoogleCommunication {
    pub fn nuovo(cartella: &Path, trasporto: Rc<dyn Trasporto>, now: Option<Adesso>) -> Self {
        GoogleCommunication {
            gmail: GmailGateway::nuovo(cartella.join("gmail.json"), trasporto.clone(), now.clone()),
            sessions: Rc::new(ArchivioGoogleWeb::nuovo(cartella.join("google-web.json"))),
            trasporto,
            now,
            clients: RefCell::default(),
        }
    }

    pub fn accounts(&self) -> Result<Vec<Json>, ErroreGoogle> {
        let mut out = self.gmail.accounts()?;
        for s in self.sessions.elenco().map_err(ErroreGoogle::Altro)? {
            let s = s.borrow();
            let errore = s.errore.as_deref().is_some_and(|e| !e.is_empty());
            let mut m = Mappa::new();
            m.insert("account", s.account.as_str());
            m.insert("provider", "gchat");
            m.insert("self_id", s.self_id.as_deref().unwrap_or_default());
            m.insert("status", if errore { "errore" } else { "ok" });
            m.insert("message", if errore { DA_REIMPORTARE } else { "" });
            out.push(Json::Obj(m));
        }
        Ok(out)
    }

    /// La sessione di Chat importata dal PC: si salva solo dopo che Chat ha detto di chi è.
    pub async fn import_chat(&self, input: &Mappa) -> Result<String, ErroreGoogle> {
        let account = email(input.get("account").filter(|v| !v.is_null()).or_else(|| input.get("email")))?;
        let ora_s = (self.now.as_ref().map_or_else(adesso_vero, |n| n()) as f64 / 1000.0).floor();
        let (user_agent, cookies) = leggi_sessione_importata(input.get("sessione").unwrap_or(&Json::Null), ora_s)?;
        let session = Rc::new(RefCell::new(SessioneGoogleWeb { account: account.clone(), user_agent, cookies, self_id: None, salvata: ora_s, errore: None }));
        let client = GoogleChat::nuovo(session.clone(), self.trasporto.clone(), OpzioniChat { now: self.now.clone(), changed: None, guardia: None })?;
        let (_, io) = client.identity().await?;
        session.borrow_mut().self_id = Some(io);
        // Validate before replacing a working session. Failed import leaves stored credentials intact.
        self.sessions.metti(session).map_err(ErroreGoogle::Altro)?;
        self.clients.borrow_mut().remove(&account);
        Ok(account)
    }

    fn client(&self, valore: Option<&Json>) -> Result<Rc<GoogleChat>, ErroreGoogle> {
        let account = email(valore)?;
        let session = self.sessions.sessione(&account).map_err(ErroreGoogle::Altro)?.ok_or_else(|| errore("Google Chat account not found", 404))?;
        if let Some(c) = self.clients.borrow().get(&account).filter(|c| Rc::ptr_eq(&c.session, &session)) {
            return Ok(c.clone());
        }
        let (archivio, s) = (self.sessions.clone(), session.clone());
        let guardia = Rc::new(move || attuale(&archivio, &s));
        let (archivio, s) = (self.sessions.clone(), session.clone());
        let changed = Rc::new(move || {
            // In-flight work from a removed/replaced session cannot reinsert that session.
            if attuale(&archivio, &s)? {
                archivio.salva().map_err(ErroreGoogle::Altro)?;
            }
            Ok(())
        });
        let o = OpzioniChat { now: self.now.clone(), changed: Some(changed), guardia: Some(guardia) };
        let client = Rc::new(GoogleChat::nuovo(session, self.trasporto.clone(), o)?);
        self.clients.borrow_mut().insert(account, client.clone());
        Ok(client)
    }

    /// Dopo un'operazione di Chat: un successo toglie il segno di sessione da reimportare, un 409 lo mette, se la sessione è ancora quella.
    fn dopo(&self, client: &GoogleChat, esito: Result<Json, ErroreGoogle>) -> Result<Json, ErroreGoogle> {
        match esito {
            Ok(v) => {
                let segnata = client.session.borrow().errore.as_deref().is_some_and(|e| !e.is_empty());
                if segnata && attuale(&self.sessions, &client.session)? {
                    client.session.borrow_mut().errore = None;
                    self.sessions.salva().map_err(ErroreGoogle::Altro)?;
                }
                Ok(v)
            }
            Err(e) => {
                if matches!(e, ErroreGoogle::Http { status: 409, .. }) && attuale(&self.sessions, &client.session)? {
                    client.session.borrow_mut().errore = Some(DA_REIMPORTARE.to_string());
                    self.sessions.salva().map_err(ErroreGoogle::Altro)?;
                }
                Err(e)
            }
        }
    }

    pub async fn read_chat(&self, data: &Mappa) -> Result<Json, ErroreGoogle> {
        let client = self.client(data.get("account"))?;
        let esito = leggi_chat(&client, data).await;
        self.dopo(&client, esito)
    }

    pub async fn write_chat(&self, data: &Mappa) -> Result<Json, ErroreGoogle> {
        let client = self.client(data.get("account"))?;
        let esito = client.write(data).await;
        self.dopo(&client, esito)
    }

    pub fn remove(&self, provider: &str, valore: Option<&Json>) -> Result<bool, ErroreGoogle> {
        let account = email(valore)?;
        if provider == "gmail" {
            let esiste = self.gmail.accounts()?.iter().any(|a| a["account"].as_str() == Some(account.as_str()));
            if esiste {
                self.gmail.tokens.rimuovi(&account).map_err(|e| ErroreGoogle::Altro(e.to_string()))?;
            }
            return Ok(esiste);
        }
        self.clients.borrow_mut().remove(&account);
        self.sessions.rimuovi(&account).map_err(ErroreGoogle::Altro)
    }
}

async fn leggi_chat(c: &GoogleChat, data: &Mappa) -> Result<Json, ErroreGoogle> {
    let topics = match data.get("topics") {
        None => 100.0,
        Some(Json::Num(n)) => *n,
        Some(_) => return Err(errore("topics must be a number", 400)),
    };
    let testo = |k: &str| match data.get(k) {
        Some(Json::Str(s)) => Some(s.as_str()),
        _ => None,
    };
    match testo("action") {
        Some("list") => c.list().await,
        Some("search") => match testo("query") {
            Some(q) => c.search(q, topics).await,
            None => Err(errore("query required", 400)),
        },
        Some("attachment") => match (testo("token"), testo("mime_type")) {
            (Some(t), Some(m)) => c.attachment(t, m).await,
            _ => Err(errore("token and mime_type required", 400)),
        },
        Some(azione @ ("read" | "group")) => match (testo("conversation"), testo("kind")) {
            (Some(conversazione), Some(kind @ ("space" | "dm"))) if azione == "group" => c.details(conversazione, kind).await,
            (Some(conversazione), Some(kind @ ("space" | "dm"))) => c.read(conversazione, kind, topics).await,
            _ => Err(errore("conversation and kind required", 400)),
        },
        _ => Err(errore("unknown Chat read action", 400)),
    }
}
