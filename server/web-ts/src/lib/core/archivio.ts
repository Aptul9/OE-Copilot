import fs from "node:fs";
import type Database from "better-sqlite3";
import { config } from "../config";
import { openDb } from "../db";
import { nelleDati, scriviJson } from "./dati";
import { oraStamp } from "./orologio";
import { type Dict, isDict, pyStr } from "./py";

// Lo stato di chi usa l'app: la configurazione (config.json) e gli eventi personali (local_events.json) stanno in gateway.db, nello
// stesso database delle sessioni. Una volta sola, alla prima apertura, si importano i due file di prima; da lì in poi il database è la
// fonte e ogni modifica è una transazione (il servizio Python rileggeva e riscriveva il file a ogni richiesta, senza blocchi).
// I due file si riscrivono dopo ogni modifica, nel formato di sempre: servono a tornare a una versione che li leggeva (fino al commit
// 142e789 c'era il servizio Python ui) senza perdere niente, e a correggere lo stato a mano (GUIDA.md, sezione 7). Non si rileggono mai,
// salvo nell'importazione.

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
export const percorsoLocali = () => nelleDati("local_events.json");

const SCHEMA = `
  CREATE TABLE IF NOT EXISTS stato_config (chiave TEXT PRIMARY KEY, valore TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS eventi_locali (n INTEGER PRIMARY KEY AUTOINCREMENT, dati TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS stato_meta (chiave TEXT PRIMARY KEY, valore TEXT NOT NULL);
`;

// ---- La connessione ------------------------------------------------------------------------------------------------------
// In globalThis: le rotte di Next hanno ciascuna la sua copia dei moduli, la connessione è una sola. Si riapre se il percorso cambia
// (lo fanno le prove, una cartella dei dati per prova).
type Aperto = { file: string; db: Database.Database };
const globale = globalThis as typeof globalThis & { __oeArchivio?: Aperto };

/** Il database dello stato, aperto e pronto: lo schema c'è e i file di prima sono stati importati. */
export function archivio(): Database.Database {
  const file = config.dbStato;
  const aperto = globale.__oeArchivio;
  if (aperto?.file === file) return aperto.db;
  aperto?.db.close();
  globale.__oeArchivio = undefined;
  const db = openDb(file);
  try {
    db.exec(SCHEMA);
    importaUnaVolta(db);
  } catch (e) {
    db.close();
    throw e;
  }
  globale.__oeArchivio = { file, db };
  return db;
}

/** Chiude il database (per le prove). */
export function chiudiArchivio(): void {
  globale.__oeArchivio?.db.close();
  globale.__oeArchivio = undefined;
}

// ---- L'importazione dei file di prima -------------------------------------------------------------------------------------
/** Una copia del file illeggibile, PRIMA di ripartire da zero: se no la prima copia su file lo sovrascriverebbe e il contenuto andrebbe perso. */
function copiaDelGuasto(percorso: string): void {
  const guasto = `${percorso}.rotto-${oraStamp()}`;
  try {
    fs.copyFileSync(percorso, guasto);
    console.error(`archivio: ${percorso} illeggibile, ne salvo una copia in ${guasto} e riparto da zero`);
  } catch {
    console.warn(`archivio: copia di ${percorso} non riuscita`);
  }
}

function configDiPrima(): Dict {
  const percorso = percorsoConfig();
  if (!fs.existsSync(percorso)) return {};
  try {
    const dati = JSON.parse(fs.readFileSync(percorso, "utf-8"));
    if (!isDict(dati)) throw new TypeError("cannot convert dictionary update sequence element #0 to a sequence");
    return dati;
  } catch (e) {
    if (e instanceof SyntaxError) {
      copiaDelGuasto(percorso);
      return {};
    }
    if ((e as NodeJS.ErrnoException).code) {
      console.warn("configurazione non leggibile", e);
      return {};
    }
    throw e;
  }
}

function localiDiPrima(): unknown[] {
  const percorso = percorsoLocali();
  let testo: string;
  try {
    testo = fs.readFileSync(percorso, "utf-8");
  } catch (e) {
    // La prima volta: nessun evento personale, è normale. Il resto no: sono dati che esistono solo qui.
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") console.error(`eventi personali: file non leggibile (${percorso})`, e);
    return [];
  }
  try {
    const dati = JSON.parse(testo);
    return Array.isArray(dati) ? dati : [];
  } catch (e) {
    console.error(`eventi personali: file non leggibile (${percorso})`, e);
    copiaDelGuasto(percorso);
    return [];
  }
}

function importaUnaVolta(db: Database.Database): void {
  if (db.prepare("SELECT 1 FROM stato_meta WHERE chiave = 'importato'").get()) return;
  const cfg = configDiPrima();
  const locali = localiDiPrima();
  db.transaction(() => {
    scriviRigheConfig(db, cfg);
    const inserisci = db.prepare("INSERT INTO eventi_locali(dati) VALUES (?)");
    for (const e of locali) inserisci.run(JSON.stringify(e));
    db.prepare("INSERT INTO stato_meta(chiave, valore) VALUES ('importato', ?)").run(oraStamp());
  }).immediate();
  console.info(`archivio: importati da config.json ${Object.keys(cfg).length} voci e da local_events.json ${locali.length} eventi`);
}

// ---- Transazioni e file di copia -----------------------------------------------------------------------------------------
let profondita = 0;
const sporco = { config: false, locali: false };

