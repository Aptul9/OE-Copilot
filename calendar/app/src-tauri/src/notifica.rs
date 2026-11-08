use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::{BASE64_STANDARD, Engine};
use serde_json::Value;
use tauri::{AppHandle, Url};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::{HSTRING, IInspectable, Interface, h};

use crate::{comparsa, finestra};

// La notifica di Windows di un promemoria. La fa l'app e non il plugin notification, che su Windows passa solo titolo, testo e suono:
// qui ha il tag della riunione (l'avviso dei 10 minuti prende il posto di quello dei 30 nel centro notifiche), scade all'inizio della
// riunione (poi esce da sola dal centro notifiche) e ha il logo dell'azienda al posto dell'icona dell'app.

/// Dalla pagina: la riunione (chiave), title e body come per il plugin, sound (se manca è muta: il suono lo fa l'avviso a comparsa),
/// il logo come data: e l'inizio della riunione in ms dal 1970
pub fn mostra(app: &AppHandle, dati: &Value) -> windows::core::Result<()> {
    let testo = |campo: &str| dati.get(campo).and_then(Value::as_str).unwrap_or_default();
    let logo = dati.get("logo").and_then(Value::as_str).and_then(|l| file_logo(&app.config().identifier, l));
    let righe: Vec<&str> = testo("body").lines().collect();
    let documento = XmlDocument::new()?;
    documento.LoadXml(&HSTRING::from(xml(testo("title"), &righe, logo.as_deref(), dati.get("sound").and_then(Value::as_str))))?;
    let notifica = ToastNotification::CreateToastNotification(&documento)?;
    notifica.SetTag(&HSTRING::from(tag(testo("chiave"))))?;
    notifica.SetGroup(h!("promemoria"))?;
    if let Some(inizio) = dati.get("inizio").and_then(Value::as_f64) {
        let scadenza = PropertyValue::CreateDateTime(DateTime { UniversalTime: ora_windows(inizio) })?.cast::<IReference<DateTime>>()?;
        notifica.SetExpirationTime(&scadenza)?;
    }
    // un clic sulla notifica porta la finestra davanti
    let app_clic = app.clone();
    notifica.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
        finestra::mostra(&app_clic);
        Ok(())
    }))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app.config().identifier.as_str()))?.Show(&notifica)
}

