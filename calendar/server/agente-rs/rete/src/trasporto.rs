//! Il trasporto HTTP che l'agente TypeScript riceveva come `FetchFn`: una richiesta, una risposta con il corpo gia' letto (entro il limite
//! chiesto), le intestazioni in ordine con la ricerca senza maiuscole. L'agente gira su un solo thread (`current_thread` + `LocalSet`): il
//! trasporto e' un `Rc<dyn Trasporto>` e i suoi future non sono `Send`.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Le intestazioni come `Headers` di fetch: nomi in minuscolo, ordine di arrivo, piu' valori per lo stesso nome.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Intestazioni {
    voci: Vec<(String, String)>,
}

impl Intestazioni {
    pub fn new() -> Self {
        Self::default()
    }

    /// `headers.append`.
    pub fn aggiungi(&mut self, nome: &str, valore: impl Into<String>) {
        self.voci.push((nome.to_ascii_lowercase(), valore.into()));
    }

    /// `headers.set`: toglie i valori che c'erano e ne mette uno.
    pub fn imposta(&mut self, nome: &str, valore: impl Into<String>) {
        self.togli(nome);
        self.aggiungi(nome, valore);
    }

    /// `headers.delete`.
    pub fn togli(&mut self, nome: &str) {
        self.voci.retain(|(n, _)| !n.eq_ignore_ascii_case(nome));
    }

    /// `headers.get`: i valori dello stesso nome uniti da ", ".
    pub fn ottieni(&self, nome: &str) -> Option<String> {
        let mut it = self.tutti(nome);
        let primo = it.next()?;
        let mut out = primo.to_string();
        for v in it {
            out.push_str(", ");
            out.push_str(v);
        }
        Some(out)
    }

    /// Il primo valore, senza copiare.
    pub fn primo(&self, nome: &str) -> Option<&str> {
        self.voci.iter().find(|(n, _)| n.eq_ignore_ascii_case(nome)).map(|(_, v)| v.as_str())
    }

    /// Ogni valore di quel nome: `headers.getSetCookie()` per "set-cookie".
    pub fn tutti<'a>(&'a self, nome: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.voci.iter().filter(move |(n, _)| n.eq_ignore_ascii_case(nome)).map(|(_, v)| v.as_str())
    }

    pub fn contiene(&self, nome: &str) -> bool {
        self.voci.iter().any(|(n, _)| n.eq_ignore_ascii_case(nome))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.voci.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.voci.len()
    }

    pub fn is_empty(&self) -> bool {
        self.voci.is_empty()
    }
}

impl<N: AsRef<str>, V: Into<String>> FromIterator<(N, V)> for Intestazioni {
    fn from_iter<T: IntoIterator<Item = (N, V)>>(iter: T) -> Self {
        let mut h = Intestazioni::new();
        for (n, v) in iter {
            h.aggiungi(n.as_ref(), v);
        }
        h
    }
}

#[derive(Clone, Debug)]
pub struct Richiesta {
    pub metodo: String,
    pub url: String,
    pub intestazioni: Intestazioni,
    pub corpo: Option<Vec<u8>>,
    /// `redirect: "follow"` (il predefinito di fetch) o `"manual"`.
    pub segui_redirect: bool,
    /// `AbortSignal.timeout`: vale per tutto lo scambio, corpo compreso.
    pub scadenza: Duration,
    /// Oltre questi byte il corpo non si legge: vedi `Risposta::testo`.
    pub max_byte: Option<usize>,
}

impl Richiesta {
    pub fn get(url: impl Into<String>) -> Self {
        Richiesta {
            metodo: "GET".into(),
            url: url.into(),
            intestazioni: Intestazioni::new(),
            corpo: None,
            segui_redirect: true,
            scadenza: Duration::from_secs(20),
            max_byte: None,
        }
    }

    pub fn post(url: impl Into<String>, corpo: impl Into<Vec<u8>>) -> Self {
        Richiesta { metodo: "POST".into(), corpo: Some(corpo.into()), ..Richiesta::get(url) }
    }

    pub fn metodo(mut self, metodo: &str) -> Self {
        self.metodo = metodo.to_string();
        self
    }

    pub fn intestazione(mut self, nome: &str, valore: impl Into<String>) -> Self {
        self.intestazioni.imposta(nome, valore);
        self
    }

    pub fn senza_redirect(mut self) -> Self {
        self.segui_redirect = false;
        self
    }

    pub fn scadenza(mut self, d: Duration) -> Self {
        self.scadenza = d;
        self
    }

