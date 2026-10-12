// Aiuti dei finti docker, sleep, date e python3 con cui gira lo script sincronizza_telefono.sh (vedi esegui.sh). Lo stato fra un processo e
// l'altro sta in $SUP_W/stato.json, la traccia in $SUP_W/traccia.txt.
import { appendFileSync, mkdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import path from "node:path";

export const sim = await import(new URL("../telefono-simulato.ts", import.meta.url).href);

export const W = process.env.SUP_W;
export const scenario = sim.SCENARI.find((s) => s.nome === process.env.SUP_SCENARIO);
export const radice = path.join(W, "opt", "oe-copilot");
const file = path.join(W, "stato.json");
export const traccia = path.join(W, "traccia.txt");

export const dellaFase = (v, n) => v[Math.min(n, v.length - 1)];
export const leggi = () => JSON.parse(readFileSync(file, "utf8"));
export const salva = (s) => writeFileSync(file, JSON.stringify(s));
export const scrivi = (riga) => appendFileSync(traccia, riga + "\n");

/** Il telefono e i file di PAUSA all'inizio del giro s.giro, come li prepara test/ciclo.test.ts. */
export function preparaGiro(s) {
  sim.iniziaGiro(s.telefono, dellaFase(scenario.fasi, s.giro));
  const manuale = path.join(radice, "telefono", "PAUSA");
  const schermo = path.join(radice, "stato", "schermo", "PAUSA");
  rmSync(manuale, { force: true });
  rmSync(schermo, { force: true });
  if (dellaFase(scenario.pausaManuale, s.giro)) writeFileSync(manuale, "");
  const eta = dellaFase(scenario.pausaSchermo, s.giro);
  if (eta !== null) {
    writeFileSync(schermo, "");
    utimesSync(schermo, s.orologio - eta, s.orologio - eta);
  }
}

export function inizia() {
  mkdirSync(path.join(radice, "telefono"), { recursive: true });
  mkdirSync(path.join(radice, "stato", "schermo"), { recursive: true });
  writeFileSync(traccia, "");
  const s = { giro: 0, orologio: sim.INIZIO, telefono: sim.nuovoStato(dellaFase(scenario.fasi, 0)) };
  preparaGiro(s);
  salva(s);
}
