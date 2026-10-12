// L'espansione di una regola RRULE come la fa dateutil.rrule (la libreria sotto recurring_ical_events): stesse regole di default,
// stesso ordine dei risultati, stesso conteggio di COUNT. Lavora sull'ora sul muro (anno, mese, giorno, ora...): il fuso lo
// applica chi la chiama. Sono previste le frequenze dal giorno all'anno, le sole che scrivono Outlook, Google e gli altri.

import { ErroreAltro, ErroreTipo, ErroreValore } from "./tempo.ts";

export type Naive = { a: number; m: number; g: number; h: number; mi: number; s: number };
export const ANNUALE = 0;
export const MENSILE = 1;
export const SETTIMANALE = 2;
export const GIORNALIERA = 3;
export const ORARIA = 4;
export const MINUTALE = 5;
export const SECONDALE = 6;

export interface Regola {
  freq: number;
  intervallo: number;
  wkst: number;
  conta: number | null;
  until: string | null;
  bymonth: number[] | null;
  byyearday: number[] | null;
  bymonthday: number[];
  bynmonthday: number[];
  byweekday: number[] | null;
  bynweekday: [number, number][] | null;
  bysetpos: number[] | null;
  byweekno: number[] | null;
  /** BYEASTER (solo di dateutil): i giorni prima o dopo la Pasqua. */
  byeaster: number[] | null;
  tempi: [number, number, number][];
  /** Le ore, i minuti e i secondi di una regola oraria, minutale o secondale (null: nessun vincolo). */
  orario: { byhour: number[] | null; byminute: number[] | null; bysecond: number[] | null } | null;
}

const FREQ: Record<string, number> = { YEARLY: ANNUALE, MONTHLY: MENSILE, WEEKLY: SETTIMANALE, DAILY: GIORNALIERA, HOURLY: ORARIA, MINUTELY: MINUTALE, SECONDLY: SECONDALE };
const GIORNI: Record<string, number> = { MO: 0, TU: 1, WE: 2, TH: 3, FR: 4, SA: 5, SU: 6 };
const INTERI = new Set(["COUNT", "INTERVAL"]);
const ELENCHI = new Set(["BYSETPOS", "BYMONTH", "BYMONTHDAY", "BYYEARDAY", "BYWEEKNO", "BYEASTER", "BYHOUR", "BYMINUTE", "BYSECOND"]);

const intero = (t: string) => {
  if (!/^[+-]?\d+$/.test(t.trim())) throw new ErroreValore(`invalid literal for int(): '${t}'`);
  return Number(t);
};

