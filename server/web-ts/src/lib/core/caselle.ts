import fs from "node:fs";
import { nelleDati, scriviJson } from "./dati";
import { secondi } from "./orologio";
import { personalizzazione } from "./personalizzazione";
import { aliasPerAccount, colorForAccount, nomePredefinitoDi, type Config } from "./stato";
import { isDict, pyStr } from "./py";

// Regole comuni alle caselle lette dall'agente (caselle.py): di che azienda è una riunione, quale copia tenere fra due uguali, che
// colore darle. Non dipendono da dove arrivano gli eventi.

/** Un evento come lo manda get_events: i campi li decide l'agente e questo codice, la pagina li legge per nome. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Ev = Record<string, any>;

const MAX_INTERVALLI_IN_CACHE = 12;

// ---- Copia su disco degli eventi ---------------------------------------------------------------------------------------
// Permette di mostrare il calendario subito anche con l'agente irraggiungibile, invece di una griglia vuota.
type Cache = Record<string, { stamp?: number; eventi?: Ev[] }>;

export function leggiCacheEventi(): Cache {
  try {
    const dati = JSON.parse(fs.readFileSync(nelleDati("events_cache.json"), "utf-8"));
    return isDict(dati) ? (dati as Cache) : {};
  } catch {
    return {};
  }
}

export function scriviCacheEventi(chiave: string, eventi: Ev[]): void {
  let dati = leggiCacheEventi();
  dati[chiave] = { stamp: secondi(), eventi };
  if (Object.keys(dati).length > MAX_INTERVALLI_IN_CACHE) {
    // Tiene i più recenti: navigando tra le settimane le chiavi crescono
    const recenti = Object.entries(dati).sort((a, b) => (b[1].stamp ?? 0) - (a[1].stamp ?? 0));
    dati = Object.fromEntries(recenti.slice(0, MAX_INTERVALLI_IN_CACHE));
  }
  try {
    scriviJson(nelleDati("events_cache.json"), dati);
  } catch {
    // la copia è un'ottimizzazione
  }
}

// ---- Di che azienda è un appuntamento ------------------------------------------------------------------------------------
// Prefisso che il programma di posta mette davanti al titolo di un invito inoltrato (FW/Fwd, I, WG, TR, RV), anche ripetuto.
export const INOLTRO_RE = /^\s*(?:(?:fwd?|i|wg|tr|rv)\s*:\s*)+/i;

/** (azienda, titolo senza prefisso) per le copie dei flussi, se no (null, titolo). */
export function aziendaDaTitolo(titolo: string): [string | null, string] {
  for (const [azienda, d] of personalizzazione().aziendeInoltrate) {
    const m = d.prefisso.exec(titolo || "");
    if (m) return [azienda, titolo.slice(m[0].length).trim() || "(senza titolo)"];
  }
  return [null, titolo];
}

/** Azienda dedotta dal nome della cartella calendario, o null. */
export function aziendaDaCartella(labelONome: unknown): string | null {
  if (!labelONome) return null;
  const testo = String(labelONome).toLowerCase();
  for (const [nome, azienda] of personalizzazione().aziendaPerCartella) if (testo.includes(nome.toLowerCase())) return pyStr(azienda);
  return null;
}

/** Nome azienda dedotto dall'indirizzo dell'organizzatore: solo per i domini aziendali, mai per i provider generici. */
export function aziendaDaEmail(email: unknown): string | null {
  if (!email || !String(email).includes("@")) return null;
  const p = personalizzazione();
  const ind = String(email).toLowerCase().trim();
  const dominio = ind.split("@").pop() as string;
  if (!p.dominiAziendali.has(dominio)) return null;
  if (p.nomiPerIndirizzo.has(ind) && !p.casellediColleghi.has(ind)) return pyStr(p.nomiPerIndirizzo.get(ind));
  if (p.nomiPerDominio.has(dominio)) return pyStr(p.nomiPerDominio.get(dominio));
  // Dominio aziendale senza nome esplicito: il nome del dominio, mai quello di un'altra azienda per ripiego
  return nomePredefinitoDi(ind);
}

/**
 * Aggiunge `ev` a `events`, riconoscendo le copie e scartando i doppioni. `seen` è l'indice (chiave -> posizione) che chi
 * legge tiene per tutta la lettura; `invitato` dice quale casella dell'utente era fra gli invitati, e si chiama solo se serve.
 */
