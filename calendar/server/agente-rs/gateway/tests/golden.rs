//! Il gateway contro le risposte registrate dal servizio ui (Python), come web-ts/test/golden-lettura.test.ts e golden-scritture.test.ts:
//! le stesse chiamate sulla stessa cartella dei dati, con lo stesso agente finto, devono dare le stesse risposte e la stessa cartella.
//! I file registrati sono gli stessi che controllano il gateway TypeScript: passano tutti e due, quindi rispondono uguale.

mod comune;

use comune::*;
use oe_gateway::{agente, api, archivio, ics, orologio, uscita};
use oe_nucleo::json::{Json, Mappa};
use std::rc::Rc;
use tokio::task::LocalSet;

/// Prepara il thread (ora fissa, rete di prova, agente e server ICS finti, cartella dei dati) e fa girare la prova.
fn con_gateway<F, Fut>(profilo: &str, prova: F)
where
    F: FnOnce(Rc<Agente>, Cartella, String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    LocalSet::new().block_on(&rt, async {
        uscita::imposta_rete_di_prova(Some(prove_locali()));
        let ics_url = ascolta(Rc::new(Ics)).await;
        let finto = Agente::nuovo();
        let agente_url = ascolta(finto.clone()).await;
        let dati = prepara_dati(profilo, &ics_url);
        ambiente(&dati.0, &agente_url);
        orologio::fissa_ora(Some(ORA_FISSA_MS));
        agente::azzera_memoria();
        ics::azzera_feed();
        prova(finto, dati, ics_url).await;
        archivio::chiudi_archivio();
        oe_gateway::db::chiudi_tutto();
    });
}

fn lettura(profilo: &str) {
    let registrato = leggi(&["ui-lettura.json"]);
    con_gateway(profilo, |finto, dati, _| async move {
        let mut fatti = 0;
        for s in FIXTURE["scenari"].as_arr().expect("scenari") {
            if s["profilo"].as_str() != Some(profilo) {
                continue;
            }
            let nome = s["nome"].as_str().expect("nome");
            *finto.modo.borrow_mut() = s["modo"].as_str().expect("modo").to_string();
            let args = s["args"].as_arr().expect("args").to_vec();
            let r = api::chiama(s["metodo"].as_str().expect("metodo"), args).await.unwrap_or_else(|e| panic!("{nome}: {} {}", e.nome(), e.messaggio()));
            uguali(nome, &normalizza(&r), &registrato[profilo]["risposte"][nome]);
            fatti += 1;
        }
        // ogni risposta registrata è stata confrontata
        assert_eq!(fatti, registrato[profilo]["risposte"].as_obj().expect("risposte").len(), "{profilo}");
        assert!(finto.viste.borrow().len() > 0, "{profilo}: l'agente finto non ha visto richieste");
        // la cartella dei dati a fine sequenza
        let mut cache = Mappa::new();
        if let Json::Obj(m) = dati.leggi("events_cache.json") {
            for (k, v) in m.iter() {
                let mut voce = Mappa::new();
                if let Some(s) = v.get("stamp") {
                    voce.insert("stamp", s.clone());
                }
                voce.insert("eventi", v.get("eventi").and_then(Json::as_arr).map_or(0, Vec::len));
                cache.insert(k.as_str(), Json::Obj(voce));
            }
        }
        let mut finale = Mappa::new();
        for f in ["config.json", "agente_caselle.json", "local_events.json"] {
            finale.insert(f, dati.leggi(f));
        }
        finale.insert("events_cache.json", Json::Obj(cache));
        finale.insert("ics_cache", dati.elenco("ics_cache"));
        uguali("la cartella dei dati a fine sequenza", &normalizza(&Json::Obj(finale)), &registrato[profilo]["dati"]);
    });
}

/// Un profilo per thread: ognuno ha la sua cartella e la sua memoria, come describe.each in vitest.
#[test]
fn lettura_per_ogni_profilo() {
    let profili: Vec<String> = FIXTURE["profili"].as_obj().expect("profili").keys().cloned().collect();
    assert!(!profili.is_empty());
    let fili: Vec<_> = profili.into_iter().map(|p| (p.clone(), std::thread::spawn(move || lettura(&p)))).collect();
    let falliti: Vec<String> = fili.into_iter().filter_map(|(p, f)| f.join().is_err().then_some(p)).collect();
    assert!(falliti.is_empty(), "profili diversi dal registrato: {falliti:?}");
}

#[test]
fn scritture() {
    let registrato = leggi(&["ui-scritture.json"]);
    con_gateway("base", |_finto, dati, ics_url| async move {
        let stato_dati = || {
            let mut m = Mappa::new();
            m.insert("config.json", dati.leggi("config.json"));
            m.insert("local_events.json", dati.leggi("local_events.json"));
            m.insert("ics_cache", dati.elenco("ics_cache"));
            normalizza(&Json::Obj(m))
        };
        let mut ultimo_feed = String::new();
        let mut fatti = 0;
        for passo in FIXTURE["scritture"].as_arr().expect("scritture") {
            let nome = passo["nome"].as_str().expect("nome");
            let metodo = passo["metodo"].as_str().expect("metodo");
            let args = Json::analizza(&passo["args"].in_js().replace("{ICS}", &ics_url).replace("{ULTIMO_FEED}", &ultimo_feed)).expect("args");
            let r = api::chiama(metodo, args.as_arr().expect("lista").to_vec()).await.unwrap_or_else(|e| panic!("{nome}: {} {}", e.nome(), e.messaggio()));
            if metodo == "add_ics_calendar" && r.get("ok").and_then(Json::as_bool) == Some(true) {
                ultimo_feed = r["feed"]["id"].as_str().expect("id").to_string();
            }
            let mut avuto = Mappa::new();
            avuto.insert("risposta", normalizza(&r));
            avuto.insert("stato", stato_dati());
            uguali(nome, &Json::Obj(avuto), &registrato[nome]);
            fatti += 1;
        }
        assert_eq!(fatti, registrato.as_obj().expect("passi").len());
    });
}
