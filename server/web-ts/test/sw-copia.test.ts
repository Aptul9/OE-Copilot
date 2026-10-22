import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { describe, expect, it } from "vitest";

// La copia di public/sw.js per quando manca la rete, con Cache Storage e rete finti: la rete risponde da `rotte` finché `inRete` è vero.
const SITO = "https://oe.test";

function cacheFinta() {
  const voci = new Map<string, Response>();
  const url = (k: string | Request) => new URL(typeof k === "string" ? k : k.url, SITO).href;
  return {
    voci,
    async match(k: string | Request) {
      return voci.get(url(k))?.clone();
    },
    async put(k: string | Request, r: Response) {
      voci.delete(url(k));
      voci.set(url(k), r);
    },
    async keys() {
      return [...voci.keys()].map((u) => new Request(u));
    },
    async delete(k: string | Request) {
      return voci.delete(url(k));
    },
    async add(k: string) {
      voci.set(url(k), new Response(`file ${k}`));
    },
  };
}

function carica({ cacheRotta = false } = {}) {
  const codice = fs.readFileSync(path.join(process.cwd(), "public", "sw.js"), "utf-8");
  const ascoltatori: Record<string, (e: unknown) => void> = {};
  const negozi = new Map<string, ReturnType<typeof cacheFinta>>();
  const caches = {
    async open(n: string) {
      if (cacheRotta) throw new DOMException("Unexpected internal error.", "UnknownError");
      if (!negozi.has(n)) negozi.set(n, cacheFinta());
      return negozi.get(n)!;
    },
    async keys() {
      return [...negozi.keys()];
    },
    async delete(n: string) {
      return negozi.delete(n);
    },
  };
  const stato = { inRete: true, rotte: {} as Record<string, () => Response> };
  const fetch = async (r: string | Request) => {
    if (!stato.inRete) throw new TypeError("Failed to fetch");
    const u = new URL(typeof r === "string" ? r : r.url, SITO).pathname;
    return stato.rotte[u]?.() ?? new Response("", { status: 404 });
  };
  const self = {
    addEventListener: (nome: string, f: (e: unknown) => void) => void (ascoltatori[nome] = f),
    location: { origin: SITO },
    skipWaiting() {},
    clients: { claim: async () => {} },
  };
  vm.runInNewContext(codice, { self, caches, fetch, Request, Response, URL, setTimeout });

  /** Un fetch della pagina: la risposta del service worker, o null se la lascia passare. */
  async function chiedi(richiesta: Request | { url: string; method: string; mode: string }) {
    const tenuta: { risposta: Promise<Response> | null } = { risposta: null };
    const attese: Promise<unknown>[] = [];
    ascoltatori.fetch({ request: richiesta, respondWith: (p: Promise<Response>) => void (tenuta.risposta = p), waitUntil: (p: Promise<unknown>) => void attese.push(p) });
    const r = tenuta.risposta ? await tenuta.risposta : null;
    await Promise.all(attese);
    return r;
  }
  const api = (metodo: string, ...args: unknown[]) => chiedi(new Request(`${SITO}/api/${metodo}`, { method: "POST", body: JSON.stringify(args) }));
  const attiva = async () => {
    const attese: Promise<unknown>[] = [];
    ascoltatori.activate({ waitUntil: (p: Promise<unknown>) => void attese.push(p) });
    await Promise.all(attese);
  };
  return { stato, negozi, api, chiedi, attiva };
}

const json = (dati: unknown, extra: ResponseInit = {}) => new Response(JSON.stringify(dati), { headers: { "content-type": "application/json", date: new Date(Date.UTC(2026, 9, 22, 6, 30)).toUTCString() }, ...extra });
const ev = (id: string, start: string) => ({ id, title: id, start, end: start.replace("T10", "T11"), allDay: false, account: "a" });

