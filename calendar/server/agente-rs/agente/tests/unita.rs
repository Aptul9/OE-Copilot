//! I pezzi riscritti uno per uno contro Python (agente-unita.json), i VTIMEZONE del calendario (ricorrenze-vtimezone.json), le occorrenze
//! annullate e "Mostra come": le prove di `agente-unita`, `ricorrenze-vtimezone`, `occorrenze-annullate` e `occupato` di agente-ts.

mod comune;

use comune::{canonico, db_in_memoria, leggi_golden, primi_utf16};
use oe_agente::fonte::{InfoCasella, estrai, eventi_da_lettura};
use oe_nucleo::android::{RigaAndroid, espandi_righe_android};
use oe_nucleo::entita::HTML5;
use oe_nucleo::errori::{Errore, Esito};
use oe_nucleo::fusi::reimposta_fusi;
use oe_nucleo::html::{html_unescape, pulisci_link};
use oe_nucleo::ics::{EventoIcs, espandi_ics, leggi_iso_locale, valida_ics};
use oe_nucleo::iso::{iso_naive, ora_locale_da};
use oe_nucleo::json::{Json, Mappa};
use oe_nucleo::ogg;
use oe_nucleo::pyfmt::{py_float, py_repr_str, py_strip};
use oe_nucleo::query::{analizza_target, parse_qs};
use oe_nucleo::tempo::{in_fuso, istante, muro, ora, testo_naive};
use sha2::{Digest, Sha256};
use std::rc::Rc;

fn errore_unita(e: &Errore) -> Json {
    match e {
        Errore::SenzaRisposta(m) => {
            let (err, msg) = m.split_once(": ").unwrap_or((m, ""));
            ogg! { "err": err, "msg": msg }
        }
        Errore::Valore(m) => ogg! { "err": "ValueError", "msg": m.as_str() },
        altro => ogg! { "err": "?", "msg": altro.messaggio() },
    }
}

/// Raccoglie le differenze: se ce ne sono, la prova dice le prime cinque.
fn controlla(nome: &str, casi: &Json, f: impl Fn(&Json) -> Esito<Json>) {
    let casi = casi.as_arr().expect("casi");
    assert!(!casi.is_empty(), "{nome}: nessun caso");
    let mut diverse = Vec::new();
    for c in casi {
        let ottenuto = f(&c["in"]).unwrap_or_else(|e| errore_unita(&e));
        let atteso = c.get("ok").or_else(|| c.get("out")).cloned().unwrap_or_else(|| ogg! { "err": c["err"].clone(), "msg": c["msg"].clone() });
        if ottenuto.in_js() != atteso.in_js() {
            diverse.push(format!("in {} atteso {} ottenuto {}", c["in"].in_js(), atteso.in_js(), ottenuto.in_js()));
        }
    }
    assert!(diverse.is_empty(), "{nome}: {} di {} diversi: {:#?}", diverse.len(), casi.len(), &diverse[..diverse.len().min(5)]);
}

#[test]
fn i_pezzi_uno_per_uno_uguali_a_python() {
    let u = leggi_golden("agente-unita.json");
    assert_eq!(HTML5.len() as f64, u["html5"].as_f64().expect("html5"));
    controlla("datetime.fromisoformat e _locale", &u["iso"], |c| Ok(Json::from(iso_naive(&ora_locale_da(c.as_str().unwrap_or_default(), "Europe/Rome")?))));
    controlla("html.unescape", &u["html"], |c| Ok(Json::from(html_unescape(c.as_str().unwrap_or_default())?)));
    controlla("_pulisci_link", &u["link"], |c| Ok(Json::from(pulisci_link(c.as_str().unwrap_or_default())?)));
    controlla("urlparse e parse_qs", &u["url"], |c| {
        let (percorso, query) = analizza_target(c.as_str().unwrap_or_default())?;
        let qs: Vec<Json> = parse_qs(&query).into_iter().map(|(k, v)| Json::Arr(vec![Json::from(k), Json::from(v)])).collect();
        Ok(ogg! { "path": percorso, "query": query, "qs": qs })
    });
    controlla("json.dumps(ensure_ascii=False)", &u["json"], |c| Ok(Json::from(Json::analizza(c.as_str().unwrap_or_default()).map_err(|e| Errore::Altro(e.messaggio))?.in_py())));
    controlla("repr di una stringa", &u["repr"], |c| Ok(Json::from(py_repr_str(c.as_str().unwrap_or_default()))));
    controlla("str.strip", &u["strip"], |c| Ok(Json::from(py_strip(c.as_str().unwrap_or_default()))));
    controlla("sorted di stringhe", &u["sort"], |c| {
        let mut v: Vec<String> = c.as_arr().expect("lista").iter().map(|x| x.as_str().unwrap_or_default().to_string()).collect();
        v.sort();
        Ok(Json::from(v))
    });
    controlla("float", &u["float"], |c| {
        let x = py_float(c)?;
        Ok(if x.is_nan() {
            Json::from("NaN")
        } else if x == f64::INFINITY {
            Json::from("Infinity")
        } else if x == f64::NEG_INFINITY {
            Json::from("-Infinity")
        } else {
            Json::from(x)
        })
    });
}

