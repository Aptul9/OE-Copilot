// Il testo iCalendar (ICS) e la sua espansione: la lettura che fa icalendar 7.3.0 (righe, parametri, date, durate, fusi), la correzione di
// X-WR-TIMEZONE di x-wr-timezone 2.0.1, e l'uscita di server/agent/ics.py (eventi con ora locale di Roma senza fuso).

import { type Componente, type Rdate, espandi } from "./serie";
import { WINDOWS } from "./windows";
import {
  type Ora,
  type Tempo,
  ErroreAltro,
  ErroreValore,
  data,
  fusoEsistente,
  inFuso,
  inOraLocale,
  istante,
  ora,
  testoNaive,
  èUtc,
} from "./tempo";

export const FUSO_LOCALE = "Europe/Rome";

interface Riga {
  nome: string;
  param: Map<string, string>;
  valore: string;
}
interface Comp {
  nome: string;
  righe: Riga[];
  figli: Comp[];
}

const decodifica = (v: string) => v.replace(/\\([\s\S])/g, (_, c: string) => (c === "n" || c === "N" ? "\n" : c));

export function leggiRigaIcs(riga: string): Riga {
  let inVirgolette = false;
  let segnoPrecedente = false;
  let primoSeparatore = -1;
  let duePunti = -1;
  for (let i = 0; i < riga.length; i++) {
    const c = riga[i]!;
    if (c === '"' && !segnoPrecedente) inVirgolette = !inVirgolette;
    else if (c === "\\" && !inVirgolette) {
      segnoPrecedente = true;
      continue;
    } else if (!inVirgolette && !segnoPrecedente) {
      if ((c === ":" || c === ";") && primoSeparatore < 0) primoSeparatore = i;
      if (c === ":" && duePunti < 0) duePunti = i;
    }
    segnoPrecedente = false;
  }
  if (duePunti <= 0) duePunti = riga.length;
  const nome = (primoSeparatore > 0 ? riga.slice(0, primoSeparatore) : riga).trim().replace(/[ \t]+/g, "");
  if (!/^[A-Za-z0-9-]+$/.test(nome) || primoSeparatore <= 0) throw new ErroreValore("Content line could not be parsed into parts: " + riga.slice(0, 40));
  const param = new Map<string, string>();
  const testoParam = riga.slice(primoSeparatore + 1, duePunti);
  let corrente = "";
  const pezzi: string[] = [];
  let virgolette = false;
  for (const c of testoParam) {
    if (c === '"') virgolette = !virgolette;
    if (c === ";" && !virgolette) {
      pezzi.push(corrente);
      corrente = "";
    } else corrente += c;
  }
  pezzi.push(corrente);
  for (const p of pezzi) {
    const i = p.indexOf("=");
    if (i > 0) param.set(p.slice(0, i).trim().toUpperCase(), p.slice(i + 1).trim().replace(/^"(.*)"$/, "$1"));
  }
  // icalendar lascia com'è il valore di una proprietà che non conosce (X-WR-CALNAME...): senza togliere i backslash
  const grezzo = riga.slice(duePunti + 1);
  return { nome: nome.toUpperCase(), param, valore: /^X-/i.test(nome) ? grezzo : decodifica(grezzo) };
}

/** Contentlines + ComponentIcalParser: le righe riunite, i componenti annidati; un testo che non è un calendario è un errore di valore. */
function leggiComponenti(testo: string): Comp {
  const righe = testo.replace(/(\r?\n)+[ \t]/g, "").split(/\r?\n/).filter((r) => r !== "");
  const pila: Comp[] = [];
  const radici: Comp[] = [];
  for (const r of righe) {
    const p = leggiRigaIcs(r);
    if (p.nome === "BEGIN") pila.push({ nome: p.valore.toUpperCase(), righe: [], figli: [] });
    else if (p.nome === "END") {
      const c = pila.pop();
      if (!c) throw new ErroreValore("END encountered without an accompanying BEGIN!");
      if (pila.length) pila[pila.length - 1]!.figli.push(c);
      else radici.push(c);
    } else {
      const sopra = pila[pila.length - 1];
      if (!sopra) throw new ErroreValore("property outside of a component");
      sopra.righe.push(p);
    }
  }
  if (!radici.length) throw new ErroreValore("Found no components where exactly one is required");
  if (radici.length > 1) throw new ErroreValore("Found multiple components where only one is allowed");
  return radici[0]!;
}

