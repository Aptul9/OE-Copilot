// L'espansione di una regola RRULE come la fa dateutil.rrule (la libreria sotto recurring_ical_events): stesse regole di default,
// stesso ordine dei risultati, stesso conteggio di COUNT. Lavora sull'ora sul muro (anno, mese, giorno, ora...): il fuso lo
// applica chi la chiama. Sono previste le frequenze dal giorno all'anno, le sole che scrivono Outlook, Google e gli altri.

import { ErroreValore } from "./tempo";

export type Naive = { a: number; m: number; g: number; h: number; mi: number; s: number };
export const ANNUALE = 0;
export const MENSILE = 1;
export const SETTIMANALE = 2;
export const GIORNALIERA = 3;

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
  tempi: [number, number, number][];
}

const FREQ: Record<string, number> = { YEARLY: ANNUALE, MONTHLY: MENSILE, WEEKLY: SETTIMANALE, DAILY: GIORNALIERA };
const GIORNI: Record<string, number> = { MO: 0, TU: 1, WE: 2, TH: 3, FR: 4, SA: 5, SU: 6 };
const INTERI = new Set(["COUNT", "INTERVAL"]);
const ELENCHI = new Set(["BYSETPOS", "BYMONTH", "BYMONTHDAY", "BYYEARDAY", "BYWEEKNO", "BYHOUR", "BYMINUTE", "BYSECOND"]);
const FREQ_ORARIE = new Set(["HOURLY", "MINUTELY", "SECONDLY"]);

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
  if (f === undefined) throw new ErroreValore("rrule.__init__() missing 1 required positional argument: 'freq'");
  if (FREQ_ORARIE.has(f)) throw new ErroreNonSupportato(f);
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
  if (!r.has("BYWEEKNO") && !byyearday && !bymonthday && !giorni) {
    if (freq === ANNUALE) {
      bymonth = bymonth ?? [inizio.m];
      bymonthday = [inizio.g];
    } else if (freq === MENSILE) bymonthday = [inizio.g];
    else if (freq === SETTIMANALE) byweekday = [dtstartSettimana];
  }
  if (r.has("BYWEEKNO")) throw new ErroreNonSupportato("BYWEEKNO");

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
  for (const h of valori("BYHOUR", inizio.h, 23)) for (const mi of valori("BYMINUTE", inizio.mi, 59)) for (const s of valori("BYSECOND", inizio.s, 59)) tempi.push([h, mi, s]);
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
    tempi,
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
  let { a: anno, m: mese, g: giorno } = inizio;
  let giornoSettimana = settimana(ordinale(anno, mese, giorno));
  const dallInizio = chiaveMuro(inizio);
  let conta = r.conta;
  let nth: Set<number> | null = null;
  let nthDi = "";
  for (let giro = 0; giro < LIMITE_GIRI; giro++) {
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
      if (r.byweekday && !r.byweekday.includes(settimana(n))) continue;
      if (nth && !nth.has(n)) continue;
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

    // il periodo dopo
    if (r.freq === ANNUALE) anno += r.intervallo;
    else if (r.freq === MENSILE) {
      mese += r.intervallo;
      if (mese > 12) {
        const div = Math.floor(mese / 12);
        mese -= div * 12;
        anno += div;
        if (mese === 0) {
          mese = 12;
          anno -= 1;
        }
      }
    } else {
      let passo: number;
      if (r.freq === SETTIMANALE) {
        passo = (r.wkst > giornoSettimana ? -(giornoSettimana + 1 + (6 - r.wkst)) : -(giornoSettimana - r.wkst)) + r.intervallo * 7;
        giornoSettimana = r.wkst;
      } else passo = r.intervallo;
      ({ a: anno, m: mese, g: giorno } = dallOrdinale(ordinale(anno, mese, giorno) + passo));
    }
  }
  throw new ErroreValore("la regola non finisce");
}