/** Il testo di una RRULE come lo legge rrulestr (nomi e valori in maiuscolo) e lo valida icalendar: una regola non valida è un errore. */
export function leggiRegola(testo: string, inizio: Naive): Regola {
  const r = new Map<string, string>();
  for (const coppia of testo.split(";")) {
    const pezzi = coppia.split("=");
    if (pezzi.length !== 2) continue;
    r.set(pezzi[0]!.trim().toUpperCase(), pezzi[1]!.trim().toUpperCase());
  }
  for (const k of r.keys()) {
    if (!(k === "FREQ" || k === "UNTIL" || k === "WKST" || k === "BYDAY" || INTERI.has(k) || ELENCHI.has(k)))
      throw new ErroreValore(`unknown parameter '${k}'`);
  }
  const f = r.get("FREQ");
  if (f === undefined) throw new ErroreTipo("rrule.__init__() missing 1 required positional argument: 'freq'");
  const freq = FREQ[f];
  if (freq === undefined) throw new ErroreValore(`invalid 'FREQ': ${f}`);
  const elenco = (k: string) => (r.has(k) ? r.get(k)!.split(",").map(intero) : null);
  const wk = r.get("WKST");
  if (wk !== undefined && GIORNI[wk] === undefined) throw new ErroreValore(`invalid 'WKST': ${wk}`);
  let conta = r.has("COUNT") ? intero(r.get("COUNT")!) : null;
  if (conta !== null && conta < 0) conta = null;
  const intervallo = r.has("INTERVAL") ? intero(r.get("INTERVAL")!) : 1;
  const until = r.get("UNTIL") ?? null;
  if (until !== null && !/^\d{8}(T\d{6}Z?)?$/.test(until)) throw new ErroreValore(`invalid until date: ${until}`);

  let bymonth = elenco("BYMONTH");
  const byyearday = elenco("BYYEARDAY");
  let bymonthday = elenco("BYMONTHDAY");
  const bysetpos = elenco("BYSETPOS");
  if (bysetpos?.some((p) => p === 0 || p < -366 || p > 366)) throw new ErroreValore("bysetpos must be between 1 and 366, or between -366 and -1");
  let giorni: Array<[number | null, number]> | null = null;
  if (r.has("BYDAY")) {
    giorni = r.get("BYDAY")!.split(",").map((g) => {
      const m = /^([+-]?\d{0,2})(SU|MO|TU|WE|TH|FR|SA)$/.exec(g);
      if (!m) throw new ErroreValore(`Expected weekday abbrevation, got: ${g}`);
      return [m[1] ? Number(m[1]) : null, GIORNI[m[2]!]!];
    });
  }
  const wkst = wk === undefined ? 0 : GIORNI[wk]!;
  const dtstartSettimana = (((Math.round(Date.UTC(inizio.a, inizio.m - 1, inizio.g) / 86400000) + 3) % 7) + 7) % 7;

  // Senza nessun BY di giorno o settimana la regola si aggancia a DTSTART
  let byweekday: number[] | null = null;
  if (!r.has("BYWEEKNO") && !r.has("BYEASTER") && !byyearday && !bymonthday && !giorni) {
    if (freq === ANNUALE) {
      bymonth = bymonth ?? [inizio.m];
      bymonthday = [inizio.g];
    } else if (freq === MENSILE) bymonthday = [inizio.g];
    else if (freq === SETTIMANALE) byweekday = [dtstartSettimana];
  }

  let bynweekday: [number, number][] | null = null;
  if (giorni) {
    const semplici = new Set<number>();
    const conOrdinale = new Map<string, [number, number]>();
    for (const [n, g] of giorni) {
      if (!n || freq > MENSILE) semplici.add(g);
      else conOrdinale.set(`${g},${n}`, [g, n]);
    }
    byweekday = semplici.size ? [...semplici].sort((x, y) => x - y) : null;
    bynweekday = conOrdinale.size ? [...conOrdinale.values()].sort((x, y) => x[0] - y[0] || x[1] - y[1]) : null;
  }
  const mensili = [...new Set(bymonthday ?? [])];
  const valori = (k: string, da: number, max: number) => {
    const v = elenco(k);
    if (v?.some((x) => x < 0 || x > max)) throw new ErroreValore(`${k.slice(2).toLowerCase()} must be in 0..${max}`);
    return v ? [...new Set(v)].sort((x, y) => x - y) : [da];
  };
  const tempi: [number, number, number][] = [];
  let orario: Regola["orario"] = null;
  if (freq < ORARIA) {
    for (const h of valori("BYHOUR", inizio.h, 23)) for (const mi of valori("BYMINUTE", inizio.mi, 59)) for (const s of valori("BYSECOND", inizio.s, 59)) tempi.push([h, mi, s]);
  } else {
    // __construct_byset: il valore deve poter essere raggiunto con l'intervallo, se no la regola sarebbe vuota
    const insieme = (v: number[], da: number, base: number) => {
      const mcd = massimoComuneDivisore(intervallo, base);
      const ok = [...new Set(v)].filter((x) => mcd === 1 || (((x - da) % mcd) + mcd) % mcd === 0);
      if (!ok.length) throw new ErroreValore("Invalid rrule byxxx generates an empty set.");
      return ok.sort((x, y) => x - y);
    };
    const semplice = (v: number[]) => [...new Set(v)].sort((x, y) => x - y);
    const h = elenco("BYHOUR");
    const mi = elenco("BYMINUTE");
    const s = elenco("BYSECOND");
    orario = {
      byhour: h ? (freq === ORARIA ? insieme(h, inizio.h, 24) : semplice(h)) : null,
      byminute: mi ? (freq === MINUTALE ? insieme(mi, inizio.mi, 60) : semplice(mi)) : freq < MINUTALE ? [inizio.mi] : null,
      bysecond: s ? (freq === SECONDALE ? insieme(s, inizio.s, 60) : semplice(s)) : freq < SECONDALE ? [inizio.s] : null,
    };
  }
  const settimane = elenco("BYWEEKNO");
  const pasque = elenco("BYEASTER");
  return {
    freq,
    intervallo,
    wkst,
    conta,
    until,
    bymonth: bymonth ? [...new Set(bymonth)].sort((x, y) => x - y) : null,
    byyearday: byyearday ? [...new Set(byyearday)].sort((x, y) => x - y) : null,
    bymonthday: mensili.filter((x) => x > 0).sort((x, y) => x - y),
    bynmonthday: mensili.filter((x) => x < 0).sort((x, y) => x - y),
    byweekday,
    bynweekday,
    bysetpos,
    byweekno: settimane ? [...new Set(settimane)].sort((x, y) => x - y) : null,
    byeaster: pasque ? [...pasque].sort((x, y) => x - y) : null,
    tempi,
    orario,
  };
}

