//! I loghi delle aziende (era lib/core/loghi.ts). Quelli di personalizzazione.json (interfaccia.loghi) restano come sono; sopra si mettono le
//! scelte fatte dalla finestra Calendari, tabella `loghi` di gateway.db: nome mostrato -> indirizzo di partenza e file. Un indirizzo lo
//! scarica il gateway una volta sola, al momento del salvataggio, e lo tiene in ui-dati/logos/url-<16 cifre di sha256(indirizzo)>.<estensione>:
//! il browser chiede sempre e solo /logos/<file> a questo sito. Un indirizzo scritto a mano in personalizzazione.json si scarica allo stesso
//! modo, in secondo piano e rinnovato ogni 7 giorni: finché il file non c'è la pagina mostra le iniziali.

use crate::archivio::{archivio, in_transazione, transazione};
use crate::dati::nelle_dati;
use crate::immagine::{controlla_immagine, EsitoImmagine, TipoImmagine, LATO_MAX, PIXEL_MAX};
use crate::personalizzazione::personalizzazione;
use crate::py::{lunghezza_js, Errore, Esito, Motivo};
use crate::rete_sicura::{controlla_url, scarica_sicuro, testo_troppo_grande, Opzioni, LIMITE_ACCETTATO};
use crate::uscita::{posti, POSTI_MAX};
use crate::valore_logo::come_lo_legge;
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;

pub const LOGHI_MAX: i64 = 60;
/// Gli indirizzi di personalizzazione.json che si scaricano: gli altri si ignorano.
pub const STATICI_MAX: usize = 40;
pub const TTL_STATICI_MS: i64 = 7 * 24 * 3600 * 1000;
const RIPROVA_MS: i64 = 5 * 60 * 1000;
/// Lo scarico in secondo piano ne tiene aperti al più 3: l'ultimo posto resta a chi ha appena premuto un pulsante nella finestra.
const SCARICHI_MAX: usize = POSTI_MAX - 1;
const NOME_MAX: usize = 200;

pub const NOME_NON_VALIDO: &str = "Nome non valido";
pub const NON_IMMAGINE: &str = "Non è un'immagine";
pub const SVG: &str = "SVG non ammesso: contiene codice o riferimenti esterni";
pub const SALVATAGGIO: &str = "Non riesco a salvare il logo sul server";
pub const NON_CE: &str = "Per questo nome non c'è un logo scelto da qui";
pub const DA_FILE: &str = "Il logo è un file caricato: non c'è un indirizzo da rileggere";
pub const CAMBIATO: &str = "Il logo è stato cambiato nel frattempo: riprova";

pub fn testo_pixel() -> String {
    format!("Immagine troppo grande (massimo {} pixel per lato e {} megapixel)", LATO_MAX, PIXEL_MAX / 1_000_000.0)
}

pub fn testo_troppi() -> String {
    format!("Troppi loghi scelti da qui (massimo {LOGHI_MAX})")
}

fn rifiuto(m: impl Into<String>) -> Errore {
    Errore::Valore(m.into())
}

#[derive(Clone, Debug)]
struct Riga {
    nome: String,
    url: Option<String>,
    file: String,
    tipo: String,
    aggiornato: i64,
}

// ---- La rete ----------------------------------------------------------------------------------------------------------------------
fn rete_di_prova() -> Option<crate::rete_sicura::SoloProve> {
    crate::uscita::rete_di_prova()
}

/// L'indirizzo scritto da chi usa l'app, controllato e ripulito del frammento: sempre lo stesso testo per lo stesso indirizzo.
fn prepara_url(testo: Option<&Json>) -> Esito<String> {
    let Some(Json::Str(t)) = testo else { return Err(crate::rete_sicura::errore(Motivo::NonValido, crate::rete_sicura::NON_VALIDO)) };
    let mut u = controlla_url(t, rete_di_prova().as_ref())?;
    u.set_fragment(None);
    Ok(u.as_str().to_string())
}

