import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { fissaOra } from "@/lib/core/orologio";
import { handle, MAX_BODY, type Deps } from "@/lib/gateway";
import { avviaTelegramFinto, CHAT_DI_PROVA, soloRetiLocali, TOKEN_DI_PROVA } from "./telegram-finto";

// I tre metodi dei promemoria attraverso il gateway vero (handle, con i metodi veri di lib/core): le regole di sempre (401, 403, 405, 413, 415)
// e il comportamento, con un Telegram finto in locale.

const METODI = ["get_promemoria", "set_promemoria", "prova_promemoria"];

let dati: string;
let finto: Awaited<ReturnType<typeof avviaTelegramFinto>>;
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
const configuraTelegram = () => {
  process.env.TELEGRAM_BOT_TOKEN = TOKEN_DI_PROVA;
  process.env.TELEGRAM_CHAT_ID = CHAT_DI_PROVA;
  process.env.TELEGRAM_API_URL = finto.url;
};

beforeEach(async () => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-promemoria-api-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  for (const nome of ["TELEGRAM_BOT_TOKEN", "TELEGRAM_CHAT_ID", "TELEGRAM_API_URL"]) delete process.env[nome];
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  finto = await avviaTelegramFinto();
  vietate = soloRetiLocali().vietate;
  chiamate = [];
  fissaOra(Date.UTC(2026, 9, 12, 13, 20, 0));
});

