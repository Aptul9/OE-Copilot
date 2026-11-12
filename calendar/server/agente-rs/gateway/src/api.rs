//! I metodi di app.Api che il servizio Python ui esponeva alla pagina (era lib/core/api.ts): la lettura e la scrittura. Ogni metodo riceve
//! gli argomenti della lista JSON e restituisce quello che finisce nel corpo della risposta; un argomento che manca vale undefined (None).

use crate::agente::{azzera_memoria, chiedi, inoltri_sostituiti, invia};
use crate::auth::righe::iso_ms;
use crate::caselle::Ev;
use crate::etichette::{aggiorna_etichette, etichette_account, indirizzi_elenco, manda_etichette};
use crate::eventi::{calendari_caselle, calendari_extra, chiave_eliminato, occupato, raccogli, tutti_accesi};
use crate::loghi::{aggiorna_logo, anteprima_logo, carica_logo, esito_negativo, imposta_logo, interfaccia_con_loghi, leggi_voci, pulisci_loghi, rinomina_logo, togli_logo};
use crate::orologio::{adesso, ora_vera};
use crate::personalizzazione::personalizzazione;
use crate::py::{is_dict, js_trim, py_bool, py_str, string_js, taglia_js, testo_pulito, Errore, Esito};
use crate::stato::{alias_per_account, confronta_punti, imposta_acceso, load_config, membri_gruppo, modifica_config, ordine_calendario, rinomina_nei_gruppi, Config};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;

const GIORNI_MAX: i64 = 62;

const METODI: &[&str] = &[
    "get_config",
    "get_personalizzazione",
    "get_calendars",
    "get_events",
    "get_disponibilita",
    "save_config",
    "set_calendar_enabled",
    "set_account_enabled",
    "set_azienda_enabled",
    "mostra_solo_azienda",
    "mostra_solo_gruppo",
    "set_account_alias",
    "add_ics_calendar",
    "remove_ics_calendar",
    "add_local_event",
    "delete_local_event",
    "elimina_evento",
    "ripristina_evento",
    "get_loghi",
    "preview_logo",
    "set_logo",
    "upload_logo",
    "refresh_logo",
    "remove_logo",
    "get_promemoria",
    "set_promemoria",
    "prova_promemoria",
    "iscrivi_push",
    "disiscrivi_push",
    "get_condivisioni",
    "add_condivisione",
    "update_condivisione",
    "revoke_condivisione",
    "delete_condivisione",
    "rotate_condivisione",
    "get_stato_caselle",
    "connect_microsoft",
    "cancel_microsoft_signin",
    "manage_microsoft",
    "get_condivisione_microsoft",
    "set_condivisione_microsoft",
    "rimuovi_casella",
    "inizia_accesso_microsoft",
    "controlla_accesso_casella",
    "completa_accesso_casella",
];

/// I metodi che il gateway serve da sé.
pub fn ha(nome: &str) -> bool {
    METODI.contains(&nome)
}

/// Un argomento mancante o di un altro tipo.
fn argomenti(m: &str) -> Errore {
    Errore::Argomenti(m.to_string())
}

/// int(valore) per un valore JSON: numeri (troncati), testi di sole cifre, booleani; il resto è un errore.
fn py_int(v: Option<&Json>) -> Esito<i64> {
    match v {
        Some(Json::Bool(b)) => Ok(i64::from(*b)),
        Some(Json::Num(n)) if n.is_finite() => Ok(n.trunc() as i64),
        Some(Json::Str(s)) => {
            let t = s.trim_matches(crate::py::spazio_js);
            let cifre = t.strip_prefix(['+', '-']).unwrap_or(t);
            if !cifre.is_empty() && cifre.bytes().all(|b| b.is_ascii_digit()) {
                return Ok(crate::py::numero_da_testo(t) as i64);
            }
            Err(argomenti("int() non riceve questo valore"))
        }
        _ => Err(argomenti("int() non riceve questo valore")),
    }
}

/// date.fromisoformat (Python 3.12) di un testo: AAAA-MM-GG o AAAAMMGG.
fn py_data(v: Option<&Json>) -> Esito<(i64, i64, i64)> {
    let t = py_str(v);
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^([0-9]{4})-?([0-9]{2})-?([0-9]{2})$").unwrap());
    let c = RE.captures(&t).ok_or_else(|| argomenti("Invalid isoformat string"))?;
    let (a, m, g) = (c[1].parse::<i64>().unwrap_or(0), c[2].parse::<i64>().unwrap_or(0), c[3].parse::<i64>().unwrap_or(0));
    let (a2, m2, g2, ..) = crate::fuso::pezzi_utc(crate::fuso::utc_js(a, m - 1, g, 0, 0, 0));
    if a2 != a || m2 != m || g2 != g {
        return Err(argomenti("day is out of range for month"));
    }
    Ok((a, m, g))
}

