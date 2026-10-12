import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiudiArchivio } from "@/lib/core/archivio";
import { apiTs } from "@/lib/core/api";
import { occupato, raccogli } from "@/lib/core/eventi";
import { azzeraFeed, percorsoCache, statoFeed, testoCalendario } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { FESTIVITA_ID } from "@/lib/core/personalizzazione";
import { loadConfig, modificaConfig } from "@/lib/core/stato";
import { costruisciIcs } from "@/lib/ics-uscita";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto } from "./agente-condivisione";

// Il lato di chi importa (lib/core/ics.ts, eventi.ts): un calendario condiviso da un altro OE Copilot si legge come ogni calendario da link, ma i suoi
// eventi restano segnati e non rendono occupata la pagina pubblica di chi li ha importati; un link revocato (404, 410) non lascia in giro la sua ultima
// copia; il nome del calendario, se non lo si scrive, è quello che l'altro ha pubblicato.

const ORA = Date.UTC(2026, 9, 12, 8, 0, 0);
const COPIA_CONDIVISA = costruisciIcs({ nome: "Calendario di Antonio", sale: "00112233445566778899aabbccddeeff", livello: "occupato", stamp: ORA, eventi: [] });
const COPIA_COMUNE = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Altro//IT\r\nX-WR-CALNAME:Un calendario qualunque\r\nEND:VCALENDAR\r\n";
const COPIA_FESTE = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Feste//IT\r\nX-WR-CALNAME:Feste\r\nEND:VCALENDAR\r\n";

let dati: string;
let agente: AgenteFinto;
let server: http.Server;
let url: string;
let risposta: { stato: number; corpo: string; chiudi?: boolean };

