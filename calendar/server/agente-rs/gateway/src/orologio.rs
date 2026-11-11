//! L'ora in un punto solo, così le prove la possono fissare (era lib/core/orologio.ts).

use crate::fuso::da_istante;
use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

thread_local! {
    static FISSO: Cell<Option<i64>> = const { Cell::new(None) };
}

/// Millisecondi dall'epoca dell'orologio di sistema, sempre veri: Date.now().
pub fn ora_vera() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Millisecondi dall'epoca, adesso (o l'ora fissata dalle prove).
pub fn adesso() -> i64 {
    FISSO.with(Cell::get).unwrap_or_else(ora_vera)
}

/// Secondi dall'epoca con la frazione, come time.time()
pub fn secondi() -> f64 {
    adesso() as f64 / 1000.0
}

/// Fissa l'ora (None la lascia scorrere): solo per le prove.
pub fn fissa_ora(ms: Option<i64>) {
    FISSO.with(|f| f.set(ms));
}

/// AAAAMMGG-HHMMSS di Roma, come time.strftime("%Y%m%d-%H%M%S")
pub fn ora_stamp() -> String {
    let n = da_istante(adesso(), 0);
    format!("{}{:02}{:02}-{:02}{:02}{:02}", n.a, n.m, n.g, n.h, n.mi, n.s)
}