fn controllato(dati: Vec<u8>) -> Esito<(TipoImmagine, Vec<u8>)> {
    if dati.len() > LIMITE_ACCETTATO {
        return Err(crate::rete_sicura::errore(Motivo::TroppoGrande, testo_troppo_grande()));
    }
    match controlla_immagine(&dati) {
        EsitoImmagine::Tipo(t) => Ok((t, dati)),
        EsitoImmagine::NonImmagine => Err(rifiuto(NON_IMMAGINE)),
        EsitoImmagine::SvgNonAmmesso => Err(rifiuto(SVG)),
        EsitoImmagine::TroppiPixel => Err(rifiuto(testo_pixel())),
    }
}

/// I byte del file di `url`, riconosciuti dai primi byte, senza aspettare un posto.
async fn scarica(url: &str) -> Esito<(TipoImmagine, Vec<u8>)> {
    let r = scarica_sicuro(url, rete_di_prova().as_ref(), &Opzioni::default()).await?;
    controllato(r.dati)
}

/// Come `scarica`, per chi risponde a una richiesta della pagina: aspetta un posto (in coda corta) o risponde "Troppe richieste".
async fn scarica_logo(url: &str) -> Esito<(TipoImmagine, Vec<u8>)> {
    let _posto = posti().prendi().await?;
    scarica(url).await
}

// ---- La tabella ------------------------------------------------------------------------------------------------------------------
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS loghi (
  nome TEXT PRIMARY KEY,
  url TEXT,
  file TEXT NOT NULL,
  tipo TEXT NOT NULL CHECK (tipo IN ('url', 'file')),
  aggiornato INTEGER NOT NULL
)";

thread_local! {
    static PRONTE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn tabella() -> Esito<std::rc::Rc<rusqlite::Connection>> {
    let db = archivio()?;
    let file = crate::config::db_stato();
    if !PRONTE.with(|p| p.borrow().contains(&file)) {
        db.execute_batch(SCHEMA)?;
        // dentro una transazione che poi torna indietro la tabella tornerebbe indietro con lei: ci si fida solo di quella creata fuori
        if !in_transazione() {
            PRONTE.with(|p| p.borrow_mut().insert(file));
        }
    }
    Ok(db)
}

/// All'avvio del gateway: la tabella si aggiunge (CREATE TABLE IF NOT EXISTS), mai si cambia o si toglie niente di quello che c'è.
pub fn migra_loghi() -> Esito<()> {
    tabella().map(|_| ())
}

fn riga_da(r: &rusqlite::Row) -> rusqlite::Result<Riga> {
    Ok(Riga { nome: r.get(0)?, url: r.get(1)?, file: r.get(2)?, tipo: r.get(3)?, aggiornato: r.get(4)? })
}

fn righe() -> Esito<Vec<Riga>> {
    let db = tabella()?;
    let mut st = db.prepare_cached("SELECT nome, url, file, tipo, aggiornato FROM loghi ORDER BY nome")?;
    let v = st.query_map([], riga_da)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn riga(nome: &str) -> Esito<Option<Riga>> {
    Ok(tabella()?.query_row("SELECT nome, url, file, tipo, aggiornato FROM loghi WHERE nome = ?", [nome], riga_da).optional()?)
}

fn voce(r: &Riga) -> Json {
    ogg! { "url": r.url.clone(), "tipo": r.tipo.clone(), "aggiornato": r.aggiornato }
}

/// I loghi scelti dalla finestra Calendari, per nome mostrato.
pub fn leggi_voci() -> Esito<Mappa> {
    Ok(righe()?.iter().map(|r| (r.nome.clone(), voce(r))).collect())
}

fn nome_valido(nome: Option<&Json>) -> Esito<String> {
    let n = match nome {
        Some(Json::Str(s)) => crate::py::js_trim(s).to_string(),
        _ => String::new(),
    };
    if n.is_empty() || lunghezza_js(&n) > NOME_MAX || n.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
        return Err(rifiuto(NOME_NON_VALIDO));
    }
    Ok(n)
}

// ---- I file ------------------------------------------------------------------------------------------------------------------------
fn cartella() -> PathBuf {
    nelle_dati(&["logos"])
}

fn hash16(dati: &[u8]) -> String {
    let h = Sha256::digest(dati);
    h.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Solo questi nomi sono dei loghi gestiti qui: tutto il resto della cartella (i file messi a mano) non si tocca mai.
static GESTITO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:url|up)-[0-9a-f]{16}\.(?:png|jpg|gif|webp|ico|svg)$").unwrap());

fn rimuovi(nome: &str) {
    if let Err(e) = std::fs::remove_file(cartella().join(nome)) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("loghi: {nome} non rimosso {e}");
        }
    }
}

