import crypto from "node:crypto";
import type { Livello } from "./condivisione-comune";
import { daIstante, type Naive } from "./core/fuso";

// Il calendario che l'app pubblica per un altro OE Copilot, o per qualunque programma che legge un link iCalendar: il testo di RFC 5545 scritto
// qui, senza dipendenze. Due livelli: "occupato" (solo gli orari, ogni blocco si chiama Occupato) e "titoli" (titolo e luogo di ogni evento).
// Il livello lo fa rispettare questo file, non chi lo chiama: nel livello "occupato" titolo e luogo non entrano nel testo anche se arrivano.

export const CRLF = "\r\n";
export const TESTO_OCCUPATO = "Occupato";
/** Ogni quanto chi legge il link deve rileggerlo: 30 minuti, la durata della copia che un altro OE Copilot tiene dei link ICS. */
export const RILETTURA = "PT30M";
const DOMINIO_UID = "oe-copilot";

/** Un evento da pubblicare: o con gli orari (istanti, millisecondi UTC) o di un giorno intero (date AAAA-MM-GG, la fine esclusa). */
export type EventoUscita =
  | { tipo: "ora"; inizio: number; fine: number; titolo: string; luogo: string }
  | { tipo: "giorno"; inizio: string; fine: string; titolo: string; luogo: string };

export type Pubblicazione = {
  /** Come si chiama il calendario per chi lo importa (X-WR-CALNAME). */
  nome: string;
  /** Il sale del link: entra solo nell'identificativo degli eventi, e non si pubblica. */
  sale: string;
  livello: Livello;
  /** L'istante di DTSTAMP, uguale per tutti gli eventi. */
  stamp: number;
  eventi: EventoUscita[];
};

// ---- Il testo ---------------------------------------------------------------------------------------------------------------
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g;

/**
 * Un valore di tipo TEXT (RFC 5545, 3.3.11): la barra rovesciata, il punto e virgola e la virgola si proteggono con una barra, ogni
 * tipo di a capo diventa \n, i caratteri di controllo si tolgono e una coppia surrogata spezzata diventa il carattere di sostituzione.
 */
export function testoIcs(valore: string): string {
  return valore
    .toWellFormed()
    .replace(/\r\n|\r|\n|\u2028|\u2029|\u0085/g, "\n")
    .replace(/\t/g, " ")
    .replace(CONTROLLO, "")
    .replace(/\\/g, "\\\\")
    .replace(/;/g, "\\;")
    .replace(/,/g, "\\,")
    .replace(/\n/g, "\\n");
}

/**
 * Una riga logica in righe fisiche di al più 75 ottetti, CRLF escluso (RFC 5545, 3.1): gli ottetti sono quelli di UTF-8, e un carattere non
 * si spezza mai. Le righe di continuazione cominciano con uno spazio, che conta nei 75.
 */
export function piega(riga: string): string {
  const pezzi: string[] = [];
  let corrente = "";
  let ottetti = 0;
  for (const carattere of riga) {
    const n = Buffer.byteLength(carattere, "utf8");
    if (ottetti + n > 75) {
      pezzi.push(corrente);
      corrente = " ";
      ottetti = 1;
    }
    corrente += carattere;
    ottetti += n;
  }
  pezzi.push(corrente);
  return pezzi.join(CRLF);
}

// ---- Date e ore -------------------------------------------------------------------------------------------------------------
const due = (n: number) => String(n).padStart(2, "0");

