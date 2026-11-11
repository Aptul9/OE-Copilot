//! Il gateway gira su una macchina con altri servizi e con l'indirizzo dei metadati del cloud: quando scarica un indirizzo scelto da chi usa
//! l'app (i loghi delle aziende, i calendari da link) deve arrivare solo a siti pubblici (era lib/core/rete-sicura.ts). Solo http e https
//! sulle porte 80 e 443, mai credenziali nell'indirizzo, il nome si risolve qui e ogni indirizzo trovato deve essere pubblico (si rifiuta
//! tutto il nome se anche uno solo non lo è), l'indirizzo a cui ci si collega è controllato di nuovo a collegamento fatto; ogni
//! reindirizzamento (al massimo 3) ricomincia da capo. Senza altre richieste valgono i limiti dei loghi: tempo totale 8 secondi, al più 1 MB
//! letto. Il contenuto dichiarato dal sito non conta: il tipo lo decide chi chiama, dai primi byte.

use crate::py::{lunghezza_js, DettaglioHttp, Errore, Esito, Motivo};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::{Host, Url};

pub const ATTESA_MS: u64 = 8000;
pub const MAX_REINDIRIZZAMENTI: usize = 3;
/// Quello che si legge dalla rete al massimo, qualunque cosa dica il sito.
pub const LIMITE_LETTURA: usize = 1024 * 1024;
/// Quello che si accetta di un logo.
pub const LIMITE_ACCETTATO: usize = 512 * 1024;
/// Quanti pezzi di corpo si accettano: un corpo a pezzi di pochi byte costa un giro a pezzo anche se il corpo è piccolo.
pub const PEZZI_MAX: usize = 20_000;
const INDIRIZZO_MAX: usize = 2048;

pub const NON_VALIDO: &str = "Indirizzo non valido";
pub const NON_AMMESSO: &str = "Indirizzo non ammesso";
pub const NON_RISPONDE: &str = "L'indirizzo non risponde";
pub const CERTIFICATO: &str = "Il certificato del sito non è valido";

pub fn testo_reindirizzamenti() -> String {
    format!("Troppi reindirizzamenti (massimo {MAX_REINDIRIZZAMENTI})")
}

pub fn testo_troppo_grande() -> String {
    format!("Immagine troppo grande (massimo {} KB)", LIMITE_ACCETTATO / 1024)
}

pub fn errore(motivo: Motivo, messaggio: impl Into<String>) -> Errore {
    Errore::Rete { motivo, messaggio: messaggio.into(), http: None }
}

/// Quello che cambia da uno scarico all'altro: i limiti e le intestazioni. Le regole sugli indirizzi sono le stesse per tutti.
#[derive(Clone, Debug)]
pub struct Opzioni {
    pub limite_lettura: usize,
    pub limite_accettato: usize,
    pub attesa_ms: u64,
    pub accept: String,
    pub user_agent: String,
    pub testo_troppo_grande: String,
}

impl Default for Opzioni {
    fn default() -> Self {
        Opzioni {
            limite_lettura: LIMITE_LETTURA,
            limite_accettato: LIMITE_ACCETTATO,
            attesa_ms: ATTESA_MS,
            accept: "image/png,image/jpeg,image/gif,image/webp,image/svg+xml,image/x-icon,image/vnd.microsoft.icon,*/*;q=0.5".to_string(),
            user_agent: "OECopilot/1.0".to_string(),
            testo_troppo_grande: testo_troppo_grande(),
        }
    }
}

// ---- Quali indirizzi sono pubblici -------------------------------------------------------------------------------------------
// Tutto quello che non è un indirizzo unicast globale: loopback, privati, link-local (anche 169.254.169.254, i metadati), CGNAT,
// documentazione, multicast, riservati, non specificato; IPv6: loopback e non specificato, IPv4 mappati e "tradotti", NAT64, unique-local,
// link-local, site-local, multicast, 6to4 e Teredo, che portano un IPv4 dentro. IPv4 e IPv6 sono due elenchi separati: un IPv4 si confronta
// solo con le reti IPv4.
const RETI_V4: [(Ipv4Addr, u32); 15] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

