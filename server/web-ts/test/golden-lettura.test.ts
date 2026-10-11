import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama, type NomeMetodoTs } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { avviaAgente, avviaIcs, CHIAVE, fixture, leggi, normalizza, ORA_FISSA_MS, preparaDati } from "./golden-comune";

// La lettura del gateway contro le risposte registrate dal servizio ui (Python): le stesse chiamate sulla stessa cartella dei dati, con
// lo stesso agente finto, devono dare le stesse risposte e la stessa cartella a fine sequenza. Le risposte "giuste" (golden/ui-lettura.json)
// le registrò il servizio ui quando c'era, con server/strumenti/prova_ui_golden.py: l'ultimo commit che li contiene è 142e789. Oggi il
// servizio non gira più e nessuno le rigenera: sono dati registrati, e una differenza è un comportamento del gateway che è cambiato.
// La configurazione e gli eventi personali della cartella sono quelli di prima: il gateway li importa nel suo database alla prima
// chiamata e da lì in poi li riscrive uguali ai file che scriveva Python.

const registrato = leggi("ui-lettura.json");

type Scenario = { profilo: string; modo: string; nome: string; metodo: NomeMetodoTs; args: unknown[] };

describe.each(Object.keys(fixture.profili))("gateway contro le risposte registrate, profilo %s", (profilo) => {
  const stato = { modo: "normale" };
  let agente: { server: http.Server; url: string };
  let ics: { server: http.Server; url: string };
  let dati: string;

  beforeAll(async () => {
    ics = await avviaIcs();
    agente = await avviaAgente(stato);
    dati = preparaDati(profilo, ics.url);
    process.env.OE_DATI = dati;
    process.env.AGENTE_URL = agente.url;
    process.env.AGENTE_CHIAVE = CHIAVE;
    delete process.env.GATEWAY_DB;
    fissaOra(ORA_FISSA_MS);
    azzeraMemoria();
    azzeraFeed();
  });

  afterAll(() => {
    fissaOra(null);
    agente.server.close();
    ics.server.close();
    chiudiArchivio();
    fs.rmSync(dati, { recursive: true, force: true });
    delete process.env.OE_DATI;
    delete process.env.AGENTE_URL;
    delete process.env.AGENTE_CHIAVE;
  });

  // le chiamate vanno in ordine, sulla stessa cartella: ognuna lascia dietro di sé colori assegnati e copie, come in produzione
  for (const s of (fixture.scenari as Scenario[]).filter((x) => x.profilo === profilo)) {
    it(s.nome, async () => {
      stato.modo = s.modo;
      const risposta = normalizza(await chiama(s.metodo, s.args));
      expect(risposta).toEqual(registrato[profilo].risposte[s.nome]);
    });
  }

  it("la cartella dei dati a fine sequenza", () => {
    const leggiDati = (n: string) => JSON.parse(fs.readFileSync(path.join(dati, n), "utf-8"));
    const cache = leggiDati("events_cache.json") as Record<string, { stamp?: number; eventi?: unknown[] }>;
    const finale = normalizza({
      "config.json": leggiDati("config.json"),
      "agente_caselle.json": leggiDati("agente_caselle.json"),
      "local_events.json": leggiDati("local_events.json"),
      "events_cache.json": Object.fromEntries(Object.entries(cache).map(([k, v]) => [k, { stamp: v.stamp, eventi: (v.eventi ?? []).length }])),
      "ics_cache": fs.readdirSync(path.join(dati, "ics_cache")).sort(),
    });
    expect(finale).toEqual(registrato[profilo].dati);
  });
});
