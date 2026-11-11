//! Il nome e il logo di ogni account Microsoft come li mostra la barra del calendario (alias, nomi_per_dominio, loghi), e le aziende inoltrate
//! con il nome della casella che ne riceve le copie: la stessa riga in tutte e due le app (era lib/core/etichette.ts). Il calendario li
//! prende con l'elenco degli account; a TeamsRelay arrivano dall'agente, a cui si mandano con i loghi come data URL quando cambiano.

use crate::agente::invia;
use crate::gateway::{cartelle_loghi, leggi_logo};
use crate::loghi::interfaccia_con_loghi;
use crate::personalizzazione::personalizzazione;
use crate::py::{js_trim, string_js};
use crate::stato::{alias_per_account, load_config};
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use sha2::{Digest, Sha256};
use std::cell::RefCell;

const LOGO_MAX: usize = 200_000;

pub struct Etichette {
    pub names: IndexMap<String, String>,
    pub logos: IndexMap<String, String>,
    pub forwarded: IndexMap<String, String>,
}

impl Etichette {
    pub fn json(&self) -> Json {
        let m = |x: &IndexMap<String, String>| Json::Obj(x.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect::<Mappa>());
        ogg! { "names": m(&self.names), "logos": m(&self.logos), "forwarded": m(&self.forwarded) }
    }
}

/// I loghi di personalizzazione.json e della finestra Calendari, per nome mostrato: il file in logos/ senza la versione.
fn file_loghi() -> IndexMap<String, String> {
    let interfaccia = interfaccia_con_loghi(&personalizzazione().interfaccia).unwrap_or_default();
    let mut out = IndexMap::new();
    if let Some(Json::Obj(l)) = interfaccia.get("loghi") {
        for (nome, v) in l.iter() {
            if let Some(f) = v.as_str().and_then(|s| s.strip_prefix("logos/")) {
                out.insert(nome.clone(), f.to_string());
            }
        }
    }
    out
}

fn senza_doppioni(v: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// Le etichette degli indirizzi dati e di quelli con un nome in nomi_per_indirizzo (un account Teams su un altro computer, che l'agente non
/// conosce, ha così il nome e il logo della sua azienda in TeamsRelay): i loghi come indirizzi di questo sito (/logos/...).
pub fn etichette_account(indirizzi: &[String]) -> Etichette {
    let cfg = load_config().unwrap_or_default();
    let p = personalizzazione();
    let norma = |i: &str| js_trim(i).to_lowercase();
    let nominati = senza_doppioni(indirizzi.iter().map(|i| norma(i)).chain(p.nomi_per_indirizzo.keys().map(|i| norma(i))));
    let tutti = senza_doppioni(nominati.iter().cloned().chain(p.aziende_inoltrate.values().map(|d| norma(&d.via))).filter(|i| i.contains('@')));
    let alias = alias_per_account(&cfg, tutti.iter().map(String::as_str));
    let names: IndexMap<String, String> = nominati.iter().filter_map(|i| alias.get(i).filter(|a| !a.is_empty()).map(|a| (i.clone(), a.clone()))).collect();
    let usati = senza_doppioni(names.values().cloned());
    let file = file_loghi();
    let logos = usati.iter().filter_map(|n| file.get(n).map(|f| (n.clone(), format!("/logos/{f}")))).collect();
    let forwarded = p
        .aziende_inoltrate
        .iter()
        .filter(|(n, _)| usati.contains(n))
        .map(|(n, d)| (n.clone(), alias.get(&norma(&d.via)).filter(|a| !a.is_empty()).cloned().unwrap_or_else(|| d.via.clone())))
        .collect();
    Etichette { names, logos, forwarded }
}

#[derive(Default)]
struct Memoria {
    firma: String,
    letto: Option<i64>,
}

thread_local! {
    static MEMORIA: RefCell<Memoria> = RefCell::new(Memoria::default());
}

/// Manda le etichette all'agente se sono cambiate dall'ultima volta, con i loghi come data URL. Un errore non ferma chi chiama.
pub async fn manda_etichette(e: &Etichette) {
    let mut logos = Mappa::new();
    for (nome, url) in &e.logos {
        let file = url.strip_prefix("/logos/").unwrap_or(url).split('?').next().unwrap_or("");
        let Some(file) = crate::auth::cookie::decodifica_componente(file) else {
            eprintln!("etichette: non mandate URI malformed");
            return;
        };
        if let Some(logo) = leggi_logo(&file, &cartelle_loghi()) {
            if logo.dati.len() <= LOGO_MAX && logo.tipo.starts_with("image/") {
                logos.insert(nome.clone(), format!("data:{};base64,{}", logo.tipo, crate::b64::codifica(&logo.dati)));
            }
        }
    }
    let mut corpo = match e.json() {
        Json::Obj(m) => m,
        _ => Mappa::new(),
    };
    corpo.insert("logos", Json::Obj(logos));
    let corpo = Json::Obj(corpo);
    let firma: String = Sha256::digest(corpo.in_js().as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    if MEMORIA.with(|m| m.borrow().firma == firma) {
        return;
    }
    match invia("/api/auth/shared/labels", &corpo).await {
        Ok(r) if r.get("ok") == Some(&Json::Bool(true)) => MEMORIA.with(|m| m.borrow_mut().firma = firma),
        Ok(r) => eprintln!("etichette: l'agente non le ha prese ({})", match r.get("errore") {
            None | Some(Json::Null) => String::new(),
            x => string_js(x),
        }),
        Err(e) => eprintln!("etichette: non mandate {}", e.messaggio()),
    }
}

/// Gli indirizzi dell'elenco degli account dell'agente (/api/auth/shared): le caselle e gli account Teams con il solo browser.
pub fn indirizzi_elenco(r: &Json) -> Vec<String> {
    let voci = crate::py::lista(r.get("accounts")).iter().chain(crate::py::lista(r.get("teamsOnly")));
    voci.filter_map(|v| match (v.get("account"), v.get("email")) {
        (Some(Json::Str(a)), _) => Some(a.clone()),
        (_, Some(Json::Str(e))) => Some(e.clone()),
        _ => None,
    })
    .filter(|s| !s.is_empty())
    .collect()
}

/// Al più ogni 10 minuti, da chi usa il calendario: rilegge l'elenco e manda le etichette se sono cambiate.
pub async fn aggiorna_etichette(adesso_ms: i64) {
    let da_leggere = MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        if m.letto.is_some_and(|l| adesso_ms - l < 600_000) {
            return false;
        }
        m.letto = Some(adesso_ms);
        true
    });
    if !da_leggere {
        return;
    }
    // l'agente non risponde: si riprova tra 10 minuti
    if let Ok(r) = crate::agente::chiedi("/api/auth/shared", &[]).await {
        if r.get("ok") == Some(&Json::Bool(true)) {
            manda_etichette(&etichette_account(&indirizzi_elenco(&r))).await;
        }
    }
}
