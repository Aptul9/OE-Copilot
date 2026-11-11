//! Gli errori del nucleo, con le stesse classi dell'agente TypeScript (`tempo.ts`, `regola.ts`): il server sceglie la risposta dalla classe.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Errore {
    /// `ErroreValore`: un ValueError di Python, il server risponde 400 con il testo.
    Valore(String),
    /// `ErroreAltro`: un errore di Python che non è un ValueError.
    Altro(String),
    /// `ErroreTipo`: un TypeError di Python; vale anche come `ErroreAltro`.
    Tipo(String),
    /// `ErroreSenzaRisposta`: il server chiude la connessione senza rispondere.
    SenzaRisposta(String),
    /// `ErroreNonSupportato`: una regola che il motore non espande.
    NonSupportato(String),
}

impl Errore {
    pub fn messaggio(&self) -> &str {
        match self {
            Errore::Valore(m) | Errore::Altro(m) | Errore::Tipo(m) | Errore::SenzaRisposta(m) | Errore::NonSupportato(m) => m,
        }
    }

    /// `instanceof ErroreValore`.
    pub fn e_valore(&self) -> bool {
        matches!(self, Errore::Valore(_))
    }

    /// `instanceof ErroreAltro`: anche un `ErroreTipo` lo è.
    pub fn e_altro(&self) -> bool {
        matches!(self, Errore::Altro(_) | Errore::Tipo(_))
    }
}

impl fmt::Display for Errore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.messaggio())
    }
}

impl std::error::Error for Errore {}

pub type Esito<T> = Result<T, Errore>;
