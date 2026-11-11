//! GET /condividi/<testo segreto>.ics: il calendario di un link di condivisione (condivisione.rs), senza sessione (era lib/condividi.ts). Chi ha
//! il link legge il calendario, e basta: l'indirizzo è la sola chiave. Un indirizzo che non vale riceve sempre la stessa risposta, la stessa
//! di un percorso che non c'è. Il testo di un calendario si tiene un minuto per link, così un lettore insistente non carica l'agente.

use crate::condivisione::{nome_pubblicato, registra_accesso, trova_per_token, Condivisione};
use crate::condivisione_eventi::{in_eventi_uscita, leggi_per_condivisione};
use crate::disponibilita::oggi_a_roma;
use crate::ics_uscita::{costruisci_ics, Pubblicazione};
use crate::orologio::adesso;
use crate::py::Esito;
use crate::web::{data_http, leggi_data_http, Resp};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use std::cell::RefCell;

pub const TTL_CORPO_MS: i64 = 60_000;
pub const TTL_GUASTO_MS: i64 = 15_000;
/// Quanto chi legge può tenere la risposta senza richiederla: 5 minuti.
pub const MAX_AGE_S: i64 = 300;
/// Gli indirizzi sbagliati che un indirizzo di rete può provare in una finestra prima di ricevere 429 al posto di 404.
pub const FALLIMENTI_MAX: i64 = 20;
pub const FINESTRA_FALLIMENTI_MS: i64 = 60_000;
const INDIRIZZI_MAX: usize = 1000;
const MEMORIA_MAX: usize = 100;

#[derive(Clone)]
struct Buono {
    corpo: String,
    etag: String,
    modificato: i64,
}

#[derive(Clone)]
struct Voce {
    versione: i64,
    fino: i64,
    /// None: un guasto; `buono` è l'ultima versione buona che si conosce.
    ok: Option<Buono>,
    buono: Option<Buono>,
}

#[derive(Default)]
struct Memoria {
    voci: IndexMap<String, Voce>,
    in_corso: IndexMap<String, tokio::sync::watch::Receiver<Option<Voce>>>,
    fallimenti: IndexMap<String, (i64, i64)>,
}

thread_local! {
    static MEMORIA: RefCell<Memoria> = RefCell::new(Memoria::default());
}

/// Svuota la memoria (per le prove).
pub fn azzera() {
    MEMORIA.with(|m| *m.borrow_mut() = Memoria::default());
}

const COMUNI: [(&str, &str); 2] = [("x-robots-tag", "noindex, nofollow"), ("x-content-type-options", "nosniff")];

fn con_comuni<'a>(h: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v = h.to_vec();
    v.extend_from_slice(&COMUNI);
    v
}

/// Il 404 di un percorso che non c'è: lo stesso corpo e le stesse intestazioni per ogni indirizzo che non vale.
fn non_trovato(testa: bool) -> Resp {
    let corpo = if testa { Vec::new() } else { br#"{"error":"non trovato"}"#.to_vec() };
    Resp::nuova(404, &con_comuni(&[("content-type", "application/json"), ("cache-control", "no-store")]), corpo)
}

fn non_disponibile(testa: bool) -> Resp {
    let corpo = if testa { Vec::new() } else { "Calendario non disponibile in questo momento: riprova fra poco.\n".as_bytes().to_vec() };
    Resp::nuova(503, &con_comuni(&[("content-type", "text/plain; charset=utf-8"), ("cache-control", "no-store"), ("retry-after", "120")]), corpo)
}

/// Chi sta chiedendo, per contare gli errori: l'ultimo indirizzo di X-Forwarded-For, quello che Caddy ha scritto.
pub fn chiave_client(xff: Option<&str>) -> String {
    let ultimo = xff.and_then(|x| x.rsplit(',').next()).map(|x| crate::py::js_trim(x).to_string()).unwrap_or_default();
    if crate::rete_sicura::ip_da_testo(&ultimo).is_some() { ultimo } else { "sconosciuto".to_string() }
}

fn conta_fallimento(chiave: &str, ora: i64) -> (i64, i64) {
    MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        let (mut da, mut n) = m.fallimenti.shift_remove(chiave).unwrap_or((ora, 0));
        if ora - da >= FINESTRA_FALLIMENTI_MS {
            (da, n) = (ora, 0);
        }
        n += 1;
        m.fallimenti.insert(chiave.to_string(), (da, n));
        // gli indirizzi si contano a decine di migliaia solo se qualcuno li inventa: si tengono gli ultimi
        if m.fallimenti.len() > INDIRIZZI_MAX {
            m.fallimenti.shift_remove_index(0);
        }
        if n == FALLIMENTI_MAX + 1 {
            eprintln!("condivisione: troppi indirizzi sbagliati da {chiave}, per un minuto la risposta è 429");
        }
        (n, (((da + FINESTRA_FALLIMENTI_MS - ora) as f64 / 1000.0).ceil() as i64).max(1))
    })
}