afterEach(async () => {
  await finto.chiudi();
  fissaOra(null);
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  for (const nome of ["OE_DATI", "TELEGRAM_BOT_TOKEN", "TELEGRAM_CHAT_ID", "TELEGRAM_API_URL"]) delete process.env[nome];
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
  it("senza sessione: 401 in JSON, il metodo non gira e Telegram non è toccato", async () => {
    configuraTelegram();
    for (const m of METODI) {
      const r = await handle(post(`/api/${m}`, m === "set_promemoria" ? [{ attivo: true }] : ["telegram"]), deps(null));
      expect(r.status, m).toBe(401);
      expect(await r.json()).toEqual({ error: "non autenticato" });
    }
    expect(chiamate).toEqual([]);
    expect(finto.richieste).toEqual([]);
  });

  it("un POST che parte da un altro sito: 403", async () => {
    configuraTelegram();
    for (const m of METODI) {
      for (const sito of ["cross-site", "same-site"]) expect((await handle(post(`/api/${m}`, ["telegram"], { "sec-fetch-site": sito }), deps(SESSIONE))).status, `${m} ${sito}`).toBe(403);
    }
    expect(chiamate).toEqual([]);
    expect(finto.richieste).toEqual([]);
  });

  it("gli altri metodi HTTP: 405; un POST che non è JSON: 415; un corpo troppo grande: 413; un corpo che non è una lista: 400", async () => {
    for (const m of METODI) {
      for (const metodo of ["DELETE", "PUT", "PATCH", "OPTIONS"]) expect((await handle(req(`/api/${m}`, { method: metodo }), deps(SESSIONE))).status, `${m} ${metodo}`).toBe(405);
      expect((await handle(req(`/api/${m}`, { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), deps(SESSIONE))).status).toBe(415);
      expect((await handle(post(`/api/${m}`, "x".repeat(MAX_BODY + 1)), deps(SESSIONE))).status).toBe(413);
      for (const corpo of ['{"attivo": true}', "5", "non json"]) expect((await handle(post(`/api/${m}`, corpo), deps(SESSIONE))).status, `${m} ${corpo}`).toBe(400);
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
  it("di norma: spento, dieci minuti, Telegram, completo; nessun canale configurato; nessun giro, nessun errore", async () => {
    expect(await api("get_promemoria")).toEqual({
      stato: 200,
      corpo: {
        promemoria: { attivo: false, minuti: [10], canali: ["telegram"], contenuto: "completo" },
        canali: [
          { nome: "telegram", configurato: false },
          { nome: "push", configurato: false, chiavi: false, chiave_pubblica: null, iscrizioni: 0 },
        ],
        ultimo_giro: null,
        ultimo_errore: null,
        ultimo_errore_il: null,
      },
    });
  });

  it("con TELEGRAM_BOT_TOKEN e TELEGRAM_CHAT_ID il canale è configurato, e la risposta non contiene né il token né la chat", async () => {
    configuraTelegram();
    const r = await api("get_promemoria");
    expect(r.corpo.canali[0]).toEqual({ nome: "telegram", configurato: true });
    expect(JSON.stringify(r)).not.toContain(TOKEN_DI_PROVA);
    expect(JSON.stringify(r)).not.toContain("SEGRETO");
    expect(JSON.stringify(r)).not.toContain(CHAT_DI_PROVA);
    expect(JSON.stringify(r)).not.toContain(finto.url);
    process.env.TELEGRAM_CHAT_ID = "";
    expect((await api("get_promemoria")).corpo.canali[0]).toEqual({ nome: "telegram", configurato: false });
  });

  it("una voce scritta male con save_config non rompe la lettura: valgono i valori predefiniti", async () => {
    await api("save_config", [{ promemoria: { attivo: "sì", minuti: "dieci", canali: 7, contenuto: [] } }]);
    expect((await api("get_promemoria")).corpo.promemoria).toEqual({ attivo: false, minuti: [10], canali: ["telegram"], contenuto: "completo" });
  });
});

describe("set_promemoria", () => {
  it("non si accende senza un canale configurato: risposta di rifiuto, lo stato non cambia", async () => {
    const r = await api("set_promemoria", [{ attivo: true }]);
    expect(r).toEqual({ stato: 200, corpo: { ok: false, error: "Nessuno dei canali scelti è configurato sul server." } });
    expect((await api("get_promemoria")).corpo.promemoria.attivo).toBe(false);
  });

  it("con il canale configurato si accende, si cambiano minuti e contenuto, e get_promemoria e get_config lo mostrano", async () => {
    configuraTelegram();
    const r = await api("set_promemoria", [{ attivo: true, minuti: [15, 5, 15], contenuto: "minimo" }]);
    expect(r).toEqual({ stato: 200, corpo: { ok: true, promemoria: { attivo: true, minuti: [5, 15], canali: ["telegram"], contenuto: "minimo" } } });
    expect((await api("get_promemoria")).corpo.promemoria).toEqual({ attivo: true, minuti: [5, 15], canali: ["telegram"], contenuto: "minimo" });
    // è una voce della configurazione come le altre, e non porta niente del canale
    const config = (await api("get_config")).corpo;
    expect(config.promemoria).toEqual({ attivo: true, minuti: [5, 15], canali: ["telegram"], contenuto: "minimo" });
    expect(JSON.stringify(config)).not.toContain("SEGRETO");
    expect(JSON.stringify(config)).not.toContain(CHAT_DI_PROVA);
  });

  it("i valori che non vanno sono rifiutati con una frase, senza eccezioni", async () => {
    configuraTelegram();
    const rifiuti: [unknown, string][] = [
      [{ minuti: [0] }, "I minuti sono da 1 a 5 numeri interi diversi, ciascuno da 1 a 1440."],
      [{ minuti: [1, 2, 3, 4, 5, 6] }, "I minuti sono da 1 a 5 numeri interi diversi, ciascuno da 1 a 1440."],
      [{ minuti: [1441] }, "I minuti sono da 1 a 5 numeri interi diversi, ciascuno da 1 a 1440."],
      [{ canali: ["whatsapp"] }, "Canale sconosciuto."],
      [{ contenuto: "tutto" }, "Il contenuto è «completo» o «minimo»."],
      [{ attivo: "sì" }, "«attivo» deve essere vero o falso."],
      [{ altro: 1 }, "Impostazione sconosciuta."],
      ["testo", "Le impostazioni dei promemoria non sono valide."],
      [null, "Le impostazioni dei promemoria non sono valide."],
    ];
    for (const [corpo, frase] of rifiuti) expect(await api("set_promemoria", [corpo]), JSON.stringify(corpo)).toEqual({ stato: 200, corpo: { ok: false, error: frase } });
    expect(await api("set_promemoria", [])).toEqual({ stato: 200, corpo: { ok: false, error: "Le impostazioni dei promemoria non sono valide." } });
    expect((await api("get_promemoria")).corpo.promemoria).toEqual({ attivo: false, minuti: [10], canali: ["telegram"], contenuto: "completo" });
  });
});

describe("prova_promemoria", () => {
  it("manda su Telegram il testo fisso e nient'altro: nessun dato di un evento, nessuna lettura di eventi", async () => {
    configuraTelegram();
    expect(await api("prova_promemoria", ["telegram"])).toEqual({ stato: 200, corpo: { ok: true } });
    expect(finto.richieste).toHaveLength(1);
    expect(finto.richieste[0].percorso).toBe(`/bot${TOKEN_DI_PROVA}/sendMessage`);
    expect(finto.richieste[0].corpo).toEqual({ chat_id: CHAT_DI_PROVA, text: "Prova dei promemoria", link_preview_options: { is_disabled: true } });
  });

  it("funziona anche a promemoria spenti (si prova prima di accendere), e una prova non accende niente", async () => {
    configuraTelegram();
    await api("prova_promemoria", ["telegram"]);
    expect((await api("get_promemoria")).corpo.promemoria.attivo).toBe(false);
  });

  it("canale sconosciuto o non configurato: una frase, e niente parte", async () => {
    expect(await api("prova_promemoria", ["telegram"])).toEqual({ stato: 200, corpo: { ok: false, error: "Canale non configurato sul server." } });
    configuraTelegram();
    for (const args of [["whatsapp"], [], [5], [null], [{ nome: "telegram" }], [["telegram"]]]) {
      expect(await api("prova_promemoria", args), JSON.stringify(args)).toEqual({ stato: 200, corpo: { ok: false, error: "Canale sconosciuto." } });
    }
    expect(finto.richieste).toEqual([]);
  });

  it("se Telegram rifiuta, la risposta dice perché con una frase fissa: né il token né quello che ha risposto Telegram", async () => {
    configuraTelegram();
    finto.risposte.push({ stato: 401, corpo: { ok: false, error_code: 401, description: `Unauthorized: bot${TOKEN_DI_PROVA}` } });
    const r = await api("prova_promemoria", ["telegram"]);
    expect(r.corpo).toEqual({ ok: false, error: "Telegram non riconosce il bot (401): controlla TELEGRAM_BOT_TOKEN" });
    expect(JSON.stringify(r)).not.toContain("SEGRETO");
    finto.risposte.push({ stato: 429, corpo: { ok: false, parameters: { retry_after: 3 } } });
    expect((await api("prova_promemoria", ["telegram"])).corpo).toEqual({ ok: false, error: "Telegram limita gli invii (429)" });
  });
});
