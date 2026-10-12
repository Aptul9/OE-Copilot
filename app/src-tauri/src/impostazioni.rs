use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

fn percorso(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|cartella| cartella.join("impostazioni.json"))
}

/// Le notifiche dei promemoria sono accese, salvo che l'utente le abbia spente dal menu dell'icona
pub fn notifiche_accese(app: &AppHandle) -> bool {
    percorso(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|testo| serde_json::from_str::<serde_json::Value>(&testo).ok())
        .and_then(|v| v.get("notifiche").and_then(|n| n.as_bool()))
        .unwrap_or(true)
}

#[cfg(desktop)]
pub fn salva_notifiche(app: &AppHandle, accese: bool) {
    if let Some(p) = percorso(app) {
        if let Some(cartella) = p.parent() {
            let _ = fs::create_dir_all(cartella);
        }
        let _ = fs::write(p, serde_json::json!({ "notifiche": accese }).to_string());
    }
}
