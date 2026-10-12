// Un VTIMEZONE come lo interpreta Python: icalendar lo riscrive in testo (proprietà in ordine alfabetico) e dateutil (tz.tzical) lo legge. Le righe di
// quel testo le prepara ics.ts; qui c'è la lettura di tzical con i suoi errori e il suo modo di dare lo scarto a un'ora sul muro, comprese le ore che
// non esistono e quelle che si ripetono (non è il modo di zoneinfo: nel buco dell'ora legale vale lo scarto nuovo, la prima ora ripetuta ha lo scarto vecchio).

import type { FusoPersonalizzato } from "./fusi.ts";
import { type Naive, type Regola, chiaveMuro, iteraRegola, leggiRegola } from "./regola.ts";
import { ErroreTipo, ErroreValore, fusoEsistente } from "./tempo.ts";

export interface RigaTz {
  nome: string;
  /** I parametri come escono nel testo: "CHIAVE=valore". */
  param: string[];
  valore: string;
}

/** dateutil.parser.parse(testo, ignoretz=True) per quello che un VTIMEZONE scrive: una data, o una data con l'ora; la Z si scarta. */
function leggiNaive(testo: string): Naive {
  const m = /^(\d{4})(\d\d)(\d\d)(?:T(\d\d)(\d\d)(\d\d))?Z?$/.exec(testo.trim().toUpperCase());
  const errore = () => new ErroreValore(`Unknown string format: ${testo}`);
  if (!m) throw errore();
  const n: Naive = { a: +m[1]!, m: +m[2]!, g: +m[3]!, h: +(m[4] ?? 0), mi: +(m[5] ?? 0), s: +(m[6] ?? 0) };
  try {
    Temporal.PlainDateTime.from({ year: n.a, month: n.m, day: n.g, hour: n.h, minute: n.mi, second: n.s }, { overflow: "reject" });
  } catch {
    throw errore();
  }
  return n;
}

/** I cambi di una osservanza, in ordine, calcolati quando servono e ricordati (rrule con cache=True). */
class Cambi {
  private memoria: number[] = [];
  private finito = false;
  private readonly flusso: Generator<number>;
  constructor(flusso: Generator<number>) {
    this.flusso = flusso;
  }

  /** L'ultimo cambio che cade a `muro` o prima (rrule.before(dt, inc=True)), o null se non ce n'è. */
  prima(muro: number): number | null {
    while (!this.finito && (this.memoria.length === 0 || this.memoria[this.memoria.length - 1]! <= muro)) {
      const n = this.flusso.next();
      if (n.done) this.finito = true;
      else this.memoria.push(n.value);
    }
    let basso = 0;
    let alto = this.memoria.length;
    while (basso < alto) {
      const medio = (basso + alto) >> 1;
      if (this.memoria[medio]! <= muro) basso = medio + 1;
      else alto = medio;
    }
    return basso === 0 ? null : this.memoria[basso - 1]!;
  }
}

function* chiaviRegola(r: Regola, inizio: Naive): Generator<number> {
  const fine = r.until === null ? null : chiaveMuro(leggiNaive(r.until));
  for (const n of iteraRegola(r, inizio, fine === null ? null : (x) => chiaveMuro(x) > fine)) yield chiaveMuro(n);
}

/** Più flussi crescenti in uno, senza ripetere un valore (rruleset). */
function* unione(flussi: Iterator<number>[]): Generator<number> {
  const teste = flussi.map((f) => f.next());
  let ultimo: number | null = null;
  for (;;) {
    let scelto = -1;
    for (let i = 0; i < teste.length; i++) {
      const t = teste[i]!;
      if (!t.done && (scelto < 0 || t.value < (teste[scelto] as IteratorYieldResult<number>).value)) scelto = i;
    }
    if (scelto < 0) return;
    const v = (teste[scelto] as IteratorYieldResult<number>).value;
    teste[scelto] = flussi[scelto]!.next();
    if (ultimo === null || v !== ultimo) {
      yield v;
      ultimo = v;
    }
  }
}

function* escludi(flusso: Generator<number>, exdate: Set<number>, exrule: Generator<number> | null, exdateConFuso: boolean): Generator<number> {
  if (exdateConFuso) throw new ErroreTipo("can't compare offset-naive and offset-aware datetimes");
  let testa: IteratorResult<number> = exrule ? exrule.next() : { done: true, value: undefined };
  for (const v of flusso) {
    while (!testa.done && testa.value < v) testa = exrule!.next();
    if (!testa.done && testa.value === v) continue;
    if (exdate.has(v)) continue;
    yield v;
  }
}

interface Osservanza {
  da: number;
  a: number;
  diff: number;
  dst: boolean;
  cambi: Cambi;
}

