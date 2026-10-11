import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Quello che le prove del gateway contro il servizio ui (Python) hanno in comune: i file registrati, l'agente finto che ripete le risposte
// registrate, il server dei calendari ICS di prova e la cartella dei dati costruita dal file di scenari (golden/ui/fixture.json).

export const GOLDEN = fileURLToPath(new URL("../../strumenti/golden/", import.meta.url));
export const leggi = (...p: string[]) => JSON.parse(fs.readFileSync(path.join(GOLDEN, ...p), "utf-8"));
export const CHIAVE = "chiave-di-prova";
export const ORA_FISSA_MS = 1_700_000_000_000;

export type Modo = { servizio?: unknown; stati?: Record<string, Record<string, unknown>>; http?: number };

export const agenteRegistrato = leggi("agente.json");
export const fixture = leggi("ui", "fixture.json");

// Le ore del giorno cambiano a ogni lettura, la porta del server ICS a ogni giro e con lei l'id (sha1 del link) dei calendari aggiunti
const norma = (testo: string) =>
  testo
    .replace(/(Letto )?alle \d\d:\d\d/g, "$1alle HH:MM")
    .replace(/http:\/\/127\.0\.0\.1:\d+/g, "http://127.0.0.1:PORTA")
    .replace(/ics_[0-9a-f]{12}/g, "ics_HASH");
export const normalizza = (v: unknown) => JSON.parse(norma(JSON.stringify(v)));
const unico = (testo: string) => testo.replace(/\r\n/g, "\n");

export function ascolta(gestore: http.RequestListener): Promise<{ server: http.Server; url: string }> {
  return new Promise((ok) => {
    const server = http.createServer(gestore);
    server.listen(0, "127.0.0.1", () => ok({ server, url: `http://127.0.0.1:${(server.address() as { port: number }).port}` }));
  });
}

/** Il server dei calendari ICS di prova: i file di golden/ics; un nome che non c'è dà 404 con la frase di http.server di Python. */
export const avviaIcs = () =>
  ascolta((req, res) => {
    const file = path.join(GOLDEN, "ics", path.basename(req.url ?? ""));
    if (!fs.existsSync(file)) return void res.writeHead(404, "File not found").end();
    // con il charset, come un server vero: senza, un testo text/* si legge come ISO-8859-1
    res.writeHead(200, { "Content-Type": "text/calendar; charset=utf-8" }).end(fs.readFileSync(file));
  });

const nomiIcs = Object.fromEntries(
  ["festivita", "riunioni", "html", "rotto"].map((n) => [unico(fs.readFileSync(path.join(GOLDEN, "ics", n + ".ics"), "utf-8")), n]),
);

/** L'agente finto: le risposte registrate dall'agente vero, con lo stato che lo scenario chiede (`stato.modo`). */
export const avviaAgente = (stato: { modo: string }) =>
  ascolta((req, res) => {
    const risposta = (codice: number, corpo: unknown) =>
      void res.writeHead(codice, { "Content-Type": "application/json; charset=utf-8" }).end(JSON.stringify(corpo));
    const modo: Modo = fixture.modi[stato.modo];
    if (modo.http) return risposta(modo.http, { errore: "finto" });
    if (req.headers.authorization !== "Bearer " + CHIAVE) return risposta(401, { errore: "non autorizzato" });
    const url = new URL(req.url ?? "", "http://x");
    if (req.method === "POST" && url.pathname === "/api/ics") {
      let corpo = "";
      req.on("data", (c) => (corpo += c));
      req.on("end", () => {
        const r = JSON.parse(corpo);
        const voce = agenteRegistrato.ics[nomiIcs[unico(r.testo)]];
        if (!voce) return risposta(200, { ok: false, errore: "calendario non registrato" });
        // senza intervallo il testo si valida soltanto
        risposta(200, r.inizio === undefined ? voce.valida : (voce.eventi[`${r.inizio}|${r.fine}`] ?? { ok: false, errore: "finestra non registrata" }));
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

/** La cartella dei dati che costruisce anche il servizio ui (prova_ui_golden.py, prepara_dati). */
export function preparaDati(profilo: string, icsUrl: string): string {
  const dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-golden-"));
  const scrivi = (nome: string, v: unknown) => fs.writeFileSync(path.join(dati, nome), JSON.stringify(v, null, 2));
  scrivi("config.json", JSON.parse(JSON.stringify(fixture.config).replace(/\{ICS\}/g, icsUrl)));
  scrivi("personalizzazione.json", fixture.profili[profilo].personalizzazione);
  scrivi("local_events.json", fixture.locali);
  scrivi("agente_caselle.json", fixture.caselle_note);
  scrivi("events_cache.json", fixture.cache_eventi);
  fs.mkdirSync(path.join(dati, "ics_cache"));
  fs.copyFileSync(path.join(GOLDEN, "ics", "festivita.ics"), path.join(dati, "ics_cache", "builtin__festivita-it.ics"));
  return dati;
}
