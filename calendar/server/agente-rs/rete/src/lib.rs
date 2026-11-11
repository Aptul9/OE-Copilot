//! La parte dell'agente che parla con Microsoft e Google: il trasporto HTTP, le sessioni, gli accessi condivisi. Ogni modulo ha lo stesso
//! nome del file di `agente-ts/src` che riscrive.

pub mod file;
pub mod js;
pub mod outlook_rest;
pub mod prova_util;
pub mod shared_commands;
pub mod shared_teams;
pub mod trasporto;
