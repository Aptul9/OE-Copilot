//! L'ultima lettura riuscita di ogni casella, su disco (ultime-letture.json accanto a tokens.json): se Outlook o Google non rispondono, o
//! l'accesso è scaduto, la casella dà gli eventi com'erano allora, fino a ETA_MAX_MS. Pagina, meet e prenotazioni vedono ancora gli
//! impegni, con il pallino giallo che dice di quando sono.

use indexmap::IndexMap;
use oe_nucleo::errori::Errore;
use oe_nucleo::json::{Json, Mappa};
use oe_rete::file::{leggi_utf8, scrivi_atomico};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

/// Quanto a lungo si danno i dati di una casella che non si legge più: le riunioni sono quasi tutte ricorrenti, un mese regge.
pub const ETA_MAX_MS: i64 = 30 * 86_400_000;
/// Gli eventi finiti da più di tanti giorni si tolgono a ogni lettura riuscita.
pub const GIORNI_PASSATI: i64 = 35;
/// Una lettura che non cambia niente riscrive il file al più ogni dieci minuti (solo per l'ora dell'ultima lettura).
const SALVA_OGNI_MS: i64 = 600_000;

struct Casella {
    letto: i64,
    eventi: Vec<Json>,
}

pub struct UltimeLetture {
    file: Option<PathBuf>,
    caselle: RefCell<IndexMap<String, Casella>>,
    salvato: Cell<i64>,
}

fn testo<'a>(e: &'a Json, k: &str) -> &'a str {
    e.get(k).and_then(Json::as_str).unwrap_or_default()
}

fn fine_di(e: &Json) -> &str {
    let f = testo(e, "end");
    if f.is_empty() { testo(e, "start") } else { f }
}

/// contiene_evento sugli orari locali senza fuso, che si confrontano come testi: gli estremi finali esclusi, quelli iniziali compresi, un
/// evento senza durata conta nell'istante in cui cade. È il filtro della lettura: un evento che c'è per lei c'è anche qui.
pub fn nel_periodo(da: &str, a: &str, e: &Json) -> bool {
    let (c, d) = (testo(e, "start"), fine_di(e));
    if c == d {
        return if da == a { c == da } else { da <= c && c < a };
    }
    if da == a {
        return c <= da && da < d;
    }
    c < a && da < d
}

impl UltimeLetture {
    /// Senza file (le prove dell'agente) non si ricorda niente. Un file illeggibile vale come vuoto: la prossima lettura lo riscrive.
    pub fn nuove(file: Option<PathBuf>) -> Self {
        let mut caselle = IndexMap::new();
        let dati = file.as_ref().and_then(|f| leggi_utf8(f).ok()).and_then(|t| Json::analizza(&t).ok());
        if let Some(Json::Obj(c)) = dati.as_ref().and_then(|d| d.get("caselle")) {
            for (id, v) in c.iter() {
                let letto = v.get("letto").and_then(Json::as_f64).unwrap_or(0.0) as i64;
                let eventi = v.get("eventi").and_then(Json::as_arr).cloned().unwrap_or_default();
                caselle.insert(id.clone(), Casella { letto, eventi });
            }
        }
        UltimeLetture { file, caselle: RefCell::new(caselle), salvato: Cell::new(0) }
    }

    pub fn attive(&self) -> bool {
        self.file.is_some()
    }

    /// Una lettura riuscita di [da, a): i suoi eventi prendono il posto di quelli che c'erano nel periodo; via quelli finiti prima di
    /// `tieni_da`.
    pub fn ricorda(&self, id: &str, da: &str, a: &str, eventi: &[Json], adesso: i64, tieni_da: &str) -> Result<(), Errore> {
        if self.file.is_none() {
            return Ok(());
        }
        let cambiato = {
            let mut caselle = self.caselle.borrow_mut();
            let c = caselle.entry(id.to_string()).or_insert(Casella { letto: 0, eventi: Vec::new() });
            let mut nuovi: Vec<Json> = c.eventi.iter().filter(|e| !nel_periodo(da, a, e) && fine_di(e) >= tieni_da).cloned().collect();
            nuovi.extend(eventi.iter().cloned());
            nuovi.sort_by(|x, y| testo(x, "start").cmp(testo(y, "start")).then_with(|| testo(x, "id").cmp(testo(y, "id"))));
            let cambiato = nuovi != c.eventi;
            c.eventi = nuovi;
            c.letto = adesso;
            cambiato
        };
        if cambiato || adesso - self.salvato.get() >= SALVA_OGNI_MS {
            self.salva(adesso)?;
        }
        Ok(())
    }

    /// Gli eventi di [da, a) all'ultima lettura riuscita e l'ora di quella lettura; None se la casella non si è mai letta o è troppo vecchia.
    pub fn eventi(&self, id: &str, da: &str, a: &str, adesso: i64) -> Option<(Vec<Json>, i64)> {
        let caselle = self.caselle.borrow();
        let c = caselle.get(id).filter(|c| c.letto > 0 && adesso - c.letto <= ETA_MAX_MS)?;
        Some((c.eventi.iter().filter(|e| nel_periodo(da, a, e)).cloned().collect(), c.letto))
    }

