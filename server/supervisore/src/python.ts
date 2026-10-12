// Le poche regole di Python su cui faceva affidamento il rilevatore in Python (tolto dal repository): il rilevatore in TypeScript deve dare lo
// stesso risultato sugli stessi dati (le uscite registrate in test/golden/rilevatore-accessi.json), quindi righe, JSON, base64, date e
// divisioni si comportano come là.

/** Il ValueError di Python. */
export class ErroreValore extends Error {}

/** Il TypeError di Python. */
export class ErroreTipo extends Error {}

const SEPARATORI_RIGA = /\r\n|[\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029]/;

/** str.splitlines(): separa sui terminatori di riga di Python e non lascia l'ultima riga vuota. */
export function righe(testo: string): string[] {
  if (testo === "") return [];
  const pezzi = testo.split(SEPARATORI_RIGA);
  if (pezzi[pezzi.length - 1] === "") pezzi.pop();
  return pezzi;
}

/** bool(valore) per un valore letto da un file JSON: vuoto e zero sono falsi. */
export function veroPython(v: unknown): boolean {
  if (v === null || v === undefined || v === false || v === 0 || v === "") return false;
  if (Array.isArray(v)) return v.length > 0;
  if (typeof v === "object") return Object.keys(v).length > 0;
  return true;
}

/** int(x) per un valore letto dal database: gli interi, i numeri con la frazione (troncati) e i testi che sono un intero. */
export function comeInteroPython(v: unknown): number {
  if (typeof v === "number") return Math.trunc(v);
  if (typeof v === "bigint") return Number(v);
  if (typeof v === "string") {
    if (/^\s*[+-]?\d+\s*$/.test(v)) return Number.parseInt(v, 10);
    throw new ErroreValore(`invalid literal for int() with base 10: '${v}'`);
  }
  throw new ErroreTipo(`int() argument must be a string, a bytes-like object or a real number, not '${v === null ? "NoneType" : typeof v}'`);
}

/** La divisione intera con arrotondamento per difetto di Python (anche per i negativi). */
export const dividiPerDifetto = (a: number, b: number): number => Math.floor(a / b);

/** int(x) di un numero: tronca verso lo zero, e come in Python non esiste per NaN e per l'infinito (ValueError, OverflowError). */
export function intero(x: number): number {
  if (!Number.isFinite(x)) throw new ErroreValore(`cannot convert float ${Number.isNaN(x) ? "NaN" : "infinity"} to integer`);
  return Math.trunc(x);
}

const ALFABETO = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/**
 * base64.b64decode(testo, altchars=b"-_") senza validate, come lo legge binascii.a2b_base64: i caratteri fuori dall'alfabeto si
 * saltano, "-" e "_" valgono come "+" e "/", il resto del testo dopo il riempimento "=" si ignora, una lunghezza impossibile o un
 * riempimento mancante è un errore (binascii.Error, una ValueError).
 */
export function decodificaBase64(testo: string): number[] {
  if (/[^\x00-\x7f]/.test(testo)) throw new ErroreValore("string argument should contain only ASCII characters");
  const uscita: number[] = [];
  let posizione = 0;
  let resto = 0;
  let riempimenti = 0;
  for (const carattere of testo) {
    if (carattere === "=") {
      if (posizione >= 2 && posizione + ++riempimenti >= 4) {
        posizione = 0;
        break;
      }
      continue;
    }
    const valore = ALFABETO.indexOf(carattere === "-" ? "+" : carattere === "_" ? "/" : carattere);
    if (valore < 0) continue;
    riempimenti = 0;
    switch (posizione) {
      case 0:
        posizione = 1;
        resto = valore;
        break;
      case 1:
        posizione = 2;
        uscita.push(((resto << 2) | (valore >> 4)) & 0xff);
        resto = valore & 0x0f;
        break;
      case 2:
        posizione = 3;
        uscita.push(((resto << 4) | (valore >> 2)) & 0xff);
        resto = valore & 0x03;
        break;
      default:
        posizione = 0;
        uscita.push(((resto << 6) | valore) & 0xff);
        resto = 0;
    }
  }
  if (posizione === 1) throw new ErroreValore("Invalid base64-encoded string");
  if (posizione !== 0) throw new ErroreValore("Incorrect padding");
  return uscita;
}

/** bytes.decode("latin-1"): un carattere per byte. */
export function comeLatin1(byte: readonly number[]): string {
  let testo = "";
  for (const b of byte) testo += String.fromCharCode(b);
  return testo;
}

const GIORNI_DEL_MESE = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/** Secondi dall'epoca di una data UTC: Date.UTC legge gli anni da 0 a 99 come 1900 a 1999, setUTCFullYear no. */
function secondiUtc(anno: number, mese: number, giorno: number, ore: number, minuti: number, secondi: number): number {
  const d = new Date(0);
  d.setUTCFullYear(anno, mese - 1, giorno);
  d.setUTCHours(ore, minuti, secondi, 0);
  return d.getTime() / 1000;
}

/**
 * datetime.strptime(testo, "%Y-%m-%d %H:%M:%S") come secondi di un orologio senza fuso (quelli che contano sono le differenze fra due
 * date lette così): ValueError se il testo non è nel formato o la data non esiste.
 */