// ---- valori -----------------------------------------------------------------------------------------------------------------
/** tzp.timezone: il nome come fuso, o null se non si trova (e allora l'orario resta senza fuso). */
function risolviFuso(tzid: string): string | null {
  const pulito = tzid.replace(/^\/+|\/+$/g, "");
  const candidati = [pulito, WINDOWS[pulito] ?? ""];
  if (tzid.startsWith("/")) {
    const pezzi = pulito.split("/");
    for (let i = 1; i < pezzi.length; i++) candidati.push(pezzi.slice(i).join("/"));
  }
  candidati.push(tzid);
  return candidati.find((c) => c !== "" && fusoEsistente(c)) ?? null;
}

const SECONDI_VALIDI = { overflow: "reject" } as const;

function dataOra(valore: string, fuso: string | null): Ora {
  const v = valore.slice(valore.lastIndexOf(":") + 1);
  if (v.length < 15 || v[8] !== "T" || !/^\d{8}T\d{6}/.test(v)) throw new ErroreValore(`Wrong datetime format: ${valore}`);
  let w: Temporal.PlainDateTime;
  try {
    w = Temporal.PlainDateTime.from(
      { year: +v.slice(0, 4), month: +v.slice(4, 6), day: +v.slice(6, 8), hour: +v.slice(9, 11), minute: +v.slice(11, 13), second: +v.slice(13, 15) },
      SECONDI_VALIDI,
    );
  } catch (e) {
    throw new ErroreValore(`Wrong datetime format: ${valore}`, { cause: e });
  }
  if (fuso) return ora(w, fuso);
  const coda = v.slice(15);
  if (coda === "") return ora(w);
  if (coda === "Z") return ora(w, "UTC");
  throw new ErroreValore(`Wrong datetime format: ${valore}`);
}

function soloData(valore: string): Temporal.PlainDate {
  if (!/^\d{8}/.test(valore)) throw new ErroreValore(`Wrong date format ${valore}`);
  try {
    return Temporal.PlainDate.from({ year: +valore.slice(0, 4), month: +valore.slice(4, 6), day: +valore.slice(6, 8) }, SECONDI_VALIDI);
  } catch (e) {
    throw new ErroreValore(`Wrong date format ${valore}`, { cause: e });
  }
}

/** vDDDTypes.from_ical: dal formato si capisce se è una data o un orario; con TZID un orario prende quel fuso (se si trova). */
export function leggiTempo(valore: string, tzid: string | null, tipo: string | null = null): Tempo {
  const fuso = tzid ? risolviFuso(tzid) : null;
  if (tipo === "DATE") return data(soloData(valore));
  if (tipo === "DATE-TIME") return dataOra(valore, fuso);
  if (/^[+-]?P/i.test(valore)) throw new ErroreValore(`Expected datetime, date, or time. Got: '${valore}'`);
  if (valore.length === 15 || valore.length === 16) return dataOra(valore, fuso);
  if (valore.length === 8) return fuso ? ora(soloData(valore).toPlainDateTime(), fuso) : data(soloData(valore));
  throw new ErroreValore(`Expected datetime, date, or time. Got: '${valore}'`);
}

const DURATA = /^([-+]?)P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/;
export function leggiDurata(valore: string): number {
  const m = DURATA.exec(valore);
  if (!m) throw new ErroreValore(`Invalid iCalendar duration: ${valore}`);
  const s = +(m[2] ?? 0) * 604800 + +(m[3] ?? 0) * 86400 + +(m[4] ?? 0) * 3600 + +(m[5] ?? 0) * 60 + +(m[6] ?? 0);
  return m[1] === "-" ? -s : s;
}

export function leggiElencoTempi(valore: string, tzid: string | null, tipo: string | null): Tempo[] {
  return valore.split(",").map((v) => leggiTempo(v, tzid, tipo));
}

