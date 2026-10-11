import fs from "node:fs";
import { nelleDati, scriviJson } from "./dati";
import { oraStamp } from "./orologio";
import { COLOR_PALETTE, ACCOUNT_LOCALE, personalizzazione } from "./personalizzazione";
import { Dict, isDict, pyStr } from "./py";

// La configurazione di chi usa l'app (config.py): calendari spenti, alias, colori, gruppi, link ICS, tema. Sta in config.json
// nella cartella dei dati e si rilegge a ogni uso, come faceva il servizio Python: la scrivono richieste diverse.

export type Config = Dict;

export const DEFAULT_CONFIG: Config = {
  // Calendari da NON mostrare: elenco di id
  disabled_calendars: [],
  // Calendari sottoscritti da link iCal: [{id, name, url}]
  ics_calendars: [],
  // Come chiamare gli account: {"mario@azienda.it": "Azienda"}
  account_aliases: {},
  // Ogni quanti secondi la pagina rilegge i calendari
  reminder_poll_seconds: 60,
  // Colore per account: {"mario@x.com": "#2563eb"}, riempito da solo
  account_colors: {},
  // Aspetto: "auto", "light" o "dark"
  theme: "auto",
  // Zoom del calendario: altezza in pixel della riga da mezz'ora
  zoom_calendario: 58,
};

export const percorsoConfig = () => nelleDati("config.json");

export function loadConfig(): Config {
  const cfg: Config = structuredClone(DEFAULT_CONFIG);
  const percorso = percorsoConfig();
  if (fs.existsSync(percorso)) {
    try {
      const dati = JSON.parse(fs.readFileSync(percorso, "utf-8"));
      if (!isDict(dati)) throw new TypeError("cannot convert dictionary update sequence element #0 to a sequence");
      Object.assign(cfg, dati);
    } catch (e) {
      if (e instanceof SyntaxError) {
        // File illeggibile: se ne tiene una copia PRIMA di ripartire dai valori di fabbrica, se no il primo salvataggio
        // successivo lo sovrascriverebbe e le impostazioni sarebbero perse davvero.
        const guasto = `${percorso}.rotto-${oraStamp()}`;
        console.error(`configurazione illeggibile: ne salvo una copia in ${guasto} e riparto dai valori predefiniti`);
        try {
          fs.copyFileSync(percorso, guasto);
        } catch {
          console.warn("copia della configurazione rotta non riuscita");
        }
      } else if ((e as NodeJS.ErrnoException).code) console.warn("configurazione non leggibile", e);
      else throw e;
    }
  }
  // Garantisce che i sotto-dizionari esistano sempre
  for (const [k, v] of [["account_colors", {}], ["disabled_calendars", []], ["ics_calendars", []], ["account_aliases", {}]] as const) {
    if (!(k in cfg)) cfg[k] = structuredClone(v);
  }
  return cfg;
}

/** Salva la configurazione; una copia di lavoro (_sola_lettura, eventi.occupato) resta in memoria. */
export function saveConfig(cfg: Config): void {
  if (cfg._sola_lettura) return;
  scriviJson(percorsoConfig(), cfg);
}

/** Il colore stabile dell'account; se manca lo assegna (il primo libero della tavolozza, o `preferito`) e lo salva. */
export function colorForAccount(cfg: Config, username: string, preferito?: unknown): string {
  if (!isDict(cfg.account_colors)) cfg.account_colors = {};
  const colori = cfg.account_colors as Record<string, string>;
  if (!(username in colori)) {
    const usati = new Set(Object.values(colori));
    if (preferito && !usati.has(preferito as string)) colori[username] = preferito as string;
    else {
      const libero = COLOR_PALETTE.find((c) => !usati.has(c));
      colori[username] = libero ?? COLOR_PALETTE[Object.keys(colori).length % COLOR_PALETTE.length];
    }
    saveConfig(cfg);
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

/** L'ordine di un calendario nella barra laterale, per nome mostrato: chi non è in elenco va in fondo. */
export function ordineCalendario(alias: string): number {
  const ordine = personalizzazione().ordineCalendari;
  const i = ordine.indexOf(alias);
  return i === -1 ? ordine.length : i;
}

export { ACCOUNT_LOCALE };
