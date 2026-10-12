use tauri::{AppHandle, Url};
use tauri_plugin_opener::OpenerExt;

#[derive(Debug, PartialEq, Eq)]
pub enum Esito {
    /// Si carica nella finestra
    Consenti,
    /// Si apre nel browser predefinito
    Browser,
    /// Si apre nell'app Teams
    Teams,
    /// Non si fa niente
    Rifiuta,
}

/// Cosa fare di un indirizzo che la finestra vuole caricare o aprire: nella finestra entrano solo il sito, la pagina di
/// avvio inclusa nell'app e about:blank; gli altri indirizzi http(s) vanno al browser, gli altri schemi vengono rifiutati.
pub fn classifica(url: &Url, origine: &Url) -> Esito {
    match url.scheme() {
        "about" if url.as_str() == "about:blank" => Esito::Consenti,
        "http" | "https" => {
            if url.origin() == origine.origin() || pagina_di_avvio(url) {
                Esito::Consenti
            } else if link_teams(url) {
                Esito::Teams
            } else {
                Esito::Browser
            }
        }
        _ => Esito::Rifiuta,
    }
}

/// La pagina inclusa nell'app, servita da Tauri su tauri.localhost
fn pagina_di_avvio(url: &Url) -> bool {
    url.host_str() == Some("tauri.localhost")
}

/// Il link per entrare in una riunione Teams (https://teams.microsoft.com/l/meetup-join/..., teams.live.com).
/// Su Android non serve lo schema msteams:, il sistema dà il link https all'app Teams se c'è (link verificati), altrimenti al browser
fn link_teams(url: &Url) -> bool {
    cfg!(not(target_os = "android"))
        && url.scheme() == "https"
        && matches!(url.host_str(), Some("teams.microsoft.com" | "teams.live.com"))
        && (url.path().starts_with("/l/meetup-join/") || url.path().starts_with("/meet/"))
}

/// Lo stesso link con lo schema msteams: (msteams://teams.microsoft.com/l/...), che il sistema dà all'app Teams;
/// se lo schema non è registrato si ripiega sul link https, cioè sul browser.
pub fn apri_teams_con(url: &Url, mut apri: impl FnMut(&str) -> bool) -> bool {
    let nativo = format!("msteams://{}", &url.as_str()["https://".len()..]);
    apri(&nativo) || apri(url.as_str())
}

fn apri(app: &AppHandle, indirizzo: &str) -> bool {
    #[cfg(feature = "prova")]
    if let Some(esito) = prova_apri(indirizzo) {
        return esito;
    }
    app.opener().open_url(indirizzo, None::<&str>).is_ok()
}

/// Solo nelle build di prova: con OE_PROVA_APRI gli indirizzi da aprire finiscono in quel file invece che nel sistema
/// (con OE_PROVA_SENZA_TEAMS lo schema msteams: risulta non registrato)
#[cfg(feature = "prova")]
fn prova_apri(indirizzo: &str) -> Option<bool> {
    use std::io::Write;

    let file = std::env::var_os("OE_PROVA_APRI")?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(file).ok()?;
    let _ = writeln!(f, "{indirizzo}");
    Some(!(indirizzo.starts_with("msteams:") && std::env::var_os("OE_PROVA_SENZA_TEAMS").is_some()))
}

/// Su Android il gestore della navigazione gira sul thread dell'interfaccia, e il plugin di apertura chiama Kotlin passando
/// da quel thread e ne aspetta la risposta: da lì si bloccherebbe, quindi l'apertura parte da un altro thread
fn apri_senza_bloccare(app: &AppHandle, indirizzo: &str) {
    #[cfg(target_os = "android")]
    {
        let (app, indirizzo) = (app.clone(), indirizzo.to_string());
        std::thread::spawn(move || {
            apri(&app, &indirizzo);
        });
    }
    #[cfg(not(target_os = "android"))]
    apri(app, indirizzo);
}

/// Per la navigazione della finestra: true se l'indirizzo si carica, altrimenti lo gira a chi di dovere e risponde no
pub fn consenti_navigazione(app: &AppHandle, origine: &Url, url: &Url) -> bool {
    match classifica(url, origine) {
        Esito::Consenti => true,
        Esito::Browser => {
            apri_senza_bloccare(app, url.as_str());
            false
        }
        Esito::Teams => {
            apri_teams_con(url, |indirizzo| apri(app, indirizzo));
            false
        }
        Esito::Rifiuta => false,
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    fn url(testo: &str) -> Url {
        Url::parse(testo).unwrap()
    }

    fn sito() -> Url {
        url("https://oe.prova.test")
    }

    #[test]
    fn il_sito_e_la_pagina_di_avvio_entrano() {
        assert_eq!(classifica(&url("https://oe.prova.test/login?next=%2F"), &sito()), Esito::Consenti);
        assert_eq!(classifica(&url("https://oe.prova.test:443/"), &sito()), Esito::Consenti);
        assert_eq!(classifica(&url("http://tauri.localhost/index.html"), &sito()), Esito::Consenti);
        assert_eq!(classifica(&url("about:blank"), &sito()), Esito::Consenti);
    }

    #[test]
    fn un_altro_sito_va_al_browser() {
        assert_eq!(classifica(&url("https://example.org/"), &sito()), Esito::Browser);
        assert_eq!(classifica(&url("http://oe.prova.test/"), &sito()), Esito::Browser);
        assert_eq!(classifica(&url("https://oe.prova.test:8443/"), &sito()), Esito::Browser);
        assert_eq!(classifica(&url("https://oe.prova.test.example.org/"), &sito()), Esito::Browser);
        assert_eq!(classifica(&url("https://zoom.us/j/123"), &sito()), Esito::Browser);
    }

    #[test]
    fn gli_altri_schemi_sono_rifiutati() {
        for testo in ["file:///C:/Windows/win.ini", "javascript:alert(1)", "data:text/html,x", "blob:https://oe.prova.test/abc", "ftp://example.org/", "mailto:a@example.org", "about:srcdoc", "tauri://localhost/"] {
            assert_eq!(classifica(&url(testo), &sito()), Esito::Rifiuta, "{testo}");
        }
    }

    #[test]
    fn i_link_teams_vanno_a_teams() {
        for testo in [
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%7d",
            "https://teams.microsoft.com/meet/1234567890?p=abc",
            "https://teams.live.com/meet/9876543210?p=abc",
            "https://teams.live.com/l/meetup-join/19%3ameeting_abc/0",
        ] {
            assert_eq!(classifica(&url(testo), &sito()), Esito::Teams, "{testo}");
        }
        assert_eq!(classifica(&url("https://teams.microsoft.com/l/chat/0/0?users=a"), &sito()), Esito::Browser);
        assert_eq!(classifica(&url("http://teams.microsoft.com/l/meetup-join/x"), &sito()), Esito::Browser);
    }

    #[test]
    fn il_link_teams_diventa_msteams_e_ripiega_sul_browser() {
        let link = url("https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%7d#x");
        let mut aperti = Vec::new();
        assert!(apri_teams_con(&link, |i| {
            aperti.push(i.to_string());
            true
        }));
        assert_eq!(aperti, ["msteams://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%7d#x"]);

        let mut aperti = Vec::new();
        assert!(apri_teams_con(&link, |i| {
            aperti.push(i.to_string());
            i.starts_with("https://")
        }));
        assert_eq!(aperti.len(), 2);
        assert!(aperti[0].starts_with("msteams://") && aperti[1] == link.as_str());

        assert!(!apri_teams_con(&link, |_| false));
    }
}
