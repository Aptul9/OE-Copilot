//! Cosa c'è nel calendario di un link (era lib/core/condivisione-eventi.ts): gli stessi eventi che l'app mostra (raccogli, lo stesso di
//! get_events), letti a blocchi di 28 giorni come la disponibilità pubblica, e ridotti a quello che il livello del link permette. I calendari
//! ricevuti da altri (link ICS, festività, i calendari condivisi da un altro OE Copilot) non escono mai; di un evento escono gli orari e, nel
//! livello con i titoli, il titolo e il luogo. Mai la descrizione, chi organizza e chi partecipa, i link per collegarsi.

use crate::agente::calendari_di_inoltro;
use crate::caselle::{Ev, SPAZI};
use crate::condivisione::{Condivisione, AVANTI, EVENTI_MAX, INDIETRO};
use crate::disponibilita::{blocchi, sposta_data, unisci, Coppia, GIORNI_BLOCCO};
use crate::eventi::raccogli;
use crate::fuso::{chiave, iso, iso_secondi, leggi_naive, mezzanotte, naive_locale, piu_giorni, Naive};
use crate::ics_uscita::{istante_da_roma, EventoUscita};
use crate::impegni::impegni;
use crate::locali::ID_CALENDARIO;
use crate::personalizzazione::ACCOUNT_LOCALE;
use crate::py::{lunghezza_js, py_bool, string_js, taglia_js, Errore, Esito};
use crate::stato::{load_config, membri_gruppo, nel_gruppo, spento, Config, Membri};
use oe_nucleo::json::Json;
use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use std::sync::LazyLock;

/// Il lavoro di un blocco al più per volta: ognuno è una richiesta all'agente.
const BLOCCHI_INSIEME: usize = 4;
const TITOLO_MAX: usize = 250;
const LUOGO_MAX: usize = 250;
/// Quanti caratteri di un titolo o di un luogo si guardano: si taglia PRIMA di ogni espressione regolare.
const ESAME_MAX: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct EventoLetto {
    pub inizio: String,
    pub fine: String,
    pub tutto_giorno: bool,
    pub titolo: String,
    pub luogo: String,
}

pub struct Lettura {
    pub livello: String,
    pub da: String,
    pub a: String,
    pub impegni: Vec<Coppia>,
    pub eventi: Vec<EventoLetto>,
}

/// Il calendario non si può leggere adesso: chi lo chiede riprova fra poco.
fn non_disponibile(m: impl Into<String>) -> Errore {
    Errore::Altro { nome: "LetturaNonDisponibile", messaggio: m.into() }
}

fn limita(v: i64, (min, max, _): (i64, i64, i64)) -> i64 {
    v.clamp(min, max)
}

/// Dal giorno `da` al giorno `a` (escluso): i giorni indietro e avanti di un link, tenuti nei limiti.
pub fn finestra(c: &Condivisione, oggi: &str) -> (String, String) {
    let indietro = limita(c.giorni_indietro, INDIETRO);
    let avanti = limita(c.giorni_avanti, AVANTI);
    (sposta_data(oggi, -indietro), sposta_data(oggi, avanti + 1))
}

// ---- Cosa può uscire di un titolo e di un luogo ------------------------------------------------------------------------------
const SCHEMI_SENZA_BARRE: [&str; 11] = ["mailto", "tel", "sip", "sips", "callto", "skype", "msteams", "zoommtg", "webex", "facetime", "sms"];
const SITI_CONFERENZA: [&str; 15] = [
    "teams.microsoft.com",
    "teams.live.com",
    "zoom.us",
    "zoomgov.com",
    "meet.google.com",
    "webex.com",
    "gotomeeting.com",
    "gotomeet.me",
    "join.skype.com",
    "whereby.com",
    "bluejeans.com",
    "chime.aws",
    "meet.jit.si",
    "8x8.vc",
    "discord.gg",
];

fn re_i(src: &str) -> Regex {
    RegexBuilder::new(&src.replace(r"\s", SPAZI)).case_insensitive(true).build().expect("espressione fissa")
}

