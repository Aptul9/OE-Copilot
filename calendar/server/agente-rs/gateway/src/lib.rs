//! Il gateway di Oe Copilot: la pagina di accesso con le sessioni, il secondo fattore e le passkey sullo stesso gateway.db di Better Auth, i
//! metodi dell'API che la pagina chiama, i loghi, la condivisione del calendario, la disponibilità pubblica, i promemoria e il server MCP
//! della posta. Ogni modulo ha lo stesso nome del file di `web-ts/src/lib` che riscrive.

pub mod agente;
pub mod api;
pub mod archivio;
pub mod auth;
pub mod b64;
pub mod caselle;
pub mod condividi;
pub mod condivisione;
pub mod condivisione_eventi;
pub mod config;
pub mod dati;
pub mod db;
pub mod disponibilita;
pub mod etichette;
pub mod eventi;
pub mod everid;
pub mod fuso;
pub mod gateway;
pub mod ics;
pub mod ics_uscita;
pub mod immagine;
pub mod impegni;
pub mod locali;
pub mod lockout;
pub mod loghi;
pub mod mcp;
pub mod orologio;
pub mod owner;
pub mod pagine;
pub mod personalizzazione;
pub mod prenota;
pub mod promemoria;
pub mod promemoria_push;
pub mod py;
pub mod rete_sicura;
pub mod safe_next;
pub mod secondo_fattore;
pub mod semaforo;
pub mod servizio;
pub mod stato;
pub mod uscita;
pub mod valore_logo;
pub mod verifica_sessione;
pub mod web;
