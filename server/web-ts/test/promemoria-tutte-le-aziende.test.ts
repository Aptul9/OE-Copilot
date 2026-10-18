import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { dipendenzeReali } from "@/lib/core/promemoria";
import { loadConfig } from "@/lib/core/stato";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto, type EventoFinto } from "./agente-condivisione";

// Gli interruttori della barra laterale filtrano la vista, non i promemoria: spegnere un'azienda (o "mostra solo" un'altra) da un dispositivo
// non deve zittire i promemoria delle sue riunioni sugli altri. La pagina chiede get_events(inizio, fine); le app Android e Windows chiedono
// get_events(inizio, fine, true); il motore del server legge da solo.

const ALFA = "antonio@alfa.example";
const GAMMA = "antonio@gamma.example";
const EVENTI: EventoFinto[] = [
  { account: ALFA, calendar: `${ALFA} · Calendar`, title: "Riunione Alfa", organizer_email: ALFA, start: "2026-10-16T10:00:00", end: "2026-10-16T10:30:00" },
  { account: GAMMA, calendar: `${GAMMA} · Calendar`, title: "Riunione Gamma", organizer_email: GAMMA, start: "2026-10-16T11:00:00", end: "2026-10-16T11:30:00" },
];
const INIZIO = "2026-10-16T08:00:00+02:00";
const FINE = "2026-10-17T08:00:00+02:00";
const titoli = (r: unknown) => ((r as { events: { title: string }[] }).events ?? []).map((e) => e.title).sort();

let dati: string;
let agente: AgenteFinto | undefined;

beforeEach(async () => {
  dati = cartellaDati({ domini_aziendali: ["alfa.example", "gamma.example"], nomi_per_dominio: { "alfa.example": "Alfa", "gamma.example": "Gamma" } });
  process.env.OE_DATI = dati;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(Date.UTC(2026, 9, 16, 6, 0, 0));
  azzeraFeed();
  agente = await avviaAgenteFinto(EVENTI);
  process.env.AGENTE_URL = agente.url;
  azzeraMemoria();
  await chiama("get_calendars", []);
  await chiama("set_azienda_enabled", ["Gamma", false]);
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

describe("promemoria di tutte le aziende", () => {
  it("la pagina non vede l'azienda spenta", async () => {
    expect(titoli(await chiama("get_events", [INIZIO, FINE]))).toEqual(["Riunione Alfa"]);
  });

  it("le app, con il terzo argomento, vedono anche l'azienda spenta", async () => {
    expect(titoli(await chiama("get_events", [INIZIO, FINE, true]))).toEqual(["Riunione Alfa", "Riunione Gamma"]);
  });

  it("il motore dei promemoria del server legge anche l'azienda spenta", async () => {
    const { eventi } = await dipendenzeReali().leggiEventi(INIZIO, FINE);
    expect(eventi.map((e) => e.title).sort()).toEqual(["Riunione Alfa", "Riunione Gamma"]);
  });

  it("l'interruttore resta spento, nella configurazione e per la pagina", async () => {
    await chiama("get_events", [INIZIO, FINE, true]);
    await dipendenzeReali().leggiEventi(INIZIO, FINE);
    expect((loadConfig().disabled_calendars as string[]).length).toBeGreaterThan(0);
    expect(titoli(await chiama("get_events", [INIZIO, FINE]))).toEqual(["Riunione Alfa"]);
  });
});