/// `a === b` di JavaScript fra due valori JSON (undefined è None): oggetti e liste non sono mai uguali.
fn uguale(a: Option<&Json>, b: Option<&Json>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Json::Null), Some(Json::Null)) => true,
        (Some(Json::Bool(x)), Some(Json::Bool(y))) => x == y,
        (Some(Json::Num(x)), Some(Json::Num(y))) => x == y,
        (Some(Json::Str(x)), Some(Json::Str(y))) => x == y,
        _ => false,
    }
}

/// `String(v || altrimenti)`
fn testo_o(v: Option<&Json>, altrimenti: &str) -> String {
    if py_bool(v) { string_js(v) } else { altrimenti.to_string() }
}

/// `String(v ?? "")`
fn testo_o_vuoto(v: Option<&Json>) -> String {
    match v {
        None | Some(Json::Null) => String::new(),
        x => string_js(x),
    }
}

fn ok() -> Json {
    ogg! { "ok": true }
}

fn no(errore: impl Into<String>) -> Json {
    ogg! { "ok": false, "error": errore.into() }
}

/// Il corpo di un risultato che dice "no" con il testo che la pagina mostra.
fn rifiuto(nome: &str, e: &Errore) -> Json {
    if !e.e_valore() {
        eprintln!("{nome}: eccezione {}: {}", e.nome(), e.messaggio());
    }
    no(e.messaggio())
}

fn unisci(base: Json, altro: Json) -> Json {
    let mut m = crate::py::mappa(base);
    if let Json::Obj(a) = altro {
        for (k, v) in a.iter() {
            m.insert(k.clone(), v.clone());
        }
    }
    Json::Obj(m)
}

fn mappa_di(cfg: Config) -> Json {
    Json::Obj(cfg)
}

/// Tutte le sorgenti di calendari: le caselle (se l'agente non risponde si va avanti senza) e le altre voci.
async fn tutte_le_sorgenti(cfg: &mut Config) -> Esito<Vec<Ev>> {
    let caselle = match calendari_caselle(cfg, false).await {
        Ok(c) => c,
        Err(Errore::Agente(_)) => Vec::new(),
        Err(e) => return Err(e),
    };
    let mut v = caselle;
    v.extend(calendari_extra(cfg)?);
    Ok(v)
}

/// Il nome mostrato di un calendario: l'alias, se no l'account.
fn nome_mostrato(c: &Ev) -> Option<Json> {
    if py_bool(c.get("alias")) { c.get("alias").cloned() } else { c.get("account").cloned() }
}

/// Dopo aver letto le sorgenti (che è lento e può assegnare colori), si cambia l'elenco dei calendari spenti sulla configurazione di adesso.
fn cambia_spenti(cambia: impl FnOnce(&mut Vec<Json>)) -> Esito<()> {
    modifica_config(|cfg| {
        let mut spenti = crate::stato::spenti_di(cfg);
        cambia(&mut spenti);
        spenti.sort_by(|a, b| confronta_punti(&string_js(Some(a)), &string_js(Some(b))));
        cfg.insert("disabled_calendars", Json::Arr(spenti));
        Ok(())
    })
}

fn togli(spenti: &mut Vec<Json>, id: Option<&Json>) {
    spenti.retain(|x| Some(x) != id);
}

fn metti(spenti: &mut Vec<Json>, id: Option<&Json>) {
    let v = id.cloned().unwrap_or(Json::Null);
    if !spenti.contains(&v) {
        spenti.push(v);
    }
}

/// Accende le aziende per cui `accesa(nome)` è vera e spegne tutte le altre.
async fn accendi_solo(accesa: impl Fn(Option<&Json>) -> bool) -> Esito<()> {
    let tutti = tutte_le_sorgenti(&mut load_config()?).await?;
    cambia_spenti(|spenti| {
        for c in &tutti {
            if accesa(nome_mostrato(c).as_ref()) {
                togli(spenti, c.get("id"));
            } else {
                metti(spenti, c.get("id"));
            }
        }
    })
}

/// Il motivo di parse_bound per un estremo che non è un testo: il TypeError di JavaScript.
fn estremo(v: Option<&Json>) -> Result<String, String> {
    match v {
        Some(Json::Str(s)) => Ok(s.clone()),
        None => Err("Cannot read properties of undefined (reading 'trim')".to_string()),
        Some(Json::Null) => Err("Cannot read properties of null (reading 'trim')".to_string()),
        _ => Err("testo.trim is not a function".to_string()),
    }
}

/// I metodi che cambiano gli orari occupati della pagina pubblica: eventi personali, calendari accesi, gruppi, caselle.
pub const CAMBIANO_DISPONIBILITA: [&str; 14] = [
    "add_local_event", "delete_local_event", "elimina_evento", "ripristina_evento", "save_config", "set_calendar_enabled", "set_account_enabled",
    "set_azienda_enabled", "mostra_solo_azienda", "mostra_solo_gruppo", "set_account_alias", "add_ics_calendar", "remove_ics_calendar", "rimuovi_casella",
];

