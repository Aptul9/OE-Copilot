//! Che cosa è un file scaricato o caricato come logo: lo dicono i primi byte, mai l'estensione né il Content-Type dichiarato dal sito (era
//! lib/core/immagine.ts). Si ammettono PNG, JPEG, GIF, WebP, ICO e SVG. Di un raster si legge la grandezza dalla testata (di un GIF, di ogni
//! fotogramma e della tela) e si rifiuta oltre LATO_MAX pixel per lato o PIXEL_MAX in tutto. Un SVG si rifiuta se ha qualcosa di attivo o che
//! porta fuori (script, foreignObject, gestori di eventi, javascript: e data:, href che non siano #frammenti, CSS che carica da fuori). Ogni
//! lettura qui è lineare nella lunghezza del file: 512 KB ostili non fermano il gateway.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TipoImmagine {
    pub ext: &'static str,
    pub mime: &'static str,
}

pub const PNG: TipoImmagine = TipoImmagine { ext: "png", mime: "image/png" };
pub const JPG: TipoImmagine = TipoImmagine { ext: "jpg", mime: "image/jpeg" };
pub const GIF: TipoImmagine = TipoImmagine { ext: "gif", mime: "image/gif" };
pub const WEBP: TipoImmagine = TipoImmagine { ext: "webp", mime: "image/webp" };
pub const ICO: TipoImmagine = TipoImmagine { ext: "ico", mime: "image/vnd.microsoft.icon" };
pub const SVG: TipoImmagine = TipoImmagine { ext: "svg", mime: "image/svg+xml" };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EsitoImmagine {
    Tipo(TipoImmagine),
    NonImmagine,
    SvgNonAmmesso,
    TroppiPixel,
}

/// Il lato più lungo che si accetta di un raster, e i pixel in tutto (16 megapixel).
pub const LATO_MAX: f64 = 4096.0;
pub const PIXEL_MAX: f64 = 16_000_000.0;

// ---- Raster: il tipo dai primi byte, la grandezza dalla testata --------------------------------------------------------------------
// Un byte fuori dal file vale 0, come undefined nelle operazioni sui bit di JavaScript.
fn b(d: &[u8], i: usize) -> u32 {
    d.get(i).copied().map_or(0, u32::from)
}

fn inizia(d: &[u8], byte: &[u8], da: usize) -> bool {
    d.len() >= da + byte.len() && &d[da..da + byte.len()] == byte
}

fn u16le(d: &[u8], i: usize) -> f64 {
    f64::from(b(d, i) | (b(d, i + 1) << 8))
}
fn u16be(d: &[u8], i: usize) -> f64 {
    f64::from((b(d, i) << 8) | b(d, i + 1))
}
fn u24le(d: &[u8], i: usize) -> f64 {
    f64::from(b(d, i) | (b(d, i + 1) << 8) | (b(d, i + 2) << 16))
}
fn u32le_n(d: &[u8], i: usize) -> u32 {
    b(d, i) | (b(d, i + 1) << 8) | (b(d, i + 2) << 16) | (b(d, i + 3) << 24)
}
fn u32be(d: &[u8], i: usize) -> f64 {
    f64::from((b(d, i) << 24) | (b(d, i + 1) << 16) | (b(d, i + 2) << 8) | b(d, i + 3))
}
fn i32le(d: &[u8], i: usize) -> f64 {
    f64::from(u32le_n(d, i) as i32)
}

type Misura = Option<(f64, f64)>;

fn troppo_grande(l: f64, a: f64) -> bool {
    l > LATO_MAX || a > LATO_MAX || l * a > PIXEL_MAX
}

/// Una testata che non si legge o dice 0 non è un'immagine; una troppo grande si rifiuta con il suo testo.
fn verdetto(tipo: TipoImmagine, m: Misura) -> EsitoImmagine {
    match m {
        Some((l, a)) if l >= 1.0 && a >= 1.0 => {
            if troppo_grande(l, a) {
                EsitoImmagine::TroppiPixel
            } else {
                EsitoImmagine::Tipo(tipo)
            }
        }
        _ => EsitoImmagine::NonImmagine,
    }
}

