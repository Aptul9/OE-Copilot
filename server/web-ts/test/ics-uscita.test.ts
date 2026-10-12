import crypto from "node:crypto";
import { describe, expect, it } from "vitest";
import { type EventoUscita, costruisciIcs, giornoIcs, istanteDaRoma, istanteIcs, piega, type Pubblicazione, testoIcs, uidCondiviso } from "@/lib/ics-uscita";
import { CORPUS, TUTTO_IL_CORPUS } from "./agente-condivisione";

// Il testo di un calendario condiviso (lib/ics-uscita.ts): gli a capo, la piegatura delle righe, l'ora UTC (anche a cavallo dei cambi d'ora), i giorni interi,
// gli identificativi, e il livello "occupato" che non lascia passare niente oltre agli orari.

const SALE = "00112233445566778899aabbccddeeff";
const ora = (a: number, m: number, g: number, h: number, mi = 0, s = 0) => ({ a, m, g, h, mi, s, us: 0 });
const utc = (a: number, m: number, g: number, h: number, mi = 0) => Date.UTC(a, m - 1, g, h, mi);
const pub = (p: Partial<Pubblicazione> = {}): Pubblicazione => ({ nome: "Calendario di prova", sale: SALE, livello: "titoli", stamp: utc(2026, 10, 12, 10), eventi: [], ...p });
const righe = (testo: string) => testo.replace(/\r\n[ ]/g, "").split("\r\n");
const dispiega = (testo: string) => testo.replace(/\r\n /g, "");
const EVENTO: Extract<EventoUscita, { tipo: "ora" }> = { tipo: "ora", inizio: utc(2026, 10, 13, 7), fine: utc(2026, 10, 13, 8), titolo: "Titolo", luogo: "Luogo" };

describe("testoIcs: i valori di tipo TEXT", () => {
  it("protegge barra rovesciata, punto e virgola e virgola", () => {
    expect(testoIcs("a\\b;c,d")).toBe("a\\\\b\\;c\\,d");
  });
  it("ogni a capo (CRLF, CR, LF, separatori di riga e di paragrafo, NEL) diventa \\n", () => {
    expect(testoIcs("uno\r\ndue\rtre\nquattro\u2028cinque\u2029sei\u0085sette")).toBe("uno\\ndue\\ntre\\nquattro\\ncinque\\nsei\\nsette");
  });
  it("toglie i caratteri di controllo e cambia la tabulazione in uno spazio", () => {
    expect(testoIcs("a\u0000b\u0007c\u001fd\u007fe\u009ff\tg")).toBe("abcdef g");
  });
  it("lascia stare accenti, simboli ed emoji", () => {
    expect(testoIcs("Città è più €5 𝄞")).toBe("Città è più €5 𝄞");
  });
  it("una coppia surrogata spezzata diventa il carattere di sostituzione, non un testo che UTF-8 non sa scrivere", () => {
    expect(testoIcs("a\ud83dz")).toBe("a\ufffdz");
  });
});

