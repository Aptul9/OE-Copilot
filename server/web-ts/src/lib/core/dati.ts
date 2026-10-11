import fs from "node:fs";
import path from "node:path";
import { config } from "../config";

/** Un file della cartella dei dati (ui-dati/ sul server). */
export const nelleDati = (...parti: string[]) => path.join(config.dati, ...parti);

/**
 * Salvataggio atomico (config.scrivi_json): prima un file temporaneo nella stessa cartella, poi la sostituzione. Un'interruzione
 * a metà lascia il file com'era, mai un JSON troncato. Come in Python: indentato di 2, senza ascii forzato, permessi 600.
 */
export function scriviJson(percorso: string, dati: unknown): void {
  const cartella = path.dirname(percorso);
  fs.mkdirSync(cartella, { recursive: true });
  const tmp = path.join(cartella, `.tmp_${process.pid}_${Math.random().toString(36).slice(2)}.json`);
  const fd = fs.openSync(tmp, "w", 0o600);
  try {
    fs.writeSync(fd, JSON.stringify(dati, null, 2));
    fs.fsyncSync(fd);
  } catch (e) {
    fs.closeSync(fd);
    fs.rmSync(tmp, { force: true });
    throw e;
  }
  fs.closeSync(fd);
  try {
    fs.renameSync(tmp, percorso);
  } catch (e) {
    fs.rmSync(tmp, { force: true });
    throw e;
  }
}

/** Il contenuto di un file di testo, o null se non c'è. */
export function leggiTesto(percorso: string): string | null {
  try {
    return fs.readFileSync(percorso, "utf-8");
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw e;
  }
}