const RETI_V6: [(Ipv6Addr, u32); 13] = [
    (Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0), 96),
    (Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0, 0), 96),
    (Ipv6Addr::new(0, 0, 0, 0, 0xffff, 0, 0, 0), 96),
    (Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0, 0), 96),
    (Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0), 48),
    (Ipv6Addr::new(0x100, 0, 0, 0, 0, 0, 0, 0), 64),
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
    (Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
    (Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10),
    (Ipv6Addr::new(0xfec0, 0, 0, 0, 0, 0, 0, 0), 10),
    (Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8),
];

fn dentro(ip: u128, rete: u128, bit: u32, larghezza: u32) -> bool {
    let maschera = if bit == 0 { 0 } else { (!0u128) << (larghezza - bit) };
    let maschera = maschera & if larghezza == 128 { !0u128 } else { (1u128 << larghezza) - 1 };
    ip & maschera == rete & maschera
}

/// Un indirizzo IP (non un nome) che si può raggiungere: pubblico. Qualunque altra cosa è no.
pub fn indirizzo_ammesso(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => !RETI_V4.iter().any(|(r, b)| dentro(u128::from(u32::from(*v)), u128::from(u32::from(*r)), *b, 32)),
        IpAddr::V6(v) => !RETI_V6.iter().any(|(r, b)| dentro(u128::from(*v), u128::from(*r), *b, 128)),
    }
}

/// Un testo che è un indirizzo IP (net.isIP), letto.
pub fn ip_da_testo(s: &str) -> Option<IpAddr> {
    s.parse::<IpAddr>().ok()
}

pub fn porta_ammessa(porta: u16) -> bool {
    porta == 80 || porta == 443
}

/// Solo per le prove: il risolutore di nomi, le regole sugli indirizzi e sulle porte, il tempo e la CA di un certificato inventato. Nessun
/// codice del gateway lo imposta: esiste solo nelle prove (funzione "prove" del crate).
#[derive(Clone, Default)]
pub struct SoloProve {
    pub risolvi: Option<Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>>,
    pub ammetti_indirizzo: Option<Arc<dyn Fn(&IpAddr) -> bool + Send + Sync>>,
    pub ammetti_porta: Option<Arc<dyn Fn(u16) -> bool + Send + Sync>>,
    pub attesa_ms: Option<u64>,
    pub ca_pem: Option<Vec<u8>>,
}

#[derive(Clone)]
struct Regole {
    ammetti_indirizzo: Arc<dyn Fn(&IpAddr) -> bool + Send + Sync>,
    ammetti_porta: Arc<dyn Fn(u16) -> bool + Send + Sync>,
    risolvi: Option<Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>>,
    ca_pem: Option<Vec<u8>>,
}

fn regole(prove: Option<&SoloProve>) -> Regole {
    Regole {
        ammetti_indirizzo: prove.and_then(|p| p.ammetti_indirizzo.clone()).unwrap_or_else(|| Arc::new(indirizzo_ammesso)),
        ammetti_porta: prove.and_then(|p| p.ammetti_porta.clone()).unwrap_or_else(|| Arc::new(porta_ammessa)),
        risolvi: prove.and_then(|p| p.risolvi.clone()),
        ca_pem: prove.and_then(|p| p.ca_pem.clone()),
    }
}

/// I caratteri che un indirizzo non può avere: controllo, spazi, DEL e il blocco C1.
fn controllo_o_spazio(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{20}' | '\u{7f}'..='\u{9f}')
}

