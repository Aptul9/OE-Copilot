#[cfg(any(test, target_os = "android"))]
use std::collections::HashSet;

use jiff::tz::{Offset, TimeZone};
use serde::Serialize;
use serde_json::Value;

// Il nucleo dei promemoria, lo stesso per Windows e Android: dalle impostazioni (get_promemoria) e dagli eventi (get_events) le voci
// da avvisare, una per riunione e anticipo, con l'istante dell'avviso e il testo. Niente rete e niente stato: legge l'API chi lo
// chiama (lo script della pagina su Windows, il lavoro in background su Android, Promemoria.kt) e decide cosa mostrare.

pub const FUSO: &str = "Europe/Rome";
/// Sul PC lo 0 ("all'inizio") vale nell'ultimo minuto e mezzo: il giro della pagina passa ogni 60 secondi più la risposta
pub const FINESTRA_INIZIO_GIRO_MS: i64 = 90_000;
const MINUTI_MAX: i64 = 1440;
const MINUTI_PREDEFINITI: i64 = 10;
const LUNGHEZZA_MAX: usize = 120;

/// Una riunione e un anticipo: quando avvisare e cosa scrivere. I campi sono quelli che leggono la pagina e Promemoria.kt
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Voce {
    pub chiave: String,
    pub riunione: String,
    pub minuti: i64,
    pub at: i64,
    pub inizio: i64,
    pub ora: String,
    pub quando: String,
    pub chi: String,
    pub titolo: String,
    pub corpo: String,
    pub azienda: String,
    pub colore: String,
    pub allarme: i32,
    pub notifica: i32,
}

/// Android: cosa fare degli allarmi dopo una lettura, e le chiavi da ricordare per la volta dopo
#[cfg(any(test, target_os = "android"))]
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Azioni {
    pub programma: Vec<Voce>,
    pub subito: Vec<Voce>,
    pub annulla: Vec<i32>,
    pub chiavi: Vec<String>,
}

fn cifre(testo: &str) -> Option<i32> {
    if testo.is_empty() || !testo.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    testo.parse().ok()
}

/// L'istante (ms dal 1970) di un orario dell'API, AAAA-MM-GG[T ]HH:MM[:SS[.fff]] con lo scarto (Z, +02:00, +0200) o senza: senza è
/// ora di Roma, come lo manda l'agente. L'ora che il cambio ripete vale la prima volta, quella che salta vale come prima del salto
fn istante(testo: &str, roma: &TimeZone) -> Option<i64> {
    let (data, resto) = (testo.get(..10)?, testo.get(10..)?);
    let resto = resto.strip_prefix('T').or_else(|| resto.strip_prefix(' '))?;
    let (orario, scarto) = match resto.find(['Z', '+', '-']) {
        Some(i) => (&resto[..i], Some(&resto[i..])),
        None => (resto, None),
    };
    let mut d = data.split('-');
    let (a, m, g) = (d.next()?, d.next()?, d.next()?);
    if d.next().is_some() || a.len() != 4 || m.len() != 2 || g.len() != 2 {
        return None;
    }
    let orario = match orario.split_once('.') {
        Some((o, frazione)) if cifre(frazione).is_some() => o,
        Some(_) => return None,
        None => orario,
    };
    let mut o = orario.split(':');
    let (h, mi, s) = (o.next()?, o.next()?, o.next().unwrap_or("00"));
    if o.next().is_some() || [h, mi, s].iter().any(|x| x.len() != 2) {
        return None;
    }
    let civile = jiff::civil::DateTime::new(cifre(a)? as i16, cifre(m)? as i8, cifre(g)? as i8, cifre(h)? as i8, cifre(mi)? as i8, cifre(s)? as i8, 0).ok()?;
    let fuso = match scarto {
        None => roma.clone(),
        Some("Z") => TimeZone::UTC,
        Some(scarto) => {
            let segno = if scarto.starts_with('-') { -1 } else { 1 };
            let hhmm = scarto[1..].replacen(':', "", 1);
            if hhmm.len() != 4 {
                return None;
            }
            TimeZone::fixed(Offset::from_seconds(segno * (cifre(&hhmm[..2])? * 3600 + cifre(&hhmm[2..])? * 60)).ok()?)
        }
    };
    Some(civile.to_zoned(fuso).ok()?.timestamp().as_millisecond())
}