/** Una regola o una frequenza che il motore non espande (BYWEEKNO, ore, minuti, secondi): non un calendario sbagliato. */
export class ErroreNonSupportato extends Error {}

const GIORNO_MS = 86400000;
const ordinale = (a: number, m: number, g: number) => Math.round(Date.UTC(a, m - 1, g) / GIORNO_MS);
const dallOrdinale = (n: number) => {
  const d = new Date(n * GIORNO_MS);
  return { a: d.getUTCFullYear(), m: d.getUTCMonth() + 1, g: d.getUTCDate() };
};
const settimana = (n: number) => (((n + 3) % 7) + 7) % 7; // lunedì = 0 (il 1/1/1970 era un giovedì)
const bisestile = (a: number) => (a % 4 === 0 && a % 100 !== 0) || a % 400 === 0;
const giorniDelMese = (a: number, m: number) => [31, bisestile(a) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1]!;
const mod7 = (x: number) => ((x % 7) + 7) % 7;
const modulo = (x: number, base: number) => ((x % base) + base) % base;
const massimoComuneDivisore = (a: number, b: number): number => {
  let x = Math.abs(a);
  let y = Math.abs(b);
  while (y) [x, y] = [y, x % y];
  return x;
};

/**
 * La maschera delle settimane di BYWEEKNO per un anno (_iterinfo.rebuild di dateutil): per ogni giorno dell'anno, e dei sette dopo, se cade in una
 * delle settimane chieste. La settimana 1 è la prima che ha almeno quattro giorni nell'anno, e inizia con il giorno WKST.
 */
