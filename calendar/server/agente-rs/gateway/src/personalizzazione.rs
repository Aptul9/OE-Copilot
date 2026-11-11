//! Nomi, domini e caselle di chi usa l'app non stanno nel codice ma in personalizzazione.json: qui si leggono e si derivano le stesse
//! costanti (era lib/core/personalizzazione.ts). Senza il file l'app funziona lo stesso e i calendari prendono il nome dal dominio della casella.

use crate::py::{elenco_testi, js_trim, minuscole, py_bool, py_str_v};
use indexmap::IndexMap;
use oe_nucleo::json::{Json, Mappa};
use regex::{Regex, RegexBuilder};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

pub const ACCOUNT_LOCALE: &str = "Eventi personali";
pub const FESTIVITA_URL: &str = "https://calendar.google.com/calendar/ical/it.italian%23holiday%40group.v.calendar.google.com/public/basic.ics";
pub const FESTIVITA_ID: &str = "builtin::festivita-it";
pub const FESTIVITA_NOME: &str = "Festività";
pub const COLORE_FESTIVITA: &str = "#8a8a8a";

/// Tinte assegnate agli account, in quest'ordine: più voci del numero degli account, se no due caselle hanno lo stesso colore.
pub const COLOR_PALETTE: [&str; 25] = [
    "#2563eb", "#16a34a", "#db2777", "#ea580c", "#7c3aed", "#0891b2", "#ca8a04", "#dc2626", "#4d7c0f", "#9333ea", "#0f766e", "#b45309", "#1e40af", "#be123c", "#525252", "#65a30d", "#0d9488",
    "#c026d3", "#f59e0b", "#6366f1", "#e11d48", "#15803d", "#a16207", "#7e22ce", "#0369a1",
];

pub struct Inoltrata {
    pub via: String,
    pub prefisso: Regex,
    pub colore: Json,
}

pub struct Personalizzazione {
    pub nomi_per_indirizzo: IndexMap<String, Json>,
    pub nomi_per_dominio: IndexMap<String, Json>,
    pub caselle_di_colleghi: BTreeSet<String>,
    pub ordine_calendari: Vec<String>,
    pub domini_aziendali: BTreeSet<String>,
    pub azienda_per_cartella: Vec<(String, Json)>,
    pub aziende_inoltrate: IndexMap<String, Inoltrata>,
    pub account_nascosti: BTreeSet<String>,
    pub mostra_tutto_il_giorno: bool,
    pub interfaccia: Mappa,
}

const SPAZI_JS: &str = r"\t\n\x0B\x0C\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}";

/// \d, \w e \s di JavaScript (ASCII le prime due, gli spazi di ECMAScript la terza) scritti per il crate regex, che li intende Unicode.
fn classi_js(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    let mut in_classe = false;
    let mut c = src.chars().peekable();
    while let Some(x) = c.next() {
        match x {
            '\\' => {
                let Some(y) = c.next() else {
                    out.push('\\');
                    break;
                };
                let (dentro, fuori) = match y {
                    'd' => ("0-9", "[0-9]"),
                    'D' => ("", "[^0-9]"),
                    'w' => ("A-Za-z0-9_", "[A-Za-z0-9_]"),
                    'W' => ("", "[^A-Za-z0-9_]"),
                    's' => (SPAZI_JS, ""),
                    'S' => ("", ""),
                    _ => ("", ""),
                };
                if y == 's' && !in_classe {
                    out.push_str(&format!("[{SPAZI_JS}]"));
                } else if y == 'S' && !in_classe {
                    out.push_str(&format!("[^{SPAZI_JS}]"));
                } else if in_classe && !dentro.is_empty() {
                    out.push_str(dentro);
                } else if !in_classe && !fuori.is_empty() {
                    out.push_str(fuori);
                } else {
                    out.push('\\');
                    out.push(y);
                }
            }
            '[' if !in_classe => {
                in_classe = true;
                out.push('[');
                if c.peek() == Some(&'^') {
                    out.push(c.next().unwrap_or('^'));
                }
                if c.peek() == Some(&']') {
                    out.push_str("\\]");
                    c.next();
                }
            }
            ']' if in_classe => {
                in_classe = false;
                out.push(']');
            }
            '[' if in_classe => out.push_str("\\["),
            _ => out.push(x),
        }
    }
    out
}

/// Un'espressione regolare di Python come la intende JavaScript: i nomi dei gruppi e le opzioni in testa; il resto è uguale. Ancorata
/// all'inizio (re.match), senza distinzione fra maiuscole e minuscole.
pub fn regex_di_python(sorgente: &str) -> Result<Regex, String> {
    let mut src = sorgente.replace("(?P<", "(?<");
    let mut multi = false;
    let mut punto = false;
    if let Some(resto) = src.strip_prefix("(?") {
        let fine = resto.find(')');
        if let Some(f) = fine {
            let flag = &resto[..f];
            if !flag.is_empty() && flag.chars().all(|c| "aiLmsux".contains(c)) {
                multi = flag.contains('m');
                punto = flag.contains('s');
                src = resto[f + 1..].to_string();
            }
        }
    }
    if src.contains("(?P=") {
        return Err("i riferimenti a un gruppo non sono ammessi".to_string());
    }
    RegexBuilder::new(&format!("^(?:{})", classi_js(&src))).case_insensitive(true).multi_line(multi).dot_matches_new_line(punto).build().map_err(|e| e.to_string())
}

