//! Lo stato di chi usa l'app: la configurazione (config.json) e gli eventi personali (local_events.json) stanno in gateway.db, nello stesso
//! database delle sessioni (era lib/core/archivio.ts). Una volta sola, alla prima apertura, si importano i due file di prima; da lì in poi il
//! database è la fonte e ogni modifica è una transazione. I due file si riscrivono dopo ogni modifica, nel formato di sempre: servono a tornare
//! a una versione che li leggeva senza perdere niente, e a correggere lo stato a mano. Non si rileggono mai, salvo nell'importazione.

use crate::dati::{nelle_dati, scrivi_json};
use crate::orologio::ora_stamp;
use crate::py::{py_str, Errore, Esito};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use rusqlite::{params, Connection, OptionalExtension};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

pub type Config = Mappa;

/// I valori di partenza della configurazione.
pub fn default_config() -> Mappa {
    match ogg! {
        // Calendari da NON mostrare: elenco di id
        "disabled_calendars": Json::Arr(vec![]),
        // Calendari sottoscritti da link iCal: [{id, name, url}]
        "ics_calendars": Json::Arr(vec![]),
        // Come chiamare gli account: {"mario@azienda.it": "Azienda"}
        "account_aliases": Json::obj(),
        // Ogni quanti secondi la pagina rilegge i calendari
        "reminder_poll_seconds": 60,
        // Colore per account: {"mario@x.com": "#2563eb"}, riempito da solo
        "account_colors": Json::obj(),
        // Aspetto: "auto", "light" o "dark"
        "theme": "auto",
        // Zoom del calendario: altezza in pixel della riga da mezz'ora
        "zoom_calendario": 58,
    } {
        Json::Obj(m) => m,
        _ => unreachable!(),
    }
}

pub fn percorso_config() -> PathBuf {
    nelle_dati(&["config.json"])
}

pub fn percorso_locali() -> PathBuf {
    nelle_dati(&["local_events.json"])
}

const SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS stato_config (chiave TEXT PRIMARY KEY, valore TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS eventi_locali (n INTEGER PRIMARY KEY AUTOINCREMENT, dati TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS stato_meta (chiave TEXT PRIMARY KEY, valore TEXT NOT NULL);
";

thread_local! {
    static PRONTI: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static PROFONDITA: Cell<u32> = const { Cell::new(0) };
    static SPORCO_CONFIG: Cell<bool> = const { Cell::new(false) };
    static SPORCO_LOCALI: Cell<bool> = const { Cell::new(false) };
}

/// Il database dello stato, aperto e pronto: lo schema c'è e i file di prima sono stati importati.
pub fn archivio() -> Esito<Rc<Connection>> {
    let file = crate::config::db_stato();
    let db = crate::db::apri(&file)?;
    if PRONTI.with(|p| p.borrow().contains(&file)) {
        return Ok(db);
    }
    db.execute_batch(SCHEMA)?;
    importa_una_volta(&db)?;
    PRONTI.with(|p| p.borrow_mut().insert(file));
    Ok(db)
}

/// Dimentica che l'archivio è pronto (per le prove, con chiudi_tutto di db.rs).
pub fn chiudi_archivio() {
    PRONTI.with(|p| p.borrow_mut().clear());
    crate::db::chiudi_tutto();
}

/// Una copia del file illeggibile, PRIMA di ripartire da zero: se no la prima copia su file lo sovrascriverebbe e il contenuto andrebbe perso.
fn copia_del_guasto(percorso: &PathBuf) {
    let guasto = PathBuf::from(format!("{}.rotto-{}", percorso.display(), ora_stamp()));
    match std::fs::copy(percorso, &guasto) {
        Ok(_) => eprintln!("archivio: {} illeggibile, ne salvo una copia in {} e riparto da zero", percorso.display(), guasto.display()),
        Err(_) => eprintln!("archivio: copia di {} non riuscita", percorso.display()),
    }
}

fn config_di_prima() -> Esito<Mappa> {
    let percorso = percorso_config();
    if !percorso.exists() {
        return Ok(Mappa::new());
    }
    let testo = match crate::dati::leggi_utf8(&percorso) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("configurazione non leggibile {e}");
            return Ok(Mappa::new());
        }
    };
    match Json::analizza(&testo) {
        Ok(Json::Obj(m)) => Ok(m),
        Ok(_) => Err(Errore::tipo("cannot convert dictionary update sequence element #0 to a sequence")),
        Err(_) => {
            copia_del_guasto(&percorso);
            Ok(Mappa::new())
        }
    }
}

