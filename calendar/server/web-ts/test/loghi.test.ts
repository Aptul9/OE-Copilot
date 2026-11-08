import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chiama } from "@/lib/core/api";
import { archivio, chiudiArchivio, DEFAULT_CONFIG, loadConfig, modificaConfig } from "@/lib/core/archivio";
import {
  aggiornaLogo,
  anteprimaLogo,
  attendiScarichi,
  azzeraScarichi,
  caricaLogo,
  impostaLogo,
  impostaReteDiProva,
  interfacciaConLoghi,
  leggiVoci,
  LOGHI_MAX,
  migraLoghi,
  MSG,
  POSTI_MAX,
  pulisciLoghi,
  rinominaLogo,
  STATICI_MAX,
  togliLogo,
  TTL_STATICI_MS,
} from "@/lib/core/loghi";
import { personalizzazione } from "@/lib/core/personalizzazione";
import { indirizzoAmmesso, type SoloProve } from "@/lib/core/rete-sicura";
import { mappaLoghi } from "@/lib/oe/logic";
import { gif, png, pngIntestazione, SVG_PULITO } from "./immagini-prova";
import { avviaServerProva, risposta, type ServerProva } from "./server-prova";

// Il logo scelto dalla finestra Calendari e quelli di personalizzazione.json: la tabella, il file in cache, l'unione, il cambio di nome,
// la pulizia. Il server di prova su 127.0.0.1 risponde dove la guardia vera non lascerebbe andare (impostaReteDiProva).

const PROVE: SoloProve = {
  risolvi: async (host) => (host.endsWith(".test") ? ["127.0.0.1"] : [host]),
  ammettiIndirizzo: (ip) => ip === "127.0.0.1" || indirizzoAmmesso(ip),
  ammettiPorta: () => true,
  attesaMs: 3000,
};

const ROSSO = png(8, [200, 30, 30]);
const BLU = png(8, [30, 30, 200]);
const VERDE = png(8, [30, 200, 30]);
const hash16 = (testo: string) => crypto.createHash("sha256").update(testo).digest("hex").slice(0, 16);
const base64 = (b: Buffer) => b.toString("base64");

let dati: string;
let s: ServerProva;
let contatori: Record<string, number>;
let cambia: Buffer;
let lentoMs = 0;
let inVolo = 0;
let massimoInVolo = 0;

const logos = () => path.join(dati, "logos");
const gestiti = () => (fs.existsSync(logos()) ? fs.readdirSync(logos()).sort() : []);
const scriviPersonalizzazione = (p: unknown) => fs.writeFileSync(path.join(dati, "personalizzazione.json"), JSON.stringify(p, null, 2));
const interfaccia = () => interfacciaConLoghi(personalizzazione().interfaccia);
const caricatoDa = (file: string) => fs.readFileSync(path.join(logos(), file));
const conta = (percorso: string) => contatori[percorso] ?? 0;

