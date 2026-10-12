// Che cosa è un file scaricato o caricato come logo: lo dicono i primi byte, mai l'estensione né il Content-Type dichiarato dal sito.
// Si ammettono PNG, JPEG, GIF, WebP, ICO e SVG. Un SVG è testo che il browser esegue se lo si apre da solo: si rifiuta se ha qualcosa di
// attivo o che porta fuori (script, foreignObject, gestori di eventi, javascript: e data:, href che non siano #frammenti, CSS che carica
// da fuori). Chi serve il file aggiunge comunque la sua difesa (gateway.ts: Content-Security-Policy con sandbox e nosniff).

export type TipoImmagine = { ext: "png" | "jpg" | "gif" | "webp" | "ico" | "svg"; mime: string };

const TIPI: Record<TipoImmagine["ext"], TipoImmagine> = {
  png: { ext: "png", mime: "image/png" },
  jpg: { ext: "jpg", mime: "image/jpeg" },
  gif: { ext: "gif", mime: "image/gif" },
  webp: { ext: "webp", mime: "image/webp" },
  ico: { ext: "ico", mime: "image/vnd.microsoft.icon" },
  svg: { ext: "svg", mime: "image/svg+xml" },
};

export type EsitoImmagine = { tipo: TipoImmagine } | { errore: "non_immagine" | "svg_non_ammesso" };

const inizia = (dati: Uint8Array, byte: number[], da = 0) => dati.length >= da + byte.length && byte.every((b, i) => dati[da + i] === b);

/** Il tipo raster dai primi byte, o null. Dello SVG si occupa `controllaImmagine`. */
function raster(dati: Uint8Array): TipoImmagine | null {
  // PNG: firma e blocco IHDR
  if (inizia(dati, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) && inizia(dati, [0x49, 0x48, 0x44, 0x52], 12)) return TIPI.png;
  if (inizia(dati, [0xff, 0xd8, 0xff])) return TIPI.jpg;
  // GIF87a e GIF89a
  if (inizia(dati, [0x47, 0x49, 0x46, 0x38]) && (dati[4] === 0x37 || dati[4] === 0x39) && dati[5] === 0x61) return TIPI.gif;
  // RIFF....WEBP
  if (inizia(dati, [0x52, 0x49, 0x46, 0x46]) && inizia(dati, [0x57, 0x45, 0x42, 0x50], 8)) return TIPI.webp;
  // ICO: 00 00 01 00, poi il numero di immagini e la loro tabella (6 byte di testata, 16 per voce)
  if (inizia(dati, [0x00, 0x00, 0x01, 0x00])) {
    const n = dati[4] | (dati[5] << 8);
    if (n >= 1 && n <= 256 && dati.length >= 6 + 16 * n) return TIPI.ico;
  }
  return null;
}

const SPAZI_INVISIBILI = /[\u0000-\u0020\u007f-\u009f\u00ad\u200b-\u200f\u2028-\u202f\u2060\ufeff]/g;

