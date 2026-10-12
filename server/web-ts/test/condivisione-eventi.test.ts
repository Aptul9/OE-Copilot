import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AVANTI, INDIETRO, type Livello } from "@/lib/condivisione-comune";
import { azzeraMemoria } from "@/lib/core/agente";
import { archivio, chiudiArchivio } from "@/lib/core/archivio";
import type { Condivisione } from "@/lib/core/condivisione";
import { EVENTI_MAX, finestra, inEventiUscita, leggiPerCondivisione, LetturaNonDisponibile, luogoEsportabile, titoloEsportabile } from "@/lib/core/condivisione-eventi";
import { raccogli } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { aggiungiEvento } from "@/lib/core/locali";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig, modificaConfig } from "@/lib/core/stato";
import { blocchi, GIORNI_BLOCCO, giorniTra, spostaData } from "@/lib/disponibilita";
import { costruisciIcs, testoIcs } from "@/lib/ics-uscita";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, copiaIcs, CORPUS, type AgenteFinto, type EventoFinto, TUTTO_IL_CORPUS } from "./agente-condivisione";

// Cosa c'è nel calendario di un link (lib/core/condivisione-eventi.ts), letto dagli stessi percorsi dell'app con un agente finto: i due livelli, gli
// interruttori e i gruppi, la finestra, i blocchi di 28 giorni, i calendari ricevuti da altri che non escono mai e quello che non deve uscire dei dettagli.

const OGGI = "2026-10-12";
const ALFA = "mario@alfa.example";
const BETA = "mario@beta.example";
const SALE = "00112233445566778899aabbccddeeff";