/// Quello che serve per entrare in una riunione a telefono o online: ID e codici, numeri da chiamare.
static DATI_DI_ACCESSO: LazyLock<Regex> = LazyLock::new(|| {
    re_i(r"(?-u:\b)(?:(?:conference|meeting|conferenza|riunione|webinar)\s*id|id\s*(?:conference|meeting|conferenza|riunione)|pass\s?code|pin|dial[\s-]*in|(?:codice|code)\s*(?:di\s*)?(?:accesso|riunione|conferenza|access|meeting)|numero\s*(?:di\s*)?(?:accesso|conferenza)|chiamami|join\s*by\s*phone)(?-u:\b)")
});
// il testo che i programmi di posta mettono nel luogo di una riunione online: dice solo che è online
static SOLO_CONFERENZA: LazyLock<Regex> = LazyLock::new(|| {
    re_i(r"^(?:(?:riunione|incontro|meeting)\s+(?:di\s+)?)?(?:microsoft\s+teams|ms\s+teams|teams|zoom|google\s+meet|webex|skype(?:\s+for\s+business)?|gotomeeting|online|virtuale)(?:\s+(?:meeting|riunione|call|chiamata|incontro))?$")
});

fn cifra(c: char) -> bool {
    c.is_ascii_digit()
}

fn lettera(c: char) -> bool {
    c.is_ascii_lowercase()
}

/// La punteggiatura che può stare intorno a una parola: ogni segno ASCII, e le virgolette e i puntini dei programmi di scrittura.
fn bordo(c: char) -> bool {
    let c = c as u32;
    c <= 47 || (58..=64).contains(&c) || (91..=96).contains(&c) || (123..=127).contains(&c) || c == 0xab || c == 0xbb || (0x2018..=0x201f).contains(&c) || c == 0x2026 || c == 0x2039 || c == 0x203a
}

fn senza_bordi(s: &str) -> &str {
    s.trim_matches(bordo)
}

/// Un nome di sito: etichette di lettere, cifre, trattini e lettere non ASCII separate da punti, l'ultima che comincia con una lettera; o un IPv4.
fn nome_di_sito(host: &str) -> bool {
    let etichette: Vec<&str> = host.split('.').collect();
    if etichette.len() < 2 {
        return false;
    }
    let mut tutte_cifre = true;
    for e in &etichette {
        if e.is_empty() {
            return false;
        }
        for c in e.chars() {
            if !(cifra(c) || lettera(c) || c == '-' || c == '_' || c as u32 > 127) {
                return false;
            }
            if !cifra(c) {
                tutte_cifre = false;
            }
        }
    }
    let ultima = etichette[etichette.len() - 1].chars().next().unwrap_or(' ');
    lettera(ultima) || ultima as u32 > 127 || (etichette.len() == 4 && tutte_cifre)
}

/// Un sito: www.; `sito/percorso` senza schema, anche con :porta; il nome di un servizio di riunioni anche da solo.
fn e_un_sito(s: &str) -> bool {
    if s.starts_with("www.") {
        return true;
    }
    if let Some(barra) = s.find('/').filter(|b| *b > 0) {
        if nome_di_sito(s[..barra].split(':').next().unwrap_or("")) {
            return true;
        }
    }
    let host = &s[..s.find(['/', '?', '#', ':']).unwrap_or(s.len())];
    SITI_CONFERENZA.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

fn incollata(c: char) -> bool {
    ":;,=()[]<>\"'|".contains(c)
}

/// Una parola che è, o porta a, un indirizzo: esce tutta la parola, fino allo spazio.
fn e_un_indirizzo(parola: &str, anche_la_posta: bool) -> bool {
    let p = parola.to_lowercase();
    if p.contains("://") {
        return true;
    }
    let s = senza_bordi(&p);
    if SCHEMI_SENZA_BARRE.iter().any(|schema| s.starts_with(&format!("{schema}:"))) {
        return true;
    }
    if anche_la_posta {
        if let Some(ch) = p.find('@').filter(|c| *c > 0) {
            if p[ch..].find('.').is_some_and(|x| x > 1) {
                return true;
            }
        }
    }
    if e_un_sito(s) {
        return true;
    }
    let pezzi: Vec<&str> = p.split(incollata).filter(|x| !x.is_empty()).collect();
    let spezzata = p.contains(incollata);
    spezzata && (pezzi.len() > 1 || p.split(incollata).count() > 1) && p.split(incollata).any(|x| e_un_sito(senza_bordi(x)))
}

/// Il testo a una riga sola, senza caratteri di controllo né invisibili e senza le parole che sono indirizzi. Una passata, tempo lineare.
pub fn pulisci_testo(testo: &str, anche_la_posta: bool) -> String {
    let mut pulito = String::with_capacity(testo.len());
    let mut in_controllo = false;
    for c in testo.chars() {
        if matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}') {
            if !in_controllo {
                pulito.push(' ');
            }
            in_controllo = true;
            continue;
        }
        in_controllo = false;
        if matches!(c, '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}') {
            continue;
        }
        pulito.push(c);
    }
    pulito.split(crate::py::spazio_js).filter(|p| !p.is_empty() && !e_un_indirizzo(p, anche_la_posta)).collect::<Vec<_>>().join(" ")
}

