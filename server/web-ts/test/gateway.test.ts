import { describe, expect, it } from "vitest";
import { handle, MAX_BODY, type Deps } from "@/lib/gateway";

type Call = { url: string; init: RequestInit };

function setup(user: unknown, reply: () => Response = () => new Response("pagina", { status: 200, headers: { "content-type": "text/html", "x-up": "1", "set-cookie": "a=b" } })) {
  const calls: Call[] = [];
  const deps: Deps = {
    user: async () => user,
    upstream: "http://ui:8181",
    fetch: (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return reply();
    }) as unknown as typeof fetch,
  };
  return { calls, deps };
}
const req = (path: string, init: RequestInit = {}) => new Request("http://gateway:3000" + path, init);
const post = (path: string, body: string, headers: Record<string, string> = {}) =>
  req(path, { method: "POST", body, headers: { "content-type": "application/json", ...headers } });

describe("senza sessione", () => {
  it("una pagina rimanda all'accesso, con la pagina di partenza", async () => {
    const { deps, calls } = setup(null);
    const r = await handle(req("/app.js?v=2"), deps);
    expect(r.status).toBe(302);
    expect(r.headers.get("location")).toBe("/login?next=%2Fapp.js%3Fv%3D2");
    expect(calls).toHaveLength(0);
  });
  it("l'API risponde 401 in JSON e non chiama il servizio", async () => {
    const { deps, calls } = setup(null);
    for (const r of [await handle(req("/api/get_events"), deps), await handle(post("/api/get_events", "[]"), deps)]) {
      expect(r.status).toBe(401);
      expect(r.headers.get("content-type")).toContain("application/json");
      expect(await r.json()).toEqual({ error: "non autenticato" });
    }
    expect(calls).toHaveLength(0);
  });
});

