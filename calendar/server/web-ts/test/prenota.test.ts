import fs from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { POST } from "@/app/api/prenota/route";
import { azzeraMemoria } from "@/lib/core/agente";
import { chiama } from "@/lib/core/api";
import { chiudiArchivio, leggiRigheLocali } from "@/lib/core/archivio";
import { occupato } from "@/lib/core/eventi";
import { azzeraFeed } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { canaleFinto, type CanaleFinto } from "@/lib/core/promemoria-canali";
import { loadConfig } from "@/lib/core/stato";
import { azzera, fuoriRegola, PRENOTAZIONI_ORA, prenota } from "@/lib/prenota";
import { leggiNaive } from "@/lib/core/fuso";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto, type EventoFinto } from "./agente-condivisione";

// Prenotazione dalla pagina pubblica: l'evento entra subito, per il gruppo della pagina, solo in uno spazio libero e dentro le regole
// (lun-ven, 9-18 di Roma, quarti d'ora, due ore di preavviso); arriva una notifica.

const ALFA = "antonio@alfa.example";
const GAMMA = "samuele@gamma.example";
// venerdì 16 ottobre 2026: Antonio occupato 11-12, Samuele 14-15
const EVENTI: EventoFinto[] = [
  { account: ALFA, calendar: `${ALFA} · Calendar`, title: "Riunione Alfa", organizer_email: ALFA, start: "2026-10-16T11:00:00", end: "2026-10-16T12:00:00" },
  { account: GAMMA, calendar: `${GAMMA} · Calendar`, title: "Riunione Gamma", organizer_email: GAMMA, start: "2026-10-16T14:00:00", end: "2026-10-16T15:00:00" },
];
// le 8:00 di Roma di venerdì 16: si prenota dalle 10:00
const ORA = Date.UTC(2026, 9, 16, 6, 0, 0);

const richiesta = (inizio: string, fine: string, extra: Record<string, unknown> = {}) => ({
  inizio: `2026-10-16T${inizio}:00`,
  fine: `2026-10-16T${fine}:00`,
  nome: "Mario Rossi",
  email: "mario@cliente.example",
  azienda: "Cliente Spa",
  argomento: "Migrazione\nsu Azure",
  ...extra,
});

let dati: string;
let agente: AgenteFinto | undefined;
let push: CanaleFinto;

beforeEach(async () => {
  dati = cartellaDati({ domini_aziendali: ["alfa.example", "gamma.example"], nomi_per_dominio: { "alfa.example": "Alfa", "gamma.example": "Gamma" } });
  process.env.OE_DATI = dati;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  fissaOra(ORA);
  azzeraFeed();
  azzera();
  agente = await avviaAgenteFinto(EVENTI);
  process.env.AGENTE_URL = agente.url;
  azzeraMemoria();
  await chiama("get_calendars", []);
  await chiama("save_config", [{ gruppi_calendari: { Antonio: ["Alfa"], Samuele: ["Gamma"] } }]);
  push = canaleFinto("push");
});

afterEach(async () => {
  await agente?.chiudi();
  agente = undefined;
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE"]) delete process.env[n];
  vi.restoreAllMocks();
});

const libero = async (gruppo: string) => (await occupato(loadConfig(), { a: 2026, m: 10, g: 16 }, 1, gruppo))[0];

