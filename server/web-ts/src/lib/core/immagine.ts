// Che cosa è un file scaricato o caricato come logo: lo dicono i primi byte, mai l'estensione né il Content-Type dichiarato dal sito.
// Si ammettono PNG, JPEG, GIF, WebP, ICO e SVG. Di un raster si legge la grandezza dalla testata e si rifiuta oltre LATO_MAX pixel per
// lato o PIXEL_MAX in tutto: un file da pochi KB può chiedere al browser di chi lo guarda gigabyte di pixel. Un SVG è testo che il
// browser esegue se lo si apre da solo: si rifiuta se ha qualcosa di attivo o che porta fuori (script, foreignObject, gestori di eventi,
// javascript: e data:, href che non siano #frammenti, CSS che carica da fuori). Chi serve il file aggiunge comunque la sua difesa
// (gateway.ts: Content-Security-Policy con sandbox e nosniff).
// Il file lo sceglie un sito che non si conosce e il controllo gira nello stesso thread del gateway: ogni lettura qui è lineare nella
// lunghezza del file (indexOf, espressioni regolari che guardano un punto solo e mai un quantificatore dentro un altro), così 512 KB
// ostili non fermano il gateway. Una prova con tempo massimo per ogni famiglia di input patologico sta in test/immagine-ostile.test.ts.

export type TipoImmagine = { ext: "png" | "jpg" | "gif" | "webp" | "ico" | "svg"; mime: string };

const TIPI: Record<TipoImmagine["ext"], TipoImmagine> = {
  png: { ext: "png", mime: "image/png" },
  jpg: { ext: "jpg", mime: "image/jpeg" },
  gif: { ext: "gif", mime: "image/gif" },
  webp: { ext: "webp", mime: "image/webp" },
  ico: { ext: "ico", mime: "image/vnd.microsoft.icon" },
  svg: { ext: "svg", mime: "image/svg+xml" },
};

export type EsitoImmagine = { tipo: TipoImmagine } | { errore: "non_immagine" | "svg_non_ammesso" | "troppi_pixel" };

/** Il lato più lungo che si accetta di un raster, e i pixel in tutto (16 megapixel). */
export const LATO_MAX = 4096;
export const PIXEL_MAX = 16_000_000;

// ---- Raster: il tipo dai primi byte, la grandezza dalla testata --------------------------------------------------------------------
const inizia = (dati: Uint8Array, byte: number[], da = 0) => dati.length >= da + byte.length && byte.every((b, i) => dati[da + i] === b);
const u16le = (d: Uint8Array, i: number) => d[i] | (d[i + 1] << 8);
const u16be = (d: Uint8Array, i: number) => (d[i] << 8) | d[i + 1];
const u24le = (d: Uint8Array, i: number) => d[i] | (d[i + 1] << 8) | (d[i + 2] << 16);
const u32le = (d: Uint8Array, i: number) => (d[i] | (d[i + 1] << 8) | (d[i + 2] << 16) | (d[i + 3] << 24)) >>> 0;
const u32be = (d: Uint8Array, i: number) => ((d[i] << 24) | (d[i + 1] << 16) | (d[i + 2] << 8) | d[i + 3]) >>> 0;
const i32le = (d: Uint8Array, i: number) => d[i] | (d[i + 1] << 8) | (d[i + 2] << 16) | (d[i + 3] << 24);

/** Larghezza e altezza dichiarate dalla testata, o null se la testata non si legge (troncata, strana). */
type Misura = readonly [number, number] | null;
const troppoGrande = (l: number, a: number) => l > LATO_MAX || a > LATO_MAX || l * a > PIXEL_MAX;

/** Una testata che non si legge o dice 0 non è un'immagine; una troppo grande si rifiuta con il suo testo. */
function verdetto(tipo: TipoImmagine, m: Misura): EsitoImmagine {
  if (!m || m[0] < 1 || m[1] < 1) return { errore: "non_immagine" };
  return troppoGrande(m[0], m[1]) ? { errore: "troppi_pixel" } : { tipo };
}