fn locali_di_prima() -> Vec<Json> {
    let percorso = percorso_locali();
    let testo = match crate::dati::leggi_utf8(&percorso) {
        Ok(t) => t,
        Err(e) => {
            // La prima volta: nessun evento personale, è normale. Il resto no: sono dati che esistono solo qui.
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("eventi personali: file non leggibile ({}) {e}", percorso.display());
            }
            return Vec::new();
        }
    };
    match Json::analizza(&testo) {
        Ok(Json::Arr(v)) => v,
        Ok(_) => Vec::new(),
        Err(e) => {
            eprintln!("eventi personali: file non leggibile ({}) SyntaxError: {e}", percorso.display());
            copia_del_guasto(&percorso);
            Vec::new()
        }
    }
}

fn importa_una_volta(db: &Connection) -> Esito<()> {
    if db.query_row("SELECT 1 FROM stato_meta WHERE chiave = 'importato'", [], |_| Ok(())).optional()?.is_some() {
        return Ok(());
    }
    let cfg = config_di_prima()?;
    let locali = locali_di_prima();
    db.execute_batch("BEGIN IMMEDIATE")?;
    let esito = (|| -> Esito<()> {
        scrivi_righe_config(db, &cfg)?;
        for e in &locali {
            db.execute("INSERT INTO eventi_locali(dati) VALUES (?)", [e.in_js()])?;
        }
        db.execute("INSERT INTO stato_meta(chiave, valore) VALUES ('importato', ?)", [ora_stamp()])?;
        Ok(())
    })();
    match esito {
        Ok(()) => db.execute_batch("COMMIT")?,
        Err(e) => {
            let _ = db.execute_batch("ROLLBACK");
            return Err(e);
        }
    }
    println!("archivio: importati da config.json {} voci e da local_events.json {} eventi", cfg.len(), locali.len());
    Ok(())
}

/// Riscrive config.json e local_events.json se la transazione appena finita li ha cambiati: stesso contenuto e formato di quando li scriveva Python.
fn copia_su_file() {
    if SPORCO_CONFIG.with(|s| s.replace(false)) {
        let scritto = load_config().map_err(|e| e.to_string()).and_then(|c| scrivi_json(&percorso_config(), &Json::Obj(c)).map_err(|e| e.to_string()));
        if let Err(e) = scritto {
            eprintln!("archivio: config.json non riscritto {e}");
        }
    }
    if SPORCO_LOCALI.with(|s| s.replace(false)) {
        let scritto = leggi_righe_locali().map_err(|e| e.to_string()).and_then(|v| scrivi_json(&percorso_locali(), &Json::Arr(v)).map_err(|e| e.to_string()));
        if let Err(e) = scritto {
            eprintln!("archivio: local_events.json non riscritto {e}");
        }
    }
}

/// Una transazione sull'archivio (BEGIN IMMEDIATE): quello che `f` legge e scrive è un blocco solo, e se restituisce un errore non cambia
/// niente. Se ne possono annidare (un punto di salvataggio); a fine della più esterna si riscrivono i file di copia.
pub fn transazione<T>(f: impl FnOnce() -> Esito<T>) -> Esito<T> {
    let db = archivio()?;
    let livello = PROFONDITA.with(|p| {
        let n = p.get();
        p.set(n + 1);
        n
    });
    let esito = (|| {
        if livello == 0 {
            db.execute_batch("BEGIN IMMEDIATE")?;
        } else {
            db.execute_batch(&format!("SAVEPOINT oe{livello}"))?;
        }
        match f() {
            Ok(v) => {
                if livello == 0 {
                    db.execute_batch("COMMIT")?;
                } else {
                    db.execute_batch(&format!("RELEASE oe{livello}"))?;
                }
                Ok(v)
            }
            Err(e) => {
                if livello == 0 {
                    let _ = db.execute_batch("ROLLBACK");
                } else {
                    let _ = db.execute_batch(&format!("ROLLBACK TO oe{livello}; RELEASE oe{livello}"));
                }
                Err(e)
            }
        }
    })();
    PROFONDITA.with(|p| p.set(livello));
    if esito.is_err() && livello == 0 {
        SPORCO_CONFIG.with(|s| s.set(false));
        SPORCO_LOCALI.with(|s| s.set(false));
    }
    if esito.is_ok() && livello == 0 {
        copia_su_file();
    }
    esito
}

/// Se una transazione è aperta adesso (db.inTransaction).
pub fn in_transazione() -> bool {
    PROFONDITA.with(|p| p.get() > 0)
}

fn analizza(testo: &str) -> Esito<Json> {
    Json::analizza(testo).map_err(|e| Errore::Altro { nome: "SyntaxError", messaggio: e.messaggio })
}

