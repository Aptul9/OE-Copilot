//! Le prove di `agente-ts/test/diretto.test.ts`: rinnovo dei token, lettura diretta dei calendari Outlook e Google.

use oe_agente::diretto::{CalendarioDiretto, GestoreToken, OUTLOOK_CLIENT_ID, OpzioniToken};
use oe_agente::fonte::Adesso;
use oe_agente::server::contesto_di;
use oe_nucleo::json::Json;
use oe_rete::trasporto::prova::{TrasportoFinto, json};
use oe_rete::trasporto::{ErroreRete, Risposta, Trasporto};
use std::path::PathBuf;
use std::rc::Rc;

fn fake_jwt(claims: &str) -> String {
    use base64::Engine;
    let b = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.sig", b(r#"{"alg":"none","typ":"JWT"}"#), b(claims))
}

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("{nome}-{}-{}", std::process::id(), oe_agente::fonte::adesso_vero()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ms(iso: &str) -> i64 {
    iso.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn dt(s: &str) -> jiff::civil::DateTime {
    s.parse().unwrap()
}

fn leggi(p: &PathBuf) -> Json {
    Json::analizza(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn rinnova_il_refresh_token_microsoft_e_legge_gli_eventi() {
    let dir = cartella("diretto-ms");
    let tokens_file = dir.join("tokens.json");
    let posta_file = dir.join("token-outlook.json");
    let ora_ms = ms("2026-11-05T10:00:00Z");
    let ora_s = ora_ms / 1000;
    std::fs::write(&tokens_file, format!(r#"{{"accounts":{{"mario.rossi@alfa.example":{{"provider":"microsoft","client_id":"{OUTLOOK_CLIENT_ID}","tenant_id":"tid-alfa","refresh_token":"rt-vecchio"}}}}}}"#)).unwrap();
    let nuovo = fake_jwt(&format!(r#"{{"upn":"mario.rossi@alfa.example","tid":"tid-alfa","exp":{},"scp":"Calendars.ReadWrite Mail.ReadWrite"}}"#, ora_s + 7200));
    let n2 = nuovo.clone();
    let finto = TrasportoFinto::con_funzione(move |r| {
        let url = r.url.clone();
        if url.contains("/oauth2/v2.0/token") {
            let corpo = String::from_utf8_lossy(r.corpo.as_deref().unwrap_or_default()).into_owned();
            assert!(corpo.contains("grant_type=refresh_token") && corpo.contains("refresh_token=rt-vecchio"), "{corpo}");
            return Ok(json(200, &format!(r#"{{"access_token":"{n2}","refresh_token":"rt-nuovo","expires_in":7200}}"#)));
        }
        if url.contains("/me/calendars?") {
            return Ok(json(200, r#"{"value":[{"Id":"cal-1","Name":"Calendar","IsDefaultCalendar":true},{"Id":"cal-2","Name":"Condiviso","IsDefaultCalendar":false}]}"#));
        }
        if url.contains("/me/calendars/cal-1/calendarview") {
            assert_eq!(r.intestazioni.primo("prefer"), Some("outlook.timezone=\"Europe/Rome\""));
            return Ok(json(
                200,
                r#"{"value":[{"Id":"ev-1","SeriesMasterId":null,"Type":"SingleInstance","Subject":"Allineamento architettura","Start":{"DateTime":"2026-11-05T11:00:00.0000000","TimeZone":"Europe/Rome"},"End":{"DateTime":"2026-11-05T12:00:00.0000000","TimeZone":"Europe/Rome"},"IsAllDay":false,"IsCancelled":false,"ShowAs":"Busy","Location":{"DisplayName":"Sala 1"},"Organizer":{"EmailAddress":{"Name":"Mario Rossi","Address":"Mario.Rossi@alfa.example"}},"Attendees":[{"EmailAddress":{"Name":"Luca","Address":"Luca@alfa.example"}}],"OnlineMeeting":{"JoinUrl":"https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc"}},{"Id":"ev-cancellato","Subject":"Annullata","Start":{"DateTime":"2026-11-05T14:00:00.0000000","TimeZone":"Europe/Rome"},"End":{"DateTime":"2026-11-05T15:00:00.0000000","TimeZone":"Europe/Rome"},"IsAllDay":false,"IsCancelled":true,"ShowAs":"Free"}]}"#,
            ));
        }
        Err(ErroreRete::Rete(format!("URL inatteso: {url}")))
    });
    let t: Rc<dyn Trasporto> = finto.clone();
    let adesso: Adesso = Rc::new(move || ora_ms);
    let gestore = Rc::new(GestoreToken::nuovo(tokens_file.clone(), OpzioniToken { token_posta: Some(posta_file.clone()), adesso: Some(adesso.clone()), trasporto: t, strict: false }));
    let ctx = contesto_di(dir.join("accessi.json"), "Europe/Rome", Some(adesso), Some(Rc::new(|_, _| {})));
    let cal = CalendarioDiretto::nuovo("android::mario.rossi@alfa.example".into(), "Alfa".into(), "mario.rossi@alfa.example", None, ctx, gestore);
    let (eventi, errori) = cal.eventi(dt("2026-11-05T00:00:00"), dt("2026-11-06T00:00:00")).await.unwrap();
    assert_eq!(errori, Vec::<String>::new());
    assert_eq!(eventi.len(), 1);
    assert_eq!(
        eventi[0].in_js(),
        r#"{"id":"android::mario.rossi@alfa.example|mev-1|2026-11-05T11:00:00","account":"mario.rossi@alfa.example","alias":"Alfa","calendar":"mario.rossi@alfa.example · Calendar","title":"Allineamento architettura","start":"2026-11-05T11:00:00","end":"2026-11-05T12:00:00","allDay":false,"location":"Sala 1","organizer":"Mario Rossi","organizer_email":"mario.rossi@alfa.example","attendees":["luca@alfa.example"],"online_url":"https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc","web_link":null,"fonte":"android","tipo":"outlook","occupato":true}"#
    );
    // la query si allarga di un giorno in UTC
    let vista = finto.richieste().into_iter().find(|r| r.url.contains("calendarview")).unwrap().url;
    assert!(vista.contains("startDateTime=2026-11-03T23%3A00%3A00Z&endDateTime=2026-11-06T23%3A00%3A00Z"), "{vista}");
    let salvati = leggi(&tokens_file);
    assert_eq!(salvati["accounts"]["mario.rossi@alfa.example"]["refresh_token"], Json::from("rt-nuovo"));
    let posta = leggi(&posta_file);
    assert_eq!(posta["account"].as_arr().unwrap().len(), 1);
    assert_eq!(posta["account"][0]["upn"], Json::from("mario.rossi@alfa.example"));
    assert_eq!(posta["account"][0]["token"], Json::from(nuovo.as_str()));
    // il file resta quello che scrive l'agente TypeScript: due spazi di rientro, a capo in fondo
    let testo = std::fs::read_to_string(&tokens_file).unwrap();
    assert!(testo.starts_with("{\n  \"accounts\": {\n    \"mario.rossi@alfa.example\": {\n      \"provider\": \"microsoft\",") && testo.ends_with("}\n"), "{testo}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&tokens_file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "current_thread")]
async fn rinnova_il_master_token_google_e_legge_i_calendari() {
    let dir = cartella("diretto-google");
    let tokens_file = dir.join("tokens.json");
    let ora_ms = ms("2026-11-05T10:00:00Z");
    std::fs::write(&tokens_file, r#"{"accounts":{"mario@beta.example":{"provider":"google","master_token":"aas_et/PROVA_MASTER_TOKEN"}}}"#).unwrap();
    let finto = TrasportoFinto::con_funzione(|r| {
        let url = r.url.clone();
        if url == "https://android.clients.google.com/auth" {
            return Ok(Risposta::nuova(200, "Auth=ya29.PROVA_GOOGLE_AT\nExpiresInDurationSec=3600\n"));
        }
        if url.contains("/users/me/calendarList") {
            return Ok(json(200, r#"{"items":[{"id":"mario@beta.example","summary":"mario@beta.example","primary":true},{"id":"extra@beta.example","summary":"Extra","primary":false}]}"#));
        }
        if url.contains("/calendars/mario%40beta.example/events") {
            assert_eq!(r.intestazioni.primo("authorization"), Some("Bearer ya29.PROVA_GOOGLE_AT"));
            return Ok(json(
                200,
                r#"{"items":[{"id":"gev1","status":"confirmed","summary":"Revisione progetto","start":{"dateTime":"2026-11-05T14:00:00Z"},"end":{"dateTime":"2026-11-05T15:00:00Z"},"organizer":{"email":"mario@beta.example","displayName":"Mario"},"attendees":[{"email":"collega@beta.example"}],"hangoutLink":"https://meet.google.com/abc-defg-hij"},{"id":"gev2","status":"confirmed","summary":"Ferie","start":{"date":"2026-11-05"},"end":{"date":"2026-11-06"},"transparency":"transparent"}]}"#,
            ));
        }
        Err(ErroreRete::Rete(format!("URL inatteso: {url}")))
    });
    let t: Rc<dyn Trasporto> = finto;
    let adesso: Adesso = Rc::new(move || ora_ms);
    let gestore = Rc::new(GestoreToken::nuovo(tokens_file.clone(), OpzioniToken { token_posta: None, adesso: Some(adesso.clone()), trasporto: t, strict: false }));
    let ctx = contesto_di(dir.join("accessi.json"), "Europe/Rome", Some(adesso), Some(Rc::new(|_, _| {})));
    let cal = CalendarioDiretto::nuovo("android::mario@beta.example".into(), "Beta".into(), "mario@beta.example", None, ctx, gestore);
    let (eventi, errori) = cal.eventi(dt("2026-11-05T00:00:00"), dt("2026-11-06T00:00:00")).await.unwrap();
    assert_eq!(errori, Vec::<String>::new());
    assert_eq!(eventi.len(), 2);
    let campi = |e: &Json, k: &[&str]| k.iter().map(|x| format!("{x}={}", e[*x].in_js())).collect::<Vec<_>>().join(" ");
    assert_eq!(campi(&eventi[0], &["title", "start", "end", "allDay", "tipo", "occupato"]), r#"title="Ferie" start="2026-11-05T00:00:00" end="2026-11-06T00:00:00" allDay=true tipo="caldav" occupato=false"#);
    assert_eq!(
        campi(&eventi[1], &["title", "start", "end", "allDay", "online_url", "tipo", "occupato"]),
        r#"title="Revisione progetto" start="2026-11-05T15:00:00" end="2026-11-05T16:00:00" allDay=false online_url="https://meet.google.com/abc-defg-hij" tipo="caldav" occupato=true"#
    );
    let _ = std::fs::remove_dir_all(&dir);
}
