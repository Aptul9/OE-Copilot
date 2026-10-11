import { type Config, modificaConfig } from "./archivio";
import { COLOR_PALETTE, ACCOUNT_LOCALE, personalizzazione } from "./personalizzazione";
import { isDict, pyStr } from "./py";

// La configurazione di chi usa l'app (era config.py, fino al commit 142e789): calendari spenti, alias, colori, gruppi, link ICS, tema. Sta nel database dello stato
// (archivio.ts); qui le regole che la leggono e la cambiano.

export { DEFAULT_CONFIG, loadConfig, modificaConfig, percorsoConfig, saveConfig, type Config } from "./archivio";

/** La tavolozza: il primo colore libero, o `preferito` se nessuno lo usa; finite le tinte si ricomincia. */
function assegnaColore(colori: Record<string, string>, username: string, preferito?: unknown): void {
  const usati = new Set(Object.values(colori));
  if (preferito && !usati.has(preferito as string)) colori[username] = preferito as string;
  else {
    const libero = COLOR_PALETTE.find((c) => !usati.has(c));
    colori[username] = libero ?? COLOR_PALETTE[Object.keys(colori).length % COLOR_PALETTE.length];
  }
}

/**
 * Il colore stabile dell'account; se manca lo assegna (il primo libero della tavolozza, o `preferito`) e lo salva. Si assegna sullo
 * stato di adesso, non su quello che chi chiede si è letto qualche istante fa: una richiesta lenta non cancella i colori (né gli
 * interruttori) assegnati o cambiati nel frattempo.
 */
export function colorForAccount(cfg: Config, username: string, preferito?: unknown): string {
  if (!isDict(cfg.account_colors)) cfg.account_colors = {};
  const colori = cfg.account_colors as Record<string, string>;
  if (!(username in colori)) {
    if (cfg._sola_lettura) assegnaColore(colori, username, preferito); // copia di lavoro: resta in memoria
    else {
      modificaConfig((vivo) => {
        if (!isDict(vivo.account_colors)) vivo.account_colors = {};
        const vivi = vivo.account_colors as Record<string, string>;
        if (!(username in vivi)) assegnaColore(vivi, username, preferito);
        Object.assign(colori, vivi);
      });
    }
  }
  return colori[username];
}

function nomePredefinito(indirizzo: string): string {
  const p = personalizzazione();
  const ind = indirizzo.toLowerCase();
  if (p.nomiPerIndirizzo.has(ind)) return pyStr(p.nomiPerIndirizzo.get(ind));
  const dominio = ind.split("@").pop() as string;
  if (p.nomiPerDominio.has(dominio)) return pyStr(p.nomiPerDominio.get(dominio));
  // Dominio sconosciuto: nome senza estensione, iniziale maiuscola
  const radice = dominio.split(".")[0];
  return radice ? radice.slice(0, 1).toUpperCase() + radice.slice(1) : indirizzo;
}

export const nomePredefinitoDi = nomePredefinito;

/** Mappa indirizzo -> nome da mostrare. Gli alias scelti a mano vincono. */
export function aliasPerAccount(cfg: Config, indirizzi: Iterable<string>): Record<string, string> {
  const scelti = isDict(cfg.account_aliases) ? (cfg.account_aliases as Record<string, unknown>) : {};
  const out: Record<string, string> = {};
  for (const ind of indirizzi) {
    const scelto = scelti[ind];
    const manuale = scelto ? pyStr(scelto).trim() : "";
    out[ind] = manuale || nomePredefinito(ind);
  }
  return out;
}

/** Le aziende di un gruppo (config "gruppi_calendari"), o null se il gruppo non esiste. */
export function membriGruppo(cfg: Config, nome: string): Set<string> | null {
  const gruppi = cfg.gruppi_calendari;
  const membri = isDict(gruppi) ? gruppi[nome] : undefined;
  return Array.isArray(membri) ? new Set(membri.map(pyStr)) : null;
}

/** Un'azienda che cambia nome resta nei suoi gruppi (senza doppioni se il nuovo nome c'era già). */
export function rinominaNeiGruppi(cfg: Config, prima: string, dopo: string): void {
  const gruppi = cfg.gruppi_calendari;
  if (prima === dopo || !isDict(gruppi)) return;
  for (const [nome, membri] of Object.entries(gruppi)) {
    if (Array.isArray(membri) && membri.includes(prima)) gruppi[nome] = [...new Set(membri.map((m) => (m === prima ? dopo : m)))];
  }
}

/** Accende o spegne un calendario: l'elenco di quelli spenti (disabled_calendars) resta ordinato. */
export function impostaAcceso(cfg: Config, id: string, acceso: boolean): void {
  const spenti = new Set<string>(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  if (acceso) spenti.delete(id);
  else spenti.add(id);
  cfg.disabled_calendars = [...spenti].sort(confrontaPunti);
}

/** sorted() di Python sui testi: per punto di codice, non per unità UTF-16 come fa sort() di JavaScript. */
export function confrontaPunti(a: string, b: string): number {
  const [pa, pb] = [Array.from(a), Array.from(b)];
  for (let i = 0; i < Math.min(pa.length, pb.length); i++) {
    const d = (pa[i].codePointAt(0) as number) - (pb[i].codePointAt(0) as number);
    if (d) return d;
  }
  return pa.length - pb.length;
}

/** L'ordine di un calendario nella barra laterale, per nome mostrato: chi non è in elenco va in fondo. */
export function ordineCalendario(alias: string): number {
  const ordine = personalizzazione().ordineCalendari;
  const i = ordine.indexOf(alias);
  return i === -1 ? ordine.length : i;
}

export { ACCOUNT_LOCALE };
