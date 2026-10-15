use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::{Stato, finestra, impostazioni};

/// Le voci del menu dell'icona: servono per leggerne lo stato dopo un clic
pub struct Voci {
    #[cfg_attr(not(feature = "prova"), allow(dead_code))]
    menu: Menu<Wry>,
    notifiche: CheckMenuItem<Wry>,
    comparsa: CheckMenuItem<Wry>,
    anonimo: CheckMenuItem<Wry>,
    avvio: CheckMenuItem<Wry>,
}

pub fn crea(app: &AppHandle) -> tauri::Result<()> {
    let stato = app.state::<Stato>();
    let avvio_acceso = app.autolaunch().is_enabled().unwrap_or(false);
    let apri = MenuItem::with_id(app, "apri", "Apri", true, None::<&str>)?;
    let notifiche = CheckMenuItem::with_id(app, "notifiche", "Notifiche", true, stato.notifiche.load(Ordering::Relaxed), None::<&str>)?;
    let comparsa = CheckMenuItem::with_id(app, "comparsa", "Avviso a comparsa", true, stato.comparsa.load(Ordering::Relaxed), None::<&str>)?;
    let anonimo = CheckMenuItem::with_id(app, "anonimo", "Avviso anonimo", true, stato.anonimo.load(Ordering::Relaxed), None::<&str>)?;
    let avvio = CheckMenuItem::with_id(app, "avvio", "Avvia con Windows", true, avvio_acceso, None::<&str>)?;
    let esci = MenuItem::with_id(app, "esci", "Esci", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&apri, &notifiche, &comparsa, &anonimo, &avvio, &PredefinedMenuItem::separator(app)?, &esci])?;
    app.manage(Voci { menu: menu.clone(), notifiche, comparsa, anonimo, avvio });

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

/// Un clic su una voce: quelle con la spunta si sono già invertite, qui si applica il nuovo stato
fn azione(app: &AppHandle, id: &str) {
    let voci = app.state::<Voci>();
    let stato = app.state::<Stato>();
    match id {
        "apri" => finestra::mostra(app),
        "notifiche" => imposta(app, id, &voci.notifiche, &stato.notifiche),
        "comparsa" => imposta(app, id, &voci.comparsa, &stato.comparsa),
        "anonimo" => imposta(app, id, &voci.anonimo, &stato.anonimo),
        "avvio" => imposta_avvio(app, voci.avvio.is_checked().unwrap_or(false)),
        "esci" => app.exit(0),
        _ => {}
    }
}

/// Un interruttore: vale dal prossimo avviso e resta salvato in impostazioni.json
fn imposta(app: &AppHandle, nome: &str, voce: &CheckMenuItem<Wry>, valore: &AtomicBool) {
    let acceso = voce.is_checked().unwrap_or_else(|_| valore.load(Ordering::Relaxed));
    valore.store(acceso, Ordering::Relaxed);
    impostazioni::salva(app, nome, acceso);
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
    let voci = app.state::<Voci>();
    let voce = match id {
        "notifiche" => Some(&voci.notifiche),
        "comparsa" => Some(&voci.comparsa),
        "anonimo" => Some(&voci.anonimo),
        "avvio" => Some(&voci.avvio),
        _ => None,
    };
    if let Some(v) = voce {
        let _ = v.set_checked(!v.is_checked().unwrap_or(false));
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
        "notifiche": app.state::<Stato>().notifiche.load(Ordering::Relaxed),
        "comparsa": app.state::<Stato>().comparsa.load(Ordering::Relaxed),
        "anonimo": app.state::<Stato>().anonimo.load(Ordering::Relaxed),
        "avvio": app.autolaunch().is_enabled().unwrap_or(false),
        "finestra": {
            "visibile": finestra.as_ref().and_then(|f| f.is_visible().ok()),
            "ridotta": finestra.as_ref().and_then(|f| f.is_minimized().ok()),
            "fuoco": finestra.as_ref().and_then(|f| f.is_focused().ok()),
        },
    })
}
