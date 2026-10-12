import crypto from "node:crypto";
import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzera, chiaveClient, condividi, type Deps, FALLIMENTI_MAX, FINESTRA_FALLIMENTI_MS, MAX_AGE_S, TTL_CORPO_MS, TTL_GUASTO_MS } from "@/lib/condividi";
import { azzeraMemoria } from "@/lib/core/agente";
import { archivio, chiudiArchivio } from "@/lib/core/archivio";
import { aggiornaCondivisione, azzeraAccessi, creaCondivisione, registraAccesso, revocaCondivisione, ruotaCondivisione, trovaPerToken } from "@/lib/core/condivisione";
import { LetturaNonDisponibile, type Lettura } from "@/lib/core/condivisione-eventi";
import { leggiPerCondivisione } from "@/lib/core/condivisione-eventi";
import { fissaOra } from "@/lib/core/orologio";
import { aggiungiEvento } from "@/lib/core/locali";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto, CORPUS } from "./agente-condivisione";

// GET /condividi/<testo segreto>.ics (lib/condividi.ts): le risposte, il 404 uguale per ogni indirizzo che non vale, i 304, il minuto in memoria, la
// risposta quando non si può leggere, il conto degli indirizzi sbagliati, l'ultimo accesso. Con i link veri nel database e, dove serve, l'agente finto.

let dati: string;
let registro: string[];
let t: number;
let chiamate: number;
let lettura: Lettura;
let errore: Error | null;
let tokens: string[];

const T0 = Date.UTC(2026, 9, 12, 8, 0, 0);
const BASE: Lettura = {
  livello: "occupato",
  da: "2026-10-05",
  a: "2027-01-11",
  impegni: [["2026-10-13T09:00:00", "2026-10-13T11:00:00"]],
  eventi: [{ inizio: "2026-10-13T09:00:00", fine: "2026-10-13T11:00:00", tuttoGiorno: false, titolo: "Riunione di prova", luogo: "Sala 3" }],
};

const deps = (extra: Partial<Deps> = {}): Deps => ({
  adesso: () => t,
  verifica: trovaPerToken,
  accesso: registraAccesso,
  leggi: async (link) => {
    chiamate++;
    if (errore) throw errore;
    return { ...lettura, livello: link.livello };
  },
  ...extra,
});

const tokenDi = (url: string | null) => /\/condividi\/([^/]+)\.ics$/.exec(url ?? "")?.[1] ?? "";
function nuovo(extra: Record<string, unknown> = {}) {
  const c = creaCondivisione({ nome: "Samuele", ...extra });
  if (!c.ok) throw new Error(c.error);
  const token = tokenDi(c.condivisione!.url);
  tokens.push(token);
  return { id: c.condivisione!.id, token };
}
const percorso = (token: string) => `/condividi/${token}.ics`;
const richiesta = (p: string, init: RequestInit = {}) => new Request("https://oe.example.test" + p, init);
const prendi = (token: string, headers: Record<string, string> = {}, metodo = "GET", d: Deps = deps()) => condividi(richiesta(percorso(token), { method: metodo, headers }), d);
const intestazioni = (r: Response) => Object.fromEntries([...r.headers.entries()].sort(([a], [b]) => (a < b ? -1 : 1)));
const righe = (testo: string) => testo.replace(/\r\n /g, "").split("\r\n").filter(Boolean);

beforeEach(() => {
  dati = cartellaDati();
  process.env.OE_DATI = dati;
  process.env.APP_URL = "https://oe.example.test";
  delete process.env.GATEWAY_DB;
  registro = [];
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation((...a: unknown[]) => void registro.push(a.map(String).join(" ")));
  t = T0;
  chiamate = 0;
  lettura = structuredClone(BASE);
  errore = null;
  tokens = [];
  fissaOra(T0);
  azzera();
  azzeraAccessi();
});

afterEach(() => {
  // il testo segreto non è mai in un log
  for (const x of tokens) expect(registro.join("\n"), "log").not.toContain(x);
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  delete process.env.OE_DATI;
  delete process.env.APP_URL;
  vi.restoreAllMocks();
});

