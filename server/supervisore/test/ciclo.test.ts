import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { Guasti, esegui, giro, idLavori, pausaAttiva, testoBattito, type DipendenzeCiclo } from "../src/ciclo.js";
import { leggiConfigurazione, type Configurazione } from "../src/configurazione.js";
import { OrologioFinto } from "../src/orologio.js";
import { ErroreTelefono, type Telefono } from "../src/telefono.js";
import { INIZIO, LAVORI_DEL_TELEFONO, OGNI, SCENARI, faseNormale, iniziaGiro, nuovoStato, rispondi, type Scenario } from "./telefono-simulato.js";

// Il ciclo contro lo script: per ogni scenario la sequenza dei comandi al telefono, delle attese e dei battiti è quella che
// sincronizza_telefono.sh ha prodotto con docker, sleep e date finti (test/golden/ciclo-<scenario>.txt, scritti dallo script vero
// con la stessa simulazione del telefono di telefono-simulato.ts: test/script-finto/tutti.sh li rigenera). Se il ciclo cambia, il confronto
// dice dove.

const dellaFase = <T>(v: readonly T[], n: number): T => v[Math.min(n, v.length - 1)] as T;
const cartelle: string[] = [];

afterEach(() => {
  for (const c of cartelle.splice(0)) rmSync(c, { recursive: true, force: true });
});

function nuovaRadice(): string {
  const radice = mkdtempSync(path.join(os.tmpdir(), "ciclo-"));
  cartelle.push(radice);
  mkdirSync(path.join(radice, "telefono"), { recursive: true });
  mkdirSync(path.join(radice, "stato", "schermo"), { recursive: true });
  return radice;
}

const configurazione = (radice: string, env: Record<string, string> = {}): Configurazione =>
  leggiConfigurazione({ OE_DIR: radice, OGNI: String(OGNI), ...env }).configurazione;

const battitoDi = (radice: string): string => readFileSync(path.join(radice, "stato", "battito.json"), "utf8").trim();

async function traccia(s: Scenario): Promise<string> {
  const radice = nuovaRadice();
  const config = configurazione(radice, {
    ACCESSI_OGNI: String(s.accessiOgni),
    GOOGLE_OGNI: String(s.googleOgni),
    TOCCO_OGNI: String(s.toccoOgni),
  });
  const orologio = new OrologioFinto(INIZIO * 1000);
  const righe: string[] = [];
  const stato = nuovoStato(dellaFase(s.fasi, 0));
  const telefono: Telefono = {
    vivo: async () => {
      righe.push("cmd true");
      return rispondi(stato, ["true"]).riuscito;
    },
    leggi: async (comando) => {
      righe.push(`cmd ${comando.join(" ")}`);
      return rispondi(stato, comando).uscita;
    },
    agisci: async (comando) => {
      righe.push(`cmd ${comando.join(" ")}`);
      rispondi(stato, comando);
    },
  };
  const fermo = new AbortController();
  let n = 0;
  const prepara = (): void => {
    iniziaGiro(stato, dellaFase(s.fasi, n));
    const manuale = path.join(radice, "telefono", "PAUSA");
    const schermo = path.join(radice, "stato", "schermo", "PAUSA");
    rmSync(manuale, { force: true });
    rmSync(schermo, { force: true });
    if (dellaFase(s.pausaManuale, n)) writeFileSync(manuale, "");
    const eta = dellaFase(s.pausaSchermo, n);
    if (eta !== null) {
      writeFileSync(schermo, "");
      const t = orologio.adesso() / 1000 - eta;
      utimesSync(schermo, t, t);
    }
  };
  prepara();
  orologio.suAttesa = (ms) => {
    if (ms !== OGNI * 1000) {
      righe.push(`sleep ${ms / 1000}`);
      return;
    }
    righe.push(`battito ${battitoDi(radice)}`, `sleep ${OGNI}`);
    n += 1;
    if (n >= s.giri) fermo.abort();
    else prepara();
  };
  await esegui({ config, telefono, orologio, rilevatore: async () => void righe.push("rilevatore"), log: () => undefined }, fermo.signal);
  return righe.join("\n") + "\n";
}

