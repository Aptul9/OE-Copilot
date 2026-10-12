import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { chiudiArchivio, loadConfig } from "@/lib/core/archivio";
import { attendiScarichi, azzeraScarichi, CODA_MAX, impostaReteDiProva, MSG, POSTI_MAX } from "@/lib/core/loghi";
import { indirizzoAmmesso, type SoloProve } from "@/lib/core/rete-sicura";
import { handle, MAX_BODY, type Deps } from "@/lib/gateway";
import { png, pngIntestazione, SVG_PULITO } from "./immagini-prova";
import { avviaServerProva, risposta, type ServerProva } from "./server-prova";

// I metodi dei loghi dietro il cancello del gateway: sessione, origine, tipo del corpo, grandezza, argomenti; i file serviti con la loro difesa.

const PROVE: SoloProve = {
  risolvi: async (host) => (host.endsWith(".test") ? ["127.0.0.1"] : [host]),
  ammettiIndirizzo: (ip) => ip === "127.0.0.1" || indirizzoAmmesso(ip),
  ammettiPorta: () => true,
  attesaMs: 3000,
};
const METODI_LOGHI = ["get_loghi", "preview_logo", "set_logo", "upload_logo", "refresh_logo", "remove_logo"];
const ROSSO = png(8, [200, 30, 30]);

let dati: string;
let s: ServerProva;

const metodi = { ha: (nome: string) => METODI_TS.has(nome), chiama: (nome: string, args: unknown[]) => chiama(nome as NomeMetodoTs, args) };
const sessione = (): Deps => ({ user: async () => ({ id: "u" }), metodi, loghi: [path.join(dati, "logos")] });
const senzaSessione = (): Deps => ({ user: async () => null, metodi, loghi: [path.join(dati, "logos")] });
const req = (percorso: string, init: RequestInit = {}) => new Request("http://gateway:3000" + percorso, init);
const post = (percorso: string, corpo: unknown, intestazioni: Record<string, string> = {}) =>
  req(percorso, { method: "POST", body: typeof corpo === "string" ? corpo : JSON.stringify(corpo), headers: { "content-type": "application/json", ...intestazioni } });
const chiamata = async (metodo: string, args: unknown[], deps = sessione()) => {
  const r = await handle(post(`/api/${metodo}`, args), deps);
  return { stato: r.status, corpo: await r.json(), intestazioni: r.headers };
};

beforeEach(async () => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-loghi-api-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  for (const m of ["info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  s = await avviaServerProva({
    "/a.png": risposta(ROSSO),
    "/s.svg": risposta(SVG_PULITO, "image/svg+xml"),
    "/pagina": risposta("<html>ciao</html>", "text/html"),
  });
  azzeraScarichi();
});

afterEach(async () => {
  await attendiScarichi();
  impostaReteDiProva(undefined);
  await s.chiudi();
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  delete process.env.OE_DATI;
  vi.restoreAllMocks();
});

describe("i metodi dei loghi sono metodi del gateway", () => {
  it("ci sono tutti", () => {
    for (const m of METODI_LOGHI) expect(METODI_TS.has(m), m).toBe(true);
  });

  describe.each(METODI_LOGHI)("%s", (metodo) => {
    const corpo = metodo === "get_loghi" ? [] : metodo === "set_logo" || metodo === "upload_logo" ? ["Alfa", "x"] : ["Alfa"];

    it("senza sessione è 401 in JSON e non parte niente", async () => {
      impostaReteDiProva(PROVE);
      const r = await handle(post(`/api/${metodo}`, corpo), senzaSessione());
      expect(r.status).toBe(401);
      expect(r.headers.get("content-type")).toContain("application/json");
      expect(await r.json()).toEqual({ error: "non autenticato" });
      expect(s.richieste).toEqual([]);
    });

    it("da un altro sito è 403", async () => {
      for (const sito of ["cross-site", "same-site"]) {
        expect((await handle(post(`/api/${metodo}`, corpo, { "sec-fetch-site": sito }), sessione())).status, sito).toBe(403);
      }
      expect((await handle(post(`/api/${metodo}`, corpo, { "sec-fetch-site": "none" }), sessione())).status).toBe(200);
    });

    it("con un tipo che non è JSON è 415", async () => {
      for (const tipo of ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data; boundary=x", ""]) {
        const r = await handle(req(`/api/${metodo}`, { method: "POST", body: JSON.stringify(corpo), headers: tipo ? { "content-type": tipo } : {} }), sessione());
        expect(r.status, tipo).toBe(415);
      }
    });

    it("con un corpo troppo grande è 413, dichiarato o no", async () => {
      expect((await handle(post(`/api/${metodo}`, "x".repeat(MAX_BODY + 1)), sessione())).status).toBe(413);
      expect((await handle(post(`/api/${metodo}`, "[]", { "content-length": String(MAX_BODY + 1) }), sessione())).status).toBe(413);
    });

    it("con un corpo che non è una lista JSON è 400", async () => {
      for (const c of ['{"nome": "Alfa"}', "5", "non json", '"Alfa"']) expect((await handle(post(`/api/${metodo}`, c), sessione())).status, c).toBe(400);
    });

    it("con GET non si chiama (404)", async () => {
      expect((await handle(req(`/api/${metodo}`), sessione())).status).toBe(404);
    });
  });
});