describe("un link che vale", () => {
  it("200 con il calendario: tipo, copia in cache, ETag, Last-Modified, niente indicizzazione, nome neutro, nessun cookie", async () => {
    const { token } = nuovo();
    const r = await prendi(token);
    expect(r.status).toBe(200);
    const corpo = await r.text();
    expect(r.headers.get("content-type")).toBe("text/calendar; charset=utf-8");
    expect(r.headers.get("cache-control")).toBe(`private, max-age=${MAX_AGE_S}`);
    expect(MAX_AGE_S).toBe(300);
    expect(r.headers.get("etag")).toMatch(/^"[0-9a-f]{32}"$/);
    expect(r.headers.get("last-modified")).toBe(new Date(T0).toUTCString());
    expect(r.headers.get("x-robots-tag")).toBe("noindex, nofollow");
    expect(r.headers.get("x-content-type-options")).toBe("nosniff");
    expect(r.headers.get("content-disposition")).toBe('inline; filename="calendario.ics"');
    expect(r.headers.get("content-length")).toBe(String(Buffer.byteLength(corpo, "utf8")));
    expect(r.headers.get("set-cookie")).toBeNull();
    expect(corpo.startsWith("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n")).toBe(true);
    const x = righe(corpo);
    for (const riga of ["PRODID:-//OE Copilot//Condivisione//IT", "METHOD:PUBLISH", "X-PUBLISHED-TTL:PT30M", "REFRESH-INTERVAL;VALUE=DURATION:PT30M", "X-OE-COPILOT-SHARE:1", "X-WR-CALNAME:Calendario condiviso"]) {
      expect(x, riga).toContain(riga);
    }
    expect(x).toContain("SUMMARY:Occupato");
    expect(x).toContain("DTSTART:20261013T070000Z");
  });

  it("il nome pubblicato è quello del link", async () => {
    const { token } = nuovo({ nome_calendario: "Antonio, lavoro" });
    expect(righe(await (await prendi(token)).text())).toContain("X-WR-CALNAME:Antonio\\, lavoro");
  });

  it("HEAD: le stesse intestazioni (anche la lunghezza), nessun corpo", async () => {
    const { token } = nuovo();
    const g = await prendi(token);
    const lunghezza = String(Buffer.byteLength(await g.text(), "utf8"));
    const h = await prendi(token, {}, "HEAD");
    expect(h.status).toBe(200);
    expect(await h.text()).toBe("");
    expect(h.headers.get("content-length")).toBe(lunghezza);
    expect(intestazioni(h)).toEqual(intestazioni(g));
  });

  it("nel livello occupato non ci sono titoli né luoghi, in quello con i titoli sì", async () => {
    const a = nuovo({ livello: "occupato" });
    const b = nuovo({ livello: "titoli" });
    const occupato = await (await prendi(a.token)).text();
    const titoli = await (await prendi(b.token)).text();
    expect(occupato).not.toContain("Riunione di prova");
    expect(occupato).not.toContain("Sala 3");
    expect(titoli).toContain("SUMMARY:Riunione di prova");
    expect(titoli).toContain("LOCATION:Sala 3");
  });

  it("la sessione non c'entra: un cookie qualunque non cambia la risposta, e un link che non vale non lo rende valido", async () => {
    const { token } = nuovo();
    const cookie = { cookie: "better-auth.session_token=abc.def; altro=1" };
    expect((await prendi(token, cookie)).status).toBe(200);
    expect((await prendi(crypto.randomBytes(32).toString("base64url"), cookie)).status).toBe(404);
  });

  it("gli altri metodi: 405 con Allow", async () => {
    const { token } = nuovo();
    for (const metodo of ["POST", "PUT", "DELETE", "PATCH"]) {
      const r = await condividi(richiesta(percorso(token), { method: metodo }), deps());
      expect(r.status, metodo).toBe(405);
      expect(r.headers.get("allow")).toBe("GET, HEAD");
    }
    expect(chiamate).toBe(0);
  });
});

