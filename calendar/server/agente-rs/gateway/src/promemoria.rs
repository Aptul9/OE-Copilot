//! I promemoria delle riunioni (era lib/core/promemoria.ts). Le impostazioni (voce `promemoria` della configurazione) sono una sola per
//! tutti: quando avvisare, cosa scrivere, il suono e i due avvisi dell'app per Windows, che le rilegge a ogni giro. Il server manda i
//! promemoria dei canali configurati (le notifiche dei browser iscritti): a ogni giro legge gli eventi che stanno per iniziare dalla stessa
//! strada di get_events e manda quello che serve. Quello che è già partito sta nella tabella promemoria_inviati di gateway.db, così un
//! riavvio del gateway non rimanda niente.

use crate::archivio::{archivio, load_config, modifica_config, Config};
use crate::caselle::Ev;
use crate::eventi::{raccogli, tutti_accesi};
use crate::fuso::{chiave as microsecondi, da_istante, da_secondi, iso, iso_secondi, naive_locale, piu_giorni, utc_js, Naive};
use crate::orologio::adesso;
use crate::promemoria_push::{self as push, EsitoInvio};
use crate::py::{py_bool, string_js, Esito};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use rusqlite::{params, OptionalExtension};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

pub const TENTATIVI_MAX: i64 = 5;
pub const CONSERVA_GIORNI: i64 = 14;
pub const MINUTI_MAX: i64 = 1440;
pub const MINUTI_VOCI: usize = 8;
pub const TESTO_PROVA: &str = "Prova dei promemoria";
// si legge un po' oltre il minuto più lontano: un evento a cavallo del limite non sfugge fra due giri
const MARGINE_MIN: i64 = 2;
const ATTESA_MAX_S: f64 = 900.0;
// lo stesso problema si scrive nel log una volta ogni tanto, non a ogni giro
const LOG_OGNI_MS: i64 = 10 * 60_000;
const INTERVALLO_PREDEFINITO_S: u64 = 30;
const PRIMO_GIRO_MS: u64 = 10_000;

/// I suoni di sistema che l'app per Windows può chiedere alla notifica: sul server si sceglie, ma li usa solo l'app.
pub fn suoni() -> Vec<String> {
    let dieci = |nome: &str| std::iter::once(nome.to_string()).chain((2..=10).map(move |n| format!("{nome}{n}"))).collect::<Vec<_>>();
    ["Default", "Reminder", "Mail", "IM", "SMS"].iter().map(|s| s.to_string()).chain(dieci("Alarm")).chain(dieci("Call")).collect()
}

// ---- Le impostazioni -----------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Impostazioni {
    pub minuti: Vec<i64>,
    pub contenuto: String,
    pub suono: String,
    pub notifica_windows: bool,
    pub comparsa: bool,
}

impl Impostazioni {
    pub fn json(&self) -> Json {
        ogg! {
            "minuti": Json::Arr(self.minuti.iter().map(|m| Json::Num(*m as f64)).collect()),
            "contenuto": self.contenuto.clone(),
            "suono": self.suono.clone(),
            "notifica_windows": self.notifica_windows,
            "comparsa": self.comparsa,
        }
    }
}

/// 0 vuol dire "all'inizio": l'avviso parte nell'ultimo minuto prima dell'inizio.
pub fn soglia_ms(minuti: i64) -> i64 {
    minuti.max(1) * 60_000
}