fn impronta(v: &Json) -> String {
    Sha256::digest(canonico(v).as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..20].to_string()
}

fn riassunto(lista: Vec<Json>) -> Json {
    let mut ordinata = lista.clone();
    ordinata.sort_by_cached_key(canonico);
    ogg! { "n": lista.len(), "h": impronta(&Json::Arr(lista)), "hs": impronta(&Json::Arr(ordinata)) }
}

fn tipo_errore(e: &Errore) -> &'static str {
    match e {
        Errore::Valore(_) => "ValueError",
        Errore::Tipo(_) => "TypeError",
        _ => "?",
    }
}

fn tipo_registrato(t: &Json) -> &'static str {
    match t.as_str() {
        Some("ValueError") => "ValueError",
        Some("TypeError") => "TypeError",
        _ => "?",
    }
}

fn valida(testo: &str) -> Json {
    match valida_ics(testo) {
        Ok(_) => ogg! { "ok": true },
        Err(e) => ogg! { "ok": false, "valore": matches!(e, Errore::Valore(_) | Errore::Tipo(_)), "errore": tipo_errore(&e), "messaggio": e.messaggio() },
    }
}

fn finestra(testo: &str, f: &[&str]) -> Json {
    match espandi_ics(testo, f[0], f[1], None) {
        Ok(v) => riassunto(v.iter().map(EventoIcs::in_json).collect()),
        Err(e) => ogg! { "errore": tipo_errore(&e), "messaggio": e.messaggio(), "valore": matches!(e, Errore::Valore(_) | Errore::Tipo(_)) },
    }
}

/// Le chiavi presenti, come le confronta toEqual (una chiave che manca è uguale a una non definita).
fn solo(m: &Json, chiavi: &[&str]) -> Json {
    let mut out = Mappa::new();
    for k in chiavi {
        if let Some(v) = m.get(k) {
            out.insert(*k, v.clone());
        }
    }
    Json::Obj(out)
}

#[test]
fn vtimezone_nel_calendario_uguale_a_python() {
    let registrato = leggi_golden("ricorrenze-vtimezone.json");
    let casi = registrato["casi"].as_arr().expect("casi");
    let mut categorie: Vec<&str> = Vec::new();
    for c in casi {
        let cat = c["categoria"].as_str().unwrap_or_default();
        if !categorie.contains(&cat) {
            categorie.push(cat);
        }
    }
    let mut diversi = Vec::new();
    let mut finestre_viste = 0;
    for cat in &categorie {
        for caso in casi.iter().filter(|c| c["categoria"].as_str() == Some(cat)) {
            if caso["continua"].as_bool() != Some(true) {
                reimposta_fusi();
            }
            let testo = caso["testo"].as_str().expect("testo");
            let attesi = caso["atteso"].as_obj().expect("atteso");
            // con "primaEventi" nessuna validazione prima delle finestre: una richiesta di eventi su un agente appena partito
            let prima = if caso["primaEventi"].as_bool() == Some(true) { None } else { Some(valida(testo)) };
            let mut ottenuti = Mappa::new();
            for f in caso["finestre"].as_arr().expect("finestre") {
                let f: Vec<&str> = f.as_arr().expect("finestra").iter().map(|x| x.as_str().unwrap_or_default()).collect();
                ottenuti.insert(f.join("|"), finestra(testo, &f));
                finestre_viste += 1;
            }
            let v = prima.unwrap_or_else(|| valida(testo));
            let av = &caso["atteso"]["valida"];
            let atteso_valida = if av["ok"].as_bool() == Some(false) || av.get("errore").is_some() {
                let mut m = solo(av, &["valore", "messaggio"]).as_obj().cloned().unwrap_or_default();
                m.insert("ok", false);
                m.insert("errore", tipo_registrato(&av["errore"]));
                Json::Obj(m)
            } else {
                ogg! { "ok": true }
            };
            let mut finestre_attese = Mappa::new();
            for (k, a) in attesi.iter().filter(|(k, _)| *k != "valida") {
                let mut a = a.clone();
                if let Some(m) = a.as_obj_mut() {
                    if let Some(e) = m.get("errore").cloned() {
                        m.insert("errore", tipo_registrato(&e));
                    }
                }
                finestre_attese.insert(k.as_str(), a);
            }
            let (x, y) = (canonico(&ogg! { "valida": atteso_valida, "finestre": finestre_attese }), canonico(&ogg! { "valida": v, "finestre": ottenuti }));
            if x != y {
                diversi.push(format!("{cat} {}: atteso {} ottenuto {}", caso["nome"].as_str().unwrap_or_default(), primi_utf16(&x, 400), primi_utf16(&y, 400)));
            }
        }
    }
    println!("vtimezone: {} casi, {finestre_viste} finestre", casi.len());
    assert!(finestre_viste >= casi.len());
    assert_eq!(diversi, Vec::<String>::new(), "{} casi diversi su {}", diversi.len(), casi.len());
}