/// Chiama un metodo con gli argomenti della richiesta (la lista JSON).
pub async fn chiama(metodo: &str, args: Vec<Json>) -> Esito<Json> {
    let esito = esegui(metodo, args).await;
    // la pagina pubblica tiene i blocchi pronti per ore: dopo un cambio li rilegge, così un evento che blocca il tempo si vede subito
    if CAMBIANO_DISPONIBILITA.contains(&metodo) {
        crate::disponibilita::dimentica_disponibilita();
    }
    esito
}

async fn esegui(metodo: &str, args: Vec<Json>) -> Esito<Json> {
    let a = |i: usize| args.get(i);
    match metodo {
        "get_config" => Ok(mappa_di(load_config()?)),
        "get_personalizzazione" => Ok(ogg! { "interfaccia": Json::Obj(interfaccia_con_loghi(&personalizzazione().interfaccia)?) }),
        "get_calendars" => get_calendars(a(0)).await,
        "get_events" => Ok(get_events(a(0), a(1), a(2), a(3)).await),
        "get_disponibilita" => get_disponibilita(a(0), a(1), a(2)).await,
        "save_config" => {
            let voci = if py_bool(a(0)) { a(0).cloned().unwrap_or_default() } else { Json::obj() };
            let Json::Obj(voci) = voci else { return Err(Errore::tipo("cannot convert dictionary update sequence element #0 to a sequence")) };
            modifica_config(|cfg| {
                for (k, v) in voci.iter() {
                    cfg.insert(k.clone(), v.clone());
                }
                Ok(())
            })?;
            Ok(ok())
        }
        "set_calendar_enabled" => {
            let (id, acceso) = (py_str(a(0)), py_bool(a(1)));
            modifica_config(|cfg| {
                imposta_acceso(cfg, &id, acceso);
                Ok(())
            })?;
            Ok(ok())
        }
        "set_account_enabled" => {
            let tutti = tutte_le_sorgenti(&mut load_config()?).await?;
            let ids: Vec<String> = tutti.iter().filter(|c| uguale(c.get("account"), a(0))).map(|c| string_js(c.get("id"))).collect();
            let acceso = py_bool(a(1));
            if !ids.is_empty() {
                modifica_config(|cfg| {
                    for id in &ids {
                        imposta_acceso(cfg, id, acceso);
                    }
                    Ok(())
                })?;
            }
            Ok(ok())
        }
        "set_azienda_enabled" => {
            let tutti = tutte_le_sorgenti(&mut load_config()?).await?;
            let suoi: Vec<&Ev> = tutti.iter().filter(|c| uguale(nome_mostrato(c).as_ref(), a(0))).collect();
            let acceso = py_bool(a(1));
            cambia_spenti(|spenti| {
                for c in &suoi {
                    if acceso {
                        togli(spenti, c.get("id"));
                    } else {
                        metti(spenti, c.get("id"));
                    }
                }
            })?;
            println!("set_azienda_enabled: {} -> {} ({} calendari)", py_str(a(0)), if acceso { "acceso" } else { "spento" }, suoi.len());
            Ok(ogg! { "ok": true, "calendari": suoi.len() })
        }
        "mostra_solo_azienda" => {
            let azienda = a(0).cloned();
            let tutte = !py_bool(azienda.as_ref());
            accendi_solo(|nome| tutte || uguale(nome, azienda.as_ref())).await?;
            println!("mostra_solo_azienda: {}", if tutte { "tutte".to_string() } else { py_str(azienda.as_ref()) });
            Ok(ok())
        }
        "mostra_solo_gruppo" => {
            let gruppo = py_str(a(0));
            let Some(membri) = membri_gruppo(&load_config()?, &gruppo) else { return Ok(no("Gruppo non trovato")) };
            accendi_solo(|nome| matches!(nome, Some(Json::Str(n)) if membri.has(n))).await?;
            println!("mostra_solo_gruppo: {gruppo} ({} aziende)", membri.aziende.len());
            Ok(ok())
        }
        "set_account_alias" => set_account_alias(a(0), a(1)),
        "add_ics_calendar" => Ok(match crate::ics::aggiungi_feed(a(0), a(1)).await {
            Ok(feed) => {
                println!("add_ics_calendar: aggiunto {}", string_js(feed.get("name")));
                ogg! { "ok": true, "feed": feed }
            }
            Err(e) => rifiuto("add_ics_calendar", &e),
        }),
        "remove_ics_calendar" => {
            crate::ics::rimuovi_feed(a(0))?;
            Ok(ok())
        }
        "add_local_event" => Ok(match crate::locali::aggiungi_evento(a(0)) {
            Ok(()) => ok(),
            Err(e) => rifiuto("add_local_event", &e),
        }),
        "delete_local_event" => {
            crate::locali::elimina_evento(a(0))?;
            Ok(ok())
        }
        "elimina_evento" => elimina_evento(a(0)),
        "ripristina_evento" => {
            let chiave = a(0).cloned();
            let restano = modifica_config(|cfg| {
                let v: Vec<Json> = crate::py::lista(cfg.get("eventi_eliminati")).iter().filter(|v| !uguale(v.get("chiave"), chiave.as_ref())).cloned().collect();
                cfg.insert("eventi_eliminati", Json::Arr(v.clone()));
                Ok(v)
            })?;
            Ok(ogg! { "ok": true, "eventi_eliminati": Json::Arr(restano) })
        }
        "get_loghi" => Ok(ogg! { "loghi": Json::Obj(leggi_voci()?) }),
        "preview_logo" => Ok(match anteprima_logo(a(0)).await {
            Ok(m) => unisci(ok(), Json::Obj(m)),
            Err(e) => esito_negativo("preview_logo", &e),
        }),
        "set_logo" => Ok(match imposta_logo(a(0), a(1)).await {
            Ok(l) => ogg! { "ok": true, "logo": l },
            Err(e) => esito_negativo("set_logo", &e),
        }),
        "upload_logo" => Ok(match carica_logo(a(0), a(1)) {
            Ok(l) => ogg! { "ok": true, "logo": l },
            Err(e) => esito_negativo("upload_logo", &e),
        }),
        "refresh_logo" => Ok(match aggiorna_logo(a(0)).await {
            Ok(l) => ogg! { "ok": true, "logo": l },
            Err(e) => esito_negativo("refresh_logo", &e),
        }),
        "remove_logo" => Ok(match togli_logo(a(0)) {
            Ok(()) => ok(),
            Err(e) => esito_negativo("remove_logo", &e),
        }),
        "get_promemoria" => crate::promemoria::leggi_promemoria(),
        "set_promemoria" => crate::promemoria::salva_promemoria(a(0)),
        "prova_promemoria" => crate::promemoria::prova_promemoria(a(0)).await,
        "iscrivi_push" => crate::promemoria_push::iscrivi(a(0)),
        "disiscrivi_push" => crate::promemoria_push::disiscrivi(a(0)),
        "get_condivisioni" => crate::condivisione::elenco_condivisioni(),
        "add_condivisione" => crate::condivisione::crea_condivisione(a(0)),
        "update_condivisione" => crate::condivisione::aggiorna_condivisione(a(0), a(1)),
        "revoke_condivisione" => crate::condivisione::revoca_condivisione(a(0)),
        "delete_condivisione" => crate::condivisione::elimina_condivisione(a(0)),
        "rotate_condivisione" => crate::condivisione::ruota_condivisione(a(0)),
        "get_stato_caselle" => Ok(get_stato_caselle().await.unwrap_or_else(|e| ogg! { "ok": false, "caselle": Json::Arr(vec![]), "error": e.messaggio() })),
        "connect_microsoft" => Ok(connect_microsoft(a(0), a(1)).await.unwrap_or_else(|e| no(e.messaggio()))),
        "cancel_microsoft_signin" => Ok(cancel_signin(a(0), a(1)).await.unwrap_or_else(|e| no(e.messaggio()))),
        "manage_microsoft" => Ok(manage_microsoft(a(0), a(1), a(2)).await.unwrap_or_else(|e| no(e.messaggio()))),
        "get_condivisione_microsoft" => Ok(get_condivisione_microsoft().await.unwrap_or_else(|e| ogg! { "ok": false, "accounts": Json::Arr(vec![]), "teamsOnly": Json::Arr(vec![]), "teamsAt": 0, "error": e.messaggio() })),
        "set_condivisione_microsoft" => Ok(set_condivisione_microsoft(a(0), a(1)).await.unwrap_or_else(|e| rifiuto("set_condivisione_microsoft", &e))),
        "rimuovi_casella" => Ok(rimuovi_casella(a(0)).await.unwrap_or_else(|e| rifiuto("rimuovi_casella", &e))),
        "inizia_accesso_microsoft" => Ok(inizia_accesso(a(0), a(1), a(2)).await.unwrap_or_else(|e| rifiuto("inizia_accesso_microsoft", &e))),
        "controlla_accesso_casella" => Ok(controlla_accesso(a(0)).await.unwrap_or_else(|e| rifiuto("controlla_accesso_casella", &e))),
        "completa_accesso_casella" => Ok(completa_accesso(a(0)).await.unwrap_or_else(|e| rifiuto("completa_accesso_casella", &e))),
        _ => Err(Errore::tipo(format!("apiTs[metodo] is not a function: {metodo}"))),
    }
}