/// Il toast: il logo se c'è, il titolo e al più due righe, il suono o il silenzio
fn xml(titolo: &str, righe: &[&str], logo: Option<&Path>, suono: Option<&str>) -> String {
    let logo = logo.and_then(|p| Url::from_file_path(p).ok()).map(|u| format!(r#"<image placement="appLogoOverride" src="{}"/>"#, scappa(u.as_str())));
    let testi: String = std::iter::once(titolo).chain(righe.iter().copied().take(2)).map(|t| format!("<text>{}</text>", scappa(t))).collect();
    let audio = match suono {
        Some(nome) => format!(r#"<audio src="ms-winsoundevent:{}"/>"#, comparsa::evento_suono(nome)),
        None => r#"<audio silent="true"/>"#.into(),
    };
    format!(r#"<toast><visual><binding template="ToastGeneric">{}{testi}</binding></visual>{audio}</toast>"#, logo.unwrap_or_default())
}

fn scappa(testo: &str) -> String {
    testo.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Lo stesso per tutti gli avvisi di una riunione, e corto: Windows accetta tag di al più 64 caratteri
fn tag(chiave: &str) -> String {
    let mut h = DefaultHasher::new();
    chiave.hash(&mut h);
    format!("{:x}", h.finish())
}

/// Da ms dal 1970 al DateTime di Windows (centinaia di ns dal 1601), mai prima di un minuto da adesso: una notifica già scaduta non si vede
fn ora_windows(ms: f64) -> i64 {
    let adesso = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
    (ms.max(adesso + 60_000.0) as i64 + 11_644_473_600_000) * 10_000
}

/// Estensione e base64 di un logo data: che il toast sa mostrare (png, jpeg, gif) e non troppo grande, come nell'avviso a comparsa
fn tipo_logo(logo: &str) -> Option<(&'static str, &str)> {
    let (tipo, dati) = logo.strip_prefix("data:image/")?.split_once(";base64,")?;
    let estensione = match tipo {
        "png" => "png",
        "jpeg" => "jpg",
        "gif" => "gif",
        _ => return None,
    };
    (dati.len() <= 400_000).then_some((estensione, dati))
}

/// Il logo come file in %LOCALAPPDATA%\<identificativo>\loghi, uno per contenuto: un'app senza pacchetto mostra nel toast solo
/// immagini locali, non indirizzi né data:
fn file_logo(identificativo: &str, logo: &str) -> Option<PathBuf> {
    let (estensione, dati) = tipo_logo(logo)?;
    let mut h = DefaultHasher::new();
    dati.hash(&mut h);
    let cartella = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(identificativo).join("loghi");
    let file = cartella.join(format!("{:x}.{estensione}", h.finish()));
    if !file.exists() {
        let contenuto = BASE64_STANDARD.decode(dati).ok()?;
        std::fs::create_dir_all(&cartella).ok()?;
        std::fs::write(&file, contenuto).ok()?;
    }
    Some(file)
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn il_toast_ha_testi_ripuliti_logo_e_suono_o_silenzio() {
        let logo = Path::new(r"C:\Dati\loghi\a1.png");
        let pieno = xml("Tra 10 min · <Riunione> & \"altro\"", &["Alle 10:00", "Sala 1", "terza riga"], Some(logo), Some("Reminder"));
        assert_eq!(
            pieno,
            r#"<toast><visual><binding template="ToastGeneric"><image placement="appLogoOverride" src="file:///C:/Dati/loghi/a1.png"/><text>Tra 10 min · &lt;Riunione&gt; &amp; &quot;altro&quot;</text><text>Alle 10:00</text><text>Sala 1</text></binding></visual><audio src="ms-winsoundevent:Notification.Reminder"/></toast>"#
        );
        let muto = xml("Tra 5 min", &["Alle 10:00"], None, None);
        assert_eq!(muto, r#"<toast><visual><binding template="ToastGeneric"><text>Tra 5 min</text><text>Alle 10:00</text></binding></visual><audio silent="true"/></toast>"#);
        assert!(xml("x", &[], None, Some("Alarm3")).contains("ms-winsoundevent:Notification.Looping.Alarm3"));
        assert!(xml("x", &[], None, Some("../x")).contains("ms-winsoundevent:Notification.Default"));
    }

    #[test]
    fn un_tag_per_riunione_e_scadenza_all_inizio() {
        assert_eq!(tag("e1|2026-10-16T10:00:00"), tag("e1|2026-10-16T10:00:00"));
        assert_ne!(tag("e1|2026-10-16T10:00:00"), tag("e2|2026-10-16T10:00:00"));
        assert!(tag(&"x".repeat(500)).len() <= 16);
        // 2100-01-01 in ms dal 1970, più i ms fra il 1601 e il 1970, in centinaia di ns
        assert_eq!(ora_windows(4_102_444_800_000.0), (4_102_444_800_000 + 11_644_473_600_000) * 10_000);
        assert!(ora_windows(0.0) > (1_700_000_000_000 + 11_644_473_600_000) * 10_000);
    }

    #[test]
    fn solo_loghi_png_jpeg_gif_non_troppo_grandi() {
        assert_eq!(tipo_logo("data:image/png;base64,AAAA"), Some(("png", "AAAA")));
        assert_eq!(tipo_logo("data:image/jpeg;base64,BBBB"), Some(("jpg", "BBBB")));
        for altro in ["data:image/svg+xml;base64,PHN2Zz4=", "data:image/x-icon;base64,AAAA", "https://example.org/x.png", "data:text/html;base64,PGI+"] {
            assert_eq!(tipo_logo(altro), None, "{altro}");
        }
        assert_eq!(tipo_logo(&format!("data:image/png;base64,{}", "A".repeat(400_004))), None);
    }
}