export function leggiRdate(valore: string, tzid: string | null, tipo: string | null): Rdate[] {
  return valore.split(",").map((p) => {
    if (!p.includes("/") && tipo !== "PERIOD") return { inizio: leggiTempo(p, tzid, tipo), fineOdurata: null };
    const parti = p.split("/");
    if (parti.length !== 2) throw new ErroreValore(`Invalid period: ${p}`);
    const inizio = leggiTempo(parti[0]!, tzid);
    return { inizio, fineOdurata: /^[+-]?P/i.test(parti[1]!) ? leggiDurata(parti[1]!) : leggiTempo(parti[1]!, tzid) };
  });
}

// ---- dal componente al calcolo -----------------------------------------------------------------------------------------------
/** x-wr-timezone: un orario in UTC passa al fuso del calendario, uno senza fuso lo prende. */
function conFusoDelCalendario(t: Tempo, z: string): Tempo {
  if (t.tipo === "data") return t;
  if (èUtc(t)) return inFuso(istante(t), z);
  return t.z === null ? ora(t.w, z, t.fold) : t;
}

function daVevento(ev: Comp, zonaCalendario: string | null, n: number): Componente {
  const prima = (nome: string) => ev.righe.find((r) => r.nome === nome);
  const tutte = (nome: string) => ev.righe.filter((r) => r.nome === nome);
  const tempo = (r: Riga) => leggiTempo(r.valore, r.param.get("TZID") ?? null, r.param.get("VALUE")?.toUpperCase() ?? null);
  const nelFuso = (t: Tempo) => (zonaCalendario ? conFusoDelCalendario(t, zonaCalendario) : t);

  const rInizio = prima("DTSTART");
  if (!rInizio) throw new ErroreAltro("DTSTART");
  const inizio = nelFuso(tempo(rInizio));
  const rFine = prima("DTEND");
  const rDurata = prima("DURATION");
  const rId = prima("RECURRENCE-ID");
  const idGrezzo = rId ? tempo(rId) : null;
  const exdate: Tempo[] = [];
  for (const r of tutte("EXDATE")) exdate.push(...leggiElencoTempi(r.valore, r.param.get("TZID") ?? null, r.param.get("VALUE")?.toUpperCase() ?? null).map(nelFuso));
  const rdate: Rdate[] = [];
  for (const r of tutte("RDATE"))
    rdate.push(...leggiRdate(r.valore, r.param.get("TZID") ?? null, r.param.get("VALUE")?.toUpperCase() ?? null).map((d) => (d.fineOdurata === null ? { ...d, inizio: nelFuso(d.inizio) } : d)));
  const sequenza = prima("SEQUENCE");
  const testo = (nome: string) => prima(nome)?.valore ?? "";
  return {
    uid: prima("UID")?.valore ?? `\u0000senza-uid-${n}`,
    inizio,
    fineProp: rFine ? nelFuso(tempo(rFine)) : null,
    durata: !rFine && rDurata ? leggiDurata(rDurata.valore) : null,
    recId: idGrezzo ? nelFuso(idGrezzo) : null,
    // x-wr-timezone ricrea il valore che cambia (UTC o senza fuso) e con lui perde i parametri: RANGE=THISANDFUTURE non vale più
    recRange: rId?.param.get("RANGE") === "THISANDFUTURE" && !(zonaCalendario && idGrezzo && idGrezzo.tipo === "ora" && (idGrezzo.z === null || èUtc(idGrezzo))),
    sequenza: sequenza && /^[+-]?\d+$/.test(sequenza.valore.trim()) ? Number(sequenza.valore) : -1,
    exdate,
    rdate,
    rrule: [...new Set(tutte("RRULE").map((r) => r.valore.trim()))],
    haRegole: tutte("RRULE").length + tutte("RDATE").length + tutte("EXDATE").length > 0,
    dati: { uid: prima("UID")?.valore ?? "", titolo: testo("SUMMARY"), luogo: testo("LOCATION"), descrizione: testo("DESCRIPTION"), organizzatore: testo("ORGANIZER") },
  };
}

function eventi(c: Comp, fuori: Comp[] = []): Comp[] {
  if (c.nome === "VEVENT") fuori.push(c);
  for (const f of c.figli) eventi(f, fuori);
  return fuori;
}

function leggiCalendario(testo: string) {
  const cal = leggiComponenti(testo);
  const prop = (nome: string) => cal.righe.find((r) => r.nome === nome)?.valore;
  return { cal, prop };
}