fn leggi(percorso: &PathBuf) -> Mappa {
    let testo = match crate::dati::leggi_testo(percorso) {
        Ok(Some(t)) => t,
        Ok(None) => return Mappa::new(),
        Err(e) => {
            eprintln!("personalizzazione non leggibile, uso i valori predefiniti {e}");
            return Mappa::new();
        }
    };
    match Json::analizza(&testo) {
        Ok(Json::Obj(m)) => m,
        Ok(_) => {
            eprintln!("personalizzazione: atteso un oggetto JSON, file ignorato");
            Mappa::new()
        }
        Err(e) => {
            eprintln!("personalizzazione non leggibile, uso i valori predefiniti SyntaxError: {e}");
            Mappa::new()
        }
    }
}

fn mappa(v: Option<&Json>) -> IndexMap<String, Json> {
    let mut out = IndexMap::new();
    if let Some(Json::Obj(m)) = v {
        for (k, x) in m.iter() {
            out.insert(js_trim(k).to_lowercase(), x.clone());
        }
    }
    out
}

fn derivate(p: &Mappa) -> Personalizzazione {
    let mut inoltrate = IndexMap::new();
    if let Some(Json::Obj(m)) = p.get("aziende_inoltrate") {
        for (nome, d) in m.iter() {
            if nome.starts_with('_') {
                continue;
            }
            let voce = (|| -> Result<Inoltrata, String> {
                let Json::Obj(d) = d else { return Err("TypeError: voce non valida".to_string()) };
                let (Some(via), Some(prefisso)) = (d.get("via"), d.get("prefisso")) else { return Err("Error: manca via o prefisso".to_string()) };
                let testo = crate::py::string_js(Some(prefisso));
                Ok(Inoltrata { via: py_str_v(via), prefisso: regex_di_python(&testo).map_err(|e| format!("SyntaxError: {e}"))?, colore: d.get("colore").cloned().unwrap_or(Json::Null) })
            })();
            match voce {
                Ok(v) => {
                    inoltrate.insert(nome.clone(), v);
                }
                Err(e) => eprintln!("personalizzazione: azienda inoltrata {} non valida, ignorata {e}", oe_nucleo::json::stringa_js(nome)),
            }
        }
    }
    let ordine = p.get("ordine_calendari");
    let ordine_calendari = if py_bool(ordine) { elenco_testi(ordine) } else { vec![ACCOUNT_LOCALE.to_string(), FESTIVITA_NOME.to_string()] };
    Personalizzazione {
        nomi_per_indirizzo: mappa(p.get("nomi_per_indirizzo")),
        nomi_per_dominio: mappa(p.get("nomi_per_dominio")),
        caselle_di_colleghi: minuscole(p.get("caselle_di_colleghi")),
        ordine_calendari,
        domini_aziendali: minuscole(p.get("domini_aziendali")),
        azienda_per_cartella: match p.get("azienda_per_cartella") {
            Some(Json::Obj(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        },
        aziende_inoltrate: inoltrate,
        account_nascosti: minuscole(p.get("account_nascosti")),
        mostra_tutto_il_giorno: py_bool(p.get("mostra_tutto_il_giorno")),
        interfaccia: match p.get("interfaccia") {
            Some(Json::Obj(m)) => m.clone(),
            _ => Mappa::new(),
        },
    }
}

thread_local! {
    static MEMORIA: RefCell<HashMap<PathBuf, (String, Rc<Personalizzazione>)>> = RefCell::new(HashMap::new());
}

fn firma(percorso: &PathBuf) -> String {
    match std::fs::metadata(percorso) {
        Ok(m) => {
            let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            format!("{t}:{}", m.len())
        }
        Err(_) => "assente".to_string(),
    }
}

/// La personalizzazione: si rilegge solo se il file è cambiato (data e dimensione), così una modifica vale subito, senza il riavvio che
/// chiedeva il servizio Python.
pub fn personalizzazione() -> Rc<Personalizzazione> {
    let percorso = crate::config::var("OE_PERSONALIZZAZIONE").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| crate::dati::nelle_dati(&["personalizzazione.json"]));
    let f = firma(&percorso);
    if let Some(v) = MEMORIA.with(|m| m.borrow().get(&percorso).filter(|(x, _)| *x == f).map(|(_, v)| v.clone())) {
        return v;
    }
    let valore = Rc::new(derivate(&if f == "assente" { Mappa::new() } else { leggi(&percorso) }));
    MEMORIA.with(|m| m.borrow_mut().insert(percorso, (f, valore.clone())));
    valore
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn il_prefisso_si_ancora_e_ignora_le_maiuscole() {
        let r = regex_di_python(r"Copia\s*:\s*").unwrap();
        assert_eq!(r.find("copia : riunione").map(|m| m.end()), Some(8));
        assert!(r.find("x Copia:").is_none());
        // \d di JavaScript: solo le cifre ASCII
        assert!(regex_di_python(r"\d").unwrap().find("٣").is_none());
        assert!(regex_di_python("(non chiusa").is_err());
        assert!(regex_di_python(r"(?i)[\s:]+").unwrap().find("\u{feff}:").is_some());
    }
}