/// Un indirizzo che non vale: 404, o 429 se chi chiede ne ha provati troppi.
fn rifiuta(testa: bool, xff: Option<&str>, ora: i64) -> Resp {
    let (n, fra) = conta_fallimento(&chiave_client(xff), ora);
    if n <= FALLIMENTI_MAX {
        return non_trovato(testa);
    }
    let corpo = if testa { Vec::new() } else { "Troppe richieste: riprova fra poco.\n".as_bytes().to_vec() };
    let fra = fra.to_string();
    Resp::nuova(429, &con_comuni(&[("content-type", "text/plain; charset=utf-8"), ("cache-control", "no-store"), ("retry-after", &fra)]), corpo)
}

async fn genera(link: &Condivisione, ora: i64, precedente: Option<Voce>) -> Voce {
    let versione = link.aggiornato;
    let buono_prima = precedente.and_then(|p| p.ok.or(p.buono));
    let esito: Esito<Buono> = async {
        let lettura = leggi_per_condivisione(link, &oggi_a_roma(ora)).await?;
        let eventi = in_eventi_uscita(&lettura)?;
        let costruisci = |stamp: i64| costruisci_ics(&Pubblicazione { nome: nome_pubblicato(&link.nome_calendario), sale: &link.sale, livello: &link.livello, stamp, eventi: &eventi });
        // DTSTAMP è l'ora in cui il contenuto è cambiato: se non è cambiato niente il testo, e con lui ETag e Last-Modified, restano gli stessi
        let mut modificato = buono_prima.as_ref().map_or(ora, |b| b.modificato);
        let mut corpo = costruisci(modificato);
        if buono_prima.as_ref().is_some_and(|b| b.corpo != corpo) {
            modificato = ora;
            corpo = costruisci(modificato);
        }
        let h: String = Sha256::digest(corpo.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        Ok(Buono { etag: format!("\"{}\"", &h[..32]), corpo, modificato })
    }
    .await;
    match esito {
        Ok(b) => Voce { versione, fino: ora + TTL_CORPO_MS, ok: Some(b), buono: None },
        Err(e) => {
            // chi legge il link vede solo "non disponibile": il motivo lo legge chi ha il gateway, qui
            if e.nome() == "LetturaNonDisponibile" {
                eprintln!("condivisione {}: calendario non disponibile, {}", link.id, e.messaggio());
            } else {
                eprintln!("condivisione {}: lettura non riuscita {}: {}", link.id, e.nome(), e.messaggio());
            }
            Voce { versione, fino: ora + TTL_GUASTO_MS, ok: None, buono: buono_prima }
        }
    }
}

/// Il testo del calendario di un link: quello generato da meno di un minuto, o uno nuovo (richieste uguali in corso fanno una lettura sola).
async fn corpo_di(link: &Condivisione, ora: i64) -> Voce {
    let chiave = format!("{}|{}", link.id, link.aggiornato);
    enum Passo {
        Fatto(Voce),
        Aspetta(tokio::sync::watch::Receiver<Option<Voce>>),
        Genera(Option<Voce>, tokio::sync::watch::Sender<Option<Voce>>),
    }
    let passo = MEMORIA.with(|m| {
        let mut m = m.borrow_mut();
        let salvata = m.voci.get(&link.id).cloned();
        if let Some(s) = salvata.as_ref().filter(|s| s.versione == link.aggiornato && s.fino > ora) {
            return Passo::Fatto(s.clone());
        }
        if let Some(rx) = m.in_corso.get(&chiave) {
            return Passo::Aspetta(rx.clone());
        }
        let (tx, rx) = tokio::sync::watch::channel(None);
        m.in_corso.insert(chiave.clone(), rx);
        Passo::Genera(salvata, tx)
    });
    match passo {
        Passo::Fatto(v) => v,
        Passo::Aspetta(mut rx) => {
            let _ = rx.wait_for(Option::is_some).await;
            let v = rx.borrow().clone();
            v.unwrap_or(Voce { versione: link.aggiornato, fino: 0, ok: None, buono: None })
        }
        Passo::Genera(salvata, tx) => {
            let voce = genera(link, ora, salvata).await;
            MEMORIA.with(|m| {
                let mut m = m.borrow_mut();
                m.voci.shift_remove(&link.id);
                m.voci.insert(link.id.clone(), voce.clone());
                if m.voci.len() > MEMORIA_MAX {
                    m.voci.shift_remove_index(0);
                }
                m.in_corso.shift_remove(&chiave);
            });
            let _ = tx.send(Some(voce.clone()));
            voce
        }
    }
}

/// La richiesta ripete quello che ha già (If-None-Match, o If-Modified-Since se non c'è l'altra): 304.
fn gia_aggiornato(se_diverso: Option<&str>, se_cambiato: Option<&str>, voce: &Buono) -> bool {
    if let Some(c) = se_diverso {
        return crate::py::js_trim(c) == "*" || c.split(',').any(|x| crate::py::js_trim(x).strip_prefix("W/").unwrap_or(crate::py::js_trim(x)) == voce.etag);
    }
    se_cambiato.and_then(leggi_data_http).is_some_and(|dal| voce.modificato.div_euclid(1000) <= dal.div_euclid(1000))
}

/// Il testo fra /condividi/ e .ics, o "" per qualunque altro percorso.
fn testo_del_percorso(percorso: &str) -> &str {
    let Some(resto) = percorso.strip_prefix("/condividi/").and_then(|r| r.strip_suffix(".ics")) else { return "" };
    if resto.is_empty() || resto.contains('/') || crate::py::lunghezza_js(resto) > 200 { "" } else { resto }
}

/// GET e HEAD di /condividi/<testo segreto>.ics.
pub async fn condividi(metodo: &str, url: &url::Url, intestazioni: &[(String, String)]) -> Resp {
    let ora = adesso();
    let testa = metodo == "HEAD";
    let h = |n: &str| crate::auth::rotte::intestazione(intestazioni, n);
    if metodo != "GET" && !testa {
        return Resp::vuota(405, &con_comuni(&[("allow", "GET, HEAD"), ("cache-control", "no-store")]));
    }
    let link = match trova_per_token(testo_del_percorso(url.path())) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("condivisione: link non verificabile {}", e.messaggio());
            return non_disponibile(testa);
        }
    };
    let Some(link) = link else { return rifiuta(testa, h("x-forwarded-for").as_deref(), ora) };
    registra_accesso(&link.id, ora);
    let voce = corpo_di(&link, ora).await;
    let Some(b) = voce.ok else { return non_disponibile(testa) };
    let max_age = format!("private, max-age={MAX_AGE_S}");
    let modificato = data_http(b.modificato);
    let base = [("cache-control", max_age.as_str()), ("etag", b.etag.as_str()), ("last-modified", modificato.as_str())];
    if gia_aggiornato(h("if-none-match").as_deref(), h("if-modified-since").as_deref(), &b) {
        return Resp::vuota(304, &con_comuni(&base));
    }
    let lunghezza = b.corpo.len().to_string();
    let mut tutte = con_comuni(&base);
    tutte.extend_from_slice(&[("content-type", "text/calendar; charset=utf-8"), ("content-length", &lunghezza), ("content-disposition", "inline; filename=\"calendario.ics\"")]);
    Resp::nuova(200, &tutte, if testa { Vec::new() } else { b.corpo.into_bytes() })
}
