// datetime.fromisoformat di Python 3.12 (il C di _datetimemodule.c, passo per passo: accetta più forme di quelle che ci si aspetta, e il
// testo dell'errore dipende da dove si ferma) e _locale() di agent.py: un estremo ISO, con o senza scarto, come ora locale senza fuso.

import { pyReprDelta, pyReprStr } from "./pyfmt.ts";
import { ErroreSenzaRisposta, ErroreValore } from "./tempo.ts";

export interface DataOraPy {
  a: number;
  m: number;
  g: number;
  h: number;
  mi: number;
  s: number;
  us: number;
  /** Lo scarto dall'UTC in secondi e microsecondi (con il segno), se il testo ne ha uno. */
  scarto: { s: number; us: number } | null;
}

const cifraAscii = (c: number) => c >= 0x30 && c <= 0x39;
const NUL = 0;

class Lettore {
  readonly b: Uint8Array;
  constructor(b: Uint8Array) {
    this.b = b;
  }
  at(i: number): number {
    return i >= 0 && i < this.b.length ? this.b[i]! : NUL;
  }
  /** parse_digits: `n` cifre ASCII di seguito, sommate a `v`; p = -1 se una non lo è. */
  cifre(p: number, n: number, v = 0): [number, number] {
    for (let i = 0; i < n; i++) {
      const t = this.at(p++) - 0x30;
      if (t < 0 || t > 9) return [-1, v];
      v = v * 10 + t;
    }
    return [p, v];
  }
}

const bisestile = (a: number) => a % 4 === 0 && (a % 100 !== 0 || a % 400 === 0);
const GIORNI_PRIMA = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const giorniPrimaAnno = (a: number) => (a - 1) * 365 + Math.floor((a - 1) / 4) - Math.floor((a - 1) / 100) + Math.floor((a - 1) / 400);
/** Il numero d'ordine di un giorno, con 0001-01-01 = 1. */
const ordinale = (a: number, m: number, g: number) => giorniPrimaAnno(a) + GIORNI_PRIMA[m]! + (m > 2 && bisestile(a) ? 1 : 0) + g;

function daOrdinale(ord: number): [number, number, number] {
  let n = ord - 1;
  const n400 = Math.floor(n / 146097);
  n -= n400 * 146097;
  const n100 = Math.floor(n / 36524);
  n -= n100 * 36524;
  const n4 = Math.floor(n / 1461);
  n -= n4 * 1461;
  const n1 = Math.floor(n / 365);
  n -= n1 * 365;
  const anno = n400 * 400 + 1 + n100 * 100 + n4 * 4 + n1;
  if (n1 === 4 || n100 === 4) return [anno - 1, 12, 31];
  const bis = n1 === 3 && (n4 !== 24 || n100 === 3);
  let mese = (n + 50) >> 5;
  let prima = GIORNI_PRIMA[mese]! + (mese > 2 && bis ? 1 : 0);
  if (prima > n) {
    mese -= 1;
    prima -= mese === 2 && bis ? 29 : [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][mese]!;
  }
  n -= prima;
  return [anno, mese, n + 1];
}

/** iso_to_ymd: da anno, settimana e giorno ISO a data; 0 se va bene, un numero negativo se la settimana o il giorno non esistono. */
function isoAData(anno: number, settimana: number, giorno: number): { cod: number; d?: [number, number, number] } {
  if (anno < 1 || anno > 9999) return { cod: -4 };
  if (settimana <= 0 || settimana >= 53) {
    let fuori = true;
    if (settimana === 53) {
      const primo = (ordinale(anno, 1, 1) + 6) % 7;
      if (primo === 3 || (primo === 2 && bisestile(anno))) fuori = false;
    }
    if (fuori) return { cod: -2 };
  }
  if (giorno <= 0 || giorno >= 8) return { cod: -3 };
  const primoGiorno = ordinale(anno, 1, 1);
  const primoSettimana = (primoGiorno + 6) % 7;
  let lunedi = primoGiorno - primoSettimana;
  if (primoSettimana > 3) lunedi += 7;
  return { cod: 0, d: daOrdinale(lunedi + (settimana - 1) * 7 + giorno - 1) };
}

function trovaSeparatore(l: Lettore): number {
  const len = l.b.length;
  if (len === 7) return 7;
  const TRATTINO = 0x2d;
  const W = 0x57;
  if (l.at(4) === TRATTINO) {
    if (l.at(5) === W) {
      if (len < 8) return -1;
      if (len > 8 && l.at(8) === TRATTINO) {
        if (len === 9) return -1;
        if (len > 10 && cifraAscii(l.at(10))) return 8;
        return 10;
      }
      return 8;
    }
    return 10;
  }
  if (l.at(4) === W) {
    let i = 7;
    for (; i < len; i++) if (!cifraAscii(l.b[i]!)) break;
    if (i < 9) return i;
    return i % 2 === 0 ? 7 : 8;
  }
  return 8;
}