describe("public/sw.js senza rete", () => {
  it("get_events: senza rete gli eventi di tutte le letture salvate nell'intervallo, detti copia", async () => {
    const sw = carica();
    let eventi = [ev("lun", "2026-10-19T10:00:00")];
    sw.stato.rotte["/api/get_events"] = () => json({ events: eventi, errors: [], fonti: { A: { stato: "ok", msg: "" } } });
    await sw.api("get_events", "2026-10-19T00:00:00+03:00", "2026-10-26T00:00:00+02:00", null, ["x"]);
    eventi = [ev("mer2", "2026-10-28T10:00:00")];
    await sw.api("get_events", "2026-10-26T00:00:00+02:00", "2026-11-02T00:00:00+02:00", null, ["x"]);
    eventi = [ev("tutti", "2026-10-20T10:00:00")];
    await sw.api("get_events", "2026-10-19T00:00:00+03:00", "2026-10-26T00:00:00+02:00", null, []);

    sw.stato.inRete = false;
    const r = await (await sw.api("get_events", "2026-10-19T00:00:00+03:00", "2026-11-02T00:00:00+02:00", null, ["x"]))!.json();
    // solo le letture con gli stessi calendari spenti, quando ce ne sono
    expect(r.events.map((e: { id: string }) => e.id)).toEqual(["lun", "mer2"]);
    expect(r).toMatchObject({ outlook_starting: true, stale: true, avviso_sorgente: "Senza rete", fonti: { A: { stato: "ok" } } });
    expect(r.stale_at).toMatch(/^2026-10-22T\d\d:30:00$/);
    // altri calendari spenti, mai letti così: valgono tutte, fuori intervallo no
    const altri = await (await sw.api("get_events", "2026-10-19T00:00:00+03:00", "2026-10-26T00:00:00+02:00", null, ["y"]))!.json();
    expect(altri.events.map((e: { id: string }) => e.id).sort()).toEqual(["lun", "tutti"]);
  });

  it("le altre letture rispondono con l'ultima copia; le scritture e un 401 non si toccano", async () => {
    const sw = carica();
    sw.stato.rotte["/api/get_config"] = () => json({ theme: "dark" });
    sw.stato.rotte["/api/get_calendars"] = () => json({ error: "sessione" }, { status: 401 });
    await sw.api("get_config");
    await sw.api("get_calendars", false);
    expect(await sw.api("save_config", { theme: "light" })).toBeNull();
    sw.stato.inRete = false;
    expect(await (await sw.api("get_config"))!.json()).toEqual({ theme: "dark" });
    await expect(sw.api("get_calendars", false)).rejects.toThrow("Failed to fetch");
  });

  it("la pagina e i suoi file si salvano all'attivazione; senza rete la pagina è la copia", async () => {
    const sw = carica();
    sw.stato.rotte["/"] = () => new Response('<script src="/_next/static/chunks/app-1.js"></script><link href="/_next/static/css/a.css">');
    await sw.attiva();
    sw.stato.inRete = false;
    const pagina = await sw.chiedi({ url: `${SITO}/`, method: "GET", mode: "navigate" });
    expect(await pagina!.text()).toContain("app-1.js");
    expect(await (await sw.chiedi(new Request(`${SITO}/_next/static/chunks/app-1.js`)))!.text()).toBe("file /_next/static/chunks/app-1.js");
    // il telefono e le altre pagine passano sempre dalla rete
    expect(await sw.chiedi({ url: `${SITO}/telefono/`, method: "GET", mode: "navigate" })).toBeNull();
    expect(await sw.chiedi({ url: `${SITO}/login`, method: "GET", mode: "navigate" })).toBeNull();
  });

  it("una Cache Storage che non si apre non ferma niente: tutto dalla rete", async () => {
    const sw = carica({ cacheRotta: true });
    sw.stato.rotte["/api/get_config"] = () => json({ theme: "dark" });
    sw.stato.rotte["/"] = () => new Response("pagina");
    sw.stato.rotte["/_next/static/a.js"] = () => new Response("file");
    await sw.attiva();
    expect(await (await sw.api("get_config"))!.json()).toEqual({ theme: "dark" });
    expect(await (await sw.chiedi({ url: `${SITO}/`, method: "GET", mode: "navigate" }))!.text()).toBe("pagina");
    expect(await (await sw.chiedi(new Request(`${SITO}/_next/static/a.js`)))!.text()).toBe("file");
    sw.stato.inRete = false;
    await expect(sw.api("get_config")).rejects.toThrow("Failed to fetch");
  });

  it("l'uscita cancella pagina e dati salvati", async () => {
    const sw = carica();
    sw.stato.rotte["/api/get_config"] = () => json({});
    await sw.api("get_config");
    expect([...sw.negozi.keys()]).toContain("oe-1-dati");
    await sw.chiedi(new Request(`${SITO}/api/auth/sign-out`, { method: "POST", body: "{}" }));
    expect([...sw.negozi.keys()]).not.toContain("oe-1-dati");
  });
});