fn secondi_di(t: &str) -> i64 {
    muro(&t.parse().expect("data"))
}

#[test]
fn vtimezone_dall_istante_al_muro_e_ritorno() {
    let registrato = leggi_golden("ricorrenze-vtimezone.json");
    let conversioni = registrato["conversioni"].as_arr().expect("conversioni");
    assert!(!conversioni.is_empty());
    for c in conversioni {
        reimposta_fusi();
        let nome = c["caso"].as_str().expect("caso");
        let caso = registrato["casi"].as_arr().expect("casi").iter().find(|x| x["nome"].as_str() == Some(nome)).expect("caso");
        valida_ics(caso["testo"].as_str().expect("testo")).expect("il calendario si valida");
        let zona = c["zona"].as_str().expect("zona");
        let da_utc: Vec<Json> = c["daUtc"]
            .as_arr()
            .expect("daUtc")
            .iter()
            .map(|r| {
                let utc = r[0].as_str().expect("utc");
                let o = in_fuso(secondi_di(utc) * 1000, zona).expect("in_fuso");
                let w = testo_naive(&o.w);
                let scarto = secondi_di(&w) - istante(&o).expect("istante") / 1000;
                Json::Arr(vec![Json::from(utc), Json::from(w), Json::from(o.fold as u32), Json::from(scarto)])
            })
            .collect();
        assert_eq!(Json::Arr(da_utc).in_js(), c["daUtc"].in_js(), "{nome}: daUtc");
        let sul_muro: Vec<Json> = c["sulMuro"]
            .as_arr()
            .expect("sulMuro")
            .iter()
            .map(|r| {
                let m = r[0].as_str().expect("muro");
                let fold = r[1].as_f64().unwrap_or_default() as u8;
                let o = ora(m.parse().expect("data"), Some(Rc::from(zona)), fold);
                Json::Arr(vec![Json::from(m), r[1].clone(), Json::from(secondi_di(m) - istante(&o).expect("istante") / 1000)])
            })
            .collect();
        assert_eq!(Json::Arr(sul_muro).in_js(), c["sulMuro"].in_js(), "{nome}: sulMuro");
    }
}

// ---- occorrenze annullate ----------------------------------------------------------------------------------------------------
fn ms(iso: &str) -> i64 {
    secondi_di(iso.trim_end_matches('Z')) * 1000
}

fn serie() -> RigaAndroid {
    RigaAndroid {
        id: 1,
        title: Some("Aggiornamento settimanale".to_string()),
        dtstart: Some(ms("2026-10-13T13:30:00Z")),
        duration: Some("P1800S".to_string()),
        event_timezone: Some("Europe/Rome".to_string()),
        rrule: Some("FREQ=WEEKLY;UNTIL=20261230T000000Z;BYDAY=TU".to_string()),
        all_day: Some(0),
        original_all_day: Some(0),
        ..Default::default()
    }
}