describe("un indirizzo che non vale: sempre lo stesso 404", () => {
  it("sconosciuto, con la forma giusta ma casuale, malformato, vuoto, revocato, ruotato: stesso stato, stesse intestazioni, stesso corpo", async () => {
    const vivo = nuovo({ nome: "Vivo" });
    const revocato = nuovo({ nome: "Revocato" });
    revocaCondivisione(revocato.id);
    const ruotato = nuovo({ nome: "Ruotato" });
    ruotaCondivisione(ruotato.id);
    const casuale = crypto.randomBytes(32).toString("base64url");
    const parti = [
      ["casuale", percorso(casuale)],
      ["revocato", percorso(revocato.token)],
      ["ruotato (il vecchio)", percorso(ruotato.token)],
      ["uno di meno", percorso(vivo.token.slice(0, 42))],
      ["uno di più", percorso(vivo.token + "A")],
      ["maiuscole cambiate", percorso(vivo.token.toUpperCase() === vivo.token ? vivo.token.toLowerCase() : vivo.token.toUpperCase())],
      ["con un carattere che non c'è", percorso(vivo.token.replace(/.$/, "!"))],
      ["vuoto", "/condividi/.ics"],
      ["senza estensione", `/condividi/${vivo.token}`],
      ["estensione sbagliata", `/condividi/${vivo.token}.ical`],
      ["doppia estensione", `/condividi/${vivo.token}.ics.ics`],
      ["in una cartella", `/condividi/x/${vivo.token}.ics`],
      ["con la barra in fondo", `/condividi/${vivo.token}.ics/`],
      ["con un carattere codificato", `/condividi/${vivo.token.slice(0, 20)}%2e${vivo.token.slice(21)}.ics`],
      ["lunghissimo", percorso("a".repeat(5000))],
    ];
    const viste: { nome: string; stato: number; intestazioni: Record<string, string>; corpo: string }[] = [];
    for (const [nome, p] of parti) {
      const r = await condividi(richiesta(p), deps());
      viste.push({ nome, stato: r.status, intestazioni: intestazioni(r), corpo: await r.text() });
    }
    for (const v of viste) {
      expect(v.stato, v.nome).toBe(404);
      expect(v.corpo, v.nome).toBe('{"error":"non trovato"}');
      expect(v.intestazioni, v.nome).toEqual(viste[0].intestazioni);
    }
    expect(viste[0].intestazioni).toEqual({ "cache-control": "no-store", "content-type": "application/json", "x-content-type-options": "nosniff", "x-robots-tag": "noindex, nofollow" });
    expect(chiamate).toBe(0);
  });

  it("la risposta non ripete l'indirizzo, e HEAD è lo stesso senza corpo", async () => {
    const sbagliato = crypto.randomBytes(32).toString("base64url");
    const r = await prendi(sbagliato);
    expect(JSON.stringify(intestazioni(r)) + (await r.text())).not.toContain(sbagliato);
    const h = await prendi(sbagliato, {}, "HEAD");
    expect(h.status).toBe(404);
    expect(await h.text()).toBe("");
  });

  it("non legge niente e non segna nessun accesso", async () => {
    const { id } = nuovo();
    await prendi(crypto.randomBytes(32).toString("base64url"));
    expect(chiamate).toBe(0);
    expect(archivio().prepare("SELECT ultimo_accesso, accessi FROM condivisioni WHERE id = ?").get(id)).toEqual({ ultimo_accesso: null, accessi: 0 });
  });

  it("un link revocato smette di valere subito, anche se il suo calendario è ancora in memoria", async () => {
    const { id, token } = nuovo();
    expect((await prendi(token)).status).toBe(200);
    t += 1000;
    revocaCondivisione(id);
    expect((await prendi(token)).status).toBe(404);
  });

  it("ruotare: l'indirizzo vecchio dà 404 subito, il nuovo 200 (con il calendario che c'era: non si rilegge)", async () => {
    const { id, token } = nuovo();
    expect((await prendi(token)).status).toBe(200);
    const r = ruotaCondivisione(id);
    if (!r.ok) throw new Error(r.error);
    const nuovoToken = tokenDi(r.condivisione!.url);
    tokens.push(nuovoToken);
    expect((await prendi(token)).status).toBe(404);
    expect((await prendi(nuovoToken)).status).toBe(200);
  });
});

