import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { archivio, chiudiArchivio, DEFAULT_CONFIG, loadConfig, modificaConfig } from "@/lib/core/archivio";
import type { Ev } from "@/lib/core/caselle";
import { naiveLocale } from "@/lib/core/fuso";
import { adesso, fissaOra } from "@/lib/core/orologio";
import {
  avviaPromemoria,
  chiaveDi,
  creaMotore,
  creaStore,
  CONSERVA_GIORNI,
  fermaPromemoria,
  istanteDaRoma,
  type Impostazioni,
  leggiImpostazioni,
  type Motore,
  PREDEFINITE,
  type Riga,
  salvaPromemoria,
  statoGlobale,
  TENTATIVI_MAX,
  testoPromemoria,
  validaImpostazioni,
} from "@/lib/core/promemoria";
import { canaleFinto, type CanaleFinto } from "@/lib/core/promemoria-canali";

// I promemoria dal server: quali eventi li hanno, una volta sola, cosa succede quando un canale fallisce, i riavvii, le impostazioni, il
// giro nel gateway. Eventi inventati, ora fissa, canali finti: nessuna rete.

const START = Date.UTC(2026, 9, 12, 13, 30, 0); // 12 ottobre 2026, 15:30:00 a Roma (ora legale)
const ORA = Date.UTC(2026, 9, 12, 13, 20, 0); // 15:20:00, dieci minuti prima
/** Fissa l'ora a `min` minuti e `sec` secondi prima di START. */
const mancano = (min: number, sec = 0) => fissaOra(START - (min * 60 + sec) * 1000);

const evento = (extra: Ev = {}): Ev => ({
  id: "prova|1|2026-10-12T15:30:00",
  account: "prova@example.test",
  alias: "Azienda di prova",
  calendar: "prova@example.test · Calendario",
  title: "Riunione di prova",
  start: "2026-10-12T15:30:00",
  end: "2026-10-12T16:00:00",
  allDay: false,
  location: "",
  organizer: "",
  online_url: null,
  ...extra,
});

let dati: string;
let log: string[];

beforeEach(() => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-promemoria-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  delete process.env.PROMEMORIA_INTERVALLO_S;
  log = [];
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation((...a: unknown[]) => void log.push(a.map(String).join(" ")));
  fissaOra(ORA);
});

afterEach(() => {
  fermaPromemoria();
  vi.useRealTimers();
  fissaOra(null);
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  delete process.env.OE_DATI;
  delete process.env.PROMEMORIA_INTERVALLO_S;
  delete (globalThis as { __oePromemoriaStato?: unknown }).__oePromemoriaStato;
  vi.restoreAllMocks();
});

/** Un motore con gli eventi, le impostazioni e i canali che la prova cambia mentre va. */
function prepara(parziali: Partial<Impostazioni> = {}, canali: CanaleFinto[] = [canaleFinto()]) {
  const p = {
    eventi: [evento()],
    problemi: 0,
    rotta: null as Error | null,
    letture: [] as [string, string][],
    imp: { ...PREDEFINITE, ...parziali } as Impostazioni,
    canali,
    motore: undefined as unknown as Motore,
    nuovo: undefined as unknown as () => Motore,
  };
  p.nuovo = () =>
    (p.motore = creaMotore({
      adesso,
      leggiEventi: async (inizio, fine) => {
        p.letture.push([inizio, fine]);
        if (p.rotta) throw p.rotta;
        return { eventi: p.eventi, problemi: p.problemi };
      },
      impostazioni: () => p.imp,
      canali: () => p.canali,
      store: creaStore(),
    }));
  p.nuovo();
  return p;
}

const righe = () => archivio().prepare("SELECT * FROM promemoria_inviati ORDER BY inizio, minuti, canale").all() as Record<string, unknown>[];

describe("quali eventi hanno un promemoria", () => {
  it("esattamente N minuti prima sì: il testo dice quanti ne mancano", async () => {
    const p = prepara();
    mancano(10);
    expect(await p.motore.giro()).toMatchObject({ letti: 1, inviati: 1 });
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 10 min, alle 15:30");
  });

  it("N più un secondo no, N più un minuto no", async () => {
    const p = prepara();
    mancano(10, 1);
    await p.motore.giro();
    mancano(11);
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    expect(righe()).toEqual([]);
  });

  it("un evento già iniziato no, nemmeno nell'istante dell'inizio; a un secondo dall'inizio sì", async () => {
    const p = prepara();
    fissaOra(START + 5 * 60_000);
    await p.motore.giro();
    fissaOra(START);
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    mancano(0, 1);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 1 min, alle 15:30");
  });

  it("all'inizio (0 minuti): parte nell'ultimo minuto prima dell'evento, una volta sola, e non prima", async () => {
    const p = prepara({ minuti: [0] });
    mancano(1, 1);
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    mancano(0, 40);
    expect(await p.motore.giro()).toMatchObject({ inviati: 1 });
    mancano(0, 10);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 1 min, alle 15:30");
    expect(righe()).toMatchObject([{ minuti: 0, esito: "inviato" }]);
  });

  it("all'inizio e 10 minuti prima: due avvisi, ognuno quando scade", async () => {
    const p = prepara({ minuti: [0, 10] });
    mancano(10);
    await p.motore.giro();
    mancano(0, 30);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(2);
    expect(righe().map((r) => r.minuti)).toEqual([0, 10]);
  });

  it("un evento di tutto il giorno non ha mai un promemoria, nemmeno a cinque minuti dalla mezzanotte", async () => {
    const p = prepara({ minuti: [10, 60, 1440] });
    p.eventi = [evento({ id: "giorno", allDay: true, start: "2026-10-12T00:00:00", end: "2026-10-13T00:00:00" })];
    fissaOra(Date.UTC(2026, 9, 11, 21, 55, 0)); // 23:55 a Roma
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    expect(p.letture).toHaveLength(1);
  });

  it("una voce nascosta (le festività di un calendario spento) non ha un promemoria", async () => {
    const p = prepara();
    p.eventi = [evento({ nascosto: true })];
    mancano(5);
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
  });

  it("un inizio che non si legge si scarta senza fermare gli altri eventi", async () => {
    const p = prepara();
    p.eventi = [evento({ id: "a", start: "boh" }), evento({ id: "b", start: undefined }), evento({ id: "c", start: "2026-13-45T99:99:99" }), evento({ id: "d" })];
    mancano(5);
    expect(await p.motore.giro()).toMatchObject({ letti: 1, inviati: 1 });
  });

  it("l'ora è quella di Roma anche con uno scarto nel testo: 13:30 UTC sono le 15:30", async () => {
    const p = prepara();
    p.eventi = [evento({ start: "2026-10-12T13:30:00Z" })];
    mancano(10);
    await p.motore.giro();
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 10 min, alle 15:30");
  });

  it("si leggono gli eventi dei prossimi minuti: da adesso al valore più grande più due minuti", async () => {
    const p = prepara({ minuti: [5, 30] });
    mancano(10);
    await p.motore.giro();
    expect(p.letture).toEqual([["2026-10-12T15:20:00", "2026-10-12T15:52:00"]]);
  });

  it("un evento senza id si riconosce lo stesso, per casella e inizio", async () => {
    const p = prepara();
    p.eventi = [evento({ id: undefined })];
    mancano(10);
    await p.motore.giro();
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(righe()[0].evento).toBe("prova@example.test|2026-10-12T15:30:00");
  });
});

