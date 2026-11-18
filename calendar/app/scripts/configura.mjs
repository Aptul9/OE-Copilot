// Scrive src-tauri/tauri.conf.local.json (ignorato da git): la configurazione tracciata con l'indirizzo del sito
// preso da OE_URL al posto del segnaposto. L'indirizzo non sta in nessun file del repository.
//   node scripts/configura.mjs            prepara il file, anche con il segnaposto (per cargo check e simili)
//   node scripts/configura.mjs --release  come sopra, ma con il segnaposto è un errore
//   node scripts/configura.mjs --prova    aggiunge il comando di prova del menu dell'icona (solo per le prove)
//   node scripts/configura.mjs --android  applica anche tauri.android.conf.json (identificativo e simili dell'app Android)
//   node scripts/configura.mjs --emulatore  OE_URL può essere http://10.0.2.2:<porta> (il PC visto dall'emulatore, solo per le prove)
//   OE_BUILD=<numero>                       la versione diventa <versione>+<numero> (la pipeline app-windows.yml, col numero del run)
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SEGNAPOSTO = "https://oe.example.com";
const cartella = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri");
const argomenti = process.argv.slice(2);
const release = argomenti.includes("--release");
const prova = argomenti.includes("--prova");
const android = argomenti.includes("--android");
const emulatore = argomenti.includes("--emulatore");

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
  const locale = url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  const pcDellEmulatore = emulatore && url.protocol === "http:" && url.hostname === "10.0.2.2";
  if (url.protocol !== "https:" && !locale && !pcDellEmulatore) {
    fallisci("OE_URL deve cominciare con https:// (http:// solo per localhost, o per 10.0.2.2 con --emulatore)");
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

// JSON Merge Patch (RFC 7396), come lo applica Tauri ai file di piattaforma
function unisci(base, patch) {
  if (patch === null || typeof patch !== "object" || Array.isArray(patch)) return patch;
  const risultato = base && typeof base === "object" && !Array.isArray(base) ? { ...base } : {};
  for (const [chiave, valore] of Object.entries(patch)) {
    if (valore === null) delete risultato[chiave];
    else risultato[chiave] = unisci(risultato[chiave], valore);
  }
  return risultato;
}

let configurazione = JSON.parse(readFileSync(join(cartella, "tauri.conf.json"), "utf8"));
if (android) configurazione = unisci(configurazione, JSON.parse(readFileSync(join(cartella, "tauri.android.conf.json"), "utf8")));
configurazione = sostituisci(configurazione);
// OE_BUILD, il numero della build della pipeline: la versione diventa <versione>+<numero>, che Windows mostra come 0.1.0.<numero>
const build = (process.env.OE_BUILD ?? "").trim();
if (build) {
  if (!/^[1-9][0-9]{0,4}$/.test(build) || Number(build) > 65535) fallisci(`OE_BUILD deve essere un numero da 1 a 65535: "${build}"`);
  configurazione.version = `${configurazione.version}+${build}`;
}
if (prova) {
  for (const capacita of configurazione.app.security.capabilities) capacita.permissions.push("allow-prova-menu");
}
for (const capacita of configurazione.app.security.capabilities) {
  if (capacita.permissions.some((p) => p === "core:default" || p.endsWith(":default"))) fallisci("la capability remota non ammette permessi default");
}
writeFileSync(join(cartella, "tauri.conf.local.json"), JSON.stringify(configurazione, null, 2) + "\n");
console.log(`configura: tauri.conf.local.json scritto, versione ${configurazione.version}${prova ? " (con i comandi di prova)" : ""}${android ? " (Android)" : ""}`);

if (android && release) {
  const proprieta = process.env.OE_KEYSTORE_PROPERTIES || join(homedir(), ".oe-copilot", "android", "keystore.properties");
  console.log(existsSync(proprieta) ? "configura: firma di rilascio da keystore.properties" : `configura: ${proprieta} non c'è, l'APK di rilascio resta NON firmato`);
}