const FIRMA_PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const IHDR = [0x49, 0x48, 0x44, 0x52];

/** PNG: la firma, la lunghezza del blocco, IHDR, poi larghezza e altezza (32 bit). `da` è dove comincia la firma. */
const misuraPng = (d: Uint8Array, da = 0): Misura => (d.length >= da + 24 && inizia(d, IHDR, da + 12) ? [u32be(d, da + 16), u32be(d, da + 20)] : null);

/** GIF: lo schermo logico, subito dopo i 6 byte della firma (16 bit l'una). */
const misuraGif = (d: Uint8Array): Misura => (d.length >= 10 ? [u16le(d, 6), u16le(d, 8)] : null);

/** WebP: il primo blocco dopo RIFF, lunghezza e WEBP è VP8X (tela, 24 bit), VP8 (con perdita, 14 bit) o VP8L (senza perdita, 14 bit). */
function misuraWebp(d: Uint8Array): Misura {
  if (inizia(d, [0x56, 0x50, 0x38, 0x58], 12)) return d.length >= 30 ? [u24le(d, 24) + 1, u24le(d, 27) + 1] : null;
  if (inizia(d, [0x56, 0x50, 0x38, 0x20], 12)) return d.length >= 30 && inizia(d, [0x9d, 0x01, 0x2a], 23) ? [u16le(d, 26) & 0x3fff, u16le(d, 28) & 0x3fff] : null;
  if (inizia(d, [0x56, 0x50, 0x38, 0x4c], 12) && d.length >= 25 && d[20] === 0x2f) {
    const b = u32le(d, 21);
    return [(b & 0x3fff) + 1, ((b >>> 14) & 0x3fff) + 1];
  }
  return null;
}

/** JPEG: si scorrono i segmenti (ognuno dice quanto è lungo) fino al primo SOF; le miniature EXIF stanno dentro un segmento e non si leggono. */
function misuraJpeg(d: Uint8Array): Misura {
  let i = 2;
  while (i + 4 <= d.length) {
    if (d[i] !== 0xff) return null;
    const m = d[i + 1];
    if (m === 0xff) {
      i++; // byte di riempimento prima di un marker
      continue;
    }
    if (m === 0x01 || (m >= 0xd0 && m <= 0xd8)) {
      i += 2; // marker senza lunghezza (TEM, RSTn, SOI)
      continue;
    }
    // 00 non sta tra i segmenti; EOI e SOS senza un SOF prima: la testata non c'è
    if (m === 0x00 || m === 0xd9 || m === 0xda) return null;
    const lunghezza = u16be(d, i + 2);
    if (lunghezza < 2) return null;
    // SOF0..SOF15, tranne DHT (c4), JPG (c8) e DAC (cc): marker, lunghezza, precisione, altezza, larghezza
    if (m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc) return i + 9 <= d.length ? [u16be(d, i + 7), u16be(d, i + 5)] : null;
    i += 2 + lunghezza;
  }
  return null;
}

const TESTATE_DIB = new Set([40, 52, 56, 64, 108, 124]);

/**
 * ICO: la tabella dice la grandezza di ogni immagine (un byte, 0 vale 256: mai sopra il tetto), ma l'immagine dentro è un PNG o un DIB con
 * la sua testata, e il browser disegna quella. Si guarda ogni voce; una che non si legge (fuori dal file, formato ignoto) il browser non
 * la decodifica e non costa niente.
 */
function esitoIco(d: Uint8Array, n: number): EsitoImmagine {
  for (let k = 0; k < n; k++) {
    const da = u32le(d, 6 + 16 * k + 12);
    let m: Misura = null;
    if (inizia(d, FIRMA_PNG, da)) m = misuraPng(d, da);
    else if (da + 16 <= d.length) {
      // DIB: la prima parola è la lunghezza della sua testata, una di quelle che esistono (12: OS/2, grandezza a 16 bit; dalle altre
      // grandezza a 32 bit, e l'altezza conta anche la maschera, il doppio). Qualunque altra cosa il browser non la decodifica.
      const dimensioneTestata = u32le(d, da);
      if (dimensioneTestata === 12) m = [u16le(d, da + 4), u16le(d, da + 6)];
      else if (TESTATE_DIB.has(dimensioneTestata)) m = [Math.abs(i32le(d, da + 4)), Math.abs(i32le(d, da + 8)) / 2];
    }
    if (m && troppoGrande(m[0], m[1])) return { errore: "troppi_pixel" };
  }
  return { tipo: TIPI.ico };
}

