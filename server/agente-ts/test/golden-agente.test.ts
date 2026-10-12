import fs from "node:fs";
import type http from "node:http";
import os from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { contestoDi, caricaFonti, creaServer } from "../src/index.ts";
import { type Db, type Fonte, occorrenzeGrezze } from "./ausiliari/android.ts";
import { canonico } from "./ausiliari/confronto.ts";
import { leggiGolden, leggiTestoGolden } from "./ausiliari/dati.ts";
import { CHIAVE_PROVA, chiediIcs, leggiTutto } from "./ausiliari/http.ts";
import { scriviDb } from "./ausiliari/memdb.ts";

// L'agente Python (tolto dal repository) su un calendar.db inventato: la risposta intera registrata una volta sola in golden/agente.json
// (calendari, caselle, servizio, eventi di cinque finestre, i calendari ICS), contro l'agente TypeScript vero, in HTTP su una porta libera,
// come faceva la vecchia prova in Python (prova_agente_golden.py, nella cronologia di git).
const golden = leggiGolden("agente.json");
const righe = leggiGolden("agente-righe.json");
const db: Db = { calendari: righe.calendari, eventi: righe.eventi, invitati: righe.invitati };
const fonti: Fonte[] = righe.fonti;

let cartella = "";
let server: http.Server;
let base = "";

beforeAll(async () => {
  cartella = fs.mkdtempSync(path.join(os.tmpdir(), "agente-golden-"));
  scriviDb(cartella, db);
  const fontiJson = path.join(cartella, "fonti.json");
  fs.writeFileSync(fontiJson, JSON.stringify(fonti.map((f) => ({ tipo: "android", ...f }))));
  const ctx = contestoDi({ accessi: path.join(cartella, "nessun-accesso.json"), zona: "Europe/Rome" });
  ({ server } = creaServer({ chiave: CHIAVE_PROVA, fonti: caricaFonti(fontiJson, cartella, ctx), battito: path.join(cartella, "nessun-battito.json"), zona: "Europe/Rome", registro: () => {} }));
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
});

afterAll(async () => {
  await new Promise<void>((ok) => server.close(() => ok()));
  fs.rmSync(cartella, { recursive: true, force: true });
});

describe("agente TypeScript in HTTP, calendar.db inventato: le risposte intere dell'agente Python", () => {
  let letto: Awaited<ReturnType<typeof leggiTutto>>;
  beforeAll(async () => {
    letto = await leggiTutto(base, Object.keys(golden.eventi));
  });

  it("GET /api/calendari: le quattro caselle, lo stato di ognuna, il servizio", () => {
    expect(letto.calendari).toEqual(golden.calendari);
  });
  for (const finestra of Object.keys(golden.eventi)) {
    it(`GET /api/eventi, finestra ${finestra}: eventi, errori, fonti, caselle, servizio`, () => {
      expect(letto.eventi[finestra]).toEqual(golden.eventi[finestra]);
    });
  }
});

describe("agente TypeScript in HTTP: POST /api/ics, le risposte registrate", () => {
  const testi: Record<string, string> = {
    festivita: leggiTestoGolden("ics", "festivita.ics"),
    riunioni: leggiTestoGolden("ics", "riunioni.ics"),
    html: leggiTestoGolden("ics", "html.ics"),
    rotto: leggiTestoGolden("ics", "rotto.ics"),
    vuoto: "",
  };
  for (const [nome, registrato] of Object.entries<any>(golden.ics)) {
    if (nome === "rifiuti") continue;
    it(`${nome}: validazione`, async () => {
      const r = await chiediIcs(base, { testo: testi[nome] });
      expect(r.stato).toBe(200);
      expect(r.dati).toEqual(registrato.valida);
    });
    for (const finestra of Object.keys(registrato.eventi)) {
      it(`${nome}: eventi della finestra ${finestra}`, async () => {
        const [inizio, fine] = finestra.split("|");
        const r = await chiediIcs(base, { testo: testi[nome], inizio, fine });
        expect(r.stato).toBe(200);
        // nello stesso ordine di Python (le serie per UID nell'ordine del testo, le occorrenze nell'ordine delle regole)
        expect(r.dati).toEqual(registrato.eventi[finestra]);
      });
    }
  }
  it("richieste sbagliate: senza chiave 401, un solo estremo, testo non stringa e data non valida 400, percorso 404", async () => {
    const primaFinestra = Object.keys(golden.eventi)[0]!.split("|")[0];
    const rifiuti = {
      "senza chiave": (await chiediIcs(base, {}, null)).stato,
      "un solo estremo": (await chiediIcs(base, { testo: testi.festivita, inizio: primaFinestra })).stato,
      "testo non stringa": (await chiediIcs(base, { testo: 5 })).stato,
      "estremo non valido": (await chiediIcs(base, { testo: testi.festivita, inizio: "ieri", fine: "oggi" })).stato,
      "percorso sconosciuto": (await chiediIcs(base, {}, CHIAVE_PROVA, "/api/altro")).stato,
    };
    expect(rifiuti).toEqual(golden.ics.rifiuti);
  });
});

describe("agente Python, calendar.db inventato: le occorrenze grezze, prima dei doppioni", () => {
  /** Un estremo della finestra come lo fa l'agente (agent._locale): senza scarto com'è, con lo scarto si porta all'ora locale di Roma. */
  const locale = (testo: string) => (/([+-]\d{2}:\d{2}|Z)$/.test(testo) ? Temporal.Instant.from(testo).toZonedDateTimeISO("Europe/Rome").toPlainDateTime().toString({ smallestUnit: "second" }) : testo);
  for (const finestra of Object.keys(golden.eventi)) {
    it(`finestra ${finestra}`, () => {
      const [inizio, fine] = finestra.split("|").map(locale) as [string, string];
      for (const f of fonti) {
        const registrate = righe.grezzi[f.id][finestra] as any[];
        const ottenute = occorrenzeGrezze(db, f, inizio, fine) ?? [];
        const senzaDescrizione = ottenute.map(({ descrizione: _d, ...resto }) => resto);
        const ordina = (v: any[]) => v.map(canonico).sort();
        expect(ordina(senzaDescrizione)).toEqual(ordina(registrate));
      }
    });
  }
});
