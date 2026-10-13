import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiudiArchivio } from "@/lib/core/archivio";
import { ANNULLATO_RE } from "@/lib/core/caselle";
import { occupato, raccogli } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig } from "@/lib/core/stato";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto } from "./agente-condivisione";

// Una riunione annullata dall'organizzatore resta nel calendario di Outlook con "Canceled:" (o "Annullato:") nel titolo: non deve uscire da raccogli,
// quindi né dalla pagina né dalla disponibilità, dalla condivisione e dai promemoria.

const SERCO = "antonio@serco.example";
let dati: string;
let agente: AgenteFinto;

beforeEach(async () => {
  dati = cartellaDati();
  agente = await avviaAgenteFinto([
    { account: SERCO, title: "Riunione di team", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00" },
    { account: SERCO, title: "Canceled: Riunione di team", start: "2026-10-13T11:00:00", end: "2026-10-13T12:00:00" },
    { account: SERCO, title: "Annullato: Revisione", start: "2026-10-13T14:00:00", end: "2026-10-13T15:00:00" },
    { account: SERCO, title: "Il Canceled: nel mezzo non conta", start: "2026-10-13T16:00:00", end: "2026-10-13T17:00:00" },
  ]);
  process.env.OE_DATI = dati;
  process.env.AGENTE_URL = agente.url;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(Date.UTC(2026, 9, 13, 6, 0, 0));
  azzeraMemoria();
  azzeraFeed();
});

afterEach(async () => {
  await agente.chiudi();
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE"]) delete process.env[n];
  vi.restoreAllMocks();
});

describe("titolo di una riunione annullata", () => {
  it.each(["Canceled: x", "Cancelled: x", "CANCELED: x", "  canceled : x", "Annullato: x", "Annullata: x", "Cancellato: x"])("%s", (t) => expect(ANNULLATO_RE.test(t)).toBe(true));
  it.each(["Canceled", "Riunione canceled: x", "Canceledx: y", "Annullamento ferie", "Cancellazione: x"])("%s resta", (t) => expect(ANNULLATO_RE.test(t)).toBe(false));
});

describe("le riunioni annullate non escono", () => {
  it("raccogli le toglie e tiene le altre", async () => {
    const [eventi] = await raccogli(loadConfig(), "2026-10-13T00:00:00", "2026-10-14T00:00:00");
    expect(eventi.map((e) => e.title)).toEqual(["Riunione di team", "Il Canceled: nel mezzo non conta"]);
  });

  it("la disponibilità non conta l'ora di una riunione annullata", async () => {
    const [impegni] = await occupato(loadConfig(), { a: 2026, m: 10, g: 13 }, 1);
    expect(impegni).toEqual([
      ["2026-10-13T09:00:00", "2026-10-13T10:00:00"],
      ["2026-10-13T16:00:00", "2026-10-13T17:00:00"],
    ]);
  });
});
