import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama, type NomeMetodoTs } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { impostaReteDiProva } from "@/lib/core/uscita";
import { avviaAgente, avviaIcs, CHIAVE, fixture, leggi, normalizza, ORA_FISSA_MS, preparaDati } from "./golden-comune";
import { PROVE_LOCALI } from "./server-prova";

// La scrittura del gateway contro le risposte registrate dal servizio ui (Python): la stessa sequenza di modifiche (interruttori, gruppi,
// alias, calendari da link, eventi personali, configurazione) sulla stessa cartella dei dati. Dopo ogni chiamata la risposta e lo stato
// (config.json, local_events.json, copie dei calendari ICS) devono essere quelli che ha registrato il servizio ui quando c'era
// (server/strumenti/prova_ui_golden.py, golden/ui-scritture.json: l'ultimo commit che li contiene è 142e789). Oggi il servizio non gira
// più e nessuno le rigenera: sono dati registrati. Lo stato del gateway sta nel database; i due file sono la copia che
// riscrive dopo ogni modifica, nel formato di prima.

const registrato = leggi("ui-scritture.json");

type Passo = { nome: string; metodo: NomeMetodoTs; args: unknown[] };

describe("scritture del gateway contro le risposte registrate", () => {
  const stato = { modo: "normale" };
  let agente: { server: http.Server; url: string };
  let ics: { server: http.Server; url: string };
  let dati: string;
  let ultimoFeed = "";

  const statoDati = () => {
    const leggiDati = (n: string) => JSON.parse(fs.readFileSync(path.join(dati, n), "utf-8"));
    return normalizza({
      "config.json": leggiDati("config.json"),
      "local_events.json": leggiDati("local_events.json"),
      "ics_cache": fs.readdirSync(path.join(dati, "ics_cache")).sort(),
    });
  };

  beforeAll(async () => {
    // il server dei calendari è su 127.0.0.1: la guardia di rete-sicura.ts lo lascia raggiungere solo con il punto di prova
    impostaReteDiProva(PROVE_LOCALI);
    ics = await avviaIcs();
    agente = await avviaAgente(stato);
    dati = preparaDati("base", ics.url);
    process.env.OE_DATI = dati;
    process.env.AGENTE_URL = agente.url;
    process.env.AGENTE_CHIAVE = CHIAVE;
    delete process.env.GATEWAY_DB;
    fissaOra(ORA_FISSA_MS);
    azzeraMemoria();
    azzeraFeed();
  });

  afterAll(() => {
    impostaReteDiProva(undefined);
    fissaOra(null);
    agente.server.close();
    ics.server.close();
    chiudiArchivio();
    fs.rmSync(dati, { recursive: true, force: true });
    delete process.env.OE_DATI;
    delete process.env.AGENTE_URL;
    delete process.env.AGENTE_CHIAVE;
  });

  for (const passo of fixture.scritture as Passo[]) {
    it(passo.nome, async () => {
      const args = JSON.parse(JSON.stringify(passo.args).replace(/\{ICS\}/g, ics.url).replace(/\{ULTIMO_FEED\}/g, ultimoFeed));
      const risposta = (await chiama(passo.metodo, args)) as { ok?: boolean; feed?: { id: string } };
      if (passo.metodo === "add_ics_calendar" && risposta.ok) ultimoFeed = (risposta.feed as { id: string }).id;
      expect({ risposta: normalizza(risposta), stato: statoDati() }).toEqual(registrato[passo.nome]);
    });
  }
});