const EVENTI: EventoFinto[] = [
  { account: ALFA, title: CORPUS.titoli[0], start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00", location: CORPUS.luoghi[0], organizer: CORPUS.organizzatori[0], organizer_email: CORPUS.posta[0], online_url: CORPUS.conferenze[0], attendees: [BETA] },
  { account: BETA, title: CORPUS.titoli[4], start: "2026-10-13T09:30:00", end: "2026-10-13T11:00:00", location: CORPUS.conferenze[0], organizer: CORPUS.organizzatori[1], organizer_email: CORPUS.posta[1] },
  { account: ALFA, title: "Ferie Bianchi", start: "2026-10-14T00:00:00", end: "2026-10-15T00:00:00", allDay: true },
  { account: BETA, title: "Colloquio con link https://zoom.us/j/123456789?pwd=segretissima", start: "2026-10-16T14:00:00", end: "2026-10-16T15:00:00", location: "Microsoft Teams Meeting" },
  { account: ALFA, title: "Turno notte", start: "2026-10-25T01:30:00", end: "2026-10-25T04:30:00" },
  { account: BETA, title: CORPUS.titoli[3], start: "2026-11-01T22:00:00", end: "2026-11-02T02:00:00", location: CORPUS.luoghi[3] },
  { account: ALFA, title: "Fine anno", start: "2026-12-20T15:00:00", end: "2026-12-20T16:00:00", location: CORPUS.luoghi[1] },
  { account: ALFA, title: CORPUS.titoli[2], start: "2026-09-12T10:00:00", end: "2026-09-12T11:00:00", location: CORPUS.luoghi[2] },
  { account: ALFA, title: "Riunione lontana", start: "2027-02-10T10:00:00", end: "2027-02-10T11:00:00" },
];

let dati: string;
let agente: AgenteFinto;

const link = (extra: Partial<Condivisione> = {}): Condivisione => ({
  id: "000000000001",
  nome: "Prova",
  token: null,
  sale: SALE,
  livello: "occupato",
  ambito: "tutti",
  gruppo: "",
  giorni_indietro: 7,
  giorni_avanti: 90,
  nome_calendario: "",
  creato: 0,
  aggiornato: 0,
  ultimo_accesso: null,
  accessi: 0,
  revocato: null,
  ...extra,
});
const testo = async (extra: Partial<Condivisione> = {}) => {
  const c = link(extra);
  return costruisciIcs({ nome: "Prova", sale: c.sale, livello: c.livello, stamp: Date.UTC(2026, 9, 12, 8), eventi: inEventiUscita(await leggiPerCondivisione(c, OGGI)) });
};
const righe = (t: string) => t.replace(/\r\n /g, "").split("\r\n").filter(Boolean);
const proprieta = (t: string, nome: string) => righe(t).filter((r) => r.startsWith(nome + ":") || r.startsWith(nome + ";"));
const intervalli = async (extra: Partial<Condivisione> = {}) => (await leggiPerCondivisione(link(extra), OGGI)).impegni;
const eventiLetti = async (extra: Partial<Condivisione> = {}) => (await leggiPerCondivisione(link({ livello: "titoli", ...extra }), OGGI)).eventi;
const titoliLetti = async (extra: Partial<Condivisione> = {}) => (await eventiLetti(extra)).map((e) => e.titolo);

beforeEach(async () => {
  dati = cartellaDati();
  agente = await avviaAgenteFinto(structuredClone(EVENTI));
  process.env.OE_DATI = dati;
  process.env.AGENTE_URL = agente.url;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(Date.UTC(2026, 9, 12, 8, 0, 0));
  azzeraMemoria();
  azzeraFeed();
  modificaConfig((c) => {
    c.gruppi_calendari = { "solo-alfa": ["Alfa"], "solo-beta": ["Beta"] };
  });
  aggiungiEvento({ title: "Dentista", start: "2026-10-16T15:00", end: "2026-10-16T16:00", allDay: false, location: "Via Garibaldi 3", notes: CORPUS.note[0] });
  aggiungiEvento({ title: "Compleanno", start: "2026-10-17T10:00", end: "2026-10-17T11:00", allDay: true, location: "", notes: "" });
});

afterEach(async () => {
  await agente.chiudi();
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE"]) delete process.env[n];
  vi.restoreAllMocks();
});

describe("finestra: i giorni indietro e avanti di un link", () => {
  it("da `indietro` giorni prima di oggi a `avanti` giorni dopo, oggi compreso: la fine è esclusa", () => {
    expect(finestra({ giorni_indietro: 7, giorni_avanti: 90 }, OGGI)).toEqual({ da: "2026-10-05", a: "2027-01-11" });
    expect(finestra({ giorni_indietro: 0, giorni_avanti: 1 }, OGGI)).toEqual({ da: "2026-10-12", a: "2026-10-14" });
  });
  it("i limiti si tengono anche se nel database ci fosse un valore fuori: al più 30 indietro e 365 avanti", () => {
    expect(finestra({ giorni_indietro: 999, giorni_avanti: 9999 }, OGGI)).toEqual(finestra({ giorni_indietro: INDIETRO.max, giorni_avanti: AVANTI.max }, OGGI));
    expect(finestra({ giorni_indietro: -5, giorni_avanti: 0 }, OGGI)).toEqual(finestra({ giorni_indietro: INDIETRO.min, giorni_avanti: AVANTI.min }, OGGI));
    expect(finestra({ giorni_indietro: 3.9, giorni_avanti: 10.2 }, OGGI)).toEqual(finestra({ giorni_indietro: 3, giorni_avanti: 10 }, OGGI));
  });
  it("un valore che non è un numero torna a quello di partenza", () => {
    expect(finestra({ giorni_indietro: NaN, giorni_avanti: Infinity }, OGGI)).toEqual(finestra({ giorni_indietro: INDIETRO.predefinito, giorni_avanti: AVANTI.predefinito }, OGGI));
  });
  it("passa il cambio d'ora e l'anno senza perdere un giorno", () => {
    expect(finestra({ giorni_indietro: 0, giorni_avanti: 30 }, "2026-10-20")).toEqual({ da: "2026-10-20", a: "2026-11-20" });
    expect(finestra({ giorni_indietro: 2, giorni_avanti: 3 }, "2026-12-31")).toEqual({ da: "2026-12-29", a: "2027-01-04" });
  });
});

describe("il livello occupato: gli orari occupati, uniti", () => {
  it("gli stessi di /disponibilita: eventi che si sovrappongono o si toccano sono un blocco solo, anche a cavallo della mezzanotte e di due blocchi di lettura", async () => {
    expect(await intervalli()).toEqual([
      ["2026-10-13T09:00:00", "2026-10-13T11:00:00"], // Alfa e Beta insieme
      ["2026-10-16T14:00:00", "2026-10-16T16:00:00"], // Beta e l'evento personale che comincia quando finisce
      ["2026-10-25T01:30:00", "2026-10-25T04:30:00"],
      ["2026-11-01T22:00:00", "2026-11-02T02:00:00"], // attraversa il confine fra due blocchi, e arriva una volta sola
      ["2026-12-20T15:00:00", "2026-12-20T16:00:00"],
    ]);
  });

  it("i giorni interi non contano come occupati, e l'evento che il calendario nasconderebbe non cambia niente", async () => {
    const i = await intervalli();
    expect(i.some(([a]) => a.startsWith("2026-10-14") || a.startsWith("2026-10-17"))).toBe(false);
  });

  it("il testo: orari UTC con la Z, tutti con il nome Occupato, e quattro ore vere per l'evento a cavallo del cambio d'ora", async () => {
    const t = await testo();
    expect(proprieta(t, "DTSTART")).toEqual([
      "DTSTART:20261013T070000Z",
      "DTSTART:20261016T120000Z",
      "DTSTART:20261024T233000Z",
      "DTSTART:20261101T210000Z",
      "DTSTART:20261220T140000Z",
    ]);
    expect(proprieta(t, "DTEND")).toEqual(["DTEND:20261013T090000Z", "DTEND:20261016T140000Z", "DTEND:20261025T033000Z", "DTEND:20261102T010000Z", "DTEND:20261220T150000Z"]);
    expect(proprieta(t, "SUMMARY")).toEqual(Array(5).fill("SUMMARY:Occupato"));
    expect(proprieta(t, "TRANSP")).toEqual(Array(5).fill("TRANSP:OPAQUE"));
  });

  it("nel testo non c'è niente di quello che l'agente sa: titoli, luoghi, chi organizza, posta, link, aziende, note", async () => {
    const t = await testo();
    const tutto = [t, t.replace(/\r\n /g, ""), t.toLowerCase()];
    for (const privata of [...TUTTO_IL_CORPUS, "Ferie Bianchi", "Turno notte", "Fine anno", "Dentista", "Compleanno", "Via Garibaldi", "Microsoft Teams", "zoom.us", "segretissima"]) {
      for (const forma of [privata, testoIcs(privata), privata.toLowerCase()]) for (const x of tutto) expect(x, forma).not.toContain(forma);
    }
  });

  it("legge a blocchi di 28 giorni che cominciano di lunedì, di fila, e sono 4 per la finestra di partenza", async () => {
    agente.richieste.length = 0;
    await intervalli();
    const finestre = agente.richieste.filter((r) => r.startsWith("GET /api/eventi")).map((r) => new URL(r, "http://x").searchParams);
    const inizi = finestre.map((q) => (q.get("inizio") as string).slice(0, 10)).sort();
    expect(inizi).toEqual(blocchi("2026-10-05", "2027-01-11"));
    expect(inizi).toHaveLength(4);
    for (const q of finestre) {
      expect(new Date((q.get("inizio") as string) + "Z").getUTCDay()).toBe(1);
      expect((q.get("inizio") as string).slice(11)).toBe("00:00:00");
      expect(giorniTra((q.get("inizio") as string).slice(0, 10), (q.get("fine") as string).slice(0, 10))).toBe(GIORNI_BLOCCO);
    }
    for (let i = 1; i < inizi.length; i++) expect(giorniTra(inizi[i - 1], inizi[i])).toBe(GIORNI_BLOCCO);
  });

  it("con la finestra più larga (30 indietro, 365 avanti) sono 15 blocchi, tutti letti, e ognuno di 28 giorni", async () => {
    agente.richieste.length = 0;
    const i = await intervalli({ giorni_indietro: 30, giorni_avanti: 365 });
    const richieste = agente.richieste.filter((r) => r.startsWith("GET /api/eventi"));
    expect(richieste).toHaveLength(blocchi("2026-09-12", "2027-10-13").length);
    expect(richieste).toHaveLength(15);
    expect(i[0]).toEqual(["2026-09-12T10:00:00", "2026-09-12T11:00:00"]);
    expect(i[i.length - 1]).toEqual(["2027-02-10T10:00:00", "2027-02-10T11:00:00"]);
  });
});

describe("la finestra tiene fuori quello che non ci sta", () => {
  it("di partenza (7 indietro, 90 avanti): l'evento di un mese fa e quello di quattro mesi avanti non ci sono", async () => {
    const t = await titoliLetti();
    expect(t).not.toContain(CORPUS.titoli[2]);
    expect(t).not.toContain("Riunione lontana");
  });
  it("con 30 indietro c'è quello di un mese fa; con 365 avanti quello lontano", async () => {
    expect(await titoliLetti({ giorni_indietro: 30 })).toContain(CORPUS.titoli[2]);
    expect(await titoliLetti({ giorni_avanti: 365 })).toContain("Riunione lontana");
  });
  it("0 indietro parte da oggi (ieri no, con 1 sì), 1 avanti arriva a domani compreso e non a dopodomani", async () => {
    agente.eventi.push(
      { account: ALFA, title: "Ieri", start: "2026-10-11T10:00:00", end: "2026-10-11T11:00:00" },
      { account: ALFA, title: "Oggi", start: "2026-10-12T10:00:00", end: "2026-10-12T11:00:00" },
      { account: ALFA, title: "Dopodomani", start: "2026-10-14T10:00:00", end: "2026-10-14T11:00:00" },
    );
    expect(await titoliLetti({ giorni_indietro: 0, giorni_avanti: 1 })).toEqual(["Oggi", CORPUS.titoli[0], CORPUS.titoli[4]]);
    expect(await titoliLetti({ giorni_indietro: 1, giorni_avanti: 1 })).toEqual(["Ieri", "Oggi", CORPUS.titoli[0], CORPUS.titoli[4]]);
    expect(await titoliLetti({ giorni_indietro: 0, giorni_avanti: 2 })).toEqual(["Oggi", CORPUS.titoli[0], CORPUS.titoli[4], "Dopodomani"]);
  });
  it("un valore fuori dai limiti nel database vale il limite: 999 indietro non legge più di 30 giorni", async () => {
    const t = await titoliLetti({ giorni_indietro: 999 });
    expect(t).toContain(CORPUS.titoli[2]);
    agente.eventi.push({ account: ALFA, title: "Quarantun giorni fa", start: "2026-09-01T10:00:00", end: "2026-09-01T11:00:00" });
    expect(await titoliLetti({ giorni_indietro: 999 })).not.toContain("Quarantun giorni fa");
    agente.eventi.push({ account: ALFA, title: "Fra 400 giorni", start: "2027-11-20T10:00:00", end: "2027-11-20T11:00:00" });
    expect(await titoliLetti({ giorni_avanti: 99999 })).not.toContain("Fra 400 giorni");
  });
});

describe("il livello con i titoli", () => {
  it("un evento ciascuno, in ordine, con titolo e luogo e basta; i giorni interi delle caselle seguono l'interruttore, quelli personali no", async () => {
    expect(await eventiLetti()).toEqual([
      { inizio: "2026-10-13T09:00:00", fine: "2026-10-13T10:00:00", tuttoGiorno: false, titolo: CORPUS.titoli[0], luogo: CORPUS.luoghi[0] },
      { inizio: "2026-10-13T09:30:00", fine: "2026-10-13T11:00:00", tuttoGiorno: false, titolo: CORPUS.titoli[4], luogo: "" },
      { inizio: "2026-10-16T14:00:00", fine: "2026-10-16T15:00:00", tuttoGiorno: false, titolo: "Colloquio con link", luogo: "" },
      { inizio: "2026-10-16T15:00:00", fine: "2026-10-16T16:00:00", tuttoGiorno: false, titolo: "Dentista", luogo: "Via Garibaldi 3" },
      { inizio: "2026-10-17", fine: "2026-10-18", tuttoGiorno: true, titolo: "Compleanno", luogo: "" },
      { inizio: "2026-10-25T01:30:00", fine: "2026-10-25T04:30:00", tuttoGiorno: false, titolo: "Turno notte", luogo: "" },
      { inizio: "2026-11-01T22:00:00", fine: "2026-11-02T02:00:00", tuttoGiorno: false, titolo: CORPUS.titoli[3], luogo: CORPUS.luoghi[3] },
      { inizio: "2026-12-20T15:00:00", fine: "2026-12-20T16:00:00", tuttoGiorno: false, titolo: "Fine anno", luogo: CORPUS.luoghi[1] },
    ]);
  });

  it("con mostra_tutto_il_giorno acceso in personalizzazione.json anche i giorni interi delle caselle, come nell'app", async () => {
    fs.writeFileSync(path.join(dati, "personalizzazione.json"), JSON.stringify({ mostra_tutto_il_giorno: true }));
    const tutti = await eventiLetti();
    expect(tutti).toContainEqual({ inizio: "2026-10-14", fine: "2026-10-15", tuttoGiorno: true, titolo: "Ferie Bianchi", luogo: "" });
    // i giorni interi non sono mai nel livello occupato, nemmeno con l'interruttore acceso
    expect((await intervalli()).some(([a]) => a.startsWith("2026-10-14"))).toBe(false);
  });

  it("il testo: i giorni interi sono date, gli altri orari UTC; titolo e luogo ci sono, protetti", async () => {
    fs.writeFileSync(path.join(dati, "personalizzazione.json"), JSON.stringify({ mostra_tutto_il_giorno: true }));
    const t = await testo({ livello: "titoli" });
    expect(proprieta(t, "DTSTART;VALUE=DATE")).toEqual(["DTSTART;VALUE=DATE:20261014", "DTSTART;VALUE=DATE:20261017"]);
    expect(proprieta(t, "DTEND;VALUE=DATE")).toEqual(["DTEND;VALUE=DATE:20261015", "DTEND;VALUE=DATE:20261018"]);
    expect(proprieta(t, "SUMMARY")).toContain("SUMMARY:" + testoIcs(CORPUS.titoli[0]));
    expect(proprieta(t, "LOCATION")).toContain("LOCATION:" + testoIcs(CORPUS.luoghi[0]));
    expect(proprieta(t, "SUMMARY")).toHaveLength(9);
  });

  it("l'evento a cavallo di due blocchi di lettura esce una volta sola", async () => {
    const t = await titoliLetti();
    expect(t.filter((x) => x === CORPUS.titoli[3])).toHaveLength(1);
  });

  it("nel testo mancano descrizione, partecipanti, chi organizza, posta, link per collegarsi, nomi di aziende e note", async () => {
    fs.writeFileSync(path.join(dati, "personalizzazione.json"), JSON.stringify({ mostra_tutto_il_giorno: true }));
    const t = await testo({ livello: "titoli" });
    const tutto = [t, t.replace(/\r\n /g, ""), t.toLowerCase()];
    for (const privata of [...CORPUS.organizzatori, ...CORPUS.posta, ...CORPUS.conferenze, ...CORPUS.aziende, ...CORPUS.note, "teams.microsoft.com", "zoom.us", "meet.google", "segretissima", "Microsoft Teams"]) {
      for (const x of tutto) expect(x, privata).not.toContain(privata);
    }
    for (const chiave of ["DESCRIPTION", "ATTENDEE", "ORGANIZER", "URL", "CATEGORIES", "CONFERENCE", "ATTACH"]) expect(t, chiave).not.toContain(chiave);
    // e quello che deve esserci c'è
    for (const titolo of [CORPUS.titoli[0], CORPUS.titoli[3], "Colloquio con link", "Dentista", "Ferie Bianchi"]) expect(t.replace(/\r\n /g, "")).toContain(testoIcs(titolo));
  });
});

describe("tutti i calendari, o un gruppo", () => {
  it("con tutti valgono gli interruttori della barra laterale: un'azienda spenta non esce", async () => {
    modificaConfig((c) => void (c.disabled_calendars = [`remoto::${BETA}`]));
    expect(await titoliLetti()).toEqual([CORPUS.titoli[0], "Dentista", "Compleanno", "Turno notte", "Fine anno"]);
    expect(await intervalli()).toEqual([
      ["2026-10-13T09:00:00", "2026-10-13T10:00:00"],
      ["2026-10-16T15:00:00", "2026-10-16T16:00:00"],
      ["2026-10-25T01:30:00", "2026-10-25T04:30:00"],
      ["2026-12-20T15:00:00", "2026-12-20T16:00:00"],
    ]);
  });

  it("anche il calendario degli eventi personali si spegne", async () => {
    modificaConfig((c) => void (c.disabled_calendars = ["local::altro"]));
    const t = await titoliLetti();
    expect(t).not.toContain("Dentista");
    expect(t).toContain(CORPUS.titoli[0]);
  });

  it("con un gruppo contano le aziende del gruppo, come in /disponibilita?gruppo=: gli interruttori non c'entrano", async () => {
    modificaConfig((c) => void (c.disabled_calendars = [`remoto::${BETA}`]));
    expect(await titoliLetti({ ambito: "gruppo", gruppo: "solo-beta" })).toEqual([CORPUS.titoli[4], "Colloquio con link", CORPUS.titoli[3]]);
    expect(await intervalli({ ambito: "gruppo", gruppo: "solo-beta" })).toEqual([
      ["2026-10-13T09:30:00", "2026-10-13T11:00:00"],
      ["2026-10-16T14:00:00", "2026-10-16T15:00:00"],
      ["2026-11-01T22:00:00", "2026-11-02T02:00:00"],
    ]);
    expect(await titoliLetti({ ambito: "gruppo", gruppo: "solo-alfa" })).toEqual([CORPUS.titoli[0], "Turno notte", "Fine anno"]);
  });

  it("un gruppo che raccoglie più aziende e gli eventi personali, se il gruppo li nomina", async () => {
    modificaConfig((c) => void (c.gruppi_calendari = { misto: ["Beta", "Eventi personali"] }));
    expect(await titoliLetti({ ambito: "gruppo", gruppo: "misto" })).toEqual([CORPUS.titoli[4], "Colloquio con link", "Dentista", "Compleanno", CORPUS.titoli[3]]);
  });

  it("un gruppo che non c'è (più) non si legge: si solleva, non si pubblica tutto", async () => {
    await expect(leggiPerCondivisione(link({ ambito: "gruppo", gruppo: "sparito" }), OGGI)).rejects.toBeInstanceOf(LetturaNonDisponibile);
    modificaConfig((c) => void (c.gruppi_calendari = {}));
    await expect(leggiPerCondivisione(link({ ambito: "gruppo", gruppo: "solo-alfa" }), OGGI)).rejects.toBeInstanceOf(LetturaNonDisponibile);
  });
});

describe("i calendari ricevuti da altri non escono mai", () => {
  beforeEach(() => {
    modificaConfig((c) => void (c.ics_calendars = [{ id: "ics_prova", name: "Calendario di un amico", url: "http://127.0.0.1:9/amico.ics" }]));
    copiaIcs(dati, "ics_prova", "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
    copiaIcs(dati, "builtin::festivita-it", "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
    agente.ics.push(
      { uid: "x1", inizio: "2026-10-15T11:00:00", fine: "2026-10-15T12:00:00", titolo: "Evento di un altro OE Copilot" },
      { uid: "x2", inizio: "2026-10-20T00:00:00", fine: "2026-10-21T00:00:00", titolo: "Festa di prova", tutto_giorno: true },
    );
  });

  it("l'app li mostra (prova che la prova li vedrebbe)...", async () => {
    const [eventi] = await raccogli(loadConfig(), "2026-10-12T00:00:00", "2026-10-26T00:00:00");
    const titoli = eventi.map((e) => e.title);
    expect(titoli).toContain("Evento di un altro OE Copilot");
    expect(titoli).toContain("Festa di prova");
  });

  it("...e il calendario condiviso no, in nessun livello, nemmeno con tutti i giorni interi accesi", async () => {
    fs.writeFileSync(path.join(dati, "personalizzazione.json"), JSON.stringify({ mostra_tutto_il_giorno: true }));
    agente.richieste.length = 0;
    for (const livello of ["occupato", "titoli"] as Livello[]) {
      const t = await testo({ livello });
      for (const privata of ["Evento di un altro OE Copilot", "Festa di prova", "amico", "Calendario di un amico"]) expect(t, `${livello} ${privata}`).not.toContain(privata);
    }
    expect((await intervalli()).some(([a]) => a.startsWith("2026-10-15T11"))).toBe(false);
    expect((await titoliLetti()).some((x) => /altro OE Copilot|Festa/.test(x))).toBe(false);
    // non si leggono nemmeno: l'agente non ha ricevuto nessuna richiesta di espandere un calendario ICS
    expect(agente.richieste.filter((r) => r.startsWith("POST /api/ics"))).toEqual([]);
  });

  it("un evento della casella che ha lo stesso titolo di uno ricevuto esce come suo, senza toccare l'altro", async () => {
    agente.eventi.push({ account: ALFA, title: "Evento di un altro OE Copilot", start: "2026-10-15T11:00:00", end: "2026-10-15T12:00:00" });
    const eventi = (await eventiLetti()).filter((e) => e.titolo === "Evento di un altro OE Copilot");
    expect(eventi).toHaveLength(1);
  });
});

describe("la lettura non scrive niente", () => {
  it("né la configurazione (i colori che le sorgenti assegnerebbero), né la copia degli eventi su disco", async () => {
    const stato = () => archivio().prepare("SELECT chiave, valore FROM stato_config ORDER BY chiave").all();
    const prima = stato();
    await leggiPerCondivisione(link({ livello: "titoli" }), OGGI);
    await leggiPerCondivisione(link({ livello: "occupato" }), OGGI);
    expect(stato()).toEqual(prima);
    expect(fs.existsSync(path.join(dati, "events_cache.json"))).toBe(false);
  });
});

describe("quando l'agente non risponde", () => {
  it("non si pubblica un calendario vuoto (per chi lo importa vorrebbe dire libero): si solleva", async () => {
    agente.giu = true;
    for (const livello of ["occupato", "titoli"] as Livello[]) await expect(leggiPerCondivisione(link({ livello }), OGGI)).rejects.toBeInstanceOf(LetturaNonDisponibile);
  });

  it("e quando torna si legge di nuovo, senza che resti una copia vecchia", async () => {
    agente.giu = true;
    await expect(leggiPerCondivisione(link(), OGGI)).rejects.toBeInstanceOf(LetturaNonDisponibile);
    agente.giu = false;
    expect((await intervalli()).length).toBe(5);
  });
});

describe("quanto è lungo un calendario", () => {
  it("al più EVENTI_MAX eventi: oltre, si pubblicano i primi e si scrive nel log", async () => {
    const ora = (i: number) => `2026-10-${String(13 + (i % 10)).padStart(2, "0")}T${String(Math.floor(i / 400) % 24).padStart(2, "0")}:${String(i % 60).padStart(2, "0")}`;
    agente.eventi = Array.from({ length: EVENTI_MAX + 30 }, (_, i) => ({ account: ALFA, title: `Evento ${String(i).padStart(5, "0")}`, start: `${ora(i)}:00`, end: `${ora(i)}:30` }));
    const letti = await eventiLetti({ giorni_avanti: 30 });
    expect(letti).toHaveLength(EVENTI_MAX);
    expect(vi.mocked(console.warn)).toHaveBeenCalledWith(expect.stringContaining(`se ne pubblicano ${EVENTI_MAX}`));
  });
});

describe("titoloEsportabile e luogoEsportabile", () => {
  it("il titolo: niente link, a una riga, non vuoto", () => {
    expect(titoloEsportabile("Riunione")).toBe("Riunione");
    expect(titoloEsportabile("  Call https://teams.microsoft.com/l/meetup-join/abc?x=1&y=2  con Anna ")).toBe("Call con Anna");
    expect(titoloEsportabile("Vai su www.example.com/stanza ora")).toBe("Vai su ora");
    expect(titoloEsportabile("riga uno\nriga due\r\ntre")).toBe("riga uno riga due tre");
    expect(titoloEsportabile("zoom.us/j/12345 sala")).toBe("sala");
    expect(titoloEsportabile("")).toBe("(senza titolo)");
    expect(titoloEsportabile(null)).toBe("(senza titolo)");
    expect(titoloEsportabile("https://meet.google.com/abc-defg-hij")).toBe("(senza titolo)");
    expect([...titoloEsportabile("x".repeat(400))].length).toBe(250);
  });

  it("il luogo: via link, indirizzi di posta e conferenze, restano i posti veri", () => {
    expect(luogoEsportabile("Sala Rossa")).toBe("Sala Rossa");
    expect(luogoEsportabile("Studio Dr. Verdi, via Roma 12")).toBe("Studio Dr. Verdi, via Roma 12");
    expect(luogoEsportabile("Sala Rossa https://teams.microsoft.com/l/meetup-join/x")).toBe("Sala Rossa");
    expect(luogoEsportabile("https://zoom.us/j/123456789?pwd=abc")).toBe("");
    expect(luogoEsportabile("zoom.us/j/123456789")).toBe("");
    expect(luogoEsportabile("Ufficio di anna.rossi@alfa.example, piano 3")).toBe("Ufficio di, piano 3");
    expect(luogoEsportabile("tel:+390212345678,,123456#")).toBe("");
    expect(luogoEsportabile("sip:sala@example.com")).toBe("");
    expect(luogoEsportabile(undefined)).toBe("");
  });

  it.each(["Microsoft Teams Meeting", "Riunione di Microsoft Teams", "Zoom Meeting", "Google Meet", "Webex", "Skype Meeting", "Online", "teams", "Microsoft Teams"])("l'etichetta di una riunione online (%s) non dice dove: il luogo è vuoto", (etichetta) => {
    expect(luogoEsportabile(etichetta)).toBe("");
  });

  it("i link per collegarsi e gli indirizzi di posta del corpus, messi nel luogo, escono senza; i link anche dal titolo (la posta nel titolo è scelta di chi lo scrive)", () => {
    for (const privata of [...CORPUS.conferenze, ...CORPUS.posta]) expect(luogoEsportabile(`Sala 3 ${privata}`), privata).toBe("Sala 3");
    for (const privata of CORPUS.conferenze) expect(titoloEsportabile(`Incontro ${privata}`), privata).toBe("Incontro");
  });
});

describe("la finestra in giorni del calendario letto è coerente con spostaData", () => {
  it("l'ultimo giorno della finestra è oggi + avanti", () => {
    const { a } = finestra({ giorni_indietro: 0, giorni_avanti: 10 }, OGGI);
    expect(spostaData(a, -1)).toBe("2026-10-22");
  });
});