fn leggi_righe_config() -> Esito<Mappa> {
    let db = archivio()?;
    let mut st = db.prepare_cached("SELECT chiave, valore FROM stato_config ORDER BY rowid")?;
    let righe = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Mappa::new();
    for r in righe {
        let (k, v) = r?;
        out.insert(k, analizza(&v)?);
    }
    Ok(out)
}

/// Rende le righe uguali a `cfg`: quelle che ci sono si aggiornano, quelle che non ci sono più si tolgono.
fn scrivi_righe_config(db: &Connection, cfg: &Mappa) -> Esito<()> {
    let mut restano = HashSet::new();
    {
        let mut scrivi = db.prepare_cached("INSERT INTO stato_config(chiave, valore) VALUES (?, ?) ON CONFLICT(chiave) DO UPDATE SET valore = excluded.valore")?;
        for (k, v) in cfg.iter() {
            if k == "_sola_lettura" {
                continue;
            }
            scrivi.execute(params![k, v.in_js()])?;
            restano.insert(k.clone());
        }
    }
    let presenti: Vec<String> = db.prepare_cached("SELECT chiave FROM stato_config")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for k in presenti {
        if !restano.contains(&k) {
            db.execute("DELETE FROM stato_config WHERE chiave = ?", [k])?;
        }
    }
    Ok(())
}

/// La configurazione: i valori predefiniti con sopra quelli salvati.
pub fn load_config() -> Esito<Config> {
    let mut cfg = default_config();
    for (k, v) in leggi_righe_config()?.iter() {
        cfg.insert(k.clone(), v.clone());
    }
    // Garantisce che i sotto-dizionari esistano sempre
    for (k, v) in [("account_colors", Json::obj()), ("disabled_calendars", Json::Arr(vec![])), ("ics_calendars", Json::Arr(vec![])), ("account_aliases", Json::obj())] {
        if !cfg.contains_key(k) {
            cfg.insert(k, v);
        }
    }
    Ok(cfg)
}

/// Salva la configurazione com'è; una copia di lavoro (_sola_lettura) resta in memoria.
pub fn save_config(cfg: &Config) -> Esito<()> {
    if crate::py::py_bool(cfg.get("_sola_lettura")) {
        return Ok(());
    }
    transazione(|| {
        scrivi_righe_config(&*archivio()?, cfg)?;
        SPORCO_CONFIG.with(|s| s.set(true));
        Ok(())
    })
}

/// Rilegge la configurazione, la cambia con `modifica` e la salva, tutto in un blocco: due richieste che arrivano insieme non si perdono le modifiche.
pub fn modifica_config<T>(modifica: impl FnOnce(&mut Config) -> Esito<T>) -> Esito<T> {
    transazione(|| {
        let mut cfg = load_config()?;
        let esito = modifica(&mut cfg)?;
        scrivi_righe_config(&*archivio()?, &cfg)?;
        SPORCO_CONFIG.with(|s| s.set(true));
        Ok(esito)
    })
}

/// Gli eventi personali, nell'ordine in cui sono stati aggiunti.
pub fn leggi_righe_locali() -> Esito<Vec<Json>> {
    let db = archivio()?;
    let mut st = db.prepare_cached("SELECT dati FROM eventi_locali ORDER BY n")?;
    let righe: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    righe.iter().map(|t| analizza(t)).collect()
}

/// Aggiunge in fondo l'evento che `crea` costruisce vedendo quelli che ci sono (serve per l'identificativo).
pub fn aggiungi_locale(crea: impl FnOnce(&[Json]) -> Json) -> Esito<()> {
    transazione(|| {
        let evento = crea(&leggi_righe_locali()?);
        archivio()?.execute("INSERT INTO eventi_locali(dati) VALUES (?)", [evento.in_js()])?;
        SPORCO_LOCALI.with(|s| s.set(true));
        Ok(())
    })
}

/// Toglie gli eventi con quell'identificativo (str(uid) come in Python). Il file di copia si riscrive anche se non ne manca nessuno.
pub fn togli_locali(uid: Option<&Json>) -> Esito<()> {
    transazione(|| {
        let db = archivio()?;
        let righe: Vec<(i64, String)> = db.prepare_cached("SELECT n, dati FROM eventi_locali")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        let cercato = py_str(uid);
        for (n, dati) in righe {
            let e = analizza(&dati)?;
            // (JSON.parse(r.dati) as Dict | null)?.uid: un valore che non è un oggetto non ha uid
            let suo = e.as_obj().and_then(|m| m.get("uid"));
            if py_str(suo) == cercato {
                db.execute("DELETE FROM eventi_locali WHERE n = ?", [n])?;
            }
        }
        SPORCO_LOCALI.with(|s| s.set(true));
        Ok(())
    })
}