/// Scrive il file tutto insieme (prima uno temporaneo, poi la sostituzione): chi lo legge non vede mai un file a metà.
fn scrivi(nome: &str, dati: &[u8]) -> Esito<()> {
    let mut casuale = [0u8; 6];
    let _ = getrandom::fill(&mut casuale);
    let tmp = format!(".logo-tmp-{}-{}", std::process::id(), casuale.iter().map(|b| format!("{b:02x}")).collect::<String>());
    crate::dati::scrivi_atomico(&cartella().join(nome), dati, 0o644, &tmp).map_err(|e| Errore::Altro { nome: "Error", messaggio: e.to_string() })
}

/// L'indirizzo di un valore di personalizzazione.json, o None se il valore è un file. Il valore si legge come lo legge il browser.
fn url_da_valore(v: &Json) -> Option<String> {
    let t = come_lo_legge(v.as_str()?);
    let minuscolo = t.to_ascii_lowercase();
    if !(minuscolo.starts_with("//") || minuscolo.starts_with("http://") || minuscolo.starts_with("https://")) {
        return None;
    }
    let assoluto = if t.starts_with("//") { format!("https:{t}") } else { t };
    let mut u = url::Url::parse(&assoluto).ok()?;
    u.set_fragment(None);
    Some(u.as_str().to_string())
}

/// Gli indirizzi di personalizzazione.json che si scaricano, senza ripetizioni e fino a STATICI_MAX.
fn url_statici() -> Vec<String> {
    let p = personalizzazione();
    let mut trovati: Vec<String> = Vec::new();
    if let Some(Json::Obj(loghi)) = p.interfaccia.get("loghi") {
        for v in loghi.values() {
            if let Some(u) = url_da_valore(v) {
                if trovati.len() < STATICI_MAX && !trovati.contains(&u) {
                    trovati.push(u);
                }
            }
        }
    }
    trovati
}

/// Toglie i file gestiti che nessuno usa più, e gli avanzi di una scrittura interrotta. Si chiama dopo ogni modifica, a transazione chiusa.
pub fn pulisci_loghi() {
    let Ok(voci) = std::fs::read_dir(cartella()) else { return };
    let usati: HashSet<String> = righe().map(|r| r.into_iter().map(|x| x.file).collect()).unwrap_or_default();
    let statici: HashSet<String> = url_statici().iter().map(|u| hash16(u.as_bytes())).collect();
    for f in voci.flatten() {
        let f = f.file_name().to_string_lossy().into_owned();
        if f.starts_with(".logo-tmp-") {
            rimuovi(&f);
        } else if GESTITO.is_match(&f) && !usati.contains(&f) && !(f.starts_with("url-") && statici.contains(&f[4..20])) {
            rimuovi(&f);
        }
    }
}

