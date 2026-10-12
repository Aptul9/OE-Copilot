import crypto from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { afterAll, describe, expect, it } from "vitest";
import { ErroreTipo, ErroreValore, espandiIcs, confrontaPy, validaIcs } from "../src/index.ts";
import { reimpostaFusi } from "../src/fusi.ts";
import { inFuso, istante, ora } from "../src/tempo.ts";
import { canonico } from "./ausiliari/confronto.ts";
import { leggiGolden } from "./ausiliari/dati.ts";

// I VTIMEZONE definiti nel calendario ICS: casi nuovi registrati con le librerie vere (icalendar 7.3.0, recurring-ical-events 3.8.2, dateutil 2.9.0,
// nell'immagine dell'agente, TZ=Europe/Rome). Ogni caso parte da una cache dei fusi vuota, come un processo nuovo, tranne quelli con "continua": quelli
// vedono i fusi che i casi prima di loro hanno lasciato (la cache dei fusi di icalendar vive quanto il processo). I casi con "primaEventi" cominciano dalle
// finestre, senza validare prima: icalendar legge i valori man mano, un VTIMEZONE dopo il suo uso conta solo dalla lettura dopo. Per ogni caso e finestra il numero di
// occorrenze e l'impronta sha256 della lista (nell'ordine di Python e ordinata), o l'errore con il suo testo; prima la validazione del calendario.
const registrato = leggiGolden<{ casi: any[]; conversioni: { caso: string; zona: string; daUtc: [string, string, number, number][]; sulMuro: [string, number, number][] }[] }>("ricorrenze-vtimezone.json");

const impronta = (v: unknown) => crypto.createHash("sha256").update(canonico(v), "utf-8").digest("hex").slice(0, 20);
const riassunto = (lista: unknown[]) => ({ n: lista.length, h: impronta(lista), hs: impronta([...lista].sort((a, b) => confrontaPy(canonico(a), canonico(b)))) });
// gli altri errori di Python (ZoneInfoNotFoundError...) qui sono tutti "?": conta il testo
const tipoErrore = (e: unknown) => (e instanceof ErroreValore ? "ValueError" : e instanceof ErroreTipo ? "TypeError" : "?");
const tipoRegistrato = (t: string) => (t === "ValueError" || t === "TypeError" ? t : "?");

function valida(testo: string) {
  try {
    validaIcs(testo);
    return { ok: true };
  } catch (e) {
    return { ok: false, valore: e instanceof ErroreValore || e instanceof ErroreTipo, errore: tipoErrore(e), messaggio: (e as Error).message };
  }
}

function finestra(testo: string, f: string[]) {
  try {
    return riassunto(espandiIcs(testo, f[0]!, f[1]!));
  } catch (e) {
    return { errore: tipoErrore(e), messaggio: (e as Error).message, valore: e instanceof ErroreValore || e instanceof ErroreTipo };
  }
}

const esiti = new Map<string, boolean>();
const categorie = [...new Set(registrato.casi.map((c) => c.categoria as string))];

describe.each(categorie)("VTIMEZONE nel calendario, categoria %s", (categoria) => {
  it.each(registrato.casi.filter((c) => c.categoria === categoria).map((c) => [c.nome, c] as const))("%s", (nome, caso) => {
    if (!caso.continua) reimpostaFusi();
    const attesi = caso.atteso;
    // con "primaEventi" nessuna validazione prima delle finestre: una richiesta di eventi su un agente appena partito
    const prima = caso.primaEventi ? null : valida(caso.testo);
    const ottenuti: Record<string, unknown> = {};
    for (const f of caso.finestre as string[][]) ottenuti[f.join("|")] = finestra(caso.testo, f);
    const v = prima ?? valida(caso.testo);
    const attesoValida = attesi.valida.ok === false || "errore" in attesi.valida ? { ok: false, valore: attesi.valida.valore, errore: tipoRegistrato(attesi.valida.errore), messaggio: attesi.valida.messaggio } : { ok: true };
    const finestreAttese = Object.fromEntries(Object.entries<any>(attesi).filter(([k]) => k !== "valida").map(([k, a]) => [k, "errore" in a ? { ...a, errore: tipoRegistrato(a.errore) } : a]));
    esiti.set(nome, isDeepStrictEqual(v, attesoValida) && isDeepStrictEqual(ottenuti, finestreAttese));
    expect({ valida: v, finestre: ottenuti }).toEqual({ valida: attesoValida, finestre: finestreAttese });
  });
});

afterAll(() => {
  const righe = categorie.map((cat) => {
    const casi = registrato.casi.filter((c) => c.categoria === cat);
    const ok = casi.filter((c) => esiti.get(c.nome)).length;
    return `${ok === casi.length ? "OK  " : "NO  "}${cat.padEnd(12)} ${String(ok).padStart(3)}/${String(casi.length).padEnd(3)} uguali a Python`;
  });
  console.log(`\nVTIMEZONE nel calendario (${[...esiti.values()].filter(Boolean).length} di ${registrato.casi.length} casi uguali alle librerie Python)\n${righe.join("\n")}`);
});

describe("VTIMEZONE, dall'istante all'ora sul muro e ritorno (come dateutil, non come zoneinfo)", () => {
  it.each(registrato.conversioni.map((c) => [c.caso, c] as const))("%s", (nome, c) => {
    reimpostaFusi();
    validaIcs(registrato.casi.find((x) => x.nome === nome).testo);
    const sec = (t: string) => Date.parse(`${t}Z`) / 1000;
    const daUtc = c.daUtc.map(([utc]) => {
      const o = inFuso(sec(utc) * 1000, c.zona);
      const muro = o.w.toString({ smallestUnit: "second" });
      return [utc, muro, o.fold, sec(muro) - istante(o) / 1000];
    });
    expect(daUtc).toEqual(c.daUtc);
    const sulMuro = c.sulMuro.map(([muro, fold]) => [muro, fold, sec(muro) - istante(ora(Temporal.PlainDateTime.from(muro), c.zona, fold as 0 | 1)) / 1000]);
    expect(sulMuro).toEqual(c.sulMuro);
  });
});