/// Toglie da capo e da coda spazi e separatori (, ; : | / -).
fn rifilato(s: &str) -> &str {
    s.trim_matches([' ', ',', ';', ':', '|', '/', '-'])
}

fn tagliato(testo: &str, max: usize) -> String {
    if lunghezza_js(testo) > max {
        crate::py::js_trim_end(&testo.chars().take(max).collect::<String>()).to_string()
    } else {
        testo.to_string()
    }
}

fn testo_di(v: Option<&Json>) -> String {
    match v {
        None | Some(Json::Null) => String::new(),
        x => string_js(x),
    }
}

/// Il titolo come esce nel livello con i titoli: senza link, a una riga sola e non troppo lungo.
pub fn titolo_esportabile(titolo: Option<&Json>) -> String {
    let t = pulisci_testo(taglia_js(&testo_di(titolo), ESAME_MAX), false);
    let t = tagliato(&t, TITOLO_MAX);
    if t.is_empty() { "(senza titolo)".to_string() } else { t }
}

/// Il luogo come esce nel livello con i titoli: senza link né indirizzi di posta, e vuoto se resta solo l'etichetta di una riunione online o
/// se parla di ID, codici, PIN o numeri per collegarsi.
pub fn luogo_esportabile(luogo: Option<&Json>) -> String {
    let p = pulisci_testo(taglia_js(&testo_di(luogo), ESAME_MAX), true).replace(" ,", ",").replace(" ;", ";").replace(" :", ":");
    let t = rifilato(&p);
    if DATI_DI_ACCESSO.is_match(t) || SOLO_CONFERENZA.is_match(t) { String::new() } else { tagliato(t, LUOGO_MAX) }
}

// ---- La lettura ---------------------------------------------------------------------------------------------------------------
fn data(n: &Naive) -> String {
    crate::fuso::data_testo(n)
}

fn fine_o_inizio(e: &Ev) -> String {
    if py_bool(e.get("end")) { string_js(e.get("end")) } else { string_js(e.get("start")) }
}

/// Un giorno intero come date AAAA-MM-GG con la fine esclusa.
fn giorno_intero(e: &Ev) -> Option<(String, String)> {
    let ini = naive_locale(&string_js(e.get("start"))).ok()?;
    let fin = naive_locale(&fine_o_inizio(e)).ok()?;
    let inizio = data(&ini);
    let mezzanotte_fine = fin.h == 0 && fin.mi == 0 && fin.s == 0 && fin.us == 0;
    let mut fine = if mezzanotte_fine && data(&fin) > inizio { data(&fin) } else { sposta_data(&data(&fin), 1) };
    if fine <= inizio {
        fine = sposta_data(&inizio, 1);
    }
    Some((inizio, fine))
}

/// Solo quello che nasce qui: una casella letta dall'agente o un evento personale.
fn nostro(e: &Ev) -> bool {
    e.get("remoto") == Some(&Json::Bool(true)) || e.get("is_local") == Some(&Json::Bool(true))
}

fn eventi_del_blocco(eventi: &[Ev], membri: Option<&Membri>) -> Vec<EventoLetto> {
    let mut out = Vec::new();
    for e in eventi {
        if membri.is_some_and(|m| !nel_gruppo(e, m)) || !py_bool(e.get("start")) {
            continue;
        }
        let titolo = titolo_esportabile(e.get("title"));
        let luogo = luogo_esportabile(e.get("location"));
        if py_bool(e.get("allDay")) {
            if let Some((inizio, fine)) = giorno_intero(e) {
                out.push(EventoLetto { inizio, fine, tutto_giorno: true, titolo, luogo });
            }
            continue;
        }
        let (Ok(ini), Ok(fin)) = (naive_locale(&string_js(e.get("start"))), naive_locale(&fine_o_inizio(e))) else { continue };
        let fine = if chiave(&fin) > chiave(&ini) { fin } else { ini };
        out.push(EventoLetto { inizio: iso_secondi(&ini), fine: iso_secondi(&fine), tutto_giorno: false, titolo, luogo });
    }
    out
}