describe("304: chi ha già il calendario non lo riscarica", () => {
  it("con If-None-Match uguale all'ETag: 304 senza corpo, con ETag e Cache-Control; anche debole, in un elenco, o *", async () => {
    const { token } = nuovo();
    const prima = await prendi(token);
    const etag = prima.headers.get("etag") as string;
    for (const valore of [etag, `W/${etag}`, `"altro", ${etag}`, `${etag} , "altro"`, "*"]) {
      const r = await prendi(token, { "if-none-match": valore });
      expect(r.status, valore).toBe(304);
      expect(await r.text()).toBe("");
      expect(r.headers.get("etag")).toBe(etag);
      expect(r.headers.get("cache-control")).toBe(`private, max-age=${MAX_AGE_S}`);
      expect(r.headers.get("content-type")).toBeNull();
    }
    // un ETag che differisce per l'ultima cifra (diversa da quella vera, qualunque sia: l'ETag è un'impronta, la sua ultima cifra cambia a ogni prova)
    const quasi = etag.slice(0, -2) + (etag.at(-2) === "0" ? "1" : "0") + '"';
    for (const valore of ['"altro"', "", quasi, "W/"]) expect((await prendi(token, { "if-none-match": valore })).status, valore).toBe(200);
  });

  it("con If-Modified-Since: 304 se il calendario non è cambiato dopo quella data, 200 se sì o se la data non vale", async () => {
    const { token } = nuovo();
    const data = (await prendi(token)).headers.get("last-modified") as string;
    expect((await prendi(token, { "if-modified-since": data })).status).toBe(304);
    expect((await prendi(token, { "if-modified-since": new Date(T0 + 3_600_000).toUTCString() })).status).toBe(304);
    expect((await prendi(token, { "if-modified-since": new Date(T0 - 3_600_000).toUTCString() })).status).toBe(200);
    expect((await prendi(token, { "if-modified-since": "ieri sera" })).status).toBe(200);
  });

  it("If-None-Match vince su If-Modified-Since: se non corrisponde, 200 anche con la data giusta", async () => {
    const { token } = nuovo();
    const r = await prendi(token);
    const data = r.headers.get("last-modified") as string;
    expect((await prendi(token, { "if-none-match": '"altro"', "if-modified-since": data })).status).toBe(200);
    expect((await prendi(token, { "if-none-match": r.headers.get("etag") as string, "if-modified-since": new Date(T0 - 3_600_000).toUTCString() })).status).toBe(304);
  });

  it("HEAD con If-None-Match: 304 pure lui", async () => {
    const { token } = nuovo();
    const etag = (await prendi(token)).headers.get("etag") as string;
    expect((await prendi(token, { "if-none-match": etag }, "HEAD")).status).toBe(304);
  });
});

describe("un minuto in memoria per link", () => {
  it("due richieste nel minuto leggono una volta sola; dopo il minuto si rilegge", async () => {
    const { token } = nuovo();
    await prendi(token);
    t += TTL_CORPO_MS - 1;
    await prendi(token);
    expect(chiamate).toBe(1);
    t += 1;
    await prendi(token);
    expect(chiamate).toBe(2);
  });

  it("richieste insieme fanno una lettura sola", async () => {
    const { token } = nuovo();
    const r = await Promise.all(Array.from({ length: 8 }, () => prendi(token)));
    expect(r.every((x) => x.status === 200)).toBe(true);
    expect(chiamate).toBe(1);
  });

  it("ogni link ha la sua", async () => {
    const a = nuovo({ nome: "A" });
    const b = nuovo({ nome: "B" });
    await prendi(a.token);
    await prendi(b.token);
    await prendi(a.token);
    expect(chiamate).toBe(2);
  });

  it("se il contenuto non cambia, ETag, Last-Modified e DTSTAMP restano gli stessi anche rileggendo (chi chiede riceve 304)", async () => {
    const { token } = nuovo();
    const a = await prendi(token);
    const corpoA = await a.text();
    t += 10 * 60_000;
    const b = await prendi(token);
    expect(chiamate).toBe(2);
    expect(b.headers.get("etag")).toBe(a.headers.get("etag"));
    expect(b.headers.get("last-modified")).toBe(a.headers.get("last-modified"));
    expect(await b.text()).toBe(corpoA);
    expect((await prendi(token, { "if-none-match": a.headers.get("etag") as string })).status).toBe(304);
  });

  it("se il contenuto cambia, cambiano ETag, Last-Modified e DTSTAMP: chi chiede riceve il nuovo", async () => {
    const { token } = nuovo();
    const a = await prendi(token);
    const corpoA = await a.text();
    t += 10 * 60_000;
    lettura = { ...lettura, impegni: [...lettura.impegni, ["2026-10-14T09:00:00", "2026-10-14T10:00:00"]] };
    const b = await prendi(token, { "if-none-match": a.headers.get("etag") as string });
    expect(b.status).toBe(200);
    expect(b.headers.get("etag")).not.toBe(a.headers.get("etag"));
    expect(b.headers.get("last-modified")).toBe(new Date(t).toUTCString());
    const corpoB = await b.text();
    expect(righe(corpoA).filter((x) => x.startsWith("DTSTAMP:"))[0]).toBe("DTSTAMP:20261012T080000Z");
    expect(righe(corpoB).filter((x) => x.startsWith("DTSTAMP:"))[0]).toBe("DTSTAMP:20261012T081000Z");
    expect(righe(corpoB).filter((x) => x.startsWith("BEGIN:VEVENT"))).toHaveLength(2);
  });

  it("cambiare le impostazioni del link vale subito, senza aspettare il minuto", async () => {
    const { id, token } = nuovo({ nome_calendario: "Prima" });
    expect(righe(await (await prendi(token)).text())).toContain("X-WR-CALNAME:Prima");
    t += 5000;
    fissaOra(t);
    aggiornaCondivisione(id, { nome_calendario: "Dopo", livello: "titoli" });
    const corpo = await (await prendi(token)).text();
    expect(righe(corpo)).toContain("X-WR-CALNAME:Dopo");
    expect(corpo).toContain("SUMMARY:Riunione di prova");
    expect(chiamate).toBe(2);
  });
});