/** Il verdetto per un raster dai primi byte, o null se non è nessuno di quelli. Dello SVG si occupa `controllaImmagine`. */
function raster(d: Uint8Array): EsitoImmagine | null {
  if (inizia(d, FIRMA_PNG)) return verdetto(TIPI.png, misuraPng(d));
  if (inizia(d, [0xff, 0xd8, 0xff])) return verdetto(TIPI.jpg, misuraJpeg(d));
  // GIF87a e GIF89a
  if (inizia(d, [0x47, 0x49, 0x46, 0x38]) && (d[4] === 0x37 || d[4] === 0x39) && d[5] === 0x61) return verdetto(TIPI.gif, misuraGif(d));
  // RIFF....WEBP
  if (inizia(d, [0x52, 0x49, 0x46, 0x46]) && inizia(d, [0x57, 0x45, 0x42, 0x50], 8)) return verdetto(TIPI.webp, misuraWebp(d));
  // ICO: 00 00 01 00, poi il numero di immagini e la loro tabella (6 byte di testata, 16 per voce)
  if (inizia(d, [0x00, 0x00, 0x01, 0x00])) {
    const n = d[4] | (d[5] << 8);
    if (n >= 1 && n <= 256 && d.length >= 6 + 16 * n) return esitoIco(d, n);
  }
  return null;
}

// ---- SVG: una lettura sola, da sinistra a destra ------------------------------------------------------------------------------------
// Il testo si divide come lo divide un analizzatore XML: la dichiarazione e il DOCTYPE, poi elementi, attributi, commenti e CDATA. Un
// file che non si divide (virgolette mancanti, un tag che non si chiude, <? o <! fuori dal prologo, qualcosa dopo la radice) il browser
// non lo mostra e qui non si ammette. Di quello che resta si guarda ogni nome di elemento, ogni attributo e il testo degli <style>, e
// quello che il browser vede (riferimenti sciolti, commenti e CDATA tolti) è quello che si controlla: niente si nasconde spezzandolo.

const NOME = /[^\s\u0000-\u002c\u002f\u003b-\u0040\u005b-\u005e\u0060\u007b-\u007f]+/y;
const RADICE = /<svg[ \t\r\n/>]/y;
const FINE_DOCTYPE = /[[>]/g;
const FINE_SOTTOINSIEME = /\][ \t\r\n]*>/g;
// <?xml version="1.0" encoding="UTF-8" standalone="no"?>: l'analizzatore XML la vuole esatta, e qui si ammette UTF-8 e nient'altro
const DICHIARAZIONE =
  /^<\?xml[ \t\r\n]+version[ \t\r\n]*=[ \t\r\n]*(?:"1\.[0-9]+"|'1\.[0-9]+')(?:[ \t\r\n]+encoding[ \t\r\n]*=[ \t\r\n]*(?:"([A-Za-z][A-Za-z0-9._-]*)"|'([A-Za-z][A-Za-z0-9._-]*)'))?(?:[ \t\r\n]+standalone[ \t\r\n]*=[ \t\r\n]*(?:"(?:yes|no)"|'(?:yes|no)'))?[ \t\r\n]*\?>/;
// L'unico DOCTYPE che si ammette: quello pubblico di W3C che scrivono i programmi di disegno. Con SYSTEM, con un sottoinsieme interno
// ([...], dove si dichiarano entità) o di un altro sito non si ammette.
const DOCTYPE_W3C = /^<!DOCTYPE[ \t\r\n]+svg[ \t\r\n]+PUBLIC[ \t\r\n]+(?:"[^"]*"|'[^']*')[ \t\r\n]+(?:"https?:\/\/www\.w3\.org\/[^"]*"|'https?:\/\/www\.w3\.org\/[^']*')[ \t\r\n]*>$/;

