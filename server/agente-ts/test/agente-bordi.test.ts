import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import type http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { caricaFonti, contestoDi, creaServer } from "../src/index.ts";
import { leggiGolden, leggiTestoGolden } from "./ausiliari/dati.ts";
import { type CasoRichiesta, type RispostaGrezza, richiestaCompleta } from "./ausiliari/http.ts";
import { type Db, scriviDb } from "./ausiliari/memdb.ts";

// L'agente Python vero (agent.py in HTTP, sulla VM, nell'immagine dell'agente, come utente senza privilegi) su 14 scenari del disco e 152
// richieste che nessun cliente normale manda: chiave mancante, sbagliata, di altra lunghezza o con caratteri non ASCII, percorsi strani,
// finestre in ogni forma sbagliata, corpi enormi o in altre codifiche, metodi che l'agente non ha, calendar.db che manca, è vuoto, è
// rotto, non si apre; più sette modi di non partire. Per ogni richiesta: stato, frase dello stato, tipo, lunghezza e corpo uguali. Un corpo
// lungo si confronta con la sua impronta sha256.
const bordi = leggiGolden("agente-bordi.json");
const golden = leggiGolden("agente-righe.json");
const ICS = { festivita: leggiTestoGolden("ics", "festivita.ics"), riunioni: leggiTestoGolden("ics", "riunioni.ics") };
const PACCHETTO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// su Windows chmod non toglie i permessi e copiare una cartella dà un altro errore: quei due scenari si provano su Linux (CI e VM)
const SU_WINDOWS = process.platform === "win32";
const SOLO_LINUX = new Set(["senza-permessi", "db-cartella"]);

/** Differenze volute, ognuna con il perché. Il confronto con Python non vale: vale quello che l'agente TypeScript fa al suo posto. */
const VOLUTE: Record<string, { perche: string; atteso: (r: RispostaGrezza) => void }> = {
  "ics-testo-percorso-di-file": {
    perche: "Python tratta un testo senza a capo come il percorso di un file e se esiste lo legge (icalendar Component.from_ical): chi manda il testo legge i file del container. L'agente TypeScript non apre mai file per un testo",
    atteso: (r) => {
      expect(r.stato).toBe(200);
      expect(JSON.parse(r.corpo!)).toEqual({ ok: false, errore: "Content line could not be parsed into parts: '/etc/os-release': /etc/os-release", valore: true });
    },
  },
  "salute-chiave-spazio-finale": {
    perche: "Node toglie gli spazi in coda al valore di un'intestazione prima che il gestore la veda, Python no: con uno spazio dopo la chiave giusta l'agente TypeScript risponde 200, Python 401",
    atteso: (r) => expect(r.stato).toBe(200),
  },
  ...Object.fromEntries(
    [
      "salute-percorso-non-ascii",
      "salute-percorso-non-ascii-un-byte",
      "eventi-percorso-non-ascii",
      "percorso-non-ascii-senza-chiave",
      "ics-percorso-non-ascii",
    ].map((nome) => [
      nome,
      {
        perche: "Node rifiuta con 400 i byte non ASCII grezzi (senza %) nel percorso della richiesta, Python li accetta (legge la riga come iso-8859-1): il gateway codifica sempre il percorso",
        atteso: (r: RispostaGrezza) => expect(r.stato).toBe(400),
      },
    ]),
  ),
};

const normalizza = (t: string, db: string) => {
  const s = t.replaceAll(db, "<DB>").replace(/<DB>\\\\calendar\.db/g, "<DB>/calendar.db").replace(/<DB>\\calendar\.db/g, "<DB>/calendar.db");
  return s.replace(/"generato": "[^"]*"/, '"generato": "ORA"').replace(/alle \d\d:\d\d/g, "alle HH:MM");
};

interface Pronto {
  server: http.Server | null;
  porta: number;
  db: string;
}

function prepara(nome: string, sc: any, base: string): { db: string; fonti: string; battito: string; accessi: string } {
  const d = path.join(base, nome);
  const db = path.join(d, "db");
  fs.mkdirSync(db, { recursive: true });
  const file = path.join(db, "calendar.db");
  const t = sc.db;
  if (t === "golden" || t === "senza-eventi" || t === "senza-colonna") {
    scriviDb(db, { calendari: golden.calendari, eventi: golden.eventi, invitati: golden.invitati } as Db);
    const sql = new DatabaseSync(file);
    if (t === "senza-eventi") sql.exec("DROP TABLE Events");
    if (t === "senza-colonna") sql.exec("ALTER TABLE Calendars DROP COLUMN sync_events");
    sql.close();
  } else if (t === "vuoto") fs.writeFileSync(file, "");
  else if (t === "corrotto") fs.writeFileSync(file, "questo non e' un database sqlite, e' solo testo ".repeat(100));
  else if (t === "cartella") fs.mkdirSync(file);
  if (sc.permessi) fs.chmodSync(file, parseInt(sc.permessi, 8));
  const fonti = path.join(d, "fonti.json");
  const f = sc.fonti ?? "golden";
  const elenco = golden.fonti.map((x: object) => ({ tipo: "android", ...x }));
  if (f === "golden") fs.writeFileSync(fonti, JSON.stringify(elenco));
  else if (f === "con-sconosciuta") fs.writeFileSync(fonti, JSON.stringify([...elenco, { tipo: "caldav", id: "x", nome: "X", account: "x@x" }]));
  else if (f === "json-rotto") fs.writeFileSync(fonti, "{non json");
  else if (f === "stringa") fs.writeFileSync(fonti, '"abc"');
  else if (f === "numero") fs.writeFileSync(fonti, "5");
  else if (f === "senza-id") fs.writeFileSync(fonti, JSON.stringify([{ tipo: "android", nome: "A", account: "a@a" }]));
  const battito = path.join(d, "battito.json");
  if (sc.battito !== undefined) fs.writeFileSync(battito, JSON.stringify(sc.battito));
  else if (sc.battitoTesto !== undefined) fs.writeFileSync(battito, sc.battitoTesto);
  const accessi = path.join(d, "accessi.json");
  if (sc.accessi !== undefined) fs.writeFileSync(accessi, JSON.stringify(sc.accessi));
  else if (sc.accessiTesto !== undefined) fs.writeFileSync(accessi, sc.accessiTesto);
  return { db, fonti, battito, accessi };
}

