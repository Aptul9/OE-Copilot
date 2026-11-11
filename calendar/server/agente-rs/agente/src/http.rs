//! Il server HTTP/1.1 dell'agente, scritto a mano su tokio. Risponde come `http.createServer` di Node dove il gateway e le prove lo
//! vedono: la frase della riga di stato la sceglie il gestore, le intestazioni si leggono come latin1 e di due uguali vale la prima, una
//! richiesta che il parser di Node rifiuta prende 400 senza arrivare al gestore, un corpo non letto si scarta a risposta data.

use std::future::Future;
use std::io;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

/// Riga di richiesta e intestazioni insieme, come `maxHeaderSize` di Node.
const MAX_TESTA: usize = 16 * 1024;
const ATTESA_TESTA: Duration = Duration::from_secs(60);
const ATTESA_RICHIESTA: Duration = Duration::from_secs(300);
/// `keepAliveTimeout` di Node: quanto resta aperta una connessione fra una richiesta e la successiva.
const ATTESA_INATTIVA: Duration = Duration::from_secs(5);

pub struct Risposta {
    pub codice: u16,
    pub motivo: String,
    /// Nell'ordine in cui vanno scritte; `Date` la aggiunge il server.
    pub intestazioni: Vec<(String, String)>,
    pub corpo: Vec<u8>,
    /// La connessione resta aperta finché il cliente finisce di mandare il corpo che il gestore non ha letto, poi serve la richiesta dopo.
    pub scarta_corpo: bool,
}

pub enum Esito {
    Risposta(Risposta),
    /// La connessione si chiude senza rispondere.
    Chiudi,
}

pub trait Gestore {
    fn gestisci(&self, r: &mut Richiesta<'_>) -> impl Future<Output = Esito>;
}

enum Corpo {
    Nessuno,
    Lunghezza(u64),
    Pezzi,
}

pub struct Richiesta<'a> {
    pub metodo: String,
    /// Il bersaglio della riga di richiesta, com'è arrivato.
    pub url: String,
    /// "1.1" o "1.0".
    pub versione: &'static str,
    pub remoto: &'a str,
    /// Nomi in minuscolo, valori letti come latin1 e senza spazi ai bordi.
    pub intestazioni: Vec<(String, String)>,
    corpo: Corpo,
    conn: &'a mut Conn,
}

impl Richiesta<'_> {
    /// `req.headers[nome]` per le intestazioni di cui Node tiene solo la prima (authorization, content-length, content-type, host).
    pub fn intestazione(&self, nome: &str) -> Option<&str> {
        self.intestazioni.iter().find(|(k, _)| k == nome).map(|(_, v)| v.as_str())
    }

    /// Il corpo intero; `None` se il cliente se ne va prima della fine.
    pub async fn leggi_corpo(&mut self) -> Option<Vec<u8>> {
        let corpo = std::mem::replace(&mut self.corpo, Corpo::Nessuno);
        let lettura = async {
            match corpo {
                Corpo::Nessuno => Ok(Vec::new()),
                Corpo::Lunghezza(n) => self.conn.leggi(n as usize).await,
                Corpo::Pezzi => {
                    let mut out = Vec::new();
                    self.conn.pezzi(Some(&mut out)).await.map(|()| out)
                }
            }
        };
        timeout(ATTESA_RICHIESTA, lettura).await.ok()?.ok()
    }

    async fn scarta(&mut self) -> bool {
        let corpo = std::mem::replace(&mut self.corpo, Corpo::Nessuno);
        let scarto = async {
            match corpo {
                Corpo::Nessuno => Ok(()),
                Corpo::Lunghezza(n) => self.conn.scarta(n).await,
                Corpo::Pezzi => self.conn.pezzi(None).await,
            }
        };
        matches!(timeout(ATTESA_RICHIESTA, scarto).await, Ok(Ok(())))
    }
}

struct Conn {
    s: TcpStream,
    /// I byte arrivati e non ancora consumati.
    buf: Vec<u8>,
}

struct Parti {
    metodo: String,
    url: String,
    versione: &'static str,
    intestazioni: Vec<(String, String)>,
    corpo: Corpo,
}

enum Testa {
    Fatta(Parti),
    Chiusa,
    Errore(u16),
}

fn trova(dove: &[u8], cosa: &[u8]) -> Option<usize> {
    dove.windows(cosa.len()).position(|f| f == cosa)
}

fn fine_prematura() -> io::Error {
    io::Error::from(io::ErrorKind::UnexpectedEof)
}

