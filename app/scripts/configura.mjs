// Scrive src-tauri/tauri.conf.local.json (ignorato da git): la configurazione tracciata con l'indirizzo del sito
// preso da OE_URL al posto del segnaposto. L'indirizzo non sta in nessun file del repository.
//   node scripts/configura.mjs            prepara il file, anche con il segnaposto (per cargo check e simili)
//   node scripts/configura.mjs --release  come sopra, ma con il segnaposto è un errore
//   node scripts/configura.mjs --prova    aggiunge il comando di prova degli interruttori del menu (solo per le prove)
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SEGNAPOSTO = "https://oe.example.com";
const cartella = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri");
const argomenti = process.argv.slice(2);
const release = argomenti.includes("--release");
const prova = argomenti.includes("--prova");

function fallisci(messaggio) {
  console.error(`configura: ${messaggio}`);
  process.exit(1);
}

function origine(testo) {
  let url;
  try {
    url = new URL(testo);
  } catch {
    fallisci(`OE_URL non è un indirizzo valido: "${testo}"`);
  }
  if (url.protocol !== "https:" && !(url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname))) {
    fallisci("OE_URL deve cominciare con https:// (http:// solo per localhost)");
  }
  if (url.username || url.password) fallisci("OE_URL non deve contenere utente o password");
  if (url.pathname !== "/" || url.search || url.hash) fallisci("OE_URL è solo l'indirizzo del sito (schema, nome e porta), senza percorso");
  return url.origin;
}

const richiesto = (process.env.OE_URL ?? "").trim();
const sito = richiesto ? origine(richiesto) : SEGNAPOSTO;
if (release && sito === SEGNAPOSTO) {
  fallisci("OE_URL non è impostato (o è ancora il segnaposto). Esempio: $env:OE_URL = 'https://oe.tuodominio.it'; npm run build");
}

function sostituisci(valore) {
  if (typeof valore === "string") return valore.replaceAll(SEGNAPOSTO, sito);
  if (Array.isArray(valore)) return valore.map(sostituisci);
  if (valore && typeof valore === "object") return Object.fromEntries(Object.entries(valore).map(([k, v]) => [k, sostituisci(v)]));
  return valore;
}

const configurazione = sostituisci(JSON.parse(readFileSync(join(cartella, "tauri.conf.json"), "utf8")));
if (prova) {
  for (const capacita of configurazione.app.security.capabilities) capacita.permissions.push("allow-prova-menu");
}
for (const capacita of configurazione.app.security.capabilities) {
  if (capacita.permissions.some((p) => p === "core:default" || p.endsWith(":default"))) fallisci("la capability remota non ammette permessi default");
}
writeFileSync(join(cartella, "tauri.conf.local.json"), JSON.stringify(configurazione, null, 2) + "\n");
console.log(`configura: tauri.conf.local.json scritto${prova ? " (con i comandi di prova)" : ""}`);