describe("il ciclo fa quello che faceva lo script, scenario per scenario", () => {
  for (const s of SCENARI) {
    test(s.nome, async () => {
      const atteso = readFileSync(path.join(import.meta.dirname, "golden", `ciclo-${s.nome}.txt`), "utf8");
      expect(await traccia(s)).toBe(atteso);
    });
  }
});

describe("pezzi del ciclo", () => {
  test("i lavori da forzare: gli identificativi senza ripetizioni, in ordine di testo come sort -u, Google solo ogni GOOGLE_OGNI giri", () => {
    expect(idLavori(LAVORI_DEL_TELEFONO, false)).toEqual(["10", "9"]);
    expect(idLavori(LAVORI_DEL_TELEFONO, true)).toEqual(["10", "12", "9"]);
    expect(idLavori("", true)).toEqual([]);
    expect(idLavori("JOB #1000/7: abc com.android.calendar/com.microsoft.office.outlook.x JOB #1000/8: def com.android.calendar/com.microsoft.office.outlook.y", false)).toEqual(["7", "8"]);
    expect(idLavori("JOB #1001/7: abc com.android.calendar/com.microsoft.office.outlook.x", false)).toEqual([]);
    expect(idLavori("JOB #1000/7: ABC com.android.calendar/com.microsoft.office.outlook.x", false)).toEqual([]);
  });

  test("il battito ha gli stessi spazi del printf dello script", () => {
    expect(testoBattito(1760000000, { pausa: false, telefono: true, davanti: true, bloccato: false })).toBe(
      '{"quando": 1760000000, "pausa": false, "telefono": true, "outlook_davanti": true, "bloccato": false}\n',
    );
  });

  test("la pausa: telefono/PAUSA, oppure stato/schermo/PAUSA più giovane di 60 secondi, un file e non una cartella", () => {
    const radice = nuovaRadice();
    const c = configurazione(radice);
    const orologio = new OrologioFinto(INIZIO * 1000);
    expect(pausaAttiva(c, orologio)).toBe(false);
    writeFileSync(c.pausaSchermo, "");
    utimesSync(c.pausaSchermo, INIZIO - 59, INIZIO - 59);
    expect(pausaAttiva(c, orologio)).toBe(true);
    utimesSync(c.pausaSchermo, INIZIO - 60, INIZIO - 60);
    expect(pausaAttiva(c, orologio)).toBe(false);
    rmSync(c.pausaSchermo);
    mkdirSync(c.pausaSchermo);
    expect(pausaAttiva(c, orologio)).toBe(false);
    writeFileSync(c.pausaManuale, "");
    expect(pausaAttiva(c, orologio)).toBe(true);
  });

  test("le variabili d'ambiente dello script e i loro valori di partenza", () => {
    const { configurazione: c, avvisi } = leggiConfigurazione({});
    expect([c.ogni, c.pausaSchermoScade, c.accessiOgni, c.googleOgni, c.toccoOgni]).toEqual([30, 60, 2, 4, 10]);
    expect([c.soglie.outlookGialloS, c.soglie.outlookRossoS, c.soglie.googleGialloS, c.soglie.googleRossoS]).toEqual([600, 1800, 900, 2700]);
    expect([c.telefono, c.pausaManuale, c.pausaSchermo, c.accessiJson]).toEqual([
      "telefono:5555",
      path.join("/opt/oe-copilot", "telefono", "PAUSA"),
      path.join("/opt/oe-copilot", "stato", "schermo", "PAUSA"),
      path.join("/opt/oe-copilot", "stato", "accessi.json"),
    ]);
    expect(avvisi).toEqual([]);
    const sbagliato = leggiConfigurazione({ TOCCO_OGNI: "dieci", ACCESSI_OGNI: "0", OGNI: "-1", OUTLOOK_ROSSO_S: "1800" });
    expect(sbagliato.avvisi.length).toBe(3);
    expect([sbagliato.configurazione.toccoOgni, sbagliato.configurazione.accessiOgni, sbagliato.configurazione.ogni]).toEqual([10, 2, 30]);
    expect(leggiConfigurazione({ TOCCO_OGNI: "0" }).configurazione.toccoOgni).toBe(0);
  });
});

