import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// I file di strumenti/golden/ sono dati di prova fissi: le risposte dell'agente Python e delle sue librerie, registrate una volta sola quando il
// Python c'era ancora (cronologia di git, per esempio il commit 00307be). Nessuno li rigenera: una differenza è un comportamento dell'agente
// TypeScript che è cambiato, da leggere prima di toccare il file.
const GOLDEN = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../strumenti/golden");

export const leggiGolden = <T = any>(...pezzi: string[]): T => JSON.parse(fs.readFileSync(path.join(GOLDEN, ...pezzi), "utf-8"));
export const leggiTestoGolden = (...pezzi: string[]): string => fs.readFileSync(path.join(GOLDEN, ...pezzi), "utf-8");

export type Riga = Record<string, any>;
