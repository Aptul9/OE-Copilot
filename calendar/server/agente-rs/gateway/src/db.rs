//! gateway.db: le sessioni di Better Auth e lo stato dell'app nello stesso file (era lib/db.ts). Il gateway gira in un thread solo, e per
//! ogni file c'è una connessione sola: quella delle sessioni e quella dello stato sono la stessa quando GATEWAY_DB è uno.

use rusqlite::Connection;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static APERTI: RefCell<HashMap<String, Rc<Connection>>> = RefCell::new(HashMap::new());
}

/// Apre il file (la cartella si crea) in WAL, con 8 secondi di attesa su un blocco. ":memory:" è un database in memoria.
pub fn apri(file: &str) -> rusqlite::Result<Rc<Connection>> {
    if let Some(c) = APERTI.with(|a| a.borrow().get(file).cloned()) {
        return Ok(c);
    }
    if file != ":memory:" {
        if let Some(cartella) = std::path::Path::new(file).parent() {
            let _ = std::fs::create_dir_all(cartella);
        }
    }
    let c = Connection::open(file)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.busy_timeout(std::time::Duration::from_millis(8000))?;
    let c = Rc::new(c);
    APERTI.with(|a| a.borrow_mut().insert(file.to_string(), c.clone()));
    Ok(c)
}

/// La connessione delle sessioni e degli utenti (GATEWAY_DB, di norma /dati/gateway.db).
pub fn app_db() -> rusqlite::Result<Rc<Connection>> {
    apri(&crate::config::db())
}

/// Chiude tutte le connessioni (per le prove, e all'uscita).
pub fn chiudi_tutto() {
    APERTI.with(|a| a.borrow_mut().clear());
}
