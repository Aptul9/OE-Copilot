// Chiede a un agente in ascolto (per esempio il container dell'immagine, con il calendar.db di costruisci-db.ts) le stesse cose che chiede la
// prova del golden e le confronta con le risposte registrate dell'agente Python. Uso: node test/confronta-http.ts <url> [chiave]
// Esce con 1 e dice quali risposte sono diverse.
import { leggiGolden, leggiTestoGolden } from "./ausiliari/dati.ts";
import { CHIAVE_PROVA, chiediIcs, leggiTutto } from "./ausiliari/http.ts";
import { isDeepStrictEqual } from "node:util";

const base = process.argv[2];
const chiave = process.argv[3] ?? CHIAVE_PROVA;
if (!base) {
  console.error("uso: node test/confronta-http.ts <url> [chiave]");
  process.exit(2);
}
const golden = leggiGolden("agente.json");
const diverse: string[] = [];
const controlla = (nome: string, ottenuto: unknown, atteso: unknown) => {
  if (!isDeepStrictEqual(JSON.parse(JSON.stringify(ottenuto)), atteso)) diverse.push(nome);
};

const letto = await leggiTutto(base, Object.keys(golden.eventi), chiave);
controlla("calendari", letto.calendari, golden.calendari);
for (const k of Object.keys(golden.eventi)) controlla(`eventi ${k}`, letto.eventi[k], golden.eventi[k]);

const testi: Record<string, string> = {
  festivita: leggiTestoGolden("ics", "festivita.ics"),
  riunioni: leggiTestoGolden("ics", "riunioni.ics"),
  html: leggiTestoGolden("ics", "html.ics"),
  rotto: leggiTestoGolden("ics", "rotto.ics"),
  vuoto: "",
};
let confronti = 1 + Object.keys(golden.eventi).length;
for (const [nome, registrato] of Object.entries<any>(golden.ics)) {
  if (nome === "rifiuti") continue;
  controlla(`ics ${nome} validazione`, (await chiediIcs(base, { testo: testi[nome] }, chiave)).dati, registrato.valida);
  confronti++;
  for (const k of Object.keys(registrato.eventi)) {
    const [inizio, fine] = k.split("|");
    controlla(`ics ${nome} ${k}`, (await chiediIcs(base, { testo: testi[nome], inizio, fine }, chiave)).dati, registrato.eventi[k]);
    confronti++;
  }
}
console.log(`${confronti} risposte confrontate con quelle registrate, ${diverse.length} diverse${diverse.length ? ": " + diverse.join("; ") : ""}`);
process.exit(diverse.length ? 1 : 0);
