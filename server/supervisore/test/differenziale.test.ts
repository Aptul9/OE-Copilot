import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, describe, expect, test } from "vitest";
import { generaCasi } from "./differenziale/casi.js";
import { differenze, eseguiTs, type Atteso } from "./differenziale/confronto.js";

// Il rilevatore in TypeScript contro le uscite registrate di rileva_accessi.py, il rilevatore in Python che sul server girava prima del
// supervisore: stessi dati inventati (log di Outlook con la rotazione, database, battito, dumpsys content, ora) e gli stessi file di
// uscita byte per byte. Le uscite del Python stanno in test/golden/rilevatore-accessi.json: registrate una volta sola (2026-10-12, Python
// 3.12 su Linux) su tutti i casi di differenziale/casi.ts, quando il Python è stato tolto dal repository (è nella cronologia di git, per
// esempio nel commit 00307be). Il file non si rigenera: se cambia un caso o un'uscita, la differenza è un comportamento del rilevatore.

const REGISTRAZIONE = path.join(import.meta.dirname, "golden", "rilevatore-accessi.json");
const registrati = (JSON.parse(readFileSync(REGISTRAZIONE, "utf8")) as { casi: Record<string, Atteso> }).casi;

// Con byte non UTF-8 in dumpsys content Python (text=True) si ferma e non dà verdetti Google, il rilevatore in TypeScript legge con i
// caratteri di sostituzione e li dà: è l'unica differenza voluta.
const DIFFERENZE_VOLUTE = ["google-dump-utf8-invalido"];

const radice = mkdtempSync(path.join(os.tmpdir(), "differenziale-"));
// su Windows togliere qualche migliaio di file può durare più dei 10 secondi di un gancio
afterAll(() => rmSync(radice, { recursive: true, force: true }), 120_000);

describe("il rilevatore in TypeScript contro le uscite registrate di rileva_accessi.py", () => {
  test("stessi file, stessa riga di riepilogo e stessi comandi al telefono su ogni caso inventato", async () => {
    const casi = path.join(radice, "casi");
    const totale = generaCasi(casi);
    expect(totale).toBeGreaterThan(250);
    const nomi = readdirSync(casi).sort();
    expect(nomi).toEqual(Object.keys(registrati).sort());
    const diversi: Record<string, string[]> = {};
    for (const nome of nomi) {
      const uscitaTs = path.join(radice, "ts", nome);
      await eseguiTs(path.join(casi, nome), uscitaTs);
      const d = differenze(registrati[nome] as Atteso, uscitaTs);
      if (d.length > 0) diversi[nome] = d;
    }
    expect(Object.keys(diversi)).toEqual(DIFFERENZE_VOLUTE);
  }, 600_000);
});