/** Il nome che il calendario dichiara (X-WR-CALNAME, o ""), o un errore di valore se il testo non è un calendario. */
export function validaIcs(testo: string): string {
  return leggiCalendario(testo).prop("X-WR-CALNAME") ?? "";
}

// ---- la finestra e l'uscita ----------------------------------------------------------------------------------------------------
export const MEETING_URL = new RegExp(
  "https?://(?:[\\p{L}\\p{N}_.-]*teams\\.microsoft\\.com|[\\p{L}\\p{N}_.-]*zoom\\.us|meet\\.google\\.com|[\\p{L}\\p{N}_.-]*webex\\.com|meet\\.jit\\.si|[\\p{L}\\p{N}_.-]*gotomeeting\\.com)/[^\\s<>\"']+",
  "iu",
);

/** Un estremo dell'intervallo (ora locale senza fuso) come lo dà astimezone(): un istante con lo scarto che Roma ha in quel momento. */
function estremoConScarto(testoLocale: string, fuso: string): Ora {
  const w = leggiIsoLocale(testoLocale);
  return ora(w, w.toZonedDateTime(fuso, { disambiguation: "compatible" }).offset, 0);
}

/** datetime.fromisoformat di un estremo senza scarto: AAAA-MM-GGTHH:MM:SS o solo la data. */
export function leggiIsoLocale(testo: string): Temporal.PlainDateTime {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2})(?::(\d{2})(?::(\d{2}))?)?)?$/.exec(testo.trim());
  if (!m) throw new ErroreValore(`Invalid isoformat string: '${testo}'`);
  try {
    return Temporal.PlainDateTime.from({ year: +m[1]!, month: +m[2]!, day: +m[3]!, hour: +(m[4] ?? 0), minute: +(m[5] ?? 0), second: +(m[6] ?? 0) }, SECONDI_VALIDI);
  } catch (e) {
    throw new ErroreValore(`Invalid isoformat string: '${testo}'`, { cause: e });
  }
}

export interface EventoIcs {
  uid: string;
  inizio: string;
  fine: string;
  tutto_giorno: boolean;
  titolo: string;
  luogo: string;
  organizzatore: string;
  link: string | null;
}

export interface OpzioniIcs {
  /** Il fuso locale (TZ dell'agente): Europe/Rome. */
  fuso?: string;
}

/** Gli eventi del calendario nell'intervallo (ora locale senza fuso, come in server/agent/ics.py), con le ricorrenze espanse. */
export function espandiIcs(testo: string, inizio: string, fine: string, opzioni: OpzioniIcs = {}): EventoIcs[] {
  const fuso = opzioni.fuso ?? FUSO_LOCALE;
  const { cal, prop } = leggiCalendario(testo);
  const finestraDa = estremoConScarto(inizio, fuso);
  const finestraA = estremoConScarto(fine, fuso);
  const nomeFuso = prop("X-WR-TIMEZONE");
  if (nomeFuso !== undefined && !fusoEsistente(nomeFuso)) throw new ErroreAltro(`No time zone found with key ${nomeFuso}`);
  const componenti = eventi(cal).map((ev, n) => daVevento(ev, nomeFuso ?? null, n));
  if ((prop("CALSCALE") ?? "GREGORIAN") !== "GREGORIAN") throw new ErroreValore("Only Gregorian calendars are supported.");
  const out: EventoIcs[] = [];
  for (const o of espandi(componenti, finestraDa, finestraA)) {
    const ini = inOraLocale(o.inizio, fuso, false);
    const fin = inOraLocale(o.fine, fuso, false);
    const { luogo, descrizione } = o.c.dati;
    const link = MEETING_URL.exec(luogo) ?? MEETING_URL.exec(descrizione);
    out.push({
      uid: o.c.dati.uid,
      inizio: testoNaive(ini),
      fine: testoNaive(fin),
      tutto_giorno: o.inizio.tipo === "data",
      titolo: o.c.dati.titolo,
      luogo,
      organizzatore: o.c.dati.organizzatore.replaceAll("mailto:", ""),
      link: link ? link[0].replace(/[>).,;"']+$/, "") : null,
    });
  }
  return out;
}