export function secondiDaDataOra(testo: string): number {
  const m = /^(\d{4})-(\d{1,2})-(\d{1,2}) (\d{1,2}):(\d{1,2}):(\d{1,2})$/.exec(testo);
  if (!m) throw new ErroreValore(`time data '${testo}' does not match format '%Y-%m-%d %H:%M:%S'`);
  const [anno, mese, giorno, ore, minuti, secondi] = m.slice(1).map(Number) as [number, number, number, number, number, number];
  const bisestile = anno % 4 === 0 && (anno % 100 !== 0 || anno % 400 === 0);
  const giorniDelMese = (GIORNI_DEL_MESE[mese - 1] ?? 0) + (mese === 2 && bisestile ? 1 : 0);
  if (anno < 1 || mese < 1 || mese > 12 || giorno < 1 || giorno > giorniDelMese || ore > 23 || minuti > 59 || secondi > 59) {
    throw new ErroreValore(`date value out of range: '${testo}'`);
  }
  return secondiUtc(anno, mese, giorno, ore, minuti, secondi);
}

/**
 * datetime.strptime(testo, "%Y-%m-%dT%H:%M:%S.%f%z").timestamp() per le date dei log di Outlook (2026-10-11T05:03:21.996+0000):
 * secondi dall'epoca con la frazione, null se il testo non è in quel formato (come il ValueError che _epoca() intercettava).
 */
export function epocaDaLog(testo: string): number | null {
  const m =
    /^(\d{4})-(\d{1,2})-(\d{1,2})T(\d{1,2}):(\d{1,2}):(\d{1,2})\.(\d{1,6})(?:Z|([+-])(\d{2}):?([0-5]\d)(?::?([0-5]\d)(?:\.(\d{1,6}))?)?)$/.exec(testo);
  if (!m) return null;
  const [anno, mese, giorno, ore, minuti, secondi] = m.slice(1, 7).map(Number) as [number, number, number, number, number, number];
  const bisestile = anno % 4 === 0 && (anno % 100 !== 0 || anno % 400 === 0);
  const giorniDelMese = (GIORNI_DEL_MESE[mese - 1] ?? 0) + (mese === 2 && bisestile ? 1 : 0);
  if (anno < 1 || mese < 1 || mese > 12 || giorno < 1 || giorno > giorniDelMese || ore > 23 || minuti > 59 || secondi > 59) return null;
  const micro = Number((m[7] ?? "").padEnd(6, "0"));
  let scarto = 0;
  if (m[8]) {
    const oreFuso = Number(m[9]);
    if (oreFuso > 23) return null;
    const frazione = Number((m[12] ?? "").padEnd(6, "0")) / 1_000_000;
    scarto = (m[8] === "-" ? -1 : 1) * (oreFuso * 3600 + Number(m[10]) * 60 + Number(m[11] ?? "0") + frazione);
  }
  const intera = secondiUtc(anno, mese, giorno, ore, minuti, secondi);
  return (Math.round((intera - scarto) * 1_000_000) + micro) / 1_000_000;
}

/**
 * json.loads di Python: oltre al JSON accetta NaN, Infinity e -Infinity come numeri. Si ritenta con le tre parole al posto giusto
 * soltanto se il testo non si legge come JSON, quindi un documento valido passa com'è.
 */
export function leggiJsonPython(testo: string): unknown {
  try {
    return JSON.parse(testo);
  } catch (errore) {
    const sentinelle = testo.replace(/(?<=[:[,]\s*)(NaN|-?Infinity)(?=\s*[,\]}])/g, (parola) => JSON.stringify(`\u0000${parola}`));
    if (sentinelle === testo) throw errore;
    return JSON.parse(sentinelle, (_chiave, valore: unknown) => {
      if (typeof valore !== "string" || !valore.startsWith("\u0000")) return valore;
      const parola = valore.slice(1);
      return parola === "NaN" ? Number.NaN : parola === "Infinity" ? Number.POSITIVE_INFINITY : Number.NEGATIVE_INFINITY;
    });
  }
}

/** float(v) per un valore letto da un file JSON: i numeri, i booleani e i testi che sono un numero decimale. */
export function comeNumero(v: unknown): number {
  if (typeof v === "number") return v;
  if (typeof v === "boolean") return v ? 1 : 0;
  if (typeof v === "string") {
    const t = v.trim();
    if (/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.test(t)) return Number(t);
    throw new ErroreValore(`could not convert string to float: '${v}'`);
  }
  throw new ErroreTipo(`float() argument must be a string or a real number, not '${v === null ? "NoneType" : typeof v}'`);
}

const FUGHE: Record<string, string> = { '"': '\\"', "\\": "\\\\", "\n": "\\n", "\r": "\\r", "\t": "\\t", "\b": "\\b", "\f": "\\f" };

function testoJson(s: string): string {
  let fuori = '"';
  for (let i = 0; i < s.length; i++) {
    const c = s.charAt(i);
    const cod = s.charCodeAt(i);
    fuori += FUGHE[c] ?? (cod < 0x20 || cod > 0x7e ? "\\u" + cod.toString(16).padStart(4, "0") : c);
  }
  return fuori + '"';
}

/** Un numero come lo scrive repr() di Python: un valore intero con la frazione ha ".0". */
export function numeroDecimale(n: number): string {
  return Number.isInteger(n) && Math.abs(n) < 1e16 ? n.toFixed(1) : String(n);
}

export type ValoreJson = null | boolean | number | string | ValoreJson[] | Map<string, ValoreJson>;

/**
 * json.dumps(valore) con le impostazioni di Python: separatori ", " e ": ", caratteri non ASCII come \uXXXX. Un oggetto è una Map, per
 * tenere l'ordine delle chiavi come il dict. I numeri sono interi.
 */
export function jsonPython(v: ValoreJson): string {
  if (v === null) return "null";
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return String(v);
  if (typeof v === "string") return testoJson(v);
  if (Array.isArray(v)) return "[" + v.map(jsonPython).join(", ") + "]";
  return "{" + [...v].map(([k, x]) => testoJson(k) + ": " + jsonPython(x)).join(", ") + "}";
}