// ---- Le scelte fatte dalla finestra ----------------------------------------------------------------------------------------------
/// Salva la scelta: il file, la riga, e poi la pulizia di quello che non serve più. Tutto in un blocco, senza attese.
fn registra(nome: &str, tipo: &str, url: Option<&str>, img: TipoImmagine, dati: &[u8]) -> Esito<Json> {
    let base = if tipo == "url" { url.unwrap_or("").as_bytes() } else { dati };
    let file = format!("{}-{}.{}", if tipo == "url" { "url" } else { "up" }, hash16(base), img.ext);
    let aggiornato = crate::orologio::ora_vera();
    tabella()?;
    let esito = transazione(|| {
        let db = tabella()?;
        let n: i64 = db.query_row("SELECT COUNT(*) AS n FROM loghi", [], |r| r.get(0))?;
        if riga(nome)?.is_none() && n >= LOGHI_MAX {
            return Err(rifiuto(testo_troppi()));
        }
        scrivi(&file, dati)?;
        db.execute(
            "INSERT INTO loghi(nome, url, file, tipo, aggiornato) VALUES (?, ?, ?, ?, ?) ON CONFLICT(nome) DO UPDATE SET url = excluded.url, file = excluded.file, tipo = excluded.tipo, aggiornato = excluded.aggiornato",
            params![nome, url, file, tipo, aggiornato],
        )?;
        Ok(())
    });
    pulisci_loghi();
    esito?;
    Ok(ogg! { "url": url.map(str::to_string), "tipo": tipo, "aggiornato": aggiornato })
}

/// Scarica l'indirizzo e lo mostra come anteprima, senza salvare niente.
pub async fn anteprima_logo(indirizzo: Option<&Json>) -> Esito<Mappa> {
    let (tipo, dati) = scarica_logo(&prepara_url(indirizzo)?).await?;
    Ok(crate::py::mappa(ogg! { "anteprima": format!("data:{};base64,{}", tipo.mime, crate::b64::codifica(&dati)), "tipo": tipo.mime, "byte": dati.len() }))
}

/// Il logo di `nome` da un indirizzo: scaricato qui, subito, una volta. Se qualcosa non va non cambia niente.
pub async fn imposta_logo(nome: Option<&Json>, indirizzo: Option<&Json>) -> Esito<Json> {
    let n = nome_valido(nome)?;
    let url = prepara_url(indirizzo)?;
    let (tipo, dati) = scarica_logo(&url).await?;
    registra(&n, "url", Some(&url), tipo, &dati)
}

/// Il logo di `nome` da un file caricato (base64): stesse regole dell'indirizzo, stessi limiti.
pub fn carica_logo(nome: Option<&Json>, base64: Option<&Json>) -> Esito<Json> {
    let n = nome_valido(nome)?;
    static B64: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+/]+={0,2}$").unwrap());
    let Some(Json::Str(b)) = base64.filter(|v| v.as_str().is_some_and(|s| B64.is_match(s))) else { return Err(rifiuto(NON_IMMAGINE)) };
    let (tipo, dati) = controllato(crate::b64::decodifica_node(b))?;
    registra(&n, "file", None, tipo, &dati)
}

/// Legge di nuovo l'indirizzo del logo di `nome`. Un logo cambiato o tolto nel frattempo da un'altra richiesta non si sovrascrive.
pub async fn aggiorna_logo(nome: Option<&Json>) -> Esito<Json> {
    let n = nome_valido(nome)?;
    let Some(prima) = riga(&n)? else { return Err(rifiuto(NON_CE)) };
    let Some(url) = prima.url.clone().filter(|u| prima.tipo == "url" && !u.is_empty()) else { return Err(rifiuto(DA_FILE)) };
    let (tipo, dati) = scarica_logo(&url).await?;
    if riga(&n)?.and_then(|r| r.url) != Some(url.clone()) {
        return Err(rifiuto(CAMBIATO));
    }
    registra(&n, "url", Some(&url), tipo, &dati)
}

/// Torna a quello di personalizzazione.json (o alle iniziali): la scelta si toglie e il suo file con lei, se nessuno lo usa.
pub fn togli_logo(nome: Option<&Json>) -> Esito<()> {
    let n = nome_valido(nome)?;
    transazione(|| {
        tabella()?.execute("DELETE FROM loghi WHERE nome = ?", [&n])?;
        Ok(())
    })?;
    pulisci_loghi();
    Ok(())
}