/// Le aziende che il link pubblica e che questa lettura dà in errore: senza i loro eventi il calendario sarebbe incompleto.
fn aziende_in_errore(cfg: &mut Config, meta: &crate::agente::Meta, errori: &[String], membri: Option<&Membri>) -> Esito<Vec<String>> {
    let dentro = |nome: &str, accesa: bool| match membri {
        None => accesa,
        Some(m) => m.has(nome),
    };
    let mut guaste: Vec<String> = Vec::new();
    let mut metti = |n: String| {
        if !guaste.contains(&n) {
            guaste.push(n);
        }
    };
    if let Some(Json::Obj(stati)) = meta.get("stato_aziende") {
        for (azienda, s) in stati.iter() {
            let accesa = py_bool(meta.get("aziende").and_then(|a| a.get(azienda)));
            if s.get("stato").and_then(Json::as_str) == Some("errore") && dentro(azienda, accesa) {
                metti(azienda.clone());
            }
        }
    }
    // le aziende inoltrate hanno le copie delle riunioni nel calendario di un'altra casella: se quella non si legge, mancano
    for c in calendari_di_inoltro(cfg)? {
        let nome = if py_bool(c.get("alias")) { string_js(c.get("alias")) } else { string_js(c.get("account")) };
        if c.get("stato").and_then(Json::as_str) == Some("errore") && dentro(&nome, py_bool(c.get("enabled"))) {
            metti(nome);
        }
    }
    // gli eventi personali stanno su questo disco: si rompono solo se uno di loro non si legge
    if errori.iter().any(|e| e.starts_with(&format!("{ACCOUNT_LOCALE}:"))) && dentro(ACCOUNT_LOCALE, !spento(cfg, ID_CALENDARIO)) {
        metti(ACCOUNT_LOCALE.to_string());
    }
    Ok(guaste)
}

struct Blocco {
    impegni: Vec<Coppia>,
    eventi: Vec<EventoLetto>,
}

async fn leggi_blocco(mut cfg: Config, inizio_blocco: String, livello: String, membri: Option<Membri>) -> Esito<Blocco> {
    let mut p = inizio_blocco.split('-').map(|x| x.parse::<i64>().unwrap_or(0));
    let (a, m, g) = (p.next().unwrap_or(0), p.next().unwrap_or(1), p.next().unwrap_or(1));
    let inizio = mezzanotte(a, m, g);
    // niente copia vecchia su disco, e niente scritture; i calendari ricevuti da altri non si leggono nemmeno
    let (letti, errori, meta) = raccogli(&mut cfg, &iso(&inizio), &iso(&piu_giorni(&inizio, GIORNI_BLOCCO)), false, false).await?;
    // l'agente non ha risposto: senza dati non si pubblica un calendario vuoto, che per chi lo importa vorrebbe dire "libero"
    if py_bool(meta.get("outlook_starting")) {
        return Err(non_disponibile("l'agente non risponde"));
    }
    let guaste = aziende_in_errore(&mut cfg, &meta, &errori, membri.as_ref())?;
    if !guaste.is_empty() {
        return Err(non_disponibile(format!("non si leggono: {}", guaste.join(", "))));
    }
    let eventi: Vec<Ev> = letti.into_iter().filter(nostro).collect();
    Ok(if livello == "occupato" {
        Blocco { impegni: impegni(&eventi, (a, m, g), GIORNI_BLOCCO, membri.as_ref()), eventi: vec![] }
    } else {
        Blocco { impegni: vec![], eventi: eventi_del_blocco(&eventi, membri.as_ref()) }
    })
}

