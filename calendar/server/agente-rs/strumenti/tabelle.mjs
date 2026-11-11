// Rigenera nucleo/src/entita.rs e nucleo/src/windows.rs dalle tabelle di agente-ts/src (entita.ts, windows.ts), che restano l'origine.
//   node calendar/server/agente-rs/strumenti/tabelle.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const qui = path.dirname(fileURLToPath(import.meta.url));
const ts = path.resolve(qui, "../../agente-ts/src");
const nucleo = path.resolve(qui, "../nucleo/src");
const { HTML5, HTML5_IMPRONTA } = await import(pathToFileURL(path.join(ts, "entita.ts")).href);
const { WINDOWS } = await import(pathToFileURL(path.join(ts, "windows.ts")).href);

const lit = (s) => '"' + [...s].map((c) => { const n = c.codePointAt(0); if (c === '"') return '\\"'; if (c === "\\") return "\\\\"; if (n >= 0x20 && n < 0x7f) return c; return `\\u{${n.toString(16)}}`; }).join("") + '"';
const cmp = (a, b) => (Buffer.compare(Buffer.from(a[0]), Buffer.from(b[0])));

const ent = [...HTML5.entries()].sort(cmp);
let o = `// GENERATO da agente-ts/src/entita.ts (html.entities.html5 di Python 3.12.15): ${ent.length} nomi, con e senza il punto e virgola finale, in ordine di byte.\n// Non si modifica a mano. Impronta sha256 delle coppie ordinate della tabella di origine: ${HTML5_IMPRONTA}\n\npub static HTML5: [(&str, &str); ${ent.length}] = [\n`;
for (const [k, v] of ent) o += `    (${lit(k)}, ${lit(v)}),\n`;
o += "];\n\n/// Il testo di un'entità, se il nome (con il punto e virgola, se c'è) è nella tabella.\npub fn html5(nome: &str) -> Option<&'static str> {\n    HTML5.binary_search_by(|(k, _)| k.as_bytes().cmp(nome.as_bytes())).ok().map(|i| HTML5[i].1)\n}\n";
fs.writeFileSync(path.join(nucleo, "entita.rs"), o);

const win = Object.entries(WINDOWS).sort(cmp);
let w = `// GENERATO da agente-ts/src/windows.ts: i nomi dei fusi di Windows e il fuso IANA che icalendar 7.3.0 dà a ciascuno (dati CLDR del 2025-04-10).\n// Non si modifica a mano.\n\npub static WINDOWS: [(&str, &str); ${win.length}] = [\n`;
for (const [k, v] of win) w += `    (${lit(k)}, ${lit(v)}),\n`;
w += "];\n\npub fn windows(nome: &str) -> Option<&'static str> {\n    WINDOWS.binary_search_by(|(k, _)| k.as_bytes().cmp(nome.as_bytes())).ok().map(|i| WINDOWS[i].1)\n}\n";
fs.writeFileSync(path.join(nucleo, "windows.rs"), w);
console.log(ent.length, "entità,", win.length, "fusi di Windows");
