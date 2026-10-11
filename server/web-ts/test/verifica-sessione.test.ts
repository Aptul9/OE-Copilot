import { describe, expect, it } from "vitest";
import { verificaSessione } from "@/lib/verifica-sessione";

const req = (headers: Record<string, string> = {}) => new Request("http://gateway:3000/api/verifica-sessione", { headers });
const con = { user: async () => ({ id: "u" }) };
const senza = { user: async () => null };

describe("con la sessione", () => {
  it("risponde 204, senza corpo e senza cache", async () => {
    const r = await verificaSessione(req({ cookie: "x=y" }), con);
    expect(r.status).toBe(204);
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(await r.text()).toBe("");
  });
  it("passa anche l'aggiornamento a WebSocket", async () => {
    expect((await verificaSessione(req({ upgrade: "websocket", connection: "Upgrade" }), con)).status).toBe(204);
  });
});

describe("senza la sessione", () => {
  it("una pagina va all'accesso e poi torna dove stava", async () => {
    const pagine: Record<string, string>[] = [{ "sec-fetch-dest": "document" }, { accept: "text/html,application/xhtml+xml" }];
    for (const intestazioni of pagine) {
      const r = await verificaSessione(req({ ...intestazioni, "x-forwarded-uri": "/telefono/?a=1" }), senza);
      expect(r.status).toBe(302);
      expect(r.headers.get("location")).toBe("/login?next=%2Ftelefono%2F%3Fa%3D1");
    }
  });
  it("l'indirizzo di ritorno non esce dal sito", async () => {
    for (const uri of ["//evil.example/telefono/", "https://evil.example/", "/" + String.fromCharCode(92) + "evil.example", "/ok" + String.fromCharCode(9) + "x"]) {
      const r = await verificaSessione(req({ accept: "text/html", "x-forwarded-uri": uri }), senza);
      expect(r.headers.get("location")).toBe("/login?next=%2F");
    }
  });
  it("senza indirizzo originale torna alla radice", async () => {
    expect((await verificaSessione(req({ accept: "text/html" }), senza)).headers.get("location")).toBe("/login?next=%2F");
  });
  it("un WebSocket riceve 401, anche se il browser accetta html", async () => {
    const r = await verificaSessione(req({ upgrade: "websocket", connection: "Upgrade", accept: "text/html" }), senza);
    expect(r.status).toBe(401);
    expect(await r.json()).toEqual({ error: "non autenticato" });
    expect(r.headers.get("cache-control")).toBe("no-store");
  });
  it("il resto (script, immagini, fetch) riceve 401 in JSON", async () => {
    const casi: Record<string, string>[] = [{}, { accept: "*/*" }, { accept: "image/avif,image/webp" }, { "sec-fetch-dest": "script" }];
    for (const intestazioni of casi) {
      const r = await verificaSessione(req(intestazioni), senza);
      expect(r.status).toBe(401);
      expect(r.headers.get("content-type")).toContain("application/json");
    }
  });
});
