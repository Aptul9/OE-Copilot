//! Il codec protobuf contro protobufjs. tests/dati/protobuf.json è registrato con l'`encodeChat` e il `decodeChat` di
//! `agente-ts/src/gchat.ts`: le richieste che l'agente costruisce, un valore a caso di ogni tipo dello schema andata e ritorno, le conversioni
//! di JavaScript sui valori, ingressi binari fatti a mano, mutati o letti come un altro tipo. Degli errori conta solo che ci siano.
//! Si rigenera con `node calendar/server/agente-rs/strumenti/vettori-protobuf.cjs` quando cambia lo schema.

use oe_nucleo::json::Json;
use oe_rete::protobuf::{codifica_chat, decodifica_chat};

fn esa(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn da_esa(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("esadecimale")).collect()
}

fn corto(s: &str) -> String {
    s.chars().take(300).collect()
}

#[test]
fn codifica_e_decodifica_come_protobufjs() {
    let vettori = Json::analizza(include_str!("dati/protobuf.json")).expect("JSON");
    let vettori = vettori.as_arr().expect("elenco");
    let mut diversi = Vec::new();
    for (i, v) in vettori.iter().enumerate() {
        let tipo = v["tipo"].as_str().expect("tipo");
        let (ottenuto, atteso) = match v.get("codifica") {
            Some(valore) => (codifica_chat(tipo, valore).map(|b| esa(&b)), v.get("hex").and_then(Json::as_str)),
            None => (decodifica_chat(tipo, &da_esa(v["decodifica"].as_str().expect("esadecimale"))).map(|j| j.in_js()), v.get("json").and_then(Json::as_str)),
        };
        let uguale = match (&ottenuto, atteso) {
            (Ok(o), Some(a)) => o == a,
            (Err(_), None) => true,
            _ => false,
        };
        if !uguale {
            diversi.push(format!("#{i} {}\n  atteso   {:?} {:?}\n  ottenuto {:?}", corto(&v.in_js()), atteso.map(corto), v.get("errore").map(Json::in_js), ottenuto.map(|o| corto(&o))));
        }
    }
    println!("{} vettori", vettori.len());
    assert!(vettori.len() >= 60, "{}", vettori.len());
    assert!(diversi.is_empty(), "{} su {} diversi:\n{}", diversi.len(), vettori.len(), diversi.join("\n"));
}