function maschereSettimane(anno: number, wkst: number, byweekno: number[]): Uint8Array {
  const lunghezza = 365 + (bisestile(anno) ? 1 : 0);
  const giornoDelLunedi = settimana(ordinale(anno, 1, 1));
  const giornoSettimana = (i: number) => (giornoDelLunedi + i) % 7;
  const maschera = new Uint8Array(lunghezza + 14);
  let no1 = (7 - giornoDelLunedi + wkst) % 7;
  const primo = no1;
  let lunghezzaSettimane: number;
  if (no1 >= 4) {
    no1 = 0;
    lunghezzaSettimane = lunghezza + mod7(giornoDelLunedi - wkst);
  } else lunghezzaSettimane = lunghezza - no1;
  const numero = Math.floor(lunghezzaSettimane / 7) + Math.floor((lunghezzaSettimane % 7) / 4);
  for (let n of byweekno) {
    if (n < 0) n += numero + 1;
    if (!(n > 0 && n <= numero)) continue;
    let i: number;
    if (n > 1) {
      i = no1 + (n - 1) * 7;
      if (no1 !== primo) i -= 7 - primo;
    } else i = no1;
    for (let j = 0; j < 7; j++) {
      maschera[i] = 1;
      i++;
      if (giornoSettimana(i) === wkst) break;
    }
  }
  if (byweekno.includes(1)) {
    // anche la settimana 1 dell'anno dopo
    let i = no1 + numero * 7;
    if (no1 !== primo) i -= 7 - primo;
    if (i < lunghezza) {
      for (let j = 0; j < 7; j++) {
        maschera[i] = 1;
        i++;
        if (giornoSettimana(i) === wkst) break;
      }
    }
  }
  if (no1) {
    // e l'ultima settimana dell'anno prima, se è fra quelle chieste
    let ultimeSettimane: number;
    if (!byweekno.includes(-1)) {
      const giornoPrima = settimana(ordinale(anno - 1, 1, 1));
      let no1Prima = (7 - giornoPrima + wkst) % 7;
      const lunghezzaPrima = 365 + (bisestile(anno - 1) ? 1 : 0);
      if (no1Prima >= 4) {
        no1Prima = 0;
        ultimeSettimane = 52 + Math.floor(((lunghezzaPrima + mod7(giornoPrima - wkst)) % 7) / 4);
      } else ultimeSettimane = 52 + Math.floor(((lunghezza - no1) % 7) / 4);
    } else ultimeSettimane = -1;
    if (byweekno.includes(ultimeSettimane)) for (let i = 0; i < no1; i++) maschera[i] = 1;
  }
  return maschera;
}
/** easter(anno) di dateutil (la Pasqua occidentale): il giorno come numero di giorni dall'epoca. */
function pasqua(a: number): number {
  const g = a % 19;
  const c = Math.floor(a / 100);
  const h = modulo(c - Math.floor(c / 4) - Math.floor((8 * c + 13) / 25) + 19 * g + 15, 30);
  const i = h - Math.floor(h / 28) * (1 - Math.floor(h / 28) * Math.floor(29 / (h + 1)) * Math.floor((21 - g) / 11));
  const j = modulo(a + Math.floor(a / 4) + i + 2 - c + Math.floor(c / 4), 7);
  const p = i - j;
  return ordinale(a, 3 + Math.floor((p + 26) / 30), 1 + modulo(p + 27 + Math.floor((p + 6) / 40), 31));
}

/** eastermask: i giorni dell'anno scelti da BYEASTER. Un indice fuori dalla lista dà IndexError, uno negativo conta dalla fine, come in Python. */
function maschereDiPasqua(anno: number, scarti: number[]): Uint8Array {
  const lunghezza = 365 + (bisestile(anno) ? 1 : 0) + 7;
  const maschera = new Uint8Array(lunghezza);
  const giorno = pasqua(anno) - ordinale(anno, 1, 1);
  for (const s of scarti) {
    let i = giorno + s;
    if (i >= lunghezza || i < -lunghezza) throw new ErroreAltro("list assignment index out of range");
    if (i < 0) i += lunghezza;
    maschera[i] = 1;
  }
  return maschera;
}

export const chiaveMuro = (n: Naive) => Date.UTC(n.a, n.m - 1, n.g, n.h, n.mi, n.s) / 1000;

