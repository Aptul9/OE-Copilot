import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { archivio, chiudiArchivio, DEFAULT_CONFIG, leggiRigheLocali, loadConfig, modificaConfig, saveConfig } from "@/lib/core/archivio";
import { aggiungiEvento, eliminaEvento } from "@/lib/core/locali";
import { confrontaPunti, colorForAccount, impostaAcceso, rinominaNeiGruppi } from "@/lib/core/stato";
import { pyStrip, testoPulito } from "@/lib/core/py";

// Lo stato dell'app nel database: l'importazione dei file di prima, le transazioni, la copia che si riscrive sui due file.

let dati: string;
const file = (nome: string) => path.join(dati, nome);
const scrivi = (nome: string, contenuto: unknown) => fs.writeFileSync(file(nome), typeof contenuto === "string" ? contenuto : JSON.stringify(contenuto, null, 2));
const leggi = (nome: string) => JSON.parse(fs.readFileSync(file(nome), "utf-8"));
const copieDelGuasto = (nome: string) => fs.readdirSync(dati).filter((f) => f.startsWith(nome + ".rotto-"));

beforeEach(() => {
  dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-archivio-"));
  process.env.OE_DATI = dati;
  delete process.env.GATEWAY_DB;
  for (const m of ["info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
});

afterEach(() => {
  chiudiArchivio();
  fs.rmSync(dati, { recursive: true, force: true });
  delete process.env.OE_DATI;
  vi.restoreAllMocks();
});

describe("l'importazione dei file di prima", () => {
  it("senza file si parte dai valori predefiniti e non si crea niente", () => {
    expect(loadConfig()).toEqual(DEFAULT_CONFIG);
    expect(leggiRigheLocali()).toEqual([]);
    expect(fs.existsSync(file("config.json"))).toBe(false);
    expect(fs.existsSync(file("local_events.json"))).toBe(false);
  });

  it("importa config.json e local_events.json, una volta sola", () => {
    scrivi("config.json", { theme: "dark", disabled_calendars: ["a"], gruppi_calendari: { g: ["X"] } });
    scrivi("local_events.json", [{ uid: "1", title: "Uno" }, { uid: "2", title: "Due" }]);
    expect(loadConfig()).toEqual({ ...DEFAULT_CONFIG, theme: "dark", disabled_calendars: ["a"], gruppi_calendari: { g: ["X"] } });
    expect(leggiRigheLocali().map((e) => e.title)).toEqual(["Uno", "Due"]);
    // da qui il database è la fonte: i file non si rileggono, nemmeno dopo aver chiuso e riaperto
    scrivi("config.json", { theme: "light" });
    scrivi("local_events.json", []);
    expect(loadConfig().theme).toBe("dark");
    chiudiArchivio();
    expect(loadConfig().theme).toBe("dark");
    expect(leggiRigheLocali()).toHaveLength(2);
  });

  it("un config.json illeggibile: se ne tiene una copia e si riparte dai valori predefiniti", () => {
    scrivi("config.json", "{non è json");
    expect(loadConfig()).toEqual(DEFAULT_CONFIG);
    expect(copieDelGuasto("config.json")).toHaveLength(1);
    expect(fs.readFileSync(file(copieDelGuasto("config.json")[0]), "utf-8")).toBe("{non è json");
    loadConfig();
    expect(copieDelGuasto("config.json")).toHaveLength(1);
  });

  it("un local_events.json illeggibile: se ne tiene una copia, gli eventi sono dati che esistono solo qui", () => {
    scrivi("local_events.json", "[{rotto");
    expect(leggiRigheLocali()).toEqual([]);
    expect(fs.readFileSync(file(copieDelGuasto("local_events.json")[0]), "utf-8")).toBe("[{rotto");
  });

  it("un config.json che non è un oggetto fa fallire senza segnare l'importazione: corretto il file, si importa", () => {
    scrivi("config.json", "[1, 2]");
    expect(() => loadConfig()).toThrow();
    scrivi("config.json", { theme: "dark" });
    expect(loadConfig().theme).toBe("dark");
  });
});

describe("le modifiche", () => {
  it("modificaConfig salva, e config.json diventa la copia uguale a quello che si legge", () => {
    modificaConfig((cfg) => {
      cfg.theme = "dark";
      cfg.gruppi_calendari = { g: ["X"] };
    });
    expect(leggi("config.json")).toEqual(loadConfig());
    expect(leggi("config.json")).toMatchObject({ theme: "dark", gruppi_calendari: { g: ["X"] }, zoom_calendario: 58 });
  });

  it("se la modifica solleva non cambia niente e la copia non si riscrive", () => {
    expect(() =>
      modificaConfig((cfg) => {
        cfg.theme = "dark";
        throw new Error("no");
      }),
    ).toThrow("no");
    expect(loadConfig().theme).toBe("auto");
    expect(fs.existsSync(file("config.json"))).toBe(false);
  });

  it("le voci tolte dalla configurazione si tolgono dal database", () => {
    scrivi("config.json", { extra: 1, theme: "dark" });
    modificaConfig((cfg) => void delete cfg.extra);
    expect(loadConfig().extra).toBeUndefined();
    expect(leggi("config.json").extra).toBeUndefined();
  });

  it("una copia di lavoro (_sola_lettura) non si salva, nemmeno quando assegna un colore", () => {
    const copia = { ...loadConfig(), theme: "dark", _sola_lettura: true };
    saveConfig(copia);
    expect(loadConfig().theme).toBe("auto");
    expect(colorForAccount(copia, "nuovo@x.example")).toMatch(/^#/);
    expect(loadConfig().account_colors).toEqual({});
  });

  it("il colore si assegna sullo stato di adesso: quello letto qualche istante fa non cancella le modifiche nel frattempo", () => {
    const vecchio = loadConfig();
    modificaConfig((cfg) => impostaAcceso(cfg, "calendario-spento", false));
    const colore = colorForAccount(vecchio, "nuovo@x.example");
    expect(loadConfig().disabled_calendars).toEqual(["calendario-spento"]);
    expect(loadConfig().account_colors).toEqual({ "nuovo@x.example": colore });
    expect(vecchio.account_colors).toEqual({ "nuovo@x.example": colore });
  });

  it("due colori diversi per due account, anche se uno dei due l'ha assegnato un altro nel frattempo", () => {
    const vecchio = loadConfig();
    const a = colorForAccount(loadConfig(), "a@x.example");
    const b = colorForAccount(vecchio, "b@x.example");
    expect(b).not.toBe(a);
  });

  it("il file di copia che non si può scrivere non ferma la modifica", () => {
    fs.mkdirSync(file("config.json")); // una cartella dove dovrebbe stare il file
    modificaConfig((cfg) => void (cfg.theme = "dark"));
    expect(loadConfig().theme).toBe("dark");
  });

  it("il database sta dove dice GATEWAY_DB, altrimenti nella cartella dei dati", () => {
    loadConfig();
    expect(fs.existsSync(file("gateway.db"))).toBe(true);
    chiudiArchivio();
    process.env.GATEWAY_DB = path.join(dati, "altro", "stato.db");
    try {
      modificaConfig((cfg) => void (cfg.theme = "dark"));
      expect(fs.existsSync(path.join(dati, "altro", "stato.db"))).toBe(true);
    } finally {
      delete process.env.GATEWAY_DB;
    }
  });
});

describe("gli eventi personali", () => {
  it("l'identificativo è il più alto fra quelli di sole cifre, più uno", () => {
    scrivi("local_events.json", [{ uid: "1" }, { uid: "7" }, { uid: "abc" }, { uid: 3 }]);
    aggiungiEvento({ title: "A", start: "2026-10-30T20:00:00" });
    eliminaEvento("7");
    aggiungiEvento({ title: "B", start: "2026-10-31T20:00:00" });
    expect(leggiRigheLocali().map((e) => e.uid)).toEqual(["1", "abc", 3, "8", "9"]);
  });

  it("senza eventi si parte da 1", () => {
    aggiungiEvento({ title: "A", start: "2026-10-30T20:00:00" });
    expect(leggiRigheLocali().map((e) => e.uid)).toEqual(["1"]);
  });

  it("togliere un identificativo toglie tutti gli eventi che lo hanno, e local_events.json segue", () => {
    scrivi("local_events.json", [{ uid: "1" }, { uid: "1" }, { uid: "2" }]);
    eliminaEvento("1");
    expect(leggiRigheLocali()).toEqual([{ uid: "2" }]);
    expect(leggi("local_events.json")).toEqual([{ uid: "2" }]);
  });

  it("un evento rifiutato non lascia niente", () => {
    expect(() => aggiungiEvento({ title: "A", start: "domani" })).toThrow("Data non valida");
    expect(() => aggiungiEvento({ title: "A", start: "2026-10-30T20:00:00", location: 5 })).toThrow("'int' object has no attribute 'strip'");
    expect(leggiRigheLocali()).toEqual([]);
    expect(fs.existsSync(file("local_events.json"))).toBe(false);
  });

  it("l'archivio resta aperto una volta sola", () => {
    expect(archivio()).toBe(archivio());
  });
});

describe("le regole di Python", () => {
  it("str.strip() toglie gli spazi di Unicode e i separatori, non il BOM", () => {
    expect(pyStrip("\u00a0 x \u2003\u001f")).toBe("x");
    expect(pyStrip("\ufeffx")).toBe("\ufeffx");
  });

  it("(valore or '').strip()", () => {
    expect(testoPulito(null)).toBe("");
    expect(testoPulito(0)).toBe("");
    expect(testoPulito("  a ")).toBe("a");
    expect(() => testoPulito(5)).toThrow("'int' object has no attribute 'strip'");
    expect(() => testoPulito(["a"])).toThrow("'list' object has no attribute 'strip'");
  });

  it("sorted() ordina per punto di codice, non per unità UTF-16", () => {
    expect(["\u{1F600}", "\uFFFF", "b", "a"].sort(confrontaPunti)).toEqual(["a", "b", "\uFFFF", "\u{1F600}"]);
  });

  it("un'azienda che cambia nome resta nei suoi gruppi, senza doppioni", () => {
    const cfg = { gruppi_calendari: { g: ["A", "B", "C"], h: ["C"] } };
    rinominaNeiGruppi(cfg, "B", "A");
    expect(cfg.gruppi_calendari).toEqual({ g: ["A", "C"], h: ["C"] });
  });
});
