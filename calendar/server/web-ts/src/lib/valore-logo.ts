// Un valore di interfaccia.loghi (personalizzazione.json) letto come lo legge il browser prima di decidere se è un file di questo sito o un
// indirizzo. Lo usano il gateway (lib/core/loghi.ts: che cosa scarica e tiene in cache) e la pagina (lib/oe/logic.ts: che cosa mostra):
// devono dare la stessa risposta, o un valore che per uno è un file e per l'altro un indirizzo passa in mezzo ("/\sito.org/x.png" per il
// browser è https://sito.org/x.png, e se il gateway lo prendesse per un file la pagina lo caricherebbe da lì).

/**
 * Il valore come lo vede l'analizzatore degli indirizzi: via i caratteri di controllo e lo spazio ai lati, via tab e a capo dentro, e la
 * barra rovesciata prima di ? e # vale una barra.
 */
export function comeLoLegge(valore: string): string {
  let da = 0;
  let a = valore.length;
  while (da < a && valore.charCodeAt(da) <= 0x20) da++;
  while (a > da && valore.charCodeAt(a - 1) <= 0x20) a--;
  const v = valore.slice(da, a).replace(/[\t\n\r]/g, "");
  const fine = v.search(/[?#]/);
  return fine < 0 ? v.replace(/\\/g, "/") : v.slice(0, fine).replace(/\\/g, "/") + v.slice(fine);
}

/** Un valore, già letto, che non sta nella cartella dei loghi di questo sito: comincia con // o con uno schema (https:, data:, javascript:...). */
export const portaFuori = (letto: string): boolean => letto.startsWith("//") || /^[a-z][a-z0-9+.-]*:/i.test(letto);
