import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { handle, MAX_BODY, type Deps } from "@/lib/gateway";

function setup(user: unknown, loghi?: string[]) {
  const chiamate: [string, unknown[]][] = [];
  const deps: Deps = {
    user: async () => user,
    loghi,
    metodi: {
      ha: (nome) => nome === "get_config" || nome === "rompe",
      chiama: async (nome, args) => {
        chiamate.push([nome, args]);
        if (nome === "rompe") throw new Error("guasto di prova");
        return { theme: "dark", args };
      },
    },
  };
  return { chiamate, deps };
}
const req = (path: string, init: RequestInit = {}) => new Request("http://gateway:3000" + path, init);
const post = (path: string, body: string, headers: Record<string, string> = {}) =>
  req(path, { method: "POST", body, headers: { "content-type": "application/json", ...headers } });

describe("senza sessione", () => {
  it("una pagina rimanda all'accesso, con la pagina di partenza", async () => {
    const { deps, chiamate } = setup(null);
    const r = await handle(req("/app.js?v=2"), deps);
    expect(r.status).toBe(302);
    expect(r.headers.get("location")).toBe("/login?next=%2Fapp.js%3Fv%3D2");
    expect(chiamate).toHaveLength(0);
  });
  it("l'API risponde 401 in JSON e non chiama il metodo", async () => {
    const { deps, chiamate } = setup(null);
    for (const r of [await handle(req("/api/get_events"), deps), await handle(post("/api/get_config", "[]"), deps)]) {
      expect(r.status).toBe(401);
      expect(r.headers.get("content-type")).toContain("application/json");
      expect(await r.json()).toEqual({ error: "non autenticato" });
    }
    expect(chiamate).toHaveLength(0);
  });
});