beforeEach(async () => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-loghi-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  delete process.env.OE_PERSONALIZZAZIONE;
  for (const m of ["info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  contatori = {};
  cambia = ROSSO;
  lentoMs = 0;
  inVolo = 0;
  massimoInVolo = 0;
  const conteggio = (percorso: string, gestore: ReturnType<typeof risposta>): ReturnType<typeof risposta> => (req, res) => {
    contatori[percorso] = (contatori[percorso] ?? 0) + 1;
    gestore(req, res);
  };
  const rotte: Record<string, ReturnType<typeof risposta>> = {
    "/a.png": risposta(ROSSO),
    "/b.png": risposta(BLU),
    "/c.png": risposta(VERDE),
    "/a.gif": risposta(gif(), "image/gif"),
    "/s.svg": risposta(SVG_PULITO, "image/svg+xml"),
    "/ostile.svg": risposta('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>', "image/svg+xml"),
    "/ostile155": risposta("<!---->".repeat(22) + "x", "image/svg+xml"),
    "/gigante.png": risposta(pngIntestazione(65535, 65535)),
    "/pagina": risposta("<!doctype html><title>x</title>", "text/html"),
    "/grande": risposta(Buffer.alloc(600 * 1024, 1)),
    "/e404": risposta("no", "text/plain", 404),
    "/cambia": (_, res) => void res.writeHead(200, { "Content-Type": "image/png", "Content-Length": String(cambia.length) }).end(cambia),
    "/lento.png": (_, res) => {
      inVolo++;
      massimoInVolo = Math.max(massimoInVolo, inVolo);
      setTimeout(() => {
        inVolo--;
        res.writeHead(200, { "Content-Type": "image/png", "Content-Length": String(ROSSO.length) }).end(ROSSO);
      }, lentoMs);
    },
  };
  for (let i = 0; i < 10; i++) rotte[`/c${i}.png`] = risposta(png(8, [i * 20, 40, 200 - i * 20]));
  for (let i = 0; i < 50; i++) rotte[`/m${i}.png`] = risposta(png(8, [i, 7, 7]));
  s = await avviaServerProva(Object.fromEntries(Object.entries(rotte).map(([p, g]) => [p, conteggio(p, g)])));
  impostaReteDiProva(PROVE);
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

describe("la tabella, creata all'avvio e solo aggiunta", () => {
  it("migraLoghi crea la tabella con le sue colonne, e due volte non cambia niente", () => {
    migraLoghi();
    migraLoghi();
    const colonne = archivio().prepare("PRAGMA table_info(loghi)").all() as { name: string; pk: number; notnull: number }[];
    expect(colonne.map((c) => c.name)).toEqual(["nome", "url", "file", "tipo", "aggiornato"]);
    expect(colonne.find((c) => c.name === "nome")?.pk).toBe(1);
    expect(leggiVoci()).toEqual({});
  });

  it("le righe che ci sono restano quando si ripassa dalla migrazione, anche con un'altra connessione", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    const prima = leggiVoci();
    chiudiArchivio();
    migraLoghi();
    expect(leggiVoci()).toEqual(prima);
  });

  it("non tocca la configurazione: get_config e i valori predefiniti restano quelli di prima", async () => {
    const prima = loadConfig();
    expect(prima).toEqual(DEFAULT_CONFIG);
    migraLoghi();
    await impostaLogo("Alfa", s.url("/a.png"));
    expect(loadConfig()).toEqual(DEFAULT_CONFIG);
    expect(await chiama("get_config", [])).toEqual(DEFAULT_CONFIG);
    expect(Object.keys(loadConfig()).some((k) => /logh|logo/i.test(k))).toBe(false);
    // le altre tabelle dello stato non sono cambiate
    const tabelle = (archivio().prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").pluck().all() as string[]).filter((n) => !n.startsWith("sqlite_"));
    expect(tabelle).toEqual(["eventi_locali", "loghi", "stato_config", "stato_meta"]);
  });

  it("il tipo di una riga è url o file, e il nome è unico", () => {
    migraLoghi();
    const db = archivio();
    const inserisci = db.prepare("INSERT INTO loghi(nome, url, file, tipo, aggiornato) VALUES (?, ?, ?, ?, ?)");
    expect(() => inserisci.run("X", null, "f.png", "altro", 1)).toThrow();
    inserisci.run("X", null, "f.png", "file", 1);
    expect(() => inserisci.run("X", null, "g.png", "file", 2)).toThrow();
  });
});

describe("un logo da un indirizzo", () => {
  it("si scarica una volta, sta in logos/ come url-<16 cifre di sha256>.<estensione> e la riga lo ricorda", async () => {
    const url = s.url("/a.png");
    const voce = await impostaLogo("Alfa", url);
    const file = `url-${hash16(url)}.png`;
    expect(file).toMatch(/^url-[0-9a-f]{16}\.png$/);
    expect(gestiti()).toEqual([file]);
    expect(caricatoDa(file).equals(ROSSO)).toBe(true);
    expect(voce).toMatchObject({ url, tipo: "url" });
    expect(voce.aggiornato).toBeGreaterThan(Date.now() - 5000);
    expect(leggiVoci()).toEqual({ Alfa: voce });
    expect(conta("/a.png")).toBe(1);
    // le richieste di dopo (la mappa, la pagina) non scaricano più niente
    interfaccia();
    interfaccia();
    expect(conta("/a.png")).toBe(1);
  });

  it("l'estensione è quella dei byte, non quella dell'indirizzo", async () => {
    await impostaLogo("Alfa", s.url("/a.png?x=1"));
    await impostaLogo("Beta", s.url("/s.svg"));
    await impostaLogo("Gamma", s.url("/a.gif"));
    expect(gestiti().map((f) => path.extname(f)).sort()).toEqual([".gif", ".png", ".svg"]);
    // un indirizzo che finisce per .png ma porta una GIF è una GIF
    await impostaLogo("Delta", s.url("/cambia"));
    cambia = gif();
    await aggiornaLogo("Delta");
    expect(gestiti().map((f) => path.extname(f)).sort()).toEqual([".gif", ".gif", ".png", ".svg"]);
  });

  it("il frammento non conta: lo stesso indirizzo con #altro è lo stesso file", async () => {
    await impostaLogo("Alfa", s.url("/a.png#uno"));
    await impostaLogo("Beta", s.url("/a.png#due"));
    expect(gestiti()).toHaveLength(1);
    expect(leggiVoci().Alfa.url).toBe(leggiVoci().Beta.url);
  });

  it("l'indirizzo si ripulisce dagli spazi e il nome anche", async () => {
    await impostaLogo("  Alfa  ", `  ${s.url("/a.png")}  `);
    expect(Object.keys(leggiVoci())).toEqual(["Alfa"]);
  });

  it("l'anteprima scarica e restituisce l'immagine senza salvare niente", async () => {
    const r = await anteprimaLogo(s.url("/a.png"));
    expect(r).toEqual({ anteprima: `data:image/png;base64,${base64(ROSSO)}`, tipo: "image/png", byte: ROSSO.length });
    expect((await anteprimaLogo(s.url("/s.svg"))).anteprima.startsWith("data:image/svg+xml;base64,")).toBe(true);
    expect(leggiVoci()).toEqual({});
    expect(gestiti()).toEqual([]);
  });

  describe("quello che non va non cambia niente e dice perché", () => {
    const casi: [string, string, string][] = [
      ["un indirizzo che non risponde con un errore", "/e404", "L'indirizzo risponde con un errore (404)"],
      ["una pagina web", "/pagina", "Non è un'immagine"],
      ["uno SVG con uno script", "/ostile.svg", "SVG non ammesso: contiene codice o riferimenti esterni"],
      ["un file troppo grande", "/grande", "Immagine troppo grande (massimo 512 KB)"],
      ["un'immagine di miliardi di pixel in pochi byte", "/gigante.png", "Immagine troppo grande (massimo 4096 pixel per lato e 16 megapixel)"],
      ["un file che mandava il controllo in tempi lunghissimi", "/ostile155", "Non è un'immagine"],
    ];
    it.each(casi)("%s", async (_, percorso, testo) => {
      await impostaLogo("Alfa", s.url("/a.png"));
      const prima = { voci: leggiVoci(), file: gestiti() };
      await expect(impostaLogo("Alfa", s.url(percorso))).rejects.toThrow(testo);
      await expect(anteprimaLogo(s.url(percorso))).rejects.toThrow(testo);
      expect({ voci: leggiVoci(), file: gestiti() }).toEqual(prima);
      expect(caricatoDa(prima.file[0]).equals(ROSSO)).toBe(true);
    });

    it("un indirizzo non ammesso (rete interna, schema, credenziali, porta) con le regole vere", async () => {
      impostaReteDiProva(undefined);
      for (const url of ["http://127.0.0.1/x.png", s.url("/a.png"), "http://10.0.0.5/x.png", "http://169.254.169.254/latest/meta-data/", "ftp://example.org/x.png", "https://u:p@example.org/x.png", "http://example.org:8080/x.png", "http://localhost/x.png", "file:///etc/passwd"]) {
        await expect(impostaLogo("Alfa", url), url).rejects.toThrow("Indirizzo non ammesso");
        await expect(anteprimaLogo(url), url).rejects.toThrow("Indirizzo non ammesso");
      }
      await expect(impostaLogo("Alfa", "non un indirizzo")).rejects.toThrow("Indirizzo non valido");
      expect(conta("/a.png")).toBe(0);
      expect(leggiVoci()).toEqual({});
      expect(gestiti()).toEqual([]);
    });

    it("un nome che non va", async () => {
      for (const nome of ["", "   ", "x".repeat(201), "a\u0000b", "a\nb", null, undefined, 5, {}]) {
        await expect(impostaLogo(nome, s.url("/a.png")), String(nome)).rejects.toThrow(MSG.nomeNonValido);
        expect(() => caricaLogo(nome, base64(ROSSO)), String(nome)).toThrow(MSG.nomeNonValido);
        await expect(aggiornaLogo(nome), String(nome)).rejects.toThrow(MSG.nomeNonValido);
        expect(() => togliLogo(nome), String(nome)).toThrow(MSG.nomeNonValido);
      }
      expect(conta("/a.png")).toBe(0);
      expect(leggiVoci()).toEqual({});
    });
  });

  it("senza il punto di prova valgono solo le regole vere, e in produzione il punto di prova non si imposta", () => {
    vi.stubEnv("NODE_ENV", "production");
    try {
      expect(() => impostaReteDiProva(PROVE)).toThrow("solo per le prove");
    } finally {
      vi.unstubAllEnvs();
    }
  });
});

describe("sostituire e togliere", () => {
  it("un altro indirizzo prende il posto del primo e il file di prima sparisce", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    const prima = gestiti();
    const v = await impostaLogo("Alfa", s.url("/b.png"));
    expect(gestiti()).toHaveLength(1);
    expect(gestiti()).not.toEqual(prima);
    expect(caricatoDa(gestiti()[0]).equals(BLU)).toBe(true);
    expect(leggiVoci()).toEqual({ Alfa: v });
  });

  it("lo stesso indirizzo scelto di nuovo non lascia niente in più", async () => {
    for (let i = 0; i < 5; i++) await impostaLogo("Alfa", s.url("/a.png"));
    expect(gestiti()).toHaveLength(1);
    expect(Object.keys(leggiVoci())).toEqual(["Alfa"]);
  });

  it("togliere il logo toglie la riga e il file, e un nome senza logo non è un errore", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    togliLogo("Alfa");
    expect(leggiVoci()).toEqual({});
    expect(gestiti()).toEqual([]);
    expect(() => togliLogo("Alfa")).not.toThrow();
    expect(() => togliLogo("Mai avuto")).not.toThrow();
  });

  it("due nomi con lo stesso indirizzo condividono il file, che resta finché lo usa qualcuno", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    await impostaLogo("Beta", s.url("/a.png"));
    expect(gestiti()).toHaveLength(1);
    togliLogo("Alfa");
    expect(gestiti()).toHaveLength(1);
    expect(caricatoDa(gestiti()[0]).equals(ROSSO)).toBe(true);
    togliLogo("Beta");
    expect(gestiti()).toEqual([]);
  });

  it("senza la scelta torna quello di personalizzazione.json", async () => {
    scriviPersonalizzazione({ interfaccia: { loghi: { Alfa: "logos/alfa.png", Beta: "logos/beta.png" } } });
    expect(interfaccia().loghi).toEqual({ Alfa: "logos/alfa.png", Beta: "logos/beta.png" });
    const v = await impostaLogo("Alfa", s.url("/a.png"));
    const loghi = interfaccia().loghi as Record<string, string>;
    expect(loghi.Alfa).toBe(`logos/${gestiti()[0]}?v=${v.aggiornato}`);
    expect(loghi.Beta).toBe("logos/beta.png");
    togliLogo("Alfa");
    expect(interfaccia().loghi).toEqual({ Alfa: "logos/alfa.png", Beta: "logos/beta.png" });
  });
});