describe("piega: righe di al più 75 ottetti", () => {
  const ottetti = (riga: string) => Buffer.byteLength(riga, "utf8");
  const fisiche = (testo: string) => piega(testo).split("\r\n");

  it("una riga corta resta com'è", () => {
    expect(piega("SUMMARY:corto")).toBe("SUMMARY:corto");
  });
  it("75 ottetti esatti non si piegano, 76 sì", () => {
    expect(fisiche("x".repeat(75))).toHaveLength(1);
    const p = fisiche("x".repeat(76));
    expect(p).toHaveLength(2);
    expect(p[1]).toBe(" x");
  });
  it("le righe di continuazione cominciano con uno spazio e non superano i 75 ottetti spazio compreso", () => {
    const p = fisiche("SUMMARY:" + "abcdefghij".repeat(30));
    expect(p.length).toBeGreaterThan(3);
    for (const [i, r] of p.entries()) {
      expect(ottetti(r)).toBeLessThanOrEqual(75);
      if (i > 0) expect(r.startsWith(" ")).toBe(true);
    }
  });
  it("con testo a più ottetti per carattere (è 2, € 3, emoji 4) non spezza mai un carattere e rimettendo insieme si ha il testo di prima", () => {
    for (const unita of ["è", "€", "𝄞", "aè€𝄞", "日本語のテキスト"]) {
      for (let n = 20; n < 90; n += 7) {
        const riga = "SUMMARY:" + unita.repeat(n);
        const p = fisiche(riga);
        for (const r of p) {
          expect(ottetti(r), `${unita} x ${n}`).toBeLessThanOrEqual(75);
          // ogni riga fisica è UTF-8 valido da sola: niente carattere di sostituzione dove non c'era
          expect(Buffer.from(r, "utf8").toString("utf8")).toBe(r);
          expect(r).not.toContain("\ufffd");
        }
        expect(p.join("\r\n").replace(/\r\n /g, "")).toBe(riga);
      }
    }
  });
});

describe("istanteDaRoma e istanteIcs: ora di Roma in UTC", () => {
  it("ora solare (+1) e ora legale (+2)", () => {
    expect(istanteIcs(istanteDaRoma(ora(2026, 1, 15, 10)))).toBe("20260115T090000Z");
    expect(istanteIcs(istanteDaRoma(ora(2026, 7, 15, 10)))).toBe("20260715T080000Z");
  });

  it("l'ultima domenica di ottobre 2026 (25/10, alle 3 si torna indietro alle 2): prima, durante e dopo", () => {
    expect(istanteIcs(istanteDaRoma(ora(2026, 10, 24, 20)))).toBe("20261024T180000Z"); // sabato, ancora legale
    expect(istanteIcs(istanteDaRoma(ora(2026, 10, 25, 1, 30)))).toBe("20261024T233000Z");
    expect(istanteIcs(istanteDaRoma(ora(2026, 10, 25, 2, 30)))).toBe("20261025T003000Z"); // l'ora che c'è due volte: la prima
    expect(istanteIcs(istanteDaRoma(ora(2026, 10, 25, 3, 0)))).toBe("20261025T020000Z"); // dopo il cambio: solare
    expect(istanteIcs(istanteDaRoma(ora(2026, 10, 25, 3, 30)))).toBe("20261025T023000Z");
    expect(istanteIcs(istanteDaRoma(ora(2026, 10, 26, 20)))).toBe("20261026T190000Z"); // lunedì, solare
  });

  it("la stessa ora di Roma il giorno prima e il giorno dopo il cambio dà ore UTC che distano un'ora in più", () => {
    const prima = istanteDaRoma(ora(2026, 10, 24, 20));
    const dopo = istanteDaRoma(ora(2026, 10, 26, 20));
    expect((dopo - prima) / 3_600_000).toBe(49);
  });

  it("l'ultima domenica di marzo 2026 (29/03, alle 2 si va avanti alle 3): l'ora che non c'è vale con lo scarto di prima", () => {
    expect(istanteIcs(istanteDaRoma(ora(2026, 3, 29, 1, 30)))).toBe("20260329T003000Z");
    expect(istanteIcs(istanteDaRoma(ora(2026, 3, 29, 2, 30)))).toBe("20260329T013000Z");
    expect(istanteIcs(istanteDaRoma(ora(2026, 3, 29, 3, 30)))).toBe("20260329T013000Z");
    expect(istanteIcs(istanteDaRoma(ora(2026, 3, 29, 12)))).toBe("20260329T100000Z");
  });

  it("un evento a cavallo del cambio d'ora dura quanto dura davvero: 1:30 e 4:30 del 25/10 sono quattro ore", () => {
    expect((istanteDaRoma(ora(2026, 10, 25, 4, 30)) - istanteDaRoma(ora(2026, 10, 25, 1, 30))) / 3_600_000).toBe(4);
  });

  it("giornoIcs toglie i trattini", () => {
    expect(giornoIcs("2026-10-30")).toBe("20261030");
  });
});

