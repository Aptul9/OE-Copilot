//! La parte dell'agente che parla con Microsoft e Google: il trasporto HTTP, le sessioni, gli accessi condivisi. Ogni modulo ha lo stesso
//! nome del file di `agente-ts/src` che riscrive.

pub mod errori;
pub mod file;
pub mod google_web;
pub mod js;
pub mod mcas_auth_forms;
pub mod mcas_outlook;
pub mod outlook_rest;
pub mod prova_util;
pub mod sessione_web;
pub mod signin;
pub mod shared_account;
pub mod shared_labels;
pub mod shared_commands;
pub mod shared_teams;
pub mod trasporto;