impl Conn {
    async fn altro(&mut self) -> io::Result<()> {
        self.buf.reserve(4096);
        if self.s.read_buf(&mut self.buf).await? == 0 {
            return Err(fine_prematura());
        }
        Ok(())
    }

    async fn testa(&mut self) -> Testa {
        loop {
            while self.buf.starts_with(b"\r\n") {
                self.buf.drain(..2);
            }
            if let Some(fine) = trova(&self.buf, b"\r\n\r\n") {
                if fine > MAX_TESTA {
                    return Testa::Errore(431);
                }
                let esito = analizza_testa(&self.buf[..fine]);
                self.buf.drain(..fine + 4);
                return esito.map_or_else(Testa::Errore, Testa::Fatta);
            }
            if self.buf.len() > MAX_TESTA {
                return Testa::Errore(431);
            }
            if self.altro().await.is_err() {
                return Testa::Chiusa;
            }
        }
    }

    async fn leggi(&mut self, n: usize) -> io::Result<Vec<u8>> {
        // la capacità cresce con quello che arriva davvero: una lunghezza dichiarata e mai mandata non occupa memoria
        let mut out = Vec::with_capacity(n.min(64 * 1024));
        let presi = n.min(self.buf.len());
        out.extend_from_slice(&self.buf[..presi]);
        self.buf.drain(..presi);
        while out.len() < n {
            let manca = n - out.len();
            out.reserve(manca.min(256 * 1024));
            if (&mut self.s).take(manca as u64).read_buf(&mut out).await? == 0 {
                return Err(fine_prematura());
            }
        }
        Ok(out)
    }

    async fn scarta(&mut self, n: u64) -> io::Result<()> {
        let presi = (n.min(self.buf.len() as u64)) as usize;
        self.buf.drain(..presi);
        let mut manca = n - presi as u64;
        let mut scarto = vec![0u8; 16 * 1024];
        while manca > 0 {
            let fin = (manca.min(scarto.len() as u64)) as usize;
            let letti = self.s.read(&mut scarto[..fin]).await?;
            if letti == 0 {
                return Err(fine_prematura());
            }
            manca -= letti as u64;
        }
        Ok(())
    }

    async fn riga(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some(fine) = trova(&self.buf, b"\r\n") {
                let riga = self.buf[..fine].to_vec();
                self.buf.drain(..fine + 2);
                return Ok(riga);
            }
            if self.buf.len() > MAX_TESTA {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            self.altro().await?;
        }
    }

    /// Un corpo `Transfer-Encoding: chunked`, fino al pezzo vuoto e alle intestazioni in coda.
    async fn pezzi(&mut self, mut out: Option<&mut Vec<u8>>) -> io::Result<()> {
        loop {
            let riga = self.riga().await?;
            let cifre = riga.split(|b| *b == b';').next().unwrap_or_default();
            let testo = std::str::from_utf8(cifre).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?.trim_matches([' ', '\t']);
            let n = u64::from_str_radix(testo, 16).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            if n == 0 {
                while !self.riga().await?.is_empty() {}
                return Ok(());
            }
            match out.as_deref_mut() {
                Some(o) => o.extend_from_slice(&self.leggi(n as usize).await?),
                None => self.scarta(n).await?,
            }
            if !self.riga().await?.is_empty() {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
        }
    }

    /// Chiude dopo aver scritto: prima la nostra metà, poi si legge e si butta quello che il cliente sta ancora mandando, perché chiudere
    /// con byte non letti azzera la connessione e la risposta può non arrivare.
    async fn chiudi_piano(&mut self) {
        let _ = self.s.shutdown().await;
        let mut scarto = vec![0u8; 16 * 1024];
        let _ = timeout(Duration::from_secs(2), async {
            while matches!(self.s.read(&mut scarto).await, Ok(n) if n > 0) {}
        })
        .await;
    }

    /// La risposta di Node a una richiesta che il suo parser non accetta.
    async fn rifiuta(&mut self, codice: u16) {
        let frase = match codice {
            431 => "Request Header Fields Too Large",
            _ => "Bad Request",
        };
        let _ = self.s.write_all(format!("HTTP/1.1 {codice} {frase}\r\nConnection: close\r\n\r\n").as_bytes()).await;
        self.chiudi_piano().await;
    }
}

/// I metodi che il parser di Node conosce (`http.METHODS`): con un altro la richiesta prende 400.
const METODI: [&str; 35] = [
    "ACL", "BIND", "CHECKOUT", "CONNECT", "COPY", "DELETE", "GET", "HEAD", "LINK", "LOCK", "M-SEARCH", "MERGE", "MKACTIVITY", "MKCALENDAR", "MKCOL", "MOVE", "NOTIFY", "OPTIONS", "PATCH", "POST",
    "PROPFIND", "PROPPATCH", "PURGE", "PUT", "QUERY", "REBIND", "REPORT", "SEARCH", "SOURCE", "SUBSCRIBE", "TRACE", "UNBIND", "UNLINK", "UNLOCK", "UNSUBSCRIBE",
];

fn di_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn spazi_ai_bordi(mut v: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', resto @ ..] = v {
        v = resto;
    }
    while let [resto @ .., b' ' | b'\t'] = v {
        v = resto;
    }
    v
}