describe("quando il calendario non si può leggere", () => {
  it("503 con Retry-After, senza copia in cache e senza calendario", async () => {
    const { token } = nuovo();
    errore = new LetturaNonDisponibile("giù");
    const r = await prendi(token);
    expect(r.status).toBe(503);
    expect(r.headers.get("retry-after")).toBe("120");
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(r.headers.get("content-type")).toMatch(/^text\/plain/);
    expect(r.headers.get("etag")).toBeNull();
    expect(await r.text()).not.toContain("BEGIN:VCALENDAR");
    expect((await prendi(token, {}, "HEAD")).status).toBe(503);
  });

  it("il guasto si ricorda quindici secondi (l'agente non si martella), poi si riprova, e al ritorno il calendario è lo stesso di prima", async () => {
    const { token } = nuovo();
    const buona = await prendi(token);
    const etag = buona.headers.get("etag");
    const modificato = buona.headers.get("last-modified");
    t += TTL_CORPO_MS;
    errore = new LetturaNonDisponibile("giù");
    expect((await prendi(token)).status).toBe(503);
    expect(chiamate).toBe(2);
    t += TTL_GUASTO_MS - 1;
    expect((await prendi(token)).status).toBe(503);
    expect(chiamate).toBe(2);
    t += 1;
    errore = null;
    const di_nuovo = await prendi(token);
    expect(chiamate).toBe(3);
    expect(di_nuovo.status).toBe(200);
    expect(di_nuovo.headers.get("etag")).toBe(etag);
    expect(di_nuovo.headers.get("last-modified")).toBe(modificato);
  });

  it("un errore che non è previsto è 503 anche lui, e va nel log (senza il testo segreto)", async () => {
    const { token } = nuovo();
    errore = new Error("guasto qualunque");
    expect((await prendi(token)).status).toBe(503);
    expect(registro.some((r) => r.includes("lettura non riuscita"))).toBe(true);
  });

  it("se il database dei link non risponde: 503, non un errore del server", async () => {
    const { token } = nuovo();
    const r = await prendi(token, {}, "GET", deps({ verifica: () => { throw new Error("database bloccato"); } }));
    expect(r.status).toBe(503);
  });
});

