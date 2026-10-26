import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiudiArchivio, leggiRigheLocali } from "@/lib/core/archivio";
import { occupato, raccogli } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { aggiungiEvento } from "@/lib/core/locali";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig } from "@/lib/core/stato";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto } from "./agente-condivisione";

// Un blocco di tempo voluto conta nella disponibilità qualunque sia la durata: un evento personale con "Blocca il tempo", o un giorno intero
// delle caselle segnato "Occupato" in Outlook o Google (occupato, dall'agente), che resta anche se i giorni interi delle caselle sono nascosti.

const SERCO = "antonio@serco.example";
let dati: string;
let agente: AgenteFinto;

beforeEach(async () => {
  dati = cartellaDati();
  agente = await avviaAgenteFinto([
    { account: SERCO, title: "Ferie da Outlook", start: "2026-10-14", end: "2026-10-15", allDay: true, occupato: true },
    { account: SERCO, title: "Promemoria", start: "2026-10-15", end: "2026-10-16", allDay: true, occupato: false },
    { account: SERCO, title: "Riunione", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00" },
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

describe("blocchi di tempo", () => {
  it("la scelta si salva nell'evento personale solo se la pagina la manda", () => {
    aggiungiEvento({ title: "Trasloco", start: "2026-10-16", end: "2026-10-17", allDay: true, location: "", notes: "", blocca: true });
    aggiungiEvento({ title: "Compleanno", start: "2026-10-17", end: "2026-10-18", allDay: true, location: "", notes: "", blocca: false });
    aggiungiEvento({ title: "Vecchio", start: "2026-10-18T09:00", end: "2026-10-18T10:00", allDay: false, location: "", notes: "" });
    expect(leggiRigheLocali().map((e) => [e.title, e.blocca])).toEqual([["Trasloco", true], ["Compleanno", false], ["Vecchio", undefined]]);
  });

  it("disponibilità: il giorno personale bloccato e il giorno Occupato di Outlook contano tutti, il compleanno e il promemoria no", async () => {
    aggiungiEvento({ title: "Trasloco", start: "2026-10-16", end: "2026-10-17", allDay: true, location: "", notes: "", blocca: true });
    aggiungiEvento({ title: "Compleanno", start: "2026-10-17", end: "2026-10-18", allDay: true, location: "", notes: "", blocca: false });
    const [impegni] = await occupato(loadConfig(), { a: 2026, m: 10, g: 13 }, 7);
    expect(impegni).toEqual([
      ["2026-10-13T09:00:00", "2026-10-13T10:00:00"],
      ["2026-10-14T00:00:00", "2026-10-15T00:00:00"],
      ["2026-10-16T00:00:00", "2026-10-17T00:00:00"],
    ]);
  });

  it("i giorni interi delle caselle restano nascosti nella pagina, tranne quello segnato Occupato", async () => {
    const [eventi] = await raccogli(loadConfig(), "2026-10-13T00:00:00", "2026-10-20T00:00:00");
    expect(eventi.map((e) => e.title)).toEqual(["Riunione", "Ferie da Outlook"]);
  });
});