fn controlla_url_con(testo: &str, r: &Regole) -> Esito<Url> {
    let t = crate::py::js_trim(testo);
    if t.is_empty() || lunghezza_js(t) > INDIRIZZO_MAX || t.chars().any(controllo_o_spazio) {
        return Err(errore(Motivo::NonValido, NON_VALIDO));
    }
    let url = Url::parse(t).map_err(|_| errore(Motivo::NonValido, NON_VALIDO))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
    }
    if !url.username().is_empty() || url.password().is_some_and(|p| !p.is_empty()) {
        return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
    }
    let porta = url.port().unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    if !(r.ammetti_porta)(porta) {
        return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
    }
    match url.host() {
        None => return Err(errore(Motivo::NonValido, NON_VALIDO)),
        Some(Host::Ipv4(ip)) => {
            if !(r.ammetti_indirizzo)(&IpAddr::V4(ip)) {
                return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
            }
        }
        Some(Host::Ipv6(ip)) => {
            if !(r.ammetti_indirizzo)(&IpAddr::V6(ip)) {
                return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
            }
        }
        Some(Host::Domain(d)) => {
            let host = d.to_lowercase();
            let host = host.strip_suffix('.').unwrap_or(&host);
            if host.is_empty() {
                return Err(errore(Motivo::NonValido, NON_VALIDO));
            }
            // un nome senza punti è un servizio della rete interna (agent, gateway...), mai un sito pubblico
            if host == "localhost" || host.ends_with(".localhost") || !host.contains('.') {
                return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
            }
        }
    }
    Ok(url)
}

/// L'indirizzo scritto da chi usa l'app come URL da raggiungere. L'analisi dell'URL riporta alla forma di sempre gli indirizzi scritti in
/// altro modo (2130706433, 0x7f.1, 0177.0.0.1, [::ffff:7f00:1]) e il controllo guarda quella.
pub fn controlla_url(testo: &str, prove: Option<&SoloProve>) -> Esito<Url> {
    controlla_url_con(testo, &regole(prove))
}

// ---- Lo scarico ---------------------------------------------------------------------------------------------------------------
/// Il risolutore di un passo: risolve il nome, rifiuta se un solo indirizzo non è pubblico, e ricorda gli indirizzi buoni per controllare
/// quello a cui il socket si collega davvero. Un nome che porta a un indirizzo non pubblico risponde come un nome che non esiste.
struct Risolutore {
    regole: Regole,
    consentiti: Arc<Mutex<HashSet<IpAddr>>>,
}