describe("gli indirizzi sbagliati si contano per chi li prova", () => {
  const da = (indirizzo: string, extra: Record<string, string> = {}) => ({ "x-forwarded-for": indirizzo, ...extra });
  const sbagliato = () => crypto.randomBytes(32).toString("base64url");

  it("chiaveClient: conta l'ultimo indirizzo di X-Forwarded-For, quello che ha scritto Caddy; gli altri li scrive chi chiede", () => {
    const k = (v?: string) => chiaveClient(new Headers(v === undefined ? {} : { "x-forwarded-for": v }));
    expect(k("203.0.113.9")).toBe("203.0.113.9");
    expect(k("1.2.3.4, 203.0.113.9")).toBe("203.0.113.9");
    expect(k("evil, 8.8.8.8 ,  203.0.113.9 ")).toBe("203.0.113.9");
    expect(k("2001:db8::1")).toBe("2001:db8::1");
    expect(k("203.0.113.9, garbage")).toBe("sconosciuto");
    expect(k("203.0.113.9,")).toBe("sconosciuto");
    expect(k("")).toBe("sconosciuto");
    expect(k("999.1.1.1")).toBe("sconosciuto");
    expect(k()).toBe("sconosciuto");
  });

  it(`dopo ${FALLIMENTI_MAX} errori in un minuto si riceve 429 al posto di 404, con Retry-After`, async () => {
    for (let i = 0; i < FALLIMENTI_MAX; i++) expect((await prendi(sbagliato(), da("203.0.113.9"))).status, `tentativo ${i + 1}`).toBe(404);
    const r = await prendi(sbagliato(), da("203.0.113.9"));
    expect(r.status).toBe(429);
    expect(Number(r.headers.get("retry-after"))).toBeGreaterThanOrEqual(1);
    expect(Number(r.headers.get("retry-after"))).toBeLessThanOrEqual(60);
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(await r.text()).not.toContain("BEGIN:VCALENDAR");
    expect((await prendi(sbagliato(), da("203.0.113.9"), "HEAD")).status).toBe(429);
  });

  it("è per indirizzo: un altro non ne è toccato, e senza intestazione c'è un solo conto per tutti", async () => {
    for (let i = 0; i <= FALLIMENTI_MAX; i++) await prendi(sbagliato(), da("203.0.113.9"));
    expect((await prendi(sbagliato(), da("203.0.113.10"))).status).toBe(404);
    for (let i = 0; i <= FALLIMENTI_MAX; i++) await prendi(sbagliato());
    expect((await prendi(sbagliato())).status).toBe(429);
    expect((await prendi(sbagliato(), da("garbage"))).status).toBe(429);
    expect((await prendi(sbagliato(), da("203.0.113.11"))).status).toBe(404);
  });

  it("scrivere un altro indirizzo all'inizio di X-Forwarded-For non serve a ricominciare il conto", async () => {
    for (let i = 0; i < FALLIMENTI_MAX; i++) await prendi(sbagliato(), da(`10.0.0.${i}, 203.0.113.9`));
    expect((await prendi(sbagliato(), da("192.0.2.1, 203.0.113.9"))).status).toBe(429);
    expect((await prendi(sbagliato(), da("203.0.113.9"))).status).toBe(429);
  });

  it("un indirizzo di rete che ha troppi errori legge comunque il suo calendario con un link che vale: il conto non può fermare chi ha la chiave", async () => {
    const { token } = nuovo();
    for (let i = 0; i <= FALLIMENTI_MAX + 5; i++) await prendi(sbagliato(), da("203.0.113.9"));
    expect((await prendi(sbagliato(), da("203.0.113.9"))).status).toBe(429);
    expect((await prendi(token, da("203.0.113.9"))).status).toBe(200);
  });

  it("le richieste che vanno bene non contano: cento letture e poi un errore è ancora 404", async () => {
    const { token } = nuovo();
    for (let i = 0; i < 100; i++) expect((await prendi(token, da("203.0.113.9"))).status).toBe(200);
    expect((await prendi(sbagliato(), da("203.0.113.9"))).status).toBe(404);
  });

  it("contano anche i percorsi sbagliati e i testi malformati, non solo quelli con la forma giusta", async () => {
    for (let i = 0; i < FALLIMENTI_MAX; i++) await condividi(richiesta(i % 2 ? "/condividi/.ics" : percorso("x".repeat(i + 1)), { headers: da("203.0.113.9") }), deps());
    expect((await prendi(sbagliato(), da("203.0.113.9"))).status).toBe(429);
  });

  it("passato il minuto si ricomincia, e il limite si scrive nel log una volta sola, con l'indirizzo e senza testi segreti", async () => {
    for (let i = 0; i < FALLIMENTI_MAX + 10; i++) await prendi(sbagliato(), da("203.0.113.9"));
    expect(registro.filter((r) => r.includes("troppi indirizzi sbagliati"))).toHaveLength(1);
    expect(registro.find((r) => r.includes("troppi indirizzi sbagliati"))).toContain("203.0.113.9");
    t += FINESTRA_FALLIMENTI_MS;
    expect((await prendi(sbagliato(), da("203.0.113.9"))).status).toBe(404);
  });

  it("un diluvio di indirizzi diversi non fa crescere la memoria senza limite e non rompe niente", async () => {
    for (let i = 0; i < 1500; i++) await prendi(sbagliato(), da(`10.${i >> 8}.${i & 255}.1`));
    expect((await prendi(sbagliato(), da("10.0.0.1"))).status).toBe(404);
  });
});