/** I riferimenti numerici (&#106; e &#x6a;) sciolti: l'analizzatore XML li risolve prima che il valore arrivi a chi lo usa. */
function sciogliRiferimenti(valore: string): string {
  const carattere = (n: number) => (Number.isInteger(n) && n >= 0 && n <= 0x10ffff ? String.fromCodePoint(n) : "\ufffd");
  return valore.replace(/&#x([0-9a-f]+);?/gi, (_, h: string) => carattere(parseInt(h, 16))).replace(/&#(\d+);?/g, (_, d: string) => carattere(parseInt(d, 10)));
}

/** Il valore di un attributo come lo vede il browser: riferimenti sciolti, spazi, invisibili e virgolette tolti, minuscolo. */
const comeLoVede = (valore: string): string => sciogliRiferimenti(valore).replace(SPAZI_INVISIBILI, "").replace(/["']/g, "").toLowerCase();

const ELEMENTI_VIETATI = /<\s*\/?\s*(?:[^\s<>/:]+:)?(?:script|foreignobject|iframe|embed|object|applet|audio|video|link|meta|base|handler|listener)\b/i;
const ATTRIBUTO = /([^\s"'<>/=]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/g;
const PROTOCOLLO_ATTIVO = /(?:javascript|vbscript|livescript|mocha|data):/;

/** Un CSS che non carica niente da fuori e non nasconde niente dietro una sequenza di escape. */
function cssPulito(css: string): boolean {
  const c = sciogliRiferimenti(css).replace(/\/\*[\s\S]*?\*\//g, "");
  if (/\\/.test(c) || /@import|expression\s*\(|behavior\s*:|-moz-binding|image-set\s*\(|\bimage\s*\(/i.test(c)) return false;
  for (const m of c.matchAll(/url\(\s*["']?\s*([^)"']*)/gi)) if (!m[1].trim().startsWith("#")) return false;
  return true;
}

/** Un SVG che si può mostrare: niente che esegua codice o carichi qualcosa da fuori. Un dubbio vale un no. */
export function svgAmmesso(testo: string): boolean {
  if (/\u0000/.test(testo)) return false;
  if (ELEMENTI_VIETATI.test(testo)) return false;
  // entità e DOCTYPE con sottoinsieme interno nascondono il contenuto a chi lo cerca come testo; le istruzioni <?...?> oltre a <?xml
  if (/<!(?:ENTITY|ATTLIST|NOTATION|ELEMENT)/i.test(testo) || /<!DOCTYPE[^>]*\[/i.test(testo) || /<\?(?!xml[\s?])/i.test(testo)) return false;
  const dichiarazione = /^\s*<\?xml[^>]*\?>/.exec(testo)?.[0] ?? "";
  const codifica = /encoding\s*=\s*["']([^"']+)["']/i.exec(dichiarazione)?.[1];
  if (codifica && !/^utf-?8$/i.test(codifica)) return false;
  if (/attributeName\s*=\s*["']?\s*(?:[^\s"':]+:)?(?:href|on[a-z]+)/i.test(testo)) return false;
  for (const m of testo.matchAll(ATTRIBUTO)) {
    const nome = m[1].toLowerCase();
    const valore = m[2] ?? m[3] ?? m[4] ?? "";
    const locale = nome.slice(nome.lastIndexOf(":") + 1);
    // un gestore di eventi, o un xml:base che sposterebbe anche i riferimenti #interni su un altro documento
    if (/^on/.test(locale) || nome === "xml:base") return false;
    const visto = comeLoVede(valore);
    if (locale === "href" && !visto.startsWith("#")) return false;
    if (PROTOCOLLO_ATTIVO.test(visto)) return false;
    if (locale === "style" && !cssPulito(valore)) return false;
  }
  for (const m of testo.matchAll(/<\s*(?:[^\s<>/:]+:)?style\b[^>]*>([\s\S]*?)<\s*\/\s*(?:[^\s<>/:]+:)?style\s*>/gi)) {
    if (!cssPulito(m[1].replace(/<!\[CDATA\[|\]\]>/g, ""))) return false;
  }
  return true;
}

// <?xml ...?>, commenti e un DOCTYPE (anche con un sottoinsieme interno: lo rifiuta svgAmmesso), poi l'elemento <svg
const RADICE_SVG = /^\s*(?:<\?xml[^>]*\?>\s*)?(?:<!--[\s\S]*?-->\s*)*(?:<!DOCTYPE[^>[]*(?:\[[\s\S]*?\])?\s*>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg[\s>/]/;

/** Riconosce il file dai primi byte e lo accetta o no. */
export function controllaImmagine(dati: Uint8Array): EsitoImmagine {
  const r = raster(dati);
  if (r) return { tipo: r };
  let testo: string;
  try {
    testo = new TextDecoder("utf-8", { fatal: true }).decode(dati).replace(/^\ufeff/, "");
  } catch {
    return { errore: "non_immagine" };
  }
  if (!RADICE_SVG.test(testo)) return { errore: "non_immagine" };
  return svgAmmesso(testo) ? { tipo: TIPI.svg } : { errore: "svg_non_ammesso" };
}
