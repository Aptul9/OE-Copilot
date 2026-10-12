import path from "node:path";
import type { Soglie } from "../configurazione.js";
import type { Orologio } from "../orologio.js";
import type { Telefono } from "../telefono.js";

export type Stato = "ok" | "avviso" | "errore";

/** Il verdetto per una casella: uno stato e un messaggio (vuoto se va tutto bene). */
export interface Verdetto {
  stato: Stato;
  msg: string;
}

/** Tutto quello che il rilevatore usa: le cartelle, il telefono, l'ora, le soglie. Le prove lo costruiscono con dati finti. */
export interface ContestoRilevatore {
  /** Radice dei dati del telefono (/opt/oe-copilot/telefono/data). */
  dati: string;
  /** Dove scrivere accessi.json: accanto stanno battito.json e google-sync.json. */
  uscita: string;
  telefono: Telefono;
  orologio: Orologio;
  soglie: Soglie;
  log: (messaggio: string) => void;
}

const OUTLOOK = "data/com.microsoft.office.outlook";

export const dbOutlook = (c: Pick<ContestoRilevatore, "dati">): string => path.join(c.dati, OUTLOOK, "databases/acompliAcct.db");
export const logOutlook = (c: Pick<ContestoRilevatore, "dati">): string => path.join(c.dati, OUTLOOK, "app_logs");
export const cartellaStato = (c: ContestoRilevatore): string => path.dirname(c.uscita);
export const battitoJson = (c: ContestoRilevatore): string => path.join(cartellaStato(c), "battito.json");
export const googleStato = (c: ContestoRilevatore): string => path.join(cartellaStato(c), "google-sync.json");

/** Secondi dell'ora di adesso: l'orologio dà millisecondi, il rilevatore ragiona in secondi con la frazione come time.time(). */
export const secondiAdesso = (c: ContestoRilevatore): number => c.orologio.adesso() / 1000;