fn eccezione(f: impl FnOnce(&mut RigaAndroid)) -> RigaAndroid {
    let mut r = RigaAndroid {
        id: 2,
        title: Some("Aggiornamento settimanale".to_string()),
        event_status: Some(2),
        dtstart: Some(ms("2026-10-13T13:30:00Z")),
        dtend: Some(ms("2026-10-13T14:00:00Z")),
        event_timezone: Some("Europe/Rome".to_string()),
        original_id: Some(1),
        original_instance_time: Some(ms("2026-10-13T13:30:00Z")),
        all_day: Some(0),
        original_all_day: Some(0),
        ..Default::default()
    };
    f(&mut r);
    r
}

fn occorrenze(righe: &[RigaAndroid]) -> Vec<(String, String)> {
    let a = |t: &str| leggi_iso_locale(t).expect("data");
    espandi_righe_android(righe, a("2026-10-12T00:00:00"), a("2026-10-28T00:00:00"), None).expect("espansione").into_iter().map(|o| (o.inizio, o.fine)).collect()
}

fn inizi(righe: &[RigaAndroid]) -> Vec<String> {
    occorrenze(righe).into_iter().map(|o| o.0).collect()
}

fn fini(righe: &[RigaAndroid]) -> Vec<String> {
    occorrenze(righe).into_iter().map(|o| o.1).collect()
}

const TUTTE: [&str; 3] = ["2026-10-13T15:30:00", "2026-10-20T15:30:00", "2026-10-27T15:30:00"];

#[test]
fn occorrenza_che_sostituisce_una_della_serie_con_lo_stato_annullato() {
    let senza_la_prima = &TUTTE[1..];
    assert_eq!(inizi(&[serie()]), TUTTE);
    let copia = |r: &mut RigaAndroid| {
        r.is_organizer = Some(1);
        r.has_attendee_data = Some(0);
    };
    assert_eq!(inizi(&[serie(), eccezione(copia)]), TUTTE);
    assert_eq!(
        inizi(&[serie(), eccezione(|r| {
            r.is_organizer = Some(0);
            r.has_attendee_data = Some(1);
        })]),
        senza_la_prima
    );
    assert_eq!(
        inizi(&[serie(), eccezione(|r| {
            r.is_organizer = Some(1);
            r.has_attendee_data = Some(1);
        })]),
        senza_la_prima
    );
    assert_eq!(inizi(&[serie(), eccezione(|_| {})]), senza_la_prima);
    let spostata = eccezione(|r| {
        copia(r);
        r.event_status = Some(1);
        r.dtstart = Some(ms("2026-10-13T15:00:00Z"));
        r.dtend = Some(ms("2026-10-13T15:30:00Z"));
    });
    assert_eq!(inizi(&[serie(), spostata]), ["2026-10-13T17:00:00", TUTTE[1], TUTTE[2]]);
    let mut sola = serie();
    sola.rrule = None;
    sola.event_status = Some(2);
    sola.is_organizer = Some(1);
    sola.has_attendee_data = Some(0);
    sola.dtend = Some(ms("2026-10-13T14:00:00Z"));
    sola.duration = None;
    assert_eq!(inizi(&[sola]), Vec::<String>::new());
    let seconda = eccezione(|r| {
        r.id = 3;
        r.original_instance_time = Some(ms("2026-10-20T13:30:00Z"));
        r.dtstart = Some(ms("2026-10-20T13:30:00Z"));
        r.dtend = Some(ms("2026-10-20T14:00:00Z"));
        r.is_organizer = Some(0);
        r.has_attendee_data = Some(1);
    });
    assert_eq!(inizi(&[serie(), eccezione(copia), seconda]), [TUTTE[0], TUTTE[2]]);
}

#[test]
fn la_fine_di_una_copia_con_lo_stato_annullato() {
    let copia = |f: fn(&mut RigaAndroid)| {
        eccezione(|r| {
            r.is_organizer = Some(1);
            r.has_attendee_data = Some(0);
            f(r);
        })
    };
    assert_eq!(fini(&[serie(), copia(|r| r.dtend = Some(ms("2026-10-13T13:30:00Z")))]), ["2026-10-13T16:00:00", "2026-10-20T16:00:00", "2026-10-27T16:00:00"]);
    assert_eq!(
        fini(&[serie(), copia(|r| {
            r.dtend = None;
            r.duration = None;
        })])[0],
        "2026-10-13T16:00:00"
    );
    let mut m = serie();
    m.dtend = Some(ms("2026-10-13T14:30:00Z"));
    m.duration = None;
    assert_eq!(fini(&[m, copia(|r| r.dtend = Some(ms("2026-10-13T13:30:00Z")))])[0], "2026-10-13T16:30:00");
    assert_eq!(fini(&[serie(), copia(|r| r.dtend = Some(ms("2026-10-13T14:15:00Z")))])[0], "2026-10-13T16:15:00");
    let mut m = serie();
    m.duration = None;
    m.dtend = None;
    assert_eq!(fini(&[m, copia(|r| r.dtend = Some(ms("2026-10-13T13:30:00Z")))])[0], "2026-10-13T15:30:00");
}