/// Il nome di un'azienda cambia e il suo logo la segue. Si chiama dentro la transazione della configurazione.
pub fn rinomina_logo(prima: &str, dopo: &str) -> Esito<()> {
    if prima == dopo {
        return Ok(());
    }
    let db = tabella()?;
    if riga(prima)?.is_none() {
        return Ok(());
    }
    if riga(dopo)?.is_some() {
        db.execute("DELETE FROM loghi WHERE nome = ?", [prima])?;
    } else {
        db.execute("UPDATE loghi SET nome = ? WHERE nome = ?", [dopo, prima])?;
    }
    Ok(())
}

/// Come lo mostra la pagina: un testo chiaro, e nel log il resto se non è un rifiuto previsto.
pub fn esito_negativo(nome_metodo: &str, e: &Errore) -> Json {
    if e.e_valore() {
        return ogg! { "ok": false, "error": e.messaggio() };
    }
    eprintln!("{nome_metodo}: eccezione {}: {}", e.nome(), e.messaggio());
    ogg! { "ok": false, "error": SALVATAGGIO }
}

// ---- Gli indirizzi scritti a mano in personalizzazione.json -----------------------------------------------------------------------
#[derive(Default)]
struct StatoScarichi {
    in_volo: HashSet<String>,
    fallite: IndexMap<String, i64>,
}

thread_local! {
    static SCARICHI: RefCell<StatoScarichi> = RefCell::new(StatoScarichi::default());
}

/// Quanti scarichi sono in corso (per le prove).
pub fn scarichi_in_corso() -> usize {
    SCARICHI.with(|s| s.borrow().in_volo.len())
}

/// Dimentica gli scarichi falliti, che altrimenti si ritentano dopo 5 minuti (per le prove).
pub fn azzera_scarichi() {
    SCARICHI.with(|s| s.borrow_mut().fallite.clear());
}

fn file_in_cache(url: &str) -> Option<(String, f64)> {
    let prefisso = format!("url-{}.", hash16(url.as_bytes()));
    let mut migliore: Option<(String, f64)> = None;
    for f in std::fs::read_dir(cartella()).ok()?.flatten() {
        let nome = f.file_name().to_string_lossy().into_owned();
        if !nome.starts_with(&prefisso) || !GESTITO.is_match(&nome) {
            continue;
        }
        let Some(mtime) = f.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64() * 1000.0) else { continue };
        if migliore.as_ref().is_none_or(|(_, m)| mtime > *m) {
            migliore = Some((nome, mtime));
        }
    }
    migliore
}

fn scrivi_statico(url: &str, img: TipoImmagine, dati: &[u8]) -> Esito<()> {
    let h = hash16(url.as_bytes());
    let file = format!("url-{h}.{}", img.ext);
    scrivi(&file, dati)?;
    // se il tipo del file è cambiato resta una copia sola
    if let Ok(voci) = std::fs::read_dir(cartella()) {
        for f in voci.flatten() {
            let f = f.file_name().to_string_lossy().into_owned();
            if f != file && f.starts_with(&format!("url-{h}.")) && GESTITO.is_match(&f) {
                rimuovi(&f);
            }
        }
    }
    Ok(())
}

