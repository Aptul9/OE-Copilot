//! L'agente: le fonti dei calendari, i token delle caselle, il gateway di posta e chat, il server HTTP. Ogni modulo ha lo stesso nome del
//! file di `agente-ts/src` che riscrive.

pub mod comunicazione;
pub mod diretto;
pub mod fonte;
pub mod gmail;
pub mod google_communication;
pub mod http;
pub mod server;
pub mod shared_provisioning;
pub mod ultime;