async fn get_calendars(force: Option<&Json>) -> Esito<Json> {
    let t0 = ora_vera();
    let mut cfg = load_config()?;
    let (cals, errore) = match calendari_caselle(&mut cfg, py_bool(force)).await {
        Ok(c) => (c, Json::Null),
        Err(e) => {
            if matches!(e, Errore::Agente(_)) {
                eprintln!("get_calendars: caselle non disponibili: {}", e.messaggio());
            } else {
                eprintln!("get_calendars: eccezione {}: {}", e.nome(), e.messaggio());
            }
            (Vec::new(), Json::Str(e.messaggio().to_string()))
        }
    };
    // Dopo le caselle, non prima: il pallino di un'azienda inoltrata dipende da com'è andata la lettura della casella che ne ospita le copie.
    let extra = calendari_extra(&mut cfg)?;
    tokio::task::spawn_local(aggiorna_etichette(ora_vera()));
    println!("get_calendars: {} delle caselle + {} extra in {:.1}s", cals.len(), extra.len(), (ora_vera() - t0) as f64 / 1000.0);
    // Ordine scelto dall'utente per nome mostrato; stabile per il resto
    let mut calendars: Vec<Ev> = cals.into_iter().chain(extra).collect();
    let chiave = |c: &Ev| ordine_calendario(&testo_o(if py_bool(c.get("alias")) { c.get("alias") } else { c.get("account") }, ""));
    calendars.sort_by_key(chiave);
    Ok(ogg! { "calendars": Json::Arr(calendars.into_iter().map(Json::Obj).collect()), "error": errore })
}

