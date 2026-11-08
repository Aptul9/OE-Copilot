import { ErroreValore } from "./py";
import type { SoloProve } from "./rete-sicura";
import { Semaforo } from "./semaforo";

// I collegamenti che il gateway apre verso siti che non si conoscono: i loghi (loghi.ts) e i calendari da link (ics.ts). I posti sono gli
// stessi per tutti e due, così nessuno dei due serve ad aprire socket senza limite. Qui sta anche il punto di prova di chi scrive le prove.

/** Quanti collegamenti verso siti esterni stanno aperti insieme, in tutto. */
export const POSTI_MAX = 4;
/** Chi aspetta un posto: al più CODA_MAX richieste, al più ATTESA_CODA_MS ciascuna; oltre, "Troppe richieste". */
export const CODA_MAX = 16;
export const ATTESA_CODA_MS = 5000;
export const TROPPE_RICHIESTE = "Troppe richieste, riprova tra poco";

// Uno solo per tutto il processo, anche se il modulo viene caricato due volte (come lo stato di loghi.ts). La chiave è quella di quando
// i collegamenti li apriva solo loghi.ts: chi guarda il semaforo da fuori (le prove di chi attacca il gateway) la cerca con quel nome.
const globale = globalThis as typeof globalThis & { __oeLoghiPosti?: Semaforo };
export const posti = (): Semaforo => (globale.__oeLoghiPosti ??= new Semaforo(POSTI_MAX, CODA_MAX, ATTESA_CODA_MS, () => new ErroreValore(TROPPE_RICHIESTE)));

let prove: SoloProve | undefined;

/**
 * Solo per le prove (vitest): permette a un server di prova su 127.0.0.1 di rispondere. Nessun codice del gateway la chiama, nessuna
 * variabile d'ambiente la attiva, e in produzione (NODE_ENV, fissato dalla build) solleva: senza, scaricaSicuro usa solo le regole vere.
 */
export function impostaReteDiProva(p: SoloProve | undefined): void {
  if (process.env.NODE_ENV === "production") throw new Error("impostaReteDiProva esiste solo per le prove");
  prove = p;
}

/** Quello che una prova ha messo con impostaReteDiProva: i soli punti del gateway che scaricano (loghi.ts, ics.ts) lo passano a scaricaSicuro. */
export const reteDiProva = (): SoloProve | undefined => prove;
