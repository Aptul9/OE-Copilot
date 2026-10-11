import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama, type NomeMetodoTs } from "@/lib/core/api";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";

// La lettura del gateway contro quella del servizio ui (Python): le stesse chiamate sulla stessa cartella dei dati, con lo stesso
// agente finto, devono dare le stesse risposte e la stessa cartella a fine sequenza. Le risposte "giuste" le ha registrate il
// servizio ui (server/strumenti/prova_ui_golden.py, golden/ui-lettura.json); la Action controlla che il servizio ui le dia ancora.

const GOLDEN = fileURLToPath(new URL("../../strumenti/golden/", import.meta.url));
const leggi = (...p: string[]) => JSON.parse(fs.readFileSync(path.join(GOLDEN, ...p), "utf-8"));
const CHIAVE = "chiave-di-prova";
const ORA_FISSA_MS = 1_700_000_000_000;

type Modo = { servizio?: unknown; stati?: Record<string, Record<string, unknown>>; http?: number };
type Scenario = { profilo: string; modo: string; nome: string; metodo: NomeMetodoTs; args: unknown[] };

const agenteRegistrato = leggi("agente.json");
const fixture = leggi("ui", "fixture.json");
const registrato = leggi("ui-lettura.json");
const norma = (testo: string) => testo.replace(/(Letto )?alle \d\d:\d\d/g, "$1alle HH:MM").replace(/http:\/\/127\.0\.0\.1:\d+/g, "http://127.0.0.1:PORTA");
const normalizza = (v: unknown) => JSON.parse(norma(JSON.stringify(v)));
const unico = (testo: string) => testo.replace(/\r\n/g, "\n");

function ascolta(gestore: http.RequestListener): Promise<{ server: http.Server; url: string }> {
  return new Promise((ok) => {
    const server = http.createServer(gestore);
    server.listen(0, "127.0.0.1", () => ok({ server, url: `http://127.0.0.1:${(server.address() as { port: number }).port}` }));
  });
}

describe.each(Object.keys(fixture.profili))("gateway contro il servizio ui, profilo %s", (profilo) => {
  const stato = { modo: "normale" };
  let agente: { server: http.Server; url: string };
  let ics: { server: http.Server; url: string };
  let dati: string;
  const nomiIcs = Object.fromEntries(["festivita", "riunioni"].map((n) => [unico(fs.readFileSync(path.join(GOLDEN, "ics", n + ".ics"), "utf-8")), n]));

  beforeAll(async () => {
    ics = await ascolta((req, res) => {
      const file = path.join(GOLDEN, "ics", path.basename(req.url ?? ""));
      if (!fs.existsSync(file)) return void res.writeHead(404).end();
      res.writeHead(200, { "Content-Type": "text/calendar" }).end(fs.readFileSync(file));
    });
    agente = await ascolta((req, res) => {
      const risposta = (codice: number, corpo: unknown) => void res.writeHead(codice, { "Content-Type": "application/json; charset=utf-8" }).end(JSON.stringify(corpo));
      const modo: Modo = fixture.modi[stato.modo];
      if (modo.http) return risposta(modo.http, { errore: "finto" });
      if (req.headers.authorization !== "Bearer " + CHIAVE) return risposta(401, { errore: "non autorizzato" });
      const url = new URL(req.url ?? "", "http://x");
      if (req.method === "POST" && url.pathname === "/api/ics") {
        let corpo = "";
        req.on("data", (c) => (corpo += c));
        req.on("end", () => {
          const r = JSON.parse(corpo);
          const nome = nomiIcs[unico(r.testo)];
          const voce = nome && agenteRegistrato.ics[nome].eventi[`${r.inizio}|${r.fine}`];
          risposta(200, voce ?? { ok: false, errore: "calendario o finestra non registrati" });
        });
        return;
      }
      let dato;
      if (url.pathname === "/api/calendari") dato = structuredClone(agenteRegistrato.calendari);
      else if (url.pathname === "/api/eventi") {
        const chiave = `${url.searchParams.get("inizio")}|${url.searchParams.get("fine")}`;
        if (!(chiave in agenteRegistrato.eventi)) return risposta(404, { errore: "finestra non registrata" });
        dato = structuredClone(agenteRegistrato.eventi[chiave]);
      } else return risposta(404, { errore: "sconosciuto" });
      if ("servizio" in modo) dato.servizio = modo.servizio;
      for (const c of dato.caselle ?? []) Object.assign(c, modo.stati?.[c.account] ?? {});
      risposta(200, dato);
    });
    // la stessa cartella dei dati che costruisce il servizio ui
    dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-golden-"));
    const scrivi = (nome: string, v: unknown) => fs.writeFileSync(path.join(dati, nome), JSON.stringify(v, null, 2));
    scrivi("config.json", JSON.parse(JSON.stringify(fixture.config).replace(/\{ICS\}/g, ics.url)));
    scrivi("personalizzazione.json", fixture.profili[profilo].personalizzazione);
    scrivi("local_events.json", fixture.locali);
    scrivi("agente_caselle.json", fixture.caselle_note);
    scrivi("events_cache.json", fixture.cache_eventi);
    fs.mkdirSync(path.join(dati, "ics_cache"));
    fs.copyFileSync(path.join(GOLDEN, "ics", "festivita.ics"), path.join(dati, "ics_cache", "builtin__festivita-it.ics"));
    process.env.OE_DATI = dati;
    process.env.AGENTE_URL = agente.url;
    process.env.AGENTE_CHIAVE = CHIAVE;
    fissaOra(ORA_FISSA_MS);
    azzeraMemoria();
    azzeraFeed();
  });

  afterAll(() => {
    fissaOra(null);
    agente.server.close();
    ics.server.close();
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