describe("l'unione con personalizzazione.json", () => {
  it("senza loghi scritti e senza scelte è l'oggetto di prima, uguale", () => {
    const base = { sigle: { Alfa: "AL" }, calendari_secondari: ["Beta"] };
    scriviPersonalizzazione({ interfaccia: base });
    expect(interfaccia()).toEqual(base);
    scriviPersonalizzazione({ interfaccia: { ...base, loghi: { Alfa: "logos/alfa.png" } } });
    expect(interfaccia()).toEqual({ ...base, loghi: { Alfa: "logos/alfa.png" } });
    scriviPersonalizzazione({});
    expect(interfaccia()).toEqual({});
  });

  it("i valori che sono file, o non sono testo, passano come sono", () => {
    scriviPersonalizzazione({ interfaccia: { loghi: { A: "logos/a.png", B: "/logos/b.png", C: "b.png", D: 5, E: null, F: "" } } });
    expect(interfaccia().loghi).toEqual({ A: "logos/a.png", B: "/logos/b.png", C: "b.png", D: 5, E: null, F: "" });
    expect(interfaccia().loghi_in_corso).toBeUndefined();
  });

  it("senza loghi scritti e con una scelta c'è solo la scelta; con una scelta per ogni nome vincono le scelte, in ordine di nome", async () => {
    scriviPersonalizzazione({ interfaccia: { sigle: { Alfa: "AL" } } });
    const a = await impostaLogo("Zeta", s.url("/a.png"));
    const b = await impostaLogo("Alfa", s.url("/b.png"));
    const fuori = interfaccia();
    expect(fuori.sigle).toEqual({ Alfa: "AL" });
    expect(Object.entries(fuori.loghi as object).sort()).toEqual(
      [
        ["Alfa", `logos/url-${hash16(s.url("/b.png"))}.png?v=${b.aggiornato}`],
        ["Zeta", `logos/url-${hash16(s.url("/a.png"))}.png?v=${a.aggiornato}`],
      ].sort(),
    );
  });

  it("una scelta ricaricata (aggiornata) cambia il testo del valore: la pagina rilegge l'immagine", async () => {
    const prima = await impostaLogo("Alfa", s.url("/cambia"));
    await new Promise((ok) => setTimeout(ok, 5));
    cambia = BLU;
    const dopo = await aggiornaLogo("Alfa");
    expect(dopo.aggiornato).toBeGreaterThan(prima.aggiornato);
    expect((interfaccia().loghi as Record<string, string>).Alfa).toContain(`?v=${dopo.aggiornato}`);
    expect(caricatoDa(gestiti()[0]).equals(BLU)).toBe(true);
  });

  it("l'oggetto della personalizzazione non si cambia", async () => {
    scriviPersonalizzazione({ interfaccia: { loghi: { Alfa: "logos/alfa.png" } } });
    const originale = personalizzazione().interfaccia;
    const copia = JSON.parse(JSON.stringify(originale));
    await impostaLogo("Alfa", s.url("/a.png"));
    interfaccia();
    expect(originale).toEqual(copia);
  });
});