/** Un istante come data e ora UTC di iCalendar: 20261025T003000Z. */
export function istanteIcs(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getUTCFullYear()).padStart(4, "0")}${due(d.getUTCMonth() + 1)}${due(d.getUTCDate())}T${due(d.getUTCHours())}${due(d.getUTCMinutes())}${due(d.getUTCSeconds())}Z`;
}

/** Una data AAAA-MM-GG come data di iCalendar: 20261025. */
export const giornoIcs = (data: string): string => data.replaceAll("-", "");

/** Lo scarto di Roma da UTC in millisecondi a un istante: 3600000 con l'ora solare, 7200000 con quella legale. */
function scartoRoma(ms: number): number {
  const n = daIstante(ms);
  return Date.UTC(n.a, n.m - 1, n.g, n.h, n.mi, n.s) - Math.floor(ms / 1000) * 1000;
}

/**
 * L'istante di un'ora di Roma scritta senza scarto, come arriva dai calendari. Un'ora che c'è due volte (l'ultima domenica di ottobre fra le
 * 2 e le 3) vale la prima, e una che non c'è (l'ultima domenica di marzo fra le 2 e le 3) vale con lo scarto di prima del salto: come datetime
 * di Python con fold=0. Si prova lo scarto di ieri e quello di domani, che sono quelli prima e dopo un cambio d'ora vicino.
 */
export function istanteDaRoma(n: Naive): number {
  const locale = Date.UTC(n.a, n.m - 1, n.g, n.h, n.mi, n.s);
  const prima = scartoRoma(locale - 86_400_000);
  const dopo = scartoRoma(locale + 86_400_000);
  const conPrima = locale - prima;
  if (scartoRoma(conPrima) === prima) return conPrima;
  const conDopo = locale - dopo;
  if (scartoRoma(conDopo) === dopo) return conDopo;
  return conPrima;
}

// ---- Gli eventi -------------------------------------------------------------------------------------------------------------
/**
 * L'identificativo di un evento: stabile per lo stesso evento nello stesso link, e senza niente di quello che c'è dietro (né l'identificativo
 * di partenza né il calendario): un HMAC del sale del link e dell'evento, tagliato. Il sale è del link e non cambia con il suo indirizzo.
 */
export function uidCondiviso(sale: string, ...parti: (string | number)[]): string {
  return `${crypto.createHmac("sha256", sale).update(parti.join("\u0000")).digest("hex").slice(0, 32)}@${DOMINIO_UID}`;
}

function evento(p: Pubblicazione, e: EventoUscita, visti: Map<string, number>): string[] {
  const occupato = p.livello === "occupato";
  const titolo = occupato ? TESTO_OCCUPATO : e.titolo;
  const luogo = occupato ? "" : e.luogo;
  // nel livello con i titoli due eventi diversi alla stessa ora non devono avere lo stesso identificativo
  const base = [e.tipo, e.inizio, e.fine, ...(occupato ? [] : [titolo, luogo])].join("\u0001");
  const n = visti.get(base) ?? 0;
  visti.set(base, n + 1);
  const righe = ["BEGIN:VEVENT", `UID:${uidCondiviso(p.sale, base, n)}`, `DTSTAMP:${istanteIcs(p.stamp)}`];
  if (e.tipo === "ora") {
    righe.push(`DTSTART:${istanteIcs(e.inizio)}`);
    // senza durata (la fine non viene dopo l'inizio) la fine si omette: per iCalendar è un evento di durata zero
    if (e.fine > e.inizio) righe.push(`DTEND:${istanteIcs(e.fine)}`);
  } else {
    righe.push(`DTSTART;VALUE=DATE:${giornoIcs(e.inizio)}`, `DTEND;VALUE=DATE:${giornoIcs(e.fine)}`);
  }
  righe.push(`SUMMARY:${testoIcs(titolo)}`);
  if (luogo) righe.push(`LOCATION:${testoIcs(luogo)}`);
  // un giorno intero non occupa l'orario, come nella disponibilità pubblica
  righe.push(`TRANSP:${e.tipo === "ora" ? "OPAQUE" : "TRANSPARENT"}`, "STATUS:CONFIRMED", "END:VEVENT");
  return righe;
}

/** Il testo del calendario, con le righe a capo CRLF e piegate a 75 ottetti. */
export function costruisciIcs(p: Pubblicazione): string {
  const righe = [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//OE Copilot//Condivisione//IT",
    "CALSCALE:GREGORIAN",
    "METHOD:PUBLISH",
    `X-WR-CALNAME:${testoIcs(p.nome)}`,
    `X-PUBLISHED-TTL:${RILETTURA}`,
    `REFRESH-INTERVAL;VALUE=DURATION:${RILETTURA}`,
    "X-OE-COPILOT-SHARE:1",
  ];
  const visti = new Map<string, number>();
  for (const e of p.eventi) righe.push(...evento(p, e, visti));
  righe.push("END:VCALENDAR");
  return righe.map(piega).join(CRLF) + CRLF;
}
