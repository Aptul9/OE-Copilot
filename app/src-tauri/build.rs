const COMANDI: &[&str] = &["notifiche_attive"];
const COMANDI_PROVA: &[&str] = &["notifiche_attive", "prova_menu"];

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_PROVA");
    let comandi = if std::env::var_os("CARGO_FEATURE_PROVA").is_some() { COMANDI_PROVA } else { COMANDI };
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(comandi))).unwrap();
}
