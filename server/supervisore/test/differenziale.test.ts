import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, describe, expect, test } from "vitest";
import { generaCasi } from "./differenziale/casi.js";
import { differenze, eseguiTs } from "./differenziale/confronto.js";

// Il rilevatore in TypeScript contro server/strumenti/rileva_accessi.py, il rilevatore in Python che sul server gira davvero: stessi
// dati inventati (log di Outlook con la rotazione, database, battito, dumpsys content, ora) e gli stessi file di uscita byte per byte.
// L'oracolo è il file Python senza modifiche: il suo script di prova cambia solo l'ora e le risposte di `docker exec`.

const PYTHON = process.platform === "win32" ? "python" : "python3";
const pythonCiSta = spawnSync(PYTHON, ["--version"]).status === 0;
const RILEVA_ACCESSI = path.resolve(import.meta.dirname, "../../strumenti/rileva_accessi.py");
const ORACOLO = path.join(import.meta.dirname, "differenziale", "oracolo.py");

// Con byte non UTF-8 in dumpsys content Python (text=True) si ferma e non dà verdetti Google, il rilevatore in TypeScript legge con i
// caratteri di sostituzione e li dà: è l'unica differenza voluta.
const DIFFERENZE_VOLUTE = ["google-dump-utf8-invalido"];

const radice = mkdtempSync(path.join(os.tmpdir(), "differenziale-"));
afterAll(() => rmSync(radice, { recursive: true, force: true }));

describe.skipIf(!pythonCiSta)("il rilevatore in TypeScript contro rileva_accessi.py", () => {
  test("stessi file, stessa riga di riepilogo e stessi comandi al telefono su ogni caso inventato", async () => {
    expect(existsSync(RILEVA_ACCESSI), `manca ${RILEVA_ACCESSI}`).toBe(true);
    const casi = path.join(radice, "casi");
    const totale = generaCasi(casi);
    expect(totale).toBeGreaterThan(250);
    const uscitaPython = path.join(radice, "python");
    const p = spawnSync(PYTHON, [ORACOLO, RILEVA_ACCESSI, casi, uscitaPython], { encoding: "utf8" });
    expect(p.stderr).toBe("");
    expect(p.status).toBe(0);
    expect(existsSync(path.join(uscitaPython, "log-solo-corrente", "stato", "accessi.json"))).toBe(true);
    const diversi: Record<string, string[]> = {};
    for (const nome of readdirSync(casi).sort()) {
      const uscitaTs = path.join(radice, "ts", nome);
      await eseguiTs(path.join(casi, nome), uscitaTs);
      const d = differenze(path.join(uscitaPython, nome), uscitaTs);
      if (d.length > 0) diversi[nome] = d;
    }
    expect(Object.keys(diversi)).toEqual(DIFFERENZE_VOLUTE);
  }, 300_000);
});