fn analizza_testa(testa: &[u8]) -> Result<Parti, u16> {
    let mut righe: Vec<&[u8]> = Vec::new();
    let mut da = 0;
    let mut i = 0;
    while i + 1 < testa.len() {
        if testa[i] == b'\r' && testa[i + 1] == b'\n' {
            righe.push(&testa[da..i]);
            da = i + 2;
            i += 2;
        } else {
            i += 1;
        }
    }
    righe.push(&testa[da..]);

    let mut pezzi = righe[0].split(|b| *b == b' ');
    let (Some(metodo), Some(url), Some(versione), None) = (pezzi.next(), pezzi.next(), pezzi.next(), pezzi.next()) else { return Err(400) };
    if !METODI.iter().any(|m| m.as_bytes() == metodo) {
        return Err(400);
    }
    // Node rifiuta i byte non ASCII grezzi nel bersaglio: chi chiama li codifica con %
    if url.is_empty() || !url.iter().all(|b| (0x21..=0x7e).contains(b)) {
        return Err(400);
    }
    let versione = match versione {
        b"HTTP/1.1" => "1.1",
        b"HTTP/1.0" => "1.0",
        _ => return Err(400),
    };

    let mut intestazioni: Vec<(String, String)> = Vec::with_capacity(righe.len() - 1);
    for riga in &righe[1..] {
        let due_punti = riga.iter().position(|b| *b == b':').ok_or(400u16)?;
        let nome = &riga[..due_punti];
        if nome.is_empty() || !nome.iter().all(|b| di_token(*b)) {
            return Err(400);
        }
        let valore = spazi_ai_bordi(&riga[due_punti + 1..]);
        if valore.iter().any(|b| (*b < 0x20 && *b != b'\t') || *b == 0x7f) {
            return Err(400);
        }
        // nome fatto di soli caratteri di token, quindi ASCII; il valore è latin1, un byte un carattere
        intestazioni.push((nome.iter().map(|b| b.to_ascii_lowercase() as char).collect(), valore.iter().map(|b| *b as char).collect()));
    }

    let valori = |nome: &str| -> Vec<&str> { intestazioni.iter().filter(|(k, _)| k == nome).map(|(_, v)| v.as_str()).collect() };
    let lunghezze = valori("content-length");
    let codifiche = valori("transfer-encoding");
    let corpo = if !codifiche.is_empty() {
        let ultima = codifiche.last().and_then(|v| v.rsplit(',').next()).unwrap_or_default().trim_matches([' ', '\t']);
        if !lunghezze.is_empty() || !ultima.eq_ignore_ascii_case("chunked") {
            return Err(400);
        }
        Corpo::Pezzi
    } else if let Some(prima) = lunghezze.first() {
        if prima.is_empty() || !prima.bytes().all(|b| b.is_ascii_digit()) || lunghezze.iter().any(|v| v != prima) {
            return Err(400);
        }
        match prima.parse::<u64>().map_err(|_| 400u16)? {
            0 => Corpo::Nessuno,
            n => Corpo::Lunghezza(n),
        }
    } else {
        Corpo::Nessuno
    };

    // metodo e bersaglio sono ASCII, controllati sopra
    Ok(Parti { metodo: String::from_utf8_lossy(metodo).into_owned(), url: String::from_utf8_lossy(url).into_owned(), versione, intestazioni, corpo })
}

/// La data di un'intestazione `Date`: "Thu, 01 Jan 1970 00:00:00 GMT".
fn data_http(secondi: u64) -> String {
    const GIORNI: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MESI: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let giorni = secondi / 86400;
    let s = secondi % 86400;
    let z = giorni + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let giorno = doy - (153 * mp + 2) / 5 + 1;
    let mese = if mp < 10 { mp + 3 } else { mp - 9 };
    let anno = yoe + era * 400 + u64::from(mese <= 2);
    format!("{}, {:02} {} {} {:02}:{:02}:{:02} GMT", GIORNI[(giorni % 7) as usize], giorno, MESI[(mese - 1) as usize], anno, s / 3600, s % 3600 / 60, s % 60)
}

