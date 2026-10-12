use std::sync::atomic::Ordering;

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder, Wry, WindowEvent};

use crate::Stato;
use crate::navigazione::{self, Esito};

pub const PRINCIPALE: &str = "main";

/// La pagina di avvio inclusa nell'app, dove la finestra torna quando una navigazione fallisce
const AVVIO: &str = if cfg!(windows) { "http://tauri.localhost/index.html" } else { "tauri://localhost/index.html" };

type Costruttore<'a> = WebviewWindowBuilder<'a, Wry, AppHandle>;

/// Porta la finestra principale davanti, anche se era nascosta nell'area di notifica o ridotta a icona
pub fn mostra(app: &AppHandle) {
    if let Some(finestra) = app.get_webview_window(PRINCIPALE) {
        let _ = finestra.unminimize();
        let _ = finestra.show();
        let _ = finestra.set_focus();
    }
}

/// La navigazione e le finestre nuove (window.open) passano dalla lista di indirizzi ammessi
fn con_guardia<'a>(app: &AppHandle, costruttore: Costruttore<'a>) -> Costruttore<'a> {
    let origine = app.state::<Stato>().origine.clone();
    let (app_nav, origine_nav) = (app.clone(), origine.clone());
    let app_nuova = app.clone();
    costruttore
        .on_navigation(move |url| navigazione::consenti_navigazione(&app_nav, &origine_nav, url))
        .on_new_window(move |url, _| {
            match navigazione::classifica(&url, &origine) {
                // un indirizzo del sito (per esempio la pagina del telefono) si apre in una finestra dell'app
                Esito::Consenti if url.scheme() != "about" => apri_secondaria(&app_nuova, url),
                Esito::Consenti | Esito::Rifiuta => {}
                _ => {
                    navigazione::consenti_navigazione(&app_nuova, &origine, &url);
                }
            }
            NewWindowResponse::Deny
        })
}

/// Le finestre in più non hanno comandi né notifiche: sono solo il sito
fn apri_secondaria(app: &AppHandle, url: Url) {
    let app = app.clone();
    // dal gestore di window.open la finestra va creata da un altro thread
    std::thread::spawn(move || {
        let numero = app.state::<Stato>().secondarie.fetch_add(1, Ordering::Relaxed) + 1;
        let costruttore = WebviewWindowBuilder::new(&app, format!("secondaria-{numero}"), WebviewUrl::External(url))
            .title("OE Copilot")
            .inner_size(1100.0, 800.0);
        let _ = con_guardia(&app, costruttore).build();
    });
}

/// La finestra del sito: parte dalla pagina di avvio, che porta al sito quando risponde. La chiusura la nasconde
pub fn crea_principale(app: &AppHandle, nascosta: bool) -> tauri::Result<()> {
    let origine = app.state::<Stato>().origine.origin().ascii_serialization();
    let script = format!(
        "window.__OE_ORIGINE__ = {};\nwindow.__OE_AVVIO__ = {};\n{}",
        serde_json::to_string(&origine).unwrap_or_default(),
        serde_json::to_string(AVVIO).unwrap_or_default(),
        include_str!("inizializzazione.js")
    );
    let costruttore = WebviewWindowBuilder::new(app, PRINCIPALE, WebviewUrl::App("index.html".into()))
        .title("OE Copilot")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .visible(!nascosta)
        .initialization_script(script);
    let finestra = con_guardia(app, costruttore).build()?;
    let da_nascondere = finestra.clone();
    finestra.on_window_event(move |evento| {
        if let WindowEvent::CloseRequested { api, .. } = evento {
            api.prevent_close();
            let _ = da_nascondere.hide();
        }
    });
    Ok(())
}
