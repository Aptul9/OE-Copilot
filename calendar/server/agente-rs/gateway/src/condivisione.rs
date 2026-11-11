//! I link con cui il calendario si condivide con un altro OE Copilot (condividi.rs li serve), era lib/core/condivisione.ts e
//! lib/condivisione-comune.ts. Ogni link ha il suo indirizzo segreto, e chi lo ha legge il calendario: il testo segreto non esce da qui se non
//! dentro `url` e non si scrive in nessun log né in nessun errore. Stanno nella tabella condivisioni di gateway.db, mai nella configurazione.

use crate::archivio::archivio;
use crate::auth::cripto::{casuali, esadecimale};
use crate::orologio::adesso;
use crate::py::{js_trim, Esito};
use crate::stato::{load_config, membri_gruppo};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use rusqlite::{params, OptionalExtension, Row};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

pub const LIVELLI: [&str; 2] = ["occupato", "titoli"];
pub const AMBITI: [&str; 2] = ["tutti", "gruppo"];
pub const INDIETRO: (i64, i64, i64) = (0, 30, 7);
pub const AVANTI: (i64, i64, i64) = (1, 365, 90);
pub const EVENTI_MAX: usize = 5000;
pub const NOME_MAX: usize = 60;
pub const NOME_CALENDARIO_MAX: usize = 80;
pub const NOME_CALENDARIO_PREDEFINITO: &str = "Calendario condiviso";
pub const LINK_MAX: i64 = 50;
pub const INTERVALLO_ACCESSI_MS: i64 = 60_000;

/// Il nome con cui il calendario si pubblica: quello scelto, o quello di partenza.
pub fn nome_pubblicato(nome_calendario: &str) -> &str {
    if nome_calendario.is_empty() { NOME_CALENDARIO_PREDEFINITO } else { nome_calendario }
}

#[derive(Clone, Debug)]
pub struct Condivisione {
    pub id: String,
    pub nome: String,
    pub token: Option<String>,
    pub sale: String,
    pub livello: String,
    pub ambito: String,
    pub gruppo: String,
    pub giorni_indietro: i64,
    pub giorni_avanti: i64,
    pub nome_calendario: String,
    pub creato: i64,
    pub aggiornato: i64,
    pub ultimo_accesso: Option<i64>,
    pub accessi: i64,
    pub revocato: Option<i64>,
}