describe("l'ora legale", () => {
  it("istanteDaRoma: ora legale, ora solare, ora ripetuta e ora che non esiste", () => {
    const di = (testo: string) => istanteDaRoma(naiveLocale(testo));
    expect(di("2026-10-12T15:30:00")).toBe(Date.UTC(2026, 9, 12, 13, 30));
    expect(di("2026-12-01T10:00:00")).toBe(Date.UTC(2026, 11, 1, 9, 0));
    // 25 ottobre 2026: alle 03:00 le lancette tornano alle 02:00
    expect(di("2026-10-25T03:30:00")).toBe(Date.UTC(2026, 9, 25, 2, 30));
    expect(di("2026-10-25T02:30:00")).toBe(Date.UTC(2026, 9, 25, 0, 30));
    // 28 marzo 2027: alle 02:00 le lancette vanno alle 03:00, le 02:30 non esistono e valgono come prima del salto
    expect(di("2027-03-28T01:30:00")).toBe(Date.UTC(2027, 2, 28, 0, 30));
    expect(di("2027-03-28T02:30:00")).toBe(Date.UTC(2027, 2, 28, 1, 30));
    expect(di("2027-03-28T03:30:00")).toBe(Date.UTC(2027, 2, 28, 1, 30));
  });

  it("nella notte in cui le lancette tornano indietro i minuti che mancano sono quelli veri", async () => {
    const p = prepara({ minuti: [60] });
    p.eventi = [evento({ id: "notte", start: "2026-10-25T03:30:00" })];
    // 02:30 la prima volta (ora legale): mancano 2 ore, non 1
    fissaOra(Date.UTC(2026, 9, 25, 0, 30));
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    // 02:30 la seconda volta (ora solare): ne manca una
    fissaOra(Date.UTC(2026, 9, 25, 1, 30));
    await p.motore.giro();
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 60 min, alle 03:30");
  });
});

describe("il testo", () => {
  const dettagli = { location: "Sala Rossa", online_url: "https://teams.example.test/l/riunione-di-prova", organizer: "Chi Organizza" };

  it("completo: minuti che mancano e ora, poi titolo e luogo; il link per entrare non c'è", () => {
    const ev = evento(dettagli);
    const t = testoPromemoria(ev, naiveLocale(ev.start), 10 * 60_000, "completo", START - 600_000);
    expect(t).toBe("Tra 10 min, alle 15:30\nRiunione di prova\nSala Rossa");
  });

  it("completo: senza luogo restano l'ora e il titolo; un luogo che è un link non c'è", () => {
    const a = evento();
    expect(testoPromemoria(a, naiveLocale(a.start), 600_000, "completo", ORA)).toBe("Tra 10 min, alle 15:30\nRiunione di prova");
    const b = evento({ location: "https://teams.example.test/l/x", online_url: "https://teams.example.test/l/x" });
    expect(testoPromemoria(b, naiveLocale(b.start), 600_000, "completo", ORA)).toBe("Tra 10 min, alle 15:30\nRiunione di prova");
  });

  it("minimo: solo i minuti, l'ora e l'etichetta dell'azienda; niente titolo, luogo, link, indirizzi", () => {
    const ev = evento({ ...dettagli, organizer_email: "organizzatore@example.test" });
    const t = testoPromemoria(ev, naiveLocale(ev.start), 600_000, "minimo", ORA);
    expect(t).toBe("Tra 10 min, alle 15:30\nAzienda di prova");
    for (const nascosto of ["Riunione", "Sala Rossa", "teams.example", "organizzatore", "prova@example.test"]) expect(t).not.toContain(nascosto);
    expect(testoPromemoria(evento({ alias: "" }), naiveLocale(ev.start), 600_000, "minimo", ORA)).toBe("Tra 10 min, alle 15:30");
  });

  it("i minuti sono quelli veri, arrotondati per eccesso; dalle due ore in su ore e minuti", () => {
    const ev = evento();
    const n = naiveLocale(ev.start);
    const prima = (ms: number) => testoPromemoria(ev, n, ms, "minimo", ORA).split("\n")[0];
    expect(prima(4 * 60_000 + 20_000)).toBe("Tra 5 min, alle 15:30");
    expect(prima(60_000)).toBe("Tra 1 min, alle 15:30");
    expect(prima(1)).toBe("Tra 1 min, alle 15:30");
    expect(prima(119 * 60_000)).toBe("Tra 119 min, alle 15:30");
    expect(prima(120 * 60_000)).toBe("Tra 2 ore, alle 15:30");
    expect(prima(150 * 60_000)).toBe("Tra 2 ore e 30 min, alle 15:30");
    expect(prima(61 * 60_000 + 1)).toBe("Tra 62 min, alle 15:30");
    expect(prima(181 * 60_000)).toBe("Tra 3 ore e 1 min, alle 15:30");
  });

  it("l'evento di domani o di un altro giorno lo dice", () => {
    const ev = evento({ start: "2026-10-13T09:00:00" });
    const n = naiveLocale(ev.start);
    expect(testoPromemoria(ev, n, 1440 * 60_000, "minimo", ORA).split("\n")[0]).toBe("Tra 24 ore, domani alle 09:00");
    const dopo = evento({ start: "2026-10-20T09:00:00" });
    expect(testoPromemoria(dopo, naiveLocale(dopo.start), 600_000, "minimo", ORA).split("\n")[0]).toBe("Tra 10 min, il 20/10 alle 09:00");
  });

  it("spazi e a capo dentro il titolo o il luogo diventano uno spazio solo; senza titolo c'è la dicitura di sempre", () => {
    const ev = evento({ title: "  Riunione \n  di\tprova  ", location: "Sala\r\nRossa" });
    expect(testoPromemoria(ev, naiveLocale(ev.start), 600_000, "completo", ORA).split("\n")).toEqual(["Tra 10 min, alle 15:30", "Riunione di prova", "Sala Rossa"]);
    expect(testoPromemoria(evento({ title: "" }), naiveLocale("2026-10-12T15:30:00"), 600_000, "completo", ORA)).toContain("(senza titolo)");
  });
});

