import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { canaleTelegram, tronca } from "@/lib/core/promemoria-canali";
import { avviaTelegramFinto, CHAT_DI_PROVA, soloRetiLocali, TOKEN_DI_PROVA } from "./telegram-finto";

// Il canale Telegram contro un server HTTP locale che ne fa le veci: il percorso, il corpo, l'esito per ogni errore. Mai api.telegram.org.

let finto: Awaited<ReturnType<typeof avviaTelegramFinto>>;
let vietate: string[];

beforeEach(async () => {
  finto = await avviaTelegramFinto();
  vietate = soloRetiLocali().vietate;
  process.env.TELEGRAM_BOT_TOKEN = TOKEN_DI_PROVA;
  process.env.TELEGRAM_CHAT_ID = CHAT_DI_PROVA;
  process.env.TELEGRAM_API_URL = finto.url;
});

afterEach(async () => {
  await finto.chiudi();
  for (const nome of ["TELEGRAM_BOT_TOKEN", "TELEGRAM_CHAT_ID", "TELEGRAM_API_URL"]) delete process.env[nome];
  vi.restoreAllMocks();
  expect(vietate).toEqual([]);
});

describe("la configurazione", () => {
  it("è configurato solo con il token e la chat, e si chiama telegram", () => {
    const c = canaleTelegram();
    expect(c.nome).toBe("telegram");
    expect(c.configurato()).toBe(true);
    delete process.env.TELEGRAM_CHAT_ID;
    expect(c.configurato()).toBe(false);
    process.env.TELEGRAM_CHAT_ID = CHAT_DI_PROVA;
    process.env.TELEGRAM_BOT_TOKEN = "";
    expect(c.configurato()).toBe(false);
    process.env.TELEGRAM_BOT_TOKEN = "   ";
    expect(c.configurato()).toBe(false);
  });

  it("gli spazi attorno ai valori di .env non contano", async () => {
    process.env.TELEGRAM_BOT_TOKEN = ` ${TOKEN_DI_PROVA}\n`;
    process.env.TELEGRAM_CHAT_ID = `${CHAT_DI_PROVA} `;
    expect(await canaleTelegram().invia("x")).toEqual({ ok: true });
    expect(finto.richieste[0].percorso).toBe(`/bot${TOKEN_DI_PROVA}/sendMessage`);
    expect(finto.richieste[0].corpo.chat_id).toBe(CHAT_DI_PROVA);
  });
});

describe("l'invio", () => {
  it("POST /bot<token>/sendMessage con un corpo JSON: chat, testo, niente anteprima del link, niente parse_mode", async () => {
    expect(await canaleTelegram().invia("Promemoria: tra 10 minuti\n<b>Riunione</b> di prova & co.")).toEqual({ ok: true });
    expect(finto.richieste).toHaveLength(1);
    const [r] = finto.richieste;
    expect(r.metodo).toBe("POST");
    expect(r.percorso).toBe(`/bot${TOKEN_DI_PROVA}/sendMessage`);
    expect(r.tipo).toMatch(/^application\/json/);
    expect(r.corpo).toEqual({
      chat_id: CHAT_DI_PROVA,
      text: "Promemoria: tra 10 minuti\n<b>Riunione</b> di prova & co.",
      link_preview_options: { is_disabled: true },
    });
    expect(r.corpo).not.toHaveProperty("parse_mode");
  });

  it("l'indirizzo dell'API si può cambiare (e un / finale non conta), di norma è api.telegram.org", async () => {
    process.env.TELEGRAM_API_URL = finto.url + "///";
    expect(await canaleTelegram().invia("x")).toEqual({ ok: true });
    expect(finto.richieste[0].percorso).toBe(`/bot${TOKEN_DI_PROVA}/sendMessage`);
    delete process.env.TELEGRAM_API_URL;
    // senza TELEGRAM_API_URL la richiesta va al servizio vero: qui il filtro delle prove la ferma prima che esca, e si vede dove andava
    const esito = await canaleTelegram().invia("x");
    expect(esito.ok).toBe(false);
    expect(vietate).toEqual(["api.telegram.org"]);
    vietate.length = 0;
  });

  it("un testo oltre 4096 caratteri si accorcia, e non si spezza una coppia surrogata", async () => {
    await canaleTelegram().invia("a".repeat(5000));
    const testo = finto.richieste[0].corpo.text as string;
    expect(testo).toHaveLength(4096);
    expect(testo.endsWith("…")).toBe(true);
    expect(tronca("abc", 4096)).toBe("abc");
    // il taglio cadrebbe fra le due metà dell'emoji: va indietro di una
    const corto = tronca("x".repeat(4094) + "😀" + "y");
    expect(corto).toBe("x".repeat(4094) + "…");
    // se l'emoji ci sta tutta resta intera
    expect(tronca("x".repeat(4093) + "😀😀")).toBe("x".repeat(4093) + "😀…");
  });
});

