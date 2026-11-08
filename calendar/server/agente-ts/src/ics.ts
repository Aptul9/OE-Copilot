// Il testo iCalendar (ICS) e la sua espansione: la lettura che fa icalendar 7.3.0 (righe, parametri, date, durate, fusi, proprietà rotte),
// la correzione di X-WR-TIMEZONE di x-wr-timezone 2.0.1, e l'uscita dell'agente Python (eventi con ora locale di Roma senza fuso).
// I testi degli errori sono quelli di Python: il gateway li mostra così com'escono dall'agente.

import { cacheFusi, fusiPersonalizzati } from "./fusi.ts";
import { SPAZI_PY, pyReprStr, pyStrip } from "./pyfmt.ts";
import { type Componente, type Rdate, espandi } from "./serie.ts";
import { type RigaTz, creaZonaDaRighe } from "./vtimezone.ts";
import { WINDOWS } from "./windows.ts";
import {
  type Ora,
  type Rotta,
  type Tempo,
  ErroreAltro,
  ErroreTipo,
  ErroreValore,
  data,
  erroreRotta,
  fusoEsistente,
  inFuso,
  inOraLocale,
  istante,
  ora,
  testoNaive,
  èUtc,
} from "./tempo.ts";

export const FUSO_LOCALE = "Europe/Rome";

type ValoreParametro = string | string[];
export interface Riga {
  nome: string;
  param: Map<string, ValoreParametro>;
  /** Il valore con le sequenze di escape tolte (\n, \, \; \: \\). */
  valore: string;
  /** Il valore com'è nel file: lo leggono le proprietà che iCalendar non conosce (X-WR-CALNAME...). */
  grezzo: string;
  /** Il fuso del parametro TZID com'era quando la riga è stata letta (icalendar legge i valori subito: un VTIMEZONE più avanti nel file non conta). */
  risolto?: Risolto;
}
/** Il risultato di tzp.timezone(TZID): un nome di fuso, o null se non si trova. */
export interface Risolto {
  fuso: string | null;
}
interface Comp {
  nome: string;
  righe: Riga[];
  figli: Comp[];
}

export const parametro = (r: Riga, nome: string): string | null => {
  const v = r.param.get(nome);
  return typeof v === "string" ? v : null;
};

// ---- le righe --------------------------------------------------------------------------------------------------------------------
const NOME_TOKEN = /[\p{L}\p{N}_.-]+/gu;
/** validate_token: il nome è un solo token fatto di lettere, cifre, "_", "." e "-" (\w di Python, che è Unicode). */
function validaNome(nome: string): void {
  const m = nome.match(NOME_TOKEN);
  if (m && m.length === 1 && m[0] === nome) return;
  throw new ErroreValore(nome);
}

/** q_split: divide su un separatore fuori dalle virgolette. */
function qSplit(st: string, sep: string, maxsplit = -1): string[] {
  if (maxsplit === 0) return [st];
  const out: string[] = [];
  let cursore = 0;
  let dentro = false;
  let tagli = 0;
  for (let i = 0; i < st.length; i++) {
    const c = st[i]!;
    if (c === '"') dentro = !dentro;
    if (!dentro && c === sep) {
      out.push(st.slice(cursore, i));
      cursore = i + 1;
      tagli++;
    }
    if (i + 1 === st.length || tagli === maxsplit) {
      out.push(st.slice(cursore));
      break;
    }
  }
  return out;
}

/** _strip_ows_around_delimiters: toglie gli spazi intorno a ";" e "=" fuori dalle virgolette. */
function senzaSpaziIntorno(st: string): string {
  if (!st || (!st.includes(" ") && !st.includes("\t"))) return st;
  const out: string[] = [];
  let inSospeso: string[] = [];
  let virgolette = false;
  let protetto = false;
  let ultimoDelimitatore = false;
  const spazio = (c: string) => c === " " || c === "\t";
  const scarica = () => {
    if (!inSospeso.length) return;
    if (!ultimoDelimitatore) out.push(...inSospeso);
    inSospeso = [];
  };
  for (const c of st) {
    if (protetto) {
      scarica();
      out.push(c);
      protetto = false;
      ultimoDelimitatore = false;
      continue;
    }
    if (c === "\\" && !virgolette) {
      scarica();
      out.push(c);
      protetto = true;
      ultimoDelimitatore = false;
      continue;
    }
    if (c === '"') {
      virgolette = !virgolette;
      scarica();
      out.push(c);
      ultimoDelimitatore = false;
      continue;
    }
    if (!virgolette && spazio(c)) {
      inSospeso.push(c);
      continue;
    }
    if (!virgolette && (c === ";" || c === "=")) {
      inSospeso = [];
      while (out.length && spazio(out[out.length - 1]!)) out.pop();
      out.push(c);
      ultimoDelimitatore = true;
      continue;
    }
    scarica();
    out.push(c);
    ultimoDelimitatore = false;
  }
  if (inSospeso.length && !ultimoDelimitatore) out.push(...inSospeso);
  return pyStrip(out.join(""));
}

