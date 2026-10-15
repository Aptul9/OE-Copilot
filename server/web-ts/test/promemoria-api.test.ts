import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { fissaOra } from "@/lib/core/orologio";
import { handle, MAX_BODY, type Deps } from "@/lib/gateway";
import { browserFinto, chiaviVapidDiProva, soloRetiLocali } from "./push-finto";

// I tre metodi dei promemoria attraverso il gateway vero (handle, con i metodi veri di lib/core): le regole di sempre (401, 403, 405, 413, 415)
// e il comportamento. Le notifiche del browser sono configurate con chiavi di prova e un browser iscritto, mai un servizio vero.

const MINUTI_NO = "Minuti: da 1 a 8 numeri interi diversi, da 0 a 1440.";
const METODI = ["get_promemoria", "set_promemoria", "prova_promemoria"];
const NOMI_VAPID = ["VAPID_PUBLIC_KEY", "VAPID_PRIVATE_KEY", "VAPID_SUBJECT"];
const PREDEFINITE = { minuti: [10], contenuto: "completo", suono: "Default", notifica_windows: true, comparsa: true };

let dati: string;
let vietate: string[];
let chiamate: string[];

function deps(utente: unknown): Deps {
  return {
    user: async () => utente,
    loghi: [],
    metodi: {
      ha: (nome) => METODI_TS.has(nome),
      chiama: (nome, args) => {
        chiamate.push(nome);
        return chiama(nome as NomeMetodoTs, args);
      },
    },
  };
}
const req = (percorso: string, init: RequestInit = {}) => new Request("http://gateway:3000" + percorso, init);
const post = (percorso: string, corpo: unknown, headers: Record<string, string> = {}) =>
  req(percorso, { method: "POST", body: typeof corpo === "string" ? corpo : JSON.stringify(corpo), headers: { "content-type": "application/json", ...headers } });
const SESSIONE = { id: "u" };
const api = async (nome: string, args: unknown[] = []) => {
  const r = await handle(post(`/api/${nome}`, args), deps(SESSIONE));
  return { stato: r.status, corpo: await r.json() };
};

beforeEach(() => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-promemoria-api-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  for (const nome of NOMI_VAPID) delete process.env[nome];
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  vietate = soloRetiLocali().vietate;
  chiamate = [];
  fissaOra(Date.UTC(2026, 9, 12, 13, 20, 0));
});

afterEach(() => {
  fissaOra(null);
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  for (const nome of ["OE_DATI", ...NOMI_VAPID]) delete process.env[nome];
  vi.restoreAllMocks();
  expect(vietate).toEqual([]);
});

describe("i metodi", () => {
  it("sono fra quelli che il gateway serve da sé, insieme ai 16 di prima", () => {
    for (const m of METODI) expect(METODI_TS.has(m)).toBe(true);
    for (const m of ["get_config", "save_config", "get_personalizzazione", "get_calendars", "get_events", "get_disponibilita", "add_local_event", "delete_local_event"]) {
      expect(METODI_TS.has(m)).toBe(true);
    }
    expect(METODI_TS.size).toBeGreaterThanOrEqual(19);
  });
});

