//! Le risposte del gateway, come le scriveva Next con `Response` e `Response.json`: lo stato, le intestazioni nell'ordine dato, il corpo con
//! la sua lunghezza. L'indirizzo di una richiesta è quello pubblico (APP_URL) con il percorso arrivato.

use oe_nucleo::json::Json;

pub struct Resp {
    pub stato: u16,
    pub intestazioni: Vec<(String, String)>,
    pub corpo: Vec<u8>,
    /// La connessione si chiude dopo la risposta (un corpo troppo grande che non si è letto).
    pub chiudi: bool,
}

impl Resp {
    pub fn nuova(stato: u16, intestazioni: &[(&str, &str)], corpo: Vec<u8>) -> Self {
        Resp { stato, intestazioni: intestazioni.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), corpo, chiudi: false }
    }

    /// `Response.json(corpo, { status, headers })`
    pub fn json(stato: u16, corpo: &Json, intestazioni: &[(&str, &str)]) -> Self {
        let mut h = vec![("content-type", "application/json")];
        h.extend_from_slice(intestazioni);
        Resp::nuova(stato, &h, corpo.in_js().into_bytes())
    }

    /// Una risposta senza corpo.
    pub fn vuota(stato: u16, intestazioni: &[(&str, &str)]) -> Self {
        Resp::nuova(stato, intestazioni, Vec::new())
    }

    pub fn rimando(stato: u16, dove: &str, intestazioni: &[(&str, &str)]) -> Self {
        let mut h = vec![("location", dove)];
        h.extend_from_slice(intestazioni);
        Resp::vuota(stato, &h)
    }

    pub fn con(mut self, k: &str, v: &str) -> Self {
        self.intestazioni.push((k.to_string(), v.to_string()));
        self
    }
}

/// La frase della riga di stato.
pub fn frase(stato: u16) -> &'static str {
    match stato {
        200 => "OK",
        201 => "Created",
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
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

const GIORNI: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MESI: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `new Date(ms).toUTCString()`: "Wed, 11 Nov 2026 04:10:57 GMT".
pub fn data_http(ms: i64) -> String {
    let (a, m, g, h, mi, s) = crate::fuso::pezzi_utc(ms);
    let giorno = (ms.div_euclid(86_400_000) + 4).rem_euclid(7) as usize;
    format!("{}, {g:02} {} {a:04} {h:02}:{mi:02}:{s:02} GMT", GIORNI[giorno], MESI[(m - 1) as usize])
}

/// `Date.parse` di una data HTTP (IMF-fixdate, quella che scrivono i browser e data_http): millisecondi, o None.
pub fn leggi_data_http(t: &str) -> Option<i64> {
    let p: Vec<&str> = crate::py::js_trim(t).split_whitespace().collect();
    let [_, g, m, a, ora, zona] = p.as_slice() else { return None };
    if !matches!(*zona, "GMT" | "UTC" | "Z") {
        return None;
    }
    let mese = MESI.iter().position(|x| x.eq_ignore_ascii_case(m))? as i64;
    let mut o = ora.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, s) = (o.next()??, o.next()??, o.next().flatten().unwrap_or(0));
    Some(crate::fuso::utc_js(a.parse().ok()?, mese, g.parse().ok()?, h, mi, s))
}

/// L'indirizzo intero di una richiesta: l'origine pubblica e il bersaglio arrivato (che comincia con /). None se il bersaglio non è un percorso.
pub fn indirizzo(bersaglio: &str) -> Option<url::Url> {
    if !bersaglio.starts_with('/') {
        return None;
    }
    let base = url::Url::parse(&crate::config::app_url()).ok()?;
    let origine = base.origin().ascii_serialization();
    url::Url::parse(&format!("{origine}{bersaglio}")).ok()
}