beforeEach(async () => {
  dati = cartellaDati();
  agente = await avviaAgenteFinto([]);
  risposta = { stato: 200, corpo: COPIA_CONDIVISA };
  server = http.createServer((req, res) => {
    if (risposta.chiudi) return void req.socket.destroy();
    res.writeHead(risposta.stato, { "Content-Type": "text/calendar; charset=utf-8" }).end(risposta.stato === 200 ? risposta.corpo : "no");
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  url = `http://127.0.0.1:${(server.address() as { port: number }).port}/condividi/prova.ics`;
  process.env.OE_DATI = dati;
  process.env.AGENTE_URL = agente.url;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(ORA);
  azzeraMemoria();
  azzeraFeed();
  // la copia delle festività è già "scaricata": nessuna rete
  fs.mkdirSync(path.join(dati, "ics_cache"), { recursive: true });
  fs.writeFileSync(percorsoCache(FESTIVITA_ID), COPIA_FESTE);
  agente.ics.push(
    { uid: "u1", inizio: "2026-10-13T09:00:00", fine: "2026-10-13T10:00:00", titolo: "Occupato", perTesto: "Calendario di Antonio" },
    { uid: "u2", inizio: "2026-10-13T09:00:00", fine: "2026-10-13T10:00:00", titolo: "Riunione", perTesto: "Un calendario qualunque" },
  );
});

afterEach(async () => {
  await agente.chiudi();
  await new Promise<void>((ok) => {
    server.closeAllConnections();
    server.close(() => ok());
  });
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE"]) delete process.env[n];
  vi.restoreAllMocks();
});

const iscrivi = (nome = "Altro") => modificaConfig((c) => void (c.ics_calendars = [{ id: "ics_altro", name: nome, url }]));
const eventiFeed = async () => (await raccogli(loadConfig(), "2026-10-12T00:00:00", "2026-10-19T00:00:00"))[0].filter((e) => String(e.id).startsWith("ics_altro|"));
const copia = () => percorsoCache("ics_altro");
const invecchia = () => fissaOra(Date.now() + 31 * 60_000);

describe("aggiungere il link di un altro OE Copilot", () => {
  it("senza nome prende quello che l'altro ha pubblicato (X-WR-CALNAME); con un nome, il suo", async () => {
    const r = await apiTs.add_ics_calendar("", url);
    expect(r).toMatchObject({ ok: true, feed: { name: "Calendario di Antonio" } });
    modificaConfig((c) => void (c.ics_calendars = []));
    const s = await apiTs.add_ics_calendar("Antonio (lavoro)", url);
    expect(s).toMatchObject({ ok: true, feed: { name: "Antonio (lavoro)" } });
  });

  it("il calendario si legge come gli altri: eventi con il titolo che l'altro ha dato (Occupato) e il nome del calendario", async () => {
    iscrivi("Calendario di Antonio");
    const e = await eventiFeed();
    expect(e).toHaveLength(1);
    expect(e[0]).toMatchObject({ title: "Occupato", account: "Calendario di Antonio", start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00", allDay: false });
  });
});

describe("gli eventi di un calendario condiviso restano segnati", () => {
  it("con X-OE-COPILOT-SHARE:1 nel testo: da_condivisione, e non su quelli di un calendario qualunque", async () => {
    iscrivi();
    expect((await eventiFeed())[0].da_condivisione).toBe(true);
    risposta.corpo = COPIA_COMUNE;
    fs.rmSync(copia(), { force: true });
    expect("da_condivisione" in (await eventiFeed())[0]).toBe(false);
  });

  it("la riga si riconosce solo come proprietà: non dentro un testo, non con un altro valore", async () => {
    iscrivi();
    const nome = "X-WR-CALNAME:Un calendario qualunque\r\n";
    for (const finto of [`BEGIN:VCALENDAR\r\n${nome}X-OE-COPILOT-SHARE:0\r\nEND:VCALENDAR\r\n`, `BEGIN:VCALENDAR\r\n${nome}DESCRIPTION:x\r\n  X-OE-COPILOT-SHARE:1\r\nEND:VCALENDAR\r\n`, `BEGIN:VCALENDAR\r\n${nome}X-OE-COPILOT-SHARE:10\r\nEND:VCALENDAR\r\n`]) {
      risposta.corpo = finto;
      fs.rmSync(copia(), { force: true });
      expect("da_condivisione" in (await eventiFeed())[0], finto).toBe(false);
    }
  });
});

describe("la riga da sola non basta: serve anche il PRODID di OE Copilot", () => {
  const nome = "X-WR-CALNAME:Un calendario qualunque\r\n";
  const PRODID = "-//OE Copilot//Condivisione//IT";
  const corpo = (prodid: string | null, riga: string) => `BEGIN:VCALENDAR\r\nVERSION:2.0\r\n${prodid === null ? "" : `PRODID:${prodid}\r\n`}${nome}${riga}END:VCALENDAR\r\n`;
  const MARCA = "X-OE-COPILOT-SHARE:1\r\n";
  const casi: [string, string | null, string, boolean][] = [
    ["tutti e due", PRODID, MARCA, true],
    ["tutti e due, con uno spazio in coda al PRODID", PRODID + " ", MARCA, true],
    ["la riga e un altro PRODID: chi non è OE Copilot non si fa riconoscere così", "-//Altro//IT", MARCA, false],
    ["la riga e il PRODID di un'altra versione del prodotto", "-//OE Copilot//Condivisione//EN", MARCA, false],
    ["la riga e il PRODID di un altro prodotto di OE Copilot", "-//OE Copilot//Calendario//IT", MARCA, false],
    ["la riga e nessun PRODID", null, MARCA, false],
    ["il solo PRODID", PRODID, "", false],
    ["il PRODID in un altro punto della riga", "x " + PRODID, MARCA, false],
  ];

  it.each(casi)("%s", async (_nome, prodid, riga, atteso) => {
    iscrivi();
    risposta.corpo = corpo(prodid, riga);
    fs.rmSync(copia(), { force: true });
    const e = await eventiFeed();
    expect(e).toHaveLength(1);
    expect("da_condivisione" in e[0]).toBe(atteso);
  });

  it("un calendario che porta solo la riga non nasconde i suoi eventi alla pagina pubblica di chi lo ha importato: contano come impegni", async () => {
    iscrivi();
    risposta.corpo = corpo("-//Altro//IT", MARCA);
    fs.rmSync(copia(), { force: true });
    expect((await occupato(loadConfig(), { a: 2026, m: 10, g: 12 }, 7))[0]).toEqual([["2026-10-13T09:00:00", "2026-10-13T10:00:00"]]);
  });
});

describe("la pagina pubblica di chi importa non conta il calendario condiviso", () => {
  const giorni = async () => (await occupato(loadConfig(), { a: 2026, m: 10, g: 12 }, 7))[0];

  it("un calendario condiviso da un altro OE Copilot: nessun blocco occupato", async () => {
    iscrivi();
    expect(await giorni()).toEqual([]);
    // ma nel calendario di chi lo ha importato si vede
    expect(await eventiFeed()).toHaveLength(1);
  });

  it("un calendario da link qualunque (gli impegni di chi lo ha) conta come prima", async () => {
    risposta.corpo = COPIA_COMUNE;
    iscrivi();
    expect(await giorni()).toEqual([["2026-10-13T09:00:00", "2026-10-13T10:00:00"]]);
  });
});

describe("un link revocato non lascia la sua ultima copia", () => {
  it("404: dopo la scadenza della copia (30 minuti) gli eventi spariscono, la copia si toglie e il pallino è rosso con il motivo", async () => {
    iscrivi();
    expect(await eventiFeed()).toHaveLength(1);
    expect(fs.existsSync(copia())).toBe(true);
    risposta.stato = 404;
    // dentro i 30 minuti non si chiede niente: si legge la copia
    expect(await eventiFeed()).toHaveLength(1);
    invecchia();
    expect(await eventiFeed()).toEqual([]);
    expect(fs.existsSync(copia())).toBe(false);
    expect(statoFeed().ics_altro).toEqual({ stato: "errore", msg: "il link non risponde più (404): è stato revocato o cambiato da chi lo aveva dato" });
    // e a ogni lettura dopo si riprova (un link rimesso a posto torna da solo)
    risposta.stato = 200;
    expect(await eventiFeed()).toHaveLength(1);
    expect(statoFeed().ics_altro.stato).toBe("ok");
  });

  it("410 uguale", async () => {
    iscrivi();
    await eventiFeed();
    risposta.stato = 410;
    invecchia();
    expect(await eventiFeed()).toEqual([]);
    expect(statoFeed().ics_altro.msg).toMatch(/\(410\)/);
  });

  it("l'errore e il motivo non portano l'indirizzo (che è la chiave del calendario)", async () => {
    iscrivi();
    await eventiFeed();
    risposta.stato = 404;
    invecchia();
    const r = await apiTs.get_events("2026-10-12T00:00:00", "2026-10-19T00:00:00");
    expect(JSON.stringify(r.errors)).not.toContain(url);
    expect(JSON.stringify(r.errors)).toContain("il link non risponde più (404)");
  });

  it("gli altri guasti tengono la copia, come sempre (meglio dati vecchi che nessun dato): 500, 503, la rete che cade", async () => {
    iscrivi();
    await eventiFeed();
    for (const stato of [500, 502, 503]) {
      risposta.stato = stato;
      invecchia();
      expect(await eventiFeed(), String(stato)).toHaveLength(1);
      expect(statoFeed().ics_altro.stato).toBe("avviso");
      expect(statoFeed().ics_altro.msg).toContain("aggiornamento non riuscito");
      expect(fs.existsSync(copia())).toBe(true);
    }
    risposta.chiudi = true;
    invecchia();
    expect(await eventiFeed()).toHaveLength(1);
    expect(statoFeed().ics_altro.stato).toBe("avviso");
  });

  it("senza una copia e con un 404: errore, nessun evento, e niente da togliere", async () => {
    risposta.stato = 404;
    iscrivi();
    expect(await eventiFeed()).toEqual([]);
    expect(statoFeed().ics_altro.stato).toBe("errore");
  });

  it("le festività non seguono la regola: un 404 di Google è un guasto, la copia resta", async () => {
    const urlFesta = (await import("@/lib/core/personalizzazione")).FESTIVITA_URL;
    const spia = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("no", { status: 404, statusText: "Not Found" }));
    invecchia();
    fs.utimesSync(percorsoCache(FESTIVITA_ID), new Date(ORA - 3_600_000), new Date(ORA - 3_600_000));
    const [testo, errore] = await testoCalendario(FESTIVITA_ID, urlFesta);
    spia.mockRestore();
    expect(testo).toBe(COPIA_FESTE);
    expect(errore).toContain("aggiornamento non riuscito");
    expect(fs.existsSync(percorsoCache(FESTIVITA_ID))).toBe(true);
  });

  it("aggiungere un link che dà 404 dice ancora quello di prima (Il link non risponde: ...)", async () => {
    risposta.stato = 404;
    const r = await apiTs.add_ics_calendar("x", url);
    expect(r).toMatchObject({ ok: false });
    expect((r as { error: string }).error).toMatch(/^Il link non risponde: 404 Client Error: /);
  });
});