fn testa_risposta(r: &Risposta) -> Vec<u8> {
    let mut t = format!("HTTP/1.1 {} {}\r\n", r.codice, r.motivo);
    for (k, v) in &r.intestazioni {
        t.push_str(k);
        t.push_str(": ");
        t.push_str(v);
        t.push_str("\r\n");
    }
    t.push_str("Date: ");
    t.push_str(&data_http(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())));
    t.push_str("\r\n");
    if r.scarta_corpo {
        t.push_str("Keep-Alive: timeout=5\r\n");
    }
    t.push_str("\r\n");
    t.into_bytes()
}

async fn connessione<G: Gestore>(s: TcpStream, remoto: String, gestore: Rc<G>) {
    let mut conn = Conn { s, buf: Vec::new() };
    let mut attesa = ATTESA_TESTA;
    loop {
        let parti = match timeout(attesa, conn.testa()).await {
            Ok(Testa::Fatta(p)) => p,
            Ok(Testa::Errore(codice)) => return conn.rifiuta(codice).await,
            Ok(Testa::Chiusa) | Err(_) => return,
        };
        // senza un ascoltatore di "connect" Node chiude la connessione
        if parti.metodo == "CONNECT" {
            return;
        }
        let continua = parti.versione == "1.1" && parti.intestazioni.iter().any(|(k, v)| k == "expect" && v.eq_ignore_ascii_case("100-continue"));
        if continua && conn.s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.is_err() {
            return;
        }
        let mut r = Richiesta { metodo: parti.metodo, url: parti.url, versione: parti.versione, remoto: &remoto, intestazioni: parti.intestazioni, corpo: parti.corpo, conn: &mut conn };
        let Esito::Risposta(risposta) = gestore.gestisci(&mut r).await else { return };
        let mut scritto = r.conn.s.write_all(&testa_risposta(&risposta)).await;
        if scritto.is_ok() && !risposta.corpo.is_empty() {
            scritto = r.conn.s.write_all(&risposta.corpo).await;
        }
        if scritto.is_err() {
            return;
        }
        if !risposta.scarta_corpo {
            return r.conn.chiudi_piano().await;
        }
        if !r.scarta().await {
            return;
        }
        attesa = ATTESA_INATTIVA;
    }
}

/// Accetta connessioni per sempre, una attività locale per connessione (va chiamata dentro un `LocalSet`).
pub async fn servi<G: Gestore + 'static>(ascolto: TcpListener, gestore: Rc<G>) {
    loop {
        let Ok((s, remoto)) = ascolto.accept().await else {
            // descrittori finiti o connessione già caduta: si riprova fra poco
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let _ = s.set_nodelay(true);
        tokio::task::spawn_local(connessione(s, remoto.ip().to_string(), gestore.clone()));
    }
}

#[cfg(test)]
mod prove {
    use super::*;
    use tokio::task::LocalSet;

    struct Eco;

    impl Gestore for Eco {
        async fn gestisci(&self, r: &mut Richiesta<'_>) -> Esito {
            if r.url == "/chiudi" {
                return Esito::Chiudi;
            }
            let scarta = r.url == "/scarta";
            let corpo = if scarta { Vec::new() } else { r.leggi_corpo().await.unwrap_or_default() };
            let testo = format!("{} {} {} [{}] {}", r.metodo, r.url, r.versione, r.intestazione("authorization").unwrap_or("-"), String::from_utf8_lossy(&corpo));
            let corpo = if r.metodo == "HEAD" { Vec::new() } else { testo.clone().into_bytes() };
            Esito::Risposta(Risposta {
                codice: if scarta { 413 } else { 200 },
                motivo: if scarta { "Request Entity Too Large" } else { "Va bene ('x')" }.to_string(),
                intestazioni: vec![("Content-Length".to_string(), testo.len().to_string()), ("Connection".to_string(), if scarta { "keep-alive" } else { "close" }.to_string())],
                corpo,
                scarta_corpo: scarta,
            })
        }
    }

    async fn scambio(richiesta: &[u8]) -> String {
        LocalSet::new()
            .run_until(async {
                let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let porta = ascolto.local_addr().unwrap().port();
                tokio::task::spawn_local(servi(ascolto, Rc::new(Eco)));
                let mut s = TcpStream::connect(("127.0.0.1", porta)).await.unwrap();
                s.write_all(richiesta).await.unwrap();
                let mut out = Vec::new();
                let _ = timeout(Duration::from_secs(10), s.read_to_end(&mut out)).await.unwrap();
                out.iter().map(|b| *b as char).collect()
            })
            .await
    }

