import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio, leggiRigheLocali } from "@/lib/core/archivio";
import { occupato } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { aggiungiEvento } from "@/lib/core/locali";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig } from "@/lib/core/stato";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto, type EventoFinto } from "./agente-condivisione";

// "A chi" di un evento personale: i gruppi dei calendari per cui è. Nella vista di un gruppo (mostra_solo_gruppo: accese solo le sue aziende, e
// quindi spento il calendario degli eventi personali) escono quelli fatti per quel gruppo; in quella di tutti ognuno esce una volta, anche se è
// per due gruppi; e conta nella disponibilità del gruppo (/disponibilita?gruppo=).

const ALFA = "antonio@alfa.example";
const GAMMA = "samuele@gamma.example";
const EVENTI: EventoFinto[] = [
  { account: ALFA, calendar: `${ALFA} · Calendar`, title: "Riunione Alfa", organizer_email: ALFA, start: "2026-10-16T09:00:00", end: "2026-10-16T09:30:00" },
  { account: GAMMA, calendar: `${GAMMA} · Calendar`, title: "Riunione Gamma", organizer_email: GAMMA, start: "2026-10-16T09:30:00", end: "2026-10-16T10:00:00" },
];
const INIZIO = "2026-10-16T08:00:00+02:00";
const FINE = "2026-10-17T08:00:00+02:00";
const titoli = (r: unknown) => ((r as { events: { title: string }[] }).events ?? []).map((e) => e.title).sort();
const evento = (title: string, ora: string, gruppi?: unknown) =>
  aggiungiEvento({ title, start: `2026-10-16T${ora}:00`, end: `2026-10-16T${ora.slice(0, 2)}:45`, allDay: false, location: "", notes: "", blocca: true, ...(gruppi === undefined ? {} : { gruppi }) });

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
  await chiama("save_config", [{ gruppi_calendari: { Antonio: ["Alfa"], Samuele: ["Gamma"] } }]);
  evento("Solo Antonio", "11:00", ["Antonio"]);
  evento("Solo Samuele", "12:00", ["Samuele"]);
  evento("Tutti e due", "13:00", ["Antonio", "Samuele"]);
  evento("Di nessuno", "14:00");
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

describe("A chi, sugli eventi personali", () => {
  it("si salva solo con dei gruppi, senza doppioni; un valore che non è un elenco non passa", () => {
    evento("Doppio", "15:00", ["Samuele", "Samuele", " ", "Antonio"]);
    expect(leggiRigheLocali().map((e) => [e.title, e.gruppi])).toEqual([
      ["Solo Antonio", ["Antonio"]],
      ["Solo Samuele", ["Samuele"]],
      ["Tutti e due", ["Antonio", "Samuele"]],
      ["Di nessuno", undefined],
      ["Doppio", ["Samuele", "Antonio"]],
    ]);
    expect(() => evento("Rotto", "16:00", "Samuele")).toThrow("A chi");
  });

  it("vista di tutti: ogni evento una volta, quello per due gruppi compreso", async () => {
    expect(titoli(await chiama("get_events", [INIZIO, FINE]))).toEqual(["Di nessuno", "Riunione Alfa", "Riunione Gamma", "Solo Antonio", "Solo Samuele", "Tutti e due"]);
  });

  it("vista di un gruppo: le sue riunioni e gli eventi fatti per lui", async () => {
    await chiama("mostra_solo_gruppo", ["Antonio"]);
    expect(titoli(await chiama("get_events", [INIZIO, FINE]))).toEqual(["Riunione Alfa", "Solo Antonio", "Tutti e due"]);
    await chiama("mostra_solo_gruppo", ["Samuele"]);
    expect(titoli(await chiama("get_events", [INIZIO, FINE]))).toEqual(["Riunione Gamma", "Solo Samuele", "Tutti e due"]);
  });

  it("con gli eventi personali accesi a mano nella vista di un gruppo escono tutti", async () => {
    await chiama("mostra_solo_gruppo", ["Antonio"]);
    const spenti = (loadConfig().disabled_calendars as string[]).filter((id) => id !== "local::altro");
    expect(titoli(await chiama("get_events", [INIZIO, FINE, null, spenti]))).toEqual(["Di nessuno", "Riunione Alfa", "Solo Antonio", "Solo Samuele", "Tutti e due"]);
  });

  it("disponibilità di un gruppo: contano i suoi eventi personali, non quelli dell'altro né quelli di nessuno", async () => {
    const [samuele] = await occupato(loadConfig(), { a: 2026, m: 10, g: 16 }, 1, "Samuele");
    expect(samuele).toEqual([
      ["2026-10-16T09:30:00", "2026-10-16T10:00:00"],
      ["2026-10-16T12:00:00", "2026-10-16T12:45:00"],
      ["2026-10-16T13:00:00", "2026-10-16T13:45:00"],
    ]);
    const [tutti] = await occupato(loadConfig(), { a: 2026, m: 10, g: 16 }, 1);
    expect(tutti.map(([i]) => i.slice(11, 16))).toEqual(["09:00", "11:00", "12:00", "13:00", "14:00"]);
  });
});
