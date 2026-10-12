// node stub.mjs <docker|sleep|date|python3> <argomenti...>
import { readFileSync } from "node:fs";
import path from "node:path";
import { leggi, preparaGiro, radice, salva, scenario, scrivi, sim } from "./comune.mjs";

const [nome, ...args] = process.argv.slice(2);
const s = leggi();

if (nome === "docker") {
  // docker exec oe-copilot-telefono <comando...>
  if (args[0] !== "exec" || args[1] !== "oe-copilot-telefono") {
    scrivi(`docker NON PREVISTO: ${args.join(" ")}`);
    process.exit(2);
  }
  const comando = args.slice(2);
  scrivi(`cmd ${comando.join(" ")}`);
  const r = sim.rispondi(s.telefono, comando);
  salva(s);
  process.stdout.write(r.uscita);
  process.exit(r.riuscito ? 0 : 1);
}

if (nome === "date") {
  if (args.join(" ") !== "+%s") {
    scrivi(`date NON PREVISTO: ${args.join(" ")}`);
    process.exit(2);
  }
  console.log(String(s.orologio));
  process.exit(0);
}

if (nome === "python3") {
  if (args[0] && args[0].endsWith("rileva_accessi.py")) scrivi("rilevatore");
  else scrivi(`python3 NON PREVISTO: ${args.join(" ")}`);
  process.exit(0);
}

if (nome === "sleep") {
  const n = Number(args[0]);
  if (n !== sim.OGNI) {
    scrivi(`sleep ${n}`);
    s.orologio += n;
    salva(s);
    process.exit(0);
  }
  let battito = "(assente)";
  try {
    battito = readFileSync(path.join(radice, "stato", "battito.json"), "utf8").trim();
  } catch {
    // il giro non l'ha scritto
  }
  scrivi(`battito ${battito}`);
  scrivi(`sleep ${n}`);
  s.orologio += n;
  s.giro += 1;
  if (s.giro >= scenario.giri) {
    salva(s);
    process.exit(99);
  }
  preparaGiro(s);
  salva(s);
  process.exit(0);
}

console.error(`stub sconosciuto: ${nome}`);
process.exit(2);