/** Quanti elementi uno dentro l'altro: un logo ne ha poche decine, e un albero di migliaia di livelli affatica il browser di chi lo guarda. */
const ANNIDAMENTO_MAX = 256;
const ELEMENTI_VIETATI = new Set(["script", "foreignobject", "iframe", "embed", "object", "applet", "audio", "video", "link", "meta", "base", "handler", "listener"]);
const SPAZI_INVISIBILI = /[\u0000-\u0020\u007f-\u009f\u00ad\u200b-\u200f\u2028-\u202f\u2060\ufeff]/g;
const PROTOCOLLO_ATTIVO = /(?:javascript|vbscript|livescript|mocha|data):/;
// quello che un CSS carica o esegue: @import, expression(), behavior, -moz-binding, e le funzioni che aprono un'immagine (anche con il prefisso del browser)
const CSS_VIETATO = /@import|expression\s*\(|behavior\s*:|-moz-binding|(?<!\w)(?:image-set|image|src|cross-fade)\(/i;
const URL_CSS = /url\(\s*["']?\s*([^)"']*)/gi;

const spazio = (c: number) => c === 0x20 || c === 0x09 || c === 0x0a || c === 0x0d;

function saltaSpazi(t: string, i: number): number {
  while (i < t.length && spazio(t.charCodeAt(i))) i++;
  return i;
}

const carattere = (n: number) => (Number.isInteger(n) && n >= 0 && n <= 0x10ffff ? String.fromCodePoint(n) : "\ufffd");
const NOMINATI: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };

/** I riferimenti (&#106;, &#x6a;, &amp;...) sciolti: l'analizzatore XML li risolve, in una volta sola, prima che il valore arrivi a chi lo usa. */
function sciogliRiferimenti(valore: string): string {
  return valore.replace(/&(?:#x([0-9a-f]+)|#(\d+)|(amp|lt|gt|quot|apos));?/gi, (_, h?: string, d?: string, nome?: string) =>
    nome ? NOMINATI[nome.toLowerCase()] : carattere(h ? parseInt(h, 16) : parseInt(d as string, 10)),
  );
}

/** Le sequenze di escape del CSS sciolte: "u\72 l(" si legge url( come lo legge il browser. */
const sciogliEscape = (css: string): string => css.replace(/\\(?:([0-9a-f]{1,6})(?:\r\n|[ \t\r\n\f])?|([^\r\n\f]))/gi, (_, h?: string, altro?: string) => (h ? carattere(parseInt(h, 16)) : (altro as string)));

/** Il valore di un attributo come lo vede il browser: riferimenti sciolti, spazi, invisibili e virgolette tolti, minuscolo. */
const comeLoVede = (valore: string): string => sciogliRiferimenti(valore).replace(SPAZI_INVISIBILI, "").replace(/["']/g, "").toLowerCase();

/** Un testo (con i riferimenti già sciolti) che non carica niente da fuori: niente @import o simili, e ogni url() solo verso un #frammento. */
function senzaCaricamenti(testo: string): boolean {
  const c = sciogliEscape(testo);
  if (CSS_VIETATO.test(c)) return false;
  for (const m of c.matchAll(URL_CSS)) if (!m[1].trim().startsWith("#")) return false;
  return true;
}

/** Un CSS (un attributo style o il testo di uno <style>): come sopra, e senza escape, che nasconderebbero una parola a chi la cerca. */
function cssPulito(css: string): boolean {
  const c = sciogliRiferimenti(css);
  return !c.includes("\\") && senzaCaricamenti(c);
}

const locale = (nome: string): string => nome.slice(nome.lastIndexOf(":") + 1).toLowerCase();

/** Un attributo che si può lasciare: non esegue, non porta fuori e non carica da fuori. */
function attributoAmmesso(nome: string, valore: string): boolean {
  const n = nome.toLowerCase();
  const l = locale(n);
  // un gestore di eventi, o un xml:base che sposterebbe anche i riferimenti #interni su un altro documento
  if (l.startsWith("on") || n === "xml:base") return false;
  const visto = comeLoVede(valore);
  if (l === "href" && !visto.startsWith("#")) return false;
  if (PROTOCOLLO_ATTIVO.test(visto)) return false;
  // l'animazione che scrive un href o un gestore (attributeName="&#111;nclick" vale onclick)
  if (l === "attributename") {
    const bersaglio = locale(visto);
    if (bersaglio === "href" || bersaglio.startsWith("on")) return false;
  }
  return l === "style" ? cssPulito(valore) : senzaCaricamenti(sciogliRiferimenti(valore));
}

/**
 * Prima della radice: la dichiarazione <?xml ...?> (solo all'inizio, e solo UTF-8), commenti, un DOCTYPE. Dice dove comincia <svg e se
 * il prologo ha qualcosa che non si ammette; null se la radice non c'è (non è uno SVG). Ogni passo consuma quello che legge.
 */
function prologo(t: string): { radice: number; vietato: boolean } | null {
  let i = 0;
  let vietato = false;
  let doctype = false;
  if (t.startsWith("<?xml") && spazio(t.charCodeAt(5))) {
    const f = t.indexOf("?>");
    if (f < 0) return null;
    const d = DICHIARAZIONE.exec(t);
    if (!d || !/^utf-?8$/i.test(d[1] ?? d[2] ?? "utf-8")) vietato = true;
    i = f + 2;
  }
  for (;;) {
    i = saltaSpazi(t, i);
    if (t.startsWith("<!--", i)) {
      const f = t.indexOf("-->", i + 4);
      if (f < 0) return null;
      i = f + 3;
    } else if (t.startsWith("<!DOCTYPE", i)) {
      if (doctype) vietato = true;
      doctype = true;
      FINE_DOCTYPE.lastIndex = i + 9;
      const m = FINE_DOCTYPE.exec(t);
      if (!m) return null;
      if (m[0] === ">") {
        if (!DOCTYPE_W3C.test(t.slice(i, m.index + 1))) vietato = true;
        i = m.index + 1;
      } else {
        // un sottoinsieme interno non si ammette; si va oltre per sapere se dopo c'è uno SVG (il testo dell'errore cambia)
        vietato = true;
        FINE_SOTTOINSIEME.lastIndex = m.index;
        const e = FINE_SOTTOINSIEME.exec(t);
        if (!e) return null;
        i = e.index + e[0].length;
      }
    } else if (t.startsWith("<?", i)) {
      // <?xml-stylesheet ...?> e ogni altra istruzione, o una seconda dichiarazione
      vietato = true;
      const f = t.indexOf("?>", i + 2);
      if (f < 0) return null;
      i = f + 2;
    } else break;
  }
  RADICE.lastIndex = i;
  return RADICE.test(t) ? { radice: i, vietato } : null;
}

/**
 * Dalla radice alla fine: elementi, attributi, commenti, CDATA. Il testo degli <style> (con CDATA e senza commenti, anche se spezzato da
 * un elemento in mezzo) si raccoglie e si controlla alla fine. True solo se tutto si divide, la radice si chiude e niente è vietato.
 */
function corpoAmmesso(t: string, da: number): boolean {
  const n = t.length;
  const aperti: string[] = [];
  let stile = 0;
  let css = "";
  let chiusa = false;
  let i = da;
  while (i < n) {
    if (t.charCodeAt(i) !== 0x3c) {
      if (chiusa) {
        // dopo la radice solo spazi (e commenti, sotto)
        i = saltaSpazi(t, i);
        if (i < n && t.charCodeAt(i) !== 0x3c) return false;
        continue;
      }
      let j = t.indexOf("<", i);
      if (j < 0) j = n;
      if (stile) css += t.slice(i, j);
      i = j;
      continue;
    }
    if (t.startsWith("<!--", i)) {
      const f = t.indexOf("-->", i + 4);
      if (f < 0) return false;
      i = f + 3;
      continue;
    }
    if (t.startsWith("<![CDATA[", i)) {
      const f = aperti.length ? t.indexOf("]]>", i + 9) : -1;
      if (f < 0) return false;
      const dentro = t.slice(i + 9, f);
      // fuori da uno <style> un CDATA con del markup dentro (un <script> scritto come testo) non serve a un logo: un dubbio vale un no
      if (stile) css += dentro;
      else if (dentro.includes("<")) return false;
      i = f + 3;
      continue;
    }
    if (t.charCodeAt(i + 1) === 0x2f) {
      // </nome>
      NOME.lastIndex = i + 2;
      const m = NOME.exec(t);
      if (!m) return false;
      const k = saltaSpazi(t, NOME.lastIndex);
      if (t.charCodeAt(k) !== 0x3e || aperti.pop() !== m[0]) return false;
      if (locale(m[0]) === "style") stile--;
      i = k + 1;
      if (!aperti.length) chiusa = true;
      continue;
    }
    if (chiusa || aperti.length >= ANNIDAMENTO_MAX) return false;
    NOME.lastIndex = i + 1;
    const m = NOME.exec(t);
    if (!m) return false;
    const nome = m[0];
    const l = locale(nome);
    if (ELEMENTI_VIETATI.has(l)) return false;
    let k = NOME.lastIndex;
    for (;;) {
      const prima = k;
      k = saltaSpazi(t, k);
      const c = t.charCodeAt(k);
      if (c === 0x3e) {
        aperti.push(nome);
        if (l === "style") stile++;
        i = k + 1;
        break;
      }
      if (c === 0x2f) {
        if (t.charCodeAt(k + 1) !== 0x3e) return false;
        i = k + 2;
        if (!aperti.length) chiusa = true;
        break;
      }
      // prima di ogni attributo ci vuole almeno uno spazio
      if (k === prima) return false;
      NOME.lastIndex = k;
      const a = NOME.exec(t);
      if (!a) return false;
      k = saltaSpazi(t, NOME.lastIndex);
      if (t.charCodeAt(k) !== 0x3d) return false;
      k = saltaSpazi(t, k + 1);
      const virgolette = t.charCodeAt(k);
      if (virgolette !== 0x22 && virgolette !== 0x27) return false;
      const f = t.indexOf(virgolette === 0x22 ? '"' : "'", k + 1);
      if (f < 0) return false;
      const valore = t.slice(k + 1, f);
      if (valore.includes("<") || !attributoAmmesso(a[0], valore)) return false;
      k = f + 1;
    }
  }
  return chiusa && (!css || cssPulito(css));
}

type Analisi = "ok" | "no" | "vietato";

/** "no": non è uno SVG; "vietato": è uno SVG che non si ammette (o non si divide); "ok". */
function analizzaSvg(testo: string): Analisi {
  const p = prologo(testo);
  if (!p) return "no";
  if (p.vietato || testo.includes("\u0000")) return "vietato";
  return corpoAmmesso(testo, p.radice) ? "ok" : "vietato";
}

/** Un SVG che si può mostrare: niente che esegua codice o carichi qualcosa da fuori. Un dubbio vale un no. */
export const svgAmmesso = (testo: string): boolean => analizzaSvg(testo) === "ok";

/** Riconosce il file dai primi byte e lo accetta o no. */
export function controllaImmagine(dati: Uint8Array): EsitoImmagine {
  const r = raster(dati);
  if (r) return r;
  let testo: string;
  try {
    testo = new TextDecoder("utf-8", { fatal: true }).decode(dati).replace(/^\ufeff/, "");
  } catch {
    return { errore: "non_immagine" };
  }
  const esito = analizzaSvg(testo);
  if (esito === "ok") return { tipo: TIPI.svg };
  return { errore: esito === "no" ? "non_immagine" : "svg_non_ammesso" };
}
