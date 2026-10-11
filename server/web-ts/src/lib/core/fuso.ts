// Date e ore "locali" dell'app: ora di Roma senza scarto, come i datetime senza fuso del servizio Python (TZ=Europe/Rome).
// Si rappresentano con i loro pezzi e si scrivono come isoformat() di Python: AAAA-MM-GGTHH:MM:SS, e .ffffff solo se i
// microsecondi non sono zero.

export const FUSO = "Europe/Rome";

export type Naive = { a: number; m: number; g: number; h: number; mi: number; s: number; us: number };

/** L'errore con cui Python rifiuta una data (ValueError): il testo è quello che l'interfaccia mostra. */
export class ErroreData extends Error {}

const due = (n: number) => String(n).padStart(2, "0");

/** Come datetime.isoformat() */
export function iso(n: Naive): string {
  const base = `${String(n.a).padStart(4, "0")}-${due(n.m)}-${due(n.g)}T${due(n.h)}:${due(n.mi)}:${due(n.s)}`;
  return n.us ? `${base}.${String(n.us).padStart(6, "0")}` : base;
}

/** Come isoformat(timespec="seconds") */
export const isoSecondi = (n: Naive) => iso({ ...n, us: 0 });

/** Un valore per confrontare due date locali: microsecondi dall'epoca, contando l'ora locale come se fosse UTC. */
export function chiave(n: Naive): number {
  return Date.UTC(n.a, n.m - 1, n.g, n.h, n.mi, n.s) * 1000 + n.us;
}

export const mezzanotte = (a: number, m: number, g: number): Naive => ({ a, m, g, h: 0, mi: 0, s: 0, us: 0 });

/** La stessa ora locale `giorni` giorni dopo (aritmetica senza fuso: un giorno è sempre un giorno). */
export function piuGiorni(n: Naive, giorni: number): Naive {
  const d = new Date(Date.UTC(n.a, n.m - 1, n.g + giorni));
  return { ...n, a: d.getUTCFullYear(), m: d.getUTCMonth() + 1, g: d.getUTCDate() };
}

const ORA_DI_ROMA = new Intl.DateTimeFormat("en-CA", {
  timeZone: FUSO,
  hourCycle: "h23",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

/** Un istante (millisecondi dall'epoca) come ora locale di Roma. */
export function daIstante(ms: number, us = 0): Naive {
  const p: Record<string, number> = {};
  for (const x of ORA_DI_ROMA.formatToParts(ms)) if (x.type !== "literal") p[x.type] = Number(x.value);
  return { a: p.year, m: p.month, g: p.day, h: p.hour, mi: p.minute, s: p.second, us };
}

const GIORNI_NEL_MESE = (a: number, m: number) => new Date(Date.UTC(a, m, 0)).getUTCDate();

// datetime.fromisoformat (Python 3.12), nelle forme che arrivano davvero: data, data e ora (T o spazio), frazione di secondo,
// scarto (Z, +HH, +HH:MM, +HHMM). Il resto è "Invalid isoformat string", come in Python.
const ISO_RE = /^(\d{4})-?(\d{2})-?(\d{2})(?:[T ](\d{2})(?::?(\d{2})(?::?(\d{2})(?:[.,](\d+))?)?)?)?(Z|[+-]\d{2}(?::?\d{2}(?::?\d{2}(?:\.\d+)?)?)?)?$/;

/** Un testo ISO come datetime: i pezzi e lo scarto in minuti (null se il testo non ne ha). */
export function leggiIso(testo: string): { naive: Naive; scarto: number | null } {
  const m = ISO_RE.exec(testo);
  if (!m) throw new ErroreData(`Invalid isoformat string: '${testo}'`);
  const [a, mo, g, h, mi, s] = [m[1], m[2], m[3], m[4] ?? "0", m[5] ?? "0", m[6] ?? "0"].map(Number);
  if (mo < 1 || mo > 12) throw new ErroreData("month must be in 1..12");
  if (g < 1 || g > GIORNI_NEL_MESE(a, mo)) throw new ErroreData("day is out of range for month");
  if (h > 23) throw new ErroreData("hour must be in 0..23");
  if (mi > 59) throw new ErroreData("minute must be in 0..59");
  if (s > 59) throw new ErroreData("second must be in 0..59");
  const us = m[7] ? Number(m[7].slice(0, 6).padEnd(6, "0")) : 0;
  let scarto: number | null = null;
  if (m[8]) {
    if (m[8] === "Z") scarto = 0;
    else {
      const segno = m[8][0] === "-" ? -1 : 1;
      const cifre = m[8].slice(1).replace(/[:.]/g, "");
      scarto = segno * (Number(cifre.slice(0, 2)) * 60 + Number(cifre.slice(2, 4) || "0"));
    }
  }
  return { naive: { a, m: mo, g, h, mi, s, us }, scarto };
}

const inUtc = (n: Naive, scarto: number) => Date.UTC(n.a, n.m - 1, n.g, n.h, n.mi, n.s) - scarto * 60000;

/**
 * Un estremo dell'intervallo (caselle.parse_bound): la pagina manda 2026-07-20T00:00:00+02:00, gli eventi sono senza fuso, quindi
 * lo scarto, che è autentico, si converte nell'ora locale. Senza scarto il testo vale com'è.
 */
export function parseBound(testo: string): Naive {
  if (typeof testo !== "string") throw new ErroreData(`'${testo === null ? "NoneType" : typeof testo}' object has no attribute 'strip'`);
  const { naive, scarto } = leggiIso(testo.trim().replace(/Z$/, "+00:00"));
  return scarto === null ? naive : daIstante(inUtc(naive, scarto), naive.us);
}

/** Come datetime.fromisoformat, ma dei soli testi senza scarto (a quelli con lo scarto Python non sa confrontare un'ora locale). */
export function leggiNaive(testo: string): Naive {
  const { naive, scarto } = leggiIso(testo);
  if (scarto !== null) throw new ErroreData("can't compare offset-naive and offset-aware datetimes");
  return naive;
}

/** Un testo ISO come ora locale senza scarto (disponibilita._naive): con lo scarto si converte, senza vale com'è. */
export function naiveLocale(testo: string): Naive {
  const { naive, scarto } = leggiIso(String(testo).trim().replace(/Z/g, "+00:00"));
  return scarto === null ? naive : daIstante(inUtc(naive, scarto), naive.us);
}

/** L'ora, HH:MM, di un istante a Roma (time.strftime("%H:%M")) */
export function oraHHMM(ms: number): string {
  const n = daIstante(ms);
  return `${due(n.h)}:${due(n.mi)}`;
}

/** datetime.fromtimestamp(secondi).isoformat(timespec="seconds"): l'ora di Roma di un istante */
export function daSecondi(secondi: number): string {
  return isoSecondi(daIstante(Math.floor(secondi) * 1000));
}
