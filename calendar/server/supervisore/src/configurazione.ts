import path from "node:path";

// Le variabili d'ambiente e i valori di partenza sono quelli del vecchio script di sincronizzazione e del rilevatore in Python (tolti dal repository). Un valore che non
// si legge non ferma il servizio: vale quello di partenza e l'avviso finisce nel log (lo script e il rilevatore si fermavano o
// davano un errore di aritmetica).

export interface Soglie {
  outlookGialloS: number;
  outlookRossoS: number;
  /** Una riga sulle push più vecchia di così non dice niente. */
  pushRecenteS: number;
  googleGialloS: number;
  googleRossoS: number;
}

export interface Configurazione {
  /** Indirizzo adb del telefono (host:porta), come per il servizio schermo. */
  telefono: string;
  adbHost: string;
  adbPorta: number;
  /** Secondi di pausa fra un giro e il successivo (OGNI). */
  ogni: number;
  pausaManuale: string;
  statoDir: string;
  pausaSchermo: string;
  pausaSchermoScade: number;
  accessiOgni: number;
  googleOgni: number;
  /** Ogni quanti giri un tocco sulla barra di stato tiene sveglio il telefono, 0 = mai. */
  toccoOgni: number;
  /** Radice dei dati del telefono (la cartella /data di Android sul server). */
  dati: string;
  accessiJson: string;
  soglie: Soglie;
  timeoutComandoMs: number;
}

export interface LetturaConfigurazione {
  configurazione: Configurazione;
  avvisi: string[];
}

function intero(env: NodeJS.ProcessEnv, nome: string, predefinito: number, minimo: number, avvisi: string[]): number {
  const grezzo = env[nome];
  if (grezzo === undefined || grezzo === "") return predefinito;
  if (/^\s*[+-]?\d+\s*$/.test(grezzo)) {
    const n = Number.parseInt(grezzo, 10);
    if (n >= minimo) return n;
  }
  avvisi.push(`${nome}=${JSON.stringify(grezzo)} non è un intero da ${minimo} in su: vale ${predefinito}`);
  return predefinito;
}

function secondi(env: NodeJS.ProcessEnv, nome: string, predefinito: number, avvisi: string[]): number {
  const grezzo = env[nome];
  if (grezzo === undefined || grezzo === "") return predefinito;
  const n = Number(grezzo);
  if (Number.isFinite(n) && n >= 0 && grezzo.trim() !== "") return n;
  avvisi.push(`${nome}=${JSON.stringify(grezzo)} non è un numero di secondi: vale ${predefinito}`);
  return predefinito;
}

export function leggiConfigurazione(env: NodeJS.ProcessEnv): LetturaConfigurazione {
  const avvisi: string[] = [];
  const radice = env.OE_DIR || "/opt/oe-copilot";
  const statoDir = path.join(radice, "stato");
  const porta = intero(env, "ADB_PORT", 5037, 1, avvisi);
  const configurazione: Configurazione = {
    telefono: env.TELEFONO || "telefono:5555",
    adbHost: env.ADB_HOST || "127.0.0.1",
    adbPorta: porta,
    ogni: secondi(env, "OGNI", 30, avvisi),
    pausaManuale: path.join(radice, "telefono", "PAUSA"),
    statoDir,
    pausaSchermo: path.join(statoDir, "schermo", "PAUSA"),
    pausaSchermoScade: intero(env, "PAUSA_SCHERMO_SCADE", 60, 0, avvisi),
    accessiOgni: intero(env, "ACCESSI_OGNI", 2, 1, avvisi),
    googleOgni: intero(env, "GOOGLE_OGNI", 4, 1, avvisi),
    toccoOgni: intero(env, "TOCCO_OGNI", 10, 0, avvisi),
    dati: path.join(radice, "telefono", "data"),
    accessiJson: path.join(statoDir, "accessi.json"),
    soglie: {
      outlookGialloS: intero(env, "OUTLOOK_GIALLO_S", 600, 0, avvisi),
      outlookRossoS: intero(env, "OUTLOOK_ROSSO_S", 1800, 0, avvisi),
      pushRecenteS: 600,
      googleGialloS: intero(env, "GOOGLE_GIALLO_S", 900, 0, avvisi),
      googleRossoS: intero(env, "GOOGLE_ROSSO_S", 2700, 0, avvisi),
    },
    timeoutComandoMs: secondi(env, "ADB_TIMEOUT_S", 30, avvisi) * 1000,
  };
  return { configurazione, avvisi };
}