describe("quando qualcosa si rompe il ciclo non muore e il battito lo dice", () => {
  function prepara(telefono: Telefono, env: Record<string, string> = {}) {
    const radice = nuovaRadice();
    const righe: string[] = [];
    const orologio = new OrologioFinto(INIZIO * 1000);
    const d: DipendenzeCiclo = {
      config: configurazione(radice, env),
      telefono,
      orologio,
      rilevatore: async () => void righe.push("rilevatore"),
      log: (m) => righe.push(`log: ${m}`),
    };
    return { radice, righe, orologio, d };
  }
  const fase = faseNormale();
  const telefonoNormale = (): Telefono => {
    const stato = nuovoStato(fase);
    return {
      vivo: async () => true,
      leggi: async (c) => rispondi(stato, c).uscita,
      agisci: async (c) => void rispondi(stato, c),
    };
  };

  test("telefono che non risponde: battito con telefono false, nessun comando e nessun rilevatore", async () => {
    const comandi: string[] = [];
    const { radice, righe, d } = prepara({ vivo: async () => false, leggi: async (c) => (comandi.push(c.join(" ")), ""), agisci: async (c) => void comandi.push(c.join(" ")) });
    const r = await giro(0, d);
    expect(r.battito).toEqual({ pausa: false, telefono: false, davanti: false, bloccato: false });
    expect(r.nonRaggiunto).toBe("nessuna risposta ai comandi adb");
    expect(comandi).toEqual([]);
    expect(righe).toEqual([]);
    expect(battitoDi(radice)).toBe('{"quando": 1760000000, "pausa": false, "telefono": false, "outlook_davanti": false, "bloccato": false}');
  });

  test("adb che cade a metà giro: il giro si ferma, il battito dice telefono false anche con la finestra già letta", async () => {
    const buono = telefonoNormale();
    let finestre = 0;
    const { radice, d, righe } = prepara({
      vivo: async () => true,
      leggi: async (c) => {
        if (c.join(" ") === "dumpsys window" && ++finestre === 2) throw new ErroreTelefono("device offline", "trasporto");
        return buono.leggi(c);
      },
      agisci: (c) => buono.agisci(c),
    });
    const r = await giro(0, d);
    expect(r.battito).toEqual({ pausa: false, telefono: false, davanti: false, bloccato: false });
    expect(r.nonRaggiunto).toBe("device offline");
    expect(righe).toEqual([]);
    expect(battitoDi(radice)).toContain('"telefono": false');
  });

  test("un comando che non risponde in tempo vale come telefono non raggiunto", async () => {
    const buono = telefonoNormale();
    const { d } = prepara({
      vivo: async () => true,
      leggi: async (c) => {
        if (c.join(" ") === "dumpsys power") throw new ErroreTelefono("nessuna risposta in 30 s", "tempo");
        return buono.leggi(c);
      },
      agisci: (c) => buono.agisci(c),
    });
    expect((await giro(0, d)).nonRaggiunto).toBe("nessuna risposta in 30 s");
  });

  test("ogni giro finisce nel log una volta sola, finché il guasto dura, e quando passa", async () => {
    let giu = true;
    const buono = telefonoNormale();
    const { orologio, d, righe } = prepara({ vivo: async () => !giu, leggi: (c) => buono.leggi(c), agisci: (c) => buono.agisci(c) });
    const fermo = new AbortController();
    let giri = 0;
    orologio.suAttesa = (ms) => {
      if (ms !== OGNI * 1000) return;
      giri += 1;
      if (giri === 4) giu = false;
      if (giri === 6) fermo.abort();
    };
    await esegui(d, fermo.signal);
    expect(righe.filter((r) => r.startsWith("log:"))).toEqual(["log: telefono non raggiunto: nessuna risposta ai comandi adb", "log: telefono non raggiunto: di nuovo a posto"]);
  });

  test("un errore che non è del telefono ferma il giro, non il servizio, e si scrive una volta", async () => {
    let giri = 0;
    const { orologio, d, righe, radice } = prepara({
      vivo: async () => {
        giri += 1;
        throw new TypeError("difetto");
      },
      leggi: async () => "",
      agisci: async () => undefined,
    });
    const fermo = new AbortController();
    orologio.suAttesa = (ms) => {
      if (ms === OGNI * 1000 && giri >= 5) fermo.abort();
    };
    await esegui(d, fermo.signal);
    expect(giri).toBe(5);
    expect(righe).toEqual(["log: giro non riuscito: difetto"]);
    expect(() => battitoDi(radice)).toThrow();
  });

  test("il battito che non si scrive e il rilevatore che si rompe non fermano il giro e si scrivono una volta", async () => {
    const { orologio, d, righe, radice } = prepara(telefonoNormale(), { ACCESSI_OGNI: "1" });
    d.rilevatore = async () => {
      throw new Error("db illeggibile");
    };
    rmSync(path.join(radice, "stato"), { recursive: true });
    writeFileSync(path.join(radice, "stato"), "un file al posto della cartella");
    const fermo = new AbortController();
    let giri = 0;
    orologio.suAttesa = (ms) => {
      if (ms === OGNI * 1000 && ++giri === 4) fermo.abort();
    };
    await esegui(d, fermo.signal);
    expect(giri).toBe(4);
    expect(righe.filter((r) => r.startsWith("log: battito")).length).toBe(1);
    expect(righe.filter((r) => r.startsWith("log: rilevatore")).length).toBe(1);
  });

  test("la cadenza: 34 secondi fra due battiti di un giro normale (30 di pausa, 2 e 2 delle attese), 35 con lo schermo da riaccendere", async () => {
    const radice = nuovaRadice();
    const config = configurazione(radice, { TOCCO_OGNI: "0" });
    const orologio = new OrologioFinto(INIZIO * 1000);
    const stato = nuovoStato({ ...faseNormale(), sveglio: false });
    const telefono: Telefono = { vivo: async () => true, leggi: async (c) => rispondi(stato, c).uscita, agisci: async (c) => void rispondi(stato, c) };
    const tempi: number[] = [];
    const fermo = new AbortController();
    orologio.suAttesa = (ms) => {
      if (ms !== OGNI * 1000) return;
      tempi.push(JSON.parse(battitoDi(radice)).quando);
      stato.sveglio = tempi.length % 2 === 0;
      if (tempi.length === 4) fermo.abort();
    };
    await esegui({ config, telefono, orologio, rilevatore: async () => undefined, log: () => undefined }, fermo.signal);
    expect(tempi.slice(1).map((t, i) => t - (tempi[i] ?? 0))).toEqual([35, 34, 35]);
  });

  test.skipIf(process.platform === "win32")("il battito sostituisce un file vecchio che il suo utente non può scrivere (basta poter scrivere la cartella)", async () => {
    const { d, radice } = prepara(telefonoNormale());
    writeFileSync(path.join(radice, "stato", "battito.json"), "vecchio");
    chmodSync(path.join(radice, "stato", "battito.json"), 0o444);
    await giro(0, d);
    expect(battitoDi(radice)).toContain('"telefono": true');
  });

  test("la riga di guasto si ripete solo se il guasto torna", () => {
    const righe: string[] = [];
    const g = new Guasti((m) => righe.push(m));
    g.segnala("a", "uno");
    g.segnala("a", "uno");
    g.segnala("a", "due");
    g.risolto("a");
    g.risolto("a");
    g.segnala("a", "due");
    expect(righe).toEqual(["a: uno", "a: due", "a: di nuovo a posto", "a: due"]);
  });
});