describe("con la sessione", () => {
  it("inoltra la pagina senza il cookie di sessione e senza set-cookie in risposta", async () => {
    const { deps, calls } = setup({ id: "u" });
    const r = await handle(req("/logos/personale.png?x=1", { headers: { cookie: "better-auth.session_token=segreto", accept: "text/html" } }), deps);
    expect(r.status).toBe(200);
    expect(await r.text()).toBe("pagina");
    expect(calls[0].url).toBe("http://ui:8181/logos/personale.png?x=1");
    const h = new Headers(calls[0].init.headers);
    expect(h.get("cookie")).toBeNull();
    expect(h.get("accept")).toBe("text/html");
    expect(r.headers.get("x-up")).toBe("1");
    expect(r.headers.get("set-cookie")).toBeNull();
    expect(r.headers.get("cache-control")).toBe("no-store");
  });

  it("un POST arriva intero e con Content-Length (il servizio ui non legge corpi a pezzi)", async () => {
    const { deps, calls } = setup({ id: "u" });
    const corpo = JSON.stringify(["2026-10-11T00:00:00+02:00", "2026-10-12T00:00:00+02:00"]);
    await handle(post("/api/get_events", corpo, { "sec-fetch-site": "same-origin" }), deps);
    const h = new Headers(calls[0].init.headers);
    expect(h.get("content-length")).toBe(String(Buffer.byteLength(corpo)));
    expect(Buffer.from(calls[0].init.body as Uint8Array).toString()).toBe(corpo);
    expect(calls[0].init.method).toBe("POST");
  });

  it("un POST senza JSON è rifiutato (415), come in server.py", async () => {
    const { deps, calls } = setup({ id: "u" });
    const r = await handle(req("/api/get_config", { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), deps);
    expect(r.status).toBe(415);
    expect(calls).toHaveLength(0);
  });

  it("un POST partito da un altro sito è rifiutato (403)", async () => {
    const { deps, calls } = setup({ id: "u" });
    for (const sito of ["cross-site", "same-site"]) {
      expect((await handle(post("/api/get_config", "[]", { "sec-fetch-site": sito }), deps)).status).toBe(403);
    }
    expect((await handle(post("/api/get_config", "[]", { "sec-fetch-site": "none" }), deps)).status).toBe(200);
    expect(calls).toHaveLength(1);
  });

  it("un corpo troppo grande è rifiutato (413)", async () => {
    const { deps, calls } = setup({ id: "u" });
    expect((await handle(post("/api/add_local_event", "x".repeat(MAX_BODY + 1)), deps)).status).toBe(413);
    expect(calls).toHaveLength(0);
  });

  it("gli altri metodi sono rifiutati (405)", async () => {
    const { deps } = setup({ id: "u" });
    expect((await handle(req("/api/x", { method: "DELETE" }), deps)).status).toBe(405);
  });

  it("HEAD non porta il corpo", async () => {
    const { deps } = setup({ id: "u" });
    const r = await handle(req("/", { method: "HEAD" }), deps);
    expect(r.status).toBe(200);
    expect(await r.text()).toBe("");
  });

  it("riporta lo stato del servizio ui (404, 500)", async () => {
    for (const stato of [404, 500]) {
      const { deps } = setup({ id: "u" }, () => Response.json({ error: "x" }, { status: stato }));
      expect((await handle(req("/api/win_close", { method: "GET" }), deps)).status).toBe(stato);
    }
  });

  describe("metodi serviti dal gateway (lib/core)", () => {
    const metodi = (chiamate: [string, unknown[]][] = []) => ({
      ha: (nome: string) => nome === "get_config" || nome === "rompe",
      chiama: async (nome: string, args: unknown[]) => {
        chiamate.push([nome, args]);
        if (nome === "rompe") throw new Error("guasto di prova");
        return { theme: "dark", args };
      },
    });

    it("un metodo del gateway risponde da sé, in JSON, senza chiamare il servizio ui", async () => {
      const { deps, calls } = setup({ id: "u" });
      const chiamate: [string, unknown[]][] = [];
      const r = await handle(post("/api/get_config", '["a", 1]'), { ...deps, metodi: metodi(chiamate) });
      expect(r.status).toBe(200);
      expect(r.headers.get("content-type")).toBe("application/json; charset=utf-8");
      expect(r.headers.get("cache-control")).toBe("no-store");
      expect(r.headers.get("x-content-type-options")).toBe("nosniff");
      expect(await r.json()).toEqual({ theme: "dark", args: ["a", 1] });
      expect(chiamate).toEqual([["get_config", ["a", 1]]]);
      expect(calls).toHaveLength(0);
    });

    it("un corpo vuoto è una lista vuota; un corpo che non è una lista, o non è JSON, è 400", async () => {
      const { deps } = setup({ id: "u" });
      expect(await (await handle(post("/api/get_config", ""), { ...deps, metodi: metodi() })).json()).toEqual({ theme: "dark", args: [] });
      for (const corpo of ['{"a": 1}', "5", "non json"]) {
        const r = await handle(post("/api/get_config", corpo), { ...deps, metodi: metodi() });
        expect(r.status).toBe(400);
        expect(typeof (await r.json()).error).toBe("string");
      }
    });

    it("un metodo che solleva è 500 con il suo testo", async () => {
      const { deps } = setup({ id: "u" });
      const r = await handle(post("/api/rompe", "[]"), { ...deps, metodi: metodi() });
      expect(r.status).toBe(500);
      expect(await r.json()).toEqual({ error: "guasto di prova" });
    });

    it("gli altri metodi, e una richiesta che non è un POST, passano al servizio ui; le regole contro altri siti valgono prima", async () => {
      const { deps, calls } = setup({ id: "u" });
      await handle(post("/api/set_account_alias", '["a", "b"]'), { ...deps, metodi: metodi() });
      await handle(req("/api/get_config"), { ...deps, metodi: metodi() });
      expect(calls.map((c) => c.url)).toEqual(["http://ui:8181/api/set_account_alias", "http://ui:8181/api/get_config"]);
      expect((await handle(post("/api/get_config", "[]", { "sec-fetch-site": "cross-site" }), { ...deps, metodi: metodi() })).status).toBe(403);
      expect((await handle(req("/api/get_config", { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), { ...deps, metodi: metodi() })).status).toBe(415);
    });

    it("senza sessione resta 401, e il metodo non gira", async () => {
      const { deps } = setup(null);
      const chiamate: [string, unknown[]][] = [];
      expect((await handle(post("/api/get_config", "[]"), { ...deps, metodi: metodi(chiamate) })).status).toBe(401);
      expect(chiamate).toHaveLength(0);
    });
  });

  it("se il servizio ui non risponde, 502", async () => {
    const { deps } = setup({ id: "u" });
    deps.fetch = (async () => {
      throw new Error("ECONNREFUSED");
    }) as unknown as typeof fetch;
    const r = await handle(req("/"), deps);
    expect(r.status).toBe(502);
    expect(await r.json()).toEqual({ error: "servizio non raggiungibile" });
  });
});
