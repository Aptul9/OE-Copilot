import { describe, expect, it } from "vitest";
import { daIstante, daSecondi, ErroreData, iso, isoSecondi, leggiNaive, naiveLocale, oraHHMM, parseBound, piuGiorni } from "@/lib/core/fuso";

// Le date locali dell'app sono ora di Roma senza scarto, come i datetime del servizio Python: i casi sono quelli che la pagina
// manda (FullCalendar, con lo scarto) e quelli che Python rifiutava, con lo stesso testo di errore.

describe("parseBound", () => {
  it("lo scarto autentico si converte nell'ora di Roma, d'estate e d'inverno", () => {
    expect(iso(parseBound("2026-07-20T00:00:00+02:00"))).toBe("2026-07-20T00:00:00");
    expect(iso(parseBound("2026-11-02T00:00:00+01:00"))).toBe("2026-11-02T00:00:00");
    expect(iso(parseBound("2026-10-04T22:00:00Z"))).toBe("2026-10-05T00:00:00"); // legale: UTC+2
    expect(iso(parseBound("2026-11-04T22:00:00Z"))).toBe("2026-11-04T23:00:00"); // solare: UTC+1
    expect(iso(parseBound("2026-10-19T00:00:00+02:00"))).toBe("2026-10-19T00:00:00");
  });

  it("scarto diverso da quello di Roma, attraverso il cambio dell'ora", () => {
    expect(iso(parseBound("2026-10-25T00:30:00+00:00"))).toBe("2026-10-25T02:30:00"); // alle 03:00 legali si torna alle 02:00 solari
    expect(iso(parseBound("2026-10-25T01:30:00+00:00"))).toBe("2026-10-25T02:30:00");
    expect(iso(parseBound("2026-03-29T00:30:00+00:00"))).toBe("2026-03-29T01:30:00");
    expect(iso(parseBound("2026-03-29T01:30:00+00:00"))).toBe("2026-03-29T03:30:00"); // 02:30 non esiste
    expect(iso(parseBound("2026-10-19T00:00:00-05:00"))).toBe("2026-10-19T07:00:00");
  });

  it("senza scarto vale com'è; la data sola è mezzanotte; i secondi e la frazione si tengono", () => {
    expect(iso(parseBound("2026-10-19T09:30:00"))).toBe("2026-10-19T09:30:00");
    expect(iso(parseBound("  2026-10-19  "))).toBe("2026-10-19T00:00:00");
    expect(iso(parseBound("2026-10-19T09:30"))).toBe("2026-10-19T09:30:00");
    expect(iso(parseBound("2026-10-19 09:30:15"))).toBe("2026-10-19T09:30:15");
    expect(iso(parseBound("2026-10-19T09:30:15.5"))).toBe("2026-10-19T09:30:15.500000");
    expect(iso(parseBound("2026-10-19T09:30:15.123456789"))).toBe("2026-10-19T09:30:15.123456");
    expect(iso(parseBound("20261019T093015"))).toBe("2026-10-19T09:30:15");
  });

  it("il testo non valido è rifiutato con il messaggio di Python", () => {
    const messaggio = (s: unknown) => {
      try {
        parseBound(s as string);
      } catch (e) {
        expect(e).toBeInstanceOf(ErroreData);
        return (e as Error).message;
      }
    };
    expect(messaggio("ieri")).toBe("Invalid isoformat string: 'ieri'");
    expect(messaggio("")).toBe("Invalid isoformat string: ''");
    expect(messaggio("2026-13-01T00:00:00")).toBe("month must be in 1..12");
    expect(messaggio("2026-02-30T00:00:00")).toBe("day is out of range for month");
    expect(messaggio("2026-10-19T24:00:00")).toBe("hour must be in 0..23");
    expect(messaggio("2026-10-19T10:60:00")).toBe("minute must be in 0..59");
    expect(messaggio("2026-10-19T10:00:60")).toBe("second must be in 0..59");
    expect(messaggio(null)).toBe("'NoneType' object has no attribute 'strip'");
  });
});

describe("altre forme", () => {
  it("leggiNaive rifiuta lo scarto, come il confronto fra un'ora locale e una con lo scarto in Python", () => {
    expect(iso(leggiNaive("2026-10-21T08:30:00"))).toBe("2026-10-21T08:30:00");
    expect(() => leggiNaive("2026-10-21T08:30:00+02:00")).toThrow("can't compare offset-naive and offset-aware datetimes");
  });

  it("naiveLocale converte come disponibilita._naive, e ogni Z diventa +00:00", () => {
    expect(iso(naiveLocale("2026-10-21T08:30:00Z"))).toBe("2026-10-21T10:30:00");
    expect(iso(naiveLocale(" 2026-12-21T08:30:00 "))).toBe("2026-12-21T08:30:00");
  });

  it("isoSecondi toglie i microsecondi, iso li scrive solo se ci sono", () => {
    const n = parseBound("2026-10-19T09:30:15.250000");
    expect(iso(n)).toBe("2026-10-19T09:30:15.250000");
    expect(isoSecondi(n)).toBe("2026-10-19T09:30:15");
  });

  it("piuGiorni è aritmetica senza fuso: attraversa mesi, anni e il cambio dell'ora senza spostare l'orario", () => {
    expect(iso(piuGiorni(parseBound("2026-10-19T00:00:00"), 21))).toBe("2026-11-09T00:00:00");
    expect(iso(piuGiorni(parseBound("2026-10-24T22:00:00"), 1))).toBe("2026-10-25T22:00:00");
    expect(iso(piuGiorni(parseBound("2026-12-30T00:00:00"), 3))).toBe("2027-01-02T00:00:00");
    expect(iso(piuGiorni(parseBound("2026-03-01T00:00:00"), -1))).toBe("2026-02-28T00:00:00");
  });

  it("l'ora di Roma di un istante: HH:MM e il testo di fromtimestamp", () => {
    expect(oraHHMM(Date.UTC(2023, 10, 14, 22, 13, 20))).toBe("23:13");
    expect(daSecondi(1_700_000_000)).toBe("2023-11-14T23:13:20");
    expect(daSecondi(1_788_000_000.5)).toBe("2026-08-29T12:40:00");
    expect(daIstante(Date.UTC(2026, 0, 1, 0, 0, 0))).toMatchObject({ a: 2026, m: 1, g: 1, h: 1 });
  });
});