export function accoda(events: Ev[], seen: Map<string, number>, ev: Ev, etichettaCalendario: string, invitato: () => string): void {
  const [inoltrata, titolo] = aziendaDaTitolo(ev.title);
  if (inoltrata) {
    ev.title = titolo;
    ev.inoltrata_da = inoltrata;
  }
  // Se né la cartella né l'organizzatore dicono di che azienda è, si guarda a quale casella era indirizzato l'invito
  if (!(inoltrata || aziendaDaCartella(etichettaCalendario) || aziendaDaEmail(ev.organizer_email))) ev.invitato_come = invitato();
  // Un invito inoltrato ("FW: ...") e la stessa riunione letta dal calendario di chi l'ha organizzata sono una riunione sola
  const titoloPulito = ev.title.replace(INOLTRO_RE, "") || ev.title;
  const key = JSON.stringify([ev.account, titoloPulito, ev.start, ev.end]);
  const posto = seen.get(key);
  if (posto !== undefined && !inoltrata && !events[posto].inoltrata_da) {
    const prima = events[posto];
    if (INOLTRO_RE.test(prima.title) && titoloPulito === ev.title) {
      ev.online_url = ev.online_url || prima.online_url;
      events[posto] = ev;
    } else prima.online_url = prima.online_url || ev.online_url;
    return;
  }
  if (posto !== undefined) {
    // La stessa riunione come invito vero e come copia del flusso: vince l'invito, che ha il link e l'organizzatore reale
    if (events[posto].inoltrata_da && !inoltrata) events[posto] = ev;
    return;
  }
  seen.set(key, events.length);
  events.push(ev);
}

/** Dà a ogni evento l'azienda (alias) e il colore con cui va mostrato. */
export function etichetta(cfg: Config, events: Ev[]): void {
  const p = personalizzazione();
  const alias = aliasPerAccount(cfg, new Set(events.map((e) => e.account)));
  // Colore per azienda ricavato dagli account già colorati: un evento riclassificato su un'azienda riusa il colore della sua casella
  const coloriNoti = (isDict(cfg.account_colors) ? cfg.account_colors : {}) as Record<string, string>;
  const aliasNoti = aliasPerAccount(cfg, Object.keys(coloriNoti));
  const coloreAzienda = new Map<string, string>();
  for (const [acc, col] of Object.entries(coloriNoti)) if (!coloreAzienda.has(aliasNoti[acc] ?? acc)) coloreAzienda.set(aliasNoti[acc] ?? acc, col);
  for (const e of events) if (!coloreAzienda.has(alias[e.account] ?? e.account)) coloreAzienda.set(alias[e.account] ?? e.account, e.color);
  // Le aziende inoltrate non hanno una casella: il colore va assegnato qui
  for (const [azienda, d] of p.aziendeInoltrate) coloreAzienda.set(azienda, colorForAccount(cfg, azienda, d.colore));
  for (const e of events) {
    e.alias = alias[e.account] ?? e.account;
    if (p.casellediColleghi.has(String(e.account).toLowerCase())) continue; // calendario di un collega: i suoi eventi restano suoi
    // Priorità: la copia di un flusso, poi una cartella dedicata a un'azienda, poi chi organizza, poi a chi era indirizzato l'invito
    const az = e.inoltrata_da || aziendaDaCartella(e.calendar) || aziendaDaEmail(e.organizer_email) || aziendaDaEmail(e.invitato_come);
    if (az && az !== e.alias) {
      e.alias = az;
      if (coloreAzienda.has(az)) e.color = coloreAzienda.get(az);
    }
  }
}

// ---- Aziende copiate da un flusso ----------------------------------------------------------------------------------------
/** Voci per la barra laterale, una per azienda inoltrata: interruttore, colore e pallino propri. `statoDi` dà il pallino. */
export function calendariInoltrati(cfg: Config, statoDi: (azienda: string) => Ev): Ev[] {
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  const out: Ev[] = [];
  for (const [azienda, d] of personalizzazione().aziendeInoltrate) {
    const calId = "inoltro::" + azienda.toLowerCase();
    out.push({
      id: calId,
      account: azienda,
      alias: azienda,
      label: "Riunioni " + azienda,
      count: null,
      nota: "copiate da Power Automate in " + d.via,
      is_inoltro: true,
      color: colorForAccount(cfg, azienda, d.colore),
      enabled: !disabled.has(calId),
      ...statoDi(azienda),
    });
  }
  return out;
}