const FIRMA_PNG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const IHDR: [u8; 4] = [0x49, 0x48, 0x44, 0x52];

/// PNG: la firma, la lunghezza del blocco, IHDR, poi larghezza e altezza (32 bit). `da` è dove comincia la firma.
fn misura_png(d: &[u8], da: usize) -> Misura {
    (d.len() >= da + 24 && inizia(d, &IHDR, da + 12)).then(|| (u32be(d, da + 16), u32be(d, da + 20)))
}

/// Quanti byte ha la tabella dei colori che dice un byte "impacchettato" (GIF).
fn tabella_gif(impacchettato: u32) -> usize {
    if impacchettato & 0x80 != 0 {
        3 << ((impacchettato & 7) + 1)
    } else {
        0
    }
}

/// I sotto-blocchi di dati di un blocco GIF: l'indice dopo l'ultimo, o None se escono dal file.
fn salta_sottoblocchi(d: &[u8], da: usize) -> Option<usize> {
    let mut i = da;
    while i < d.len() {
        let n = d[i] as usize;
        if n == 0 {
            return Some(i + 1);
        }
        i += n + 1;
    }
    None
}

/// GIF: ogni fotogramma e la tela (lo schermo e i bordi di destra e di sotto di ogni fotogramma) devono stare nei limiti. Una struttura che
/// non torna, senza fotogrammi o che finisce senza il trailer non è un GIF.
fn esito_gif(d: &[u8]) -> EsitoImmagine {
    if d.len() < 13 {
        return EsitoImmagine::NonImmagine;
    }
    let mut tela = (u16le(d, 6), u16le(d, 8));
    if tela.0 < 1.0 || tela.1 < 1.0 {
        return EsitoImmagine::NonImmagine;
    }
    let mut i = Some(13 + tabella_gif(b(d, 10)));
    let mut fotogrammi = 0;
    while let Some(j) = i.filter(|j| *j < d.len()) {
        let blocco = d[j];
        if blocco == 0x3b {
            // il trailer chiude il file, e quello che c'è dopo il browser non lo guarda
            if fotogrammi == 0 {
                return EsitoImmagine::NonImmagine;
            }
            return if troppo_grande(tela.0, tela.1) { EsitoImmagine::TroppiPixel } else { EsitoImmagine::Tipo(GIF) };
        }
        if blocco == 0x21 {
            i = salta_sottoblocchi(d, j + 2);
        } else if blocco == 0x2c && j + 10 <= d.len() {
            let (sinistra, sopra, larghezza, altezza) = (u16le(d, j + 1), u16le(d, j + 3), u16le(d, j + 5), u16le(d, j + 7));
            if troppo_grande(larghezza, altezza) {
                return EsitoImmagine::TroppiPixel;
            }
            tela = (tela.0.max(sinistra + larghezza), tela.1.max(sopra + altezza));
            fotogrammi += 1;
            // il descrittore (10 byte), la tabella locale, il byte della lunghezza minima dei codici e i sotto-blocchi dei dati
            i = salta_sottoblocchi(d, j + 10 + tabella_gif(b(d, j + 9)) + 1);
        } else {
            return EsitoImmagine::NonImmagine;
        }
    }
    EsitoImmagine::NonImmagine
}

/// WebP: il primo blocco dopo RIFF, lunghezza e WEBP è VP8X (tela, 24 bit), VP8 (con perdita, 14 bit) o VP8L (senza perdita, 14 bit).
fn misura_webp(d: &[u8]) -> Misura {
    if inizia(d, b"VP8X", 12) {
        return (d.len() >= 30).then(|| (u24le(d, 24) + 1.0, u24le(d, 27) + 1.0));
    }
    if inizia(d, b"VP8 ", 12) {
        return (d.len() >= 30 && inizia(d, &[0x9d, 0x01, 0x2a], 23)).then(|| (f64::from((b(d, 26) | (b(d, 27) << 8)) & 0x3fff), f64::from((b(d, 28) | (b(d, 29) << 8)) & 0x3fff)));
    }
    if inizia(d, b"VP8L", 12) && d.len() >= 25 && d[20] == 0x2f {
        let x = u32le_n(d, 21);
        return Some((f64::from(x & 0x3fff) + 1.0, f64::from((x >> 14) & 0x3fff) + 1.0));
    }
    None
}

