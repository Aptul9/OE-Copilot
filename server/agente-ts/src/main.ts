// Avvio dell'agente: stesse variabili d'ambiente e stessi valori di partenza di server/agent/agent.py.
//   API_KEY   la chiave dell'API (senza, l'agente non parte)       PORTA     8180
//   FONTI     /app/fonti.json (le caselle da leggere)               ANDROID_DB  /android-calendario (la cartella di calendar.db, in sola lettura)
//   BATTITO   /stato/battito.json (il segno di vita del servizio)   ACCESSI   /stato/accessi.json (lo stato degli accessi dal rilevatore)
//   TZ        Europe/Rome

import { pyRepr } from "./pyfmt.ts";
import { caricaFonti, contestoDi, creaServer, registroSuStderr } from "./server.ts";

const zona = process.env.TZ || "Europe/Rome";
const registro = registroSuStderr(zona);

try {
  Temporal.Now.zonedDateTimeISO(zona);
  const chiave = process.env.API_KEY ?? "";
  const portaTesto = process.env.PORTA ?? "8180";
  if (!/^\s*[+-]?\d+\s*$/.test(portaTesto)) throw new Error(`invalid literal for int() with base 10: ${pyRepr(portaTesto)}`);
  const porta = Number(portaTesto.trim());
  const cartellaDb = process.env.ANDROID_DB ?? "/android-calendario";
  const ctx = contestoDi({ accessi: process.env.ACCESSI ?? "/stato/accessi.json", zona });
  // le fonti si leggono prima di controllare la chiave, come in Python: un fonti.json che manca ferma l'avvio con 1
  const fonti = caricaFonti(process.env.FONTI ?? "/app/fonti.json", cartellaDb, ctx);
  if (!chiave) {
    registro("ERROR", "API_KEY mancante: l'API resterebbe aperta a chiunque, non parto");
    process.exit(2);
  }
  const { server } = creaServer({ chiave, fonti, battito: process.env.BATTITO ?? "/stato/battito.json", zona, registro });
  server.listen(porta, "0.0.0.0", () => registro("INFO", `agente in ascolto sulla porta ${porta}, fonti: ${pyRepr(fonti.map((f) => f.nome))}`));
  const ferma = () => {
    server.close(() => process.exit(0));
    server.closeAllConnections();
  };
  process.on("SIGTERM", ferma);
  process.on("SIGINT", ferma);
} catch (e) {
  registro("ERROR", e instanceof Error ? (e.stack ?? e.message) : String(e));
  process.exit(1);
}
