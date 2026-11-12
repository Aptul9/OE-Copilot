import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { chiaveEliminato, occupato, raccogli } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { loadConfig } from "@/lib/core/stato";
import { azzera as azzeraDisponibilita, leggiBlocco } from "@/lib/disponibilita";
import { eliminatiRecenti, eliminatoDa, scadeTra } from "@/lib/oe/logic";
import type { EventoEliminato } from "@/lib/oe/types";
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

describe("la disponibilità pubblica dopo un cambio", () => {
  it("un metodo che cambia gli orari occupati fa rileggere i blocchi tenuti pronti; una lettura no", async () => {
    azzeraDisponibilita();
    let letture = 0;
    const deps = { core: async () => (letture++, { ok: true, impegni: [], festivita: [], completo: true }) };
    await leggiBlocco("2026-10-12", "", deps);
    await chiama("get_config", []);
    await leggiBlocco("2026-10-12", "", deps);
    expect(letture).toBe(1);
    await chiama("elimina_evento", [{ title: "Riunione di team", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00" }]);
    await leggiBlocco("2026-10-12", "", deps);
    expect(letture).toBe(2);
  });
});

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
    expect(r.eventi_eliminati).toMatchObject([{ chiave: r.chiave, title: "Riunione di team", alias: "Serco", eliminato_il: "2026-10-13T06:00:00.000Z" }]);
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

describe("elenco degli eliminati: una settimana per ripristinare", () => {
  const voce = (title: string, fine: string, eliminato_il?: string): EventoEliminato => ({ chiave: title, title, start: fine, end: fine, allDay: false, alias: "Serco", ...(eliminato_il ? { eliminato_il } : {}) });
  const ora = Date.parse("2026-11-07T10:00:00Z");

  it("tiene le voci eliminate negli ultimi 7 giorni, l'ultima in cima; le vecchie senza data contano dalla fine dell'evento", () => {
    const voci = [
      voce("vecchia", "2026-11-20T10:00:00Z", "2026-10-30T10:00:00Z"),
      voce("ieri", "2026-10-01T10:00:00Z", "2026-11-06T09:00:00Z"),
      voce("al limite", "2026-12-01T10:00:00Z", "2026-10-31T10:00:00Z"),
      voce("senza data, finita da poco", "2026-11-03T11:00:00Z"),
      voce("senza data, finita da tempo", "2026-10-30T11:00:00Z"),
      voce("illeggibile", "domani"),
    ];
    expect(eliminatiRecenti(voci, ora).map((v) => v.title)).toEqual(["ieri", "senza data, finita da poco", "al limite"]);
    expect(eliminatiRecenti(undefined, ora)).toEqual([]);
  });

  it("dice quando esce dall'elenco, a giorni di calendario", () => {
    expect(scadeTra(voce("a", "x", "2026-11-07T08:00:00Z"), ora)).toBe("scade tra 7 giorni");
    expect(scadeTra(voce("b", "x", "2026-11-01T09:00:00Z"), ora)).toBe("scade domani");
    expect(scadeTra(voce("c", "x", "2026-10-31T11:00:00Z"), ora)).toBe("scade oggi");
    expect(scadeTra(voce("senza data", "2026-11-05T09:00:00Z"), ora)).toBe("scade tra 5 giorni");
  });

  it("dice da quanto è eliminata, a giorni di calendario", () => {
    expect(eliminatoDa(voce("a", "x", "2026-11-07T08:00:00Z"), ora)).toBe("oggi");
    expect(eliminatoDa(voce("b", "x", "2026-11-06T09:00:00Z"), ora)).toBe("ieri");
    expect(eliminatoDa(voce("c", "x", "2026-11-02T09:00:00Z"), ora)).toBe("5 giorni fa");
    expect(eliminatoDa(voce("d", "x"), ora)).toBe("");
  });
});
