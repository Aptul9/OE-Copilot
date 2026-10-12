// Quello che la condivisione del calendario (lib/core/condivisione.ts, lib/condividi.ts) e la pagina che la gestisce sanno entrambe: i livelli, i
// limiti, la forma di un link come la vede la pagina. Niente di Node qui dentro: lo importa anche il codice che gira nel browser.

export const LIVELLI = ["occupato", "titoli"] as const;
export type Livello = (typeof LIVELLI)[number];
export const AMBITI = ["tutti", "gruppo"] as const;
export type Ambito = (typeof AMBITI)[number];

/** Quanti giorni prima di oggi e dopo oggi si pubblicano: i valori di partenza e i limiti che nessun link supera. */
export const INDIETRO = { min: 0, max: 30, predefinito: 7 } as const;
export const AVANTI = { min: 1, max: 365, predefinito: 90 } as const;
export const NOME_MAX = 60;
export const NOME_CALENDARIO_MAX = 80;
export const NOME_CALENDARIO_PREDEFINITO = "Calendario condiviso";
/** Quanti link, revocati compresi, ci possono essere insieme. */
export const LINK_MAX = 50;
/** Quanto si aspetta fra due letture del calendario, in minuti: lo stesso numero che il link dichiara (X-PUBLISHED-TTL). */
export const RILETTURA_MINUTI = 30;

export const AVVISO_LIVELLO: Record<Livello, string> = {
  occupato: "Occupato: chi ha il link vede solo gli orari occupati, senza titoli.",
  titoli: "Con titoli: chi ha il link vede titoli e luoghi.",
};

/** Un link di condivisione come lo vede la pagina: il testo segreto sta solo dentro `url` (vuoto quando il link è revocato). */
export type CondivisionePubblica = {
  id: string;
  nome: string;
  livello: Livello;
  ambito: Ambito;
  gruppo: string;
  giorni_indietro: number;
  giorni_avanti: number;
  nome_calendario: string;
  creato: number;
  aggiornato: number;
  ultimo_accesso: number | null;
  accessi: number;
  /** Quando è stato revocato (millisecondi), o null se vale ancora. */
  revocato: number | null;
  url: string | null;
  /** Il link è per un gruppo che non c'è più: l'indirizzo non serve niente finché non se ne sceglie un altro. */
  gruppo_mancante: boolean;
};

export type CondivisioniRisposta = { ok: true; condivisioni: CondivisionePubblica[]; max: number };
export type CondivisioneEsito = { ok: true; condivisione?: CondivisionePubblica } | { ok: false; error: string };

/** Il nome con cui il calendario si pubblica: quello scelto, o quello di partenza. */
export const nomePubblicato = (nomeCalendario: string): string => nomeCalendario || NOME_CALENDARIO_PREDEFINITO;

/** Da quanto tempo, a parole: "mai", "adesso", "12 min fa", "3 ore fa", "2 giorni fa". */
export function quandoLetto(ms: number | null, adesso: number): string {
  if (ms === null) return "mai";
  const minuti = Math.floor(Math.max(0, adesso - ms) / 60_000);
  if (minuti < 1) return "adesso";
  if (minuti < 60) return `${minuti} min fa`;
  const ore = Math.floor(minuti / 60);
  if (ore < 24) return ore === 1 ? "1 ora fa" : `${ore} ore fa`;
  const giorni = Math.floor(ore / 24);
  return giorni === 1 ? "1 giorno fa" : `${giorni} giorni fa`;
}
