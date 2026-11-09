import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto, type EventoFinto } from "./agente-condivisione";

// Un'azienda le cui riunioni arrivano copiate in un'altra casella e che ora ha anche la sua casella: finché questa si legge, le copie non
// servono e l'azienda non compare due volte; se la sua casella va in errore, le copie tornano.

const ALFA = "antonio@alfa.example";
const GAMMA = "antonio@gamma.example";
const EVENTI: EventoFinto[] = [
  { account: ALFA, calendar: `${ALFA} · Calendar`, title: "[Gamma] Riunione", organizer_email: ALFA, start: "2026-10-16T10:00:00", end: "2026-10-16T10:30:00" },
  { account: ALFA, calendar: `${ALFA} · Calendar`, title: "[Gamma] Solo copia", organizer_email: ALFA, start: "2026-10-16T12:00:00", end: "2026-10-16T12:30:00" },
  { account: ALFA, calendar: `${ALFA} · Calendar`, title: "Riunione Alfa", organizer_email: ALFA, start: "2026-10-16T09:00:00", end: "2026-10-16T09:30:00" },
  { account: GAMMA, calendar: `${GAMMA} · Calendar`, title: "Riunione", organizer_email: GAMMA, start: "2026-10-16T10:00:00", end: "2026-10-16T10:30:00" },
];
const INIZIO = "2026-10-16T08:00:00+02:00";
const FINE = "2026-10-17T08:00:00+02:00";
type Evento = { title: string; alias: string };
const eventi = async () => ((await chiama("get_events", [INIZIO, FINE, true])) as { events: Evento[] }).events.map((e) => `${e.alias}: ${e.title}`).sort();
const calendari = async () => ((await chiama("get_calendars", [])) as { calendars: { id: string }[] }).calendars.map((c) => c.id);

let dati: string;
let agente: AgenteFinto | undefined;

beforeEach(async () => {
  dati = cartellaDati({
    domini_aziendali: ["alfa.example", "gamma.example"],
    nomi_per_dominio: { "alfa.example": "Alfa", "gamma.example": "Gamma" },
    aziende_inoltrate: { Gamma: { via: ALFA, prefisso: "^\\[Gamma\\]" } },
  });
  process.env.OE_DATI = dati;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(Date.UTC(2026, 9, 16, 6, 0, 0));
  azzeraFeed();
  agente = await avviaAgenteFinto(EVENTI);
  process.env.AGENTE_URL = agente.url;
  azzeraMemoria();
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

describe("azienda inoltrata con una casella sua", () => {
  it("finché la sua casella si legge: niente calendario delle copie, niente copie, una sola riunione", async () => {
    expect(await calendari()).not.toContain("inoltro::gamma");
    expect(await eventi()).toEqual(["Alfa: Riunione Alfa", "Gamma: Riunione"]);
  });

  it("con la sua casella in errore le copie tornano, anche quelle che la casella non ha", async () => {
    agente!.stati = { [GAMMA]: { stato: "errore", msg: "Accesso scaduto" } };
    expect(await calendari()).toContain("inoltro::gamma");
    expect(await eventi()).toEqual(["Alfa: Riunione Alfa", "Gamma: Riunione", "Gamma: Solo copia"]);
  });
});
