import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { chiaveEliminato, occupato, raccogli } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig } from "@/lib/core/stato";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto } from "./agente-condivisione";

// Elimina sulla scheda di una riunione delle caselle: il telefono si legge e basta, quindi la riunione si nasconde nel gateway
// (config eventi_eliminati) e raccogli la salta, per la pagina, la disponibilità, la condivisione e i promemoria.

const SERCO = "antonio@serco.example";
let dati: string;
let agente: AgenteFinto;

beforeEach(async () => {
  dati = cartellaDati();
  agente = await avviaAgenteFinto([
    { account: SERCO, title: "Riunione di team", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00" },
    { account: SERCO, title: "Change notturno", start: "2026-10-13T22:00:00", end: "2026-10-14T06:00:00" },
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

const titoli = async () => (await raccogli(loadConfig(), "2026-10-13T00:00:00", "2026-10-15T00:00:00"))[0].map((e) => e.title);

describe("chiave di un evento eliminato", () => {
  it("l'orario della pagina (istante) e quello del server (ora di Roma) danno la stessa chiave, con o senza FW: davanti", () => {
    const server = chiaveEliminato({ title: "Riunione di team", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00" });
    expect(server).not.toBeNull();
    expect(chiaveEliminato({ title: "FW: riunione di team ", start: "2026-10-13T07:00:00.000Z", end: "2026-10-13T08:00:00.000Z" })).toBe(server);
    expect(chiaveEliminato({ title: "Riunione di team", start: "2026-10-13T10:00:00", end: "2026-10-13T11:00:00" })).not.toBe(server);
  });
  it("un giorno intero ha la stessa chiave come data o come mezzanotte; orari illeggibili non ne hanno", () => {
    expect(chiaveEliminato({ title: "Ferie", start: "2026-10-13", end: "2026-10-14" })).toBe(chiaveEliminato({ title: "Ferie", start: "2026-10-13T00:00:00", end: "2026-10-14T00:00:00" }));
    expect(chiaveEliminato({ title: "x", start: "ieri" })).toBeNull();
    expect(chiaveEliminato({ title: "x" })).toBeNull();
  });
});

describe("elimina_evento e ripristina_evento", () => {
  it("l'evento eliminato non esce più da raccogli né dalla disponibilità, e ripristinato torna", async () => {
    // come lo manda la pagina: orari già passati all'ora del dispositivo
    const r = (await chiama("elimina_evento", [{ id: "x", title: "Riunione di team", start: "2026-10-13T07:00:00.000Z", end: "2026-10-13T08:00:00.000Z", allDay: false, account: SERCO, alias: "Serco" }])) as {
      ok: boolean;
      chiave: string;
      eventi_eliminati: { chiave: string; title: string; alias: string }[];
    };
    expect(r.ok).toBe(true);
    expect(r.eventi_eliminati).toMatchObject([{ chiave: r.chiave, title: "Riunione di team", alias: "Serco" }]);
    expect(await titoli()).toEqual(["Change notturno"]);
    const [impegni] = await occupato(loadConfig(), { a: 2026, m: 10, g: 13 }, 1);
    expect(impegni).toEqual([["2026-10-13T22:00:00", "2026-10-14T06:00:00"]]);

    // due volte lo stesso evento resta una voce sola
    await chiama("elimina_evento", [{ title: "Riunione di team", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00" }]);
    expect(loadConfig().eventi_eliminati).toHaveLength(1);

    expect(await chiama("ripristina_evento", [r.chiave])).toEqual({ ok: true, eventi_eliminati: [] });
    expect(await titoli()).toEqual(["Riunione di team", "Change notturno"]);
  });

  it("un evento senza orari leggibili non si elimina e non cambia niente", async () => {
    expect(await chiama("elimina_evento", [{ title: "x", start: "domani" }])).toEqual({ ok: false, error: "Evento non riconosciuto" });
    expect(await chiama("elimina_evento", ["non un evento"])).toEqual({ ok: false, error: "Evento non riconosciuto" });
    expect(loadConfig().eventi_eliminati).toBeUndefined();
    expect(await titoli()).toHaveLength(2);
  });
});