async fn get_events(inizio: Option<&Json>, fine: Option<&Json>, per_promemoria: Option<&Json>, spenti: Option<&Json>) -> Json {
    let t0 = ora_vera();
    let esito: Esito<Json> = async {
        let mut cfg = load_config()?;
        if let Some(Json::Arr(v)) = spenti {
            if v.iter().all(|x| x.as_str().is_some()) {
                cfg.insert("disabled_calendars", Json::Arr(v.clone()));
            }
        }
        let mut cfg = if per_promemoria == Some(&Json::Bool(true)) { tutti_accesi(&cfg) } else { cfg };
        let (da, a) = match (estremo(inizio), estremo(fine)) {
            (Ok(d), Ok(a)) => (d, a),
            (Err(m), _) | (_, Err(m)) => return Err(Errore::tipo(m)),
        };
        let (events, errors, meta) = raccogli(&mut cfg, &da, &a, true, true).await?;
        println!(
            "get_events: {} eventi in {:.1}s (stale={}, errori: {})",
            events.len(),
            (ora_vera() - t0) as f64 / 1000.0,
            py_bool(meta.get("stale")),
            Json::Arr(errors.iter().cloned().map(Json::Str).collect()).in_js()
        );
        let mut m = Mappa::new();
        m.insert("events", Json::Arr(events.into_iter().map(Json::Obj).collect()));
        m.insert("errors", Json::Arr(errors.into_iter().map(Json::Str).collect()));
        for (k, v) in meta.iter() {
            m.insert(k.clone(), v.clone());
        }
        Ok(Json::Obj(m))
    }
    .await;
    esito.unwrap_or_else(|e| {
        eprintln!("get_events: eccezione {}: {}", e.nome(), e.messaggio());
        ogg! { "events": Json::Arr(vec![]), "errors": Json::Arr(vec![Json::Str(e.messaggio().to_string())]), "stale": false, "stale_at": Json::Null, "outlook_starting": false }
    })
}

async fn get_disponibilita(da: Option<&Json>, giorni: Option<&Json>, gruppo: Option<&Json>) -> Esito<Json> {
    let letti = (|| {
        let primo = py_data(da)?;
        let n = match giorni {
            None => 7,
            g => py_int(g)?,
        };
        if !(1..=GIORNI_MAX).contains(&n) {
            return Err(argomenti("giorni"));
        }
        Ok((primo, n))
    })();
    let (primo, n) = match letti {
        Ok(x) => x,
        Err(Errore::Argomenti(_)) => return Ok(no("Intervallo non valido")),
        Err(e) => return Err(e),
    };
    let gruppo = if py_bool(gruppo) { py_str(gruppo) } else { String::new() };
    match occupato(&load_config()?, primo, n, &gruppo).await {
        Ok((impegni, festivita, completo)) => Ok(ogg! {
            "ok": true,
            "impegni": Json::Arr(impegni.into_iter().map(|[a, b]| Json::Arr(vec![Json::Str(a), Json::Str(b)])).collect()),
            "festivita": Json::Arr(festivita.into_iter().map(|[a, b, c]| Json::Arr(vec![Json::Str(a), Json::Str(b), Json::Str(c)])).collect()),
            "completo": completo,
        }),
        Err(e) if e.messaggio() == "Gruppo non trovato" => Ok(no("Gruppo non trovato")),
        Err(e) => {
            eprintln!("get_disponibilita: eccezione {}: {}", e.nome(), e.messaggio());
            Ok(no("Calendari non leggibili"))
        }
    }
}