    pub fn max_byte(mut self, n: Option<usize>) -> Self {
        self.max_byte = n;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Risposta {
    pub stato: u16,
    pub intestazioni: Intestazioni,
    pub corpo: Vec<u8>,
    pub url_finale: String,
    /// Il corpo superava `max_byte`: il trasporto non l'ha letto. `testo()` e `byte()` rispondono con `RispostaGrande`.
    pub oltre_max: Option<usize>,
}

impl Risposta {
    pub fn nuova(stato: u16, corpo: impl Into<Vec<u8>>) -> Self {
        Risposta { stato, intestazioni: Intestazioni::new(), corpo: corpo.into(), url_finale: String::new(), oltre_max: None }
    }

    pub fn con_intestazione(mut self, nome: &str, valore: impl Into<String>) -> Self {
        self.intestazioni.aggiungi(nome, valore);
        self
    }

    /// `resp.ok`.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.stato)
    }

    /// `resp.statusText` per i codici che Node conosce; "" per gli altri.
    pub fn testo_stato(&self) -> &'static str {
        testo_stato(self.stato)
    }

    pub fn byte(&self) -> Result<&[u8], ErroreRete> {
        match self.oltre_max {
            Some(n) => Err(ErroreRete::RispostaGrande(n)),
            None => Ok(&self.corpo),
        }
    }

    /// `leggiTesto` / `resp.text()`: UTF-8 con i byte non validi sostituiti da U+FFFD, senza il BOM iniziale come `TextDecoder`.
    pub fn testo(&self) -> Result<String, ErroreRete> {
        let b = self.byte()?;
        let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b);
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}

pub fn testo_stato(stato: u16) -> &'static str {
    match stato {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// Gli errori di rete con il testo che l'agente TypeScript ne ricavava.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErroreRete {
    Timeout,
    /// `TypeError: fetch failed`: la causa resta qui per il debug, non nel testo.
    Rete(String),
    RispostaGrande(usize),
}

impl fmt::Display for ErroreRete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroreRete::Timeout => f.write_str("The operation was aborted due to timeout"),
            ErroreRete::Rete(_) => f.write_str("fetch failed"),
            ErroreRete::RispostaGrande(n) => write!(f, "risposta oltre {n} byte"),
        }
    }
}

impl std::error::Error for ErroreRete {}

pub type EsitoRete = Result<Risposta, ErroreRete>;
pub type FutureRete<'a> = Pin<Box<dyn Future<Output = EsitoRete> + 'a>>;

pub trait Trasporto {
    fn invia(&self, r: Richiesta) -> FutureRete<'_>;
}

const PREDEFINITE: [(&str, &str); 5] = [
    ("accept", "*/*"),
    ("accept-language", "*"),
    ("sec-fetch-mode", "cors"),
    ("user-agent", "node"),
    ("accept-encoding", "gzip, deflate"),
];

/// Il trasporto vero, con reqwest. Due client: uno segue i redirect come fetch, l'altro no (`redirect: "manual"`).
pub struct TrasportoReqwest {
    segue: reqwest::Client,
    manuale: reqwest::Client,
}

impl TrasportoReqwest {
    pub fn nuovo() -> Result<Self, ErroreRete> {
        let base = || reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).pool_max_idle_per_host(2);
        let segue = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(20))
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|e| ErroreRete::Rete(e.to_string()))?;
        let manuale = base().build().map_err(|e| ErroreRete::Rete(e.to_string()))?;
        Ok(TrasportoReqwest { segue, manuale })
    }

    async fn scambio(&self, r: Richiesta) -> EsitoRete {
        let rete = |e: &dyn fmt::Display| ErroreRete::Rete(e.to_string());
        let originale = r.url.clone();
        let (url, riscritto) = indirizzo(&r.url);
        let metodo = reqwest::Method::from_bytes(r.metodo.as_bytes()).map_err(|e| rete(&e))?;
        let client = if r.segui_redirect { &self.segue } else { &self.manuale };
        let mut rq = client.request(metodo, url.as_str());
        let mut h = r.intestazioni;
        for (n, v) in PREDEFINITE {
            if !h.contiene(n) {
                h.aggiungi(n, v);
            }
        }
        for (n, v) in h.iter() {
            let n = reqwest::header::HeaderName::from_bytes(n.as_bytes()).map_err(|e| rete(&e))?;
            let v = reqwest::header::HeaderValue::from_str(v).map_err(|e| rete(&e))?;
            rq = rq.header(n, v);
        }
        if let Some(c) = r.corpo {
            rq = rq.body(c);
        }
        let mut resp = rq.send().await.map_err(|e| rete(&e))?;
        let mut intest = Intestazioni::new();
        for (n, v) in resp.headers() {
            intest.aggiungi(n.as_str(), String::from_utf8_lossy(v.as_bytes()).into_owned());
        }
        let stato = resp.status().as_u16();
        let finale = resp.url().as_str();
        let url_finale = if riscritto && finale == url { originale } else { finale.to_string() };
        let mut risposta = Risposta { stato, intestazioni: intest, corpo: Vec::new(), url_finale, oltre_max: None };
        let max = r.max_byte;
        if let Some(max) = max {
            let dichiarata = risposta.intestazioni.primo("content-length").and_then(|v| v.trim().parse::<f64>().ok());
            if dichiarata.is_some_and(|d| d > max as f64) {
                risposta.oltre_max = Some(max);
                return Ok(risposta);
            }
        }
        while let Some(pezzo) = resp.chunk().await.map_err(|e| rete(&e))? {
            if max.is_some_and(|m| risposta.corpo.len() + pezzo.len() > m) {
                risposta.corpo = Vec::new();
                risposta.oltre_max = max;
                return Ok(risposta);
            }
            risposta.corpo.extend_from_slice(&pezzo);
        }
        Ok(risposta)
    }
}