describe("costruisciIcs: la forma del calendario", () => {
  it("l'intestazione: versione, prodotto, metodo, nome, la rilettura ogni 30 minuti e il segno di OE Copilot", () => {
    const r = righe(costruisciIcs(pub({ nome: "Calendario di Antonio" })));
    expect(r.slice(0, 9)).toEqual([
      "BEGIN:VCALENDAR",
      "VERSION:2.0",
      "PRODID:-//OE Copilot//Condivisione//IT",
      "CALSCALE:GREGORIAN",
      "METHOD:PUBLISH",
      "X-WR-CALNAME:Calendario di Antonio",
      "X-PUBLISHED-TTL:PT30M",
      "REFRESH-INTERVAL;VALUE=DURATION:PT30M",
      "X-OE-COPILOT-SHARE:1",
    ]);
    expect(r.slice(-2)).toEqual(["END:VCALENDAR", ""]);
  });

  it("ogni riga finisce con CRLF e non c'è nessun LF isolato, nemmeno con un titolo che ne contiene", () => {
    const testo = costruisciIcs(pub({ eventi: [{ ...EVENTO, titolo: "a\nb\r\nc", luogo: "x\ry" }] }));
    expect(testo.endsWith("\r\n")).toBe(true);
    expect(testo.replace(/\r\n/g, "")).not.toMatch(/[\r\n]/);
    expect(testo.split("\r\n").every((r) => Buffer.byteLength(r, "utf8") <= 75)).toBe(true);
  });

  it("il nome del calendario si protegge e si piega come ogni valore", () => {
    const nome = "Calendario; di, prova \\ con è e € " + "lungo ".repeat(20);
    const testo = costruisciIcs(pub({ nome }));
    const riga = righe(testo).find((r) => r.startsWith("X-WR-CALNAME:"));
    expect(riga).toBe("X-WR-CALNAME:" + testoIcs(nome));
    expect(testo.split("\r\n").every((r) => Buffer.byteLength(r, "utf8") <= 75)).toBe(true);
  });

  it("un evento con titolo e luogo: orari UTC con la Z, titolo e luogo protetti, DTSTAMP, TRANSP, STATUS", () => {
    const testo = costruisciIcs(pub({ eventi: [{ tipo: "ora", inizio: istanteDaRoma(ora(2026, 10, 25, 2, 30)), fine: istanteDaRoma(ora(2026, 10, 25, 3, 30)), titolo: "Cena; con, amici", luogo: "Trattoria \"Da Mario\", via Roma 1" }] }));
    const r = righe(testo);
    const i = r.indexOf("BEGIN:VEVENT");
    expect(r.slice(i + 2)).toEqual([
      "DTSTAMP:20261012T100000Z",
      "DTSTART:20261025T003000Z",
      "DTEND:20261025T023000Z",
      "SUMMARY:Cena\\; con\\, amici",
      "LOCATION:Trattoria \"Da Mario\"\\, via Roma 1",
      "TRANSP:OPAQUE",
      "STATUS:CONFIRMED",
      "END:VEVENT",
      "END:VCALENDAR",
      "",
    ]);
  });

  it("un titolo lungo con testo non ASCII si piega e si rilegge uguale", () => {
    const titolo = "Riunione di verifica dell'avanzamento è più lunga di così: " + "perché sì, €100; 𝄞 ".repeat(12);
    const testo = costruisciIcs(pub({ eventi: [{ ...EVENTO, titolo }] }));
    expect(testo.split("\r\n").every((r) => Buffer.byteLength(r, "utf8") <= 75)).toBe(true);
    expect(righe(testo).find((r) => r.startsWith("SUMMARY:"))).toBe("SUMMARY:" + testoIcs(titolo));
  });

  it("un giorno intero: date senza ora, la fine esclusa, e non occupa l'orario", () => {
    const testo = costruisciIcs(pub({ eventi: [{ tipo: "giorno", inizio: "2026-10-30", fine: "2026-10-31", titolo: "Ferie", luogo: "" }] }));
    const r = righe(testo);
    expect(r).toContain("DTSTART;VALUE=DATE:20261030");
    expect(r).toContain("DTEND;VALUE=DATE:20261031");
    expect(r).toContain("TRANSP:TRANSPARENT");
    expect(r.some((x) => x.startsWith("LOCATION"))).toBe(false);
  });

  it("senza durata la fine si omette (un evento di durata zero per iCalendar)", () => {
    const r = righe(costruisciIcs(pub({ eventi: [{ ...EVENTO, fine: EVENTO.inizio }] })));
    expect(r.some((x) => x.startsWith("DTEND"))).toBe(false);
    expect(r.some((x) => x.startsWith("DTSTART:"))).toBe(true);
  });

  it("niente di quello che iCalendar chiamerebbe dettagli: né descrizione, né partecipanti, né organizzatore, né indirizzi, né categorie", () => {
    const testo = costruisciIcs(pub({ eventi: [EVENTO] }));
    for (const chiave of ["DESCRIPTION", "ATTENDEE", "ORGANIZER", "URL", "CATEGORIES", "CLASS", "COMMENT", "CONTACT", "ATTACH", "CONFERENCE", "X-MICROSOFT", "X-GOOGLE"]) {
      expect(testo, chiave).not.toContain(chiave);
    }
  });
});