/// Il nome da mostrare per un account. Vuoto = torna a quello automatico. Le aziende nei gruppi, e il loro logo, seguono il nuovo nome.
fn set_account_alias(account: Option<&Json>, alias: Option<&Json>) -> Esito<Json> {
    let Some(Json::Str(conto)) = account else { return Err(Errore::tipo("indirizzo.toLowerCase is not a function")) };
    let nuovo = testo_pulito(alias)?;
    modifica_config(|cfg| {
        if !is_dict(cfg.get("account_aliases")) {
            cfg.insert("account_aliases", Json::obj());
        }
        let prima = alias_per_account(cfg, [conto.as_str()]).get(conto).cloned().unwrap_or_default();
        if let Some(Json::Obj(scelti)) = cfg.get_mut("account_aliases") {
            if nuovo.is_empty() {
                scelti.remove(conto);
            } else {
                scelti.insert(conto.clone(), nuovo.clone());
            }
        }
        let dopo = alias_per_account(cfg, [conto.as_str()]).get(conto).cloned().unwrap_or_default();
        rinomina_nei_gruppi(cfg, &prima, &dopo);
        rinomina_logo(&prima, &dopo)?;
        // la scelta di "secondario" segue il nome, come i gruppi e il logo; se il nome nuovo ne ha già una vale quella
        if prima != dopo {
            if let Some(Json::Obj(sec)) = cfg.get_mut("secondari_scelti") {
                if let Some(v) = sec.get(&prima).cloned() {
                    if !sec.contains_key(&dopo) {
                        sec.insert(dopo.clone(), v);
                    }
                    sec.remove(&prima);
                }
            }
        }
        Ok(())
    })?;
    pulisci_loghi();
    Ok(ok())
}

/// Elimina dalla vista un evento delle caselle o dei link, che da qui non si possono cambiare: resta nella configurazione (eventi_eliminati).
fn elimina_evento(evento: Option<&Json>) -> Esito<Json> {
    let e = match evento {
        Some(Json::Obj(m)) => m.clone(),
        _ => Mappa::new(),
    };
    let Some(chiave) = chiave_eliminato(&e)? else { return Ok(no("Evento non riconosciuto")) };
    let testo = |v: Option<&Json>| match v {
        Some(Json::Str(s)) => taglia_js(s, 300).to_string(),
        _ => String::new(),
    };
    let alias = if py_bool(e.get("alias")) { e.get("alias") } else { e.get("account") };
    let voce = ogg! {
        "chiave": chiave.clone(),
        "title": testo(e.get("title")),
        "start": testo(e.get("start")),
        "end": testo(e.get("end")),
        "allDay": e.get("allDay") == Some(&Json::Bool(true)),
        "alias": testo(alias),
        "eliminato_il": iso_ms(adesso()),
    };
    let chiave_json = Json::Str(chiave.clone());
    let eliminati = modifica_config(|cfg| {
        let mut v: Vec<Json> = crate::py::lista(cfg.get("eventi_eliminati")).iter().filter(|x| !uguale(x.get("chiave"), Some(&chiave_json))).cloned().collect();
        v.push(voce);
        cfg.insert("eventi_eliminati", Json::Arr(v.clone()));
        Ok(v)
    })?;
    println!("elimina_evento: {} eliminati", eliminati.len());
    Ok(ogg! { "ok": true, "chiave": chiave, "eventi_eliminati": Json::Arr(eliminati) })
}

async fn get_stato_caselle() -> Esito<Json> {
    let r = chiedi("/api/auth/stato", &[]).await?;
    let p = personalizzazione();
    let sostituiti = inoltri_sostituiti(&load_config()?);
    let mut caselle = Vec::new();
    for c in crate::py::lista(r.get("caselle")) {
        if !py_bool(c.get("account")) || p.account_nascosti.contains(&string_js(c.get("account")).to_lowercase()) {
            continue;
        }
        let account = string_js(c.get("account")).to_lowercase();
        let inoltrate: Vec<Json> = p.aziende_inoltrate.iter().filter(|(az, d)| d.via.to_lowercase() == account && !sostituiti.contains(*az)).map(|(az, _)| Json::Str(az.clone())).collect();
        let mut m = match c {
            Json::Obj(m) => m.clone(),
            _ => Mappa::new(),
        };
        m.insert("inoltrate", Json::Arr(inoltrate));
        caselle.push(Json::Obj(m));
    }
    Ok(ogg! { "ok": true, "caselle": Json::Arr(caselle) })
}

/// Un corpo per l'agente: le voci undefined non si scrivono, come JSON.stringify.
fn corpo(voci: &[(&str, Option<&Json>)]) -> Json {
    let mut m = Mappa::new();
    for (k, v) in voci {
        if let Some(v) = v {
            m.insert(*k, (*v).clone());
        }
    }
    Json::Obj(m)
}

