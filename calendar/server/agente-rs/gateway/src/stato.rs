//! La configurazione di chi usa l'app (era lib/core/stato.ts): calendari spenti, alias, colori, gruppi, link ICS, tema. Sta nel database dello
//! stato (archivio.rs); qui le regole che la leggono e la cambiano.

pub use crate::archivio::{load_config, modifica_config, save_config, Config};
use crate::personalizzazione::{personalizzazione, COLOR_PALETTE};
use crate::py::{js_trim, py_bool, py_str, py_str_v, Esito};
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use std::cmp::Ordering;
use std::collections::HashSet;

/// La tavolozza: il primo colore libero, o `preferito` se nessuno lo usa; finite le tinte si ricomincia.
fn assegna_colore(colori: &mut Mappa, username: &str, preferito: Option<&Json>) {
    let usati: HashSet<String> = colori.values().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.in_js())).collect();
    // `preferito && !usati.has(preferito)`: un valore che non è un testo non è mai fra i colori usati, e si scrive com'è
    if py_bool(preferito) {
        let p = preferito.unwrap();
        let gia = p.as_str().is_some_and(|s| usati.contains(s));
        if !gia {
            colori.insert(username, p.clone());
            return;
        }
    }
    let libero = COLOR_PALETTE.iter().find(|c| !usati.contains(**c));
    let scelto = libero.copied().unwrap_or(COLOR_PALETTE[colori.len() % COLOR_PALETTE.len()]);
    colori.insert(username, scelto);
}

fn colori_mut(cfg: &mut Config) -> &mut Mappa {
    if !matches!(cfg.get("account_colors"), Some(Json::Obj(_))) {
        cfg.insert("account_colors", Json::obj());
    }
    cfg.get_mut("account_colors").and_then(Json::as_obj_mut).expect("appena messo")
}

/// Il colore stabile dell'account; se manca lo assegna (il primo libero della tavolozza, o `preferito`) e lo salva. Si assegna sullo stato
/// di adesso, non su quello che chi chiede si è letto qualche istante fa: una richiesta lenta non cancella i colori (né gli interruttori)
/// assegnati o cambiati nel frattempo.
pub fn color_for_account(cfg: &mut Config, username: &str, preferito: Option<&Json>) -> Esito<Json> {
    let sola_lettura = py_bool(cfg.get("_sola_lettura"));
    let colori = colori_mut(cfg);
    if !colori.contains_key(username) {
        if sola_lettura {
            assegna_colore(colori, username, preferito);
        } else {
            let vivi = modifica_config(|vivo| {
                let vivi = colori_mut(vivo);
                if !vivi.contains_key(username) {
                    assegna_colore(vivi, username, preferito);
                }
                Ok(vivi.clone())
            })?;
            let colori = colori_mut(cfg);
            for (k, v) in vivi.iter() {
                colori.insert(k.clone(), v.clone());
            }
        }
    }
    Ok(colori_mut(cfg).get(username).cloned().unwrap_or(Json::Null))
}

pub fn nome_predefinito(indirizzo: &str) -> String {
    let p = personalizzazione();
    let ind = indirizzo.to_lowercase();
    if let Some(n) = p.nomi_per_indirizzo.get(&ind) {
        return py_str_v(n);
    }
    let dominio = ind.rsplit('@').next().unwrap_or("").to_string();
    if let Some(n) = p.nomi_per_dominio.get(&dominio) {
        return py_str_v(n);
    }
    // Dominio sconosciuto: nome senza estensione, iniziale maiuscola
    let radice = dominio.split('.').next().unwrap_or("");
    if radice.is_empty() {
        return indirizzo.to_string();
    }
    let mut c = radice.chars();
    let primo = c.next().map(|x| x.to_uppercase().collect::<String>()).unwrap_or_default();
    primo + c.as_str()
}

/// Mappa indirizzo -> nome da mostrare. Gli alias scelti a mano vincono.
pub fn alias_per_account<'a>(cfg: &Config, indirizzi: impl IntoIterator<Item = &'a str>) -> IndexMap<String, String> {
    let scelti = cfg.get("account_aliases").and_then(Json::as_obj);
    let mut out = IndexMap::new();
    for ind in indirizzi {
        let scelto = scelti.and_then(|m| m.get(ind));
        let manuale = if py_bool(scelto) { js_trim(&py_str(scelto)).to_string() } else { String::new() };
        out.insert(ind.to_string(), if manuale.is_empty() { nome_predefinito(ind) } else { manuale });
    }
    out
}