/** _tzicalvtz: un fuso fatto di osservanze STANDARD e DAYLIGHT. */
export class ZonaVtz implements FusoPersonalizzato {
  private readonly osservanze: Osservanza[];
  constructor(osservanze: Osservanza[]) {
    this.osservanze = osservanze;
  }

  private trova(muro: number, fold: 0 | 1): Osservanza {
    if (this.osservanze.length === 1) return this.osservanze[0]!;
    let ultimoCambio: number | null = null;
    let ultima: Osservanza | null = null;
    for (const o of this.osservanze) {
      const cercato = o.diff < 0 && fold ? muro - o.diff : muro;
      const cambio = o.cambi.prima(cercato);
      if (cambio !== null && (ultimoCambio === null || ultimoCambio < cambio)) {
        ultimoCambio = cambio;
        ultima = o;
      }
    }
    if (ultima === null) {
      // prima del primo cambio: la prima osservanza standard; se non ce n'è Python inciampa (comp[0] su un oggetto che non si indicizza)
      ultima = this.osservanze.find((o) => !o.dst) ?? null;
      if (ultima === null) throw new ErroreTipo("'_tzicalvtzcomp' object is not subscriptable");
    }
    return ultima;
  }

  scarto(muro: number, fold: 0 | 1): number {
    return this.trova(muro, fold).a;
  }

  dst(muro: number, fold: 0 | 1): number {
    const o = this.trova(muro, fold);
    return o.dst ? o.diff : 0;
  }
}

/** tzical._parse_offset. */
function leggiScarto(testo: string): number {
  let s = testo.trim();
  if (!s) throw new ErroreValore("empty offset");
  let segno = 1;
  if (s[0] === "+" || s[0] === "-") {
    segno = s[0] === "+" ? 1 : -1;
    s = s.slice(1);
  }
  const num = (t: string) => {
    if (!/^\d+$/.test(t)) throw new ErroreValore(`invalid literal for int() with base 10: '${t}'`);
    return Number(t);
  };
  if (s.length === 4) return (num(s.slice(0, 2)) * 3600 + num(s.slice(2)) * 60) * segno;
  if (s.length === 6) return (num(s.slice(0, 2)) * 3600 + num(s.slice(2, 4)) * 60 + num(s.slice(4))) * segno;
  throw new ErroreValore("invalid offset: " + s);
}

/**
 * rrulestr._parse_date_value: un elenco di date e ore separate da virgola. Con TZID di un fuso che esiste i valori hanno il fuso e non si confrontano con i cambi,
 * che non ce l'hanno: Python dà TypeError quando li usa (aware); con un TZID che non trova li lascia senza fuso. I parametri nei messaggi sono in maiuscolo,
 * rrulestr maiuscolizza tutto il testo prima.
 */
function leggiValoriData(valore: string, param: string[]): { date: Naive[]; conFuso: boolean } {
  let trovato = false;
  let conFuso = false;
  for (const p of param) {
    if (p.toUpperCase().startsWith("TZID=")) {
      if (fusoEsistente(p.slice(5))) conFuso = true;
      continue;
    }
    if (p.toUpperCase() !== "VALUE=DATE-TIME" && p.toUpperCase() !== "VALUE=DATE") throw new ErroreValore("unsupported parm: " + p.toUpperCase());
    if (trovato) throw new ErroreValore("Multiple VALUE parameters specified: " + param.join(";").toUpperCase());
    trovato = true;
  }
  return { date: valore.split(",").map(leggiNaive), conFuso };
}

/** rrulestr(righe, compatible=True, ignoretz=True, cache=True): l'insieme dei cambi di una osservanza. */
function leggiCambi(righe: RigaTz[]): Cambi {
  const regole: string[] = [];
  const rdate: string[] = [];
  const exregole: string[] = [];
  const exdate: Naive[] = [];
  let exdateConFuso = false;
  let dtstart: Naive | null = null;
  for (const r of righe) {
    if (r.nome === "RRULE") {
      if (r.param.length) throw new ErroreValore("unsupported RRULE parm: " + r.param[0]!.toUpperCase());
      regole.push(r.valore);
    } else if (r.nome === "RDATE") {
      for (const p of r.param) if (p.toUpperCase() !== "VALUE=DATE-TIME") throw new ErroreValore("unsupported RDATE parm: " + p.toUpperCase());
      rdate.push(r.valore);
    } else if (r.nome === "EXRULE") {
      if (r.param.length) throw new ErroreValore("unsupported EXRULE parm: " + r.param[0]!.toUpperCase());
      exregole.push(r.valore);
    } else if (r.nome === "EXDATE") {
      const v = leggiValoriData(r.valore, r.param);
      exdate.push(...v.date);
      exdateConFuso = exdateConFuso || v.conFuso;
    } else if (r.nome === "DTSTART") {
      const v = leggiValoriData(r.valore, r.param).date;
      if (v.length !== 1) throw new ErroreValore("Multiple DTSTART values specified:" + r.valore);
      dtstart = v[0]!;
    } else throw new ErroreValore("unsupported property: " + r.nome);
  }
  if (dtstart === null) throw new ErroreValore("Unsupported rule set without DTSTART");
  const inizio = dtstart;
  const flussi: Iterator<number>[] = regole.map((t) => chiaviRegola(leggiRegola(t, inizio), inizio));
  const esplicite = rdate.flatMap((v) => v.split(",").map((d) => chiaveMuro(leggiNaive(d))));
  esplicite.push(chiaveMuro(inizio));
  flussi.push([...new Set(esplicite)].sort((x, y) => x - y)[Symbol.iterator]());
  const flussiEsclusi = exregole.map((t) => chiaviRegola(leggiRegola(t, inizio), inizio));
  const esclusiDate = new Set(exdate.map(chiaveMuro));
  return new Cambi(escludi(unione(flussi), esclusiDate, flussiEsclusi.length ? unione(flussiEsclusi) : null, exdateConFuso));
}

