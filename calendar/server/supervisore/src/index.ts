import { Guasti, esegui, preparaCartelle } from "./ciclo.js";
import { leggiConfigurazione } from "./configurazione.js";
import { orologioVero } from "./orologio.js";
import { rilevaAccessi, senzaIndirizzi } from "./rilevatore/accessi.js";
import { problemaIngressi } from "./rilevatore/ingressi.js";
import { creaTelefonoAdb } from "./telefono.js";

const log = (messaggio: string): void => console.log(`${new Date().toISOString()} ${messaggio}`);
const testo = (e: unknown): string => (e instanceof Error ? (e.stack ?? e.message) : String(e));

async function principale(): Promise<void> {
  const { configurazione: c, avvisi } = leggiConfigurazione(process.env);
  for (const avviso of avvisi) log(`avviso: ${avviso}`);
  const telefono = creaTelefonoAdb(c);
  const fermo = new AbortController();
  for (const segnale of ["SIGTERM", "SIGINT"] as const) process.on(segnale, () => fermo.abort());
  // il ciclo non deve morire: un rifiuto o un errore che nessuno ha gestito finisce nel log
  process.on("unhandledRejection", (e) => log(`rifiuto non gestito: ${testo(e)}`));
  process.on("uncaughtException", (e) => log(`errore non gestito: ${testo(e)}`));

  await preparaCartelle(c, log);
  const guasti = new Guasti(log);
  let ultimoRiepilogo = "";
  // un controllo che si rompe ogni volta scrive il suo errore una volta sola, finché dura
  let giaScritti = new Set<string>();
  const rilevatore = async (): Promise<void> => {
    const problema = await problemaIngressi(c.dati);
    if (problema === null) guasti.risolto("rilevatore");
    else guasti.segnala("rilevatore", problema);
    const scritti = new Set<string>();
    const logRilevatore = (m: string): void => {
      scritti.add(m);
      if (!giaScritti.has(m)) log(m);
    };
    const riepilogo = await rilevaAccessi({ dati: c.dati, uscita: c.accessiJson, telefono, orologio: orologioVero, soglie: c.soglie, log: logRilevatore });
    giaScritti = scritti;
    const senzaNomi = senzaIndirizzi(riepilogo);
    if (senzaNomi !== ultimoRiepilogo) log(senzaNomi);
    ultimoRiepilogo = senzaNomi;
  };
  log(`supervisore avviato: telefono ${c.telefono} (server adb ${c.adbHost}:${c.adbPorta}), un giro ogni ${c.ogni} s, tocco ogni ${c.toccoOgni} giri`);
  await esegui({ config: c, telefono, orologio: orologioVero, rilevatore, log }, fermo.signal);
  log("supervisore fermato");
}

principale().catch((e) => {
  log(`supervisore non partito: ${testo(e)}`);
  process.exit(1);
});