describe("rileggere l'indirizzo", () => {
  it("rilegge, cambia il file e la data; se il tipo cambia il file di prima sparisce", async () => {
    cambia = ROSSO;
    const prima = await impostaLogo("Alfa", s.url("/cambia"));
    const fileDiPrima = gestiti();
    expect(fileDiPrima).toHaveLength(1);
    await new Promise((ok) => setTimeout(ok, 5));
    cambia = BLU;
    const dopo = await aggiornaLogo("Alfa");
    expect(dopo.aggiornato).toBeGreaterThan(prima.aggiornato);
    expect(gestiti()).toEqual(fileDiPrima);
    expect(caricatoDa(gestiti()[0]).equals(BLU)).toBe(true);
    expect(conta("/cambia")).toBe(2);
    // ora l'indirizzo restituisce una GIF: il file è un altro, quello di prima non resta
    cambia = gif();
    await aggiornaLogo("Alfa");
    expect(gestiti()).toHaveLength(1);
    expect(path.extname(gestiti()[0])).toBe(".gif");
  });

  it("se la rilettura non riesce resta il file di prima", async () => {
    cambia = ROSSO;
    await impostaLogo("Alfa", s.url("/cambia"));
    cambia = Buffer.from("<html>non più un'immagine</html>");
    await expect(aggiornaLogo("Alfa")).rejects.toThrow("Non è un'immagine");
    expect(caricatoDa(gestiti()[0]).equals(ROSSO)).toBe(true);
  });

  it("un nome senza logo scelto, o con un file caricato, non si rilegge", async () => {
    await expect(aggiornaLogo("Nessuno")).rejects.toThrow(MSG.nonCe);
    caricaLogo("Caricato", base64(ROSSO));
    await expect(aggiornaLogo("Caricato")).rejects.toThrow(MSG.daFile);
  });

  it("un logo cambiato o tolto mentre la rilettura aspetta la risposta non si sovrascrive", async () => {
    lentoMs = 150;
    await impostaLogo("Alfa", s.url("/lento.png"));
    lentoMs = 150;
    const lenta = aggiornaLogo("Alfa");
    // nel frattempo un'altra richiesta cambia l'indirizzo
    lentoMs = 0;
    await impostaLogo("Alfa", s.url("/b.png"));
    await expect(lenta).rejects.toThrow(MSG.cambiato);
    expect(leggiVoci().Alfa.url).toBe(s.url("/b.png"));
    expect(caricatoDa(gestiti()[0]).equals(BLU)).toBe(true);
    // e se nel frattempo è stato tolto
    lentoMs = 150;
    await impostaLogo("Beta", s.url("/lento.png"));
    const altra = aggiornaLogo("Beta");
    togliLogo("Beta");
    await expect(altra).rejects.toThrow(MSG.cambiato);
    expect(leggiVoci().Beta).toBeUndefined();
  });
});

