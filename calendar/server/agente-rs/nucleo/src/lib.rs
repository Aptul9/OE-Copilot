//! Il nucleo senza rete dell'agente: i valori JSON come li scrive e li legge l'agente TypeScript, le forme di Python (repr, strip, json.dumps),
//! il tempo, le ricorrenze, i calendari ICS e le righe di calendar.db. Ogni modulo ha lo stesso nome del file di `agente-ts/src` che riscrive.

pub mod android;
pub mod entita;
pub mod errori;
pub mod fusi;
pub mod iso;
pub mod json;
pub mod html;
pub mod ics;
pub mod pyfmt;
pub mod query;
pub mod regola;
pub mod serie;
pub mod tempo;
pub mod vtimezone;
pub mod windows;