let base = "";
const pronti = new Map<string, Pronto>();

beforeAll(async () => {
  base = fs.mkdtempSync(path.join(os.tmpdir(), "agente-bordi-"));
  for (const [nome, sc] of Object.entries<any>(bordi.scenari)) {
    if (SOLO_LINUX.has(nome) && SU_WINDOWS) continue;
    const p = prepara(nome, sc, base);
    const ctx = contestoDi({ accessi: p.accessi, zona: "Europe/Rome", registro: () => {} });
    const { server } = creaServer({ chiave: bordi.chiave, fonti: caricaFonti(p.fonti, p.db, ctx), battito: p.battito, zona: "Europe/Rome", registro: () => {} });
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
    pronti.set(nome, { server, porta: (server.address() as net.AddressInfo).port, db: p.db });
  }
}, 60000);

afterAll(async () => {
  for (const p of pronti.values()) await new Promise<void>((ok) => p.server!.close(() => ok()));
  fs.rmSync(base, { recursive: true, force: true });
});

describe("agente TypeScript contro Python, richieste di bordo", () => {
  for (const r of bordi.richieste as (CasoRichiesta & { atteso: any })[]) {
    if (SOLO_LINUX.has(r.scenario) && SU_WINDOWS) continue;
    const voluta = VOLUTE[r.nome];
    it(`${r.nome}${voluta ? " (differenza voluta)" : ""}`, async () => {
      const p = pronti.get(r.scenario)!;
      const ottenuto = await richiestaCompleta(p.porta, r, bordi.chiave, ICS);
      if (voluta) return voluta.atteso(ottenuto);
      const a = r.atteso;
      if (a.chiusa || a.timeout || a.azzerata) return expect({ chiusa: ottenuto.chiusa, timeout: ottenuto.timeout, azzerata: ottenuto.azzerata }).toEqual({ chiusa: a.chiusa, timeout: a.timeout, azzerata: a.azzerata });
      const corpo = normalizza(ottenuto.corpo ?? "", p.db);
      // un messaggio col percorso del calendario è lungo quanto il percorso della cartella di prova, diverso da quello dell'oracolo: la lunghezza vale
      // allora come lunghezza del corpo, e il corpo si confronta più sotto col percorso tolto
      const lunghezza = corpo !== ottenuto.corpo ? String(Buffer.byteLength(ottenuto.corpo ?? "")) : a.lunghezza;
      expect({ stato: ottenuto.stato, motivo: ottenuto.motivo, tipo: ottenuto.tipo, lunghezza: ottenuto.lunghezza }).toEqual({ stato: a.stato, motivo: a.motivo, tipo: a.tipo, lunghezza });
      if (a.corpoSha256) expect({ caratteri: corpo.length, sha256: crypto.createHash("sha256").update(corpo, "utf-8").digest("hex") }).toEqual({ caratteri: a.corpoCaratteri, sha256: a.corpoSha256 });
      else expect(corpo).toBe(a.corpo ?? "");
    }, 60000);
  }
});

describe("agente TypeScript contro Python, avvii che falliscono", () => {
  for (const [nome, av] of Object.entries<any>(bordi.avvii)) {
    it(`${nome}: esce con ${av.atteso.uscita}`, async () => {
      const dir = path.join(base, "avvio-" + nome);
      const p = prepara("avvio-" + nome, { db: "golden", fonti: av.fonti }, base);
      void dir;
      const porta = await new Promise<number>((ok) => {
        const s = net.createServer().listen(0, "127.0.0.1", () => {
          const n = (s.address() as net.AddressInfo).port;
          s.close(() => ok(n));
        });
      });
      const env: NodeJS.ProcessEnv = { ...process.env, FONTI: p.fonti, ANDROID_DB: p.db, BATTITO: p.battito, ACCESSI: p.accessi, PORTA: av.porta ?? String(porta), TZ: "Europe/Rome" };
      if (av.senzaChiave) delete env.API_KEY;
      else env.API_KEY = bordi.chiave;
      const figlio = spawn(process.execPath, [path.join(PACCHETTO, "src", "main.ts")], { env, stdio: "ignore" });
      const uscita = await new Promise<number | null>((ok) => {
        const t = setTimeout(() => ok(null), 8000);
        figlio.on("exit", (c) => {
          clearTimeout(t);
          ok(c);
        });
      });
      if (uscita === null) figlio.kill();
      expect({ uscita }).toEqual({ uscita: av.atteso.uscita });
      expect(av.atteso.ascolta).toBe(false);
    }, 30000);
  }
});
