// Le righe di calendar.db del telefono Android rimesse in serie e espanse: quello che l'agente Python (tolto dal repository) faceva fra la query SQL e
// l'espansione (_quando, la durata "P900S", le colonne RRULE/RDATE/EXDATE, le occorrenze annullate che diventano EXDATE, le modifiche).

import { type Riga, FUSO_LOCALE, leggiElencoTempi, leggiIsoLocale, leggiProprieta, leggiRdate, leggiRigaIcs, parametro, regolaPerPython, èRotta } from "./ics.ts";
import { pyReprStr, pyStrip } from "./pyfmt.ts";
import { type Componente, type Rdate, espandi } from "./serie.ts";
import { type Tempo, ErroreValore, data, erroreRotta, fusoEsistente, inFuso, inOraLocale, ora, testoNaive } from "./tempo.ts";

/** Le colonne di Events che servono all'espansione (la riga com'è nel database, già senza gli eventi cancellati). */
export interface RigaAndroid {
  _id: number;
  _sync_id: string | null;
  title: string | null;
  eventLocation: string | null;
  description: string | null;
  eventStatus: number | null;
  dtstart: number | null;
  dtend: number | null;
  eventTimezone: string | null;
  eventEndTimezone: string | null;
  duration: string | null;
  allDay: number | null;
  rrule: string | null;
  rdate: string | null;
  exrule: string | null;
  exdate: string | null;
  original_id: number | null;
  original_sync_id: string | null;
  originalInstanceTime: number | null;
  originalAllDay: number | null;
}

export interface OccorrenzaAndroid {
  uid: string;
  titolo: string;
  luogo: string;
  descrizione: string;
  inizio: string;
  fine: string;
  tuttoGiorno: boolean;
}

export interface OpzioniAndroid {
  fuso?: string;
}

const ANNULLATO = 2; // Events.STATUS_CANCELED

/** Istante di Android (millisecondi UTC + nome del fuso) -> data/ora: i giorni interi a mezzanotte UTC sono date, un fuso che non c'è è UTC. */
function quando(ms: number, fuso: string | null, tuttoIlGiorno: boolean): Tempo {
  if (tuttoIlGiorno) return data(Temporal.Instant.fromEpochMilliseconds(ms).toZonedDateTimeISO("UTC").toPlainDate());
  if (!fuso || fuso === "UTC" || !fusoEsistente(fuso)) return inFuso(ms, "UTC");
  return inFuso(ms, fuso);
}

const DURATA = /^([+-])?P(?:(\d+)W)?(?:(\d+)D)?T?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$/;

/** La durata di Android in secondi: scrive anche "P900S", senza la T che iCalendar vuole davanti a ore, minuti e secondi. */
export function durataAndroid(testo: string | null): number {
  const m = DURATA.exec(pyStrip(testo ?? "").toUpperCase());
  if (!m) throw new ErroreValore(`durata non riconosciuta: ${testo === null ? "None" : pyReprStr(testo)}`);
  const s = +(m[2] ?? 0) * 604800 + +(m[3] ?? 0) * 86400 + +(m[4] ?? 0) * 3600 + +(m[5] ?? 0) * 60 + +(m[6] ?? 0);
  return m[1] === "-" ? -s : s;
}

/** Una colonna di ricorrenza (RRULE, RDATE...) come riga iCalendar: EXDATE e RDATE possono arrivare come "TZID=Europe/Rome;2026..." Una riga che non si legge si salta, come fa Event.from_ical. */
function colonna(nome: string, valore: string | null): Riga | null {
  const v = pyStrip(valore ?? "");
  if (!v) return null;
  const i = v.lastIndexOf(";");
  try {
    return leggiRigaIcs(nome !== "RRULE" && nome !== "EXRULE" && i >= 0 ? `${nome};${v.slice(0, i)}:${v.slice(i + 1)}` : `${nome}:${v}`);
  } catch (e) {
    if (e instanceof ErroreValore) return null;
    throw e;
  }
}

