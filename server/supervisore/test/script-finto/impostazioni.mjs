// Le variabili d'ambiente dello script per lo scenario, come righe export per la shell.
import { scenario, sim } from "./comune.mjs";

console.log(`export OGNI=${sim.OGNI} ACCESSI_OGNI=${scenario.accessiOgni} GOOGLE_OGNI=${scenario.googleOgni} TOCCO_OGNI=${scenario.toccoOgni}`);