/** tzical(testo).get(): il fuso di un VTIMEZONE, o l'errore con il testo di Python. */
export function creaZonaDaRighe(righe: RigaTz[]): ZonaVtz {
  if (!righe.length) throw new ErroreValore("empty string");
  let tzid: string | null = null;
  let osservanze: Osservanza[] = [];
  let dentro = false;
  let tipo: string | null = null;
  let trovatoInizio = false;
  let da: number | null = null;
  let a: number | null = null;
  let righeCambi: RigaTz[] = [];
  let finito = false;
  for (const r of righe) {
    if (dentro) {
      if (r.nome === "BEGIN") {
        if (r.valore !== "STANDARD" && r.valore !== "DAYLIGHT") throw new ErroreValore("unknown component: " + r.valore);
        tipo = r.valore;
        trovatoInizio = false;
        da = null;
        a = null;
        righeCambi = [];
      } else if (r.nome === "END") {
        if (r.valore === "VTIMEZONE") {
          if (tipo) throw new ErroreValore("component not closed: " + tipo);
          if (!tzid) throw new ErroreValore("mandatory TZID not found");
          if (!osservanze.length) throw new ErroreValore("at least one component is needed");
          dentro = false;
          finito = true;
        } else if (r.valore === tipo) {
          if (!trovatoInizio) throw new ErroreValore("mandatory DTSTART not found");
          if (da === null) throw new ErroreValore("mandatory TZOFFSETFROM not found");
          if (a === null) throw new ErroreValore("mandatory TZOFFSETFROM not found");
          osservanze.push({ da, a, diff: a - da, dst: tipo === "DAYLIGHT", cambi: leggiCambi(righeCambi) });
          tipo = null;
        } else throw new ErroreValore("invalid component end: " + r.valore);
      } else if (tipo) {
        if (r.nome === "DTSTART") {
          for (const p of r.param) if (p !== "VALUE=DATE-TIME") throw new ErroreValore("Unsupported DTSTART param in VTIMEZONE: " + p);
          righeCambi.push(r);
          trovatoInizio = true;
        } else if (r.nome === "RRULE" || r.nome === "RDATE" || r.nome === "EXRULE" || r.nome === "EXDATE") righeCambi.push(r);
        else if (r.nome === "TZOFFSETFROM") {
          if (r.param.length) throw new ErroreValore(`unsupported ${r.nome} parm: ${r.param[0]} `);
          da = leggiScarto(r.valore);
        } else if (r.nome === "TZOFFSETTO") {
          if (r.param.length) throw new ErroreValore("unsupported TZOFFSETTO parm: " + r.param[0]);
          a = leggiScarto(r.valore);
        } else if (r.nome === "TZNAME") {
          if (r.param.length) throw new ErroreValore("unsupported TZNAME parm: " + r.param[0]);
        } else if (r.nome !== "COMMENT") throw new ErroreValore("unsupported property: " + r.nome);
      } else if (r.nome === "TZID") {
        if (r.param.length) throw new ErroreValore("unsupported TZID parm: " + r.param[0]);
        tzid = r.valore;
      } else if (r.nome !== "TZURL" && r.nome !== "LAST-MODIFIED" && r.nome !== "COMMENT") throw new ErroreValore("unsupported property: " + r.nome);
    } else if (r.nome === "BEGIN" && r.valore === "VTIMEZONE") {
      tzid = null;
      osservanze = [];
      dentro = true;
    }
  }
  if (!finito) throw new ErroreValore("no timezones defined");
  return new ZonaVtz(osservanze);
}