describe("con la sessione", () => {
  it("un POST senza JSON è rifiutato (415), come faceva il servizio ui", async () => {
    const { deps, chiamate } = setup({ id: "u" });
    const r = await handle(req("/api/get_config", { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), deps);
    expect(r.status).toBe(415);
    expect(chiamate).toHaveLength(0);
  });

  it("un POST partito da un altro sito è rifiutato (403)", async () => {
    const { deps, chiamate } = setup({ id: "u" });
    for (const sito of ["cross-site", "same-site"]) {
      expect((await handle(post("/api/get_config", "[]", { "sec-fetch-site": sito }), deps)).status).toBe(403);
    }
    expect((await handle(post("/api/get_config", "[]", { "sec-fetch-site": "none" }), deps)).status).toBe(200);
    expect(chiamate).toHaveLength(1);
  });

  it("un corpo troppo grande è rifiutato (413)", async () => {
    const { deps, chiamate } = setup({ id: "u" });
    expect((await handle(post("/api/get_config", "x".repeat(MAX_BODY + 1)), deps)).status).toBe(413);
    expect(chiamate).toHaveLength(0);
  });

  it("gli altri metodi sono rifiutati (405)", async () => {
    const { deps } = setup({ id: "u" });
    for (const metodo of ["DELETE", "PUT", "PATCH", "OPTIONS"]) {
      expect((await handle(req("/api/get_config", { method: metodo }), deps)).status).toBe(405);
      expect((await handle(req("/logos/acme.png", { method: metodo }), deps)).status).toBe(405);
    }
  });

  describe("metodi serviti dal gateway (lib/core)", () => {
    it("un metodo del gateway risponde da sé, in JSON", async () => {
      const { deps, chiamate } = setup({ id: "u" });
      const r = await handle(post("/api/get_config", '["a", 1]'), deps);
      expect(r.status).toBe(200);
      expect(r.headers.get("content-type")).toBe("application/json; charset=utf-8");
      expect(r.headers.get("cache-control")).toBe("no-store");
      expect(r.headers.get("x-content-type-options")).toBe("nosniff");
      expect(await r.json()).toEqual({ theme: "dark", args: ["a", 1] });
      expect(chiamate).toEqual([["get_config", ["a", 1]]]);
    });

    it("un corpo vuoto è una lista vuota; un corpo che non è una lista, o non è JSON, è 400", async () => {
      const { deps } = setup({ id: "u" });
      expect(await (await handle(post("/api/get_config", ""), deps)).json()).toEqual({ theme: "dark", args: [] });
      for (const corpo of ['{"a": 1}', "5", "non json"]) {
        const r = await handle(post("/api/get_config", corpo), deps);
        expect(r.status).toBe(400);
        expect(typeof (await r.json()).error).toBe("string");
      }
    });

    it("un metodo che solleva è 500 con il suo testo", async () => {
      const { deps } = setup({ id: "u" });
      const r = await handle(post("/api/rompe", "[]"), deps);
      expect(r.status).toBe(500);
      expect(await r.json()).toEqual({ error: "guasto di prova" });
    });

    it("un metodo che il gateway non serve è 404; le regole contro altri siti valgono prima", async () => {
      const { deps, chiamate } = setup({ id: "u" });
      for (const percorso of ["/api/set_account_alias", "/api/win_close", "/api", "/api/a/b", "/logos/acme.png", "/"]) {
        const r = await handle(post(percorso, "[]"), deps);
        expect(r.status).toBe(404);
        expect(r.headers.get("content-type")).toContain("application/json");
        expect(await r.json()).toEqual({ error: "metodo non disponibile" });
      }
      expect((await handle(post("/api/win_close", "[]", { "sec-fetch-site": "cross-site" }), deps)).status).toBe(403);
      expect((await handle(req("/api/win_close", { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), deps)).status).toBe(415);
      expect(chiamate).toHaveLength(0);
    });

    it("un GET non chiama mai un metodo, anche se il nome è quello di uno servito", async () => {
      const { deps, chiamate } = setup({ id: "u" });
      const r = await handle(req("/api/get_config"), deps);
      expect(r.status).toBe(404);
      expect(await r.json()).toEqual({ error: "non trovato" });
      expect(chiamate).toHaveLength(0);
    });

    it("senza sessione resta 401, e il metodo non gira", async () => {
      const { deps, chiamate } = setup(null);
      expect((await handle(post("/api/get_config", "[]"), deps)).status).toBe(401);
      expect(chiamate).toHaveLength(0);
    });
  });

  it("quello che non è un metodo né un logo è 404, in JSON, senza proxy da nessuna parte", async () => {
    const { deps } = setup({ id: "u" });
    for (const percorso of ["/", "/index.html", "/app.js", "/logos", "/logos/", "/api/get_config", "/salute/x"]) {
      for (const metodo of ["GET", "HEAD"]) {
        const r = await handle(req(percorso, { method: metodo }), { ...deps, loghi: [] });
        expect(r.status).toBe(404);
        expect(r.headers.get("content-type")).toContain("application/json");
        if (metodo === "GET") expect(await r.json()).toEqual({ error: "non trovato" });
      }
    }
  });
});

describe("loghi, GET e HEAD /logos/<file>", () => {
  const PNG = Buffer.from(
    "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000001e221bc330000000049454e44ae426082",
    "hex",
  );
  const PREDEFINITO = Buffer.concat([PNG, Buffer.from("predefinito")]);
  const SEGRETO = "fuori dalla cartella dei loghi";
  const TIPI = {
    "a.png": "image/png",
    "a.jpg": "image/jpeg",
    "a.jpeg": "image/jpeg",
    "a.gif": "image/gif",
    "a.webp": "image/webp",
    "a.svg": "image/svg+xml",
    "a.ico": "image/vnd.microsoft.icon",
    "a.PNG": "image/png",
    "a.txt": "application/octet-stream",
    "a.bin": "application/octet-stream",
    senzaestensione: "application/octet-stream",
  };

  let radice: string;
  let dati: string;
  let logos: string;
  let predefiniti: string;
  let deps: Deps;

  const prendi = (percorso: string, metodo = "GET") => handle(req(percorso, { method: metodo }), deps);
  const nonTrovato = async (percorso: string) => {
    const r = await prendi(percorso);
    expect(r.status, percorso).toBe(404);
    expect(r.headers.get("content-type")).toContain("application/json");
    const corpo = await r.text();
    expect(corpo, percorso).toBe('{"error":"non trovato"}');
    expect(corpo).not.toContain("fuori");
  };

  beforeAll(() => {
    radice = fs.mkdtempSync(path.join(os.tmpdir(), "oe-loghi-"));
    dati = path.join(radice, "dati");
    logos = path.join(dati, "logos");
    predefiniti = path.join(radice, "predefiniti");
    fs.mkdirSync(path.join(logos, "sub", "vuota"), { recursive: true });
    fs.mkdirSync(predefiniti);
    // fuori dalle due cartelle dei loghi, dove un nome con .. andrebbe a finire
    fs.writeFileSync(path.join(radice, "segreto.txt"), SEGRETO);
    fs.writeFileSync(path.join(dati, "segreto-dati.txt"), SEGRETO);
    fs.writeFileSync(path.join(logos, "acme.png"), PNG);
    fs.writeFileSync(path.join(logos, "mio logo.png"), PNG);
    fs.writeFileSync(path.join(logos, "sub", "nested.png"), PNG);
    for (const nome of Object.keys(TIPI)) fs.writeFileSync(path.join(logos, nome), PNG);
    fs.writeFileSync(path.join(predefiniti, "acme.png"), PREDEFINITO);
    fs.writeFileSync(path.join(predefiniti, "personale.png"), PREDEFINITO);
    deps = setup({ id: "u" }, [logos, predefiniti]).deps;
  });

  afterAll(() => fs.rmSync(radice, { recursive: true, force: true }));

  it("un logo della cartella dei dati si serve, con tipo, senza cache e senza indovinare il tipo", async () => {
    const r = await prendi("/logos/acme.png");
    expect(r.status).toBe(200);
    expect(r.headers.get("content-type")).toBe("image/png");
    expect(r.headers.get("cache-control")).toBe("no-cache");
    expect(r.headers.get("x-content-type-options")).toBe("nosniff");
    expect(r.headers.get("content-length")).toBe(String(PNG.length));
    expect(Buffer.from(await r.arrayBuffer()).equals(PNG)).toBe(true);
  });

  it("quello che non c'è nella cartella dei dati si prende da quella dell'immagine", async () => {
    const r = await prendi("/logos/personale.png");
    expect(r.status).toBe(200);
    expect(Buffer.from(await r.arrayBuffer()).equals(PREDEFINITO)).toBe(true);
  });

  it("se il file c'è in tutte e due, vince la cartella dei dati", async () => {
    expect(Buffer.from(await (await prendi("/logos/acme.png")).arrayBuffer()).equals(PNG)).toBe(true);
    expect(fs.existsSync(path.join(predefiniti, "acme.png"))).toBe(true);
  });

  it("senza la cartella dei dati (primo avvio) si serve quella dell'immagine; senza nessuna delle due è 404", async () => {
    const solo = setup({ id: "u" }, [path.join(radice, "non-esiste"), predefiniti]).deps;
    expect((await handle(req("/logos/personale.png"), solo)).status).toBe(200);
    expect((await handle(req("/logos/acme.png"), solo)).status).toBe(200);
    const niente = setup({ id: "u" }, [path.join(radice, "non-esiste"), path.join(radice, "neanche")]).deps;
    expect((await handle(req("/logos/personale.png"), niente)).status).toBe(404);
  });

  it("un logo che non c'è è 404 in JSON", async () => {
    await nonTrovato("/logos/nessuno.png");
    await nonTrovato("/logos/acme.png.bak");
  });

  it("la query non conta e il nome si decodifica una volta sola, come faceva il servizio ui", async () => {
    expect((await prendi("/logos/acme.png?v=3&x=%2e%2e")).status).toBe(200);
    expect((await prendi("/logos/ac%6De.png")).status).toBe(200);
    const spazio = await prendi("/logos/mio%20logo.png");
    expect(spazio.status).toBe(200);
    expect(Buffer.from(await spazio.arrayBuffer()).equals(PNG)).toBe(true);
    await nonTrovato("/logos/ac%256De.png");
  });

  it("un file in una sottocartella della cartella dei loghi si serve; una cartella no", async () => {
    expect((await prendi("/logos/sub/nested.png")).status).toBe(200);
    await nonTrovato("/logos/sub");
    await nonTrovato("/logos/sub/");
    await nonTrovato("/logos/sub/vuota");
    await nonTrovato("/logos/");
  });

  it("il tipo viene dall'estensione; per le altre è application/octet-stream", async () => {
    for (const [nome, tipo] of Object.entries(TIPI)) {
      const r = await prendi("/logos/" + nome);
      expect(r.status, nome).toBe(200);
      expect(r.headers.get("content-type"), nome).toBe(tipo);
      expect(r.headers.get("x-content-type-options"), nome).toBe("nosniff");
    }
  });

  it("HEAD porta le intestazioni e la lunghezza, non il corpo; un logo che non c'è è 404 anche così", async () => {
    const r = await prendi("/logos/acme.png", "HEAD");
    expect(r.status).toBe(200);
    expect(r.headers.get("content-type")).toBe("image/png");
    expect(r.headers.get("cache-control")).toBe("no-cache");
    expect(r.headers.get("x-content-type-options")).toBe("nosniff");
    expect(r.headers.get("content-length")).toBe(String(PNG.length));
    expect(await r.text()).toBe("");
    expect((await prendi("/logos/personale.png", "HEAD")).status).toBe(200);
    expect((await prendi("/logos/nessuno.png", "HEAD")).status).toBe(404);
    expect((await prendi("/logos/..%2fsegreto-dati.txt", "HEAD")).status).toBe(404);
  });

  it("senza sessione un logo che c'è non si serve: la pagina rimanda all'accesso", async () => {
    const { deps: senza } = setup(null, [logos, predefiniti]);
    const r = await handle(req("/logos/acme.png"), senza);
    expect(r.status).toBe(302);
    expect(r.headers.get("location")).toBe("/login?next=%2Flogos%2Facme.png");
    expect((await handle(req("/logos/acme.png", { method: "HEAD" }), senza)).status).toBe(302);
  });

  describe("niente fuori dalle cartelle dei loghi", () => {
    const tentativi = () => [
      "/logos/..%2fsegreto-dati.txt",
      "/logos/..%2Fsegreto-dati.txt",
      "/logos/%2e%2e%2fsegreto-dati.txt",
      "/logos/..%2f..%2fsegreto.txt",
      "/logos/..%2f..%2f..%2f..%2f..%2f..%2f..%2f..%2fsegreto.txt",
      "/logos/sub%2f..%2f..%2fsegreto-dati.txt",
      "/logos/sub%2f..%2f..%2f..%2fsegreto.txt",
      "/logos/sub/..%2f..%2fsegreto-dati.txt",
      "/logos/..%5csegreto-dati.txt",
      "/logos/..%2f",
      "/logos/%252e%252e%252fsegreto-dati.txt",
      "/logos/a%00.png",
      "/logos/%E0%A4%A",
      "/logos//segreto.txt",
      "/logos//etc/passwd",
      "/logos/%2fetc%2fpasswd",
      "/logos/" + encodeURIComponent(path.join(radice, "segreto.txt")),
      "/logos/" + encodeURIComponent(path.join(dati, "segreto-dati.txt")),
      "/logos/" + encodeURIComponent(predefiniti + path.sep + ".." + path.sep + "segreto.txt"),
    ];

    it("un nome con .., assoluto o annidato che esce non si serve (404, e il segreto non esce)", async () => {
      for (const percorso of tentativi()) await nonTrovato(percorso);
    });

    it("un .. che resta dentro la cartella è un file come un altro", async () => {
      expect((await prendi("/logos/sub%2f..%2facme.png")).status).toBe(200);
    });

    it("i percorsi che il client ha già normalizzato (.. come segmento) non sono più un logo", async () => {
      for (const percorso of ["/logos/../segreto.txt", "/logos/%2e%2e/segreto-dati.txt", "/logos/sub/../../segreto-dati.txt"]) {
        const r = await prendi(percorso);
        expect(r.status, percorso).toBe(404);
        expect(await r.text()).not.toContain("fuori");
      }
    });

    it("una cartella collegata che porta fuori dalla cartella non si segue", async (ctx) => {
      const esterna = path.join(radice, "cartella-esterna");
      fs.mkdirSync(esterna);
      fs.writeFileSync(path.join(esterna, "segreto.txt"), SEGRETO);
      try {
        // junction su Windows (non chiede privilegi), collegamento simbolico altrove
        fs.symlinkSync(esterna, path.join(logos, "esterno"), "junction");
      } catch (e) {
        return ctx.skip(String(e));
      }
      expect(fs.readFileSync(path.join(logos, "esterno", "segreto.txt"), "utf-8")).toBe(SEGRETO);
      await nonTrovato("/logos/esterno/segreto.txt");
      expect((await prendi("/logos/acme.png")).status).toBe(200);
    });

    it("un collegamento a un file fuori non si segue, uno a un file dentro sì", async (ctx) => {
      try {
        fs.symlinkSync(path.join(dati, "segreto-dati.txt"), path.join(logos, "porta-fuori.png"), "file");
        fs.symlinkSync(path.join(logos, "acme.png"), path.join(logos, "alias.png"), "file");
      } catch (e) {
        return ctx.skip(String(e));
      }
      await nonTrovato("/logos/porta-fuori.png");
      const r = await prendi("/logos/alias.png");
      expect(r.status).toBe(200);
      expect(Buffer.from(await r.arrayBuffer()).equals(PNG)).toBe(true);
    });
  });

  describe("cartelle di norma, senza indicarle", () => {
    const salvati = { dati: process.env.OE_DATI, predefiniti: process.env.LOGHI_PREDEFINITI };
    const ripristina = (nome: string, valore: string | undefined) => {
      if (valore === undefined) delete process.env[nome];
      else process.env[nome] = valore;
    };
    afterAll(() => {
      ripristina("OE_DATI", salvati.dati);
      ripristina("LOGHI_PREDEFINITI", salvati.predefiniti);
    });

    it("logos/ dentro la cartella dei dati (OE_DATI) e poi LOGHI_PREDEFINITI", async () => {
      process.env.OE_DATI = dati;
      process.env.LOGHI_PREDEFINITI = predefiniti;
      const { deps: senzaCartelle } = setup({ id: "u" });
      const acme = await handle(req("/logos/acme.png"), senzaCartelle);
      expect(Buffer.from(await acme.arrayBuffer()).equals(PNG)).toBe(true);
      const personale = await handle(req("/logos/personale.png"), senzaCartelle);
      expect(Buffer.from(await personale.arrayBuffer()).equals(PREDEFINITO)).toBe(true);
      expect((await handle(req("/logos/..%2fsegreto-dati.txt"), senzaCartelle)).status).toBe(404);
    });

    it("senza LOGHI_PREDEFINITI, logos-predefiniti accanto a dove gira il server: nel repository c'è personale.png", async () => {
      process.env.OE_DATI = path.join(radice, "vuota");
      delete process.env.LOGHI_PREDEFINITI;
      const { deps: senzaCartelle } = setup({ id: "u" });
      const r = await handle(req("/logos/personale.png"), senzaCartelle);
      expect(r.status).toBe(200);
      expect(r.headers.get("content-type")).toBe("image/png");
      const icona = fs.readFileSync(path.join(process.cwd(), "logos-predefiniti", "personale.png"));
      expect(Buffer.from(await r.arrayBuffer()).equals(icona)).toBe(true);
    });
  });
});