describe("uidCondiviso e gli identificativi degli eventi", () => {
  const uids = (testo: string) => righe(testo).filter((r) => r.startsWith("UID:")).map((r) => r.slice(4));

  it("ha la forma di 32 cifre esadecimali, una chiocciola e un nome che non dice niente", () => {
    expect(uidCondiviso(SALE, "a", 1)).toMatch(/^[0-9a-f]{32}@oe-copilot$/);
  });

  it("è lo stesso per lo stesso evento, a ogni lettura, e cambia con il sale e con l'orario", () => {
    const a = costruisciIcs(pub({ livello: "occupato", eventi: [EVENTO] }));
    const b = costruisciIcs(pub({ livello: "occupato", eventi: [EVENTO], stamp: utc(2026, 10, 12, 11) }));
    expect(uids(a)).toEqual(uids(b));
    const altroSale = costruisciIcs(pub({ livello: "occupato", sale: "ffeeddccbbaa99887766554433221100", eventi: [EVENTO] }));
    expect(uids(altroSale)[0]).not.toBe(uids(a)[0]);
    const altraOra = costruisciIcs(pub({ livello: "occupato", eventi: [{ ...EVENTO, fine: EVENTO.fine + 60_000 }] }));
    expect(uids(altraOra)[0]).not.toBe(uids(a)[0]);
  });

  it("non rivela niente: non contiene l'orario né il titolo, e non è l'impronta semplice dell'orario (chi non ha il sale non può verificarlo)", () => {
    const e = EVENTO;
    const uid = uids(costruisciIcs(pub({ livello: "occupato", eventi: [EVENTO] })))[0];
    for (const pezzo of [String(e.inizio), String(e.fine), "20261013", "0700", "Titolo", "Luogo"]) expect(uid).not.toContain(pezzo);
    for (const prova of [`${e.inizio}|${e.fine}`, ["ora", e.inizio, e.fine].join("\u0001"), `ora\u0001${e.inizio}\u0001${e.fine}\u00010`]) {
      for (const f of ["sha256", "sha1", "md5"]) expect(uid.slice(0, 32)).not.toBe(crypto.createHash(f).update(prova).digest("hex").slice(0, 32));
    }
  });

  it("mille eventi diversi hanno mille identificativi diversi", () => {
    const eventi: EventoUscita[] = Array.from({ length: 1000 }, (_, i) => ({ tipo: "ora", inizio: utc(2026, 10, 13, 7) + i * 900_000, fine: utc(2026, 10, 13, 7) + i * 900_000 + 600_000, titolo: "", luogo: "" }));
    expect(new Set(uids(costruisciIcs(pub({ livello: "occupato", eventi })))).size).toBe(1000);
  });

  it("con i titoli, due eventi diversi alla stessa ora hanno identificativi diversi, e due uguali pure (l'ordine li distingue)", () => {
    const uno = { ...EVENTO, titolo: "Uno" };
    const due = { ...EVENTO, titolo: "Due" };
    const u = uids(costruisciIcs(pub({ eventi: [uno, due, uno] })));
    expect(new Set(u).size).toBe(3);
    expect(uids(costruisciIcs(pub({ eventi: [uno, due, uno] })))).toEqual(u);
  });
});