impl Condivisione {
    fn da(r: &Row) -> rusqlite::Result<Self> {
        Ok(Condivisione {
            id: r.get("id")?,
            nome: r.get("nome")?,
            token: r.get("token")?,
            sale: r.get("sale")?,
            livello: r.get("livello")?,
            ambito: r.get("ambito")?,
            gruppo: r.get("gruppo")?,
            giorni_indietro: r.get("giorni_indietro")?,
            giorni_avanti: r.get("giorni_avanti")?,
            nome_calendario: r.get("nome_calendario")?,
            creato: r.get("creato")?,
            aggiornato: r.get("aggiornato")?,
            ultimo_accesso: r.get("ultimo_accesso")?,
            accessi: r.get("accessi")?,
            revocato: r.get("revocato")?,
        })
    }
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS condivisioni (
  id TEXT PRIMARY KEY,
  nome TEXT NOT NULL,
  token TEXT UNIQUE,
  sale TEXT NOT NULL,
  livello TEXT NOT NULL DEFAULT 'occupato' CHECK (livello IN ('occupato', 'titoli')),
  ambito TEXT NOT NULL DEFAULT 'tutti' CHECK (ambito IN ('tutti', 'gruppo')),
  gruppo TEXT NOT NULL DEFAULT '',
  giorni_indietro INTEGER NOT NULL DEFAULT 7,
  giorni_avanti INTEGER NOT NULL DEFAULT 90,
  nome_calendario TEXT NOT NULL DEFAULT '',
  creato INTEGER NOT NULL,
  aggiornato INTEGER NOT NULL,
  ultimo_accesso INTEGER,
  accessi INTEGER NOT NULL DEFAULT 0,
  revocato INTEGER
)";

#[derive(Default, Clone, Copy)]
struct Accessi {
    scritto: i64,
    pendenti: i64,
    ultimo: i64,
}

thread_local! {
    static PRONTE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static ACCESSI: RefCell<HashMap<String, Accessi>> = RefCell::new(HashMap::new());
}

fn db() -> Esito<std::rc::Rc<rusqlite::Connection>> {
    let d = archivio()?;
    let file = crate::config::db_stato();
    if !PRONTE.with(|p| p.borrow().contains(&file)) {
        d.execute_batch(SCHEMA)?;
        PRONTE.with(|p| p.borrow_mut().insert(file));
    }
    Ok(d)
}

/// Crea la tabella dei link se non c'è: la chiama l'avvio del gateway, e comunque il primo uso.
pub fn prepara_condivisioni() -> Esito<()> {
    db().map(|_| ())
}

// ---- Il testo segreto -------------------------------------------------------------------------------------------------------
/// 32 byte casuali in base64url, senza riempimento: 43 caratteri.
pub fn forma_token(s: &str) -> bool {
    s.len() == 43 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn nuovo_token() -> String {
    crate::b64::codifica_url(&casuali(32))
}

fn forma_id(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// L'indirizzo pubblico di un link: APP_URL, il percorso fisso e il testo segreto.
pub fn indirizzo_link(token: &str) -> String {
    format!("{}/condividi/{token}.ics", crate::config::app_url())
}

/// Il link che ha questo testo segreto, o None se non c'è, è revocato o non ha la forma giusta. Il confronto è fra impronte di lunghezza
/// fissa, in tempo costante, e scorre sempre tutti i link.
pub fn trova_per_token(testo: &str) -> Esito<Option<Condivisione>> {
    let valido = forma_token(testo);
    let dato = Sha256::digest(if valido { testo } else { "" }.as_bytes());
    let d = db()?;
    let mut st = d.prepare_cached("SELECT * FROM condivisioni WHERE token IS NOT NULL AND revocato IS NULL")?;
    let tutte = st.query_map([], Condivisione::da)?.collect::<Result<Vec<_>, _>>()?;
    let mut trovata = None;
    for r in tutte {
        let altra = Sha256::digest(r.token.as_deref().unwrap_or("").as_bytes());
        let diversi = dato.iter().zip(altra.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b));
        if diversi == 0 {
            trovata = Some(r);
        }
    }
    Ok(if valido { trovata } else { None })
}

// ---- Gli accessi ------------------------------------------------------------------------------------------------------------
/// Segna che il link è stato letto adesso; nel database al più una volta al minuto per link. Un guasto non rovina la risposta.
pub fn registra_accesso(id: &str, ms: i64) {
    let v = ACCESSI.with(|a| {
        let mut a = a.borrow_mut();
        let v = a.entry(id.to_string()).or_default();
        v.pendenti += 1;
        v.ultimo = ms;
        *v
    });
    if ms - v.scritto < INTERVALLO_ACCESSI_MS {
        return;
    }
    let scritto = db().and_then(|d| Ok(d.execute("UPDATE condivisioni SET ultimo_accesso = ?, accessi = accessi + ? WHERE id = ?", params![ms, v.pendenti, id])?));
    match scritto {
        Ok(_) => ACCESSI.with(|a| {
            if let Some(x) = a.borrow_mut().get_mut(id) {
                x.scritto = ms;
                x.pendenti = 0;
            }
        }),
        Err(e) => eprintln!("condivisione {id}: accesso non registrato {}", e.messaggio()),
    }
}

// ---- Lettura ----------------------------------------------------------------------------------------------------------------
fn righe() -> Esito<Vec<Condivisione>> {
    let d = db()?;
    let mut st = d.prepare_cached("SELECT * FROM condivisioni ORDER BY creato, id")?;
    let v = st.query_map([], Condivisione::da)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn riga(id: Option<&Json>) -> Esito<Option<Condivisione>> {
    let Some(Json::Str(id)) = id.filter(|x| x.as_str().is_some_and(forma_id)) else { return Ok(None) };
    Ok(db()?.query_row("SELECT * FROM condivisioni WHERE id = ?", [id], Condivisione::da).optional()?)
}

/// Il link come lo vede la pagina: con gli accessi che aspettano ancora di andare nel database, senza sale, con l'indirizzo se vale ancora.
fn pubblica(c: &Condivisione, gruppi: Option<&Mappa>) -> Json {
    let m = ACCESSI.with(|a| a.borrow().get(&c.id).copied());
    let ultimo = c.ultimo_accesso.unwrap_or(0).max(m.map_or(0, |x| x.ultimo));
    let revocato = c.revocato.is_some_and(|r| r != 0);
    ogg! {
        "id": c.id.clone(),
        "nome": c.nome.clone(),
        "livello": c.livello.clone(),
        "ambito": c.ambito.clone(),
        "gruppo": c.gruppo.clone(),
        "giorni_indietro": c.giorni_indietro as f64,
        "giorni_avanti": c.giorni_avanti as f64,
        "nome_calendario": c.nome_calendario.clone(),
        "creato": c.creato as f64,
        "aggiornato": c.aggiornato as f64,
        "ultimo_accesso": if ultimo == 0 { Json::Null } else { Json::Num(ultimo as f64) },
        "accessi": (c.accessi + m.map_or(0, |x| x.pendenti)) as f64,
        "revocato": c.revocato.map_or(Json::Null, |r| Json::Num(r as f64)),
        "url": match &c.token { Some(t) if !t.is_empty() && !revocato => Json::Str(indirizzo_link(t)), _ => Json::Null },
        "gruppo_mancante": c.ambito == "gruppo" && !revocato && !gruppi.is_some_and(|g| matches!(g.get(&c.gruppo), Some(Json::Arr(_)))),
    }
}

fn gruppi_di_adesso() -> Option<Mappa> {
    load_config().ok().and_then(|c| c.get("gruppi_calendari").and_then(Json::as_obj).cloned())
}

/// Tutti i link, dal più vecchio.
pub fn elenco_condivisioni() -> Esito<Json> {
    let gruppi = gruppi_di_adesso();
    Ok(ogg! { "ok": true, "condivisioni": Json::Arr(righe()?.iter().map(|c| pubblica(c, gruppi.as_ref())).collect()), "max": LINK_MAX as f64 })
}

// ---- Scrittura --------------------------------------------------------------------------------------------------------------
#[derive(Clone)]
struct Campi {
    nome: String,
    livello: String,
    ambito: String,
    gruppo: String,
    giorni_indietro: i64,
    giorni_avanti: i64,
    nome_calendario: String,
}

fn controllo(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}'))
}

fn intero(v: &Json) -> Option<i64> {
    match v {
        Json::Num(n) if n.fract() == 0.0 && n.is_finite() => Some(*n as i64),
        Json::Str(s) => {
            let t = s.trim_matches(crate::py::spazio_js);
            (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())).then(|| crate::py::numero_da_testo(t) as i64)
        }
        _ => None,
    }
}

/// I campi di un link: `dati` è quello che arriva dalla pagina, `base` i valori che restano se un campo non c'è.
fn leggi_campi(dati: Option<&Json>, base: Option<Campi>) -> Result<Campi, String> {
    let Some(Json::Obj(d)) = dati else { return Err("I dati del link non sono validi.".to_string()) };
    let nuovo = base.is_none();
    let mut c = base.unwrap_or(Campi {
        nome: String::new(),
        livello: "occupato".to_string(),
        ambito: "tutti".to_string(),
        gruppo: String::new(),
        giorni_indietro: INDIETRO.2,
        giorni_avanti: AVANTI.2,
        nome_calendario: String::new(),
    });
    // il gruppo si controlla quando lo si sceglie, non a ogni modifica: un link il cui gruppo è sparito si può comunque rinominare o revocare
    let scelta_del_gruppo = nuovo || d.contains_key("ambito") || d.contains_key("gruppo");
    if let Some(n) = d.get("nome") {
        let Json::Str(n) = n else { return Err("Il nome non è valido.".to_string()) };
        c.nome = js_trim(n).to_string();
    }
    if c.nome.is_empty() {
        return Err("Scrivi per chi è il link: il nome di chi lo riceve.".to_string());
    }
    if c.nome.chars().count() > NOME_MAX {
        return Err(format!("Il nome è troppo lungo: al più {NOME_MAX} caratteri."));
    }
    if controllo(&c.nome) {
        return Err("Il nome non può avere a capo né caratteri di controllo.".to_string());
    }
    if let Some(l) = d.get("livello") {
        match l.as_str().filter(|x| LIVELLI.contains(x)) {
            Some(x) => c.livello = x.to_string(),
            None => return Err("Il livello non è valido: scegli Occupato oppure Con titoli.".to_string()),
        }
    }
    if let Some(a) = d.get("ambito") {
        match a.as_str().filter(|x| AMBITI.contains(x)) {
            Some(x) => c.ambito = x.to_string(),
            None => return Err("Cosa condividere non è valido: tutti i calendari, oppure un gruppo.".to_string()),
        }
    }
    if let Some(g) = d.get("gruppo") {
        let Json::Str(g) = g else { return Err("Il nome del gruppo non è valido.".to_string()) };
        c.gruppo = js_trim(g).to_string();
    }
    if c.ambito == "tutti" {
        c.gruppo = String::new();
    } else {
        if c.gruppo.is_empty() {
            return Err("Scegli il gruppo da condividere.".to_string());
        }
        if c.gruppo.chars().count() > crate::disponibilita::GRUPPO_MAX || controllo(&c.gruppo) {
            return Err("Il nome del gruppo non è valido.".to_string());
        }
        if scelta_del_gruppo && load_config().ok().and_then(|cfg| membri_gruppo(&cfg, &c.gruppo)).is_none() {
            return Err(format!("Il gruppo «{}» non esiste: si crea in Gestisci calendari.", c.gruppo));
        }
    }
    if let Some(v) = d.get("giorni_indietro") {
        match intero(v).filter(|n| (INDIETRO.0..=INDIETRO.1).contains(n)) {
            Some(n) => c.giorni_indietro = n,
            None => return Err(format!("Giorni indietro: un numero intero da {} a {}.", INDIETRO.0, INDIETRO.1)),
        }
    }
    if let Some(v) = d.get("giorni_avanti") {
        match intero(v).filter(|n| (AVANTI.0..=AVANTI.1).contains(n)) {
            Some(n) => c.giorni_avanti = n,
            None => return Err(format!("Giorni avanti: un numero intero da {} a {}.", AVANTI.0, AVANTI.1)),
        }
    }
    if let Some(n) = d.get("nome_calendario") {
        let Json::Str(n) = n else { return Err("Il nome del calendario non è valido.".to_string()) };
        c.nome_calendario = js_trim(n).to_string();
    }
    if c.nome_calendario.chars().count() > NOME_CALENDARIO_MAX {
        return Err(format!("Il nome del calendario è troppo lungo: al più {NOME_CALENDARIO_MAX} caratteri."));
    }
    if controllo(&c.nome_calendario) {
        return Err("Il nome del calendario non può avere a capo né caratteri di controllo.".to_string());
    }
    Ok(c)
}

fn no(e: impl Into<String>) -> Json {
    ogg! { "ok": false, "error": e.into() }
}

fn con(c: &Condivisione) -> Json {
    ogg! { "ok": true, "condivisione": pubblica(c, gruppi_di_adesso().as_ref()) }
}

fn applica(c: &mut Condivisione, k: Campi) {
    c.nome = k.nome;
    c.livello = k.livello;
    c.ambito = k.ambito;
    c.gruppo = k.gruppo;
    c.giorni_indietro = k.giorni_indietro;
    c.giorni_avanti = k.giorni_avanti;
    c.nome_calendario = k.nome_calendario;
}

/// Crea un link: l'indirizzo segreto lo fa il server e la pagina lo riceve nella risposta.
pub fn crea_condivisione(dati: Option<&Json>) -> Esito<Json> {
    let campi = match leggi_campi(dati, None) {
        Ok(c) => c,
        Err(e) => return Ok(no(e)),
    };
    let d = db()?;
    let n: i64 = d.query_row("SELECT COUNT(*) FROM condivisioni", [], |r| r.get(0))?;
    if n >= LINK_MAX {
        return Ok(no(format!("Ci sono già {LINK_MAX} link: elimina quelli che non servono più.")));
    }
    let ora = adesso();
    let mut nuovo = Condivisione {
        id: esadecimale(&casuali(6)),
        nome: String::new(),
        token: Some(nuovo_token()),
        sale: esadecimale(&casuali(16)),
        livello: String::new(),
        ambito: String::new(),
        gruppo: String::new(),
        giorni_indietro: 0,
        giorni_avanti: 0,
        nome_calendario: String::new(),
        creato: ora,
        aggiornato: ora,
        ultimo_accesso: None,
        accessi: 0,
        revocato: None,
    };
    applica(&mut nuovo, campi);
    d.execute(
        "INSERT INTO condivisioni (id, nome, token, sale, livello, ambito, gruppo, giorni_indietro, giorni_avanti, nome_calendario, creato, aggiornato, ultimo_accesso, accessi, revocato)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![nuovo.id, nuovo.nome, nuovo.token, nuovo.sale, nuovo.livello, nuovo.ambito, nuovo.gruppo, nuovo.giorni_indietro, nuovo.giorni_avanti, nuovo.nome_calendario, nuovo.creato, nuovo.aggiornato, nuovo.ultimo_accesso, nuovo.accessi, nuovo.revocato],
    )?;
    println!("condivisione {}: creata ({}, {})", nuovo.id, nuovo.livello, nuovo.ambito);
    Ok(con(&nuovo))
}

/// Cambia i campi dati (gli altri restano).
pub fn aggiorna_condivisione(id: Option<&Json>, dati: Option<&Json>) -> Esito<Json> {
    let Some(mut c) = riga(id)? else { return Ok(no("Link non trovato.")) };
    if c.revocato.is_some_and(|r| r != 0) {
        return Ok(no("Il link è revocato: non si può più cambiare."));
    }
    let base = Campi { nome: c.nome.clone(), livello: c.livello.clone(), ambito: c.ambito.clone(), gruppo: c.gruppo.clone(), giorni_indietro: c.giorni_indietro, giorni_avanti: c.giorni_avanti, nome_calendario: c.nome_calendario.clone() };
    let campi = match leggi_campi(dati, Some(base)) {
        Ok(k) => k,
        Err(e) => return Ok(no(e)),
    };
    c.aggiornato = adesso().max(c.aggiornato + 1);
    applica(&mut c, campi);
    db()?.execute(
        "UPDATE condivisioni SET nome = ?, livello = ?, ambito = ?, gruppo = ?, giorni_indietro = ?, giorni_avanti = ?, nome_calendario = ?, aggiornato = ? WHERE id = ?",
        params![c.nome, c.livello, c.ambito, c.gruppo, c.giorni_indietro, c.giorni_avanti, c.nome_calendario, c.aggiornato, c.id],
    )?;
    println!("condivisione {}: cambiata", c.id);
    Ok(con(&c))
}

/// Revoca: l'indirizzo smette di valere subito e il suo testo segreto si cancella. La riga resta.
pub fn revoca_condivisione(id: Option<&Json>) -> Esito<Json> {
    let Some(mut c) = riga(id)? else { return Ok(no("Link non trovato.")) };
    if c.revocato.is_some_and(|r| r != 0) {
        return Ok(con(&c));
    }
    let ora = adesso();
    c.aggiornato = ora.max(c.aggiornato + 1);
    c.token = None;
    c.revocato = Some(ora);
    db()?.execute("UPDATE condivisioni SET token = NULL, revocato = ?, aggiornato = ? WHERE id = ?", params![ora, c.aggiornato, c.id])?;
    println!("condivisione {}: revocata", c.id);
    Ok(con(&c))
}

/// Toglie il link dall'elenco, qualunque sia il suo stato. Un id che non c'è non cambia niente.
pub fn elimina_condivisione(id: Option<&Json>) -> Esito<Json> {
    if let Some(c) = riga(id)? {
        db()?.execute("DELETE FROM condivisioni WHERE id = ?", [&c.id])?;
        ACCESSI.with(|a| a.borrow_mut().remove(&c.id));
        println!("condivisione {}: eliminata", c.id);
    }
    Ok(ogg! { "ok": true })
}

/// Cambia l'indirizzo segreto: il vecchio smette di valere subito.
pub fn ruota_condivisione(id: Option<&Json>) -> Esito<Json> {
    let Some(mut c) = riga(id)? else { return Ok(no("Link non trovato.")) };
    if c.revocato.is_some_and(|r| r != 0) {
        return Ok(no("Il link è revocato: se ne crea uno nuovo."));
    }
    let token = nuovo_token();
    c.aggiornato = adesso().max(c.aggiornato + 1);
    db()?.execute("UPDATE condivisioni SET token = ?, aggiornato = ? WHERE id = ?", params![token, c.aggiornato, c.id])?;
    c.token = Some(token);
    println!("condivisione {}: indirizzo cambiato", c.id);
    Ok(con(&c))
}
