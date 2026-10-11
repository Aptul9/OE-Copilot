import fs from "node:fs";
import { nelleDati, leggiTesto } from "./dati";
import { Dict, elencoTesti, isDict, minuscole, oppure, pyBool, pyStr } from "./py";

// Nomi, domini e caselle di chi usa l'app non stanno nel codice ma in personalizzazione.json (config.py): qui si leggono e si
// derivano le stesse costanti. Senza il file l'app funziona lo stesso e i calendari prendono il nome dal dominio della casella.

export const ACCOUNT_LOCALE = "Eventi personali";
export const FESTIVITA_URL = "https://calendar.google.com/calendar/ical/it.italian%23holiday%40group.v.calendar.google.com/public/basic.ics";
export const FESTIVITA_ID = "builtin::festivita-it";
export const FESTIVITA_NOME = "Festività";
export const COLORE_FESTIVITA = "#8a8a8a";

/** Tinte assegnate agli account, in quest'ordine: più voci del numero degli account, se no due caselle hanno lo stesso colore. */
export const COLOR_PALETTE = [
  "#2563eb", "#16a34a", "#db2777", "#ea580c", "#7c3aed", "#0891b2", "#ca8a04", "#dc2626", "#4d7c0f", "#9333ea",
  "#0f766e", "#b45309", "#1e40af", "#be123c", "#525252", "#65a30d", "#0d9488", "#c026d3", "#f59e0b", "#6366f1",
  "#e11d48", "#15803d", "#a16207", "#7e22ce", "#0369a1",
];

export type Inoltrata = { via: string; prefisso: RegExp; colore: unknown };

export type Personalizzazione = {
  nomiPerIndirizzo: Map<string, unknown>;
  nomiPerDominio: Map<string, unknown>;
  casellediColleghi: Set<string>;
  ordineCalendari: string[];
  dominiAziendali: Set<string>;
  aziendaPerCartella: [string, unknown][];
  aziendeInoltrate: Map<string, Inoltrata>;
  accountNascosti: Set<string>;
  mostraTuttoIlGiorno: boolean;
  interfaccia: Dict;
};

/** Un'espressione regolare di Python per JavaScript: i nomi dei gruppi e le opzioni in testa; il resto è uguale. */
export function regexDiPython(sorgente: string): RegExp {
  let flag = "i";
  let src = sorgente.replace(/\(\?P</g, "(?<").replace(/\(\?P=(\w+)\)/g, "\\k<$1>");
  const inline = /^\(\?([aiLmsux]+)\)/.exec(src);
  if (inline) {
    src = src.slice(inline[0].length);
    if (inline[1].includes("m")) flag += "m";
    if (inline[1].includes("s")) flag += "s";
  }
  // re.match: la ricerca parte dall'inizio del testo
  return new RegExp(`^(?:${src})`, flag);
}

function leggi(percorso: string): Dict {
  let testo: string | null;
  try {
    testo = leggiTesto(percorso);
  } catch (e) {
    console.error("personalizzazione non leggibile, uso i valori predefiniti", e);
    return {};
  }
  if (testo === null) return {};
  try {
    const dati = JSON.parse(testo);
    if (isDict(dati)) return dati;
    console.error("personalizzazione: atteso un oggetto JSON, file ignorato");
  } catch (e) {
    // Un file rotto non deve fermare l'app: si parte senza personalizzazione, ma lo si scrive nel log
    console.error("personalizzazione non leggibile, uso i valori predefiniti", e);
  }
  return {};
}

function derivate(p: Dict): Personalizzazione {
  const mappa = (v: unknown) => new Map(Object.entries(isDict(v) ? v : {}).map(([k, x]) => [pyStr(k).trim().toLowerCase(), x] as const));
  const inoltrate = new Map<string, Inoltrata>();
  for (const [nome, d] of Object.entries(isDict(p.aziende_inoltrate) ? p.aziende_inoltrate : {})) {
    if (nome.startsWith("_")) continue;
    try {
      if (!isDict(d)) throw new TypeError("voce non valida");
      if (!("via" in d) || !("prefisso" in d)) throw new Error("manca via o prefisso");
      inoltrate.set(nome, { via: pyStr(d.via), prefisso: regexDiPython(String(d.prefisso)), colore: d.colore ?? null });
    } catch (e) {
      console.error(`personalizzazione: azienda inoltrata ${JSON.stringify(nome)} non valida, ignorata`, e);
    }
  }
  return {
    nomiPerIndirizzo: mappa(p.nomi_per_indirizzo),
    nomiPerDominio: mappa(p.nomi_per_dominio),
    casellediColleghi: minuscole(p.caselle_di_colleghi),
    ordineCalendari: elencoTesti(oppure(p.ordine_calendari, ["Eventi personali", "Festività"])),
    dominiAziendali: minuscole(p.domini_aziendali),
    aziendaPerCartella: Object.entries(isDict(p.azienda_per_cartella) ? p.azienda_per_cartella : {}),
    aziendeInoltrate: inoltrate,
    accountNascosti: minuscole(p.account_nascosti),
    mostraTuttoIlGiorno: pyBool(p.mostra_tutto_il_giorno),
    interfaccia: isDict(p.interfaccia) ? { ...p.interfaccia } : {},
  };
}

const memoria = new Map<string, { firma: string; valore: Personalizzazione }>();

/**
 * La personalizzazione: si rilegge solo se il file è cambiato (data e dimensione), così una modifica vale subito, senza il
 * riavvio che chiedeva il servizio Python (che la leggeva una volta sola, all'avvio).
 */
export function personalizzazione(): Personalizzazione {
  const percorso = process.env.OE_PERSONALIZZAZIONE || nelleDati("personalizzazione.json");
  let firma = "assente";
  try {
    const s = fs.statSync(percorso);
    firma = `${s.mtimeMs}:${s.size}`;
  } catch {
    // non c'è: valori predefiniti
  }
  const nota = memoria.get(percorso);
  if (nota && nota.firma === firma) return nota.valore;
  const valore = derivate(firma === "assente" ? {} : leggi(percorso));
  memoria.set(percorso, { firma, valore });
  return valore;
}