describe("una volta sola", () => {
  it("lo stesso evento nello stesso giro e nei giri dopo: un messaggio", async () => {
    const p = prepara();
    mancano(10);
    await p.motore.giro();
    await p.motore.giro();
    mancano(9);
    await p.motore.giro();
    mancano(1);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(righe()).toHaveLength(1);
  });

  it("la riga ha la chiave (occorrenza, inizio, minuti, canale) e il resto che serve a leggerla", async () => {
    const p = prepara();
    mancano(10);
    await p.motore.giro();
    const [r] = righe();
    expect(r).toEqual({
      chiave: chiaveDi("prova|1|2026-10-12T15:30:00", "2026-10-12T15:30:00", 10, "finto"),
      evento: "prova|1|2026-10-12T15:30:00",
      inizio: "2026-10-12T15:30:00",
      minuti: 10,
      canale: "finto",
      inviato_il: ORA,
      esito: "inviato",
      tentativi: 1,
      errore: null,
    });
  });

  it("due valori dei minuti: un avviso per ciascuno, ognuno quando scade", async () => {
    const p = prepara({ minuti: [5, 10] });
    mancano(10);
    await p.motore.giro();
    mancano(9);
    await p.motore.giro();
    mancano(5.5);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    mancano(5);
    await p.motore.giro();
    mancano(4);
    await p.motore.giro();
    expect(p.canali[0].inviati.map((t) => t.split("\n")[0])).toEqual(["Tra 10 min, alle 15:30", "Tra 5 min, alle 15:30"]);
    expect(righe().map((r) => [r.minuti, r.esito])).toEqual([[5, "inviato"], [10, "inviato"]]);
  });

  it("primo giro in ritardo con due valori già scaduti: un messaggio solo, con i minuti veri, che li chiude tutti e due", async () => {
    const p = prepara({ minuti: [5, 10] });
    mancano(4, 20);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 5 min, alle 15:30");
    expect(righe().map((r) => [r.minuti, r.esito, r.tentativi])).toEqual([[5, "inviato", 1], [10, "inviato", 1]]);
    mancano(3);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
  });

  it("un evento spostato (inizio nuovo) si avvisa di nuovo, con la stessa identità e con una nuova", async () => {
    const p = prepara();
    mancano(10);
    await p.motore.giro();
    p.eventi = [evento({ start: "2026-10-12T15:50:00" })];
    fissaOra(START + 10 * 60_000); // 15:40
    await p.motore.giro();
    p.eventi = [evento({ id: "altro|2|2026-10-12T16:10:00", start: "2026-10-12T16:10:00" })];
    fissaOra(START + 30 * 60_000); // 16:00
    await p.motore.giro();
    expect(p.canali[0].inviati.map((t) => t.split("\n")[0])).toEqual([
      "Tra 10 min, alle 15:30",
      "Tra 10 min, alle 15:50",
      "Tra 10 min, alle 16:10",
    ]);
    expect(righe()).toHaveLength(3);
  });

  it("un evento cancellato prima del promemoria non lo ha: sparisce dalla lettura e non resta niente", async () => {
    const p = prepara();
    mancano(12);
    await p.motore.giro();
    p.eventi = [];
    mancano(10);
    await p.motore.giro();
    mancano(5);
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    expect(righe()).toEqual([]);
  });

  it("le occorrenze di una serie sono distinte: ognuna ha il suo promemoria", async () => {
    const p = prepara({ minuti: [70] });
    const serie = { title: "Riunione di prova", organizer: "" };
    p.eventi = [
      evento({ ...serie, id: "prova|a77|2026-10-12T15:30:00", start: "2026-10-12T15:30:00" }),
      evento({ ...serie, id: "prova|a77|2026-10-12T16:30:00", start: "2026-10-12T16:30:00" }),
      evento({ ...serie, id: "prova|a77|2026-10-13T15:30:00", start: "2026-10-13T15:30:00" }),
    ];
    mancano(10);
    await p.motore.giro();
    await p.motore.giro();
    expect(p.canali[0].inviati.map((t) => t.split("\n")[0])).toEqual(["Tra 10 min, alle 15:30", "Tra 70 min, alle 16:30"]);
    mancano(-50); // 16:20, la prima è finita: la seconda ha già avuto il suo
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(2);
  });

  it("più eventi vicini: uno per evento, in ordine di inizio", async () => {
    const p = prepara();
    p.eventi = [evento({ id: "b", title: "Dopo", start: "2026-10-12T15:30:00" }), evento({ id: "a", title: "Prima", start: "2026-10-12T15:25:00" })];
    mancano(10);
    await p.motore.giro();
    expect(p.canali[0].inviati.map((t) => t.split("\n")[1])).toEqual(["Prima", "Dopo"]);
  });

  it("un riavvio non rimanda niente: un motore nuovo sullo stesso database, anche con il database riaperto", async () => {
    const p = prepara();
    mancano(10);
    await p.motore.giro();
    p.nuovo();
    await p.motore.giro();
    chiudiArchivio();
    p.nuovo();
    mancano(9);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(righe()).toHaveLength(1);
  });

  it("il server fermo e poi di nuovo su prima dell'evento: parte il promemoria in ritardo, con i minuti veri", async () => {
    const p = prepara();
    // nessun giro da 30 minuti prima dell'evento fino a 4 minuti prima
    mancano(4);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(p.canali[0].inviati[0].split("\n")[0]).toBe("Tra 4 min, alle 15:30");
  });

  it("due canali: ognuno riceve il suo, e un evento già avvisato su uno non si rimanda su quello", async () => {
    const a = canaleFinto("uno");
    const b = canaleFinto("due");
    const p = prepara({}, [a]);
    mancano(10);
    await p.motore.giro();
    p.canali = [a, b];
    await p.motore.giro();
    expect(a.inviati).toHaveLength(1);
    expect(b.inviati).toHaveLength(1);
    expect(righe().map((r) => r.canale)).toEqual(["due", "uno"]);
  });
});

