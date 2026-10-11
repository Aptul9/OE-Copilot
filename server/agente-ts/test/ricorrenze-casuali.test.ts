import { describe, expect, it } from "vitest";
import { esitoCaso } from "./ausiliari/confronto";
import { leggiGolden } from "./ausiliari/dati";

// Serie e calendari generati a caso (seme fisso: righe di calendar.db con serie, esclusioni, aggiunte, occorrenze annullate e
// spostate, fusi diversi, cambi dell'ora; testi ICS con X-WR-TIMEZONE, orari UTC e senza fuso, giorni interi, RECURRENCE-ID), registrati
// con le librerie Python in Linux. È un campione di quelli che hanno almeno un evento nelle loro finestre.
const casuali = leggiGolden<{ seme: number; casi: any[] }>("ricorrenze-casuali.json");

describe(`serie casuali (seme ${casuali.seme}), uguali a Python`, () => {
  for (const tipo of ["android", "ics"]) {
    const casi = casuali.casi.filter((c) => c.tipo === tipo);
    it(`${tipo}: ${casi.length} casi`, () => {
      const diversi = casi.map((c) => [c.nome, esitoCaso(c)] as const).filter(([, e]) => e.stato !== "uguale");
      expect(diversi.map(([nome, e]) => `${nome}: ${e.dettaglio}`)).toEqual([]);
    });
  }
});