describe("le regole del gateway valgono anche per loro", () => {
  it("senza sessione: 401 in JSON e il metodo non gira", async () => {
    for (const m of METODI) {
      const r = await handle(post(`/api/${m}`, m === "set_promemoria" ? [{ comparsa: false }] : ["push"]), deps(null));
      expect(r.status, m).toBe(401);
      expect(await r.json()).toEqual({ error: "non autenticato" });
    }
    expect(chiamate).toEqual([]);
  });

  it("un POST che parte da un altro sito: 403", async () => {
    for (const m of METODI) {
      for (const sito of ["cross-site", "same-site"]) expect((await handle(post(`/api/${m}`, ["push"], { "sec-fetch-site": sito }), deps(SESSIONE))).status, `${m} ${sito}`).toBe(403);
    }
    expect(chiamate).toEqual([]);
  });

  it("gli altri metodi HTTP: 405; un POST che non è JSON: 415; un corpo troppo grande: 413; un corpo che non è una lista: 400", async () => {
    for (const m of METODI) {
      for (const metodo of ["DELETE", "PUT", "PATCH", "OPTIONS"]) expect((await handle(req(`/api/${m}`, { method: metodo }), deps(SESSIONE))).status, `${m} ${metodo}`).toBe(405);
      expect((await handle(req(`/api/${m}`, { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), deps(SESSIONE))).status).toBe(415);
      expect((await handle(post(`/api/${m}`, "x".repeat(MAX_BODY + 1)), deps(SESSIONE))).status).toBe(413);
      for (const corpo of ['{"comparsa": false}', "5", "non json"]) expect((await handle(post(`/api/${m}`, corpo), deps(SESSIONE))).status, `${m} ${corpo}`).toBe(400);
    }
    expect(chiamate).toEqual([]);
  });

  it("un GET non chiama mai un metodo, anche se ha il suo nome", async () => {
    for (const m of METODI) {
      const r = await handle(req(`/api/${m}`), deps(SESSIONE));
      expect(r.status).toBe(404);
      expect(await r.json()).toEqual({ error: "non trovato" });
    }
    expect(chiamate).toEqual([]);
  });

  it("la risposta è JSON senza copia in cache", async () => {
    const r = await handle(post("/api/get_promemoria", []), deps(SESSIONE));
    expect(r.status).toBe(200);
    expect(r.headers.get("content-type")).toBe("application/json; charset=utf-8");
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(r.headers.get("x-content-type-options")).toBe("nosniff");
  });
});

describe("get_promemoria", () => {
  it("di norma: dieci minuti, completo, suono predefinito, notifica e avviso a comparsa accesi; notifiche del browser non configurate; nessun errore", async () => {
    expect(await api("get_promemoria")).toEqual({
      stato: 200,
      corpo: {
        promemoria: PREDEFINITE,
        canali: [{ nome: "push", configurato: false, chiavi: false, chiave_pubblica: null, iscrizioni: 0 }],
        ultimo_errore: null,
        ultimo_errore_il: null,
      },
    });
  });

  it("con le chiavi e un browser iscritto le notifiche del browser sono configurate, e la risposta non porta né la chiave privata né l'iscrizione", async () => {
    const k = chiaviVapidDiProva();
    Object.assign(process.env, k);
    expect((await api("iscrivi_push", [browserFinto("https://push.example.test/send/1").iscrizione])).corpo).toEqual({ ok: true });
    const r = await api("get_promemoria");
    expect(r.corpo.canali).toEqual([{ nome: "push", configurato: true, chiavi: true, chiave_pubblica: k.VAPID_PUBLIC_KEY, iscrizioni: 1 }]);
    expect(JSON.stringify(r)).not.toContain(k.VAPID_PRIVATE_KEY);
    expect(JSON.stringify(r)).not.toContain("push.example.test");
  });

  it("una voce scritta male con save_config non rompe la lettura: valgono i valori predefiniti", async () => {
    await api("save_config", [{ promemoria: { minuti: "dieci", contenuto: [], suono: 3, notifica_windows: "no", comparsa: 0 } }]);
    expect((await api("get_promemoria")).corpo.promemoria).toEqual(PREDEFINITE);
  });

  it("le voci di prima (attivo, canali) non contano e spariscono al primo salvataggio", async () => {
    await api("save_config", [{ promemoria: { attivo: true, canali: ["telegram"], minuti: [5], contenuto: "minimo" } }]);
    expect((await api("get_promemoria")).corpo.promemoria).toEqual({ ...PREDEFINITE, minuti: [5], contenuto: "minimo" });
    expect((await api("set_promemoria", [{ suono: "Mail" }])).corpo.ok).toBe(true);
    expect((await api("get_config")).corpo.promemoria).toEqual({ ...PREDEFINITE, minuti: [5], contenuto: "minimo", suono: "Mail" });
  });
});

describe("set_promemoria", () => {
  it("gli interruttori dell'app per Windows, minuti e contenuto si salvano, e get_promemoria e get_config lo mostrano", async () => {
    const r = await api("set_promemoria", [{ notifica_windows: false, comparsa: true, minuti: [15, 5, 15], contenuto: "minimo" }]);
    const atteso = { minuti: [5, 15], contenuto: "minimo", suono: "Default", notifica_windows: false, comparsa: true };
    expect(r).toEqual({ stato: 200, corpo: { ok: true, promemoria: atteso } });
    expect((await api("get_promemoria")).corpo.promemoria).toEqual(atteso);
    expect((await api("get_config")).corpo.promemoria).toEqual(atteso);
    expect((await api("set_promemoria", [{ comparsa: false }])).corpo.promemoria).toEqual({ ...atteso, comparsa: false });
  });

  it("si salvano l'orario all'inizio (0), fino a otto orari e un suono; i valori stanno in ordine e la voce sopravvive a una nuova lettura", async () => {
    const r = await api("set_promemoria", [{ minuti: [30, 0, 15, 60, 10, 5, 20, 45], suono: "Reminder" }]);
    expect(r.corpo).toEqual({ ok: true, promemoria: { ...PREDEFINITE, minuti: [0, 5, 10, 15, 20, 30, 45, 60], suono: "Reminder" } });
    expect((await api("get_promemoria")).corpo.promemoria).toEqual({ ...PREDEFINITE, minuti: [0, 5, 10, 15, 20, 30, 45, 60], suono: "Reminder" });
    expect((await api("set_promemoria", [{ minuti: [0] }])).corpo.promemoria.minuti).toEqual([0]);
  });

  it("tutti i suoni elencati dal server sono accettati (Alarm e Call da 1 a 10), un nome scritto male no", async () => {
    for (const suono of ["Default", "Reminder", "Mail", "IM", "SMS", "Alarm", "Alarm10", "Call", "Call7"]) {
      expect((await api("set_promemoria", [{ suono }])).corpo, suono).toMatchObject({ ok: true, promemoria: { suono } });
    }
    expect((await api("set_promemoria", [{ suono: "alarm" }])).corpo).toEqual({ ok: false, error: "Suono sconosciuto." });
    expect((await api("get_promemoria")).corpo.promemoria.suono).toBe("Call7");
  });

  it("i valori che non vanno sono rifiutati con una frase, senza eccezioni, e niente cambia", async () => {
    const rifiuti: [unknown, string][] = [
      [{ minuti: [-1] }, MINUTI_NO],
      [{ minuti: [] }, MINUTI_NO],
      [{ minuti: [1, 2, 3, 4, 5, 6, 7, 8, 9] }, MINUTI_NO],
      [{ minuti: [1441] }, MINUTI_NO],
      [{ minuti: [1.5] }, MINUTI_NO],
      [{ suono: "Fischio" }, "Suono sconosciuto."],
      [{ suono: 3 }, "Suono sconosciuto."],
      [{ contenuto: "tutto" }, "Il contenuto è «completo» o «minimo»."],
      [{ notifica_windows: "sì" }, "«notifica_windows» deve essere vero o falso."],
      [{ comparsa: 1 }, "«comparsa» deve essere vero o falso."],
      [{ attivo: true }, "Impostazione sconosciuta."],
      [{ canali: ["push"] }, "Impostazione sconosciuta."],
      [{ altro: 1 }, "Impostazione sconosciuta."],
      ["testo", "Le impostazioni dei promemoria non sono valide."],
      [null, "Le impostazioni dei promemoria non sono valide."],
    ];
    for (const [corpo, frase] of rifiuti) expect(await api("set_promemoria", [corpo]), JSON.stringify(corpo)).toEqual({ stato: 200, corpo: { ok: false, error: frase } });
    expect(await api("set_promemoria", [])).toEqual({ stato: 200, corpo: { ok: false, error: "Le impostazioni dei promemoria non sono valide." } });
    expect((await api("get_promemoria")).corpo.promemoria).toEqual(PREDEFINITE);
  });
});

describe("prova_promemoria", () => {
  it("canale sconosciuto o non configurato: una frase, e niente parte", async () => {
    expect(await api("prova_promemoria", ["push"])).toEqual({ stato: 200, corpo: { ok: false, error: "Canale non configurato sul server." } });
    for (const args of [["whatsapp"], [], [5], [null], [{ nome: "push" }], [["push"]]]) {
      expect(await api("prova_promemoria", args), JSON.stringify(args)).toEqual({ stato: 200, corpo: { ok: false, error: "Canale sconosciuto." } });
    }
  });
});