describe("un file caricato", () => {
  it("passa dalle stesse regole: il file è up-<16 cifre>.<estensione>, la riga non ha un indirizzo", async () => {
    const v = caricaLogo("Alfa", base64(ROSSO));
    expect(v).toMatchObject({ url: null, tipo: "file" });
    expect(gestiti()).toHaveLength(1);
    expect(gestiti()[0]).toMatch(/^up-[0-9a-f]{16}\.png$/);
    expect(caricatoDa(gestiti()[0]).equals(ROSSO)).toBe(true);
    expect(leggiVoci()).toEqual({ Alfa: v });
  });

  it("rifiuta quello che non è base64, che non è un'immagine, che è troppo grande o ha uno script", () => {
    expect(() => caricaLogo("Alfa", "questo non è base64!")).toThrow(MSG.nonImmagine);
    expect(() => caricaLogo("Alfa", "")).toThrow(MSG.nonImmagine);
    expect(() => caricaLogo("Alfa", 5)).toThrow(MSG.nonImmagine);
    expect(() => caricaLogo("Alfa", base64(Buffer.from("<html>ciao</html>")))).toThrow(MSG.nonImmagine);
    expect(() => caricaLogo("Alfa", base64(Buffer.alloc(600 * 1024, 1)))).toThrow("Immagine troppo grande (massimo 512 KB)");
    expect(() => caricaLogo("Alfa", base64(Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>')))).toThrow(MSG.svg);
    expect(leggiVoci()).toEqual({});
    expect(gestiti()).toEqual([]);
  });

  it("sostituisce un logo da indirizzo e il file di prima sparisce", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    caricaLogo("Alfa", base64(BLU));
    expect(gestiti()).toHaveLength(1);
    expect(gestiti()[0]).toMatch(/^up-/);
  });
});

describe("il cambio di nome", () => {
  const nomi = { nomi_per_dominio: { "alfa.example": "Alfa", "beta.example": "Beta" } };
  beforeEach(() => scriviPersonalizzazione(nomi));

  it("set_account_alias sposta il logo sul nuovo nome, e il file resta lo stesso", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    const file = gestiti();
    expect(await chiama("set_account_alias", ["mario@alfa.example", "Alfa Italia"])).toEqual({ ok: true });
    expect(Object.keys(leggiVoci())).toEqual(["Alfa Italia"]);
    expect(gestiti()).toEqual(file);
    expect((interfaccia().loghi as Record<string, string>)["Alfa Italia"]).toContain(file[0]);
    expect((interfaccia().loghi as Record<string, string>).Alfa).toBeUndefined();
  });

  it("segue anche i gruppi, come prima", async () => {
    await chiama("save_config", [{ gruppi_calendari: { g: ["Alfa", "Beta"] } }]);
    await impostaLogo("Alfa", s.url("/a.png"));
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa Italia"]);
    expect(loadConfig().gruppi_calendari).toEqual({ g: ["Alfa Italia", "Beta"] });
    expect(Object.keys(leggiVoci())).toEqual(["Alfa Italia"]);
  });

  it("segue anche la scelta di secondario; se il nome nuovo ne ha già una vale quella", async () => {
    await chiama("save_config", [{ secondari_scelti: { Alfa: true, Beta: false } }]);
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa Italia"]);
    expect(loadConfig().secondari_scelti).toEqual({ "Alfa Italia": true, Beta: false });
    await chiama("set_account_alias", ["luigi@beta.example", "Alfa Italia"]);
    expect(loadConfig().secondari_scelti).toEqual({ "Alfa Italia": true });
  });

  it("un nome vuoto riporta quello automatico, e il logo lo segue", async () => {
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa Italia"]);
    await impostaLogo("Alfa Italia", s.url("/a.png"));
    await chiama("set_account_alias", ["mario@alfa.example", ""]);
    expect(Object.keys(leggiVoci())).toEqual(["Alfa"]);
  });

  it("se il nome nuovo ha già il suo logo resta quello, e il file dell'altro si toglie", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    await impostaLogo("Alfa Italia", s.url("/b.png"));
    expect(gestiti()).toHaveLength(2);
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa Italia"]);
    expect(Object.keys(leggiVoci())).toEqual(["Alfa Italia"]);
    expect(leggiVoci()["Alfa Italia"].url).toBe(s.url("/b.png"));
    expect(gestiti()).toHaveLength(1);
    expect(caricatoDa(gestiti()[0]).equals(BLU)).toBe(true);
  });

  it("senza un logo non cambia niente: la tabella resta vuota, non nasce la cartella", async () => {
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa Italia"]);
    expect(leggiVoci()).toEqual({});
    expect(fs.existsSync(logos())).toBe(false);
  });

  it("lo stesso nome (o la stessa chiamata due volte) non cambia niente; gli altri nomi restano", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    await impostaLogo("Beta", s.url("/b.png"));
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa"]);
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa X"]);
    await chiama("set_account_alias", ["mario@alfa.example", "Alfa X"]);
    expect(Object.keys(leggiVoci()).sort()).toEqual(["Alfa X", "Beta"]);
  });

  it("rinominaLogo da solo: la riga cambia nome, ma il file lo toglie chi pulisce dopo", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    modificaConfig(() => rinominaLogo("Alfa", "Gamma"));
    expect(Object.keys(leggiVoci())).toEqual(["Gamma"]);
    expect(() => rinominaLogo("Gamma", "Gamma")).not.toThrow();
    expect(() => rinominaLogo("Mai", "Altro")).not.toThrow();
  });

  it("se la modifica della configurazione non riesce, il logo non cambia nome", async () => {
    await impostaLogo("Alfa", s.url("/a.png"));
    expect(() =>
      modificaConfig(() => {
        rinominaLogo("Alfa", "Gamma");
        throw new Error("no");
      }),
    ).toThrow("no");
    expect(Object.keys(leggiVoci())).toEqual(["Alfa"]);
  });
});

describe("la cartella non cresce", () => {
  it("la pulizia toglie i file gestiti che nessuno usa e gli avanzi di una scrittura, e non tocca gli altri", () => {
    migraLoghi();
    fs.mkdirSync(logos(), { recursive: true });
    const scrivi = (nome: string) => fs.writeFileSync(path.join(logos(), nome), "x");
    // messi a mano da chi usa l'app, o dell'immagine: mai toccati
    for (const nome of ["alfa.png", "personale.png", "url-xyz.png", "url-0123456789abcde.png", "url-0123456789abcdef.png.bak", "up-0123456789abcdef.txt", "Url-0123456789abcdef.png", "x.svg"]) scrivi(nome);
    // gestiti ma senza nessuno che li usi
    for (const nome of ["url-0123456789abcdef.png", "up-fedcba9876543210.jpg", "url-aaaaaaaaaaaaaaaa.svg", ".logo-tmp-1-ab", ".logo-tmp-99-cd"]) scrivi(nome);
    pulisciLoghi();
    expect(gestiti()).toEqual(["Url-0123456789abcdef.png", "alfa.png", "personale.png", "up-0123456789abcdef.txt", "url-0123456789abcde.png", "url-0123456789abcdef.png.bak", "url-xyz.png", "x.svg"].sort());
  });

  it("una cartella che non c'è non è un problema e non si crea", () => {
    expect(() => pulisciLoghi()).not.toThrow();
    expect(fs.existsSync(logos())).toBe(false);
  });

  it("cambiare logo molte volte lascia sempre un file solo", async () => {
    for (let i = 0; i < 30; i++) {
      await impostaLogo("Alfa", s.url(`/m${i}.png`));
      expect(gestiti()).toHaveLength(1);
    }
    for (let i = 0; i < 5; i++) {
      caricaLogo("Alfa", base64(png(8, [i, 1, 1])));
      expect(gestiti()).toHaveLength(1);
    }
  });

  it(`al più ${LOGHI_MAX} scelte: la successiva si rifiuta e non lascia un file, sostituire o togliere restano possibili`, async () => {
    for (let i = 0; i < LOGHI_MAX; i++) caricaLogo(`Azienda ${i}`, base64(png(8, [i, 2, 2])));
    expect(Object.keys(leggiVoci())).toHaveLength(LOGHI_MAX);
    expect(gestiti()).toHaveLength(LOGHI_MAX);
    expect(() => caricaLogo("Una di troppo", base64(png(8, [99, 9, 9])))).toThrow(MSG.troppi);
    await expect(impostaLogo("Una di troppo", s.url("/a.png"))).rejects.toThrow(MSG.troppi);
    expect(gestiti()).toHaveLength(LOGHI_MAX);
    caricaLogo("Azienda 3", base64(png(8, [200, 3, 3])));
    expect(gestiti()).toHaveLength(LOGHI_MAX);
    togliLogo("Azienda 0");
    caricaLogo("Una di troppo", base64(png(8, [99, 9, 9])));
    expect(gestiti()).toHaveLength(LOGHI_MAX);
  });

  it("i file gestiti sono al più LOGHI_MAX più quelli di personalizzazione.json (STATICI_MAX)", async () => {
    const statici = Object.fromEntries(Array.from({ length: STATICI_MAX + 5 }, (_, i) => [`Voce ${i}`, `http://stat${i}.test:${s.porta}/m${i % 50}.png`]));
    scriviPersonalizzazione({ interfaccia: { loghi: statici } });
    for (let giro = 0; giro < 30; giro++) {
      interfaccia();
      await attendiScarichi();
    }
    // quelli oltre il tetto non si scaricano mai
    expect(gestiti().length).toBeLessThanOrEqual(STATICI_MAX);
    expect(Object.keys(interfaccia().loghi as object).length).toBeLessThanOrEqual(STATICI_MAX);
    expect(LOGHI_MAX + STATICI_MAX).toBe(100);
  });
});