/// JPEG: si scorrono i segmenti (ognuno dice quanto è lungo) fino al primo SOF.
fn misura_jpeg(d: &[u8]) -> Misura {
    let mut i = 2usize;
    while i + 4 <= d.len() {
        if d[i] != 0xff {
            return None;
        }
        let m = d[i + 1];
        if m == 0xff {
            i += 1; // byte di riempimento prima di un marker
            continue;
        }
        if m == 0x01 || (0xd0..=0xd8).contains(&m) {
            i += 2; // marker senza lunghezza (TEM, RSTn, SOI)
            continue;
        }
        // 00 non sta tra i segmenti; EOI e SOS senza un SOF prima: la testata non c'è
        if m == 0x00 || m == 0xd9 || m == 0xda {
            return None;
        }
        let lunghezza = u16be(d, i + 2) as usize;
        if lunghezza < 2 {
            return None;
        }
        // SOF0..SOF15, tranne DHT (c4), JPG (c8) e DAC (cc): marker, lunghezza, precisione, altezza, larghezza
        if (0xc0..=0xcf).contains(&m) && m != 0xc4 && m != 0xc8 && m != 0xcc {
            return (i + 9 <= d.len()).then(|| (u16be(d, i + 7), u16be(d, i + 5)));
        }
        i += 2 + lunghezza;
    }
    None
}

const TESTATE_DIB: [u32; 6] = [40, 52, 56, 64, 108, 124];

/// ICO: si guarda la testata di ogni immagine dentro (un PNG o un DIB), che è quella che il browser disegna.
fn esito_ico(d: &[u8], n: usize) -> EsitoImmagine {
    for k in 0..n {
        let da = u32le_n(d, 6 + 16 * k + 12) as usize;
        let mut m: Misura = None;
        if inizia(d, &FIRMA_PNG, da) {
            m = misura_png(d, da);
        } else if da + 16 <= d.len() {
            let testata = u32le_n(d, da);
            if testata == 12 {
                m = Some((u16le(d, da + 4), u16le(d, da + 6)));
            } else if TESTATE_DIB.contains(&testata) {
                m = Some((i32le(d, da + 4).abs(), i32le(d, da + 8).abs() / 2.0));
            }
        }
        if let Some((l, a)) = m {
            if troppo_grande(l, a) {
                return EsitoImmagine::TroppiPixel;
            }
        }
    }
    EsitoImmagine::Tipo(ICO)
}

/// Il verdetto per un raster dai primi byte, o None se non è nessuno di quelli.
fn raster(d: &[u8]) -> Option<EsitoImmagine> {
    if inizia(d, &FIRMA_PNG, 0) {
        return Some(verdetto(PNG, misura_png(d, 0)));
    }
    if inizia(d, &[0xff, 0xd8, 0xff], 0) {
        return Some(verdetto(JPG, misura_jpeg(d)));
    }
    if inizia(d, b"GIF8", 0) && (b(d, 4) == 0x37 || b(d, 4) == 0x39) && b(d, 5) == 0x61 {
        return Some(esito_gif(d));
    }
    if inizia(d, b"RIFF", 0) && inizia(d, b"WEBP", 8) {
        return Some(verdetto(WEBP, misura_webp(d)));
    }
    if inizia(d, &[0x00, 0x00, 0x01, 0x00], 0) {
        let n = (b(d, 4) | (b(d, 5) << 8)) as usize;
        if (1..=256).contains(&n) && d.len() >= 6 + 16 * n {
            return Some(esito_ico(d, n));
        }
    }
    None
}