function daRighe(righe: RigaAndroid[]): Componente[] {
  const perSync = new Map<string, number>();
  for (const r of righe) if (r._sync_id) perSync.set(r._sync_id, r._id);
  const annullate = new Map<number, [number, number | null][]>();
  const componenti: Componente[] = [];
  for (const r of righe) {
    const serie = r.original_id || (r.original_sync_id ? perSync.get(r.original_sync_id) : undefined);
    if (serie && r.eventStatus === ANNULLATO && r.originalInstanceTime !== null) {
      const lista = annullate.get(serie);
      if (lista) lista.push([r.originalInstanceTime, r.originalAllDay]);
      else annullate.set(serie, [[r.originalInstanceTime, r.originalAllDay]]);
      continue;
    }
    if (r.eventStatus === ANNULLATO || r.dtstart === null) continue;
    const giorno = Boolean(r.allDay);
    const fuso = r.eventTimezone;
    let rdate: Rdate[] = [];
    let exdate: Tempo[] = [];
    let regole: string[] = [];
    let haRegole = false;
    const problemi: Error[] = [];
    if (!serie) {
      const rr = colonna("RRULE", r.rrule);
      if (rr) regole = [regolaPerPython(rr.valore)];
      const rd = colonna("RDATE", r.rdate);
      colonna("EXRULE", r.exrule);
      const ex = colonna("EXDATE", r.exdate);
      // una proprietà che non si legge resta rotta e l'errore esce quando il calcolo la usa: prima EXDATE, poi RDATE
      if (ex) {
        const e = leggiProprieta(ex, "vDDDLists", "dts", () => leggiElencoTempi(ex.valore, parametro(ex, "TZID")));
        if (èRotta(e)) problemi.push(erroreRotta(e));
        else exdate = e;
      }
      if (rd) {
        const d = leggiProprieta(rd, "vDDDLists", "dts", () => leggiRdate(rd.valore, parametro(rd, "TZID")));
        if (èRotta(d)) problemi.push(erroreRotta(d));
        else rdate = d;
      }
      haRegole = Boolean(rr) || Boolean(r.rdate?.trim()) || Boolean(r.exdate?.trim());
    }
    componenti.push({
      uid: `a${serie || r._id}`,
      inizio: quando(r.dtstart, fuso, giorno),
      fineProp: r.dtend !== null ? quando(r.dtend, r.eventEndTimezone || fuso, giorno) : null,
      durata: r.dtend === null && r.duration ? durataAndroid(r.duration) : null,
      recId: serie && r.originalInstanceTime !== null ? quando(r.originalInstanceTime, fuso, Boolean(r.originalAllDay)) : null,
      recRange: false,
      sequenza: -1,
      exdate,
      rdate,
      rrule: regole,
      haRegole,
      problemi,
      dati: { uid: `a${serie || r._id}`, titolo: r.title || "", luogo: r.eventLocation || "", descrizione: r.description || "", organizzatore: "" },
    });
  }
  // Le occorrenze annullate di una serie diventano EXDATE della serie
  for (const c of componenti) {
    const id = /^a(\d+)$/.exec(c.uid)?.[1];
    if (!c.rrule.length || id === undefined) continue;
    for (const [ms, giorno] of annullate.get(Number(id)) ?? []) {
      const z = c.inizio.tipo === "ora" && c.inizio.z !== "UTC" ? c.inizio.z : null;
      c.exdate.push(quando(ms, z, Boolean(giorno)));
      c.haRegole = true;
    }
  }
  return componenti;
}

/**
 * Le occorrenze degli eventi del telefono fra due date locali (AAAA-MM-GGTHH:MM:SS, senza fuso), prima di togliere i doppioni.
 * Un errore in una serie fa fallire tutto l'intervallo, come in Python.
 */
export function espandiRigheAndroid(righe: RigaAndroid[], inizio: string | Temporal.PlainDateTime, fine: string | Temporal.PlainDateTime, opzioni: OpzioniAndroid = {}): OccorrenzaAndroid[] {
  const fuso = opzioni.fuso ?? FUSO_LOCALE;
  const componenti = daRighe(righe);
  const da = ora(typeof inizio === "string" ? leggiIsoLocale(inizio) : inizio, fuso);
  const a = ora(typeof fine === "string" ? leggiIsoLocale(fine) : fine, fuso);
  return espandi(componenti, da, a).map((o) => ({
    uid: o.c.dati.uid,
    titolo: o.c.dati.titolo,
    luogo: o.c.dati.luogo,
    descrizione: o.c.dati.descrizione,
    inizio: testoNaive(inOraLocale(o.inizio, fuso, true)),
    fine: testoNaive(inOraLocale(o.fine, fuso, true)),
    tuttoGiorno: o.inizio.tipo === "data",
  }));
}
