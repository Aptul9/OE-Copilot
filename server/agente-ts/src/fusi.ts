// I fusi che non sono nomi IANA, quelli che un VTIMEZONE definisce dentro il calendario, e la cache dei nomi già risolti. Come il tzp di icalendar
// la cache vive quanto il processo: un nome risolto o definito una volta resta quello finché l'agente non riparte.

/** Un fuso definito da un VTIMEZONE. `muro` è l'ora sul muro in secondi dall'epoca letta come se fosse UTC; `fold` vale 1 per la seconda delle due ore ripetute. */
export interface FusoPersonalizzato {
  /** Lo scarto dall'UTC, in secondi (utcoffset). */
  scarto(muro: number, fold: 0 | 1): number;
  /** Quanto l'ora legale aggiunge, in secondi, 0 se l'ora è standard (dst). */
  dst(muro: number, fold: 0 | 1): number;
}

/** I fusi definiti dai VTIMEZONE dei calendari letti, per nome. */
export const fusiPersonalizzati = new Map<string, FusoPersonalizzato>();

/** tzp.__tz_cache: il nome (pulito dalle barre) che si è già risolto, e il fuso a cui porta: un nome IANA o il nome di un fuso personalizzato. */
export const cacheFusi = new Map<string, string>();

/** Per le prove: un processo nuovo. */
export function reimpostaFusi(): void {
  fusiPersonalizzati.clear();
  cacheFusi.clear();
}