async fn connect_microsoft(account: Option<&Json>, capacita: Option<&Json>) -> Esito<Json> {
    let attuale = chiedi("/api/auth/shared", &[]).await?;
    if !py_bool(attuale.get("ok")) || !matches!(attuale.get("signins"), Some(Json::Arr(_))) {
        return Err(Errore::valore("Microsoft sign-in service is upgrading or unavailable"));
    }
    let op = Json::Str("connect".to_string());
    let r = invia("/api/auth/shared", &corpo(&[("account", account), ("operation", Some(&op)), ("capabilities", capacita)])).await?;
    Ok(if py_bool(r.get("ok")) { ok() } else { no(testo_o(r.get("errore"), "Sign-in could not start")) })
}

async fn cancel_signin(account: Option<&Json>, id: Option<&Json>) -> Esito<Json> {
    if !matches!(account, Some(Json::Str(_))) || !matches!(id, Some(Json::Str(_))) {
        return Err(Errore::valore("Invalid sign-in"));
    }
    let op = Json::Str("signin-cancel".to_string());
    let r = invia("/api/auth/shared", &corpo(&[("account", account), ("id", id), ("operation", Some(&op))])).await?;
    Ok(if py_bool(r.get("ok")) { ok() } else { no(testo_o(r.get("errore"), "Sign-in not cancelled")) })
}

async fn manage_microsoft(account: Option<&Json>, operazione: Option<&Json>, on: Option<&Json>) -> Esito<Json> {
    if !matches!(account, Some(Json::Str(_))) || !["remove", "chat", "calls"].contains(&string_js(operazione).as_str()) {
        return Err(Errore::valore("Invalid operation"));
    }
    let r = invia("/api/auth/shared", &corpo(&[("account", account), ("operation", operazione), ("on", on)])).await?;
    if !py_bool(r.get("ok")) {
        return Ok(no(testo_o(r.get("errore"), "Operation failed")));
    }
    azzera_memoria();
    Ok(ogg! { "ok": true, "pending": true })
}

fn lista_o_vuota(v: Option<&Json>) -> Json {
    match v {
        Some(Json::Arr(a)) => Json::Arr(a.clone()),
        _ => Json::Arr(vec![]),
    }
}

async fn get_condivisione_microsoft() -> Esito<Json> {
    let r = chiedi("/api/auth/shared", &[]).await?;
    if !py_bool(r.get("ok")) {
        return Ok(ogg! { "ok": false, "accounts": Json::Arr(vec![]), "teamsOnly": Json::Arr(vec![]), "teamsAt": 0, "error": testo_o(r.get("errore"), "Stato degli account non letto") });
    }
    let etichette = etichette_account(&indirizzi_elenco(&r));
    manda_etichette(&etichette).await;
    Ok(ogg! {
        "ok": true,
        "labels": etichette.json(),
        "accounts": lista_o_vuota(r.get("accounts")),
        "teamsOnly": lista_o_vuota(r.get("teamsOnly")),
        "commands": lista_o_vuota(r.get("commands")),
        "signins": lista_o_vuota(r.get("signins")),
        "teamsAt": match r.get("teamsAt") { Some(Json::Num(n)) => *n, _ => 0.0 },
    })
}

/// `x.trim().toLowerCase()` di un campo che deve essere un testo, o il TypeError di JavaScript.
fn chiave_testo(v: Option<&Json>, campo: &str) -> Esito<String> {
    match v {
        Some(Json::Str(s)) => Ok(js_trim(s).to_lowercase()),
        _ => Err(Errore::tipo(format!("Cannot read properties of undefined (reading '{campo}')"))),
    }
}

async fn set_condivisione_microsoft(account: Option<&Json>, capacita: Option<&Json>) -> Esito<Json> {
    let casella = py_str(account);
    let spegni = capacita == Some(&Json::Null);
    if !spegni && !matches!(capacita, Some(Json::Arr(_))) {
        return Err(Errore::valore("capacità: un elenco, o null per spegnere"));
    }
    if let Some(Json::Arr(c)) = capacita {
        if c.iter().any(|x| x.as_str() == Some("chat")) {
            let ultimo = chiedi("/api/auth/shared", &[]).await?;
            if !py_bool(ultimo.get("ok")) {
                return Err(Errore::valore("Teams report unavailable"));
            }
            let chiave = js_trim(&casella).to_lowercase();
            let Some(Json::Arr(accounts)) = ultimo.get("accounts") else { return Err(Errore::tipo("latest.accounts.find is not a function")) };
            let mut attuale = None;
            for x in accounts {
                if chiave_testo(x.get("account"), "trim")? == chiave {
                    attuale = Some(x);
                    break;
                }
            }
            let ha_chat = attuale.and_then(|x| x.get("shared")).and_then(|s| s.get("capabilities")).is_some_and(|c| crate::py::lista(Some(c)).iter().any(|x| x.as_str() == Some("chat")));
            if !ha_chat {
                let at = crate::py::numero(ultimo.get("teamsAt"));
                let ora_s = ora_vera() as f64 / 1000.0;
                if at == 0.0 || at.is_nan() || ora_s - at > 600.0 || at > ora_s {
                    return Err(Errore::valore("Teams report unavailable. Cannot enable safely."));
                }
                let Some(Json::Arr(altri)) = ultimo.get("teamsOnly") else { return Err(Errore::tipo("others.some is not a function")) };
                for t in altri {
                    if chiave_testo(t.get("email"), "trim")? == chiave {
                        return Err(Errore::valore("Existing browser sign-in preserved. Shared Teams cannot take over this slot."));
                    }
                }
            }
        }
    }
    let cas = Json::Str(casella.clone());
    let falso = Json::Bool(false);
    let c = if spegni { corpo(&[("account", Some(&cas)), ("enabled", Some(&falso))]) } else { corpo(&[("account", Some(&cas)), ("capabilities", capacita)]) };
    let r = invia("/api/auth/shared", &c).await?;
    if !py_bool(r.get("ok")) {
        return Ok(no(testo_o(r.get("errore"), "Condivisione non cambiata")));
    }
    azzera_memoria();
    let descrizione = if spegni { "spenta".to_string() } else { string_js(capacita) };
    println!("set_condivisione_microsoft: {} {descrizione}", string_js(r.get("account")));
    Ok(ogg! { "ok": true, "account": testo_o_vuoto(r.get("account")), "shared": match r.get("shared") { None | Some(Json::Null) => Json::Null, Some(x) => x.clone() } })
}

