import { cpSync, existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { leggiConfigurazione } from "../../src/configurazione.js";
import { rilevaAccessi } from "../../src/rilevatore/accessi.js";
import { ErroreTelefono, type Telefono } from "../../src/telefono.js";

/** Le uscite registrate di rileva_accessi.py su un caso (test/golden/rilevatore-accessi.json). I file di stato sono testo ASCII: Python li scriveva con json.dump. */
export interface Atteso {
  /** stato/accessi.json come scritto, null se main() non lo scriveva. */
  accessi: string | null;
  /** stato/google-sync.json alla fine del giro, null se non c'era. */
  google: string | null;
  /** La riga che main() stampa, con l'a capo (vuota se non stampava niente). */
  riepilogo: string;
  /** I controlli che main() segnalava non riusciti sullo standard error, per nome. */
  nonRiusciti: string[];
  /** I comandi chiesti al telefono, uno per riga, col timeout in secondi. */
  comandi: string;
  /** Il nome dell'errore che fermava main(), null se non si fermava. */
  crash: string | null;
}

/** Il rilevatore in TypeScript su un caso: stato/ (dove scrive accessi.json e google-sync.json), stdout.txt, stderr.txt, comandi.txt, crash.txt. */
export async function eseguiTs(caso: string, uscita: string): Promise<void> {
  const adesso = Number(readFileSync(path.join(caso, "adesso"), "utf8"));
  const env = JSON.parse(readFileSync(path.join(caso, "env.json"), "utf8")) as Record<string, string>;
  rmSync(uscita, { recursive: true, force: true });
  cpSync(path.join(caso, "stato"), path.join(uscita, "stato"), { recursive: true });
  const comandi: string[] = [];
  const stderr: string[] = [];
  const telefono: Telefono = {
    vivo: async () => true,
    agisci: async (c) => {
      throw new ErroreTelefono(`azione non permessa: ${c.join(" ")}`, "non-permesso");
    },
    leggi: async (c, o) => {
      const testo = c.join(" ");
      comandi.push(`${testo} timeout=${(o?.timeoutMs ?? 30_000) / 1000}`);
      const file = { "dumpsys content": "dumpsys-content.txt", "date +%Y-%m-%d %H:%M:%S": "date.txt" }[testo];
      if (file === undefined) throw new Error(`comando non previsto: ${testo}`);
      const percorso = path.join(caso, file);
      return existsSync(percorso) ? readFileSync(percorso, "utf8") : "";
    },
  };
  let riepilogo = "";
  let crash: string | null = null;
  try {
    riepilogo = await rilevaAccessi({
      dati: path.join(caso, "dati"),
      uscita: path.join(uscita, "stato", "accessi.json"),
      telefono,
      orologio: { adesso: () => adesso * 1000, dormi: async () => undefined },
      soglie: leggiConfigurazione(env).configurazione.soglie,
      log: (m) => stderr.push(m),
    });
  } catch (e) {
    crash = e instanceof Error ? e.constructor.name : "Error";
  }
  writeFileSync(path.join(uscita, "stdout.txt"), riepilogo === "" ? "" : riepilogo + "\n");
  writeFileSync(path.join(uscita, "stderr.txt"), stderr.map((m) => m + "\n").join(""));
  writeFileSync(path.join(uscita, "comandi.txt"), comandi.join("\n") + "\n");
  if (crash !== null) writeFileSync(path.join(uscita, "crash.txt"), crash + "\n");
}

const leggi = (cartella: string, file: string): Buffer | null => (existsSync(path.join(cartella, file)) ? readFileSync(path.join(cartella, file)) : null);
const testo = (cartella: string, file: string): string | null => leggi(cartella, file)?.toString("utf8") ?? null;
const controlliNonRiusciti = (t: string | null): string[] =>
  (t ?? "")
    .split("\n")
    .filter((r) => r.includes("non riuscito"))
    .map((r) => r.replace(/^rilevatore: (.+?) non riuscito.*$/, "$1"));

/** Le differenze fra le uscite registrate e quelle del rilevatore in TypeScript su un caso: vuoto se sono uguali (file, byte per byte). */
export function differenze(atteso: Atteso, ts: string): string[] {
  const difetti: string[] = [];
  for (const [file, registrato] of [
    ["stato/accessi.json", atteso.accessi],
    ["stato/google-sync.json", atteso.google],
  ] as const) {
    const y = leggi(ts, file);
    if (registrato === null && y === null) continue;
    if (registrato === null || y === null) difetti.push(`${file}: c'è solo ${registrato === null ? "in TypeScript" : "nella registrazione"}`);
    else if (!Buffer.from(registrato, "latin1").equals(y)) difetti.push(`${file}: byte diversi`);
  }
  if (atteso.riepilogo !== testo(ts, "stdout.txt")) difetti.push("riga di riepilogo diversa");
  if (atteso.nonRiusciti.join("|") !== controlliNonRiusciti(testo(ts, "stderr.txt")).join("|")) difetti.push("controlli non riusciti diversi");
  if (atteso.comandi !== testo(ts, "comandi.txt")) difetti.push("comandi al telefono diversi");
  if (atteso.crash !== (testo(ts, "crash.txt")?.replace(/\n$/, "") ?? null)) difetti.push("uno dei due si ferma con un errore");
  return difetti;
}