impl reqwest::dns::Resolve for Risolutore {
    fn resolve(&self, nome: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let regole = self.regole.clone();
        let consentiti = self.consentiti.clone();
        let host = nome.as_str().to_string();
        Box::pin(async move {
            let indirizzi: Vec<IpAddr> = match &regole.risolvi {
                Some(f) => f(&host).iter().filter_map(|s| ip_da_testo(s)).collect(),
                None => tokio::net::lookup_host((host.as_str(), 0)).await?.map(|a| a.ip()).collect(),
            };
            if indirizzi.is_empty() || !indirizzi.iter().all(|a| (regole.ammetti_indirizzo)(a)) {
                return Err(Box::new(std::io::Error::other(NON_RISPONDE)) as Box<dyn std::error::Error + Send + Sync>);
            }
            if let Ok(mut c) = consentiti.lock() {
                c.extend(indirizzi.iter().copied());
            }
            let iter: Box<dyn Iterator<Item = SocketAddr> + Send> = Box::new(indirizzi.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(iter)
        })
    }
}

/// `true` se l'errore della richiesta viene dal certificato del sito.
fn da_certificato(e: &reqwest::Error) -> bool {
    let mut testo = format!("{e:?}");
    let mut sorgente = std::error::Error::source(e);
    while let Some(s) = sorgente {
        testo.push_str(&format!(" {s:?}"));
        sorgente = s.source();
    }
    testo.contains("InvalidCertificate") || testo.contains("UnknownIssuer") || testo.contains("CertificateExpired") || testo.contains("NotValidForName") || testo.contains("certificate")
}

struct Passo {
    stato: u16,
    testo_stato: String,
    location: Option<String>,
    tipo: Option<String>,
    dati: Option<Vec<u8>>,
}

fn senza_risposta() -> Errore {
    errore(Motivo::NonRisponde, NON_RISPONDE)
}

/// Una sola richiesta GET, senza seguire i reindirizzamenti: lo stato, dove rimandano, e il corpo se la risposta è un successo.
async fn un_passo(url: &Url, r: &Regole, o: &Opzioni) -> Esito<Passo> {
    let consentiti: Arc<Mutex<HashSet<IpAddr>>> = Arc::new(Mutex::new(HashSet::new()));
    match url.host() {
        Some(Host::Ipv4(ip)) => {
            consentiti.lock().map(|mut c| c.insert(IpAddr::V4(ip))).ok();
        }
        Some(Host::Ipv6(ip)) => {
            consentiti.lock().map(|mut c| c.insert(IpAddr::V6(ip))).ok();
        }
        _ => {}
    }
    // un client per richiesta: niente connessioni tenute aperte e riusate tra un sito e l'altro
    let mut costruttore = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .dns_resolver(Arc::new(Risolutore { regole: r.clone(), consentiti: consentiti.clone() }));
    if let Some(pem) = &r.ca_pem {
        if let Ok(c) = reqwest::Certificate::from_pem(pem) {
            costruttore = costruttore.add_root_certificate(c);
        }
    }
    let client = costruttore.build().map_err(|_| senza_risposta())?;
    let risposta = client
        .get(url.clone())
        .header("Accept", &o.accept)
        .header("Accept-Encoding", "identity")
        .header("User-Agent", &o.user_agent)
        .send()
        .await
        .map_err(|e| if da_certificato(&e) { errore(Motivo::Certificato, CERTIFICATO) } else { senza_risposta() })?;
    // A collegamento fatto: l'indirizzo dall'altra parte deve essere uno di quelli controllati
    let collegato = risposta.remote_addr().map(|a| a.ip());
    let ok = collegato.is_some_and(|a| consentiti.lock().map(|c| c.contains(&a)).unwrap_or(false) && (r.ammetti_indirizzo)(&a));
    if !ok {
        return Err(errore(Motivo::NonAmmesso, NON_AMMESSO));
    }
    let stato = risposta.status().as_u16();
    let testo_stato = risposta
        .extensions()
        .get::<hyper::ext::ReasonPhrase>()
        .map(|f| String::from_utf8_lossy(f.as_bytes()).into_owned())
        .unwrap_or_else(|| risposta.status().canonical_reason().unwrap_or("").to_string());
    let location = risposta.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
    if !(200..300).contains(&stato) {
        // rimando, errore: il corpo non serve
        return Ok(Passo { stato, testo_stato, location, tipo: None, dati: None });
    }
    let dichiarata = risposta.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| crate::py::numero_da_testo(v).is_finite().then(|| crate::py::numero_da_testo(v)));
    if dichiarata.is_some_and(|d| d > o.limite_lettura as f64) {
        return Err(errore(Motivo::TroppoGrande, o.testo_troppo_grande.clone()));
    }
    let tipo = risposta.headers().get("content-type").map(|v| v.as_bytes().iter().map(|b| *b as char).collect::<String>());
    let capienza = match dichiarata {
        Some(d) if d >= 0.0 => d as usize,
        _ => o.limite_lettura,
    };
    let pezzi_max = (PEZZI_MAX as f64 * (o.limite_lettura as f64 / LIMITE_LETTURA as f64).max(1.0)).ceil() as usize;
    let mut corpo: Vec<u8> = Vec::with_capacity(capienza.min(64 * 1024));
    let mut pezzi = 0;
    let mut risposta = risposta;
    loop {
        match risposta.chunk().await {
            Ok(Some(c)) => {
                pezzi += 1;
                if pezzi > pezzi_max {
                    return Err(senza_risposta());
                }
                if corpo.len() + c.len() > capienza {
                    return Err(errore(Motivo::TroppoGrande, o.testo_troppo_grande.clone()));
                }
                corpo.extend_from_slice(&c);
            }
            Ok(None) => break,
            Err(_) => return Err(senza_risposta()),
        }
    }
    Ok(Passo { stato, testo_stato, location: None, tipo, dati: Some(corpo) })
}

pub struct Scaricato {
    pub dati: Vec<u8>,
    pub url: Url,
    /// Il Content-Type che dichiara il sito: non è una prova di niente.
    pub tipo_dichiarato: Option<String>,
}

