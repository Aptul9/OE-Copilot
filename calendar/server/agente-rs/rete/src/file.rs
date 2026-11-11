//! La scrittura atomica che i file di stato dell'agente fanno sempre allo stesso modo: si scrive `<file>.tmp` e lo si rinomina sul file.

use oe_nucleo::errori::Errore;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn con_suffisso(file: &Path, suffisso: &str) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(suffisso);
    PathBuf::from(s)
}

/// `writeFileSync(file + ".tmp", contenuto, { mode })` e `renameSync`. `modo` vale solo dove il file si crea, come in Node.
pub fn scrivi_atomico(file: &Path, contenuto: &str, modo: Option<u32>) -> Result<(), Errore> {
    let tmp = con_suffisso(file, ".tmp");
    let errore = |e: std::io::Error| Errore::Altro(e.to_string());
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(modo.unwrap_or(0o666));
    }
    #[cfg(not(unix))]
    let _ = modo;
    o.open(&tmp).and_then(|mut f| f.write_all(contenuto.as_bytes())).map_err(errore)?;
    std::fs::rename(&tmp, file).map_err(errore)
}

/// `readFileSync(file, "utf8")`: i byte non validi diventano U+FFFD.
pub fn leggi_utf8(file: &Path) -> std::io::Result<String> {
    Ok(match String::from_utf8(std::fs::read(file)?) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}