/// Avvia in secondo piano lo scarico di un indirizzo di personalizzazione.json: uno alla volta per indirizzo, al più SCARICHI_MAX insieme,
/// solo se un posto c'è subito, e dopo un errore si riprova dopo 5 minuti.
fn avvia_scarico(url: &str) {
    let ora = crate::orologio::ora_vera();
    let libero = SCARICHI.with(|s| {
        let s = s.borrow();
        let fallita = s.fallite.get(url).copied();
        !(s.in_volo.contains(url) || s.in_volo.len() >= SCARICHI_MAX || fallita.is_some_and(|f| ora - f < RIPROVA_MS))
    });
    if !libero {
        return;
    }
    let Some(posto) = posti().prova_prendere() else { return };
    SCARICHI.with(|s| s.borrow_mut().in_volo.insert(url.to_string()));
    let url = url.to_string();
    tokio::task::spawn_local(async move {
        let esito = match scarica(&url).await {
            Ok((tipo, dati)) => scrivi_statico(&url, tipo, &dati),
            Err(e) => Err(e),
        };
        match esito {
            Ok(()) => {
                SCARICHI.with(|s| s.borrow_mut().fallite.shift_remove(&url));
                pulisci_loghi();
            }
            Err(e) => {
                SCARICHI.with(|s| {
                    let mut s = s.borrow_mut();
                    s.fallite.insert(url.clone(), crate::orologio::ora_vera());
                    if s.fallite.len() > 4 * STATICI_MAX {
                        s.fallite.shift_remove_index(0);
                    }
                });
                // solo il sito, mai l'indirizzo intero: potrebbe portarsi dietro un token
                let host = url::Url::parse(&url).ok().map(|u| format!("{}{}", u.host_str().unwrap_or(""), u.port().map(|p| format!(":{p}")).unwrap_or_default())).unwrap_or_default();
                eprintln!("loghi: {host}: {}", e.messaggio());
            }
        }
        SCARICHI.with(|s| s.borrow_mut().in_volo.remove(&url));
        drop(posto);
    });
}

/// I valori di interfaccia.loghi con gli indirizzi sostituiti dal file in cache; senza il file (ancora) la voce manca e il nome va in
/// `in_corso`. Un nome che ha una scelta fatta dalla finestra non si scarica: vince la scelta.
fn risolvi_statici(statici: &Mappa, sostituiti: &HashSet<String>) -> (Vec<(String, Json)>, Vec<String>) {
    let mut valori = Vec::new();
    let mut in_corso = Vec::new();
    let ammessi: HashSet<String> = url_statici().into_iter().collect();
    for (nome, v) in statici.iter() {
        let Some(url) = url_da_valore(v) else {
            valori.push((nome.clone(), v.clone()));
            continue;
        };
        if !ammessi.contains(&url) || sostituiti.contains(nome) {
            continue;
        }
        let c = file_in_cache(&url);
        if c.as_ref().is_none_or(|(_, m)| crate::orologio::ora_vera() as f64 - m > TTL_STATICI_MS as f64) {
            avvia_scarico(&url);
        }
        match c {
            Some((file, mtime)) => valori.push((nome.clone(), Json::Str(format!("logos/{file}?v={}", mtime.trunc() as i64)))),
            None => in_corso.push(nome.clone()),
        }
    }
    (valori, in_corso)
}

/// L'interfaccia di personalizzazione.json con i loghi uniti: prima quelli scritti nel file, sopra le scelte fatte dalla finestra, che
/// vincono. Senza né l'una né le altre è l'oggetto di prima.
pub fn interfaccia_con_loghi(interfaccia: &Mappa) -> Esito<Mappa> {
    let statici = interfaccia.get("loghi").and_then(Json::as_obj);
    let scelte = righe()?;
    if statici.is_none() && scelte.is_empty() {
        return Ok(interfaccia.clone());
    }
    let (valori, in_corso) = match statici {
        Some(s) => risolvi_statici(s, &scelte.iter().map(|r| r.nome.clone()).collect()),
        None => (Vec::new(), Vec::new()),
    };
    let mut fuori = interfaccia.clone();
    let mut loghi = Mappa::new();
    for (k, v) in valori {
        loghi.insert(k, v);
    }
    for r in &scelte {
        loghi.insert(r.nome.clone(), format!("logos/{}?v={}", r.file, r.aggiornato));
    }
    fuori.insert("loghi", Json::Obj(loghi));
    if !in_corso.is_empty() {
        fuori.insert("loghi_in_corso", Json::Arr(in_corso.into_iter().map(Json::Str).collect()));
    }
    Ok(fuori)
}