describe("dalla pagina, passando dal gateway", () => {
  beforeEach(() => impostaReteDiProva(PROVE));

  it("scegliere un logo: la risposta, la mappa, il file servito con la sua difesa, togliere", async () => {
    const prima = await chiamata("get_config", []);
    const r = await chiamata("set_logo", ["Alfa", s.url("/a.png")]);
    expect(r.stato).toBe(200);
    expect(r.intestazioni.get("content-type")).toBe("application/json; charset=utf-8");
    expect(r.intestazioni.get("cache-control")).toBe("no-store");
    expect(r.corpo).toMatchObject({ ok: true, logo: { url: s.url("/a.png"), tipo: "url" } });

    expect((await chiamata("get_loghi", [])).corpo).toEqual({ loghi: { Alfa: r.corpo.logo } });
    const mappa = (await chiamata("get_personalizzazione", [])).corpo.interfaccia.loghi as Record<string, string>;
    expect(mappa.Alfa).toMatch(/^logos\/url-[0-9a-f]{16}\.png\?v=\d+$/);

    // il file lo serve il solito GET /logos/<file>, senza guardare la query
    const file = mappa.Alfa.split("?")[0].replace(/^logos\//, "");
    const get = await handle(req(`/logos/${file}?v=1`), sessione());
    expect(get.status).toBe(200);
    expect(get.headers.get("content-type")).toBe("image/png");
    expect(get.headers.get("content-security-policy")).toBe("default-src 'none'; style-src 'unsafe-inline'; sandbox");
    expect(get.headers.get("x-content-type-options")).toBe("nosniff");
    expect(get.headers.get("cache-control")).toBe("no-cache");
    expect(Buffer.from(await get.arrayBuffer()).equals(ROSSO)).toBe(true);
    const head = await handle(req(`/logos/${file}`, { method: "HEAD" }), sessione());
    expect(head.status).toBe(200);
    expect(head.headers.get("content-security-policy")).toContain("sandbox");
    // senza sessione non si serve
    expect((await handle(req(`/logos/${file}`), senzaSessione())).status).toBe(302);

    expect((await chiamata("remove_logo", ["Alfa"])).corpo).toEqual({ ok: true });
    expect((await chiamata("get_loghi", [])).corpo).toEqual({ loghi: {} });
    expect((await handle(req(`/logos/${file}`), sessione())).status).toBe(404);

    // la configurazione non ha visto niente di tutto questo
    expect((await chiamata("get_config", [])).corpo).toEqual(prima.corpo);
    expect(Object.keys(loadConfig()).some((k) => /logh|logo/i.test(k))).toBe(false);
  });

  it("l'anteprima restituisce l'immagine e non salva", async () => {
    const r = await chiamata("preview_logo", [s.url("/a.png")]);
    expect(r.corpo).toEqual({ ok: true, anteprima: `data:image/png;base64,${ROSSO.toString("base64")}`, tipo: "image/png", byte: ROSSO.length });
    expect((await chiamata("get_loghi", [])).corpo).toEqual({ loghi: {} });
    expect(fs.existsSync(path.join(dati, "logos"))).toBe(false);
  });

  it("rileggere", async () => {
    await chiamata("set_logo", ["Alfa", s.url("/a.png")]);
    const r = await chiamata("refresh_logo", ["Alfa"]);
    expect(r.corpo).toMatchObject({ ok: true, logo: { tipo: "url" } });
    expect((await chiamata("refresh_logo", ["Beta"])).corpo).toEqual({ ok: false, error: "Per questo nome non c'è un logo scelto da qui" });
  });

  it("un file caricato in base64 dentro il corpo", async () => {
    const r = await chiamata("upload_logo", ["Alfa", ROSSO.toString("base64")]);
    expect(r.corpo).toMatchObject({ ok: true, logo: { url: null, tipo: "file" } });
    // 512 KB di immagine stanno nel corpo (700 KB di base64); 600 KB si rifiutano con il loro testo
    const quasi = Buffer.concat([ROSSO, Buffer.alloc(512 * 1024 - ROSSO.length)]);
    expect((await chiamata("upload_logo", ["Beta", quasi.toString("base64")])).corpo).toMatchObject({ ok: true });
    const troppo = Buffer.concat([ROSSO, Buffer.alloc(600 * 1024 - ROSSO.length)]);
    expect((await chiamata("upload_logo", ["Gamma", troppo.toString("base64")])).corpo).toEqual({ ok: false, error: "Immagine troppo grande (massimo 512 KB)" });
    // e oltre il limite del corpo è 413
    const enorme = Buffer.alloc(800 * 1024, 1).toString("base64");
    expect((await handle(post("/api/upload_logo", ["Delta", enorme]), sessione())).status).toBe(413);
  });

  it("i rifiuti previsti sono 200 con ok false e il testo italiano, mai un 500", async () => {
    const casi: [string, unknown[], string][] = [
      ["set_logo", ["Alfa", s.url("/pagina")], "Non è un'immagine"],
      ["set_logo", ["Alfa", s.url("/nessuna")], "L'indirizzo risponde con un errore (404)"],
      ["set_logo", [], "Nome non valido"],
      ["set_logo", [5, s.url("/a.png")], "Nome non valido"],
      ["set_logo", ["", s.url("/a.png")], "Nome non valido"],
      ["set_logo", ["Alfa"], "Indirizzo non valido"],
      ["set_logo", ["Alfa", 5], "Indirizzo non valido"],
      ["set_logo", ["Alfa", { url: "x" }], "Indirizzo non valido"],
      ["set_logo", ["Alfa", ""], "Indirizzo non valido"],
      ["preview_logo", [], "Indirizzo non valido"],
      ["preview_logo", ["javascript:alert(1)"], "Indirizzo non ammesso"],
      ["upload_logo", ["Alfa"], "Non è un'immagine"],
      ["upload_logo", ["Alfa", "$$$"], "Non è un'immagine"],
      ["upload_logo", ["Alfa", [1, 2]], "Non è un'immagine"],
      ["remove_logo", [], "Nome non valido"],
      ["refresh_logo", [], "Nome non valido"],
    ];
    for (const [metodo, args, errore] of casi) {
      const r = await chiamata(metodo, args);
      expect(r.stato, `${metodo} ${JSON.stringify(args)}`).toBe(200);
      expect(r.corpo, `${metodo} ${JSON.stringify(args)}`).toEqual({ ok: false, error: errore });
    }
    expect(console.error).not.toHaveBeenCalled();
  });

  it("un guasto nostro (la cartella non si scrive) dà un testo generico, senza percorsi, e finisce nel log", async () => {
    // al posto della cartella dei loghi c'è un file
    fs.writeFileSync(path.join(dati, "logos"), "x");
    const r = await chiamata("set_logo", ["Alfa", s.url("/a.png")]);
    expect(r.stato).toBe(200);
    expect(r.corpo).toEqual({ ok: false, error: "Non riesco a salvare il logo sul server" });
    expect(JSON.stringify(r.corpo)).not.toContain(dati);
    expect(console.error).toHaveBeenCalled();
    expect((await chiamata("get_loghi", [])).corpo).toEqual({ loghi: {} });
  });
});

describe("la guardia vera dietro il gateway", () => {
  it("un indirizzo della rete interna dà 'Indirizzo non ammesso' e non parte nessuna richiesta", async () => {
    impostaReteDiProva(undefined);
    for (const url of [s.url("/a.png"), "http://127.0.0.1/x.png", "http://localhost/x.png", "http://169.254.169.254/latest/meta-data/", "http://10.0.0.5/x.png", "http://[::1]/x.png", "http://2130706433/x.png", "http://[::ffff:7f00:1]/x.png", "file:///etc/passwd", "http://user:pw@example.org/x.png", "http://example.org:6379/"]) {
      for (const metodo of ["set_logo", "preview_logo"]) {
        const r = await chiamata(metodo, metodo === "set_logo" ? ["Alfa", url] : [url]);
        expect(r.stato, url).toBe(200);
        expect(r.corpo, `${metodo} ${url}`).toEqual({ ok: false, error: "Indirizzo non ammesso" });
      }
    }
    expect(s.richieste).toEqual([]);
    expect((await chiamata("get_loghi", [])).corpo).toEqual({ loghi: {} });
  });
});

describe("tante richieste insieme", () => {
  it(`verso un sito che non risponde: al più ${POSTI_MAX} collegamenti aperti, ${CODA_MAX} richieste in coda, le altre 'Troppe richieste'`, { timeout: 30_000 }, async () => {
    let aperte = 0;
    let massimo = 0;
    const appeso = await avviaServerProva({
      "/appeso": (_, res) => {
        massimo = Math.max(massimo, ++aperte);
        res.on("close", () => aperte--);
      },
    });
    try {
      // il sito non risponde mai: ogni scarico finisce alla scadenza, e il posto passa al successivo
      impostaReteDiProva({ ...PROVE, attesaMs: 300 });
      const fuori = 40;
      const risposte = await Promise.all(Array.from({ length: POSTI_MAX + CODA_MAX + fuori }, (_, i) => chiamata("preview_logo", [`http://sito${i}.test:${appeso.porta}/appeso`])));
      expect(risposte.every((r) => r.stato === 200 && r.corpo.ok === false)).toBe(true);
      expect(risposte.filter((r) => r.corpo.error === MSG.troppeRichieste)).toHaveLength(fuori);
      expect(MSG.troppeRichieste).toBe("Troppe richieste, riprova tra poco");
      expect(risposte.filter((r) => r.corpo.error === "L'indirizzo non risponde")).toHaveLength(POSTI_MAX + CODA_MAX);
      // i socket aperti verso il sito non sono mai stati più dei posti, e quelli rifiutati non si sono aperti
      expect(massimo).toBeLessThanOrEqual(POSTI_MAX);
      expect(massimo).toBeGreaterThan(0);
      expect(appeso.richieste).toHaveLength(POSTI_MAX + CODA_MAX);
      expect(console.error).not.toHaveBeenCalled();
    } finally {
      await appeso.chiudi();
    }
  });

  it("dopo la raffica i posti sono liberi: la richiesta successiva passa", async () => {
    impostaReteDiProva(PROVE);
    expect((await chiamata("preview_logo", [s.url("/a.png")])).corpo).toMatchObject({ ok: true });
  });

  it("anche set_logo e refresh_logo passano dagli stessi posti: con i posti e la coda occupati rispondono subito 'Troppe richieste', e non cambiano niente", { timeout: 30_000 }, async () => {
    const appeso = await avviaServerProva({ "/appeso": () => {} });
    try {
      impostaReteDiProva({ ...PROVE, attesaMs: 600 });
      await chiamata("set_logo", ["Alfa", s.url("/a.png")]);
      const occupanti = Array.from({ length: POSTI_MAX + CODA_MAX }, (_, i) => chiamata("preview_logo", [`http://sito${i}.test:${appeso.porta}/appeso`]));
      await new Promise((ok) => setTimeout(ok, 100));
      const t0 = performance.now();
      expect((await chiamata("refresh_logo", ["Alfa"])).corpo).toEqual({ ok: false, error: MSG.troppeRichieste });
      expect((await chiamata("set_logo", ["Beta", s.url("/a.png")])).corpo).toEqual({ ok: false, error: MSG.troppeRichieste });
      expect(performance.now() - t0).toBeLessThan(400);
      expect(Object.keys((await chiamata("get_loghi", [])).corpo.loghi)).toEqual(["Alfa"]);
      await Promise.all(occupanti);
      // liberi i posti, passano
      expect((await chiamata("refresh_logo", ["Alfa"])).corpo).toMatchObject({ ok: true });
      expect((await chiamata("set_logo", ["Beta", s.url("/a.png")])).corpo).toMatchObject({ ok: true });
    } finally {
      await appeso.chiudi();
    }
  });
});
describe("i file che bloccavano il controllo non fermano il gateway", () => {
  const ESEMPI: [string, string | Buffer, string][] = [
    ["22 commenti vuoti e una x (155 byte)", "<!---->".repeat(22) + "x", "Non è un'immagine"],
    ["un nome lunghissimo dentro <svg>", "<svg>" + "a".repeat(400_000), MSG.svg],
    ["< e centinaia di migliaia di spazi", "<svg><" + " ".repeat(400_000) + "x", MSG.svg],
    ["<!DOCTYPE ripetuto 50000 volte", "<svg>" + "<!DOCTYPE".repeat(50_000), MSG.svg],
    ["attributeName=a ripetuto 20000 volte", "<svg>" + "attributeName=a".repeat(20_000), MSG.svg],
    ["una PNG da 65535 x 65535 pixel", pngIntestazione(65535, 65535), MSG.pixel],
  ];

  it.each(ESEMPI)("%s: upload_logo risponde subito con il suo testo", async (_, contenuto, errore) => {
    const t0 = performance.now();
    const r = await chiamata("upload_logo", ["Alfa", Buffer.from(contenuto).toString("base64")]);
    expect(performance.now() - t0).toBeLessThan(1000);
    expect(r.corpo).toEqual({ ok: false, error: errore });
  });

  it.each(ESEMPI)("%s: preview_logo e set_logo da un indirizzo rispondono subito con il suo testo", async (_, contenuto, errore) => {
    impostaReteDiProva(PROVE);
    const sito = await avviaServerProva({ "/x": risposta(contenuto, "image/svg+xml") });
    try {
      const t0 = performance.now();
      expect((await chiamata("preview_logo", [sito.url("/x")])).corpo).toEqual({ ok: false, error: errore });
      expect((await chiamata("set_logo", ["Alfa", sito.url("/x")])).corpo).toEqual({ ok: false, error: errore });
      expect(performance.now() - t0).toBeLessThan(2000);
      expect((await chiamata("get_loghi", [])).corpo).toEqual({ loghi: {} });
    } finally {
      await sito.chiudi();
    }
  });
});

describe("tutto quello che sta in /logos/ porta la sua difesa", () => {
  it("anche un SVG messo a mano e il logo dell'immagine: nosniff e la politica che non esegue niente", async () => {
    const predefiniti = path.join(dati, "predefiniti");
    fs.mkdirSync(path.join(dati, "logos"), { recursive: true });
    fs.mkdirSync(predefiniti);
    fs.writeFileSync(path.join(dati, "logos", "mano.svg"), '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>');
    fs.writeFileSync(path.join(predefiniti, "personale.png"), ROSSO);
    const deps: Deps = { ...sessione(), loghi: [path.join(dati, "logos"), predefiniti] };
    for (const [percorso, tipo] of [["/logos/mano.svg", "image/svg+xml"], ["/logos/personale.png", "image/png"]]) {
      for (const metodo of ["GET", "HEAD"]) {
        const r = await handle(req(percorso, { method: metodo }), deps);
        expect(r.status, `${metodo} ${percorso}`).toBe(200);
        expect(r.headers.get("content-type")).toBe(tipo);
        expect(r.headers.get("x-content-type-options")).toBe("nosniff");
        expect(r.headers.get("content-security-policy")).toBe("default-src 'none'; style-src 'unsafe-inline'; sandbox");
      }
    }
  });
});