describe("un canale che fallisce", () => {
  const guasto = (errore = "Canale di prova non risponde"): { ok: false; errore: string } => ({ ok: false, errore });

  it("due canali, uno che fallisce: l'altro riceve, il primo non è segnato inviato e riprova al giro dopo, finché riesce", async () => {
    const buono = canaleFinto("buono");
    const rotto = canaleFinto("rotto");
    rotto.esiti = [guasto(), guasto()];
    const p = prepara({}, [buono, rotto]);
    mancano(10);
    await p.motore.giro();
    expect(buono.inviati).toHaveLength(1);
    expect(rotto.inviati).toEqual([]);
    expect(righe().map((r) => [r.canale, r.esito, r.tentativi, r.errore])).toEqual([
      ["buono", "inviato", 1, null],
      ["rotto", "riprovo", 1, "Canale di prova non risponde"],
    ]);
    mancano(9, 30);
    await p.motore.giro();
    expect(rotto.tentativi).toBe(2);
    expect(rotto.inviati).toEqual([]);
    mancano(9);
    await p.motore.giro();
    expect(rotto.inviati).toHaveLength(1);
    expect(buono.inviati).toHaveLength(1);
    expect(righe().map((r) => [r.canale, r.esito, r.tentativi, r.errore])).toEqual([
      ["buono", "inviato", 1, null],
      ["rotto", "inviato", 3, null],
    ]);
    // il testo dei tentativi in ritardo ha i minuti di quel momento
    expect(rotto.inviati[0].split("\n")[0]).toBe("Tra 9 min, alle 15:30");
  });

  it("dopo cinque tentativi falliti la riga è fallita e non si riprova più; gli altri canali non ne risentono", async () => {
    const buono = canaleFinto("buono");
    const rotto = canaleFinto("rotto");
    rotto.esiti = Array.from({ length: 20 }, () => guasto());
    const p = prepara({}, [buono, rotto]);
    for (let i = 0; i < TENTATIVI_MAX + 3; i++) {
      mancano(10, -i);
      await p.motore.giro();
    }
    expect(rotto.tentativi).toBe(TENTATIVI_MAX);
    expect(righe().find((r) => r.canale === "rotto")).toMatchObject({ esito: "fallito", tentativi: TENTATIVI_MAX, errore: "Canale di prova non risponde" });
    expect(buono.inviati).toHaveLength(1);
    // la riga fallita resta fallita anche per un motore nuovo
    p.nuovo();
    mancano(8);
    await p.motore.giro();
    expect(rotto.tentativi).toBe(TENTATIVI_MAX);
    expect(log.filter((l) => l.includes("abbandonati")).join("\n")).toBe("promemoria: 1 promemoria abbandonati dopo 5 tentativi sul canale rotto: Canale di prova non risponde");
  });

  it("un invio fallito ferma quel canale per il giro: più eventi, un tentativo solo, dall'evento più vicino", async () => {
    const buono = canaleFinto("buono");
    const rotto = canaleFinto("rotto");
    rotto.esiti = [guasto(), guasto()];
    const p = prepara({}, [buono, rotto]);
    p.eventi = [evento({ id: "a", start: "2026-10-12T15:28:00" }), evento({ id: "b", start: "2026-10-12T15:30:00" })];
    mancano(10);
    await p.motore.giro();
    expect(buono.inviati).toHaveLength(2);
    expect(rotto.tentativi).toBe(1);
    expect(righe().filter((r) => r.canale === "rotto")).toHaveLength(1);
    await p.motore.giro();
    expect(rotto.tentativi).toBe(2);
    await p.motore.giro();
    expect(rotto.tentativi).toBe(4); // riesce il primo evento e subito dopo il secondo
    expect(rotto.inviati).toHaveLength(2);
  });

  it("429: aspetta il tempo che dice il servizio, senza altri tentativi, poi riprova", async () => {
    const c = canaleFinto("push");
    c.esiti = [{ ok: false, errore: "Il servizio limita gli invii (429)", attesaS: 30 }];
    const p = prepara({}, [c]);
    mancano(10);
    await p.motore.giro();
    expect(c.tentativi).toBe(1);
    mancano(9, 50); // 10 secondi dopo
    await p.motore.giro();
    mancano(9, 31); // 29 secondi dopo
    await p.motore.giro();
    expect(c.tentativi).toBe(1);
    mancano(9, 29); // 31 secondi dopo
    await p.motore.giro();
    expect(c.tentativi).toBe(2);
    expect(c.inviati).toHaveLength(1);
    expect(c.inviati[0].split("\n")[0]).toBe("Tra 10 min, alle 15:30");
  });

  it("il limite di attesa è 15 minuti, qualunque cosa chieda il servizio", async () => {
    const c = canaleFinto("push");
    c.esiti = [{ ok: false, errore: "Il servizio limita gli invii (429)", attesaS: 86_400 }];
    const p = prepara({ minuti: [60] }, [c]);
    mancano(60);
    await p.motore.giro();
    mancano(46); // 14 minuti dopo
    await p.motore.giro();
    expect(c.tentativi).toBe(1);
    mancano(44); // 16 minuti dopo
    await p.motore.giro();
    expect(c.tentativi).toBe(2);
    expect(c.inviati).toHaveLength(1);
  });

  it("l'attesa di un canale non ferma gli altri", async () => {
    const lento = canaleFinto("lento");
    lento.esiti = [{ ok: false, errore: "limite", attesaS: 60 }];
    const veloce = canaleFinto("veloce");
    const p = prepara({}, [lento, veloce]);
    p.eventi = [evento({ id: "a", start: "2026-10-12T15:28:00" }), evento({ id: "b", start: "2026-10-12T15:30:00" })];
    mancano(10);
    await p.motore.giro();
    expect(veloce.inviati).toHaveLength(2);
    expect(lento.tentativi).toBe(1);
  });

  it("un canale che solleva un'eccezione è un invio fallito come un altro: tentativo contato, frase fissa, il testo dell'eccezione non esce", async () => {
    const c = canaleFinto("storto");
    c.invia = async () => {
      throw new Error("TypeError con dentro Riunione di prova e prova@example.test");
    };
    const p = prepara({}, [c]);
    for (let i = 0; i < TENTATIVI_MAX + 1; i++) {
      mancano(10, -i);
      await p.motore.giro();
    }
    expect(righe()).toHaveLength(1);
    expect(righe()[0]).toMatchObject({ esito: "fallito", tentativi: TENTATIVI_MAX, errore: "invio sul canale storto non riuscito" });
    expect(log.join("\n")).not.toContain("Riunione");
    expect(log.join("\n")).not.toContain("prova@example.test");
  });

  it("un tentativo che riesce dopo un fallimento non segna niente di più: 1 riga, esito inviato, tentativi 2", async () => {
    const c = canaleFinto();
    c.esiti = [guasto()];
    const p = prepara({}, [c]);
    mancano(10);
    await p.motore.giro();
    mancano(9, 30);
    await p.motore.giro();
    expect(righe()).toHaveLength(1);
    expect(righe()[0]).toMatchObject({ esito: "inviato", tentativi: 2, errore: null });
  });
});