describe("i salvataggi insieme", () => {
  it("venti nomi con cinque indirizzi, tutti insieme: venti righe, cinque file, niente di avanzo", async () => {
    const nomi = Array.from({ length: 20 }, (_, i) => `Azienda ${i}`);
    await Promise.all(nomi.map((n, i) => impostaLogo(n, s.url(`/c${i % 5}.png`))));
    expect(Object.keys(leggiVoci()).sort()).toEqual([...nomi].sort());
    expect(gestiti()).toHaveLength(5);
    for (const [nome, voce] of Object.entries(leggiVoci())) {
      const i = Number(nome.split(" ")[1]);
      expect(voce.url).toBe(s.url(`/c${i % 5}.png`));
    }
    for (const f of gestiti()) expect(fs.statSync(path.join(logos(), f)).size).toBeGreaterThan(0);
    expect(gestiti().some((f) => f.startsWith(".logo-tmp-"))).toBe(false);
  });

  it("dieci indirizzi diversi per lo stesso nome insieme: una riga sola, con il suo file, e nessun avanzo", async () => {
    const esiti = await Promise.allSettled(Array.from({ length: 10 }, (_, i) => impostaLogo("Alfa", s.url(`/c${i}.png`))));
    expect(esiti.every((e) => e.status === "fulfilled")).toBe(true);
    const voci = leggiVoci();
    expect(Object.keys(voci)).toEqual(["Alfa"]);
    expect(gestiti()).toHaveLength(1);
    const i = Number(/\/c(\d)\.png$/.exec(voci.Alfa.url as string)?.[1]);
    expect(caricatoDa(gestiti()[0]).equals(png(8, [i * 20, 40, 200 - i * 20]))).toBe(true);
    expect(gestiti()[0]).toBe(`url-${hash16(voci.Alfa.url as string)}.png`);
  });

  it("scegliere, rileggere e togliere insieme lascia uno stato coerente: la riga ha il suo file, o non c'è niente", async () => {
    for (let giro = 0; giro < 8; giro++) {
      await Promise.allSettled([
        impostaLogo("Alfa", s.url("/a.png")),
        impostaLogo("Alfa", s.url("/b.png")),
        aggiornaLogo("Alfa"),
        Promise.resolve().then(() => togliLogo("Alfa")),
        impostaLogo("Beta", s.url("/b.png")),
        Promise.resolve().then(() => caricaLogo("Alfa", base64(VERDE))),
      ]);
      const righe = Object.keys(leggiVoci());
      const files = gestiti();
      // ogni file è usato da una riga, ogni riga ha il suo file
      const file = (nome: string) => {
        const v = leggiVoci()[nome];
        return v.tipo === "file" ? files.find((f) => f.startsWith("up-")) : `url-${hash16(v.url as string)}.png`;
      };
      for (const r of righe) expect(files, r).toContain(file(r));
      expect(files.length).toBeLessThanOrEqual(righe.length);
      expect(files.every((f) => !f.startsWith(".logo-tmp-"))).toBe(true);
      for (const r of righe) togliLogo(r);
      expect(gestiti()).toEqual([]);
    }
  });

  it("cambiare nome e scegliere il logo insieme non perde la riga", async () => {
    scriviPersonalizzazione({ nomi_per_dominio: { "alfa.example": "Alfa" } });
    await impostaLogo("Alfa", s.url("/a.png"));
    await Promise.all([chiama("set_account_alias", ["mario@alfa.example", "Alfa Due"]), impostaLogo("Beta", s.url("/b.png")), impostaLogo("Gamma", s.url("/c.png"))]);
    expect(Object.keys(leggiVoci()).sort()).toEqual(["Alfa Due", "Beta", "Gamma"]);
    expect(gestiti()).toHaveLength(3);
  });
});