    fn salva(&self, adesso: i64) -> Result<(), Errore> {
        let Some(file) = &self.file else { return Ok(()) };
        let mut caselle = Mappa::new();
        for (id, c) in self.caselle.borrow().iter() {
            let mut m = Mappa::new();
            m.insert("letto", c.letto);
            m.insert("eventi", Json::Arr(c.eventi.clone()));
            caselle.insert(id.as_str(), m);
        }
        let mut tutto = Mappa::new();
        tutto.insert("caselle", caselle);
        scrivi_atomico(file, &Json::Obj(tutto).in_js(), Some(0o600))?;
        self.salvato.set(adesso);
        Ok(())
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    fn ev(id: &str, inizio: &str, fine: &str) -> Json {
        let mut m = Mappa::new();
        m.insert("id", id);
        m.insert("start", inizio);
        m.insert("end", fine);
        Json::Obj(m)
    }

    fn ids(v: &[Json]) -> Vec<&str> {
        v.iter().map(|e| testo(e, "id")).collect()
    }

    #[test]
    fn il_periodo_come_contiene_evento() {
        let e = ev("a", "2026-11-13T10:00:00", "2026-11-13T11:00:00");
        assert!(nel_periodo("2026-11-13T00:00:00", "2026-11-14T00:00:00", &e));
        assert!(!nel_periodo("2026-11-13T11:00:00", "2026-11-14T00:00:00", &e));
        assert!(!nel_periodo("2026-11-12T00:00:00", "2026-11-13T10:00:00", &e));
        assert!(nel_periodo("2026-11-13T10:30:00", "2026-11-13T10:30:00", &e));
        let punto = ev("p", "2026-11-13T10:00:00", "2026-11-13T10:00:00");
        assert!(nel_periodo("2026-11-13T10:00:00", "2026-11-13T11:00:00", &punto));
        assert!(!nel_periodo("2026-11-13T09:00:00", "2026-11-13T10:00:00", &punto));
    }

    #[test]
    fn una_lettura_sostituisce_solo_il_suo_periodo_e_resta_su_disco() {
        let dir = std::env::temp_dir().join(format!("oe-ultime-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ultime-letture.json");
        let _ = std::fs::remove_file(&file);
        let u = UltimeLetture::nuove(Some(file.clone()));
        let lun = ev("lun", "2026-11-16T09:00:00", "2026-11-16T10:00:00");
        let mar = ev("mar", "2026-11-17T09:00:00", "2026-11-17T10:00:00");
        let vecchio = ev("vecchio", "2026-09-01T09:00:00", "2026-09-01T10:00:00");
        u.ricorda("x", "2026-11-16T00:00:00", "2026-11-23T00:00:00", &[lun.clone(), mar.clone()], 1000, "2026-08-01T00:00:00").unwrap();
        u.ricorda("x", "2026-09-01T00:00:00", "2026-09-02T00:00:00", &[vecchio], 1000, "2026-08-01T00:00:00").unwrap();
        // martedì annullato: la lettura della settimana non lo porta più
        u.ricorda("x", "2026-11-16T00:00:00", "2026-11-23T00:00:00", &[lun], 2000, "2026-08-01T00:00:00").unwrap();
        let (v, letto) = u.eventi("x", "2026-11-16T00:00:00", "2026-11-23T00:00:00", 3000).unwrap();
        assert_eq!((ids(&v), letto), (vec!["lun"], 2000));

        let riletto = UltimeLetture::nuove(Some(file.clone()));
        let (v, _) = riletto.eventi("x", "2026-01-01T00:00:00", "2027-01-01T00:00:00", 3000).unwrap();
        assert_eq!(ids(&v), vec!["vecchio", "lun"]);
        // dopo 35 giorni il passato se ne va, alla prossima lettura riuscita
        riletto.ricorda("x", "2026-11-16T00:00:00", "2026-11-23T00:00:00", &[], 4000, "2026-10-09T00:00:00").unwrap();
        assert_eq!(ids(&riletto.eventi("x", "2026-01-01T00:00:00", "2027-01-01T00:00:00", 4000).unwrap().0), Vec::<&str>::new());
        assert!(riletto.eventi("y", "2026-01-01T00:00:00", "2027-01-01T00:00:00", 4000).is_none());
        assert!(riletto.eventi("x", "2026-01-01T00:00:00", "2027-01-01T00:00:00", 4000 + ETA_MAX_MS + 1).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn senza_file_non_ricorda() {
        let u = UltimeLetture::nuove(None);
        u.ricorda("x", "2026-11-16T00:00:00", "2026-11-23T00:00:00", &[ev("a", "2026-11-16T09:00:00", "2026-11-16T10:00:00")], 1, "").unwrap();
        assert!(u.eventi("x", "2026-11-16T00:00:00", "2026-11-23T00:00:00", 2).is_none());
    }
}