describe("senza canali pronti, con la lettura rotta", () => {
  it("senza un canale pronto (di norma, nessun browser iscritto): non legge, non invia, non scrive niente e non è un errore", async () => {
    const c = canaleFinto("push", false);
    const p = prepara({}, [c]);
    mancano(5);
    await p.motore.giro();
    expect(p.letture).toEqual([]);
    expect(c.tentativi).toBe(0);
    expect(p.motore.stato).toEqual({ ultimoGiro: START - 300_000, ultimoErrore: null });
    expect(archivio().prepare("SELECT name FROM sqlite_master WHERE name = 'promemoria_inviati'").all()).toEqual([]);
    expect(log.filter((l) => l.startsWith("promemoria:"))).toEqual([]);
  });

  it("un canale che diventa pronto (un browser che si iscrive) riceve dal giro dopo", async () => {
    const c = canaleFinto("push", false);
    const p = prepara({}, [c]);
    mancano(5);
    await p.motore.giro();
    c.configuratoOra = true;
    mancano(4, 30);
    await p.motore.giro();
    expect(c.inviati).toHaveLength(1);
  });

  it("lettura fallita: niente inviato, l'errore si ricorda senza il testo dell'errore, e al giro dopo riprende", async () => {
    const p = prepara();
    p.rotta = new Error("connessione rifiutata mentre leggevo Riunione di prova di prova@example.test");
    mancano(5);
    await p.motore.giro();
    expect(p.canali[0].inviati).toEqual([]);
    expect(p.motore.stato.ultimoErrore?.testo).toBe("lettura degli eventi non riuscita");
    expect(log.join("\n")).not.toContain("Riunione");
    expect(log.join("\n")).not.toContain("prova@example.test");
    p.rotta = null;
    mancano(4, 30);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
  });

  it("sorgenti con problemi: gli eventi letti partono lo stesso, e il problema si conta senza nominarlo", async () => {
    const p = prepara();
    p.problemi = 2;
    mancano(10);
    await p.motore.giro();
    expect(p.canali[0].inviati).toHaveLength(1);
    expect(p.motore.stato.ultimoErrore?.testo).toBe("la lettura degli eventi ha avuto problemi");
    expect(log.join("\n")).toContain("(2 sorgenti)");
  });

  it("l'ultimo giro è l'ora dell'ultimo giro", async () => {
    const p = prepara();
    mancano(10);
    await p.motore.giro();
    expect(p.motore.stato.ultimoGiro).toBe(START - 600_000);
    mancano(9);
    await p.motore.giro();
    expect(p.motore.stato.ultimoGiro).toBe(START - 540_000);
  });

  it("due giri non si sovrappongono: il secondo, mentre il primo legge ancora, non fa niente", async () => {
    const c = canaleFinto();
    let sblocca: () => void = () => {};
    const attesa = new Promise<void>((ok) => (sblocca = ok));
    let letture = 0;
    const m = creaMotore({
      adesso,
      leggiEventi: async () => {
        letture++;
        await attesa;
        return { eventi: [evento()], problemi: 0 };
      },
      impostazioni: () => ({ ...PREDEFINITE }),
      canali: () => [c],
      store: creaStore(),
    });
    mancano(10);
    const primo = m.giro();
    const secondo = await m.giro();
    expect(secondo).toEqual({ letti: 0, inviati: 0, falliti: 0, abbandonati: 0 });
    sblocca();
    expect((await primo).inviati).toBe(1);
    expect(letture).toBe(1);
  });

  it("un errore imprevisto in un giro non ferma i successivi", async () => {
    const c = canaleFinto();
    let rotto = true;
    const m = creaMotore({
      adesso,
      leggiEventi: async () => ({ eventi: [evento()], problemi: 0 }),
      impostazioni: () => {
        if (rotto) throw new Error("database in manutenzione con Riunione di prova");
        return { ...PREDEFINITE };
      },
      canali: () => [c],
      store: creaStore(),
    });
    mancano(10);
    await m.giro();
    expect(m.stato.ultimoErrore?.testo).toBe("giro non riuscito");
    rotto = false;
    await m.giro();
    expect(c.inviati).toHaveLength(1);
    expect(log.join("\n")).not.toContain("Riunione");
  });
});