// ---- SVG: una lettura sola, da sinistra a destra ------------------------------------------------------------------------------------
// Il testo si divide come lo divide un analizzatore XML: la dichiarazione e il DOCTYPE, poi elementi, attributi, commenti e CDATA. Un file
// che non si divide il browser non lo mostra e qui non si ammette. Di quello che resta si guarda ogni nome di elemento, ogni attributo e il
// testo degli <style>, e quello che il browser vede (riferimenti sciolti, commenti e CDATA tolti) è quello che si controlla.

/// Gli spazi di `\s` di JavaScript.
fn spazio_js(c: char) -> bool {
    crate::py::spazio_js(c)
}

/// Un carattere che può stare in un nome: non uno spazio, non uno dei segni ASCII che chiudono un nome.
fn di_nome(c: char) -> bool {
    if spazio_js(c) {
        return false;
    }
    let n = c as u32;
    !(n <= 0x2c || n == 0x2f || (0x3b..=0x40).contains(&n) || (0x5b..=0x5e).contains(&n) || n == 0x60 || (0x7b..=0x7f).contains(&n))
}

/// La fine del nome che comincia a `i`, o None se lì non comincia un nome.
fn nome_a(t: &str, i: usize) -> Option<usize> {
    let fine = t.get(i..)?.char_indices().find(|(_, c)| !di_nome(*c)).map_or(t.len(), |(j, _)| i + j);
    (fine > i).then_some(fine)
}

fn byte(t: &str, i: usize) -> Option<u8> {
    t.as_bytes().get(i).copied()
}

fn spazio_xml(c: Option<u8>) -> bool {
    matches!(c, Some(b' ' | b'\t' | b'\n' | b'\r'))
}

fn salta_spazi(t: &str, mut i: usize) -> usize {
    while spazio_xml(byte(t, i)) {
        i += 1;
    }
    i
}

fn trova(t: &str, cosa: &str, da: usize) -> Option<usize> {
    t.get(da..).and_then(|r| r.find(cosa)).map(|j| da + j)
}

static DICHIARAZIONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^<\?xml[ \t\r\n]+version[ \t\r\n]*=[ \t\r\n]*(?:"1\.[0-9]+"|'1\.[0-9]+')(?:[ \t\r\n]+encoding[ \t\r\n]*=[ \t\r\n]*(?:"([A-Za-z][A-Za-z0-9._-]*)"|'([A-Za-z][A-Za-z0-9._-]*)'))?(?:[ \t\r\n]+standalone[ \t\r\n]*=[ \t\r\n]*(?:"(?:yes|no)"|'(?:yes|no)'))?[ \t\r\n]*\?>"#).unwrap()
});
static DOCTYPE_W3C: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^<!DOCTYPE[ \t\r\n]+svg[ \t\r\n]+PUBLIC[ \t\r\n]+(?:"[^"]*"|'[^']*')[ \t\r\n]+(?:"https?://www\.w3\.org/[^"]*"|'https?://www\.w3\.org/[^']*')[ \t\r\n]*>$"#).unwrap()
});

/// Quanti elementi uno dentro l'altro: un logo ne ha poche decine.
const ANNIDAMENTO_MAX: usize = 256;
const ELEMENTI_VIETATI: [&str; 13] = ["script", "foreignobject", "iframe", "embed", "object", "applet", "audio", "video", "link", "meta", "base", "handler", "listener"];

fn invisibile(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{20}' | '\u{7f}'..='\u{9f}' | '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202f}' | '\u{2060}' | '\u{feff}')
}

fn carattere(n: Option<u64>) -> char {
    n.filter(|x| *x <= 0x10ffff).and_then(|x| char::from_u32(x as u32)).unwrap_or('\u{fffd}')
}

fn cifre_radice(s: &str, radice: u32) -> Option<u64> {
    s.chars().try_fold(0u64, |n, c| n.checked_mul(u64::from(radice))?.checked_add(u64::from(c.to_digit(radice)?)))
}