const escapeParametro = (v: string) => v.replaceAll("%", "%25").replaceAll("\\,", "%2C").replaceAll("\\:", "%3A").replaceAll("\\;", "%3B").replaceAll("\\\\", "%5C");
const unescapeParametro = (v: string) => v.replaceAll("%2C", ",").replaceAll("%3A", ":").replaceAll("%3B", ";").replaceAll("%5C", "\\").replaceAll("%25", "%");
const rfc6868 = (v: string) => v.replace(/\^\^|\^n|\^'/g, (m) => (m === "^^" ? "^" : m === "^n" ? "\n" : '"'));

function validaValoreParametro(v: string, tra_virgolette: boolean): void {
  if ((tra_virgolette ? /[\x00-\x08\x0a-\x1f\x7f"]/ : /[\x00-\x08\x0a-\x1f\x7f",:;]/).test(v)) throw new ErroreValore(v);
}

/** Parameters.from_ical: nome=valore;nome=valore, i valori fra virgolette o separati da virgola. */
function leggiParametri(st: string): Map<string, ValoreParametro> {
  const out = new Map<string, ValoreParametro>();
  for (const param of qSplit(st, ";")) {
    try {
      const kv = qSplit(param, "=", 1);
      if (kv.length !== 2) throw new ErroreValore(`not enough values to unpack (expected 2, got ${kv.length})`);
      const [chiave, val] = kv as [string, string];
      validaNome(chiave);
      const valori: string[] = [];
      for (const v of qSplit(val, ",")) {
        if (v.startsWith('"') && v.endsWith('"')) {
          const v2 = v.replace(/^"+|"+$/g, "");
          validaValoreParametro(v2, true);
          valori.push(rfc6868(v2));
        } else {
          validaValoreParametro(v, false);
          valori.push(rfc6868(v));
        }
      }
      out.set(chiave.toUpperCase(), !valori.length ? val : valori.length === 1 ? valori[0]! : valori);
    } catch (e) {
      if (e instanceof ErroreValore) throw new ErroreValore(`${pyReprStr(param)} is not a valid parameter string: ${e.message}`);
      throw e;
    }
  }
  return out;
}

const senzaBackslash = (v: string) => v.replace(/\\([\\,;:nN])/g, (_, c: string) => (c === "n" || c === "N" ? "\n" : c));

/** Contentline.parts(): nome, parametri e valore di una riga. Con una riga che non si legge: ValueError con il testo di Python. */
export function leggiRigaIcs(riga: string): Riga {
  try {
    let tagliaNome = -1;
    let tagliaValore = -1;
    let virgolette = false;
    let protetto = false;
    for (let i = 0; i < riga.length; i++) {
      const c = riga[i]!;
      if (c === '"' && !protetto) virgolette = !virgolette;
      else if (c === "\\" && !virgolette) {
        protetto = true;
        continue;
      } else if (!virgolette && !protetto) {
        if ((c === ":" || c === ";") && tagliaNome < 0) tagliaNome = i;
        if (c === ":" && tagliaValore < 0) tagliaValore = i;
      }
      protetto = false;
    }
    // "not value_split" di Python vale anche per l'indice 0
    if (tagliaValore <= 0) tagliaValore = riga.length;
    const nomeGrezzo = tagliaNome > 0 ? riga.slice(0, tagliaNome) : riga;
    const nome = pyStrip(nomeGrezzo).replace(/[ \t]+/g, "");
    validaNome(nome);
    if (tagliaNome <= 0 || tagliaNome + 1 === tagliaValore) throw new ErroreValore("Invalid content line");
    const testoParametri = escapeParametro(senzaSpaziIntorno(riga.slice(tagliaNome + 1, tagliaValore)));
    const param = new Map<string, ValoreParametro>();
    for (const [k, v] of leggiParametri(testoParametri)) param.set(unescapeParametro(k).toUpperCase(), Array.isArray(v) ? v.map(unescapeParametro) : unescapeParametro(v));
    const grezzo = riga.slice(tagliaValore + 1);
    return { nome: nome.toUpperCase(), param, valore: senzaBackslash(grezzo), grezzo };
  } catch (e) {
    if (e instanceof ErroreValore) throw new ErroreValore(`Content line could not be parsed into parts: '${riga}': ${e.message}`);
    throw e;
  }
}

// ---- i componenti -------------------------------------------------------------------------------------------------------------
const DOPPIO_ACCAPO = /(?:(?<!\n)\r\n|(?<![\r\n])\n)(?:\r?\n)*[ \t]/g;

/** _format_error di icalendar: la descrizione e l'inizio del testo, accorciato a 97 caratteri in tutto. */
function formatoErrore(descrizione: string, testo: string): string {
  const massimo = 100 - 3;
  const puntini = "[...]";
  const punti = Array.from(testo);
  if (descrizione.length + punti.length + puntini.length > massimo) return `${descrizione}: ${punti.slice(0, massimo - descrizione.length - puntini.length).join("")} ${puntini}`;
  return `${descrizione}: ${testo}`;
}

/** I componenti di primo livello del testo. In un VEVENT una riga che non si legge si salta (Event ignora gli errori), altrove ferma tutto. */
function leggiComponenti(testo: string): Comp[] {
  const righe = testo.replace(DOPPIO_ACCAPO, "").split(/\r?\n/).filter((r) => r !== "");
  const pila: Comp[] = [];
  const radici: Comp[] = [];
  for (const r of righe) {
    let p: Riga;
    try {
      p = leggiRigaIcs(r);
    } catch (e) {
      if (e instanceof ErroreValore && pila[pila.length - 1]?.nome === "VEVENT") continue;
      throw e;
    }
    if (p.nome === "BEGIN") pila.push({ nome: p.valore.toUpperCase(), righe: [], figli: [] });
    else if (p.nome === "END") {
      const c = pila.pop();
      if (!c) throw new ErroreValore("END encountered without an accompanying BEGIN!");
      if (pila.length) pila[pila.length - 1]!.figli.push(c);
      else radici.push(c);
      if (p.valore.toUpperCase() === "VTIMEZONE") registraVtimezone(c);
    } else {
      const sopra = pila[pila.length - 1];
      if (!sopra) {
        if (p.nome === "X-COMMENT") continue;
        throw new ErroreValore(`Property "${p.nome}" does not have a parent component.`);
      }
      sopra.righe.push(p);
      // icalendar legge i valori man mano: il fuso di un TZID è quello che si trova adesso, e in un VTIMEZONE un valore sbagliato ferma tutto
      const tzid = NOMI_DATA_ORA.has(p.nome) ? p.param.get("TZID") : undefined;
      if (typeof tzid === "string" && tzid !== "") p.risolto = { fuso: risolviFuso(tzid) };
      if (sopra.nome === "VTIMEZONE" || ((sopra.nome === "STANDARD" || sopra.nome === "DAYLIGHT") && pila[pila.length - 2]?.nome === "VTIMEZONE")) validaProprietaFuso(p);
    }
  }
  return radici;
}

// ---- valori -----------------------------------------------------------------------------------------------------------------
/** tzp.timezone: il nome come fuso, o null se non si trova (e allora l'orario resta senza fuso). Il primo nome che porta a un fuso resta nella cache. */
function risolviFuso(tzid: string): string | null {
  const pulito = tzid.replace(/^\/+|\/+$/g, "");
  const candidati = [pulito, WINDOWS[pulito] ?? ""];
  if (tzid.startsWith("/")) {
    const pezzi = pulito.split("/");
    for (let i = 1; i < pezzi.length; i++) candidati.push(pezzi.slice(i).join("/"));
  }
  candidati.push(tzid);
  for (const c of candidati) {
    if (c === "") continue;
    const fuso = cacheFusi.get(c) ?? (fusoEsistente(c) ? c : undefined);
    if (fuso !== undefined) {
      cacheFusi.set(pulito, fuso);
      return fuso;
    }
  }
  return null;
}

// ---- VTIMEZONE ---------------------------------------------------------------------------------------------------------------
const NOMI_DATA_ORA = new Set(["DTSTART", "DTEND", "RECURRENCE-ID", "DUE", "RDATE", "EXDATE"]);
const due = (n: number) => String(n).padStart(2, "0");

/** Il fuso di una riga con TZID: quello che icalendar ha trovato quando l'ha letta, o il nome da cercare. */
const tzidDi = (r: Riga): string | Risolto | null => r.risolto ?? parametro(r, "TZID");

/** vUTCOffset.from_ical, in secondi. */
function leggiScartoIcal(valore: string): number {
  const intero = (t: string) => {
    if (!/^\s*[+-]?\d+(?:_\d+)*\s*$/.test(t)) throw new ErroreValore(`Expected UTC offset, got: ${valore}`);
    return Number(t.replaceAll("_", ""));
  };
  const secondi = valore.slice(5, 7);
  const scarto = intero(valore.slice(1, 3)) * 3600 + intero(valore.slice(3, 5)) * 60 + (secondi === "" ? 0 : intero(secondi));
  if (scarto >= 86400) throw new ErroreValore(`Offset must be less than 24 hours, was ${valore}`);
  return valore.slice(0, 1) === "-" ? -scarto : scarto;
}

/** vUTCOffset.to_ical. */
function testoScarto(s: number): string {
  const a = Math.abs(s);
  return `${s < 0 ? "-" : "+"}${due(Math.floor(a / 3600))}${due(Math.floor((a % 3600) / 60))}${a % 60 ? due(a % 60) : ""}`;
}

/** Il messaggio con cui vRecur.from_ical rifiuta una regola, per le cause che si conoscono. */
function messaggioRegola(testo: string): string {
  for (const coppia of testo.split(";")) {
    const kv = coppia.split("=");
    if (kv.length !== 2) continue;
    const k = kv[0]!.toUpperCase();
    const v = kv[1]!;
    if (k === "FREQ" && !FREQUENZE.has(v.toUpperCase())) return `Expected frequency, got: ${v}`;
    if (INTERI_REGOLA.has(k)) {
      const x = v.split(",").find((e) => !/^\s*[+-]?\d+\s*$/.test(e));
      if (x !== undefined) return `invalid literal for int() with base 10: ${pyReprStr(x)}`;
    }
    if (k === "BYDAY" || k === "WKST" || k === "BYWEEKDAY") {
      const x = v.split(",").find((e) => !/^([+-]?\d{1,2})?(MO|TU|WE|TH|FR|SA|SU)$/i.test(e));
      if (x !== undefined) return `Expected weekday abbrevation, got: ${x}`;
    }
  }
  return `Error in recurrence rule: ${testo}`;
}

/** Quello che icalendar legge subito in un VTIMEZONE e nelle sue osservanze: un valore che non si legge ferma tutto (nei VEVENT no, lì la riga resta rotta). */
function validaProprietaFuso(p: Riga): void {
  switch (p.nome) {
    case "TZOFFSETFROM":
    case "TZOFFSETTO":
      leggiScartoIcal(p.valore);
      break;
    case "DTSTART":
      leggiTempo(p.valore, tzidDi(p));
      break;
    case "RRULE":
    case "EXRULE":
      if (regolaRifiutata(p.valore)) throw new ErroreValore(messaggioRegola(p.valore));
      break;
    case "RDATE":
      if (p.valore !== "") leggiRdate(p.valore, tzidDi(p));
      break;
    case "EXDATE":
      leggiElencoTempi(p.valore, tzidDi(p));
      break;
  }
}

/** canonsort_keys: i nomi canonici per primi, gli altri in ordine alfabetico; le righe dello stesso nome restano come stanno. */
function ordinate(righe: Riga[], canonici: string[]): Riga[] {
  const nomi = [...new Set(righe.map((r) => r.nome))];
  const ordine = [...canonici.filter((n) => nomi.includes(n)), ...nomi.filter((n) => !canonici.includes(n)).sort()];
  return ordine.flatMap((n) => righe.filter((r) => r.nome === n));
}

const testiParametri = (r: Riga): string[] =>
  [...r.param.keys()].sort().map((k) => {
    const v = r.param.get(k)!;
    const t = Array.isArray(v) ? v.join(",") : v;
    return `${k}=${/[,;:]/.test(t) ? `"${t}"` : t}`;
  });

/** Una riga come la riscrive icalendar. Nel secondo tentativo un DTSTART che è una data diventa la mezzanotte (create_timezone). */
function rigaTz(r: Riga, dataComeOra: boolean): RigaTz {
  const param = testiParametri(r);
  if (r.nome === "TZOFFSETFROM" || r.nome === "TZOFFSETTO") return { nome: r.nome, param, valore: testoScarto(leggiScartoIcal(r.valore)) };
  if (r.nome === "RRULE" || r.nome === "EXRULE") return { nome: r.nome, param, valore: regolaPerPython(r.valore) };
  if (r.nome === "DTSTART" && dataComeOra && /^\d{8}$/.test(r.valore)) return { nome: r.nome, param: [], valore: `${r.valore}T000000` };
  return { nome: r.nome, param, valore: r.valore };
}

/** Component.to_ical() di un VTIMEZONE, riga per riga. `secondoTentativo`: tolte le proprietà X-, le date di DTSTART diventano mezzanotte. */
function righeTz(c: Comp, secondoTentativo: boolean, figlioDiVtimezone: boolean): RigaTz[] {
  const fuori: RigaTz[] = [{ nome: "BEGIN", param: [], valore: c.nome }];
  for (const r of ordinate(c.righe, c.nome === "VTIMEZONE" ? ["TZID"] : [])) {
    if (secondoTentativo && r.nome.startsWith("X-")) continue;
    fuori.push(rigaTz(r, secondoTentativo && figlioDiVtimezone));
  }
  for (const f of c.figli) fuori.push(...righeTz(f, secondoTentativo, c.nome === "VTIMEZONE"));
  fuori.push({ nome: "END", param: [], valore: c.nome });
  return fuori;
}

/** tzp.cache_timezone_component: un VTIMEZONE che definisce un nome che il resto non conosce diventa quel fuso, per sempre (finché l'agente non riparte). */
function registraVtimezone(c: Comp): void {
  const riga = c.righe.find((r) => r.nome === "TZID");
  if (!riga) return;
  const grezzo = riga.valore;
  const id = grezzo.replace(/^\/+|\/+$/g, "");
  if (fusoEsistente(id) || fusoEsistente(grezzo) || cacheFusi.has(id)) return;
  let zona;
  try {
    zona = creaZonaDaRighe(righeTz(c, false, false));
  } catch (e) {
    if (!(e instanceof ErroreValore)) throw e;
    zona = creaZonaDaRighe(righeTz(c, true, false));
  }
  fusiPersonalizzati.set(id, zona);
  cacheFusi.set(id, id);
}

const SECONDI_VALIDI = { overflow: "reject" } as const;

/** vDatetime.from_ical. */
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

/** vDate.from_ical. */
function soloData(valore: string): Temporal.PlainDate {
  if (!/^\d{8}$/.test(valore)) throw new ErroreValore(`Wrong date format ${valore}`);
  try {
    return Temporal.PlainDate.from({ year: +valore.slice(0, 4), month: +valore.slice(4, 6), day: +valore.slice(6, 8) }, SECONDI_VALIDI);
  } catch (e) {
    throw new ErroreValore(`Wrong date format ${valore}`, { cause: e });
  }
}

const DURATA = /^([-+]?)P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/;
/** vDuration.from_ical, in secondi. */
export function leggiDurata(valore: string): number {
  const m = DURATA.exec(valore);
  if (!m) throw new ErroreValore(`Invalid iCalendar duration: ${valore}`);
  const s = +(m[2] ?? 0) * 604800 + +(m[3] ?? 0) * 86400 + +(m[4] ?? 0) * 3600 + +(m[5] ?? 0) * 60 + +(m[6] ?? 0);
  return m[1] === "-" ? -s : s;
}

type Vddd = { t: Tempo } | { durata: number } | { periodo: Rdate };

/** vDDDTypes.from_ical: dal formato si capisce se è una durata, un periodo, una data o un orario; con TZID un orario prende quel fuso (se si trova). */
function leggiVddd(ical: string, tzid: string | Risolto | null): Vddd {
  const u = ical.toUpperCase();
  if (/^(P|-P|\+P)/.test(u)) return { durata: leggiDurata(ical) };
  if (u.includes("/")) return { periodo: leggiPeriodo(ical, tzid) };
  const fuso = tzid === null ? null : typeof tzid === "string" ? (tzid ? risolviFuso(tzid) : null) : tzid.fuso;
  if (ical.length === 15 || ical.length === 16) return { t: dataOra(ical, fuso) };
  if (ical.length === 8) return { t: fuso ? ora(soloData(ical).toPlainDateTime(), fuso) : data(soloData(ical)) };
  if (ical.length === 6 || ical.length === 7) {
    const orario = ical.slice(ical.lastIndexOf(":") + 1).replace(/Z$/, "");
    if (orario.length !== 6 || !/^\d+$/.test(orario)) throw new ErroreValore(`Expected time, got: ${orario}`);
    throw new ErroreTipo("un orario senza data non è un inizio");
  }
  throw new ErroreValore(`Expected datetime, date, or time. Got: '${ical}'`);
}

/** vPeriod.from_ical: inizio e fine, o inizio e durata. */
function leggiPeriodo(ical: string, tzid: string | Risolto | null): Rdate {
  try {
    const parti = ical.split("/");
    if (parti.length !== 2) throw new ErroreValore("too many values to unpack");
    const inizio = leggiVddd(parti[0]!, tzid);
    const fine = leggiVddd(parti[1]!, tzid);
    if (!("t" in inizio)) throw new ErroreTipo("inizio del periodo");
    return { inizio: inizio.t, fineOdurata: "durata" in fine ? fine.durata : "t" in fine ? fine.t : null };
  } catch {
    throw new ErroreValore(`Expected period format, got: ${ical}`);
  }
}

/** Un valore che deve essere una data o un orario (DTSTART, DTEND, RECURRENCE-ID, un elemento di EXDATE). */
export function leggiTempo(valore: string, tzid: string | Risolto | null): Tempo {
  const v = leggiVddd(valore, tzid);
  if (!("t" in v)) throw new ErroreTipo("una durata o un periodo non sono una data");
  return v.t;
}

/** vDDDLists.from_ical: elementi separati da virgola. */
export function leggiElencoTempi(valore: string, tzid: string | Risolto | null): Tempo[] {
  return valore.split(",").map((v) => leggiTempo(v, tzid));
}

/** RDATE: date, orari o periodi separati da virgola. */
export function leggiRdate(valore: string, tzid: string | Risolto | null): Rdate[] {
  return valore.split(",").map((p) => {
    const v = leggiVddd(p, tzid);
    if ("t" in v) return { inizio: v.t, fineOdurata: null };
    if ("periodo" in v) return v.periodo;
    throw new ErroreTipo("una durata non è una data");
  });
}

// ---- dal componente al calcolo -----------------------------------------------------------------------------------------------
/** x-wr-timezone: un orario in UTC passa al fuso del calendario, uno senza fuso lo prende. */
function conFusoDelCalendario(t: Tempo, z: string): Tempo {
  if (t.tipo === "data") return t;
  if (èUtc(t)) return inFuso(istante(t), z);
  return t.z === null ? ora(t.w, z, t.fold) : t;
}

/** Il testo di una proprietà di testo (vText) come "escape": ';' ',' '\' e gli accapo, per i messaggi dei calendari con una regola rotta. */
const comeTesto = (v: string) => v.replaceAll("\\N", "\n").replaceAll("\\", "\\\\").replaceAll(";", "\\;").replaceAll(",", "\\,").replaceAll("\r\n", "\\n").replaceAll("\n", "\\n").replaceAll("\r", "\\n");

const INTERI_REGOLA = new Set(["COUNT", "INTERVAL", "BYSECOND", "BYMINUTE", "BYHOUR", "BYWEEKNO", "BYMONTHDAY", "BYYEARDAY", "BYSETPOS"]);
const FREQUENZE = new Set(["SECONDLY", "MINUTELY", "HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"]);

/** vRecur.from_ical rifiuta la regola? (una frequenza che non c'è, un intero che non lo è, un giorno che non è un giorno, un UNTIL che non è una data) */
function regolaRifiutata(testo: string): boolean {
  for (const coppia of testo.split(";")) {
    const kv = coppia.split("=");
    if (kv.length !== 2) continue;
    const k = kv[0]!.toUpperCase();
    const v = kv[1]!;
    if (k === "FREQ" && !FREQUENZE.has(v.toUpperCase())) return true;
    if (INTERI_REGOLA.has(k) && v.split(",").some((x) => !/^\s*[+-]?\d+\s*$/.test(x) || Math.abs(Number(x)) > 2147483647)) return true;
    if ((k === "BYDAY" || k === "WKST" || k === "BYWEEKDAY") && v.split(",").some((x) => !/^([+-]?\d{1,2})?(MO|TU|WE|TH|FR|SA|SU)$/i.test(x))) return true;
    if (k === "BYMONTH" && v.split(",").some((x) => !/^\d{1,2}L?$/.test(x))) return true;
    if (k === "UNTIL") {
      try {
        leggiTempo(v, null);
      } catch {
        return true;
      }
    }
  }
  return false;
}

/** Il testo di una proprietà letta in un VEVENT: il valore, o la proprietà rotta con l'errore che il calcolo troverà quando la userà. */
export function leggiProprieta<T>(r: Riga, atteso: string, attributo: string, leggi: () => T): T | Rotta {
  try {
    return leggi();
  } catch (e) {
    if (e instanceof ErroreValore || e instanceof ErroreTipo) return { nome: r.nome, atteso, attributo, errore: e.message };
    throw e;
  }
}
export const èRotta = (v: unknown): v is Rotta => typeof v === "object" && v !== null && "attributo" in v && "atteso" in v;

/** Il testo della regola come lo passa Python a dateutil: quello che iCalendar non legge resta un testo con gli escape, il resto com'è. */
const ORDINE_REGOLA = ["RSCALE", "FREQ", "UNTIL", "COUNT", "INTERVAL", "BYSECOND", "BYMINUTE", "BYHOUR", "BYDAY", "BYWEEKDAY", "BYMONTHDAY", "BYYEARDAY", "BYWEEKNO", "BYMONTH", "BYSETPOS", "WKST", "SKIP"];

/** vRecur.to_ical(): le parti nell'ordine di RFC 5545 (le altre in fondo, in ordine alfabetico), in maiuscolo, gli interi come numeri. */
function regolaCanonica(valore: string): string {
  const parti = new Map<string, string>();
  for (const coppia of valore.split(";")) {
    const kv = coppia.split("=");
    if (kv.length !== 2) continue;
    const k = kv[0]!.toUpperCase();
    const v = kv[1]!;
    if (k === "FREQ" || k === "BYDAY" || k === "WKST" || k === "BYWEEKDAY") parti.set(k, v.toUpperCase());
    else if (INTERI_REGOLA.has(k)) parti.set(k, v.split(",").map((x) => String(Number(x.trim()))).join(","));
    else if (k === "BYMONTH") parti.set(k, v.split(",").map((x) => (/^0*(\d+)(L?)$/.exec(x) ? `${Number(/^0*(\d+)/.exec(x)![1])}${x.endsWith("L") ? "L" : ""}` : x)).join(","));
    else parti.set(k, v);
  }
  const chiavi = [...ORDINE_REGOLA.filter((k) => parti.has(k)), ...[...parti.keys()].filter((k) => !ORDINE_REGOLA.includes(k)).sort()];
  return chiavi.map((k) => `${k}=${parti.get(k)}`).join(";");
}

export const regolaPerPython = (valore: string) => (regolaRifiutata(valore) ? comeTesto(valore) : regolaCanonica(valore.trim()));

function daVevento(ev: Comp, zonaCalendario: string | null, n: number): Componente {
  const prima = (nome: string) => ev.righe.find((r) => r.nome === nome);
  const tutte = (nome: string) => ev.righe.filter((r) => r.nome === nome);
  const nelFuso = (t: Tempo) => (zonaCalendario ? conFusoDelCalendario(t, zonaCalendario) : t);
  // l'ordine in cui recurring_ical_events usa le proprietà è quello in cui un errore esce: RECURRENCE-ID, DTSTART, DTEND o DURATION, EXDATE, RDATE
  const problemi: Error[] = [];
  const tempo = (r: Riga) => leggiProprieta(r, "vDDDTypes", "dt", () => nelFuso(leggiTempo(r.valore, tzidDi(r))));

  const rId = prima("RECURRENCE-ID");
  const idGrezzo = rId ? leggiProprieta(rId, "vDDDTypes", "dt", () => leggiTempo(rId.valore, tzidDi(rId))) : null;
  if (idGrezzo && èRotta(idGrezzo)) problemi.push(erroreRotta(idGrezzo));
  const rInizio = prima("DTSTART");
  let inizio: Tempo = data(Temporal.PlainDate.from("2000-01-01"));
  if (!rInizio) problemi.push(new ErroreAltro("'DTSTART'"));
  else {
    const i = tempo(rInizio);
    if (èRotta(i)) problemi.push(erroreRotta(i));
    else inizio = i;
  }
  const rFine = prima("DTEND");
  const rDurata = prima("DURATION");
  let fineProp: Tempo | null = null;
  let durata: number | null = null;
  if (rFine) {
    const f = tempo(rFine);
    if (èRotta(f)) problemi.push(erroreRotta(f));
    else fineProp = f;
  } else if (rDurata) {
    const d = leggiProprieta(rDurata, "vDDDTypes", "dt", () => {
      const v = leggiVddd(rDurata.valore, parametro(rDurata, "TZID"));
      if (!("durata" in v)) throw new ErroreTipo("una data non è una durata");
      return v.durata;
    });
    if (èRotta(d)) problemi.push(erroreRotta(d));
    else durata = d;
  }
  const exdate: Tempo[] = [];
  for (const r of tutte("EXDATE")) {
    const e = leggiProprieta(r, "vDDDLists", "dts", () => leggiElencoTempi(r.valore, tzidDi(r)).map(nelFuso));
    if (èRotta(e)) problemi.push(erroreRotta(e));
    else exdate.push(...e);
  }
  const rdate: Rdate[] = [];
  for (const r of tutte("RDATE")) {
    const e = leggiProprieta(r, "vDDDLists", "dts", () => leggiRdate(r.valore, tzidDi(r)).map((d) => (d.fineOdurata === null ? { ...d, inizio: nelFuso(d.inizio) } : d)));
    if (èRotta(e)) problemi.push(erroreRotta(e));
    else rdate.push(...e);
  }
  const sequenza = prima("SEQUENCE");
  const testo = (nome: string) => prima(nome)?.valore ?? "";
  return {
    uid: prima("UID")?.valore ?? `\u0000senza-uid-${n}`,
    inizio,
    fineProp,
    durata,
    recId: idGrezzo && !èRotta(idGrezzo) ? nelFuso(idGrezzo) : null,
    // x-wr-timezone ricrea il valore che cambia (UTC o senza fuso) e con lui perde i parametri: RANGE=THISANDFUTURE non vale più
    recRange: rId?.param.get("RANGE") === "THISANDFUTURE" && !(zonaCalendario && idGrezzo && !èRotta(idGrezzo) && idGrezzo.tipo === "ora" && (idGrezzo.z === null || èUtc(idGrezzo))),
    sequenza: sequenza && /^[+-]?\d+$/.test(sequenza.valore.trim()) ? Number(sequenza.valore) : -1,
    exdate,
    rdate,
    // una regola che iCalendar non legge resta un testo, con gli escape di un valore di testo ("\;" al posto di ";")
    rrule: [...new Set(tutte("RRULE").map((r) => regolaPerPython(r.valore)))],
    haRegole: tutte("RRULE").length + tutte("RDATE").length + tutte("EXDATE").length > 0,
    problemi,
    dati: { uid: prima("UID")?.valore ?? "", titolo: testo("SUMMARY"), luogo: testo("LOCATION"), descrizione: testo("DESCRIPTION"), organizzatore: testo("ORGANIZER") },
  };
}

function eventi(c: Comp, fuori: Comp[] = []): Comp[] {
  if (c.nome === "VEVENT") fuori.push(c);
  for (const f of c.figli) eventi(f, fuori);
  return fuori;
}

/** Calendar.from_ical: un solo componente, il primo livello del testo. */
function leggiCalendario(testo: string) {
  const radici = leggiComponenti(testo);
  if (radici.length > 1) throw new ErroreValore(formatoErrore("Found multiple components where only one is allowed", testo));
  if (radici.length < 1) throw new ErroreValore(formatoErrore("Found no components where exactly one is required", testo));
  const cal = radici[0]!;
  const prop = (nome: string) => {
    const r = cal.righe.find((x) => x.nome === nome);
    if (!r) return undefined;
    // le proprietà che iCalendar non conosce (X-WR-CALNAME...) restano com'erano nel file
    return /^X-/.test(nome) ? r.grezzo : r.valore;
  };
  return { cal, prop };
}

/** Il nome che il calendario dichiara (X-WR-CALNAME, o ""), o un errore di valore se il testo non è un calendario. */
export function validaIcs(testo: string): string {
  return leggiCalendario(testo).prop("X-WR-CALNAME") ?? "";
}

// ---- la finestra e l'uscita ----------------------------------------------------------------------------------------------------
// \w e \s di Python (re, testo Unicode) non sono quelli di JavaScript: lettere e cifre di ogni alfabeto, e gli spazi di str.isspace()
export const MEETING_URL = new RegExp(
  `https?://(?:[\\p{L}\\p{N}_.-]*teams\\.microsoft\\.com|[\\p{L}\\p{N}_.-]*zoom\\.us|meet\\.google\\.com|[\\p{L}\\p{N}_.-]*webex\\.com|meet\\.jit\\.si|[\\p{L}\\p{N}_.-]*gotomeeting\\.com)/[^${SPAZI_PY}<>"']+`,
  "iu",
);

/** Un estremo dell'intervallo (ora locale senza fuso) come lo dà astimezone(): un istante con lo scarto che Roma ha in quel momento. */
function estremoConScarto(testoLocale: string, fuso: string): Ora {
  const w = leggiIsoLocale(testoLocale);
  return ora(w, w.toZonedDateTime(fuso, { disambiguation: "compatible" }).offset, 0);
}

/** datetime.fromisoformat di un estremo senza scarto: AAAA-MM-GGTHH:MM:SS (con i microsecondi, se ci sono) o solo la data. */
export function leggiIsoLocale(testo: string): Temporal.PlainDateTime {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2})(?::(\d{2})(?::(\d{2})(?:\.(\d{1,6}))?)?)?)?$/.exec(testo.trim());
  if (!m) throw new ErroreValore(`Invalid isoformat string: ${pyReprStr(testo)}`);
  try {
    const micro = Number((m[7] ?? "").padEnd(6, "0"));
    return Temporal.PlainDateTime.from(
      { year: +m[1]!, month: +m[2]!, day: +m[3]!, hour: +(m[4] ?? 0), minute: +(m[5] ?? 0), second: +(m[6] ?? 0), millisecond: Math.floor(micro / 1000), microsecond: micro % 1000 },
      SECONDI_VALIDI,
    );
  } catch (e) {
    throw new ErroreValore(`Invalid isoformat string: ${pyReprStr(testo)}`, { cause: e });
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

/** Gli eventi del calendario nell'intervallo (ora locale senza fuso, come nell'agente Python), con le ricorrenze espanse. */
export function espandiIcs(testo: string, inizio: string, fine: string, opzioni: OpzioniIcs = {}): EventoIcs[] {
  const fuso = opzioni.fuso ?? FUSO_LOCALE;
  const { cal, prop } = leggiCalendario(testo);
  const finestraDa = estremoConScarto(inizio, fuso);
  const finestraA = estremoConScarto(fine, fuso);
  const nomeFuso = prop("X-WR-TIMEZONE");
  if (nomeFuso !== undefined && !fusoEsistente(nomeFuso)) throw new ErroreAltro(pyReprStr(`No time zone found with key ${nomeFuso}`));
  if ((prop("CALSCALE") ?? "GREGORIAN") !== "GREGORIAN") throw new ErroreValore("Only Gregorian calendars are supported.");
  const componenti = eventi(cal).map((ev, n) => daVevento(ev, nomeFuso ?? null, n));
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
