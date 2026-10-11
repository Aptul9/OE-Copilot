import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const GOLDEN = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../strumenti/golden");

export const leggiGolden = <T = any>(...pezzi: string[]): T => JSON.parse(fs.readFileSync(path.join(GOLDEN, ...pezzi), "utf-8"));
export const leggiTestoGolden = (...pezzi: string[]): string => fs.readFileSync(path.join(GOLDEN, ...pezzi), "utf-8");

export type Riga = Record<string, any>;