describe("gli errori diventano frasi fisse, senza il token e senza quello che ha risposto Telegram", () => {
  const casi: [string, number, unknown][] = [
    ["401", 401, { ok: false, error_code: 401, description: "Unauthorized: Riunione di prova" }],
    ["404 (token scritto male)", 404, { ok: false, error_code: 404, description: "Not Found" }],
    ["403", 403, { ok: false, error_code: 403, description: "Forbidden: bot was blocked by the user" }],
    ["400", 400, { ok: false, error_code: 400, description: "Bad Request: chat not found" }],
    ["429", 429, { ok: false, error_code: 429, description: "Too Many Requests: retry after 7", parameters: { retry_after: 7 } }],
    ["500", 500, { ok: false, error_code: 500, description: "Internal Server Error" }],
    ["502", 502, "non json"],
    ["418", 418, {}],
  ];

  it.each(casi)("%s", async (_nome, stato, corpo) => {
    finto.risposte.push(typeof corpo === "string" ? { stato, testo: corpo } : { stato, corpo });
    const esito = await canaleTelegram().invia("Riunione di prova");
    expect(esito.ok).toBe(false);
    if (esito.ok) return;
    expect(esito.errore).not.toContain(TOKEN_DI_PROVA);
    expect(esito.errore).not.toContain(CHAT_DI_PROVA);
    expect(esito.errore).not.toContain("Riunione");
    expect(esito.errore).not.toMatch(/Unauthorized|Forbidden|Bad Request|chat not found|Internal Server/);
  });

  it("401 e 404: il token; 403: il bot non può scrivere; 400: la chat", async () => {
    const chiedi = async (stato: number, corpo: unknown = {}) => {
      finto.risposte.push({ stato, corpo });
      const e = await canaleTelegram().invia("x");
      return e.ok ? "" : e.errore;
    };
    expect(await chiedi(401)).toContain("TELEGRAM_BOT_TOKEN");
    expect(await chiedi(404)).toContain("TELEGRAM_BOT_TOKEN");
    expect(await chiedi(403)).toContain("403");
    expect(await chiedi(400)).toContain("TELEGRAM_CHAT_ID");
    expect(await chiedi(400, { parameters: { migrate_to_chat_id: -1001234567890 } })).toContain("supergruppo");
    expect(await chiedi(500)).toBe("Telegram non disponibile (500)");
    expect(await chiedi(503)).toBe("Telegram non disponibile (503)");
  });

  it("429: l'attesa è retry_after del corpo, poi l'intestazione Retry-After, poi 30 secondi", async () => {
    const attesa = async (r: { corpo?: unknown; intestazioni?: Record<string, string> }) => {
      finto.risposte.push({ stato: 429, ...r });
      const e = await canaleTelegram().invia("x");
      return e.ok ? null : e.attesaS;
    };
    expect(await attesa({ corpo: { ok: false, parameters: { retry_after: 12 } } })).toBe(12);
    expect(await attesa({ corpo: {}, intestazioni: { "Retry-After": "5" } })).toBe(5);
    expect(await attesa({ corpo: {} })).toBe(30);
    expect(await attesa({ corpo: { parameters: { retry_after: "boh" } } })).toBe(30);
    expect(await attesa({ corpo: { parameters: { retry_after: 0 } } })).toBe(30);
  });

  it("200 senza ok:true, o con un corpo che non è JSON, non è un invio riuscito", async () => {
    finto.risposte.push({ stato: 200, corpo: { ok: false } }, { stato: 200, testo: "<html>" });
    for (let i = 0; i < 2; i++) expect((await canaleTelegram().invia("x")).ok).toBe(false);
  });

  it("nessuna risposta in 10 secondi (qui 150 ms): tempo scaduto", async () => {
    finto.risposte.push({ stato: 200, corpo: { ok: true }, ritardoMs: 1500 });
    const esito = await canaleTelegram({ attesaMs: 150 }).invia("x");
    expect(esito).toEqual({ ok: false, errore: "Telegram non risponde (tempo scaduto)" });
  });

  it("server che non c'è: non raggiungibile, e il testo non nomina l'indirizzo (dentro c'è il token)", async () => {
    await finto.chiudi();
    const esito = await canaleTelegram().invia("x");
    expect(esito).toEqual({ ok: false, errore: "Telegram non raggiungibile" });
  });
});