describe("il livello occupato: niente oltre agli orari, qualunque cosa arrivi", () => {
  // eventi con tutto quello che non deve uscire, passati direttamente alla funzione
  const eventi: EventoUscita[] = CORPUS.titoli.flatMap((titolo, i): EventoUscita[] => [
    { tipo: "ora", inizio: utc(2026, 10, 13, 7 + i), fine: utc(2026, 10, 13, 7 + i, 45), titolo, luogo: `${CORPUS.luoghi[i % CORPUS.luoghi.length]} ${CORPUS.conferenze[i % CORPUS.conferenze.length]} ${CORPUS.posta[i % CORPUS.posta.length]}` },
    { tipo: "giorno", inizio: "2026-10-20", fine: "2026-10-21", titolo: `${titolo} ${CORPUS.organizzatori[i % 2]}`, luogo: CORPUS.aziende[i % 2] },
  ]);

  it("ogni evento si chiama Occupato, senza luogo, e nel testo non c'è nessuna delle stringhe private", () => {
    const testo = costruisciIcs(pub({ livello: "occupato", nome: "Occupato", eventi }));
    const visibile = [testo, dispiega(testo), testo.toLowerCase()];
    for (const privata of TUTTO_IL_CORPUS) {
      for (const forma of [privata, testoIcs(privata), privata.toLowerCase()]) for (const t of visibile) expect(t, forma).not.toContain(forma);
    }
    const r = righe(testo);
    expect(r.filter((x) => x.startsWith("SUMMARY:"))).toEqual(Array(eventi.length).fill("SUMMARY:Occupato"));
    expect(r.some((x) => x.startsWith("LOCATION"))).toBe(false);
  });

  it("le sole proprietà di un evento sono UID, DTSTAMP, DTSTART, DTEND, SUMMARY, TRANSP e STATUS", () => {
    const testo = costruisciIcs(pub({ livello: "occupato", eventi }));
    const proprieta = new Set(righe(testo).filter((x) => x && !/^(BEGIN|END):/.test(x)).map((x) => x.split(/[:;]/)[0]));
    expect([...proprieta].sort()).toEqual(
      ["CALSCALE", "DTEND", "DTSTAMP", "DTSTART", "METHOD", "PRODID", "REFRESH-INTERVAL", "STATUS", "SUMMARY", "TRANSP", "UID", "VERSION", "X-OE-COPILOT-SHARE", "X-PUBLISHED-TTL", "X-WR-CALNAME"].sort(),
    );
  });

  it("nel livello con i titoli, invece, titolo e luogo ci sono (è quello che il livello dice)", () => {
    const testo = costruisciIcs(pub({ livello: "titoli", eventi }));
    expect(dispiega(testo)).toContain("SUMMARY:" + testoIcs(CORPUS.titoli[0]));
    expect(dispiega(testo)).toContain("LOCATION:");
  });
});
