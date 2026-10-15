use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::finestra;

/// Le voci del menu dell'icona: servono per leggerne lo stato dopo un clic
pub struct Voci {
    #[cfg_attr(not(feature = "prova"), allow(dead_code))]
    menu: Menu<Wry>,
    avvio: CheckMenuItem<Wry>,
}

pub fn crea(app: &AppHandle) -> tauri::Result<()> {
    let avvio_acceso = app.autolaunch().is_enabled().unwrap_or(false);
    let apri = MenuItem::with_id(app, "apri", "Apri", true, None::<&str>)?;
    let avvio = CheckMenuItem::with_id(app, "avvio", "Avvia con Windows", true, avvio_acceso, None::<&str>)?;
    let esci = MenuItem::with_id(app, "esci", "Esci", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&apri, &avvio, &PredefinedMenuItem::separator(app)?, &esci])?;
    app.manage(Voci { menu: menu.clone(), avvio });

    let icona = app.default_window_icon().cloned().ok_or(tauri::Error::InvalidIcon(std::io::Error::other("icona dell'app mancante")))?;
    TrayIconBuilder::with_id("oe-copilot")
        .icon(icona)
        .tooltip("OE Copilot")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, evento| azione(app, evento.id().as_ref()))
        .on_tray_icon_event(|icona, evento| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = evento {
                finestra::mostra(icona.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Un clic su una voce: la spunta di "Avvia con Windows" si è già invertita, qui si applica il nuovo stato
fn azione(app: &AppHandle, id: &str) {
    match id {
        "apri" => finestra::mostra(app),
        "avvio" => imposta_avvio(app, app.state::<Voci>().avvio.is_checked().unwrap_or(false)),
        "esci" => app.exit(0),
        _ => {}
    }
}

/// L'avvio con Windows scrive la voce Run dell'utente; se non riesce la spunta torna com'era davvero
fn imposta_avvio(app: &AppHandle, acceso: bool) {
    let gestore = app.autolaunch();
    let _ = if acceso { gestore.enable() } else { gestore.disable() };
    let _ = app.state::<Voci>().avvio.set_checked(gestore.is_enabled().unwrap_or(false));
}

/// Solo per le prove: un clic su una voce, con lo stesso percorso di un clic vero
#[cfg(feature = "prova")]
pub fn simula(app: &AppHandle, id: &str) {
    if id == "avvio" {
        let voce = &app.state::<Voci>().avvio;
        let _ = voce.set_checked(!voce.is_checked().unwrap_or(false));
    }
    azione(app, id);
}

/// Solo per le prove: le voci del menu con il testo e la spunta, più lo stato della finestra principale
#[cfg(feature = "prova")]
pub fn riepilogo(app: &AppHandle) -> serde_json::Value {
    use tauri::menu::MenuItemKind;

    let voci = app.state::<Voci>();
    let elenco: Vec<_> = voci
        .menu
        .items()
        .unwrap_or_default()
        .iter()
        .map(|voce| match voce {
            MenuItemKind::MenuItem(v) => serde_json::json!({ "id": v.id().as_ref() as &str, "testo": v.text().unwrap_or_default() }),
            MenuItemKind::Check(v) => serde_json::json!({
                "id": v.id().as_ref() as &str,
                "testo": v.text().unwrap_or_default(),
                "spunta": v.is_checked().unwrap_or(false),
            }),
            _ => serde_json::json!({ "id": voce.id().as_ref() as &str, "separatore": true }),
        })
        .collect();
    let finestra = app.get_webview_window(finestra::PRINCIPALE);
    serde_json::json!({
        "voci": elenco,
        "avvio": app.autolaunch().is_enabled().unwrap_or(false),
        "finestra": {
            "visibile": finestra.as_ref().and_then(|f| f.is_visible().ok()),
            "ridotta": finestra.as_ref().and_then(|f| f.is_minimized().ok()),
            "fuoco": finestra.as_ref().and_then(|f| f.is_focused().ok()),
        },
    })
}
