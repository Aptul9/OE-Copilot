//! I file della cartella dei dati (ui-dati/ sul server), era lib/core/dati.ts.

use oe_nucleo::json::Json;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Un file della cartella dei dati.
pub fn nelle_dati(parti: &[&str]) -> PathBuf {
    let mut p = PathBuf::from(crate::config::dati());
    for x in parti {
        p.push(x);
    }
    p
}

fn casuale_base36() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    let mut n = u64::from_le_bytes(b);
    let mut s = String::new();
    while n > 0 && s.len() < 11 {
        let c = (n % 36) as u8;
        s.push(if c < 10 { (b'0' + c) as char } else { (b'a' + c - 10) as char });
        n /= 36;
    }
    s
}

/// Salvataggio atomico (config.scrivi_json): prima un file temporaneo nella stessa cartella, poi la sostituzione. Un'interruzione a metà
/// lascia il file com'era, mai un JSON troncato. Come in Python: indentato di 2, senza ascii forzato, permessi 600.
pub fn scrivi_json(percorso: &Path, dati: &Json) -> io::Result<()> {
    scrivi_atomico(percorso, dati.in_js_rientrato(2).as_bytes(), 0o600, &format!(".tmp_{}_{}.json", std::process::id(), casuale_base36()))
}

/// Scrive `contenuto` in un file temporaneo `nome_tmp` della stessa cartella, poi lo sostituisce a `percorso`.
pub fn scrivi_atomico(percorso: &Path, contenuto: &[u8], modo: u32, nome_tmp: &str) -> io::Result<()> {
    let cartella = percorso.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(cartella)?;
    let tmp = cartella.join(nome_tmp);
    let scritto = (|| {
        let mut f = apri_con_modo(&tmp, modo)?;
        f.write_all(contenuto)?;
        f.sync_all()
    })();
    if let Err(e) = scritto {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, percorso) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(unix)]
fn apri_con_modo(p: &Path, modo: u32) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create(true).truncate(true).mode(modo).open(p)
}

#[cfg(not(unix))]
fn apri_con_modo(p: &Path, _modo: u32) -> io::Result<fs::File> {
    fs::OpenOptions::new().write(true).create(true).truncate(true).open(p)
}

/// `fs.readFileSync(percorso, "utf-8")`: i byte che non sono UTF-8 diventano U+FFFD, il BOM resta.
pub fn leggi_utf8(percorso: &Path) -> io::Result<String> {
    let b = fs::read(percorso)?;
    Ok(match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}

/// Il contenuto di un file di testo, o None se non c'è.
pub fn leggi_testo(percorso: &Path) -> io::Result<Option<String>> {
    match leggi_utf8(percorso) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