/// Quello che il link pubblica oggi. Con "tutti" valgono gli interruttori della barra laterale, con un gruppo contano le aziende del gruppo.
/// Errore LetturaNonDisponibile se l'agente non risponde, se un'azienda pubblicata non si legge, se il gruppo non c'è o se gli eventi sono troppi.
pub async fn leggi_per_condivisione(link: &Condivisione, oggi: &str) -> Esito<Lettura> {
    let (da, a) = finestra(link, oggi);
    let config = load_config()?;
    let mut membri = None;
    if link.ambito == "gruppo" {
        membri = Some(membri_gruppo(&config, &link.gruppo).ok_or_else(|| non_disponibile("gruppo non trovato"))?);
    }
    // una copia di lavoro che non va su disco (le sorgenti salvano la configurazione quando assegnano un colore)
    let mut cfg = config.clone();
    cfg.insert("_sola_lettura", true);
    if membri.is_some() {
        cfg.insert("disabled_calendars", Json::Arr(vec![]));
    }
    let inizi = blocchi(&da, &a);
    let mut letti: Vec<Blocco> = Vec::new();
    for gruppo in inizi.chunks(BLOCCHI_INSIEME) {
        let lavori = gruppo.iter().map(|b| leggi_blocco(cfg.clone(), b.clone(), link.livello.clone(), membri.clone()));
        for b in futures_util::future::join_all(lavori).await {
            letti.push(b?);
        }
    }
    let inizio_finestra = format!("{da}T00:00:00");
    let fine_finestra = format!("{a}T00:00:00");
    // un impegno lungo a cavallo di due blocchi arriva due volte: si uniscono, e si tengono quelli che toccano la finestra
    let tutti: Vec<Coppia> = letti.iter().flat_map(|b| b.impegni.iter().cloned()).collect();
    let occupati: Vec<Coppia> = unisci(&tutti).into_iter().filter(|[i, f]| *f > inizio_finestra && *i < fine_finestra).collect();
    let mut visti = HashSet::new();
    let mut eventi: Vec<EventoLetto> = letti
        .into_iter()
        .flat_map(|b| b.eventi)
        .filter(|e| {
            let dentro = if e.tutto_giorno { e.fine > da && e.inizio < a } else { (e.fine > inizio_finestra || (e.fine == e.inizio && e.inizio >= inizio_finestra)) && e.inizio < fine_finestra };
            let k = Json::Arr(vec![Json::Str(e.titolo.to_lowercase()), Json::Str(e.inizio.clone()), Json::Str(e.fine.clone()), Json::Bool(e.tutto_giorno)]).in_js();
            dentro && visti.insert(k)
        })
        .collect();
    let u = crate::stato::confronta_utf16;
    eventi.sort_by(|x, y| u(&x.inizio, &y.inizio).then_with(|| u(&x.fine, &y.fine)).then_with(|| u(&x.titolo, &y.titolo)));
    // tagliare in silenzio vorrebbe dire pubblicare i primi eventi e dare per liberi i mesi dopo: meglio non rispondere
    if occupati.len() + eventi.len() > EVENTI_MAX {
        return Err(non_disponibile(format!("più di {EVENTI_MAX} eventi nella finestra: servono meno giorni")));
    }
    Ok(Lettura { livello: link.livello.clone(), da, a, impegni: occupati, eventi })
}

/// Gli eventi di una lettura come li scrive il calendario: gli orari diventano istanti UTC, i giorni interi restano date.
pub fn in_eventi_uscita(l: &Lettura) -> Esito<Vec<EventoUscita>> {
    let ora = |t: &str| -> Esito<i64> { Ok(istante_da_roma(&leggi_naive(t)?)) };
    if l.livello == "occupato" {
        return l.impegni.iter().map(|[i, f]| Ok(EventoUscita::Ora { inizio: ora(i)?, fine: ora(f)?, titolo: String::new(), luogo: String::new() })).collect();
    }
    l.eventi
        .iter()
        .map(|e| {
            Ok(if e.tutto_giorno {
                EventoUscita::Giorno { inizio: e.inizio.clone(), fine: e.fine.clone(), titolo: e.titolo.clone(), luogo: e.luogo.clone() }
            } else {
                EventoUscita::Ora { inizio: ora(&e.inizio)?, fine: ora(&e.fine)?, titolo: e.titolo.clone(), luogo: e.luogo.clone() }
            })
        })
        .collect()
}

#[cfg(test)]
mod prove {
    use super::*;

    fn t(s: &str) -> Json {
        Json::Str(s.to_string())
    }

    #[test]
    fn titoli_e_luoghi_senza_link() {
        assert_eq!(titolo_esportabile(Some(&t("Riunione https://teams.microsoft.com/l/x con Alfa"))), "Riunione con Alfa");
        assert_eq!(titolo_esportabile(Some(&t("Call Meet:meet.google.com/abc-defg oggi"))), "Call oggi");
        assert_eq!(titolo_esportabile(Some(&t("vedi (bit.ly/x) e www.esempio.it"))), "vedi e");
        assert_eq!(titolo_esportabile(Some(&t("\u{200b}\u{202e}"))), "(senza titolo)");
        assert_eq!(titolo_esportabile(Some(&t("Pranzo con mario@alfa.it"))), "Pranzo con mario@alfa.it");
        assert_eq!(luogo_esportabile(Some(&t("Sala 2, mario@alfa.it"))), "Sala 2");
        assert_eq!(luogo_esportabile(Some(&t("Microsoft Teams Meeting"))), "");
        assert_eq!(luogo_esportabile(Some(&t("Dial-in 02 1234, PIN 5678"))), "");
        assert_eq!(luogo_esportabile(Some(&t("Pinerolo, sala A"))), "Pinerolo, sala A");
        assert_eq!(luogo_esportabile(Some(&t("Via Roma 1 - tel:+39021234"))), "Via Roma 1");
    }

    #[test]
    fn i_testi_lunghi_si_leggono_in_fretta() {
        let lungo = "a.b/".repeat(50_000);
        let inizio = std::time::Instant::now();
        let _ = pulisci_testo(&lungo, true);
        assert!(inizio.elapsed().as_millis() < 2000);
    }
}