/** Riscrive config.json e local_events.json se la transazione appena finita li ha cambiati: stesso contenuto e formato di quando li scriveva Python. */
function copiaSuFile(): void {
  if (sporco.config) {
    sporco.config = false;
    try {
      scriviJson(percorsoConfig(), loadConfig());
    } catch (e) {
      console.warn("archivio: config.json non riscritto", e);
    }
  }
  if (sporco.locali) {
    sporco.locali = false;
    try {
      scriviJson(percorsoLocali(), leggiRigheLocali());
    } catch (e) {
      console.warn("archivio: local_events.json non riscritto", e);
    }
  }
}

/**
 * Una transazione sull'archivio (BEGIN IMMEDIATE): quello che `fn` legge e scrive è un blocco solo, e se solleva non cambia niente.
 * Se ne possono annidare; a fine della più esterna si riscrivono i file di copia.
 */
export function transazione<T>(fn: () => T): T {
  const db = archivio();
  profondita++;
  let esito: T;
  try {
    esito = db.transaction(fn).immediate();
  } catch (e) {
    if (profondita === 1) sporco.config = sporco.locali = false;
    throw e;
  } finally {
    profondita--;
  }
  if (profondita === 0) copiaSuFile();
  return esito;
}

// ---- La configurazione ---------------------------------------------------------------------------------------------------
function leggiRigheConfig(): Dict {
  const righe = archivio().prepare("SELECT chiave, valore FROM stato_config ORDER BY rowid").all() as { chiave: string; valore: string }[];
  return Object.fromEntries(righe.map((r) => [r.chiave, JSON.parse(r.valore)]));
}

/** Rende le righe uguali a `cfg`: quelle che ci sono si aggiornano, quelle che non ci sono più si tolgono. */
function scriviRigheConfig(db: Database.Database, cfg: Dict): void {
  const voci = Object.entries(cfg).filter(([k, v]) => k !== "_sola_lettura" && v !== undefined);
  const scrivi = db.prepare("INSERT INTO stato_config(chiave, valore) VALUES (?, ?) ON CONFLICT(chiave) DO UPDATE SET valore = excluded.valore");
  for (const [k, v] of voci) scrivi.run(k, JSON.stringify(v));
  const restano = new Set(voci.map(([k]) => k));
  const togli = db.prepare("DELETE FROM stato_config WHERE chiave = ?");
  for (const r of db.prepare("SELECT chiave FROM stato_config").all() as { chiave: string }[]) if (!restano.has(r.chiave)) togli.run(r.chiave);
}

/** La configurazione: i valori predefiniti con sopra quelli salvati. */
export function loadConfig(): Config {
  const cfg: Config = structuredClone(DEFAULT_CONFIG);
  Object.assign(cfg, leggiRigheConfig());
  // Garantisce che i sotto-dizionari esistano sempre
  for (const [k, v] of [["account_colors", {}], ["disabled_calendars", []], ["ics_calendars", []], ["account_aliases", {}]] as const) {
    if (!(k in cfg)) cfg[k] = structuredClone(v);
  }
  return cfg;
}

/** Salva la configurazione com'è; una copia di lavoro (_sola_lettura, eventi.occupato) resta in memoria. */
export function saveConfig(cfg: Config): void {
  if (cfg._sola_lettura) return;
  transazione(() => {
    scriviRigheConfig(archivio(), cfg);
    sporco.config = true;
  });
}

/**
 * Rilegge la configurazione, la cambia con `modifica` e la salva, tutto in un blocco: due richieste che arrivano insieme non si perdono
 * le modifiche (con il file succedeva, la configurazione si leggeva all'inizio e si scriveva alla fine di una richiesta lenta).
 */
export function modificaConfig<T>(modifica: (cfg: Config) => T): T {
  return transazione(() => {
    const cfg = loadConfig();
    const esito = modifica(cfg);
    scriviRigheConfig(archivio(), cfg);
    sporco.config = true;
    return esito;
  });
}

// ---- Gli eventi personali -----------------------------------------------------------------------------------------------
/** Gli eventi personali, nell'ordine in cui sono stati aggiunti. */
export function leggiRigheLocali(): Dict[] {
  const righe = archivio().prepare("SELECT dati FROM eventi_locali ORDER BY n").all() as { dati: string }[];
  return righe.map((r) => JSON.parse(r.dati));
}

/** Aggiunge in fondo l'evento che `crea` costruisce vedendo quelli che ci sono (serve per l'identificativo). */
export function aggiungiLocale(crea: (esistenti: Dict[]) => Dict): void {
  transazione(() => {
    const evento = crea(leggiRigheLocali());
    archivio().prepare("INSERT INTO eventi_locali(dati) VALUES (?)").run(JSON.stringify(evento));
    sporco.locali = true;
  });
}

/** Toglie gli eventi con quell'identificativo (str(uid) come in Python). Il file di copia si riscrive anche se non ne manca nessuno. */
export function togliLocali(uid: unknown): void {
  transazione(() => {
    const db = archivio();
    const togli = db.prepare("DELETE FROM eventi_locali WHERE n = ?");
    for (const r of db.prepare("SELECT n, dati FROM eventi_locali").all() as { n: number; dati: string }[]) {
      if (pyStr((JSON.parse(r.dati) as Dict | null)?.uid) === pyStr(uid)) togli.run(r.n);
    }
    sporco.locali = true;
  });
}