/** I giorni (come numero di giorni dall'epoca) in cui cade l'ennesimo giorno della settimana di BYDAY, nel mese o nell'anno del periodo. */
function conOrdinale(r: Regola, anno: number, mese: number): Set<number> {
  const intervalli: [number, number][] = [];
  if (r.freq === ANNUALE) {
    if (r.bymonth) {
      for (const m of r.bymonth) if (m >= 1 && m <= 12) intervalli.push([ordinale(anno, m, 1), ordinale(anno, m, giorniDelMese(anno, m))]);
    } else intervalli.push([ordinale(anno, 1, 1), ordinale(anno, 12, 31)]);
  } else intervalli.push([ordinale(anno, mese, 1), ordinale(anno, mese, giorniDelMese(anno, mese))]);
  const segnati = new Set<number>();
  for (const [primo, ultimo] of intervalli)
    for (const [g, n] of r.bynweekday!) {
      const i = n < 0 ? ultimo + (n + 1) * 7 - mod7(settimana(ultimo + (n + 1) * 7) - g) : primo + (n - 1) * 7 + mod7(7 - settimana(primo + (n - 1) * 7) + g);
      if (i >= primo && i <= ultimo) segnati.add(i);
    }
  return segnati;
}

const LIMITE_GIRI = 3_000_000;

/**
 * I risultati della regola in ordine, a partire da DTSTART (compreso se rientra nella regola). `dopoUntil` dice se un risultato
 * supera UNTIL (il confronto dipende dal fuso, lo fa chi chiama): al primo che lo supera la regola finisce.
 */
export function* iteraRegola(r: Regola, inizio: Naive, dopoUntil: ((n: Naive) => boolean) | null): Generator<Naive> {
  yield* r.freq >= ORARIA ? iteraOraria(r, inizio, dopoUntil) : iteraPerGiorni(r, inizio, dopoUntil);
}

