// Date e orari come li tratta Python (date e datetime), per riprodurre le risposte di recurring_ical_events senza una libreria di
// calendari: un orario con fuso è l'ora sul muro più il nome del fuso (e se è la seconda delle due ore ripetute dal cambio dell'ora).
// Due orari dello stesso fuso si confrontano e si sottraggono sul muro, due di fusi diversi sugli istanti: è la regola di Python.

export type Data = { tipo: "data"; d: Temporal.PlainDate };
export type Ora = { tipo: "ora"; w: Temporal.PlainDateTime; z: string | null; fold: 0 | 1 };
export type Tempo = Data | Ora;

/** Un valore non valido (ValueError di Python): il calendario o la regola non si leggono. */
export class ErroreValore extends Error {}
/** Un errore che Python non classifica come valore (KeyError, fuso sconosciuto...). */
export class ErroreAltro extends Error {}

export const data = (d: Temporal.PlainDate): Data => ({ tipo: "data", d });
export const ora = (w: Temporal.PlainDateTime, z: string | null = null, fold: 0 | 1 = 0): Ora => ({ tipo: "ora", w, z, fold });

const UTC_ALIAS = new Set(["UTC", "Etc/UTC", "Etc/UCT", "UCT", "Universal", "Etc/Universal", "Zulu", "Etc/Zulu"]);
export const èUtc = (o: Ora) => o.z !== null && UTC_ALIAS.has(o.z);

/** L'istante in millisecondi dall'epoca. */
export function istante(o: Ora): number {
  if (o.z === null) throw new ErroreValore("orario senza fuso");
  return o.w.toZonedDateTime(o.z, { disambiguation: o.fold ? "later" : "compatible" }).epochMilliseconds;
}

const muro = (w: Temporal.PlainDateTime) => Date.UTC(w.year, w.month - 1, w.day, w.hour, w.minute, w.second) / 1000;

/** -1, 0, 1 come il confronto fra datetime: sul muro se hanno lo stesso fuso (o nessuno), sugli istanti se no. */
export function confronta(a: Ora, b: Ora): number {
  if ((a.z === null) !== (b.z === null)) throw new ErroreAltro("can't compare offset-naive and offset-aware datetimes");
  if (a.z === b.z) return Math.sign(Temporal.PlainDateTime.compare(a.w, b.w));
  return Math.sign(istante(a) - istante(b));
}

/** a - b in secondi, con la stessa regola di confronto. */
export function differenza(a: Ora, b: Ora): number {
  if ((a.z === null) !== (b.z === null)) throw new ErroreAltro("can't subtract offset-naive and offset-aware datetimes");
  return a.z === b.z ? muro(a.w) - muro(b.w) : (istante(a) - istante(b)) / 1000;
}

/** datetime + timedelta: aritmetica sul muro, nello stesso fuso, e il risultato è la prima delle due ore ripetute. */
export function somma(o: Ora, secondi: number): Ora {
  return ora(secondi === 0 ? o.w : o.w.add({ seconds: secondi }), o.z, 0);
}

export const mezzanotte = (d: Temporal.PlainDate) => d.toPlainDateTime();

export function inFuso(ms: number, fuso: string): Ora {
  const z = Temporal.Instant.fromEpochMilliseconds(ms).toZonedDateTimeISO(fuso);
  const w = z.toPlainDateTime();
  const primo = w.toZonedDateTime(fuso, { disambiguation: "compatible" }).epochMilliseconds;
  return ora(w, fuso, primo === ms ? 0 : 1);
}

/** convert_to_datetime: una data diventa la mezzanotte; con `z` un orario senza fuso lo prende, senza `z` uno con fuso lo perde. */
export function comeOra(t: Tempo, z: string | null): Ora {
  if (t.tipo === "data") return ora(mezzanotte(t.d), z);
  if (t.z === null) return z === null ? t : ora(t.w, z, t.fold);
  return z === null ? ora(t.w, null) : t;
}

