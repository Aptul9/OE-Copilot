// Scrive in una cartella un calendar.db inventato (quello del golden dell'agente) e il suo fonti.json: serve a provare l'immagine dell'agente
// in CI, dove non c'è un telefono. Uso: node test/costruisci-db.ts <cartella>
import fs from "node:fs";
import path from "node:path";
import { leggiGolden } from "./ausiliari/dati.ts";
import { scriviDb } from "./ausiliari/memdb.ts";

const cartella = process.argv[2];
if (!cartella) {
  console.error("uso: node test/costruisci-db.ts <cartella>");
  process.exit(2);
}
const righe = leggiGolden("agente-righe.json");
fs.mkdirSync(cartella, { recursive: true });
scriviDb(cartella, { calendari: righe.calendari, eventi: righe.eventi, invitati: righe.invitati });
fs.writeFileSync(path.join(cartella, "fonti.json"), JSON.stringify(righe.fonti.map((f: object) => ({ tipo: "android", ...f }))));
console.log(`calendar.db e fonti.json in ${cartella}`);
