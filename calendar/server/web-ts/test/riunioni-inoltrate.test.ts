import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiudiArchivio } from "@/lib/core/archivio";
import { raccogli } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig } from "@/lib/core/stato";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto, type EventoFinto } from "./agente-condivisione";

// Lo stesso invito arriva in più copie: inoltrato ("FW: ..."), come riunione fatta rispondendo a una mail ("RE: ..."), o senza prefisso. Hanno lo stesso
// titolo a meno del prefisso e lo stesso orario: nella pagina è una riunione sola, qualunque sia l'ordine in cui l'agente le manda.

const ALFA = "antonio@alfa.example";
const GAMMA = "antonio@gamma.example";
const LINK = "https://teams.example/l/meetup-join/abc";
const ORARIO = { start: "2026-10-16T16:00:00", end: "2026-10-16T16:50:00" };

// Le due copie come le manda l'agente per la stessa casella: l'invito inoltrato nel calendario predefinito, e la copia nel calendario "Beta"
const INOLTRATO: EventoFinto = { account: ALFA, calendar: `${ALFA} · Calendar`, title: "FW: Riunione di prova", organizer_email: "organizzatore@delta.example", attendees: [ALFA], online_url: LINK, ...ORARIO };
const RISPOSTA: EventoFinto = { account: ALFA, calendar: `${ALFA} · Beta`, title: "RE: Riunione di prova", organizer_email: ALFA, online_url: LINK, ...ORARIO };

let dati: string;
let agente: AgenteFinto | undefined;

beforeEach(() => {
  dati = cartellaDati({ azienda_per_cartella: { "· beta": "Beta" }, domini_aziendali: ["alfa.example", "gamma.example"], nomi_per_dominio: { "alfa.example": "Alfa", "gamma.example": "Gamma" } });
  process.env.OE_DATI = dati;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(Date.UTC(2026, 9, 16, 6, 0, 0));
  azzeraFeed();
});

afterEach(async () => {
  await agente?.chiudi();
  agente = undefined;
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE"]) delete process.env[n];
  vi.restoreAllMocks();
});

/** Gli eventi che la pagina mostra per il 2026-10-16, dato quello che l'agente manda. */
async function pagina(eventi: EventoFinto[]) {
  agente = await avviaAgenteFinto(eventi);
  process.env.AGENTE_URL = agente.url;
  azzeraMemoria();
  const [letti] = await raccogli(loadConfig(), "2026-10-16T00:00:00", "2026-10-17T00:00:00");
  return letti;
}

const ordini = [
  ["come arrivano", false],
  ["in ordine inverso", true],
] as const;
const inOrdine = (eventi: EventoFinto[], inverso: boolean) => (inverso ? [...eventi].reverse() : eventi);

describe.each(ordini)("la stessa casella, %s", (_, inverso) => {
  it("l'inoltro (FW:) e la copia nel calendario di un'azienda (RE:) sono una riunione sola, quella dell'azienda", async () => {
    const letti = await pagina(inOrdine([INOLTRATO, RISPOSTA], inverso));
    expect(letti.map((e) => [e.alias, e.title])).toEqual([["Beta", "RE: Riunione di prova"]]);
  });

  it("l'inoltro cede all'invito senza prefisso e ne prende il link se l'altro non ce l'ha", async () => {
    const originale: EventoFinto = { ...RISPOSTA, title: "Riunione di prova", online_url: null };
    const letti = await pagina(inOrdine([INOLTRATO, originale], inverso));
    expect(letti.map((e) => [e.title, e.online_url])).toEqual([["Riunione di prova", LINK]]);
  });
});

describe.each(ordini)("caselle diverse, %s", (_, inverso) => {
  const originale: EventoFinto = { ...INOLTRATO, account: GAMMA, calendar: `${GAMMA} · Calendar`, title: "Riunione di prova", organizer_email: GAMMA, attendees: [] };

  it("l'inoltro in una casella e la riunione nell'altra sono una riunione sola, quella senza prefisso", async () => {
    const letti = await pagina(inOrdine([INOLTRATO, originale], inverso));
    expect(letti.map((e) => [e.account, e.title])).toEqual([[GAMMA, "Riunione di prova"]]);
  });

  it("l'inoltro e la risposta (RE:) sono una riunione sola, quella che non è un inoltro", async () => {
    const risposta: EventoFinto = { ...originale, title: "RE: Riunione di prova" };
    const letti = await pagina(inOrdine([INOLTRATO, risposta], inverso));
    expect(letti.map((e) => [e.account, e.title])).toEqual([[GAMMA, "RE: Riunione di prova"]]);
  });
});

describe("quello che è diverso resta diverso", () => {
  it("stesso titolo a un'altra ora", async () => {
    const letti = await pagina([RISPOSTA, { ...INOLTRATO, start: "2026-10-16T17:00:00", end: "2026-10-16T17:50:00" }]);
    expect(letti.map((e) => e.start)).toEqual(["2026-10-16T16:00:00", "2026-10-16T17:00:00"]);
  });

  it("stessa ora con un altro titolo", async () => {
    const letti = await pagina([RISPOSTA, { ...INOLTRATO, title: "FW: Un'altra riunione" }]);
    expect(letti.map((e) => e.title).sort()).toEqual(["FW: Un'altra riunione", "RE: Riunione di prova"]);
  });

  it("le lettere iniziali di una parola non sono un prefisso", async () => {
    const letti = await pagina([{ ...RISPOSTA, title: "Rete: guasto" }, { ...RISPOSTA, title: "Trasferta: guasto" }, { ...RISPOSTA, title: "guasto" }]);
    expect(letti.map((e) => e.title).sort()).toEqual(["Rete: guasto", "Trasferta: guasto", "guasto"]);
  });
});