/// I riferimenti (&#106;, &#x6a;, &amp;...) sciolti: l'analizzatore XML li risolve, in una volta sola, prima che il valore arrivi a chi lo usa.
fn sciogli_riferimenti(valore: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i-u)&(?:#x([0-9a-f]+)|#([0-9]+)|(amp|lt|gt|quot|apos));?").unwrap());
    RE.replace_all(valore, |c: &regex::Captures| {
        if let Some(nome) = c.get(3) {
            return match nome.as_str().to_ascii_lowercase().as_str() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                _ => "'",
            }
            .to_string();
        }
        match (c.get(1), c.get(2)) {
            (Some(h), _) => carattere(cifre_radice(h.as_str(), 16)).to_string(),
            (_, Some(d)) => carattere(cifre_radice(d.as_str(), 10)).to_string(),
            _ => String::new(),
        }
    })
    .into_owned()
}

/// Le sequenze di escape del CSS sciolte: "u\72 l(" si legge url( come lo legge il browser.
fn sciogli_escape(css: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\\(?:([0-9a-f]{1,6})(?:\r\n|[ \t\r\n\x0C])?|([^\r\n\x0C]))").unwrap());
    RE.replace_all(css, |c: &regex::Captures| match (c.get(1), c.get(2)) {
        (Some(h), _) => carattere(cifre_radice(h.as_str(), 16)).to_string(),
        (_, Some(x)) => x.as_str().to_string(),
        _ => String::new(),
    })
    .into_owned()
}

/// Il valore di un attributo come lo vede il browser: riferimenti sciolti, spazi, invisibili e virgolette tolti, minuscolo.
fn come_lo_vede(valore: &str) -> String {
    sciogli_riferimenti(valore).chars().filter(|c| !invisibile(*c) && *c != '"' && *c != '\'').collect::<String>().to_lowercase()
}