    fn senza_data(risposta: &str) -> String {
        risposta.split("\r\n").filter(|r| !r.starts_with("Date: ")).collect::<Vec<_>>().join("\r\n")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn una_richiesta_semplice_e_la_frase_del_gestore() {
        let r = scambio(b"GET /api/salute?x=1 HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer uno \r\nauthorization: Bearer due\r\n\r\n").await;
        assert_eq!(senza_data(&r), "HTTP/1.1 200 Va bene ('x')\r\nContent-Length: 37\r\nConnection: close\r\n\r\nGET /api/salute?x=1 1.1 [Bearer uno] ");
        assert!(r.contains("\r\nDate: ") && r.contains(" GMT\r\n"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn il_corpo_con_la_lunghezza_e_a_pezzi() {
        let r = scambio(b"POST /a HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello").await;
        assert!(r.ends_with("POST /a 1.1 [-] hello"), "{r}");
        let r = scambio(b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nhel\r\n2;x=1\r\nlo\r\n0\r\nT: 1\r\n\r\n").await;
        assert!(r.ends_with("POST /a 1.1 [-] hello"), "{r}");
        let r = scambio(b"HEAD /a HTTP/1.0\r\n\r\n").await;
        assert_eq!(senza_data(&r), "HTTP/1.1 200 Va bene ('x')\r\nContent-Length: 16\r\nConnection: close\r\n\r\n");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn le_intestazioni_sono_latin1() {
        let r = scambio(b"GET /a HTTP/1.1\r\nAuthorization: Bearer chiave-\xc3\xa9\r\n\r\n").await;
        // il gestore riscrive in UTF-8 i due caratteri latin1, e `scambio` rilegge ogni byte come un carattere
        let atteso: String = "[Bearer chiave-\u{c3}\u{a9}] ".bytes().map(|b| b as char).collect();
        assert!(r.ends_with(&atteso), "{r}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn quello_che_node_rifiuta_prende_400() {
        for richiesta in [
            &b"GET /api/salute?x=\xc3\xa9 HTTP/1.1\r\n\r\n"[..],
            b"GET /a b HTTP/1.1\r\n\r\n",
            b"get /a HTTP/1.1\r\n\r\n",
            b"GET /a HTTP/2.0\r\n\r\n",
            b"GET /a HTTP/1.1\r\nNome sbagliato: x\r\n\r\n",
            b"GET /a HTTP/1.1\r\nSenza due punti\r\n\r\n",
            b"POST /a HTTP/1.1\r\nContent-Length: +5\r\n\r\nhello",
            b"POST /a HTTP/1.1\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\nhello",
            b"GET /a HTTP/1.1\r\nX: a\nb\r\n\r\n",
        ] {
            assert_eq!(scambio(richiesta).await, "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n", "{}", String::from_utf8_lossy(richiesta));
        }
        let mut lunga = b"GET /a HTTP/1.1\r\nX: ".to_vec();
        lunga.extend(std::iter::repeat_n(b'a', 20_000));
        lunga.extend_from_slice(b"\r\n\r\n");
        assert_eq!(scambio(&lunga).await, "HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn chiudere_senza_rispondere() {
        assert_eq!(scambio(b"GET /chiudi HTTP/1.1\r\n\r\n").await, "");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn un_corpo_scartato_lascia_la_connessione_alla_richiesta_dopo() {
        let r = scambio(b"POST /scarta HTTP/1.1\r\nContent-Length: 5\r\n\r\nhelloGET /dopo HTTP/1.1\r\n\r\n").await;
        let risposte: Vec<&str> = r.split("HTTP/1.1 ").collect();
        assert_eq!(risposte.len(), 3, "{r}");
        assert!(risposte[1].starts_with("413 Request Entity Too Large\r\n") && risposte[1].contains("\r\nKeep-Alive: timeout=5\r\n"), "{r}");
        assert!(risposte[2].ends_with("GET /dopo 1.1 [-] "), "{r}");
        let r = scambio(b"POST /scarta HTTP/1.1\r\nExpect: 100-continue\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\nGET /dopo HTTP/1.1\r\n\r\n").await;
        assert!(r.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 413 ") && r.ends_with("GET /dopo 1.1 [-] "), "{r}");
    }

    #[test]
    fn la_data() {
        assert_eq!(data_http(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(data_http(951_782_400 + 3661), "Tue, 29 Feb 2000 01:01:01 GMT");
        assert_eq!(data_http(1_794_370_257), "Wed, 11 Nov 2026 04:10:57 GMT");
    }
}