/// Le aziende di un gruppo; `gruppo` c'è quando sono quelle di un gruppo della configurazione (un evento personale fatto "per" quel gruppo,
/// campo `gruppi`, ne fa parte anche lui).
#[derive(Clone, Debug, Default)]
pub struct Membri {
    pub gruppo: Option<String>,
    pub aziende: HashSet<String>,
}

impl Membri {
    pub fn has(&self, nome: &str) -> bool {
        self.aziende.contains(nome)
    }
}

/// Le aziende di un gruppo (config "gruppi_calendari"), o None se il gruppo non esiste.
pub fn membri_gruppo(cfg: &Config, nome: &str) -> Option<Membri> {
    match cfg.get("gruppi_calendari").and_then(|g| g.as_obj()).and_then(|g| g.get(nome)) {
        Some(Json::Arr(m)) => Some(Membri { gruppo: Some(nome.to_string()), aziende: m.iter().map(py_str_v).collect() }),
        _ => None,
    }
}

/// Un evento sta in un gruppo se è di una delle sue aziende, o se è un evento personale fatto per quel gruppo ("A chi").
pub fn nel_gruppo(e: &Mappa, membri: &Membri) -> bool {
    let nome = if py_bool(e.get("alias")) { e.get("alias") } else { e.get("account") };
    let nome = if py_bool(nome) { crate::py::string_js(nome) } else { String::new() };
    if membri.has(&nome) {
        return true;
    }
    let Some(gruppo) = &membri.gruppo else { return false };
    e.get("is_local") == Some(&Json::Bool(true)) && matches!(e.get("gruppi"), Some(Json::Arr(g)) if g.iter().any(|x| x.as_str() == Some(gruppo)))
}

/// Un'azienda che cambia nome resta nei suoi gruppi (senza doppioni se il nuovo nome c'era già).
pub fn rinomina_nei_gruppi(cfg: &mut Config, prima: &str, dopo: &str) {
    if prima == dopo {
        return;
    }
    let Some(Json::Obj(gruppi)) = cfg.get_mut("gruppi_calendari") else { return };
    for membri in gruppi.values_mut() {
        if let Json::Arr(m) = membri {
            if m.iter().any(|x| x.as_str() == Some(prima)) {
                let mut visti = Vec::<Json>::new();
                for x in m.iter() {
                    let y = if x.as_str() == Some(prima) { Json::Str(dopo.to_string()) } else { x.clone() };
                    if !visti.contains(&y) {
                        visti.push(y);
                    }
                }
                *m = visti;
            }
        }
    }
}

/// L'elenco dei calendari spenti, senza doppioni, come Set di JavaScript (l'ordine di inserimento).
pub fn spenti_di(cfg: &Config) -> Vec<Json> {
    let mut out: Vec<Json> = Vec::new();
    if let Some(Json::Arr(v)) = cfg.get("disabled_calendars") {
        for x in v {
            if !out.contains(x) {
                out.push(x.clone());
            }
        }
    }
    out
}

/// `new Set(disabled_calendars).has(id)`
pub fn spento(cfg: &Config, id: &str) -> bool {
    matches!(cfg.get("disabled_calendars"), Some(Json::Arr(v)) if v.iter().any(|x| x.as_str() == Some(id)))
}

/// Ordina i calendari spenti come sorted() di Python sui testi (per punto di codice); un valore che non è un testo si confronta come String().
pub fn ordina_spenti(v: &mut [Json]) {
    v.sort_by(|a, b| confronta_punti(&crate::py::string_js(Some(a)), &crate::py::string_js(Some(b))));
}

/// Accende o spegne un calendario: l'elenco di quelli spenti (disabled_calendars) resta ordinato.
pub fn imposta_acceso(cfg: &mut Config, id: &str, acceso: bool) {
    let mut spenti = spenti_di(cfg);
    let voce = Json::Str(id.to_string());
    if acceso {
        spenti.retain(|x| *x != voce);
    } else if !spenti.contains(&voce) {
        spenti.push(voce);
    }
    ordina_spenti(&mut spenti);
    cfg.insert("disabled_calendars", Json::Arr(spenti));
}

/// sorted() di Python sui testi: per punto di codice, non per unità UTF-16 come fa sort() di JavaScript.
pub fn confronta_punti(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

/// sort() di JavaScript sui testi: per unità UTF-16.
pub fn confronta_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// L'ordine di un calendario nella barra laterale, per nome mostrato: chi non è in elenco va in fondo.
pub fn ordine_calendario(alias: &str) -> usize {
    let p = personalizzazione();
    p.ordine_calendari.iter().position(|x| x == alias).unwrap_or(p.ordine_calendari.len())
}
