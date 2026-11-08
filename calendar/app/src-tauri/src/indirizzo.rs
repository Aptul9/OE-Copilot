use std::path::Path;

use tauri::Url;

/// Il file con l'indirizzo scelto nell'app, nella cartella dei dati locali: vale al posto di quello della build (OE_URL).
/// Lo legge anche Promemoria.kt (sito()), con lo stesso nome
pub const FILE: &str = "indirizzo.txt";

/// Quello che si scrive nel campo, ridotto all'origine del sito: senza schema si parte da https://, http:// solo per localhost.
/// Percorso, parametri e ancora si tolgono (un indirizzo incollato dal browser ha quasi sempre /login); utente e password no
pub fn normalizza(testo: &str) -> Result<Url, String> {
    let testo = testo.trim();
    if testo.is_empty() {
        return Err("Scrivi l'indirizzo del server.".into());
    }
    let completo = if testo.contains("://") { testo.to_string() } else { format!("https://{testo}") };
    let url = Url::parse(&completo).map_err(|_| "Indirizzo non valido.".to_string())?;
    let locale = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => {}
        "http" if locale => {}
        "http" => return Err("Serve https:// (http:// solo per localhost).".into()),
        _ => return Err("Indirizzo non valido.".into()),
    }
    if url.host_str().is_none() {
        return Err("Indirizzo non valido.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("L'indirizzo non deve contenere utente o password.".into());
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|_| "Indirizzo non valido.".to_string())
}

/// L'indirizzo scelto, se c'è e vale ancora: un file che non si legge o non è un indirizzo ammesso è come se non ci fosse
pub fn leggi(cartella: &Path) -> Option<Url> {
    normalizza(&std::fs::read_to_string(cartella.join(FILE)).ok()?).ok()
}

/// Scrive l'indirizzo in un file accanto e lo rinomina: un'interruzione a metà non lascia un indirizzo troncato
pub fn salva(cartella: &Path, url: &Url) -> Result<(), String> {
    let errore = |e: std::io::Error| format!("Indirizzo non salvato: {e}");
    std::fs::create_dir_all(cartella).map_err(errore)?;
    let provvisorio = cartella.join(format!("{FILE}.nuovo"));
    std::fs::write(&provvisorio, url.origin().ascii_serialization()).map_err(errore)?;
    std::fs::rename(&provvisorio, cartella.join(FILE)).map_err(errore)
}

#[cfg(test)]
mod prove {
    use super::*;

    fn origine(testo: &str) -> String {
        normalizza(testo).unwrap().as_str().to_string()
    }

    #[test]
    fn senza_schema_si_parte_da_https() {
        assert_eq!(origine("oe.217-76-49-98.sslip.io"), "https://oe.217-76-49-98.sslip.io/");
        assert_eq!(origine("  oe.prova.test  "), "https://oe.prova.test/");
    }

    #[test]
    fn percorso_parametri_e_ancora_si_tolgono() {
        assert_eq!(origine("https://oe.prova.test/login?next=%2F#x"), "https://oe.prova.test/");
        assert_eq!(origine("oe.prova.test/telefono/"), "https://oe.prova.test/");
    }

    #[test]
    fn la_porta_resta_tranne_quella_dello_schema() {
        assert_eq!(origine("https://oe.prova.test:8443"), "https://oe.prova.test:8443/");
        assert_eq!(origine("https://oe.prova.test:443"), "https://oe.prova.test/");
    }

    #[test]
    fn http_solo_per_localhost() {
        assert_eq!(origine("http://localhost:3000"), "http://localhost:3000/");
        assert_eq!(origine("http://127.0.0.1:3000/x"), "http://127.0.0.1:3000/");
        assert_eq!(origine("http://[::1]:3000"), "http://[::1]:3000/");
        assert_eq!(normalizza("http://oe.prova.test").unwrap_err(), "Serve https:// (http:// solo per localhost).");
        assert_eq!(normalizza("http://localhost.example.org").unwrap_err(), "Serve https:// (http:// solo per localhost).");
    }

    #[test]
    fn il_resto_si_rifiuta() {
        for testo in ["", "   ", "https://", "ftp://oe.prova.test", "javascript:alert(1)", "file:///C:/Windows/win.ini", "data:text/html,x", "https://utente:segreto@oe.prova.test", "https://utente@oe.prova.test", "https://oe prova.test", "https://[::1"] {
            assert!(normalizza(testo).is_err(), "{testo:?}");
        }
    }

    #[test]
    fn salva_e_rileggi() {
        let cartella = std::env::temp_dir().join(format!("oe-indirizzo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cartella);
        assert_eq!(leggi(&cartella), None);
        salva(&cartella, &normalizza("oe.prova.test").unwrap()).unwrap();
        assert_eq!(leggi(&cartella).unwrap().as_str(), "https://oe.prova.test/");
        assert_eq!(std::fs::read_to_string(cartella.join(FILE)).unwrap(), "https://oe.prova.test");
        assert!(!cartella.join(format!("{FILE}.nuovo")).exists());
        salva(&cartella, &normalizza("https://altro.prova.test:8443").unwrap()).unwrap();
        assert_eq!(leggi(&cartella).unwrap().as_str(), "https://altro.prova.test:8443/");
        std::fs::remove_dir_all(&cartella).unwrap();
    }

    #[test]
    fn un_file_rovinato_e_come_se_non_ci_fosse() {
        let cartella = std::env::temp_dir().join(format!("oe-indirizzo-rovinato-{}", std::process::id()));
        std::fs::create_dir_all(&cartella).unwrap();
        for contenuto in ["", "\u{0}\u{1}", "ftp://oe.prova.test", "http://oe.prova.test"] {
            std::fs::write(cartella.join(FILE), contenuto).unwrap();
            assert_eq!(leggi(&cartella), None, "{contenuto:?}");
        }
        std::fs::remove_dir_all(&cartella).unwrap();
    }
}