/// Un campo di un evento come testo, come String() nella pagina; un campo che manca è vuoto
fn campo(ev: &Value, nome: &str) -> String {
    match ev.get(nome) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Vero come in JavaScript: true, un numero diverso da 0, un testo non vuoto, un oggetto
fn vero(valore: Option<&Value>) -> bool {
    match valore {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn taglia(testo: &str) -> String {
    testo.chars().take(LUNGHEZZA_MAX).collect()
}

/// Gli anticipi in minuti (da 0 a 1440, ordinati, senza doppioni; 10 se non ce n'è uno buono) e "Solo ora e azienda" dalle impostazioni
/// di get_promemoria. Senza impostazioni (un server senza il metodo) valgono i valori di partenza
fn leggi_impostazioni(impostazioni: Option<&Value>) -> (Vec<i64>, bool) {
    let mut anticipi: Vec<i64> = impostazioni
        .and_then(|p| p.get("minuti"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.as_i64().or_else(|| m.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)))
        .filter(|m| (0..=MINUTI_MAX).contains(m))
        .collect();
    anticipi.sort_unstable();
    anticipi.dedup();
    if anticipi.is_empty() {
        anticipi.push(MINUTI_PREDEFINITI);
    }
    (anticipi, impostazioni.and_then(|p| p.get("contenuto")).and_then(Value::as_str) == Some("minimo"))
}

/// Le voci di ogni riunione a orario non ancora iniziata e di ogni anticipo, in ordine di inizio. L'avviso di un anticipo cade `minuti`
/// prima dell'inizio, quello dello 0 `finestra_zero` prima (sul PC FINESTRA_INIZIO_GIRO_MS, su Android 0: l'allarme è esatto). Il testo
/// vale al momento dell'avviso, o adesso se quel momento è già passato: "Tra 5 min" dice quanto manca davvero. Con "Solo ora e azienda"
/// al posto del titolo c'è l'azienda e il luogo no; il link per entrare non va mai nell'avviso
pub fn pianifica(impostazioni: Option<&Value>, eventi: &Value, ora: i64, finestra_zero: i64) -> Vec<Voce> {
    let Ok(roma) = TimeZone::get(FUSO) else {
        return Vec::new();
    };
    let (anticipi, minimo) = leggi_impostazioni(impostazioni);
    let mut voci = Vec::new();
    for ev in eventi.as_array().into_iter().flatten().filter(|ev| ev.is_object()) {
        if vero(ev.get("allDay")) || vero(ev.get("nascosto")) {
            continue;
        }
        let start = campo(ev, "start");
        let Some(inizio) = istante(&start, &roma).filter(|&inizio| inizio > ora) else {
            continue;
        };
        let riunione = format!("{}|{start}", campo(ev, "id"));
        let Ok(alle) = jiff::Timestamp::from_millisecond(inizio).map(|t| t.to_zoned(roma.clone())) else {
            continue;
        };
        let alle = format!("{:02}:{:02}", alle.hour(), alle.minute());
        let titolo_evento = campo(ev, "title");
        let chi = taglia(if minimo { campo(ev, "alias") } else if titolo_evento.is_empty() { "(senza titolo)".into() } else { titolo_evento }.as_str());
        let mut corpo = format!("Alle {alle}");
        let luogo = campo(ev, "location");
        let luogo = luogo.trim();
        let con_link = luogo.to_ascii_lowercase().contains("http://") || luogo.to_ascii_lowercase().contains("https://");
        if !minimo && !luogo.is_empty() && !con_link {
            corpo = format!("{corpo}\n{}", taglia(luogo));
        }
        let alias = campo(ev, "alias");
        let azienda = if alias.is_empty() { campo(ev, "account") } else { alias };
        for &minuti in &anticipi {
            let at = inizio - if minuti == 0 { finestra_zero } else { minuti * 60_000 };
            let mancano = ((inizio - at.max(ora)) as f64 / 60_000.0).round() as i64;
            let quando = if mancano < 1 { "Adesso".to_string() } else { format!("Tra {mancano} min") };
            let titolo = if chi.is_empty() { quando.clone() } else { format!("{quando} · {chi}") };
            let chiave = format!("{riunione}|{minuti}");
            voci.push(Voce {
                allarme: codice(&chiave),
                notifica: codice(&riunione),
                chiave,
                riunione: riunione.clone(),
                minuti,
                at,
                inizio,
                ora: alle.clone(),
                quando,
                chi: chi.clone(),
                titolo,
                corpo: corpo.clone(),
                azienda: azienda.clone(),
                colore: campo(ev, "color"),
            });
        }
    }
    voci.sort_by_key(|v| v.inizio);
    voci
}

/// Un numero stabile da un testo (FNV-1a a 32 bit): il codice dell'allarme di una voce e l'id della notifica di una riunione su Android
fn codice(testo: &str) -> i32 {
    testo.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193)) as i32
}

/// Android: cosa fare degli allarmi dopo una lettura. Le voci future si programmano (lo stesso codice prende il posto dell'allarme di
/// prima); una voce già passata con la riunione non ancora iniziata parte subito, una volta per riunione, se non era mai stata vista
/// (riunione creata all'ultimo, app appena installata); gli allarmi delle voci che non sono più future si annullano. `chiavi` è quello
/// che l'app ricorda per la volta dopo
// ponytail: una voce già vista non riparte nemmeno se il suo allarme si è perso con un riavvio prima della lettura dopo; se serve, il
// ricevitore dell'allarme segna le voci mostrate e qui contano quelle
#[cfg(any(test, target_os = "android"))]
pub fn azioni_android(voci: Vec<Voce>, gia: &[String], ora: i64) -> Azioni {
    let gia: HashSet<&str> = gia.iter().map(String::as_str).collect();
    let mut azioni = Azioni::default();
    let mut avvisate = HashSet::new();
    for voce in voci {
        azioni.chiavi.push(voce.chiave.clone());
        if voce.at > ora {
            azioni.programma.push(voce);
        } else if !gia.contains(voce.chiave.as_str()) && avvisate.insert(voce.riunione.clone()) {
            azioni.subito.push(voce);
        }
    }
    let future: HashSet<&str> = azioni.programma.iter().map(|v| v.chiave.as_str()).collect();
    azioni.annulla = gia.into_iter().filter(|chiave| !future.contains(chiave)).map(codice).collect();
    azioni
}

/// Il lavoro in background di Android (Promemoria.kt, NucleoPromemoria.aggiorna): in entrata le impostazioni e gli eventi come li dà
/// l'API, l'ora e le chiavi ricordate dalla volta prima, in JSON; in uscita le Azioni in JSON. Su Android lo 0 vale all'ora esatta
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_oecopilot_app_NucleoPromemoria_aggiorna<'a>(
    mut env: jni::JNIEnv<'a>,
    _classe: jni::objects::JClass<'a>,
    impostazioni: jni::objects::JString<'a>,
    eventi: jni::objects::JString<'a>,
    ora: jni::sys::jlong,
    gia: jni::objects::JString<'a>,
) -> jni::sys::jstring {
    let mut json = |testo: &jni::objects::JString<'a>| -> Value {
        env.get_string(testo).ok().and_then(|t| serde_json::from_str(&String::from(t)).ok()).unwrap_or(Value::Null)
    };
    let impostazioni = json(&impostazioni);
    let eventi = json(&eventi);
    let gia: Vec<String> = serde_json::from_value(json(&gia)).unwrap_or_default();
    let azioni = azioni_android(pianifica(Some(&impostazioni).filter(|p| !p.is_null()), &eventi, ora, 0), &gia, ora);
    env.new_string(serde_json::to_string(&azioni).unwrap_or_default()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod prove {
    use super::*;
    use serde_json::json;

    const MIN: i64 = 60_000;

    fn utc(a: i16, m: i8, g: i8, h: i8, mi: i8) -> i64 {
        jiff::civil::DateTime::new(a, m, g, h, mi, 0, 0).unwrap().to_zoned(TimeZone::UTC).unwrap().timestamp().as_millisecond()
    }

    fn evento(id: &str, titolo: &str, start: &str) -> Value {
        json!({ "id": id, "title": titolo, "start": start, "end": start, "allDay": false, "location": "" })
    }

    fn con(mut ev: Value, campo: &str, valore: Value) -> Value {
        ev[campo] = valore;
        ev
    }

    fn roma() -> TimeZone {
        TimeZone::get(FUSO).unwrap()
    }

    #[test]
    fn orari_dell_api_in_ora_di_roma_o_con_lo_scarto() {
        let dieci_estate = utc(2026, 10, 12, 8, 0);
        for testo in ["2026-10-12T10:00:00", "2026-10-12T10:00", "2026-10-12 10:00:00", "2026-10-12T10:00:00.250", "2026-10-12T10:00:00+02:00", "2026-10-12T10:00:00+0200", "2026-10-12T08:00:00Z"] {
            assert_eq!(istante(testo, &roma()), Some(dieci_estate), "{testo}");
        }
        // inverno: UTC+1
        assert_eq!(istante("2026-12-01T10:00:00", &roma()), Some(utc(2026, 12, 1, 9, 0)));
        assert_eq!(istante("2026-12-01T10:00:00-05:00", &roma()), Some(utc(2026, 12, 1, 15, 0)));
        // l'ora che il cambio ripete vale la prima volta, come sul server; quella che salta vale come prima del salto
        assert_eq!(istante("2026-10-25T02:30:00", &roma()), Some(utc(2026, 10, 25, 0, 30)));
        assert_eq!(istante("2026-03-29T02:30:00", &roma()), Some(utc(2026, 3, 29, 1, 30)));
        for illeggibile in ["", "domani", "2026-10-12", "2026-10-12T10", "2026-10-12X10:00:00", "2026-13-01T10:00:00", "2026-10-12T10:00:00+2"] {
            assert_eq!(istante(illeggibile, &roma()), None, "{illeggibile}");
        }
    }

    #[test]
    fn una_voce_per_riunione_e_anticipo_con_il_testo_della_notifica() {
        // 09:55 a Roma (UTC+2), riunione alle 10:00 senza scarto, anticipo predefinito di 10 minuti: già raggiunto
        let adesso = utc(2026, 10, 12, 7, 55);
        let eventi = json!([con(evento("e1", "Riunione A", "2026-10-12T10:00:00"), "location", json!("Sala 1"))]);
        let voci = pianifica(None, &eventi, adesso, FINESTRA_INIZIO_GIRO_MS);
        assert_eq!(
            voci,
            vec![Voce {
                chiave: "e1|2026-10-12T10:00:00|10".into(),
                riunione: "e1|2026-10-12T10:00:00".into(),
                minuti: 10,
                at: utc(2026, 10, 12, 7, 50),
                inizio: utc(2026, 10, 12, 8, 0),
                ora: "10:00".into(),
                quando: "Tra 5 min".into(),
                chi: "Riunione A".into(),
                titolo: "Tra 5 min · Riunione A".into(),
                corpo: "Alle 10:00\nSala 1".into(),
                azienda: "".into(),
                colore: "".into(),
                allarme: codice("e1|2026-10-12T10:00:00|10"),
                notifica: codice("e1|2026-10-12T10:00:00"),
            }]
        );
    }

    #[test]
    fn il_testo_vale_al_momento_dell_avviso_o_adesso_se_e_gia_passato() {
        let inizio = utc(2026, 10, 12, 8, 0);
        let eventi = json!([evento("e1", "Riunione F", "2026-10-12T10:00:00")]);
        let p = json!({ "minuti": [10, 30] });
        // mezz'ora prima di tutto: ogni voce parla del suo anticipo
        let voci = pianifica(Some(&p), &eventi, inizio - 45 * MIN, FINESTRA_INIZIO_GIRO_MS);
        assert_eq!(voci.iter().map(|v| (v.minuti, v.at, v.quando.as_str())).collect::<Vec<_>>(), [(10, inizio - 10 * MIN, "Tra 10 min"), (30, inizio - 30 * MIN, "Tra 30 min")]);
        // a 20 minuti: quella dei 30 è già passata e parla di adesso, quella dei 10 no
        let voci = pianifica(Some(&p), &eventi, inizio - 20 * MIN, FINESTRA_INIZIO_GIRO_MS);
        assert_eq!(voci.iter().map(|v| v.quando.as_str()).collect::<Vec<_>>(), ["Tra 10 min", "Tra 20 min"]);
    }

    #[test]
    fn all_inizio_un_minuto_e_mezzo_prima_sul_pc_e_all_ora_esatta_su_android() {
        let inizio = utc(2026, 10, 12, 8, 0);
        let eventi = json!([evento("e1", "Riunione H", "2026-10-12T10:00:00")]);
        let p = json!({ "minuti": [0] });
        let pc = |ora| pianifica(Some(&p), &eventi, ora, FINESTRA_INIZIO_GIRO_MS).remove(0);
        assert_eq!(pc(inizio - 2 * MIN).at, inizio - 90_000);
        assert_eq!(pc(inizio - 85_000).titolo, "Tra 1 min · Riunione H");
        assert_eq!(pc(inizio - 20_000).titolo, "Adesso · Riunione H");
        let android = pianifica(Some(&p), &eventi, inizio - 2 * MIN, 0).remove(0);
        assert_eq!((android.at, android.titolo.as_str()), (inizio, "Adesso · Riunione H"));
    }

    #[test]
    fn niente_voci_per_giornata_intera_nascosti_gia_iniziati_e_senza_orario() {
        let adesso = utc(2026, 10, 12, 7, 55);
        let eventi = json!([
            con(evento("a", "Tutto il giorno", "2026-10-12T00:00:00"), "allDay", json!(true)),
            con(evento("b", "Nascosto", "2026-10-12T10:00:00"), "nascosto", json!(true)),
            evento("c", "Già iniziato", "2026-10-12T09:50:00"),
            evento("d", "Adesso", "2026-10-12T09:55:00"),
            evento("e", "Senza orario", ""),
            evento("f", "Orario illeggibile", "domani"),
            json!(null),
            json!("non un evento"),
            evento("g", "Lontano", "2026-10-12T11:00:00"),
        ]);
        let voci = pianifica(None, &eventi, adesso, FINESTRA_INIZIO_GIRO_MS);
        // il lontano c'è, con l'avviso alle 10:50: decide chi chiama quando mostrarlo
        assert_eq!(voci.iter().map(|v| (v.chiave.as_str(), v.at)).collect::<Vec<_>>(), [("g|2026-10-12T11:00:00|10", utc(2026, 10, 12, 8, 50))]);
        assert!(pianifica(None, &json!({ "events": [] }), adesso, 0).is_empty());
    }

    #[test]
    fn anticipi_dal_server_ordinati_senza_doppioni_e_senza_valori_strani() {
        let inizio = utc(2026, 10, 12, 8, 0);
        let eventi = json!([evento("e1", "Riunione F", "2026-10-12T10:00:00")]);
        let minuti = |p: Option<Value>| pianifica(p.as_ref(), &eventi, inizio - 2 * 1440 * MIN, 0).iter().map(|v| v.minuti).collect::<Vec<_>>();
        assert_eq!(minuti(Some(json!({ "minuti": [30, 10, 10, "x", -5, 0, 2.5, 1441, 1440, null, true] }))), [0, 10, 30, 1440]);
        // senza impostazioni (server vecchio, 404), con una lista vuota o tutta strana: 10 minuti
        for p in [None, Some(json!({})), Some(json!({ "minuti": [] })), Some(json!({ "minuti": ["x", -1] })), Some(json!({ "minuti": "10" })), Some(json!("strano"))] {
            assert_eq!(minuti(p.clone()), [10], "{p:?}");
        }
    }

    #[test]
    fn solo_ora_e_azienda_al_posto_di_titolo_e_luogo() {
        let adesso = utc(2026, 10, 12, 7, 55);
        let p = json!({ "contenuto": "minimo" });
        let ev = con(con(evento("e1", "Segreto", "2026-10-12T10:00:00"), "alias", json!("ROAR")), "location", json!("Sala 1"));
        let v = pianifica(Some(&p), &json!([ev]), adesso, 0).remove(0);
        assert_eq!((v.chi.as_str(), v.titolo.as_str(), v.corpo.as_str()), ("ROAR", "Tra 5 min · ROAR", "Alle 10:00"));
        let senza_azienda = pianifica(Some(&p), &json!([evento("e2", "Segreto", "2026-10-12T10:00:00")]), adesso, 0).remove(0);
        assert_eq!((senza_azienda.chi.as_str(), senza_azienda.titolo.as_str()), ("", "Tra 5 min"));
        assert!(!format!("{v:?}{senza_azienda:?}").contains("Segreto"));
    }

    #[test]
    fn luogo_senza_link_testi_tagliati_e_titolo_mancante() {
        let adesso = utc(2026, 10, 12, 7, 55);
        let lungo = "é".repeat(200);
        let eventi = json!([
            con(evento("e1", "Riunione E", "2026-10-12T10:00:00"), "location", json!("https://teams.microsoft.com/l/meetup-join/x")),
            con(evento("e2", &lungo, "2026-10-12T10:00:00"), "location", json!(format!("  {lungo}  "))),
            json!({ "id": "e3", "start": "2026-10-12T10:00:00" }),
        ]);
        let voci = pianifica(None, &eventi, adesso, 0);
        assert_eq!(voci[0].corpo, "Alle 10:00");
        assert_eq!(voci[1].chi, "é".repeat(120));
        assert_eq!(voci[1].corpo, format!("Alle 10:00\n{}", "é".repeat(120)));
        assert_eq!(voci[2].titolo, "Tra 5 min · (senza titolo)");
    }

    #[test]
    fn in_ordine_di_inizio_con_azienda_colore_e_id_numerici() {
        let adesso = utc(2026, 10, 12, 7, 0);
        let eventi = json!([
            con(con(evento("tardi", "Seconda", "2026-10-12T10:15:00"), "alias", json!("ROAR")), "color", json!("#123456")),
            con(evento("presto", "Prima", "2026-10-12T10:00:00"), "account", json!("a@b.it")),
            { "id": 7, "title": "Numero", "start": "2026-10-12T10:00:00" },
        ]);
        let voci = pianifica(None, &eventi, adesso, 0);
        assert_eq!(voci.iter().map(|v| v.riunione.as_str()).collect::<Vec<_>>(), ["presto|2026-10-12T10:00:00", "7|2026-10-12T10:00:00", "tardi|2026-10-12T10:15:00"]);
        assert_eq!((voci[0].azienda.as_str(), voci[2].azienda.as_str(), voci[2].colore.as_str()), ("a@b.it", "ROAR", "#123456"));
    }

    #[test]
    fn codici_stabili_e_diversi_per_allarmi_e_notifiche() {
        // FNV-1a a 32 bit
        assert_eq!(codice(""), 0x811c9dc5_u32 as i32);
        assert_eq!(codice("a"), 0xe40c292c_u32 as i32);
        assert_ne!(codice("e1|2026-10-12T10:00:00|10"), codice("e1|2026-10-12T10:00:00|30"));
    }

    #[test]
    fn android_programma_il_futuro_avvisa_subito_una_volta_e_annulla_quello_che_sparisce() {
        let inizio = utc(2026, 10, 12, 8, 0);
        let adesso = inizio - 20 * MIN;
        let p = json!({ "minuti": [5, 30, 60] });
        let eventi = json!([evento("e1", "Riunione", "2026-10-12T10:00:00"), evento("e2", "Vecchia", "2026-10-12T11:00:00")]);
        let voci = pianifica(Some(&p), &eventi, adesso, 0);
        // e1 mai vista: 30 e 60 sono passati, si avvisa una volta sola adesso, e 5 si programma; e2 è tutta nel futuro
        let prima = azioni_android(voci.clone(), &[], adesso);
        assert_eq!(prima.subito.iter().map(|v| v.chiave.as_str()).collect::<Vec<_>>(), ["e1|2026-10-12T10:00:00|30"]);
        assert_eq!(prima.subito[0].quando, "Tra 20 min");
        assert_eq!(
            prima.programma.iter().map(|v| v.chiave.as_str()).collect::<Vec<_>>(),
            ["e1|2026-10-12T10:00:00|5", "e2|2026-10-12T11:00:00|5", "e2|2026-10-12T11:00:00|30", "e2|2026-10-12T11:00:00|60"]
        );
        assert!(prima.annulla.is_empty());
        assert_eq!(prima.chiavi.len(), 6);
        // la volta dopo, con e2 sparita: niente di nuovo da avvisare subito, gli allarmi di e2 si annullano
        let solo_e1 = pianifica(Some(&p), &json!([evento("e1", "Riunione", "2026-10-12T10:00:00")]), adesso + MIN, 0);
        let dopo = azioni_android(solo_e1, &prima.chiavi, adesso + MIN);
        assert!(dopo.subito.is_empty());
        assert_eq!(dopo.programma.iter().map(|v| v.chiave.as_str()).collect::<Vec<_>>(), ["e1|2026-10-12T10:00:00|5"]);
        let mut annullati = dopo.annulla.clone();
        annullati.sort();
        let mut attesi: Vec<i32> = ["e2|2026-10-12T11:00:00|5", "e2|2026-10-12T11:00:00|30", "e2|2026-10-12T11:00:00|60", "e1|2026-10-12T10:00:00|30", "e1|2026-10-12T10:00:00|60"].map(codice).to_vec();
        attesi.sort();
        assert_eq!(annullati, attesi);
        assert_eq!(dopo.chiavi, ["e1|2026-10-12T10:00:00|5", "e1|2026-10-12T10:00:00|30", "e1|2026-10-12T10:00:00|60"]);
    }

    #[test]
    fn i_nomi_dei_campi_sono_quelli_che_leggono_pagina_e_android() {
        let adesso = utc(2026, 10, 12, 7, 55);
        let voci = pianifica(None, &json!([evento("e1", "Riunione A", "2026-10-12T10:00:00")]), adesso, 0);
        let voce = serde_json::to_value(&voci[0]).unwrap();
        let mut campi: Vec<&str> = voce.as_object().unwrap().keys().map(String::as_str).collect();
        campi.sort();
        assert_eq!(campi, ["allarme", "at", "azienda", "chi", "chiave", "colore", "corpo", "inizio", "minuti", "notifica", "ora", "quando", "riunione", "titolo"]);
        let azioni = serde_json::to_value(azioni_android(voci, &[], adesso)).unwrap();
        let mut campi: Vec<&str> = azioni.as_object().unwrap().keys().map(String::as_str).collect();
        campi.sort();
        assert_eq!(campi, ["annulla", "chiavi", "programma", "subito"]);
    }
}