/** make_comparable: se c'è un orario con fuso, tutti diventano orari con quel fuso (il primo che si trova); se sono tutte date restano date. */
export function rendiComparabili(v: Tempo[]): Tempo[] {
  let z: string | null = null;
  let tutteDate = true;
  for (const t of v) {
    if (t.tipo === "data") continue;
    tutteDate = false;
    if (t.z !== null) {
      z = t.z;
      break;
    }
  }
  return tutteDate ? v : v.map((t) => comeOra(t, z));
}

/** Confronto di due valori già resi comparabili (due date o due orari). */
export function confrontaT(a: Tempo, b: Tempo): number {
  if (a.tipo === "data" && b.tipo === "data") return Math.sign(Temporal.PlainDate.compare(a.d, b.d));
  if (a.tipo === "ora" && b.tipo === "ora") return confronta(a, b);
  throw new ErroreAltro("can't compare datetime.datetime to datetime.date");
}

export function confrontaGrande(a: Tempo, b: Tempo): boolean {
  const [x, y] = rendiComparabili([a, b]) as [Tempo, Tempo];
  return confrontaT(x, y) > 0;
}

/** a - b in secondi di due valori comparabili. */
export function differenzaT(a: Tempo, b: Tempo): number {
  if (a.tipo === "data" && b.tipo === "data") return a.d.since(b.d, { largestUnit: "days" }).days * 86400;
  if (a.tipo === "ora" && b.tipo === "ora") return differenza(a, b);
  throw new ErroreAltro("unsupported operand type(s) for -: date and datetime");
}

/** Tempo + timedelta: a una data si aggiungono solo i giorni interi. */
export function sommaT(t: Tempo, secondi: number): Tempo {
  return t.tipo === "data" ? data(t.d.add({ days: Math.floor(secondi / 86400) })) : somma(t, secondi);
}

/** Il valore come data (convert_to_date). */
export const comeData = (t: Tempo): Temporal.PlainDate => (t.tipo === "data" ? t.d : t.w.toPlainDate());

const due = (n: number) => String(n).padStart(2, "0");
export const testoNaive = (w: Temporal.PlainDateTime) =>
  `${String(w.year).padStart(4, "0")}-${due(w.month)}-${due(w.day)}T${due(w.hour)}:${due(w.minute)}:${due(w.second)}`;

/**
 * to_recurrence_ids: come si riconosce un'occorrenza. Un orario con fuso ne ha due (l'istante in UTC e l'ora sul muro), uno senza
 * fuso o una data ne ha uno: un'esclusione che coincide in uno qualunque dei due toglie l'occorrenza.
 */
export function idRicorrenza(t: Tempo): string[] {
  if (t.tipo === "data") return [testoNaive(mezzanotte(t.d))];
  if (t.z === null) return [testoNaive(t.w)];
  const utc = Temporal.Instant.fromEpochMilliseconds(istante(t)).toZonedDateTimeISO("UTC").toPlainDateTime();
  return [testoNaive(utc), testoNaive(t.w)];
}

/** datetime.astimezone(fuso).replace(tzinfo=None): l'ora locale di Roma di un valore (le date a mezzanotte). */
export function inOraLocale(t: Tempo, fuso: string, stessoFusoInvariato: boolean): Temporal.PlainDateTime {
  if (t.tipo === "data") return mezzanotte(t.d);
  if (t.z === null) return t.w;
  if (stessoFusoInvariato && t.z === fuso) return t.w;
  return Temporal.Instant.fromEpochMilliseconds(istante(t)).toZonedDateTimeISO(fuso).toPlainDateTime();
}

/** Se il nome è un fuso per Python: ZoneInfo distingue le maiuscole ("europe/rome" no) e non conosce gli scarti ("+01:00"). */
const fusiNoti = new Map<string, boolean>();
export function fusoEsistente(nome: string): boolean {
  let ok = fusiNoti.get(nome);
  if (ok === undefined) {
    try {
      const id = Temporal.Instant.fromEpochMilliseconds(0).toZonedDateTimeISO(nome).timeZoneId;
      ok = !/^[+-]/.test(nome) && (id === nome || id.toLowerCase() !== nome.toLowerCase());
    } catch {
      ok = false;
    }
    fusiNoti.set(nome, ok);
  }
  return ok;
}
