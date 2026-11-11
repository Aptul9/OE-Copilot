//! Gli errori degli accessi Microsoft e Google con le classi dell'agente TypeScript: chi li prende sceglie la risposta dalla classe
//! (needsProxy, needsBrowser, un accesso da rifare, un errore da mostrare).

use crate::trasporto::ErroreRete;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ErroreAccesso {
    /// `ProxyBrowserRequired`: serve la sessione del browser del proxy di sicurezza di Microsoft.
    ProxyBrowser(String),
    /// `SessioneScaduta`: il browser va riaperto dal proprietario, niente da riprovare.
    SessioneScaduta(String),
    /// `SharedRefreshExpired`.
    SharedRefreshExpired(String),
    /// `SharedAuthorizationRefused`.
    SharedRifiutato(String),
    /// `ErroreAccessoScaduto`: l'accesso salvato non vale più.
    Scaduto(String),
    /// Un errore della rete (fetch failed, timeout, risposta troppo grande).
    Rete(ErroreRete),
    /// Un `Error` qualunque, con il suo testo.
    Altro(String),
}

impl ErroreAccesso {
    pub fn altro(m: impl Into<String>) -> Self {
        ErroreAccesso::Altro(m.into())
    }

    pub fn messaggio(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for ErroreAccesso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroreAccesso::ProxyBrowser(m)
            | ErroreAccesso::SessioneScaduta(m)
            | ErroreAccesso::SharedRefreshExpired(m)
            | ErroreAccesso::SharedRifiutato(m)
            | ErroreAccesso::Scaduto(m)
            | ErroreAccesso::Altro(m) => f.write_str(m),
            ErroreAccesso::Rete(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ErroreAccesso {}

impl From<ErroreRete> for ErroreAccesso {
    fn from(e: ErroreRete) -> Self {
        ErroreAccesso::Rete(e)
    }
}

pub type EsitoAccesso<T> = Result<T, ErroreAccesso>;
