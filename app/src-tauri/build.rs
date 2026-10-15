const COMANDI: &[&str] = &["interruttori", "mostra_avviso"];
const COMANDI_PROVA: &[&str] = &["interruttori", "mostra_avviso", "prova_menu"];

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_PROVA");
    let mobile = matches!(std::env::var("CARGO_CFG_TARGET_OS").as_deref(), Ok("android" | "ios"));
    let comandi = if std::env::var_os("CARGO_FEATURE_PROVA").is_some() && !mobile { COMANDI_PROVA } else { COMANDI };
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(comandi))).unwrap();
}
