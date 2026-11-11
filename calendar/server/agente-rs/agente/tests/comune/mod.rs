//! Gli aiuti delle prove: i file di strumenti/golden (mai rigenerati: una differenza è un comportamento dell'agente che è cambiato), il
//! calendar.db costruito da righe in memoria come quello del telefono, il JSON con le chiavi in ordine.
#![allow(dead_code)]

use oe_nucleo::json::{Json, Mappa};
use rusqlite::Connection;
use rusqlite::types::Value;
use std::path::{Path, PathBuf};

pub fn cartella_golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../strumenti/golden")
}

pub fn leggi_testo_golden(pezzi: &[&str]) -> String {
    let mut p = cartella_golden();
    for x in pezzi {
        p.push(x);
    }
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

pub fn leggi_golden(nome: &str) -> Json {
    Json::analizza(&leggi_testo_golden(&[nome])).expect("golden JSON")
}

const SCHEMA: &str = "
  CREATE TABLE Calendars (_id INTEGER PRIMARY KEY, account_name TEXT, account_type TEXT, calendar_displayName TEXT,
                          isPrimary INTEGER, deleted INTEGER DEFAULT 0, sync_events INTEGER DEFAULT 1);
  CREATE TABLE Events (_id INTEGER PRIMARY KEY, _sync_id TEXT, calendar_id INTEGER, title TEXT, eventLocation TEXT,
                       description TEXT, eventStatus INTEGER, dtstart INTEGER, dtend INTEGER, eventTimezone TEXT,
                       eventEndTimezone TEXT, duration TEXT, allDay INTEGER DEFAULT 0, rrule TEXT, rdate TEXT,
                       exrule TEXT, exdate TEXT, original_id INTEGER, original_sync_id TEXT,
                       originalInstanceTime INTEGER, originalAllDay INTEGER, organizer TEXT, deleted INTEGER DEFAULT 0,
                       isOrganizer INTEGER, hasAttendeeData INTEGER);
  CREATE TABLE Attendees (_id INTEGER PRIMARY KEY, event_id INTEGER, attendeeName TEXT, attendeeEmail TEXT,
                          attendeeRelationship INTEGER);
";

fn valore(v: &Json) -> Value {
    match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Integer(i64::from(*b)),
        Json::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::Integer(*n as i64),
        Json::Num(n) => Value::Real(*n),
        Json::Str(s) => Value::Text(s.clone()),
        altro => Value::Text(altro.in_js()),
    }
}

/// Riempie le tabelle con calendari [_id, account, tipo, nome, primario, cancellato, sincronizza], eventi (oggetti con le colonne) e
/// invitati [evento, nome, email, ruolo].
pub fn riempi(sql: &Connection, calendari: &Json, eventi: &Json, invitati: &Json) {
    sql.execute_batch(SCHEMA).expect("schema");
    for r in calendari.as_arr().into_iter().flatten() {
        let v: Vec<Value> = r.as_arr().expect("riga").iter().map(valore).collect();
        sql.execute("INSERT INTO Calendars VALUES (?,?,?,?,?,?,?)", rusqlite::params_from_iter(v)).expect("calendario");
    }
    for r in eventi.as_arr().into_iter().flatten() {
        let m = r.as_obj().expect("evento");
        let colonne: Vec<&String> = m.keys().collect();
        let sql_testo = format!("INSERT INTO Events ({}) VALUES ({})", colonne.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(","), vec!["?"; colonne.len()].join(","));
        sql.execute(&sql_testo, rusqlite::params_from_iter(m.values().map(valore))).expect("evento");
    }
    for r in invitati.as_arr().into_iter().flatten() {
        let v: Vec<Value> = r.as_arr().expect("riga").iter().map(valore).collect();
        sql.execute("INSERT INTO Attendees (event_id, attendeeName, attendeeEmail, attendeeRelationship) VALUES (?,?,?,?)", rusqlite::params_from_iter(v)).expect("invitato");
    }
}

pub fn db_in_memoria(calendari: &Json, eventi: &Json, invitati: &Json) -> Connection {
    let sql = Connection::open_in_memory().expect("memoria");
    riempi(&sql, calendari, eventi, invitati);
    sql
}

/// Scrive calendar.db nella cartella (la cartella c'è già).
pub fn scrivi_db(cartella: &Path, calendari: &Json, eventi: &Json, invitati: &Json) -> PathBuf {
    let file = cartella.join("calendar.db");
    let _ = std::fs::remove_file(&file);
    let sql = Connection::open(&file).expect("calendar.db");
    riempi(&sql, calendari, eventi, invitati);
    file
}

/// JSON con le chiavi in ordine, per confrontare due oggetti che le hanno scritte in ordine diverso.
pub fn ordinato(v: &Json) -> Json {
    match v {
        Json::Arr(a) => Json::Arr(a.iter().map(ordinato).collect()),
        Json::Obj(m) => {
            let mut chiavi: Vec<&String> = m.keys().collect();
            chiavi.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let mut out = Mappa::new();
            for k in chiavi {
                out.insert(k.as_str(), ordinato(m.get(k).unwrap_or(&Json::Null)));
            }
            Json::Obj(out)
        }
        altro => altro.clone(),
    }
}

pub fn canonico(v: &Json) -> String {
    ordinato(v).in_js()
}

/// Una richiesta HTTP/1.1 semplice, la risposta intera: il codice e il corpo.
pub async fn chiedi(porta: u16, metodo: &str, percorso: &str, chiave: Option<&str>, corpo: Option<&str>) -> (u16, String) {
    let (stato, _, testo) = chiedi_tutto(porta, metodo, percorso, chiave, corpo).await;
    (stato, testo)
}

/// Come `chiedi`, con le intestazioni della risposta (nomi in minuscolo).
pub async fn chiedi_tutto(porta: u16, metodo: &str, percorso: &str, chiave: Option<&str>, corpo: Option<&str>) -> (u16, Vec<(String, String)>, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", porta)).await.expect("connessione");
    let mut testa = format!("{metodo} {percorso} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Some(k) = chiave {
        testa.push_str(&format!("Authorization: Bearer {k}\r\n"));
    }
    if let Some(c) = corpo {
        testa.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", c.len()));
    }
    testa.push_str("\r\n");
    s.write_all(testa.as_bytes()).await.expect("scrittura");
    if let Some(c) = corpo {
        s.write_all(c.as_bytes()).await.expect("corpo");
    }
    let mut dati = Vec::new();
    s.read_to_end(&mut dati).await.expect("lettura");
    let i = dati.windows(4).position(|f| f == b"\r\n\r\n").expect("risposta");
    let testa = String::from_utf8_lossy(&dati[..i]).into_owned();
    let mut righe = testa.split("\r\n");
    let stato = righe.next().unwrap_or_default().split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or_default();
    let intestazioni = righe.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
    (stato, intestazioni, String::from_utf8_lossy(&dati[i + 4..]).into_owned())
}

/// `testo.slice(0, n)` di JavaScript, in unità UTF-16.
pub fn primi_utf16(s: &str, n: usize) -> String {
    let mut usate = 0;
    let mut out = String::new();
    for c in s.chars() {
        usate += c.len_utf16();
        if usate > n {
            break;
        }
        out.push(c);
    }
    out
}
