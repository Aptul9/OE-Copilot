//! I fusi che non sono nomi IANA, quelli che un VTIMEZONE definisce dentro il calendario, e la cache dei nomi già risolti. Come il tzp di icalendar
//! la cache vive quanto il processo: un nome risolto o definito una volta resta quello finché l'agente non riparte.

use crate::errori::Esito;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Un fuso definito da un VTIMEZONE. `muro` è l'ora sul muro in secondi dall'epoca letta come se fosse UTC; `fold` vale 1 per la seconda delle due ore ripetute.
pub trait FusoPersonalizzato {
    /// Lo scarto dall'UTC, in secondi (utcoffset).
    fn scarto(&self, muro: i64, fold: u8) -> Esito<i64>;
    /// Quanto l'ora legale aggiunge, in secondi, 0 se l'ora è standard (dst).
    fn dst(&self, muro: i64, fold: u8) -> Esito<i64>;
}

thread_local! {
    static PERSONALIZZATI: RefCell<HashMap<String, Rc<dyn FusoPersonalizzato>>> = RefCell::new(HashMap::new());
    static CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Il fuso definito da un VTIMEZONE con questo nome, se c'è.
pub fn fuso_personalizzato(nome: &str) -> Option<Rc<dyn FusoPersonalizzato>> {
    PERSONALIZZATI.with(|p| p.borrow().get(nome).cloned())
}

pub fn registra_fuso_personalizzato(nome: &str, fuso: Rc<dyn FusoPersonalizzato>) {
    PERSONALIZZATI.with(|p| p.borrow_mut().insert(nome.to_string(), fuso));
}

/// tzp.__tz_cache: il nome (pulito dalle barre) che si è già risolto, e il fuso a cui porta: un nome IANA o il nome di un fuso personalizzato.
pub fn cache_fusi_get(nome: &str) -> Option<String> {
    CACHE.with(|c| c.borrow().get(nome).cloned())
}

pub fn cache_fusi_ha(nome: &str) -> bool {
    CACHE.with(|c| c.borrow().contains_key(nome))
}

pub fn cache_fusi_imposta(nome: &str, fuso: &str) {
    CACHE.with(|c| c.borrow_mut().insert(nome.to_string(), fuso.to_string()));
}

/// Per le prove: un processo nuovo.
pub fn reimposta_fusi() {
    PERSONALIZZATI.with(|p| p.borrow_mut().clear());
    CACHE.with(|c| c.borrow_mut().clear());
}