describe("prenotazione di un meeting", () => {
  it("entra subito nel calendario del gruppo, blocca il tempo e manda la notifica", async () => {
    const e = await prenota({ ...richiesta("10:00", "10:45"), gruppo: "Samuele" }, { canali: [push] });
    expect(e).toEqual({ ok: true, inizio: "2026-10-16T10:00:00", fine: "2026-10-16T10:45:00" });
    const [riga] = leggiRigheLocali();
    expect(riga).toMatchObject({ title: "Meeting con Mario Rossi (Cliente Spa)", start: "2026-10-16T10:00:00", end: "2026-10-16T10:45:00", allDay: false, blocca: true, gruppi: ["Samuele"] });
    expect(riga.notes).toContain("Email: mario@cliente.example");
    expect(riga.notes).toContain("Argomento: Migrazione\nsu Azure");
    expect(await libero("Samuele")).toContainEqual(["2026-10-16T10:00:00", "2026-10-16T10:45:00"]);
    expect(await libero("Antonio")).not.toContainEqual(["2026-10-16T10:00:00", "2026-10-16T10:45:00"]);
    await vi.waitFor(() => expect(push.inviati).toHaveLength(1));
    expect(push.inviati[0].split("\n")).toEqual(["Meeting prenotato: ven 16 ott 10:00-10:45", "Mario Rossi (Cliente Spa) · mario@cliente.example", "Migrazione", "su Azure"]);
  });

  it("dalla pagina di tutti è per ogni gruppo", async () => {
    expect((await prenota(richiesta("10:00", "10:15"), { canali: [push] })).ok).toBe(true);
    expect(leggiRigheLocali()[0].gruppi).toEqual(["Antonio", "Samuele"]);
  });

  it("solo uno spazio libero per i calendari del gruppo, e non due volte lo stesso", async () => {
    expect(await prenota({ ...richiesta("11:30", "12:00"), gruppo: "Antonio" }, { canali: [push] })).toMatchObject({ ok: false, stato: 409 });
    expect(await prenota({ ...richiesta("11:30", "12:00"), gruppo: "Samuele" }, { canali: [push] })).toMatchObject({ ok: true });
    expect(await prenota(richiesta("11:00", "11:30"), { canali: [push] })).toMatchObject({ ok: false, stato: 409 });
    // in contemporanea per lo stesso spazio ne passa una sola
    const due = await Promise.all([prenota({ ...richiesta("15:00", "15:30"), gruppo: "Samuele" }, { canali: [push] }), prenota({ ...richiesta("15:15", "15:45"), gruppo: "Samuele" }, { canali: [push] })]);
    expect(due.map((e) => e.ok)).toEqual([true, false]);
    // a ridosso va bene: finire alle 14:00 quando alle 14:00 c'è una riunione
    expect(await prenota({ ...richiesta("13:30", "14:00"), gruppo: "Samuele" }, { canali: [push] })).toMatchObject({ ok: true });
  });

  it.each([
    ["sabato", { inizio: "2026-10-17T10:00:00", fine: "2026-10-17T10:30:00" }, "lunedì al venerdì"],
    ["prima delle 9", { inizio: "2026-10-19T08:45:00", fine: "2026-10-19T09:15:00" }, "dalle 9:00 alle 18:00"],
    ["dopo le 18", { inizio: "2026-10-19T17:45:00", fine: "2026-10-19T18:15:00" }, "dalle 9:00 alle 18:00"],
    ["non a quarti d'ora", { inizio: "2026-10-19T10:10:00", fine: "2026-10-19T10:40:00" }, "quarti d'ora"],
    ["meno di 15 minuti", { inizio: "2026-10-19T10:00:00", fine: "2026-10-19T10:00:00" }, "almeno 15 minuti"],
    ["a cavallo di due giorni", { inizio: "2026-10-19T17:00:00", fine: "2026-10-20T09:30:00" }, "stesso giorno"],
    ["senza preavviso", { inizio: "2026-10-16T09:45:00", fine: "2026-10-16T10:15:00" }, "due ore di preavviso"],
    ["troppo avanti", { inizio: "2026-12-21T10:00:00", fine: "2026-12-21T10:30:00" }, "60 giorni"],
  ])("regole: %s", (_, r, messaggio) => {
    expect(fuoriRegola({ inizio: leggiNaive(r.inizio), fine: leggiNaive(r.fine) }, ORA)).toContain(messaggio);
  });

  it("le 18:00 in punto e le 10:00 con due ore esatte vanno bene", () => {
    expect(fuoriRegola({ inizio: leggiNaive("2026-10-16T10:00:00"), fine: leggiNaive("2026-10-16T18:00:00") }, ORA)).toBeNull();
  });

  it.each([
    ["senza nome", { nome: " " }, 400, "nome"],
    ["email sbagliata", { email: "mario@" }, 400, "email"],
    ["campo nascosto riempito", { sito: "https://spam.example" }, 400, "non valida"],
    ["orario scritto male", { inizio: "domani alle 10" }, 400, "orario"],
    ["con lo scarto", { inizio: "2026-10-16T10:00:00+02:00" }, 400, "orario"],
    ["gruppo che non c'è", { gruppo: "Nessuno" }, 404, "nessun calendario"],
  ])("rifiuta: %s", async (_, extra, stato, messaggio) => {
    const e = await prenota({ ...richiesta("10:00", "10:30"), ...extra }, { canali: [push] });
    expect(e).toMatchObject({ ok: false, stato });
    expect(!e.ok && e.errore.toLowerCase()).toContain(messaggio.toLowerCase());
    expect(leggiRigheLocali()).toEqual([]);
  });

  it(`al più ${PRENOTAZIONI_ORA} in un'ora, poi 429`, async () => {
    for (let i = 0; i < PRENOTAZIONI_ORA; i++) {
      const h = 10 + Math.floor(i / 4);
      const m = (i % 4) * 15;
      const s = `${h}:${String(m).padStart(2, "0")}`;
      const f = m === 45 ? `${h + 1}:00` : `${h}:${String(m + 15).padStart(2, "0")}`;
      expect((await prenota({ ...richiesta(s, f), gruppo: "Samuele" }, { canali: [push] })).ok).toBe(true);
    }
    expect(await prenota({ ...richiesta("16:00", "16:15"), gruppo: "Samuele" }, { canali: [push] })).toMatchObject({ ok: false, stato: 429 });
  });

  it("senza notifiche configurate la prenotazione c'è lo stesso", async () => {
    expect((await prenota({ ...richiesta("10:00", "10:30"), gruppo: "Samuele" }, { canali: [canaleFinto("push", false)] })).ok).toBe(true);
    expect(leggiRigheLocali()).toHaveLength(1);
  });

  it("l'API: JSON obbligatorio, errori con il loro stato", async () => {
    const post = (corpo: string, tipo = "application/json") => POST(new Request("http://x/api/prenota", { method: "POST", headers: { "content-type": tipo }, body: corpo }));
    expect((await post("{}", "text/plain")).status).toBe(415);
    expect((await post("{rotto")).status).toBe(400);
    const r = await post(JSON.stringify({ ...richiesta("10:00", "10:30"), gruppo: "Samuele" }));
    expect(r.status).toBe(200);
    expect(await r.json()).toEqual({ ok: true, inizio: "2026-10-16T10:00:00", fine: "2026-10-16T10:30:00" });
    const doppia = await post(JSON.stringify({ ...richiesta("10:15", "10:45"), gruppo: "Samuele" }));
    expect(doppia.status).toBe(409);
    expect((await doppia.json()).error).toContain("non è più libero");
  });
});