async fn rimuovi_casella(account: Option<&Json>) -> Esito<Json> {
    let r = invia("/api/auth/rimuovi", &ogg! { "account": py_str(account) }).await?;
    if !py_bool(r.get("ok")) {
        return Ok(no(testo_o(r.get("errore"), "Casella non rimossa")));
    }
    azzera_memoria();
    println!("rimuovi_casella: {}", string_js(r.get("account")));
    Ok(ogg! { "ok": true, "account": testo_o_vuoto(r.get("account")) })
}

async fn inizia_accesso(email: Option<&Json>, provider: Option<&Json>, modo: Option<&Json>) -> Esito<Json> {
    let r = invia(
        "/api/auth/inizio",
        &ogg! {
            "email": match email { Some(Json::Str(s)) => js_trim(s).to_string(), _ => String::new() },
            "provider": if provider.and_then(Json::as_str) == Some("google") { "google" } else { "microsoft" },
            "modo": if modo.and_then(Json::as_str) == Some("device") { "device" } else { "pkce" },
        },
    )
    .await?;
    if !py_bool(r.get("ok")) || !matches!(r.get("url"), Some(Json::Str(_))) {
        return Ok(no(testo_o(r.get("errore"), "Avvio login non riuscito")));
    }
    let mut m = Mappa::new();
    m.insert("ok", true);
    m.insert("provider", if r.get("provider").and_then(Json::as_str) == Some("google") { "google" } else { "microsoft" });
    m.insert("modo", if r.get("modo").and_then(Json::as_str) == Some("device") { "device" } else { "pkce" });
    m.insert("url", r.get("url").cloned().unwrap_or_default());
    m.insert("state", testo_o_vuoto(r.get("state")));
    if let Some(Json::Str(c)) = r.get("user_code") {
        m.insert("user_code", c.clone());
    }
    if let Some(Json::Num(n)) = r.get("intervallo_s") {
        m.insert("intervallo_s", *n);
    }
    if let Some(Json::Str(u)) = r.get("url_pkce") {
        m.insert("url_pkce", u.clone());
    }
    Ok(Json::Obj(m))
}

async fn controlla_accesso(state: Option<&Json>) -> Esito<Json> {
    let s = match state {
        Some(Json::Str(s)) => js_trim(s).to_string(),
        _ => String::new(),
    };
    let r = invia("/api/auth/controlla", &ogg! { "state": s }).await?;
    if !py_bool(r.get("ok")) {
        return Ok(no(testo_o(r.get("errore"), "Controllo accesso non riuscito")));
    }
    if r.get("stato").and_then(Json::as_str) == Some("completato") {
        azzera_memoria();
        return Ok(ogg! { "ok": true, "stato": "completato", "account": testo_o_vuoto(r.get("account")), "provider": match r.get("provider") { None | Some(Json::Null) => "microsoft".to_string(), x => string_js(x) } });
    }
    Ok(ogg! { "ok": true, "stato": "in_attesa" })
}

async fn completa_accesso(dati: Option<&Json>) -> Esito<Json> {
    let Some(d @ Json::Obj(_)) = dati else { return Ok(no("Dati non validi")) };
    let r = invia("/api/auth/completa", d).await?;
    if !py_bool(r.get("ok")) {
        return Ok(no(testo_o(r.get("errore"), "Accesso non riuscito")));
    }
    azzera_memoria();
    Ok(ogg! { "ok": true, "account": testo_o_vuoto(r.get("account")), "provider": match r.get("provider") { None | Some(Json::Null) => "microsoft".to_string(), x => string_js(x) } })
}