fn parola(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn inizia_ci(t: &str, i: usize, cosa: &str) -> bool {
    t.get(i..i + cosa.len()).is_some_and(|x| x.eq_ignore_ascii_case(cosa))
}

/// @import, expression(), behavior, -moz-binding, e le funzioni che aprono un'immagine (anche con il prefisso del browser).
fn css_vietato(c: &str) -> bool {
    let mut prima: Option<char> = None;
    for (i, ch) in c.char_indices() {
        if inizia_ci(c, i, "@import") || inizia_ci(c, i, "-moz-binding") {
            return true;
        }
        for (parola_chiave, segno) in [("expression", '('), ("behavior", ':')] {
            if inizia_ci(c, i, parola_chiave) {
                let resto = &c[i + parola_chiave.len()..];
                if resto.trim_start_matches(spazio_js).starts_with(segno) {
                    return true;
                }
            }
        }
        if !parola(prima) && ["image-set(", "image(", "src(", "cross-fade("].iter().any(|f| inizia_ci(c, i, f)) {
            return true;
        }
        prima = Some(ch);
    }
    false
}

/// Ogni url( del testo, con quello che c'è dentro fino a ) o a una virgoletta, tolti gli spazi e la virgoletta d'apertura.
fn url_css(c: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(j) = (i..c.len()).find(|&j| c.is_char_boundary(j) && inizia_ci(c, j, "url(")) {
        let mut k = j + 4;
        k += c[k..].len() - c[k..].trim_start_matches(spazio_js).len();
        if matches!(byte(c, k), Some(b'"' | b'\'')) {
            k += 1;
        }
        k += c[k..].len() - c[k..].trim_start_matches(spazio_js).len();
        let fine = c[k..].find([')', '"', '\'']).map_or(c.len(), |x| k + x);
        out.push(&c[k..fine]);
        i = fine.max(j + 1);
    }
    out
}

/// Un testo (con i riferimenti già sciolti) che non carica niente da fuori: niente @import o simili, e ogni url() solo verso un #frammento.
fn senza_caricamenti(testo: &str) -> bool {
    let c = sciogli_escape(testo);
    if css_vietato(&c) {
        return false;
    }
    url_css(&c).iter().all(|u| crate::py::js_trim(u).starts_with('#'))
}

/// Un CSS (un attributo style o il testo di uno <style>): come sopra, e senza escape, che nasconderebbero una parola a chi la cerca.
fn css_pulito(css: &str) -> bool {
    let c = sciogli_riferimenti(css);
    !c.contains('\\') && senza_caricamenti(&c)
}

fn locale(nome: &str) -> String {
    nome[nome.rfind(':').map_or(0, |i| i + 1)..].to_lowercase()
}

fn protocollo_attivo(visto: &str) -> bool {
    ["javascript:", "vbscript:", "livescript:", "mocha:", "data:"].iter().any(|p| visto.contains(p))
}

/// Un attributo che si può lasciare: non esegue, non porta fuori e non carica da fuori.
fn attributo_ammesso(nome: &str, valore: &str) -> bool {
    let n = nome.to_lowercase();
    let l = locale(&n);
    // un gestore di eventi, o un xml:base che sposterebbe anche i riferimenti #interni su un altro documento
    if l.starts_with("on") || n == "xml:base" {
        return false;
    }
    let visto = come_lo_vede(valore);
    if l == "href" && !visto.starts_with('#') {
        return false;
    }
    if protocollo_attivo(&visto) {
        return false;
    }
    // l'animazione che scrive un href o un gestore (attributeName="&#111;nclick" vale onclick)
    if l == "attributename" {
        let bersaglio = locale(&visto);
        if bersaglio == "href" || bersaglio.starts_with("on") {
            return false;
        }
    }
    if l == "style" {
        css_pulito(valore)
    } else {
        senza_caricamenti(&sciogli_riferimenti(valore))
    }
}

/// Prima della radice: la dichiarazione <?xml ...?> (solo all'inizio, e solo UTF-8), commenti, un DOCTYPE. Dice dove comincia <svg e se
/// il prologo ha qualcosa che non si ammette; None se la radice non c'è (non è uno SVG).
fn prologo(t: &str) -> Option<(usize, bool)> {
    let mut i = 0;
    let mut vietato = false;
    let mut doctype = false;
    if t.starts_with("<?xml") && spazio_xml(byte(t, 5)) {
        let f = t.find("?>")?;
        match DICHIARAZIONE.captures(t) {
            Some(d) => {
                let codifica = d.get(1).or_else(|| d.get(2)).map_or("utf-8", |m| m.as_str());
                let c = codifica.to_ascii_lowercase();
                if c != "utf-8" && c != "utf8" {
                    vietato = true;
                }
            }
            None => vietato = true,
        }
        i = f + 2;
    }
    loop {
        i = salta_spazi(t, i);
        if t[i..].starts_with("<!--") {
            i = trova(t, "-->", i + 4)? + 3;
        } else if t[i..].starts_with("<!DOCTYPE") {
            if doctype {
                vietato = true;
            }
            doctype = true;
            let m = (i + 9..t.len()).find(|&j| matches!(byte(t, j), Some(b'[' | b'>')))?;
            if byte(t, m) == Some(b'>') {
                if !DOCTYPE_W3C.is_match(&t[i..m + 1]) {
                    vietato = true;
                }
                i = m + 1;
            } else {
                // un sottoinsieme interno non si ammette; si va oltre per sapere se dopo c'è uno SVG (il testo dell'errore cambia)
                vietato = true;
                let mut j = m;
                let fine = loop {
                    let k = trova(t, "]", j)?;
                    let dopo = salta_spazi(t, k + 1);
                    if byte(t, dopo) == Some(b'>') {
                        break dopo + 1;
                    }
                    j = k + 1;
                };
                i = fine;
            }
        } else if t[i..].starts_with("<?") {
            // <?xml-stylesheet ...?> e ogni altra istruzione, o una seconda dichiarazione
            vietato = true;
            i = trova(t, "?>", i + 2)? + 2;
        } else {
            break;
        }
    }
    let radice = t[i..].starts_with("<svg") && matches!(byte(t, i + 4), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>'));
    radice.then_some((i, vietato))
}

/// Dalla radice alla fine: elementi, attributi, commenti, CDATA. True solo se tutto si divide, la radice si chiude e niente è vietato.
fn corpo_ammesso(t: &str, da: usize) -> bool {
    let n = t.len();
    let mut aperti: Vec<&str> = Vec::new();
    let mut stile = 0i32;
    let mut css = String::new();
    let mut chiusa = false;
    let mut i = da;
    while i < n {
        if byte(t, i) != Some(b'<') {
            if chiusa {
                // dopo la radice solo spazi (e commenti, sotto)
                i = salta_spazi(t, i);
                if i < n && byte(t, i) != Some(b'<') {
                    return false;
                }
                continue;
            }
            let j = trova(t, "<", i).unwrap_or(n);
            if stile != 0 {
                css.push_str(&t[i..j]);
            }
            i = j;
            continue;
        }
        if t[i..].starts_with("<!--") {
            let Some(f) = trova(t, "-->", i + 4) else { return false };
            i = f + 3;
            continue;
        }
        if t[i..].starts_with("<![CDATA[") {
            let f = if aperti.is_empty() { None } else { trova(t, "]]>", i + 9) };
            let Some(f) = f else { return false };
            let dentro = &t[i + 9..f];
            // fuori da uno <style> un CDATA con del markup dentro non serve a un logo: un dubbio vale un no
            if stile != 0 {
                css.push_str(dentro);
            } else if dentro.contains('<') {
                return false;
            }
            i = f + 3;
            continue;
        }
        if byte(t, i + 1) == Some(b'/') {
            // </nome>
            let Some(fine) = nome_a(t, i + 2) else { return false };
            let nome = &t[i + 2..fine];
            let k = salta_spazi(t, fine);
            if byte(t, k) != Some(b'>') || aperti.pop() != Some(nome) {
                return false;
            }
            if locale(nome) == "style" {
                stile -= 1;
            }
            i = k + 1;
            if aperti.is_empty() {
                chiusa = true;
            }
            continue;
        }
        if chiusa || aperti.len() >= ANNIDAMENTO_MAX {
            return false;
        }
        let Some(fine) = nome_a(t, i + 1) else { return false };
        let nome = &t[i + 1..fine];
        let l = locale(nome);
        if ELEMENTI_VIETATI.contains(&l.as_str()) {
            return false;
        }
        let mut k = fine;
        loop {
            let prima = k;
            k = salta_spazi(t, k);
            match byte(t, k) {
                Some(b'>') => {
                    aperti.push(nome);
                    if l == "style" {
                        stile += 1;
                    }
                    i = k + 1;
                    break;
                }
                Some(b'/') => {
                    if byte(t, k + 1) != Some(b'>') {
                        return false;
                    }
                    i = k + 2;
                    if aperti.is_empty() {
                        chiusa = true;
                    }
                    break;
                }
                _ => {}
            }
            // prima di ogni attributo ci vuole almeno uno spazio
            if k == prima {
                return false;
            }
            let Some(fine_a) = nome_a(t, k) else { return false };
            let attributo = &t[k..fine_a];
            k = salta_spazi(t, fine_a);
            if byte(t, k) != Some(b'=') {
                return false;
            }
            k = salta_spazi(t, k + 1);
            let virgolette = match byte(t, k) {
                Some(b'"') => "\"",
                Some(b'\'') => "'",
                _ => return false,
            };
            let Some(f) = trova(t, virgolette, k + 1) else { return false };
            let valore = &t[k + 1..f];
            if valore.contains('<') || !attributo_ammesso(attributo, valore) {
                return false;
            }
            k = f + 1;
        }
    }
    chiusa && (css.is_empty() || css_pulito(&css))
}

#[derive(PartialEq)]
enum Analisi {
    Ok,
    No,
    Vietato,
}

fn analizza_svg(testo: &str) -> Analisi {
    let Some((radice, vietato)) = prologo(testo) else { return Analisi::No };
    if vietato || testo.contains('\u{0}') {
        return Analisi::Vietato;
    }
    if corpo_ammesso(testo, radice) {
        Analisi::Ok
    } else {
        Analisi::Vietato
    }
}

/// Un SVG che si può mostrare: niente che esegua codice o carichi qualcosa da fuori. Un dubbio vale un no.
pub fn svg_ammesso(testo: &str) -> bool {
    analizza_svg(testo) == Analisi::Ok
}

/// Riconosce il file dai primi byte e lo accetta o no.
pub fn controlla_immagine(dati: &[u8]) -> EsitoImmagine {
    if let Some(r) = raster(dati) {
        return r;
    }
    let Ok(testo) = std::str::from_utf8(dati) else { return EsitoImmagine::NonImmagine };
    // TextDecoder toglie il BOM, e la pagina un secondo
    let testo = testo.strip_prefix('\u{feff}').unwrap_or(testo);
    let testo = testo.strip_prefix('\u{feff}').unwrap_or(testo);
    match analizza_svg(testo) {
        Analisi::Ok => EsitoImmagine::Tipo(SVG),
        Analisi::No => EsitoImmagine::NonImmagine,
        Analisi::Vietato => EsitoImmagine::SvgNonAmmesso,
    }
}

#[cfg(test)]
mod prove {
    use super::*;

    #[test]
    fn svg_ammessi_e_no() {
        assert!(svg_ammesso(r#"<?xml version="1.0" encoding="UTF-8"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><style>.a{fill:url(#g)}</style><rect class="a" width="1" height="1"/></svg>"#));
        for no in [
            r#"<svg><script>x</script></svg>"#,
            r#"<svg onload="x"/>"#,
            r#"<svg><a href="https://x.example">x</a></svg>"#,
            r#"<svg><a href="&#106;avascript:x">x</a></svg>"#,
            r#"<svg style="background:u\72 l(https://x)"/>"#,
            r#"<svg><style>@import url(x.css)</style></svg>"#,
            r##"<svg><image href="#a" style="fill:image(x)"/></svg>"##,
            r#"<?xml version="1.0" encoding="latin1"?><svg/>"#,
            r#"<!DOCTYPE svg [<!ENTITY x "y">]><svg/>"#,
            r#"<svg><g></svg>"#,
            "<svg/>x",
        ] {
            assert!(!svg_ammesso(no), "{no}");
        }
        assert_eq!(controlla_immagine(b"<html/>"), EsitoImmagine::NonImmagine);
        assert_eq!(controlla_immagine(b"<svg><script/></svg>"), EsitoImmagine::SvgNonAmmesso);
    }

    #[test]
    fn un_gif_con_un_fotogramma_enorme_sotto_uno_schermo_piccolo() {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&[16, 0, 16, 0, 0, 0, 0]);
        g.extend_from_slice(&[0x2c, 0, 0, 0, 0, 0x68, 0x10, 0x68, 0x10, 0]);
        g.extend_from_slice(&[2, 0, 0x3b]);
        assert_eq!(controlla_immagine(&g), EsitoImmagine::TroppiPixel);
    }

    #[test]
    fn png_dalla_testata() {
        let mut p = FIRMA_PNG.to_vec();
        p.extend_from_slice(&[0, 0, 0, 13]);
        p.extend_from_slice(&IHDR);
        p.extend_from_slice(&[0, 0, 0, 64, 0, 0, 0, 64]);
        assert_eq!(controlla_immagine(&p), EsitoImmagine::Tipo(PNG));
        p[16..20].copy_from_slice(&[0, 0, 0x20, 0]);
        assert_eq!(controlla_immagine(&p), EsitoImmagine::TroppiPixel);
    }
}