describe("l'ultimo accesso", () => {
  it("una lettura, un HEAD e un 304 lo segnano; un 404 no", async () => {
    const { id, token } = nuovo();
    const riga = () => archivio().prepare("SELECT ultimo_accesso, accessi FROM condivisioni WHERE id = ?").get(id);
    const etag = (await prendi(token)).headers.get("etag") as string;
    expect(riga()).toEqual({ ultimo_accesso: T0, accessi: 1 });
    t += 61_000;
    await prendi(token, {}, "HEAD");
    expect(riga()).toEqual({ ultimo_accesso: T0 + 61_000, accessi: 2 });
    t += 61_000;
    expect((await prendi(token, { "if-none-match": etag })).status).toBe(304);
    expect(riga()).toEqual({ ultimo_accesso: T0 + 122_000, accessi: 3 });
    await prendi(crypto.randomBytes(32).toString("base64url"));
    expect(riga()).toEqual({ ultimo_accesso: T0 + 122_000, accessi: 3 });
  });

  it("anche se il calendario non si legge, l'accesso c'è stato", async () => {
    const { id, token } = nuovo();
    errore = new LetturaNonDisponibile("giù");
    expect((await prendi(token)).status).toBe(503);
    expect(archivio().prepare("SELECT accessi FROM condivisioni WHERE id = ?").pluck().get(id)).toBe(1);
  });
});

describe("attraverso la lettura vera, con un agente finto", () => {
  let agente: AgenteFinto;
  beforeEach(async () => {
    agente = await avviaAgenteFinto([
      { account: "mario@alfa.example", title: CORPUS.titoli[0], start: "2026-10-13T09:00:00", end: "2026-10-13T10:00:00", location: CORPUS.luoghi[0], organizer: CORPUS.organizzatori[0], organizer_email: CORPUS.posta[0], online_url: CORPUS.conferenze[0] },
    ]);
    process.env.AGENTE_URL = agente.url;
    process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
    azzeraMemoria();
    aggiungiEvento({ title: "Dentista", start: "2026-10-14T15:00", end: "2026-10-14T16:00", allDay: false, location: "Via Garibaldi 3", notes: CORPUS.note[0] });
  });
  afterEach(async () => {
    await agente.chiudi();
    delete process.env.AGENTE_URL;
    delete process.env.AGENTE_CHIAVE;
  });
  const vero = (): Deps => deps({ leggi: leggiPerCondivisione });

  it("occupato: due blocchi, nessun dettaglio nel testo", async () => {
    const { token } = nuovo();
    const corpo = await (await prendi(token, {}, "GET", vero())).text();
    expect(righe(corpo).filter((r) => r.startsWith("DTSTART:"))).toEqual(["DTSTART:20261013T070000Z", "DTSTART:20261014T130000Z"]);
    for (const privata of [...CORPUS.titoli, ...CORPUS.luoghi, ...CORPUS.organizzatori, ...CORPUS.posta, ...CORPUS.conferenze, ...CORPUS.aziende, ...CORPUS.note, "Dentista", "Garibaldi"]) {
      expect(corpo.replace(/\r\n /g, ""), privata).not.toContain(privata);
    }
  });

  it("con i titoli: titolo e luogo, e niente di chi organizza, della posta, dei link", async () => {
    const { token } = nuovo({ livello: "titoli" });
    const corpo = (await (await prendi(token, {}, "GET", vero())).text()).replace(/\r\n /g, "");
    expect(corpo).toContain("SUMMARY:" + CORPUS.titoli[0]);
    expect(corpo).toContain("SUMMARY:Dentista");
    for (const privata of [...CORPUS.organizzatori, ...CORPUS.posta, ...CORPUS.conferenze, ...CORPUS.note]) expect(corpo, privata).not.toContain(privata);
  });

  it("con l'agente che non risponde: 503, mai un calendario vuoto", async () => {
    const { token } = nuovo();
    agente.giu = true;
    const r = await prendi(token, {}, "GET", vero());
    expect(r.status).toBe(503);
    expect(await r.text()).not.toContain("BEGIN:VCALENDAR");
  });
});
