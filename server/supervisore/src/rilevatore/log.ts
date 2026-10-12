import { existsSync } from "node:fs";
import { open, type FileHandle } from "node:fs/promises";

const BYTE_DELLA_CODA = 600_000;

const dopoLaPrimaRiga = (testo: string): string => {
  const i = testo.indexOf("\n");
  return i < 0 ? testo : testo.slice(i + 1);
};

async function leggiFinoAllaFine(file: FileHandle, inizio: number): Promise<Buffer> {
  const pezzi: Buffer[] = [];
  let posizione = inizio;
  for (;;) {
    const pezzo = Buffer.alloc(65536);
    const { bytesRead } = await file.read(pezzo, 0, pezzo.length, posizione);
    if (bytesRead === 0) break;
    pezzi.push(pezzo.subarray(0, bytesRead));
    posizione += bytesRead;
  }
  return Buffer.concat(pezzi);
}

/**
 * Le ultime righe intere di un log di Outlook, o null se non c'è (_coda di rileva_accessi.py). Quando il log è appena ruotato quello
 * nuovo è quasi vuoto: si guarda anche la coda del precedente (.1), altrimenti ogni casella sembrerebbe muta.
 */
export async function coda(percorso: string, byte = BYTE_DELLA_CODA): Promise<string | null> {
  const pezzi: string[] = [];
  for (const nome of [percorso + ".1", percorso]) {
    let file: FileHandle | undefined;
    try {
      file = await open(nome, "r");
      const fine = (await file.stat()).size;
      const testo = (await leggiFinoAllaFine(file, Math.max(0, fine - byte))).toString("utf8");
      pezzi.push(fine > byte ? dopoLaPrimaRiga(testo) : testo);
    } catch {
      continue;
    } finally {
      await file?.close().catch(() => undefined);
    }
  }
  if (pezzi.length === 0 || !existsSync(percorso)) return null;
  return pezzi.slice(-2).join("\n");
}
