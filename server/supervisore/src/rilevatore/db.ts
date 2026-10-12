import { existsSync, rmSync, statSync } from "node:fs";
import { chmod, copyFile, mkdtemp } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

export interface CopiaDb {
  db: DatabaseSync;
  chiudi: () => void;
}

const inMemoria = (): string => {
  try {
    return statSync("/dev/shm").isDirectory() ? "/dev/shm" : os.tmpdir();
  } catch {
    return os.tmpdir();
  }
};

/** Copia il database coi suoi file di appoggio e lo apre: Android lo tiene aperto (_apri_ro di rileva_accessi.py). */
export async function apriCopia(percorso: string): Promise<CopiaDb> {
  const cartella = await mkdtemp(path.join(inMemoria(), "oe-accessi-"));
  await chmod(cartella, 0o700);
  const chiudi = (): void => rmSync(cartella, { recursive: true, force: true });
  try {
    for (const suffisso of ["", "-journal", "-wal", "-shm"]) {
      if (existsSync(percorso + suffisso)) await copyFile(percorso + suffisso, path.join(cartella, "db" + suffisso));
    }
    const db = new DatabaseSync(path.join(cartella, "db"));
    return {
      db,
      chiudi: () => {
        try {
          db.close();
        } finally {
          chiudi();
        }
      },
    };
  } catch (e) {
    chiudi();
    throw e;
  }
}
