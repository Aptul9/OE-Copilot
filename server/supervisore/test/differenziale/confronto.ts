import { cpSync, existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { leggiConfigurazione } from "../../src/configurazione.js";
import { rilevaAccessi } from "../../src/rilevatore/accessi.js";
import { ErroreTelefono, type Telefono } from "../../src/telefono.js";

/** Il rilevatore in TypeScript su un caso, con gli stessi file di uscita dell'oracolo (oracolo.py): stato/, stdout.txt, stderr.txt, comandi.txt, crash.txt. */
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

/** Le differenze fra le uscite dell'oracolo e del rilevatore in TypeScript su un caso: vuoto se sono uguali (file, byte per byte). */
export function differenze(oracolo: string, ts: string): string[] {
  const difetti: string[] = [];
  for (const file of ["stato/accessi.json", "stato/google-sync.json"]) {
    const x = leggi(oracolo, file);
    const y = leggi(ts, file);
    if (x === null && y === null) continue;
    if (x === null || y === null) difetti.push(`${file}: c'è solo ${x === null ? "in TypeScript" : "in Python"}`);
    else if (!x.equals(y)) difetti.push(`${file}: byte diversi`);
  }
  if (testo(oracolo, "stdout.txt") !== testo(ts, "stdout.txt")) difetti.push("riga di riepilogo diversa");
  if (controlliNonRiusciti(testo(oracolo, "stderr.txt")).join("|") !== controlliNonRiusciti(testo(ts, "stderr.txt")).join("|")) difetti.push("controlli non riusciti diversi");
  if (testo(oracolo, "comandi.txt") !== testo(ts, "comandi.txt")) difetti.push("comandi al telefono diversi");
  if (testo(oracolo, "crash.txt") !== testo(ts, "crash.txt")) difetti.push("uno dei due si ferma con un errore");
  return difetti;
}
