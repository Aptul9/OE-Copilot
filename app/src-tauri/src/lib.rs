mod finestra;
mod impostazioni;
mod navigazione;
#[cfg(desktop)]
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(desktop)]
use std::sync::atomic::AtomicU32;

use tauri::{Manager, Url};
#[cfg(desktop)]
use tauri_plugin_autostart::MacosLauncher;
#[cfg(desktop)]
use tauri_plugin_notification::NotificationExt;
#[cfg(desktop)]
use tauri_plugin_window_state::StateFlags;

/// Quello che l'app sa di sé: l'indirizzo del sito e l'interruttore delle notifiche
pub struct Stato {
    pub origine: Url,
    pub notifiche: AtomicBool,
    #[cfg(desktop)]
    pub secondarie: AtomicU32,
}

/// L'indirizzo del sito viene dalla configurazione di build (scripts/configura.mjs, variabile OE_URL)
fn origine_da_config(config: &tauri::Config) -> Result<Url, String> {
    let testo = config
        .plugins
        .0
        .get("oe")
        .and_then(|c| c.get("url"))
        .and_then(|u| u.as_str())
        .ok_or("manca plugins.oe.url nella configurazione")?;
    let url = Url::parse(testo).map_err(|e| format!("indirizzo del sito non valido: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("indirizzo del sito non ammesso: {testo}"));
    }
    Ok(url)
}

/// La pagina chiede se le notifiche sono accese (interruttore del menu dell'icona, su Android sempre accese)
#[tauri::command]
fn notifiche_attive(stato: tauri::State<'_, Stato>) -> bool {
    stato.notifiche.load(Ordering::Relaxed)
}

/// Solo nelle build di prova: elenco delle voci del menu dell'icona, con l'esito di un clic se richiesto
#[cfg(all(feature = "prova", desktop))]
#[tauri::command]
fn prova_menu(app: tauri::AppHandle, azione: Option<String>) -> serde_json::Value {
    if let Some(id) = azione {
        tray::simula(&app, &id);
    }
    tray::riepilogo(&app)
}

/// Un'app senza console non lascia traccia se non parte (per esempio WebView2 che non si avvia): gli errori all'avvio
/// finiscono in %LOCALAPPDATA%\<identificativo dell'app>\errore.log (com.oecopilot.desktop), con l'ora in secondi dal 1970
#[cfg(desktop)]
fn registra_errore(identificativo: &str, testo: &str) {
    use std::io::Write;

    let Some(cartella) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join(identificativo)) else {
        return;
    };
    let _ = std::fs::create_dir_all(&cartella);
    let secondi = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(cartella.join("errore.log")) {
        let _ = writeln!(file, "[{secondi}] {testo}");
    }
}

/// WebView2 rifiuta la creazione con "risorsa in uso" (0x800700AA) se l'istanza precedente sullo stesso profilo non ha
/// ancora finito di chiudersi, per esempio dopo un riavvio subito dopo una chiusura forzata: si riprova per mezzo minuto
#[cfg(desktop)]
fn crea_finestra_con_tentativi(app: &tauri::AppHandle, nascosta: bool) -> tauri::Result<()> {
    const TENTATIVI: u32 = 10;
    let mut tentativo = 1;
    loop {
        match finestra::crea_principale(app, nascosta) {
            Err(errore) if tentativo < TENTATIVI => {
                registra_errore(&app.config().identifier, &format!("finestra non creata (tentativo {tentativo} di {TENTATIVI}): {errore}"));
                tentativo += 1;
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            esito => return esito,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let contesto = tauri::generate_context!();
    #[cfg(desktop)]
    {
        let identificativo = contesto.config().identifier.clone();
        std::panic::set_hook(Box::new(move |info| registra_errore(&identificativo, &info.to_string().replace('\n', " "))));
    }

    let costruttore = tauri::Builder::default();
    #[cfg(desktop)]
    let costruttore = costruttore
        .plugin(tauri_plugin_single_instance::init(|app, _argomenti, _cartella| finestra::mostra(app)))
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .with_filter(|etichetta| etichetta == finestra::PRINCIPALE)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--nascosto"])));
    let costruttore = costruttore
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build());

    #[cfg(all(feature = "prova", desktop))]
    let costruttore = costruttore.invoke_handler(tauri::generate_handler![notifiche_attive, prova_menu]);
    #[cfg(not(all(feature = "prova", desktop)))]
    let costruttore = costruttore.invoke_handler(tauri::generate_handler![notifiche_attive]);

    costruttore
        .setup(|app| {
            let origine = origine_da_config(app.config())?;
            app.manage(Stato {
                origine,
                notifiche: AtomicBool::new(impostazioni::notifiche_accese(app.handle())),
                #[cfg(desktop)]
                secondarie: AtomicU32::new(0),
            });
            #[cfg(desktop)]
            {
                let nascosta = std::env::args().any(|a| a == "--nascosto");
                crea_finestra_con_tentativi(app.handle(), nascosta)?;
                tray::crea(app.handle())?;
                // un clic sulla notifica porta la finestra davanti
                let app_clic = app.handle().clone();
                app.notification().on_action(move |_| finestra::mostra(&app_clic))?;
            }
            #[cfg(mobile)]
            finestra::crea_principale(app.handle(), false)?;
            Ok(())
        })
        .run(contesto)
        .expect("avvio di OE Copilot non riuscito");
}