function* iteraPerGiorni(r: Regola, inizio: Naive, dopoUntil: ((n: Naive) => boolean) | null): Generator<Naive> {
  let { a: anno, m: mese, g: giorno } = inizio;
  let giornoSettimana = settimana(ordinale(anno, mese, giorno));
  const dallInizio = chiaveMuro(inizio);
  let conta = r.conta;
  let nth: Set<number> | null = null;
  let nthDi = "";
  let annoSettimane = -1;
  let settimaneDi: Uint8Array | null = null;
  let annoPasqua = -1;
  let pasquaDi: Uint8Array | null = null;
  for (let giro = 0; giro < LIMITE_GIRI; giro++) {
    if (r.byweekno && annoSettimane !== anno) {
      settimaneDi = maschereSettimane(anno, r.wkst, r.byweekno);
      annoSettimane = anno;
    }
    if (r.byeaster && annoPasqua !== anno) {
      pasquaDi = maschereDiPasqua(anno, r.byeaster);
      annoPasqua = anno;
    }
    let da: number;
    let fine: number;
    if (r.freq === ANNUALE) {
      da = ordinale(anno, 1, 1);
      fine = da + (bisestile(anno) ? 366 : 365);
    } else if (r.freq === MENSILE) {
      da = ordinale(anno, mese, 1);
      fine = da + giorniDelMese(anno, mese);
    } else if (r.freq === SETTIMANALE) {
      da = ordinale(anno, mese, giorno);
      fine = da;
      for (let j = 0; j < 7; j++) {
        fine++;
        if (settimana(fine) === r.wkst) break;
      }
    } else {
      da = ordinale(anno, mese, giorno);
      fine = da + 1;
    }
    if (r.bynweekday && (r.freq === ANNUALE || r.freq === MENSILE)) {
      const di = r.freq === ANNUALE ? `${anno}` : `${anno}-${mese}`;
      if (di !== nthDi) {
        nth = conOrdinale(r, anno, mese);
        nthDi = di;
      }
    }

    const giorniValidi: number[] = [];
    for (let n = da; n < fine; n++) {
      const { a, m, g } = dallOrdinale(n);
      if (r.bymonth && !r.bymonth.includes(m)) continue;
      if (settimaneDi && !settimaneDi[n - ordinale(anno, 1, 1)]) continue;
      if (r.byweekday && !r.byweekday.includes(settimana(n))) continue;
      if (nth && !nth.has(n)) continue;
      if (pasquaDi && !pasquaDi[n - ordinale(anno, 1, 1)]) continue;
      if ((r.bymonthday.length || r.bynmonthday.length) && !r.bymonthday.includes(g) && !r.bynmonthday.includes(g - giorniDelMese(a, m) - 1)) continue;
      if (r.byyearday) {
        const dellAnno = n - ordinale(a, 1, 1) + 1;
        if (!r.byyearday.includes(dellAnno) && !r.byyearday.includes(dellAnno - (bisestile(a) ? 366 : 365) - 1)) continue;
      }
      giorniValidi.push(n);
    }

    const risultati: Naive[] = [];
    if (r.bysetpos && r.tempi.length) {
      const visti = new Set<number>();
      const T = r.tempi.length;
      for (const pos of r.bysetpos) {
        const q = pos < 0 ? Math.floor(pos / T) : Math.floor((pos - 1) / T);
        const t = (pos < 0 ? pos : pos - 1) - q * T;
        const k = q < 0 ? giorniValidi.length + q : q;
        if (k < 0 || k >= giorniValidi.length) continue;
        const d = dallOrdinale(giorniValidi[k]!);
        const [h, mi, s] = r.tempi[t]!;
        const res = { ...d, h, mi, s };
        const c = chiaveMuro(res);
        if (!visti.has(c)) {
          visti.add(c);
          risultati.push(res);
        }
      }
      risultati.sort((x, y) => chiaveMuro(x) - chiaveMuro(y));
    } else {
      for (const n of giorniValidi) for (const [h, mi, s] of r.tempi) risultati.push({ ...dallOrdinale(n), h, mi, s });
    }
    for (const res of risultati) {
      if (dopoUntil?.(res)) return;
      if (chiaveMuro(res) >= dallInizio) {
        if (conta !== null && --conta < 0) return;
        yield res;
      }
    }

    // il periodo dopo (dopo l'anno 9999 dateutil si ferma: una regola che non ha mai un giorno valido finisce)
    if (r.freq === ANNUALE) {
      anno += r.intervallo;
      if (anno > 9999) return;
    } else if (r.freq === MENSILE) {
      mese += r.intervallo;
      if (mese > 12) {
        const div = Math.floor(mese / 12);
        mese -= div * 12;
        anno += div;
        if (mese === 0) {
          mese = 12;
          anno -= 1;
        }
        if (anno > 9999) return;
      }
    } else {
      let passo: number;
      if (r.freq === SETTIMANALE) {
        passo = (r.wkst > giornoSettimana ? -(giornoSettimana + 1 + (6 - r.wkst)) : -(giornoSettimana - r.wkst)) + r.intervallo * 7;
        giornoSettimana = r.wkst;
      } else passo = r.intervallo;
      ({ a: anno, m: mese, g: giorno } = dallOrdinale(ordinale(anno, mese, giorno) + passo));
      if (anno > 9999) return;
    }
  }
  throw new ErroreValore("la regola non finisce");
}

/** next value di dateutil (__mod_distance): quanti giri e quale valore, partendo da `valore`, finché cade fra quelli chiesti. Senza trovarlo Python ha un TypeError. */
function distanza(valore: number, chiesti: number[], base: number, intervallo: number): [number, number] {
  let accumulo = 0;
  for (let i = 1; i <= base; i++) {
    const somma = valore + intervallo;
    accumulo += Math.floor(somma / base);
    valore = modulo(somma, base);
    if (chiesti.includes(valore)) return [accumulo, valore];
  }
  throw new ErroreTipo("cannot unpack non-iterable NoneType object");
}

/**
 * Le regole orarie, minutali e secondali: _iter di dateutil per HOURLY, MINUTELY e SECONDLY, un giorno alla volta. L'ora, il minuto e il secondo
 * avanzano di INTERVAL; con BYHOUR, BYMINUTE o BYSECOND dello stesso livello si salta al valore chiesto che segue; il giorno si controlla con gli
 * altri vincoli e, se non passa, si salta alla fine del giorno.
 */