describe("gli indirizzi scritti a mano in personalizzazione.json", () => {
  const scrivi = (loghi: Record<string, unknown>) => scriviPersonalizzazione({ interfaccia: { loghi } });

  it("la prima volta il logo manca (le iniziali) e il nome è in corso; scaricato, compare, e non si riscarica", async () => {
    const url = `http://logo.prova.test:${s.porta}/a.png`;
    scrivi({ Alfa: url, Beta: "logos/beta.png" });
    const primo = interfaccia();
    expect(primo.loghi).toEqual({ Beta: "logos/beta.png" });
    expect(primo.loghi_in_corso).toEqual(["Alfa"]);
    await attendiScarichi();
    const secondo = interfaccia();
    const file = `url-${hash16(new URL(url).href)}.png`;
    expect(secondo.loghi).toEqual({ Alfa: expect.stringMatching(new RegExp(`^logos/${file}\\?v=\\d+$`)), Beta: "logos/beta.png" });
    expect(secondo.loghi_in_corso).toBeUndefined();
    expect(caricatoDa(file).equals(ROSSO)).toBe(true);
    interfaccia();
    interfaccia();
    await attendiScarichi();
    expect(conta("/a.png")).toBe(1);
  });

  it("un indirizzo senza lo schema (//sito/file) è https; lo stesso indirizzo di due nomi è un file solo", async () => {
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/a.png`, Beta: `http://u.prova.test:${s.porta}/a.png`, Gamma: "//sito.prova.test/x.png" });
    interfaccia();
    await attendiScarichi();
    const loghi = interfaccia().loghi as Record<string, string>;
    expect(loghi.Alfa).toBe(loghi.Beta);
    expect(conta("/a.png")).toBe(1);
    expect(gestiti()).toHaveLength(1);
  });

  it("se lo scarico non riesce la voce manca e si riprova solo dopo 5 minuti", async () => {
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/e404` });
    expect(interfaccia().loghi_in_corso).toEqual(["Alfa"]);
    await attendiScarichi();
    expect(conta("/e404")).toBe(1);
    const dopo = interfaccia();
    expect(dopo.loghi).toEqual({});
    expect(dopo.loghi_in_corso).toEqual(["Alfa"]);
    await attendiScarichi();
    expect(conta("/e404")).toBe(1);
    azzeraScarichi();
    interfaccia();
    await attendiScarichi();
    expect(conta("/e404")).toBe(2);
    expect(gestiti()).toEqual([]);
  });

  it("nel log c'è il sito e il motivo, mai l'indirizzo intero con il suo token", async () => {
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/e404?token=SEGRETO123` });
    interfaccia();
    await attendiScarichi();
    const righe = vi.mocked(console.warn).mock.calls.map((c) => c.map(String).join(" "));
    expect(righe.some((r) => r.includes("loghi:") && r.includes("L'indirizzo risponde con un errore (404)"))).toBe(true);
    expect(righe.join("\n")).not.toContain("SEGRETO123");
    expect(righe.join("\n")).not.toContain("/e404");
  });

  it("un file di 7 giorni fa si rinnova in secondo piano e intanto si mostra quello che c'è; se il rinnovo non riesce resta quello", async () => {
    const url = `http://u.prova.test:${s.porta}/cambia`;
    scrivi({ Alfa: url });
    interfaccia();
    await attendiScarichi();
    const file = gestiti()[0];
    expect(conta("/cambia")).toBe(1);
    const vecchio = (Date.now() - TTL_STATICI_MS - 60_000) / 1000;
    fs.utimesSync(path.join(logos(), file), vecchio, vecchio);
    cambia = BLU;
    const durante = interfaccia();
    // intanto il logo di prima è ancora lì
    expect((durante.loghi as Record<string, string>).Alfa).toContain(file);
    expect(durante.loghi_in_corso).toBeUndefined();
    await attendiScarichi();
    expect(conta("/cambia")).toBe(2);
    expect(caricatoDa(file).equals(BLU)).toBe(true);
    expect(fs.statSync(path.join(logos(), file)).mtimeMs).toBeGreaterThan(Date.now() - 10_000);
    // fresco: non si riscarica
    interfaccia();
    await attendiScarichi();
    expect(conta("/cambia")).toBe(2);
    // di nuovo vecchio, ma il sito ora risponde male: il file resta
    fs.utimesSync(path.join(logos(), file), vecchio, vecchio);
    cambia = Buffer.from("non più un'immagine");
    interfaccia();
    await attendiScarichi();
    expect(caricatoDa(file).equals(BLU)).toBe(true);
    expect((interfaccia().loghi as Record<string, string>).Alfa).toContain(file);
  });

  it("un indirizzo che la guardia vera non ammette non si scarica e non rompe niente", async () => {
    impostaReteDiProva(undefined);
    scrivi({ Alfa: s.url("/a.png"), Beta: "http://10.0.0.5/x.png", Gamma: "http://169.254.169.254/latest/", Delta: "ftp://example.org/x.png", Epsilon: "https://u:p@example.org/x.png", Zeta: "http://[::1]/x" });
    const fuori = interfaccia();
    expect(fuori.loghi).toEqual({ Delta: "ftp://example.org/x.png" });
    await attendiScarichi();
    expect(conta("/a.png")).toBe(0);
    expect(gestiti()).toEqual([]);
  });

  it("un nome che ha una scelta dalla finestra non si scarica: vince la scelta", async () => {
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/a.png`, Beta: `http://u.prova.test:${s.porta}/b.png` });
    await impostaLogo("Alfa", s.url("/c.png"));
    interfaccia();
    await attendiScarichi();
    expect(conta("/a.png")).toBe(0);
    expect(conta("/b.png")).toBe(1);
    expect((interfaccia().loghi as Record<string, string>).Alfa).toContain(`url-${hash16(s.url("/c.png"))}`);
    // tolta la scelta, si scarica quello scritto a mano
    togliLogo("Alfa");
    interfaccia();
    await attendiScarichi();
    expect(conta("/a.png")).toBe(1);
  });

  it("un valore con la barra rovesciata, con un tab o con un a capo è un indirizzo come gli altri: lo scarica il gateway e alla pagina non arriva mai il valore", async () => {
    const B = "\\";
    const porta = s.porta;
    const stessoDi = `http://u.prova.test:${porta}/a.png`;
    scrivi({
      Alfa: stessoDi,
      Beta: `http:${B}${B}u.prova.test:${porta}${B}a.png`,
      Gamma: `  http:/\t/u.prova.test:${porta}/a.png `,
      Delta: `/${B}u.prova.test:${porta}/b.png`,
      Epsilon: `${B}${B}u.prova.test:${porta}${B}c.png`,
      Zeta: `/\t/u.prova.test:${porta}/d.png`,
      Eta: `/\n/u.prova.test:${porta}/e.png`,
      Theta: `${B}/u.prova.test:${porta}/f.png`,
      Iota: "logos/iota.png",
    });
    const primo = interfaccia();
    // nessun valore di questi arriva alla pagina così com'è: o è il file in cache, o manca (e il nome è in corso)
    expect(Object.values(primo.loghi as object)).toEqual(["logos/iota.png"]);
    expect(primo.loghi_in_corso).toEqual(["Alfa", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta"]);
    for (let giro = 0; giro < 6; giro++) {
      await attendiScarichi();
      interfaccia();
    }
    await attendiScarichi();
    const fuori = interfaccia();
    const loghi = fuori.loghi as Record<string, string>;
    // i tre che portano a http://u.prova.test:PORTA/a.png sono lo stesso file, scaricato una volta sola
    expect(conta("/a.png")).toBe(1);
    expect(loghi.Alfa).toBe(loghi.Beta);
    expect(loghi.Alfa).toBe(loghi.Gamma);
    expect(loghi.Alfa).toMatch(new RegExp(`^logos/url-${hash16(new URL(stessoDi).href)}\\.png\\?v=\\d+$`));
    // gli altri sono https (senza schema), e il server di prova parla http: lo scarico non riesce, ma il valore non passa lo stesso
    for (const nome of ["Delta", "Epsilon", "Zeta", "Eta", "Theta"]) expect(loghi[nome], nome).toBeUndefined();
    expect(fuori.loghi_in_corso).toEqual(["Delta", "Epsilon", "Zeta", "Eta", "Theta"]);
    expect(loghi.Iota).toBe("logos/iota.png");
    // e quello che la pagina mostra non esce mai da questo sito
    for (const url of Object.values(mappaLoghi(loghi))) expect(new URL(url, "https://app.example.test/").origin).toBe("https://app.example.test");
  });

  it("un valore che il gateway non scarica (ftp:, data:, https:/ senza due barre) non passa alla pagina in nessun modo: la pagina lo scarta", () => {
    const valori = { A: "ftp://u.prova.test/x.png", B: "data:image/png;base64,AAAA", C: "https:/u.prova.test/x.png", D: "https:u.prova.test/x.png", E: "javascript:alert(1)", F: "C:\\loghi\\x.png" };
    scrivi(valori);
    const fuori = interfaccia();
    // per il gateway sono file: passano come sono, e non parte nessuno scarico
    expect(fuori.loghi).toEqual(valori);
    expect(fuori.loghi_in_corso).toBeUndefined();
    expect(contatori).toEqual({});
    // ma la pagina non li usa: nessuno è un percorso di questo sito
    expect(mappaLoghi(fuori.loghi as Record<string, string>)).toEqual({ "Eventi personali": "/logos/personale.png" });
  });

  it("un file che bloccava il controllo, o un'immagine di miliardi di pixel, non ferma lo scarico: la voce manca e il motivo è nel log", async () => {
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/ostile155`, Beta: `http://u.prova.test:${s.porta}/gigante.png` });
    const t0 = performance.now();
    interfaccia();
    await attendiScarichi();
    expect(performance.now() - t0).toBeLessThan(2000);
    const dopo = interfaccia();
    expect(dopo.loghi).toEqual({});
    expect(dopo.loghi_in_corso).toEqual(["Alfa", "Beta"]);
    const righe = vi.mocked(console.warn).mock.calls.map((c) => c.map(String).join(" ")).join("\n");
    expect(righe).toContain(MSG.nonImmagine);
    expect(righe).toContain(MSG.pixel);
    expect(gestiti()).toEqual([]);
  });

  it("lo scarico in secondo piano usa gli stessi posti dei metodi della finestra: a posti occupati non parte (nessuna coda), liberi riparte", async () => {
    lentoMs = 400;
    const occupanti = Array.from({ length: POSTI_MAX }, (_, i) => anteprimaLogo(s.url(`/lento.png?n=${i}`)));
    await new Promise((ok) => setTimeout(ok, 60));
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/a.png` });
    expect(interfaccia().loghi_in_corso).toEqual(["Alfa"]);
    await attendiScarichi();
    expect(conta("/a.png")).toBe(0);
    await Promise.all(occupanti);
    interfaccia();
    await attendiScarichi();
    expect(conta("/a.png")).toBe(1);
    expect(Object.keys(interfaccia().loghi as object)).toEqual(["Alfa"]);
  });

  it("tre scarichi lenti in secondo piano lasciano il quarto posto a chi preme un pulsante nella finestra: non aspetta", async () => {
    lentoMs = 700;
    scrivi(Object.fromEntries(Array.from({ length: 8 }, (_, i) => [`Voce ${i}`, `http://l${i}.test:${s.porta}/lento.png?n=${i}`])));
    interfaccia();
    await new Promise((ok) => setTimeout(ok, 150));
    expect(massimoInVolo).toBe(3);
    const t0 = performance.now();
    expect((await anteprimaLogo(s.url("/a.png"))).tipo).toBe("image/png");
    expect(performance.now() - t0).toBeLessThan(400);
    expect(massimoInVolo).toBe(3);
  });

  it("gli scarichi insieme sono al più 3 e gli altri partono ai giri dopo", async () => {
    lentoMs = 120;
    const statici = Object.fromEntries(Array.from({ length: 8 }, (_, i) => [`Voce ${i}`, `http://l${i}.test:${s.porta}/lento.png?n=${i}`]));
    scrivi(statici);
    for (let giro = 0; giro < 8; giro++) {
      interfaccia();
      await attendiScarichi();
    }
    expect(massimoInVolo).toBeLessThanOrEqual(3);
    expect(massimoInVolo).toBeGreaterThanOrEqual(2);
    expect(Object.keys(interfaccia().loghi as object)).toHaveLength(8);
  });

  it("tolto un indirizzo da personalizzazione.json, il suo file sparisce alla prima pulizia; uno ancora scritto resta", async () => {
    scrivi({ Alfa: `http://u.prova.test:${s.porta}/a.png`, Beta: `http://u.prova.test:${s.porta}/b.png` });
    interfaccia();
    await attendiScarichi();
    expect(gestiti()).toHaveLength(2);
    scrivi({ Beta: `http://u.prova.test:${s.porta}/b.png`, Gamma: "logos/gamma.png" });
    pulisciLoghi();
    expect(gestiti()).toEqual([`url-${hash16(`http://u.prova.test:${s.porta}/b.png`)}.png`]);
  });

  it("i file messi a mano restano serviti come prima (nessuna voce cambia)", () => {
    scrivi({ Alfa: "logos/alfa.png" });
    fs.mkdirSync(logos(), { recursive: true });
    fs.writeFileSync(path.join(logos(), "alfa.png"), ROSSO);
    expect(interfaccia().loghi).toEqual({ Alfa: "logos/alfa.png" });
    pulisciLoghi();
    expect(gestiti()).toEqual(["alfa.png"]);
  });
});