/** parse_isoformat_date: 0 se va bene, un numero negativo se no. `len` è dove finisce la parte della data. */
function leggiData(l: Lettore, len: number, o: DataOraPy): number {
  let [p, anno] = l.cifre(0, 4);
  if (p < 0) return -1;
  const conSeparatore = l.at(p) === 0x2d;
  if (conSeparatore) p++;
  if (l.at(p) === 0x57) {
    p++;
    let settimana: number;
    [p, settimana] = l.cifre(p, 2);
    if (p < 0) return -3;
    let giorno = 0;
    if (p < len) {
      if (conSeparatore && l.at(p++) !== 0x2d) return -2;
      [p, giorno] = l.cifre(p, 1);
      if (p < 0) return -4;
    } else giorno = 1;
    const r = isoAData(anno, settimana, giorno);
    if (r.cod) return -3 + r.cod;
    [o.a, o.m, o.g] = r.d!;
    return 0;
  }
  o.a = anno;
  let mese: number;
  [p, mese] = l.cifre(p, 2);
  if (p < 0) return -1;
  o.m = mese;
  if (conSeparatore && l.at(p++) !== 0x2d) return -2;
  let giorno: number;
  [p, giorno] = l.cifre(p, 2);
  if (p < 0) return -1;
  o.g = giorno;
  return 0;
}

const CORREZIONE = [100000, 10000, 1000, 100, 10];

/** parse_hh_mm_ss_ff: 0 se la stringa finisce qui, 1 se segue altro, un numero negativo se non è un'ora. */
function leggiOra(l: Lettore, inizio: number, fine: number, v: { h: number; mi: number; s: number; us: number }): number {
  v.h = v.mi = v.s = v.us = 0;
  const campi = ["h", "mi", "s"] as const;
  let p = inizio;
  let conSeparatore = true;
  for (let i = 0; i < 3; i++) {
    let n: number;
    [p, n] = l.cifre(p, 2);
    if (p < 0) return -3;
    v[campi[i]!] = n;
    const c = l.at(p++);
    if (i === 0) conSeparatore = c === 0x3a;
    if (p >= fine) return c !== NUL ? 1 : 0;
    if (conSeparatore && c === 0x3a) continue;
    if (c === 0x2e || c === 0x2c) break;
    if (!conSeparatore) p--;
    else return -4;
  }
  const resto = fine - p;
  const daLeggere = resto < 0 || resto >= 6 ? 6 : resto;
  let micro: number;
  [p, micro] = l.cifre(p, daLeggere);
  if (p < 0) return -3;
  if (daLeggere < 6 && daLeggere > 0) micro *= CORREZIONE[daLeggere - 1]!;
  v.us = micro;
  while (cifraAscii(l.at(p))) p++;
  return l.at(p) !== NUL ? 1 : 0;
}

/** parse_isoformat_time: 0 senza scarto, 1 con scarto, un numero negativo se non va. */
function leggiOraEScarto(l: Lettore, inizio: number, fine: number, o: DataOraPy): number {
  let marca = inizio;
  do {
    const c = l.at(marca);
    if (c === 0x5a || c === 0x2b || c === 0x2d) break;
  } while (++marca < fine);
  const rv = leggiOra(l, inizio, marca, o);
  if (rv < 0) return rv;
  if (marca === fine) return rv === 1 ? -5 : 0;
  if (l.at(marca) === 0x5a) {
    o.scarto = { s: 0, us: 0 };
    return l.at(marca + 1) !== NUL ? -5 : 1;
  }
  const segno = l.at(marca) === 0x2d ? -1 : 1;
  const z = { h: 0, mi: 0, s: 0, us: 0 };
  const rz = leggiOra(l, marca + 1, fine, z);
  o.scarto = { s: segno * (z.h * 3600 + z.mi * 60 + z.s), us: segno * z.us };
  return rz ? -5 : 1;
}

function sanifica(testo: string): string | null {
  const punti = Array.from(testo);
  if (punti.length < 7) return null;
  // un surrogato solo può stare al posto del separatore (posizioni 7, 8 o 10): diventa una T
  for (const pos of [7, 8, 10]) {
    if (pos > punti.length) break;
    const c = punti[pos]?.charCodeAt(0);
    if (c !== undefined && punti[pos]!.length === 1 && c >= 0xd800 && c <= 0xdfff) {
      punti[pos] = "T";
      break;
    }
  }
  const pulito = punti.join("");
  return pulito.isWellFormed() ? pulito : null;
}