// ---- occupato dal "Mostra come" -----------------------------------------------------------------------------------------------
fn righe_occupato() -> Json {
    let riga = |id: u32, titolo: &str, giorno: u32, fuso: &str, da: &str, a: &str| {
        let m = ogg! {
            "_sync_id": Json::Null, "calendar_id": 1, "eventLocation": Json::Null, "description": Json::Null, "eventStatus": Json::Null,
            "eventEndTimezone": Json::Null, "duration": Json::Null, "allDay": giorno, "rrule": Json::Null, "rdate": Json::Null, "exrule": Json::Null,
            "exdate": Json::Null, "original_id": Json::Null, "original_sync_id": Json::Null, "originalInstanceTime": Json::Null, "originalAllDay": 0,
            "organizer": "mario@alfa.example", "isOrganizer": 1, "hasAttendeeData": 0, "eventTimezone": fuso, "_id": id, "title": titolo,
            "dtstart": ms(da), "dtend": ms(a),
        };
        m
    };
    Json::Arr(vec![
        riga(1, "Ferie", 1, "UTC", "2026-11-05T00:00:00Z", "2026-11-07T00:00:00Z"),
        riga(2, "Compleanno", 1, "UTC", "2026-11-05T00:00:00Z", "2026-11-06T00:00:00Z"),
        riga(3, "Change", 0, "Europe/Rome", "2026-11-04T08:00:00Z", "2026-11-20T17:00:00Z"),
    ])
}

fn leggi_occupato(con_colonna: bool) -> Vec<Json> {
    let calendari = Json::analizza(r#"[[1, "mario@alfa.example", "com.microsoft.office.outlook", "Calendar", 1, 0, 1]]"#).expect("calendari");
    let sql = db_in_memoria(&calendari, &righe_occupato(), &Json::Arr(Vec::new()));
    if con_colonna {
        sql.execute_batch("ALTER TABLE Events ADD COLUMN availability INTEGER DEFAULT 0").expect("colonna");
        for (id, v) in [(1, 0), (2, 1), (3, 2)] {
            sql.execute("UPDATE Events SET availability = ? WHERE _id = ?", (v, id)).expect("aggiorna");
        }
    }
    let mut tipo = None;
    let lettura = estrai(&sql, "mario@alfa.example", &[], &mut tipo).ok().flatten().expect("lettura");
    let info = InfoCasella { id: "android::mario@alfa.example", nome: "Alfa", account: "mario@alfa.example", tipo: Some("outlook") };
    let a = |t: &str| leggi_iso_locale(t).expect("data");
    eventi_da_lettura(&lettura, &info, a("2026-11-01T00:00:00"), a("2026-12-01T00:00:00"), "Europe/Rome", None).expect("eventi")
}

fn per_titolo(eventi: &[Json], t: &str) -> Json {
    eventi.iter().find(|e| e["title"].as_str() == Some(t)).cloned().expect("evento")
}

#[test]
fn occupato_dal_mostra_come() {
    let con = leggi_occupato(true);
    assert_eq!(per_titolo(&con, "Ferie")["occupato"], Json::Bool(true));
    assert_eq!(per_titolo(&con, "Compleanno")["occupato"], Json::Bool(false));
    assert_eq!(per_titolo(&con, "Change")["occupato"], Json::Bool(false));
    let senza = leggi_occupato(false);
    for t in ["Ferie", "Compleanno", "Change"] {
        let s = per_titolo(&senza, t);
        assert!(s.get("occupato").is_none());
        let mut resto = per_titolo(&con, t);
        resto.as_obj_mut().expect("oggetto").remove("occupato");
        assert_eq!(canonico(&resto), canonico(&s));
    }
}