describe("la tabella e la pulizia", () => {
  const riga = (extra: Partial<Riga> = {}): Riga => ({
    chiave: chiaveDi("vecchio", "2026-09-01T10:00:00", 10, "finto"),
    evento: "vecchio",
    inizio: "2026-09-01T10:00:00",
    minuti: 10,
    canale: "finto",
    inviatoIl: ORA,
    esito: "inviato",
    tentativi: 1,
    errore: null,
    ...extra,
  });

  it("promemoria_inviati sta in gateway.db (lo stesso database dello stato) con le colonne previste", () => {
    creaStore().leggi("x");
    expect(fs.existsSync(path.join(dati, "gateway.db"))).toBe(true);
    const colonne = (archivio().prepare("PRAGMA table_info(promemoria_inviati)").all() as { name: string }[]).map((c) => c.name);
    expect(colonne).toEqual(["chiave", "evento", "inizio", "minuti", "canale", "inviato_il", "esito", "tentativi", "errore"]);
    expect(archivio().prepare("SELECT name FROM sqlite_master WHERE name = 'stato_config'").all()).toHaveLength(1);
  });

  it("scrivere due volte la stessa chiave aggiorna la riga", () => {
    const s = creaStore();
    s.scrivi([riga({ esito: "riprovo", tentativi: 1, errore: "x" })]);
    s.scrivi([riga({ esito: "inviato", tentativi: 2, inviatoIl: ORA + 1, errore: null })]);
    expect(righe()).toHaveLength(1);
    expect(s.leggi(riga().chiave)).toEqual(riga({ tentativi: 2, inviatoIl: ORA + 1 }));
    expect(s.leggi("non c'è")).toBeUndefined();
  });

  it("a ogni giro si tolgono le righe più vecchie di 14 giorni, non quelle di 14 giorni esatti", async () => {
    const s = creaStore();
    const giorno = 86_400_000;
    s.scrivi([
      riga({ chiave: "vecchia", inviatoIl: ORA - CONSERVA_GIORNI * giorno - 1 }),
      riga({ chiave: "limite", inviatoIl: ORA - CONSERVA_GIORNI * giorno }),
      riga({ chiave: "recente", inviatoIl: ORA - 2 * giorno }),
    ]);
    const p = prepara();
    p.eventi = [];
    await p.motore.giro();
    expect(righe().map((r) => r.chiave)).toEqual(["limite", "recente"]);
    expect(s.pulisci(ORA)).toBe(2);
    expect(righe()).toEqual([]);
  });

  it("senza un canale pronto non si pulisce niente", async () => {
    creaStore().scrivi([riga({ inviatoIl: ORA - 30 * 86_400_000 })]);
    const p = prepara({}, [canaleFinto("push", false)]);
    await p.motore.giro();
    expect(righe()).toHaveLength(1);
  });
});

