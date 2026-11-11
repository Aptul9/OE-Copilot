//! Avvio dell'agente: le variabili d'ambiente e i valori di partenza sono quelli dell'agente Python.
//!   API_KEY   la chiave dell'API (senza, l'agente non parte)       PORTA     8180
//!   FONTI     /app/fonti.json (le caselle da leggere)               ANDROID_DB  /android-calendario (la cartella di calendar.db, in sola lettura)
//!   BATTITO   /stato/battito.json (il segno di vita del servizio)   ACCESSI   /stato/accessi.json (lo stato degli accessi dal rilevatore)
//!   TZ        Europe/Rome                                            GOOGLE_STATE  la cartella di tokens.json (gmail.json, google-web.json)

use oe_agente::diretto::{GestoreToken, OpzioniToken};
use oe_agente::fonte::{Adesso, Livello, adesso_vero};
use oe_agente::google_communication::GoogleCommunication;
use oe_rete::trasporto::{Trasporto, TrasportoReqwest};
use oe_agente::http::servi;
use oe_agente::server::{Agente, OpzioniAgente, carica_fonti, contesto_di, nomi_fonti, registro_su_stderr, strip};
use oe_nucleo::errori::Errore;
use oe_nucleo::pyfmt::py_repr_str;
use oe_nucleo::tempo::fuso_iana;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::LocalSet;

fn env(nome: &str, base: &str) -> String {
    std::env::var(nome).unwrap_or_else(|_| base.to_string())
}

fn main() {
    let zona = std::env::var("TZ").ok().filter(|z| !z.is_empty()).unwrap_or_else(|| "Europe/Rome".to_string());
    let adesso: Adesso = Rc::new(adesso_vero);
    let registro = registro_su_stderr(&zona, adesso.clone());
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            registro(Livello::Error, &e.to_string());
            std::process::exit(1);
        }
    };
    let uscita = LocalSet::new().block_on(&runtime, avvia(zona, adesso, registro.clone()));
    std::process::exit(uscita);
}

async fn avvia(zona: String, adesso: Adesso, registro: oe_agente::fonte::Registro) -> i32 {
    let preparato = (|| -> Result<_, Errore> {
        fuso_iana(&zona)?;
        let chiave = std::env::var("API_KEY").unwrap_or_default();
        let porta_testo = env("PORTA", "8180");
        let t = strip(&porta_testo);
        let cifre = t.strip_prefix(['+', '-']).unwrap_or(t);
        if cifre.is_empty() || !cifre.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Errore::Valore(format!("invalid literal for int() with base 10: {}", py_repr_str(&porta_testo))));
        }
        let porta: i64 = t.parse().unwrap_or(i64::MAX);
        let porta = u16::try_from(porta).map_err(|_| Errore::Altro(format!("RangeError [ERR_SOCKET_BAD_PORT]: Port should be >= 0 and < 65536. Received {porta}.")))?;
        let cartella_db = PathBuf::from(env("ANDROID_DB", "/android-calendario"));
        let accessi = PathBuf::from(env("ACCESSI", "/stato/accessi.json"));
        let token_posta = PathBuf::from(env("TOKEN_POSTA", "/stato/token-outlook.json"));
        let trasporto: Rc<dyn Trasporto> = Rc::new(TrasportoReqwest::nuovo().map_err(|e| Errore::Altro(e.to_string()))?);
        let percorso_tokens = PathBuf::from(env("TOKENS", "/stato/tokens.json"));
        // GOOGLE_STATE: gmail.json e google-web.json; senza, accanto a tokens.json
        let cartella_google = std::env::var("GOOGLE_STATE").map(PathBuf::from).unwrap_or_else(|_| percorso_tokens.parent().map(Path::to_path_buf).unwrap_or_default());
        let google = Rc::new(GoogleCommunication::nuovo(&cartella_google, trasporto.clone(), None));
        let tokens = Rc::new(GestoreToken::nuovo(
            percorso_tokens,
            OpzioniToken { token_posta: Some(token_posta.clone()), adesso: Some(adesso.clone()), trasporto, strict: false },
        ));
        let mut ctx = contesto_di(accessi.clone(), &zona, Some(adesso.clone()), Some(registro.clone()));
        ctx.tokens = Some(tokens.clone());
        // le fonti si leggono prima di controllare la chiave, come in Python: un fonti.json che manca ferma l'avvio con 1
        let fonti = carica_fonti(&PathBuf::from(env("FONTI", "/app/fonti.json")), &cartella_db, &ctx)?;
        Ok((chiave, porta, accessi, token_posta, fonti, tokens, google))
    })();
    let (chiave, porta, accessi, token_posta, fonti, tokens, google) = match preparato {
        Ok(x) => x,
        Err(e) => {
            registro(Livello::Error, e.messaggio());
            return 1;
        }
    };
    if chiave.is_empty() {
        registro(Livello::Error, "API_KEY mancante: l'API resterebbe aperta a chiunque, non parto");
        return 2;
    }
    // TEAMS_BROKER_KEY: la chiave stretta di TeamsRelay (elenco degli account condivisi, token della chat); vuota, TeamsRelay non entra.
    // Uguale ad API_KEY darebbe a TeamsRelay la chiave piena senza che nessuno se ne accorga.
    let chiave_teams = std::env::var("TEAMS_BROKER_KEY").unwrap_or_default();
    if !chiave_teams.is_empty() && chiave_teams == chiave {
        registro(Livello::Error, "TEAMS_BROKER_KEY uguale ad API_KEY: TeamsRelay avrebbe la chiave piena, non parto");
        return 2;
    }
    let nomi = nomi_fonti(&fonti);
    let mut o = OpzioniAgente::nuove(&chiave, fonti, PathBuf::from(env("BATTITO", "/stato/battito.json")), &zona);
    o.chiave_teams = chiave_teams;
    o.token_posta = token_posta;
    o.accessi = accessi;
    o.adesso = adesso;
    o.registro = registro.clone();
    o.tokens = Some(tokens);
    o.google = Some(google);
    let agente = Agente::nuovo(o);
    let ascolto = match TcpListener::bind(("0.0.0.0", porta)).await {
        Ok(a) => a,
        Err(e) => {
            registro(Livello::Error, &format!("listen {e}"));
            return 1;
        }
    };
    registro(Livello::Info, &format!("agente in ascolto sulla porta {porta}, fonti: {nomi}"));
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return 1 };
    tokio::select! {
        () = servi(ascolto, Rc::new(agente)) => 1,
        _ = term.recv() => 0,
        _ = int.recv() => 0,
    }
}