/** datetime.fromisoformat(): la data e l'ora come scritte nel testo, con lo scarto se c'è; ValueError con il testo di Python se no. */
export function fromIsoformat(testo: string): DataOraPy {
  const invalida = () => new ErroreValore(`Invalid isoformat string: ${pyReprStr(testo)}`);
  const pulito = sanifica(testo);
  if (pulito === null) throw invalida();
  const l = new Lettore(new TextEncoder().encode(pulito));
  const len = l.b.length;
  const sep = trovaSeparatore(l);
  const o: DataOraPy = { a: 0, m: 0, g: 0, h: 0, mi: 0, s: 0, us: 0, scarto: null };
  if (sep < 0) throw invalida();
  let rv = leggiData(l, sep, o);
  if (!rv && len > sep) {
    let p = sep;
    const primo = l.at(p);
    if ((primo & 0x80) === 0) p += 1;
    else {
      const alto = primo & 0xf0;
      p += alto === 0xe0 ? 3 : alto === 0xf0 ? 4 : 2;
    }
    rv = leggiOraEScarto(l, p, len, o);
  }
  if (rv < 0) throw invalida();
  if (rv === 1 && o.scarto && (o.scarto.s !== 0)) {
    // new_timezone: lo scarto deve stare fra -24 e +24 ore (esclusi)
    const totale = o.scarto.s * 1_000_000 + o.scarto.us;
    const giorni = Math.floor(totale / 86_400_000_000);
    const resto = totale - giorni * 86_400_000_000;
    if ((giorni === -1 && resto === 0) || giorni < -1 || giorni >= 1) {
      const secondi = Math.floor(resto / 1_000_000);
      throw new ErroreValore(`offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not ${pyReprDelta(giorni, secondi, resto - secondi * 1_000_000)}.`);
    }
  }
  if (rv === 1 && o.scarto && o.scarto.s === 0) o.scarto = { s: 0, us: 0 }; // lo scarto zero è UTC, senza i microsecondi
  if (o.a < 1 || o.a > 9999) throw new ErroreValore(`year ${o.a} is out of range`);
  if (o.m < 1 || o.m > 12) throw new ErroreValore("month must be in 1..12");
  if (o.g < 1 || o.g > [0, 31, bisestile(o.a) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][o.m]!) throw new ErroreValore("day is out of range for month");
  if (o.h > 23) throw new ErroreValore("hour must be in 0..23");
  if (o.mi > 59) throw new ErroreValore("minute must be in 0..59");
  if (o.s > 59) throw new ErroreValore("second must be in 0..59");
  return o;
}

const due = (n: number) => String(n).padStart(2, "0");

/** isoformat() di una data e ora senza fuso: i microsecondi solo se ci sono. */
export function isoNaive(w: Temporal.PlainDateTime): string {
  const base = `${String(w.year).padStart(4, "0")}-${due(w.month)}-${due(w.day)}T${due(w.hour)}:${due(w.minute)}:${due(w.second)}`;
  const us = w.millisecond * 1000 + w.microsecond;
  return us ? `${base}.${String(us).padStart(6, "0")}` : base;
}

/** _locale() di agent.py: un estremo ISO come ora locale senza fuso (con uno scarto si converte, senza si lascia com'è). */
export function oraLocaleDa(testo: string, zona: string): Temporal.PlainDateTime {
  const d = fromIsoformat(testo.replaceAll("Z", "+00:00"));
  const base = { year: d.a, month: d.m, day: d.g, hour: d.h, minute: d.mi, second: d.s };
  if (d.scarto === null) return Temporal.PlainDateTime.from({ ...base, millisecond: Math.floor(d.us / 1000), microsecond: d.us % 1000 });
  // astimezone(): l'istante in UTC (fuori da 0001..9999 Python solleva OverflowError, che l'agente non prende) e poi l'ora locale
  let micro = d.us - d.scarto.us;
  const riporto = Math.floor(micro / 1_000_000);
  micro -= riporto * 1_000_000;
  const utc = Temporal.PlainDateTime.from(base).subtract({ seconds: d.scarto.s - riporto });
  if (utc.year < 1 || utc.year > 9999) throw new ErroreSenzaRisposta("OverflowError: date value out of range");
  const locale = utc.toZonedDateTime("UTC").withTimeZone(zona).toPlainDateTime();
  if (locale.year < 1 || locale.year > 9999) throw new ErroreSenzaRisposta("OverflowError: date value out of range");
  return locale.with({ millisecond: Math.floor(micro / 1000), microsecond: micro % 1000 });
}
