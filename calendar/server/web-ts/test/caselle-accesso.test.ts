import fs from "node:fs";
import http from "node:http";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { CHIAVE_AGENTE, cartellaDati } from "./agente-condivisione";

// La finestra Account: get_stato_caselle porta le aziende copiate in ogni casella, rimuovi_casella passa la richiesta all'agente.

const BANDPEY = "antonio@bandpey.example";
const SERCO = "antonio@serco.example";
const NASCOSTA = "collega@serco.example";

let dati: string;
let server: http.Server;
let ricevuti: unknown[];

beforeEach(async () => {
  dati = cartellaDati();
  ricevuti = [];
  fs.writeFileSync(
    `${dati}/personalizzazione.json`,
    JSON.stringify({
      account_nascosti: [NASCOSTA],
      aziende_inoltrate: { Ever: { via: "Antonio@Bandpey.example", prefisso: "^EVER:" }, Zurich: { via: BANDPEY, prefisso: "^ZU:" } },
    }),
  );
  server = http.createServer((req, res) => {
    const json = (corpo: unknown) => void res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(corpo));
    if (req.headers.authorization !== "Bearer " + CHIAVE_AGENTE) return void res.writeHead(401).end();
    if (req.method === "GET" && req.url === "/api/auth/stato") {
      return json({
        caselle: [BANDPEY, SERCO, NASCOSTA].map((account) => ({ id: `android::${account}`, nome: "x", account, provider: "microsoft", configurato: true, scaduto: account === SERCO, stato: "ok", msg: "" })),
      });
    }
    if (req.method === "POST" && req.url === "/api/auth/rimuovi") {
      let corpo = "";
      req.on("data", (c) => (corpo += c));
      req.on("end", () => {
        const r = JSON.parse(corpo) as { account: string };
        ricevuti.push(r);
        json(r.account === SERCO ? { ok: true, account: SERCO } : { ok: false, errore: "Casella non trovata" });
      });
      return;
    }
    res.writeHead(404).end();
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  process.env.OE_DATI = dati;
  process.env.AGENTE_URL = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  azzeraMemoria();
});

afterEach(async () => {
  await new Promise<void>((ok) => server.close(() => ok()));
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE"]) delete process.env[n];
  vi.restoreAllMocks();
});

describe("finestra Account", () => {
  it("get_stato_caselle: le aziende copiate stanno con la casella che le riceve, le caselle nascoste non ci sono", async () => {
    const r = (await chiama("get_stato_caselle", [])) as { ok: boolean; caselle: { account: string; scaduto: boolean; inoltrate: string[] }[] };
    expect(r.ok).toBe(true);
    expect(r.caselle.map((c) => [c.account, c.scaduto, c.inoltrate])).toEqual([
      [BANDPEY, false, ["Ever", "Zurich"]],
      [SERCO, true, []],
    ]);
  });

  it("rimuovi_casella: passa l'indirizzo all'agente e ne riporta l'esito", async () => {
    expect(await chiama("rimuovi_casella", [SERCO])).toEqual({ ok: true, account: SERCO });
    expect(await chiama("rimuovi_casella", ["altro@serco.example"])).toEqual({ ok: false, error: "Casella non trovata" });
    expect(ricevuti).toEqual([{ account: SERCO }, { account: "altro@serco.example" }]);
  });
});