/// Scarica `indirizzo` con le regole del modulo e restituisce i byte e l'indirizzo a cui si è arrivati. Restituisce Errore::Rete con il
/// testo che la pagina mostra. `prove` lo passa solo chi scrive le prove (SoloProve).
pub async fn scarica_sicuro(indirizzo: &str, prove: Option<&SoloProve>, o: &Opzioni) -> Esito<Scaricato> {
    let r = regole(prove);
    let attesa = Duration::from_millis(prove.and_then(|p| p.attesa_ms).unwrap_or(o.attesa_ms));
    let lavoro = async {
        let mut url = controlla_url_con(indirizzo, &r)?;
        let mut salti = 0;
        loop {
            let passo = un_passo(&url, &r, o).await?;
            if (300..400).contains(&passo.stato) && passo.location.is_some() {
                if salti >= MAX_REINDIRIZZAMENTI {
                    return Err(errore(Motivo::Reindirizzamenti, testo_reindirizzamenti()));
                }
                // ogni tappa ricomincia dai controlli sull'indirizzo, compreso il nome da risolvere di nuovo
                let prossimo = url.join(passo.location.as_deref().unwrap_or("")).map_err(|_| errore(Motivo::NonValido, NON_VALIDO))?;
                url = controlla_url_con(prossimo.as_str(), &r)?;
                salti += 1;
                continue;
            }
            let Some(dati) = passo.dati.filter(|_| (200..300).contains(&passo.stato)) else {
                return Err(Errore::Rete {
                    motivo: Motivo::ErroreHttp,
                    messaggio: format!("L'indirizzo risponde con un errore ({})", passo.stato),
                    http: Some(DettaglioHttp { stato: passo.stato, testo: passo.testo_stato, url: url.as_str().to_string() }),
                });
            };
            if dati.len() > o.limite_accettato {
                return Err(errore(Motivo::TroppoGrande, o.testo_troppo_grande.clone()));
            }
            return Ok(Scaricato { dati, url, tipo_dichiarato: passo.tipo });
        }
    };
    match tokio::time::timeout(attesa, lavoro).await {
        Ok(esito) => esito,
        Err(_) => Err(senza_risposta()),
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    fn ammesso(s: &str) -> bool {
        indirizzo_ammesso(&s.parse().unwrap())
    }

    #[test]
    fn indirizzi_pubblici_e_no() {
        for no in ["127.0.0.1", "10.1.2.3", "169.254.169.254", "100.64.0.1", "::1", "::ffff:127.0.0.1", "::ffff:0:a00:1", "64:ff9b::a00:1", "2002:a00:1::", "fd00::1", "fe80::1", "ff02::1", "0.0.0.0", "240.0.0.1", "2001::1", "2001:db8::1"] {
            assert!(!ammesso(no), "{no}");
        }
        for si in ["8.8.8.8", "1.1.1.1", "2a00:1450:4001:80b::200e", "2606:4700::1111"] {
            assert!(ammesso(si), "{si}");
        }
    }

    #[test]
    fn le_forme_strane_degli_indirizzi_si_riconoscono() {
        for no in ["http://2130706433/", "http://0x7f.1/", "http://0177.0.0.1/", "http://[::ffff:7f00:1]/", "http://127.1/", "http://agent:8180/", "http://localhost/", "http://x.localhost/", "http://agent./", "ftp://a.b/", "http://u:p@a.example/", "http://a.example:8080/"] {
            assert!(matches!(controlla_url(no, None), Err(Errore::Rete { motivo: Motivo::NonAmmesso, .. })), "{no}");
        }
        for no in ["", "notaurl", "http://a b.example/", "http:///"] {
            assert!(matches!(controlla_url(no, None), Err(Errore::Rete { motivo: Motivo::NonValido, .. })), "{no}");
        }
        assert_eq!(controlla_url(" https://Esempio.IT/x#y ", None).unwrap().as_str(), "https://esempio.it/x#y");
    }
}