describe("le impostazioni", () => {
  it("di norma: un avviso dieci minuti prima, testo completo, suono predefinito, notifica e avviso a comparsa accesi; la configurazione dell'app non cambia", () => {
    expect(leggiImpostazioni(loadConfig())).toEqual({ minuti: [10], contenuto: "completo", suono: "Default", notifica_windows: true, comparsa: true });
    expect(leggiImpostazioni(loadConfig())).toEqual(PREDEFINITE);
    expect(loadConfig()).toEqual(DEFAULT_CONFIG);
    expect(loadConfig()).not.toHaveProperty("promemoria");
  });

  it("un valore che non va cede il posto al predefinito, un interruttore si spegne solo con false, le voci di prima non contano", () => {
    const leggi = (promemoria: unknown) => leggiImpostazioni({ promemoria });
    expect(leggi({ minuti: [0, 99999], contenuto: "boh", suono: "Fischio", notifica_windows: "false", comparsa: 0, attivo: true, canali: ["push"] })).toEqual(PREDEFINITE);
    expect(leggi({ notifica_windows: false, comparsa: false, minuti: [30, 5, 30], contenuto: "minimo" })).toEqual({
      minuti: [5, 30],
      contenuto: "minimo",
      suono: "Default",
      notifica_windows: false,
      comparsa: false,
    });
    expect(leggi("testo")).toEqual(leggiImpostazioni({}));
    expect(leggi({ minuti: [1, 2, 3, 4, 5, 6, 7, 8, 9] }).minuti).toEqual([10]);
  });

  it("i minuti: interi da 0 a 1440, al più 8 diversi, almeno uno; i doppioni si fondono e l'elenco si ordina", () => {
    const prova = (minuti: unknown) => validaImpostazioni({ minuti }, PREDEFINITE);
    expect(prova([1])).toMatchObject({ ok: true, valore: { minuti: [1] } });
    expect(prova([1440])).toMatchObject({ ok: true });
    expect(prova([15, 5, 10, 5, 15])).toMatchObject({ ok: true, valore: { minuti: [5, 10, 15] } });
    expect(prova([0])).toMatchObject({ ok: true, valore: { minuti: [0] } });
    expect(prova([1, 2, 3, 4, 5, 6, 7, 8])).toMatchObject({ ok: true });
    for (const male of [[1441], [-5], [1.5], ["10"], [null], [NaN], [Infinity], [], "10", 10, null, { 0: 10 }, [1, 2, 3, 4, 5, 6, 7, 8, 9], [[10]]]) {
      expect(prova(male), JSON.stringify(male)).toMatchObject({ ok: false });
    }
  });

  it("il contenuto, gli interruttori dell'app e i campi sconosciuti", () => {
    const prova = (d: unknown) => validaImpostazioni(d, PREDEFINITE);
    expect(prova({ contenuto: "minimo" })).toMatchObject({ ok: true, valore: { contenuto: "minimo" } });
    expect(prova({ notifica_windows: false, comparsa: false })).toMatchObject({ ok: true, valore: { notifica_windows: false, comparsa: false } });
    for (const male of [{ contenuto: "Completo" }, { contenuto: "" }, { contenuto: null }, { notifica_windows: "true" }, { comparsa: 1 }, { comparsa: null }, { attivo: true }, { canali: [] }, { altro: 1 }, [], "x", null, 5]) {
      expect(prova(male), JSON.stringify(male)).toMatchObject({ ok: false });
    }
    // un campo che manca resta com'è
    const prima = { minuti: [7], contenuto: "minimo" as const, suono: "Mail", notifica_windows: false, comparsa: true };
    expect(validaImpostazioni({}, prima)).toEqual({ ok: true, valore: prima });
  });

  it("salvaPromemoria scrive nello stato (stato_config) e nella copia config.json, e una richiesta rifiutata non cambia niente", () => {
    const atteso = { minuti: [5, 15], contenuto: "minimo", suono: "Default", notifica_windows: true, comparsa: false };
    expect(salvaPromemoria({ comparsa: false, minuti: [15, 5], contenuto: "minimo" })).toEqual({ ok: true, promemoria: atteso });
    expect(leggiImpostazioni(loadConfig())).toEqual(atteso);
    const riga = archivio().prepare("SELECT valore FROM stato_config WHERE chiave = 'promemoria'").get() as { valore: string };
    expect(JSON.parse(riga.valore)).toEqual(atteso);
    expect(JSON.parse(fs.readFileSync(path.join(dati, "config.json"), "utf-8")).promemoria.minuti).toEqual([5, 15]);
    expect(salvaPromemoria({ minuti: [-1] })).toMatchObject({ ok: false });
    expect(leggiImpostazioni(loadConfig()).minuti).toEqual([5, 15]);
    // un campo che manca resta com'è
    expect(salvaPromemoria({ comparsa: true })).toMatchObject({ ok: true, promemoria: { minuti: [5, 15], contenuto: "minimo", comparsa: true } });
  });

  it("un rifiuto non scrive niente, nemmeno la copia config.json", () => {
    expect(salvaPromemoria({ minuti: [-1] })).toMatchObject({ ok: false });
    expect(fs.existsSync(path.join(dati, "config.json"))).toBe(false);
    expect(archivio().prepare("SELECT 1 FROM stato_config WHERE chiave = 'promemoria'").all()).toEqual([]);
  });

  it("le impostazioni si leggono a ogni giro: minuti e contenuto cambiati dalla pagina valgono subito", async () => {
    const c = canaleFinto("push");
    const m = creaMotore({
      adesso,
      leggiEventi: async () => ({ eventi: [evento()], problemi: 0 }),
      impostazioni: () => leggiImpostazioni(loadConfig()),
      canali: () => [c],
      store: creaStore(),
    });
    mancano(10);
    modificaConfig((cfg) => void (cfg.promemoria = { minuti: [5] }));
    await m.giro();
    expect(c.inviati).toEqual([]);
    modificaConfig((cfg) => void (cfg.promemoria = { minuti: [10, 5], contenuto: "minimo" }));
    await m.giro();
    expect(c.inviati).toEqual(["Tra 10 min, alle 15:30\nAzienda di prova"]);
  });
});

