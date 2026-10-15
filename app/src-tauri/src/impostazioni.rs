use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};

fn percorso(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|cartella| cartella.join("impostazioni.json"))
}

fn leggi(app: &AppHandle) -> Map<String, Value> {
    percorso(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|testo| serde_json::from_str::<Value>(&testo).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Un interruttore del menu dell'icona (notifiche, comparsa, anonimo) com'è stato lasciato, o il suo valore di partenza
pub fn interruttore(app: &AppHandle, nome: &str, predefinito: bool) -> bool {
    leggi(app).get(nome).and_then(Value::as_bool).unwrap_or(predefinito)
}

/// Salva un interruttore lasciando gli altri come sono
#[cfg(desktop)]
pub fn salva(app: &AppHandle, nome: &str, acceso: bool) {
    let Some(p) = percorso(app) else { return };
    let mut tutti = leggi(app);
    tutti.insert(nome.into(), acceso.into());
    if let Some(cartella) = p.parent() {
        let _ = fs::create_dir_all(cartella);
    }
    let _ = fs::write(p, Value::Object(tutti).to_string());
}