function* iteraOraria(r: Regola, inizio: Naive, dopoUntil: ((n: Naive) => boolean) | null): Generator<Naive> {
  const o = r.orario!;
  const freq = r.freq;
  const intervallo = r.intervallo;
  let { a: anno, m: mese, g: giorno, h: ora, mi: minuto, s: secondo } = inizio;
  const dallInizio = chiaveMuro(inizio);
  let conta = r.conta;
  let annoSettimane = -1;
  let settimaneDi: Uint8Array | null = null;
  let annoPasqua = -1;
  let pasquaDi: Uint8Array | null = null;
  const orari = (h: number, mi: number, s: number): [number, number, number][] => {
    if (freq === ORARIA) return o.byminute!.flatMap((m2) => o.bysecond!.map((s2): [number, number, number] => [h, m2, s2]));
    if (freq === MINUTALE) return o.bysecond!.map((s2): [number, number, number] => [h, mi, s2]);
    return [[h, mi, s]];
  };
  let timeset: [number, number, number][] =
    (o.byhour && !o.byhour.includes(ora)) || (freq >= MINUTALE && o.byminute && !o.byminute.includes(minuto)) || (freq >= SECONDALE && o.bysecond && !o.bysecond.includes(secondo))
      ? []
      : orari(ora, minuto, secondo);
  for (let giro = 0; giro < LIMITE_GIRI_ORARI; giro++) {
    if (r.byweekno && annoSettimane !== anno) {
      settimaneDi = maschereSettimane(anno, r.wkst, r.byweekno);
      annoSettimane = anno;
    }
    if (r.byeaster && annoPasqua !== anno) {
      pasquaDi = maschereDiPasqua(anno, r.byeaster);
      annoPasqua = anno;
    }
    const n = ordinale(anno, mese, giorno);
    let filtrato = false;
    if (r.bymonth && !r.bymonth.includes(mese)) filtrato = true;
    else if (settimaneDi && !settimaneDi[n - ordinale(anno, 1, 1)]) filtrato = true;
    else if (r.byweekday && !r.byweekday.includes(settimana(n))) filtrato = true;
    else if (pasquaDi && !pasquaDi[n - ordinale(anno, 1, 1)]) filtrato = true;
    else if ((r.bymonthday.length || r.bynmonthday.length) && !r.bymonthday.includes(giorno) && !r.bynmonthday.includes(giorno - giorniDelMese(anno, mese) - 1)) filtrato = true;
    else if (r.byyearday) {
      const dellAnno = n - ordinale(anno, 1, 1) + 1;
      if (!r.byyearday.includes(dellAnno) && !r.byyearday.includes(dellAnno - (bisestile(anno) ? 366 : 365) - 1)) filtrato = true;
    }
    const risultati: Naive[] = [];
    if (r.bysetpos && timeset.length) {
      const T = timeset.length;
      const giorni = filtrato ? [] : [n];
      const visti = new Set<number>();
      for (const pos of r.bysetpos) {
        const q = pos < 0 ? Math.floor(pos / T) : Math.floor((pos - 1) / T);
        const t = (pos < 0 ? pos : pos - 1) - q * T;
        const k = q < 0 ? giorni.length + q : q;
        if (k < 0 || k >= giorni.length) continue;
        const [h, mi, s] = timeset[t]!;
        const res = { ...dallOrdinale(giorni[k]!), h, mi, s };
        const c = chiaveMuro(res);
        if (!visti.has(c)) {
          visti.add(c);
          risultati.push(res);
        }
      }
      risultati.sort((x, y) => chiaveMuro(x) - chiaveMuro(y));
    } else if (!filtrato) for (const [h, mi, s] of timeset) risultati.push({ ...dallOrdinale(n), h, mi, s });
    for (const res of risultati) {
      if (dopoUntil?.(res)) return;
      if (chiaveMuro(res) >= dallInizio) {
        if (conta !== null && --conta < 0) return;
        yield res;
      }
    }

    let correggiGiorno = false;
    if (freq === ORARIA) {
      if (filtrato) ora += Math.floor((23 - ora) / intervallo) * intervallo;
      let giorniInPiu: number;
      if (o.byhour) [giorniInPiu, ora] = distanza(ora, o.byhour, 24, intervallo);
      else {
        giorniInPiu = Math.floor((ora + intervallo) / 24);
        ora = modulo(ora + intervallo, 24);
      }
      if (giorniInPiu) {
        giorno += giorniInPiu;
        correggiGiorno = true;
      }
      timeset = orari(ora, minuto, secondo);
    } else if (freq === MINUTALE) {
      if (filtrato) minuto += Math.floor((1439 - (ora * 60 + minuto)) / intervallo) * intervallo;
      let trovato = false;
      const giro24 = 24 * 60;
      for (let j = 0; j < Math.floor(giro24 / massimoComuneDivisore(intervallo, giro24)); j++) {
        let oreInPiu: number;
        if (o.byminute) [oreInPiu, minuto] = distanza(minuto, o.byminute, 60, intervallo);
        else {
          oreInPiu = Math.floor((minuto + intervallo) / 60);
          minuto = modulo(minuto + intervallo, 60);
        }
        const giorniInPiu = Math.floor((ora + oreInPiu) / 24);
        ora = modulo(ora + oreInPiu, 24);
        if (giorniInPiu) {
          giorno += giorniInPiu;
          correggiGiorno = true;
          filtrato = false;
        }
        if (!o.byhour || o.byhour.includes(ora)) {
          trovato = true;
          break;
        }
      }
      if (!trovato) throw new ErroreValore("Invalid combination of interval and byhour resulting in empty rule.");
      timeset = orari(ora, minuto, secondo);
    } else {
      if (filtrato) secondo += Math.floor((86399 - (ora * 3600 + minuto * 60 + secondo)) / intervallo) * intervallo;
      let trovato = false;
      const giro24 = 24 * 3600;
      for (let j = 0; j < Math.floor(giro24 / massimoComuneDivisore(intervallo, giro24)); j++) {
        let minutiInPiu: number;
        if (o.bysecond) [minutiInPiu, secondo] = distanza(secondo, o.bysecond, 60, intervallo);
        else {
          minutiInPiu = Math.floor((secondo + intervallo) / 60);
          secondo = modulo(secondo + intervallo, 60);
        }
        const oreInPiu = Math.floor((minuto + minutiInPiu) / 60);
        minuto = modulo(minuto + minutiInPiu, 60);
        if (oreInPiu) {
          ora += oreInPiu;
          const giorniInPiu = Math.floor(ora / 24);
          ora = modulo(ora, 24);
          if (giorniInPiu) {
            giorno += giorniInPiu;
            correggiGiorno = true;
          }
        }
        if ((!o.byhour || o.byhour.includes(ora)) && (!o.byminute || o.byminute.includes(minuto)) && (!o.bysecond || o.bysecond.includes(secondo))) {
          trovato = true;
          break;
        }
      }
      if (!trovato) throw new ErroreValore("Invalid combination of interval, byhour and byminute resulting in empty rule.");
      timeset = orari(ora, minuto, secondo);
    }
    if (correggiGiorno && giorno > 28) {
      let giorniMese = giorniDelMese(anno, mese);
      while (giorno > giorniMese) {
        giorno -= giorniMese;
        mese++;
        if (mese === 13) {
          mese = 1;
          anno++;
          if (anno > 9999) return;
        }
        giorniMese = giorniDelMese(anno, mese);
      }
    }
  }
  throw new ErroreValore("la regola non finisce");
}

const LIMITE_GIRI_ORARI = 200_000_000;