describe("nessun dato di un evento nei log", () => {
  it("né titolo, luogo, partecipanti, link o indirizzi, in nessuno dei casi (invio, errore, tentativi esauriti, lettura rotta, limite)", async () => {
    const segreti = ["Riunione di prova", "Sala Rossa", "organizzatore@example.test", "partecipante@example.test", "riunione-segreta", "Chi Organizza", "prova@example.test", "Azienda di prova"];
    const ricco = (id: string, start: string) =>
      evento({
        id,
        start,
        location: "Sala Rossa",
        organizer: "Chi Organizza",
        organizer_email: "organizzatore@example.test",
        attendees: ["partecipante@example.test"],
        online_url: "https://teams.example.test/l/riunione-segreta",
      });
    const buono = canaleFinto("buono");
    const rotto = canaleFinto("rotto");
    rotto.esiti = Array.from({ length: 30 }, (_, i) => (i % 7 === 0 ? { ok: false as const, errore: "Il servizio limita gli invii (429)", attesaS: 2 } : { ok: false as const, errore: "Il servizio non è raggiungibile" }));
    const p = prepara({}, [buono, rotto]);
    p.eventi = [ricco("a", "2026-10-12T15:30:00"), ricco("b", "2026-10-12T15:33:00")];
    p.problemi = 3;
    for (let i = 0; i < 12; i++) {
      mancano(10, -i * 20);
      await p.motore.giro();
    }
    p.rotta = new Error(`lettura ${segreti.join(" ")}`);
    mancano(3);
    await p.motore.giro();
    expect(log.length).toBeGreaterThan(0);
    const tutto = log.join("\n");
    for (const s of segreti) expect(tutto, s).not.toContain(s);
    expect(tutto).toContain("promemoria:");
    // e i testi inviati, questi sì, hanno i dettagli
    expect(buono.inviati.join("\n")).toContain("Riunione di prova");
  });

  it("lo stesso problema si scrive una volta ogni dieci minuti, non a ogni giro", async () => {
    const c = canaleFinto();
    c.esiti = Array.from({ length: 10 }, () => ({ ok: false as const, errore: "Canale di prova non risponde" }));
    const p = prepara({ minuti: [60] }, [c]);
    const righeLog = () => log.filter((l) => l.startsWith("promemoria: Canale di prova")).length;
    mancano(50);
    await p.motore.giro();
    mancano(49);
    await p.motore.giro();
    mancano(45);
    await p.motore.giro();
    expect(righeLog()).toBe(1);
    mancano(39);
    await p.motore.giro();
    expect(righeLog()).toBe(2);
  });
});

describe("il giro dentro il gateway", () => {
  it("avviaPromemoria richiamata più volte lascia un timer solo, e fermaPromemoria lo toglie", () => {
    vi.useFakeTimers({ now: ORA });
    fissaOra(null);
    avviaPromemoria();
    avviaPromemoria();
    avviaPromemoria();
    expect(vi.getTimerCount()).toBe(1);
    expect(log.filter((l) => l.includes("pronto"))).toEqual(["promemoria: pronto, un giro ogni 30 s"]);
    fermaPromemoria();
    expect(vi.getTimerCount()).toBe(0);
    avviaPromemoria();
    expect(vi.getTimerCount()).toBe(1);
  });

  it("il timer non tiene vivo il processo (unref)", () => {
    avviaPromemoria();
    const timer = (globalThis as unknown as { __oePromemoria: { timer: NodeJS.Timeout } }).__oePromemoria.timer;
    expect(timer.hasRef()).toBe(false);
  });

  it("SIGTERM toglie il timer, e nessun gestore resta", () => {
    vi.useFakeTimers({ now: ORA });
    fissaOra(null);
    const altro = () => {};
    process.on("SIGTERM", altro);
    try {
      const prima = process.listenerCount("SIGTERM");
      avviaPromemoria();
      expect(process.listenerCount("SIGTERM")).toBe(prima + 1);
      expect(vi.getTimerCount()).toBe(1);
      (process as NodeJS.EventEmitter).emit("SIGTERM");
      expect(vi.getTimerCount()).toBe(0);
      expect(process.listenerCount("SIGTERM")).toBe(prima);
    } finally {
      process.off("SIGTERM", altro);
    }
  });

  it("il primo giro dopo 10 secondi, poi uno ogni PROMEMORIA_INTERVALLO_S (30 di norma), a giro finito", async () => {
    vi.useFakeTimers({ now: ORA });
    fissaOra(null);
    // senza browser iscritti il giro si ferma subito ma lascia l'ora
    avviaPromemoria();
    expect(statoGlobale().ultimoGiro).toBeNull();
    await vi.advanceTimersByTimeAsync(9_999);
    expect(statoGlobale().ultimoGiro).toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    expect(statoGlobale().ultimoGiro).toBe(ORA + 10_000);
    await vi.advanceTimersByTimeAsync(29_999);
    expect(statoGlobale().ultimoGiro).toBe(ORA + 10_000);
    await vi.advanceTimersByTimeAsync(1);
    expect(statoGlobale().ultimoGiro).toBe(ORA + 40_000);
    expect(statoGlobale().ultimoErrore).toBeNull();
  });

  it("PROMEMORIA_INTERVALLO_S cambia l'intervallo; un valore che non va vale 30", () => {
    const intervallo = (valore: string | undefined) => {
      if (valore === undefined) delete process.env.PROMEMORIA_INTERVALLO_S;
      else process.env.PROMEMORIA_INTERVALLO_S = valore;
      log.length = 0;
      avviaPromemoria();
      fermaPromemoria();
      return log.find((l) => l.includes("pronto"));
    };
    vi.useFakeTimers({ now: ORA });
    expect(intervallo("60")).toBe("promemoria: pronto, un giro ogni 60 s");
    expect(intervallo("5")).toBe("promemoria: pronto, un giro ogni 5 s");
    expect(intervallo("3600")).toBe("promemoria: pronto, un giro ogni 3600 s");
    for (const male of [undefined, "", "abc", "0", "4", "3601", "-30", "1.5"]) expect(intervallo(male), String(male)).toBe("promemoria: pronto, un giro ogni 30 s");
  });

  it("senza browser iscritti (di norma) il giro gira ma non legge gli eventi e non apre nessuna connessione", async () => {
    vi.useFakeTimers({ now: ORA });
    fissaOra(null);
    const rete = vi.spyOn(globalThis, "fetch");
    avviaPromemoria();
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    expect(rete).not.toHaveBeenCalled();
    expect(statoGlobale().ultimoErrore).toBeNull();
    expect(archivio().prepare("SELECT name FROM sqlite_master WHERE name = 'promemoria_inviati'").all()).toEqual([]);
  });
});