fn minuti_validi(v: Option<&Json>) -> Option<Vec<i64>> {
    let Some(Json::Arr(a)) = v else { return None };
    let mut out = Vec::new();
    for m in a {
        let Json::Num(n) = m else { return None };
        if n.fract() != 0.0 || !n.is_finite() || *n < 0.0 || *n > MINUTI_MAX as f64 {
            return None;
        }
        let n = *n as i64;
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out.sort_unstable();
    (1..=MINUTI_VOCI).contains(&out.len()).then_some(out)
}

/// Le impostazioni dallo stato, sempre usabili: un valore che non va cede il posto al predefinito.
pub fn leggi_impostazioni(cfg: &Config) -> Impostazioni {
    let vuota = Mappa::new();
    let g = cfg.get("promemoria").and_then(Json::as_obj).unwrap_or(&vuota);
    Impostazioni {
        minuti: minuti_validi(g.get("minuti")).unwrap_or_else(|| vec![10]),
        contenuto: if g.get("contenuto").and_then(Json::as_str) == Some("minimo") { "minimo" } else { "completo" }.to_string(),
        suono: g.get("suono").and_then(Json::as_str).filter(|s| suoni().iter().any(|x| x == s)).unwrap_or("Default").to_string(),
        notifica_windows: g.get("notifica_windows") != Some(&Json::Bool(false)),
        comparsa: g.get("comparsa") != Some(&Json::Bool(false)),
    }
}

/// Le nuove impostazioni sopra le attuali: un campo che manca resta com'è.
pub fn valida_impostazioni(dati: Option<&Json>, attuali: &Impostazioni) -> Result<Impostazioni, String> {
    let Some(Json::Obj(d)) = dati else { return Err("Le impostazioni dei promemoria non sono valide.".to_string()) };
    let mut nuove = attuali.clone();
    for (campo, v) in d.iter() {
        match campo.as_str() {
            "minuti" => nuove.minuti = minuti_validi(Some(v)).ok_or_else(|| format!("Minuti: da 1 a {MINUTI_VOCI} numeri interi diversi, da 0 a {MINUTI_MAX}."))?,
            "contenuto" => match v.as_str() {
                Some(c @ ("completo" | "minimo")) => nuove.contenuto = c.to_string(),
                _ => return Err("Il contenuto è «completo» o «minimo».".to_string()),
            },
            "suono" => match v.as_str() {
                Some(s) if suoni().iter().any(|x| x == s) => nuove.suono = s.to_string(),
                _ => return Err("Suono sconosciuto.".to_string()),
            },
            "notifica_windows" | "comparsa" => {
                let Json::Bool(b) = v else { return Err(format!("«{campo}» deve essere vero o falso.")) };
                if campo == "comparsa" {
                    nuove.comparsa = *b;
                } else {
                    nuove.notifica_windows = *b;
                }
            }
            _ => return Err("Impostazione sconosciuta.".to_string()),
        }
    }
    Ok(nuove)
}

// ---- Il tempo e il testo ---------------------------------------------------------------------------------------------------
/// L'istante (millisecondi dall'epoca) di un'ora di Roma. Nell'ora che il cambio ripete vale la prima, in quella che salta vale come prima del salto.
pub fn istante_da_roma(n: &Naive) -> i64 {
    let locale = utc_js(n.a, n.m - 1, n.g, n.h, n.mi, n.s);
    let voluta = microsecondi(&Naive { us: 0, ..*n });
    for scarto_min in [120, 60] {
        let ms = locale - scarto_min * 60_000;
        if microsecondi(&da_istante(ms, 0)) == voluta {
            return ms;
        }
    }
    locale - 60 * 60_000
}

fn durata(min: i64) -> String {
    if min < 120 {
        return format!("{min} min");
    }
    let resto = min % 60;
    format!("{} ore", min / 60) + &if resto != 0 { format!(" e {resto} min") } else { String::new() }
}

fn quando(inizio: &Naive, adesso_ms: i64) -> String {
    let oggi = da_istante(adesso_ms, 0);
    let stesso = |a: &Naive, b: &Naive| a.a == b.a && a.m == b.m && a.g == b.g;
    let ora = format!("alle {:02}:{:02}", inizio.h, inizio.mi);
    if stesso(inizio, &oggi) {
        return ora;
    }
    if stesso(inizio, &piu_giorni(&oggi, 1)) { format!("domani {ora}") } else { format!("il {:02}/{:02} {ora}", inizio.g, inizio.m) }
}

/// `v.replace(/\s+/g, " ").trim()` per un testo, "" per il resto.
fn pulito(v: Option<&Json>) -> String {
    let Some(Json::Str(s)) = v else { return String::new() };
    let mut out = String::with_capacity(s.len());
    let mut spazio = false;
    for c in s.chars() {
        if crate::py::spazio_js(c) {
            spazio = true;
        } else {
            if spazio && !out.is_empty() {
                out.push(' ');
            }
            spazio = false;
            out.push(c);
        }
    }
    out
}

/// Il testo del promemoria; la prima riga è il titolo della notifica. I minuti sono quelli che mancano davvero, arrotondati per eccesso.
pub fn testo_promemoria(ev: &Ev, inizio: &Naive, restanti_ms: i64, contenuto: &str, adesso_ms: i64) -> String {
    let mut righe = vec![format!("Tra {}, {}", durata(((restanti_ms as f64) / 60_000.0).ceil().max(1.0) as i64), quando(inizio, adesso_ms))];
    if contenuto == "minimo" {
        let azienda = pulito(ev.get("alias"));
        if !azienda.is_empty() {
            righe.push(azienda);
        }
    } else {
        let titolo = pulito(ev.get("title"));
        righe.push(if titolo.is_empty() { "(senza titolo)".to_string() } else { titolo });
        let luogo = pulito(ev.get("location"));
        let l = luogo.to_ascii_lowercase();
        if !luogo.is_empty() && !l.contains("http://") && !l.contains("https://") {
            righe.push(luogo);
        }
    }
    righe.join("\n")
}

// ---- Quello che è già partito --------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Riga {
    pub chiave: String,
    pub evento: String,
    pub inizio: String,
    pub minuti: i64,
    pub canale: String,
    pub inviato_il: i64,
    pub esito: String,
    pub tentativi: i64,
    pub errore: Option<String>,
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS promemoria_inviati (
  chiave TEXT PRIMARY KEY,
  evento TEXT NOT NULL,
  inizio TEXT NOT NULL,
  minuti INTEGER NOT NULL,
  canale TEXT NOT NULL,
  inviato_il INTEGER NOT NULL,
  esito TEXT NOT NULL,
  tentativi INTEGER NOT NULL,
  errore TEXT
)";

pub fn chiave_di(evento: &str, inizio: &str, minuti: i64, canale: &str) -> String {
    Json::Arr(vec![Json::Str(evento.to_string()), Json::Str(inizio.to_string()), Json::Num(minuti as f64), Json::Str(canale.to_string())]).in_js()
}

thread_local! {
    static PRONTE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn store() -> Esito<std::rc::Rc<rusqlite::Connection>> {
    let d = archivio()?;
    let file = crate::config::db_stato();
    if !PRONTE.with(|p| p.borrow().contains(&file)) {
        d.execute_batch(SCHEMA)?;
        PRONTE.with(|p| p.borrow_mut().insert(file));
    }
    Ok(d)
}

fn leggi_riga(chiave: &str) -> Esito<Option<Riga>> {
    Ok(store()?
        .query_row("SELECT chiave, evento, inizio, minuti, canale, inviato_il, esito, tentativi, errore FROM promemoria_inviati WHERE chiave = ?", [chiave], |r| {
            Ok(Riga { chiave: r.get(0)?, evento: r.get(1)?, inizio: r.get(2)?, minuti: r.get(3)?, canale: r.get(4)?, inviato_il: r.get(5)?, esito: r.get(6)?, tentativi: r.get(7)?, errore: r.get(8)? })
        })
        .optional()?)
}

fn scrivi_righe(righe: &[Riga]) -> Esito<()> {
    let d = store()?;
    d.execute_batch("BEGIN")?;
    let esito = (|| -> Esito<()> {
        let mut st = d.prepare_cached(
            "INSERT INTO promemoria_inviati(chiave, evento, inizio, minuti, canale, inviato_il, esito, tentativi, errore)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(chiave) DO UPDATE SET inviato_il = excluded.inviato_il, esito = excluded.esito, tentativi = excluded.tentativi, errore = excluded.errore",
        )?;
        for r in righe {
            st.execute(params![r.chiave, r.evento, r.inizio, r.minuti, r.canale, r.inviato_il, r.esito, r.tentativi, r.errore])?;
        }
        Ok(())
    })();
    match esito {
        Ok(()) => d.execute_batch("COMMIT")?,
        Err(e) => {
            let _ = d.execute_batch("ROLLBACK");
            return Err(e);
        }
    }
    Ok(())
}

fn pulisci(prima: i64) -> Esito<usize> {
    Ok(store()?.execute("DELETE FROM promemoria_inviati WHERE inviato_il < ?", [prima])?)
}

// ---- Il motore -----------------------------------------------------------------------------------------------------------
#[derive(Default)]
struct Motore {
    ultimo_giro: Option<i64>,
    ultimo_errore: Option<(String, i64)>,
    blocco_fino_a: HashMap<String, i64>,
    ultimi_log: HashMap<String, i64>,
    in_corso: bool,
}

thread_local! {
    static MOTORE: RefCell<Motore> = RefCell::new(Motore::default());
}

/// Un problema: si ricorda come ultimo errore e, una volta ogni tanto per `codice`, si scrive nel log. Solo frasi fisse e numeri.
fn segnala(codice: &str, testo: &str, riga: Option<&str>) {
    let t = adesso();
    MOTORE.with(|m| {
        let mut m = m.borrow_mut();
        m.ultimo_errore = Some((testo.to_string(), t));
        if m.ultimi_log.get(codice).is_some_and(|u| t - u < LOG_OGNI_MS) {
            return;
        }
        m.ultimi_log.insert(codice.to_string(), t);
        eprintln!("promemoria: {}", riga.unwrap_or(testo));
    });
}

struct Candidato {
    ev: Ev,
    id: String,
    inizio: String,
    naive: Naive,
    istante: i64,
}

/// Gli eventi che possono avere un promemoria: a orario, non nascosti, non ancora iniziati, con un inizio che si legge. In ordine di inizio.
fn candidati(eventi: Vec<Ev>, ora: i64) -> Vec<Candidato> {
    let mut visti = HashSet::new();
    let mut out = Vec::new();
    for ev in eventi {
        if py_bool(ev.get("allDay")) || py_bool(ev.get("nascosto")) || !py_bool(ev.get("start")) {
            continue;
        }
        let Ok(naive) = naive_locale(&string_js(ev.get("start"))) else { continue };
        let istante = istante_da_roma(&naive);
        if istante <= ora {
            continue;
        }
        let id = match ev.get("id") {
            None | Some(Json::Null) => {
                let account = match ev.get("account") {
                    None | Some(Json::Null) => String::new(),
                    x => string_js(x),
                };
                format!("{account}|{}", string_js(ev.get("start")))
            }
            x => string_js(x),
        };
        let inizio = iso_secondi(&naive);
        if !visti.insert(Json::Arr(vec![Json::Str(id.clone()), Json::Str(inizio.clone())]).in_js()) {
            continue;
        }
        out.push(Candidato { ev, id, inizio, naive, istante });
    }
    out.sort_by(|a, b| a.istante.cmp(&b.istante).then_with(|| crate::stato::confronta_utf16(&a.id, &b.id)));
    out
}

#[derive(Default, Debug)]
pub struct Riepilogo {
    pub letti: usize,
    pub inviati: usize,
    pub falliti: usize,
    pub abbandonati: usize,
}

async fn giro_interno(r: &mut Riepilogo) -> Esito<()> {
    let inizio_giro = adesso();
    MOTORE.with(|m| m.borrow_mut().ultimo_giro = Some(inizio_giro));
    // nessun canale pronto (di norma: nessun browser iscritto) non è un problema: il giro non fa altro
    if !push::configurato() {
        return Ok(());
    }
    let canale = push::NOME;
    let imp = leggi_impostazioni(&load_config()?);
    pulisci(inizio_giro - CONSERVA_GIORNI * 86_400_000)?;
    let piu_lontano = imp.minuti.iter().copied().max().unwrap_or(0);
    let fine = inizio_giro + (piu_lontano + MARGINE_MIN) * 60_000;
    let mut cfg = tutti_accesi(&load_config()?);
    let eventi = match raccogli(&mut cfg, &iso(&da_istante(inizio_giro, 0)), &iso(&da_istante(fine, 0)), false, true).await {
        Ok((eventi, errori, _)) => {
            if !errori.is_empty() {
                segnala("sorgenti", "la lettura degli eventi ha avuto problemi", Some(&format!("la lettura degli eventi ha avuto problemi ({} sorgenti)", errori.len())));
            }
            eventi
        }
        Err(e) => {
            segnala("lettura", "lettura degli eventi non riuscita", Some(&format!("lettura degli eventi non riuscita ({})", e.nome())));
            return Ok(());
        }
    };
    let vicini = candidati(eventi, inizio_giro);
    r.letti = vicini.len();
    for c in &vicini {
        if MOTORE.with(|m| m.borrow().blocco_fino_a.get(canale).copied().unwrap_or(0)) > adesso() {
            break;
        }
        let t = adesso();
        let restanti = c.istante - t;
        if restanti <= 0 {
            continue;
        }
        let mut aperti = Vec::new();
        for n in imp.minuti.iter().copied().filter(|n| restanti <= soglia_ms(*n)) {
            let chiave = chiave_di(&c.id, &c.inizio, n, canale);
            let riga = leggi_riga(&chiave)?;
            if riga.as_ref().is_none_or(|x| x.esito == "riprovo") {
                aperti.push((n, chiave, riga));
            }
        }
        if aperti.is_empty() {
            continue;
        }
        let esito = push::invia(&testo_promemoria(&c.ev, &c.naive, restanti, &imp.contenuto, t)).await;
        let dopo = adesso();
        let riga = |x: &(i64, String, Option<Riga>), come: &str, errore: Option<String>| Riga {
            chiave: x.1.clone(),
            evento: c.id.clone(),
            inizio: c.inizio.clone(),
            minuti: x.0,
            canale: canale.to_string(),
            inviato_il: dopo,
            esito: come.to_string(),
            tentativi: x.2.as_ref().map_or(0, |r| r.tentativi) + 1,
            errore,
        };
        if esito.ok {
            scrivi_righe(&aperti.iter().map(|x| riga(x, "inviato", None)).collect::<Vec<_>>())?;
            r.inviati += 1;
            continue;
        }
        let righe: Vec<Riga> = aperti
            .iter()
            .map(|x| riga(x, if x.2.as_ref().map_or(0, |r| r.tentativi) + 1 >= TENTATIVI_MAX { "fallito" } else { "riprovo" }, Some(esito.errore.clone())))
            .collect();
        let chiusi = righe.iter().filter(|x| x.esito == "fallito").count();
        scrivi_righe(&righe)?;
        r.falliti += 1;
        r.abbandonati += chiusi;
        if let Some(a) = esito.attesa_s.filter(|a| *a > 0.0) {
            let s = a.ceil().clamp(1.0, ATTESA_MAX_S) as i64;
            MOTORE.with(|m| m.borrow_mut().blocco_fino_a.insert(canale.to_string(), dopo + s * 1000));
        }
        // l'abbandono ha un codice suo: non deve restare nascosto dal limite sulla riga dei tentativi falliti
        if chiusi > 0 {
            segnala(&format!("abbandono:{canale}"), &esito.errore, Some(&format!("{chiusi} promemoria abbandonati dopo {TENTATIVI_MAX} tentativi sul canale {canale}: {}", esito.errore)));
        } else {
            segnala(&format!("invio:{canale}"), &esito.errore, None);
        }
        break;
    }
    if r.inviati > 0 {
        println!("promemoria: inviati {}", r.inviati);
    }
    Ok(())
}

/// Un giro: se c'è un canale pronto legge le impostazioni e gli eventi dei prossimi minuti e manda quello che serve.
pub async fn giro() -> Riepilogo {
    let mut r = Riepilogo::default();
    if MOTORE.with(|m| std::mem::replace(&mut m.borrow_mut().in_corso, true)) {
        return r;
    }
    if let Err(e) = giro_interno(&mut r).await {
        // il testo dell'errore può nominare un percorso, un indirizzo o un titolo: nel log va una frase fissa e il tipo
        segnala("giro", "giro non riuscito", Some(&format!("giro non riuscito ({})", e.nome())));
    }
    MOTORE.with(|m| m.borrow_mut().in_corso = false);
    r
}

fn intervallo() -> Duration {
    let s = crate::py::numero_da_testo(&crate::config::var("PROMEMORIA_INTERVALLO_S").unwrap_or_else(|| "x".to_string()));
    let s = if s.fract() == 0.0 && (5.0..=3600.0).contains(&s) { s as u64 } else { INTERVALLO_PREDEFINITO_S };
    Duration::from_secs(s)
}

/// Fa partire il giro ogni PROMEMORIA_INTERVALLO_S secondi (30): il giro dopo parte a giro finito, due giri non si sovrappongono mai.
pub fn avvia_promemoria() {
    let attesa = intervallo();
    tokio::task::spawn_local(async move {
        tokio::time::sleep(Duration::from_millis(PRIMO_GIRO_MS)).await;
        loop {
            giro().await;
            tokio::time::sleep(attesa).await;
        }
    });
    println!("promemoria: pronto, un giro ogni {} s", attesa.as_secs());
}

// ---- I metodi dell'API ---------------------------------------------------------------------------------------------------
/// Le impostazioni, quali canali il server ha configurato e l'ultimo problema: mai dati di un evento.
pub fn leggi_promemoria() -> Esito<Json> {
    let mut canale = Mappa::new();
    canale.insert("nome", push::NOME);
    canale.insert("configurato", push::configurato());
    for (k, v) in push::dettagli() {
        canale.insert(k, v);
    }
    let errore = MOTORE.with(|m| m.borrow().ultimo_errore.clone());
    Ok(ogg! {
        "promemoria": leggi_impostazioni(&load_config()?).json(),
        "canali": Json::Arr(vec![Json::Obj(canale)]),
        "ultimo_errore": errore.as_ref().map_or(Json::Null, |e| Json::Str(e.0.clone())),
        "ultimo_errore_il": errore.map_or(Json::Null, |e| Json::Str(da_secondi(e.1 as f64 / 1000.0))),
    })
}

pub fn salva_promemoria(dati: Option<&Json>) -> Esito<Json> {
    match valida_impostazioni(dati, &leggi_impostazioni(&load_config()?)) {
        Err(e) => Ok(ogg! { "ok": false, "error": e }),
        Ok(v) => {
            let j = v.json();
            modifica_config(|cfg| {
                cfg.insert("promemoria", j.clone());
                Ok(())
            })?;
            Ok(ogg! { "ok": true, "promemoria": j })
        }
    }
}

/// Manda il testo fisso di prova su un canale: mai dati di un evento, e solo su richiesta.
pub async fn prova_promemoria(nome: Option<&Json>) -> Esito<Json> {
    if nome.and_then(Json::as_str) != Some(push::NOME) {
        return Ok(ogg! { "ok": false, "error": "Canale sconosciuto." });
    }
    if !push::configurato() {
        return Ok(ogg! { "ok": false, "error": "Canale non configurato sul server." });
    }
    let EsitoInvio { ok, errore, .. } = push::invia(TESTO_PROVA).await;
    Ok(if ok { ogg! { "ok": true } } else { ogg! { "ok": false, "error": errore } })
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn il_testo_del_promemoria() {
        let ev = crate::py::mappa(ogg! { "title": "  Riunione \n con  Alfa ", "location": "Sala 2", "alias": "Alfa" });
        let inizio = crate::fuso::leggi_naive("2026-11-11T15:00:00").unwrap();
        let ora = istante_da_roma(&crate::fuso::leggi_naive("2026-11-11T14:50:00").unwrap());
        assert_eq!(testo_promemoria(&ev, &inizio, 9 * 60_000 + 1, "completo", ora), "Tra 10 min, alle 15:00\nRiunione con Alfa\nSala 2");
        assert_eq!(testo_promemoria(&ev, &inizio, 150 * 60_000, "minimo", ora), "Tra 2 ore e 30 min, alle 15:00\nAlfa");
        let domani = crate::fuso::leggi_naive("2026-11-12T09:05:00").unwrap();
        assert_eq!(testo_promemoria(&ev, &domani, 120 * 60_000, "minimo", ora), "Tra 2 ore, domani alle 09:05\nAlfa");
    }

    #[test]
    fn le_impostazioni() {
        let attuali = leggi_impostazioni(&Mappa::new());
        assert_eq!(attuali.minuti, vec![10]);
        let n = valida_impostazioni(Some(&ogg! { "minuti": Json::Arr(vec![Json::Num(15.0), Json::Num(0.0), Json::Num(15.0)]), "suono": "Alarm3" }), &attuali).unwrap();
        assert_eq!((n.minuti.clone(), n.suono.as_str()), (vec![0, 15], "Alarm3"));
        assert!(valida_impostazioni(Some(&ogg! { "minuti": Json::Arr(vec![Json::Num(1.5)]) }), &attuali).is_err());
        assert_eq!(valida_impostazioni(Some(&ogg! { "comparsa": 1 }), &attuali).unwrap_err(), "«comparsa» deve essere vero o falso.");
    }
}
