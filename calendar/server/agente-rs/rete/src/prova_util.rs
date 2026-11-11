//! Aiuti per i test, anche quelli del crate `agente`: una cartella temporanea che si toglie da sola.

use std::path::{Path, PathBuf};

pub struct CartellaTemp(PathBuf);

impl CartellaTemp {
    pub fn nuova() -> Self {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("random");
        let nome: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let p = std::env::temp_dir().join(format!("oe-rete-{nome}"));
        std::fs::create_dir_all(&p).expect("cartella temporanea");
        CartellaTemp(p)
    }

    pub fn percorso(&self) -> &Path {
        &self.0
    }
}

impl Drop for CartellaTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
