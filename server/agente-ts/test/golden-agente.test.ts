import { describe, expect, it } from "vitest";
import { ErroreValore, espandiIcs, validaIcs } from "../src/index";
import { type Db, type Fonte, eventiAgente, occorrenzeGrezze } from "./ausiliari/android";
import { canonico } from "./ausiliari/confronto";
import { leggiGolden, leggiTestoGolden } from "./ausiliari/dati";

// L'agente Python (server/agent) su un calendar.db inventato: la risposta intera registrata in golden/agente.json, per quattro caselle
// e cinque finestre (fra cui il giorno del cambio dell'ora e una finestra con lo scarto), contro le occorrenze del motore TypeScript
// con i doppioni tolti come li toglie fonte_android.py.
const golden = leggiGolden("agente.json");
const righe = leggiGolden("agente-righe.json");
const db: Db = { calendari: righe.calendari, eventi: righe.eventi, invitati: righe.invitati };
const fonti: Fonte[] = righe.fonti;

/** Un estremo della finestra come lo fa l'agente (agent._locale): senza scarto com'è, con lo scarto si porta all'ora locale di Roma. */
function locale(testo: string): string {
  if (!/([+-]\d{2}:\d{2}|Z)$/.test(testo)) return testo;
  return Temporal.Instant.from(testo).toZonedDateTimeISO("Europe/Rome").toPlainDateTime().toString({ smallestUnit: "second" });
}
describe("agente Python, calendar.db inventato: le risposte registrate", () => {
  for (const finestra of Object.keys(golden.eventi)) {
    it(`eventi della finestra ${finestra}`, () => {
      const [inizio, fine] = finestra.split("|").map(locale) as [string, string];
      const attesi = golden.eventi[finestra].eventi;
      const ottenuti = fonti.flatMap((f) => eventiAgente(db, f, inizio, fine) ?? []);
      expect(ottenuti.length).toBe(attesi.length);
      expect(canonico(ottenuti)).toBe(canonico(attesi));
    });
    it(`occorrenze grezze (prima dei doppioni) della finestra ${finestra}`, () => {
      const [inizio, fine] = finestra.split("|").map(locale) as [string, string];
      for (const f of fonti) {
        const registrate = righe.grezzi[f.id][finestra] as any[];
        const ottenute = occorrenzeGrezze(db, f, inizio, fine) ?? [];
        const senzaDescrizione = ottenute.map(({ descrizione: _d, ...resto }) => resto);
        const ordina = (v: any[]) => v.map(canonico).sort();
        expect(ordina(senzaDescrizione)).toEqual(ordina(registrate));
      }
    });
  }
});

describe("agente Python, calendari ICS (POST /api/ics): le risposte registrate", () => {
  const testi: Record<string, string> = {
    festivita: leggiTestoGolden("ics", "festivita.ics"),
    riunioni: leggiTestoGolden("ics", "riunioni.ics"),
    html: leggiTestoGolden("ics", "html.ics"),
    rotto: leggiTestoGolden("ics", "rotto.ics"),
    vuoto: "",
  };
  for (const [nome, registrato] of Object.entries<any>(golden.ics)) {
    if (nome === "rifiuti") continue;
    it(`${nome}: validazione`, () => {
      const atteso = registrato.valida;
      try {
        const n = validaIcs(testi[nome]!);
        expect(atteso).toEqual({ ok: true, nome: n });
      } catch (e) {
        expect(atteso.ok).toBe(false);
        expect(e instanceof ErroreValore).toBe(atteso.valore);
      }
    });
    for (const finestra of Object.keys(registrato.eventi)) {
      it(`${nome}: eventi della finestra ${finestra}`, () => {
        const [inizio, fine] = finestra.split("|").map(locale) as [string, string];
        const atteso = registrato.eventi[finestra];
        let eventi: unknown[] | null = null;
        let errore: unknown = null;
        try {
          eventi = espandiIcs(testi[nome]!, inizio, fine);
        } catch (e) {
          errore = e;
        }
        if (atteso.ok) {
          expect(errore).toBeNull();
          // nello stesso ordine di Python (le serie per UID nell'ordine del testo, le occorrenze nell'ordine delle regole)
          expect(canonico(eventi)).toBe(canonico(atteso.eventi));
        } else {
          expect(eventi).toBeNull();
          expect(errore instanceof ErroreValore).toBe(atteso.valore);
        }
      });
    }  }
});