impl Trasporto for TrasportoReqwest {
    fn invia(&self, r: Richiesta) -> FutureRete<'_> {
        Box::pin(async move {
            let scadenza = r.scadenza;
            match tokio::time::timeout(scadenza, self.scambio(r)).await {
                Ok(esito) => esito,
                Err(_) => Err(ErroreRete::Timeout),
            }
        })
    }
}

/// Con la funzione `rete-prova` e `OE_RETE_PROVA` impostata, `https://host/percorso` diventa `<base>/host/percorso` (base predefinita
/// `http://127.0.0.1:9000`). Il secondo valore dice se l'indirizzo e' stato riscritto.
#[cfg(feature = "rete-prova")]
fn indirizzo(url: &str) -> (String, bool) {
    if let (Ok(base), Some(resto)) = (std::env::var("OE_RETE_PROVA"), url.strip_prefix("https://")) {
        let base = if base.is_empty() { "http://127.0.0.1:9000" } else { base.trim_end_matches('/') };
        return (format!("{base}/{resto}"), true);
    }
    (url.to_string(), false)
}

#[cfg(not(feature = "rete-prova"))]
fn indirizzo(url: &str) -> (String, bool) {
    (url.to_string(), false)
}

/// Il trasporto finto dei test: risposte in sequenza o decise da una funzione, e il registro delle richieste ricevute.
pub mod prova {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    type Funzione = Box<dyn FnMut(&Richiesta) -> EsitoRete>;

    #[derive(Default)]
    pub struct TrasportoFinto {
        copione: RefCell<VecDeque<EsitoRete>>,
        funzione: RefCell<Option<Funzione>>,
        registro: RefCell<Vec<Richiesta>>,
    }

    impl TrasportoFinto {
        pub fn nuovo() -> Rc<Self> {
            Rc::new(Self::default())
        }

        /// Risposte consegnate nell'ordine, una per richiesta; finite quelle, ogni richiesta e' un errore di rete.
        pub fn con_copione(esiti: impl IntoIterator<Item = EsitoRete>) -> Rc<Self> {
            let t = Self::default();
            *t.copione.borrow_mut() = esiti.into_iter().collect();
            Rc::new(t)
        }

        /// Come la funzione `fetchFn` dei test TypeScript.
        pub fn con_funzione(f: impl FnMut(&Richiesta) -> EsitoRete + 'static) -> Rc<Self> {
            let t = Self::default();
            *t.funzione.borrow_mut() = Some(Box::new(f));
            Rc::new(t)
        }

        pub fn accoda(&self, esito: EsitoRete) {
            self.copione.borrow_mut().push_back(esito);
        }

        /// Le richieste ricevute finora.
        pub fn richieste(&self) -> Vec<Richiesta> {
            self.registro.borrow().clone()
        }

        pub fn chiamate(&self) -> usize {
            self.registro.borrow().len()
        }
    }

    impl Trasporto for TrasportoFinto {
        fn invia(&self, mut r: Richiesta) -> FutureRete<'_> {
            // come fetch, il corpo e' gia' un testo: il limite si applica qui come nel trasporto vero
            let esito = {
                self.registro.borrow_mut().push(r.clone());
                let da_funzione = self.funzione.borrow_mut().as_mut().map(|f| f(&r));
                let mut e = match da_funzione {
                    Some(e) => e,
                    None => self.copione.borrow_mut().pop_front().unwrap_or_else(|| Err(ErroreRete::Rete("copione finito".into()))),
                };
                if let (Ok(risp), Some(max)) = (&mut e, r.max_byte.take()) {
                    let dichiarata = risp.intestazioni.primo("content-length").and_then(|v| v.trim().parse::<f64>().ok());
                    if risp.corpo.len() > max || dichiarata.is_some_and(|d| d > max as f64) {
                        risp.corpo = Vec::new();
                        risp.oltre_max = Some(max);
                    }
                }
                if let Ok(risp) = &mut e {
                    if risp.url_finale.is_empty() {
                        risp.url_finale = r.url.clone();
                    }
                }
                e
            };
            Box::pin(async move { esito })
        }
    }

    /// Una risposta JSON, come `Response.json`.
    pub fn json(stato: u16, corpo: &str) -> Risposta {
        Risposta::nuova(stato, corpo.as_bytes().to_vec()).con_intestazione("content-type", "application/json")
    }

    pub fn rifiuto_rete() -> EsitoRete {
        Err(ErroreRete::Rete("prova".into()))
    }
}
