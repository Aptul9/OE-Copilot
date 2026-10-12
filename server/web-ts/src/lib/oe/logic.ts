import { comeLoLegge, portaFuori } from "../valore-logo";
import type { Calendario, Evento, FonteStato, GruppiCalendari, SchedaTimesheet } from "./types";

// Funzioni pure della pagina, portate dalla vecchia pagina (app.js, nella storia di git fino a 2d57a83^): niente DOM, niente rete, provabili da sole.

export const capitalizza = (s: string) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

/** Iniziali per il cerchietto: "Alfa" -> "AL", "Beta Servizi" -> "BS", "Alfa · m.rossi" -> "AL". */
export function iniziali(nome: string | null | undefined): string {
  let s = String(nome || "").split("·")[0].trim();
  s = s.split("@")[0];
  const parole = s.split(/[\s._\-+]/).filter(Boolean);
  if (parole.length >= 2) return (parole[0][0] + parole[1][0]).toUpperCase();
  return s.slice(0, 2).toUpperCase();
}

/**
 * Dove sta un logo dichiarato in personalizzazione.json: "logos/serco.png", nella cartella dei loghi del server. Un indirizzo di un altro
 * sito qui non arriva: lo scarica il gateway e la pagina riceve il file in cache (lib/core/loghi.ts). Se arrivasse comunque non si carica
 * da lì ("" e si vedono le iniziali): il valore si legge come lo legge il browser, quindi anche "/\sito.org/x.png" e "\\sito.org\x.png"
 * (per lui https://sito.org/x.png) restano fuori.
 */
export function urlLogo(valore: string): string {
  const v = comeLoLegge(valore);
  if (!v || portaFuori(v)) return "";
  return v.startsWith("/") ? v : `/${v}`;
}

// Un dominio scritto da solo (esempio.org) non è un indirizzo: si offre l'icona del sito. Se ha già un percorso (esempio.org/logo.png) basta lo schema.
const DOMINIO = "(?:[a-z0-9-]+\\.)+[a-z]{2,}(?::\\d+)?";
export const dominioScritto = (testo: string): string | null => (new RegExp(`^${DOMINIO}$`, "i").test(testo.trim()) ? testo.trim().toLowerCase() : null);
export const iconaDelSito = (dominio: string) => `https://${dominio}/favicon.ico`;
/** L'indirizzo da mandare al server: quello scritto, con https:// davanti se è un dominio con un percorso e niente schema. */
export const indirizzoDaTesto = (testo: string): string => (new RegExp(`^${DOMINIO}[/?#]`, "i").test(testo.trim()) ? `https://${testo.trim()}` : testo.trim());

/** Il logo degli Eventi personali sta nell'immagine del gateway (logos-predefiniti/personale.png): nessuno lo dichiara in personalizzazione.json. */
export const LOGO_EVENTI_PERSONALI = "/logos/personale.png";

/** I loghi da mostrare, per nome come lo mostra la barra: quello degli Eventi personali, poi quelli dichiarati (personalizzazione.json e finestra Calendari), che vincono. */
export const mappaLoghi = (loghi: Record<string, string> | undefined): Record<string, string> => ({
  "Eventi personali": LOGO_EVENTI_PERSONALI,
  ...Object.fromEntries(
    Object.entries(loghi ?? {})
      .map(([nome, valore]) => [nome, typeof valore === "string" ? urlLogo(valore) : ""] as const)
      .filter(([, url]) => url),
  ),
});

/** Il colore mescolato alla superficie del tema, senza trasparenza: la griglia non traspare dall'evento. */
export const tintaOpaca = (colore: string, percento: number, base = "var(--card)") =>
  `color-mix(in srgb, ${colore} ${percento}%, ${base})`;

// ---- Date -----------------------------------------------------------------
export const giorniTra = (a: Date, b: Date) => Math.round((b.getTime() - a.getTime()) / 86400000);
export const spostaGiorni = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);

const due = (n: number) => String(n).padStart(2, "0");

/** ISO con lo scarto orario locale, come quello che manda FullCalendar (2026-10-11T00:00:00+02:00). */
export function isoLocale(d: Date): string {
  const scarto = -d.getTimezoneOffset();
  const assoluto = Math.abs(scarto);
  return (
    `${d.getFullYear()}-${due(d.getMonth() + 1)}-${due(d.getDate())}T${due(d.getHours())}:${due(d.getMinutes())}:${due(d.getSeconds())}` +
    `${scarto < 0 ? "-" : "+"}${due(Math.floor(assoluto / 60))}:${due(assoluto % 60)}`
  );
}

/** "YYYY-MM-DD" in ora locale. */
export const chiaveGiorno = (d: Date) => `${d.getFullYear()}-${due(d.getMonth() + 1)}-${due(d.getDate())}`;

/** "YYYY-MM-DDTHH:MM" in ora locale, per i campi datetime-local. */
export const perDatetimeLocal = (d: Date) => `${chiaveGiorno(d)}T${due(d.getHours())}:${due(d.getMinutes())}`;

export function fmtRange(start: Date | null, end: Date | null, tuttoIlGiorno: boolean): string {
  const opt: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" };
  const giorno = start ? start.toLocaleDateString("it-IT", { weekday: "long", day: "numeric", month: "long" }) : "";
  if (tuttoIlGiorno) return `${capitalizza(giorno)} · tutto il giorno`;
  const s = start ? start.toLocaleTimeString("it-IT", opt) : "";
  const e = end ? end.toLocaleTimeString("it-IT", opt) : "";
  return `${capitalizza(giorno)} · ${s}–${e}`;
}

// ---- Festività e eventi di un giorno intero, sotto la data -----------------
export type ChipGiorno = { title: string; color: string };
export type InfoGiorni = { festivi: Record<string, string>; tuttoGiorno: Record<string, ChipGiorno[]> };

export function raccogliFestivi(eventi: Evento[]): InfoGiorni {
  const festivi: Record<string, string> = {};
  const tuttoGiorno: Record<string, ChipGiorno[]> = {};
  for (const e of eventi) {
    if (!e.allDay || e.nascosto) continue;   // calendario spento: niente riga sotto il giorno
    const da = new Date(e.start);
    let ultimo = e.end ? new Date(e.end) : new Date(da);
    // La fine di un evento "tutto il giorno" è esclusiva: "1 nov → 2 nov" è solo il 1.
    if (e.end && ultimo > da && ultimo.getHours() === 0 && ultimo.getMinutes() === 0) ultimo = new Date(ultimo.getTime() - 86400000);
    for (const d = new Date(da); chiaveGiorno(d) <= chiaveGiorno(ultimo); d.setDate(d.getDate() + 1)) {
      const g = chiaveGiorno(d);
      if (e.account === "Festività") festivi[g] = e.title;
      else (tuttoGiorno[g] ??= []).push({ title: e.title, color: e.color });
    }
  }
  return { festivi, tuttoGiorno };
}

// ---- Elenco calendari raggruppato per azienda -------------------------------
export type GruppoAzienda = {
  azienda: string;
  color: string;
  caselle: string[];
  tot: number;
  on: number;
  rotto: boolean;
  avviso: boolean;
  motivi: string[];
};

const senzaAccenti = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

/** Le aziende il cui nome, o quello di una delle loro caselle, contiene il testo cercato (senza badare a maiuscole e accenti). Testo vuoto = tutte. */
export function filtraAziende(gruppi: GruppoAzienda[], ricerca: string): GruppoAzienda[] {
  const q = senzaAccenti(ricerca.trim());
  if (!q) return gruppi;
  return gruppi.filter((g) => [g.azienda, ...g.caselle].some((t) => senzaAccenti(t).includes(q)));
}

/** Una voce per azienda mostrata, non per casella: la stessa azienda può avere calendari in posti diversi. */
export function raggruppaAziende(cals: Calendario[]): GruppoAzienda[] {
  const gruppi = new Map<string, GruppoAzienda>();
  for (const c of cals) {
    const azienda = c.alias || c.account;
    let g = gruppi.get(azienda);
    if (!g) {
      g = { azienda, color: c.color, caselle: [], tot: 0, on: 0, rotto: false, avviso: false, motivi: [] };
      gruppi.set(azienda, g);
    }
    if (!g.caselle.includes(c.account)) g.caselle.push(c.account);
    g.tot++;
    if (c.enabled) g.on++;
    // Basta un calendario in errore per marcare la casella: meglio un allarme in più che un rosso mancato.
    if (c.stato === "avviso") {
      g.avviso = true;
      if (c.stato_msg) g.motivi.push(c.stato_msg);
    } else if (c.stato && c.stato !== "ok") {
      g.rotto = true;
      if (c.stato_msg) g.motivi.push(c.stato_msg);
    }
  }
  return [...gruppi.values()];
}

export type Salute = { classe: "good" | "warn" | "bad" | "attesa"; titolo: string };
const CLASSE_SALUTE: Record<string, Salute["classe"]> = { ok: "good", avviso: "warn", errore: "bad" };

/** Il pallino di un'azienda: conta prima l'ultima lettura degli eventi, poi lo stato dei calendari. */
export function salute(statoFonti: Record<string, FonteStato>, g: GruppoAzienda, inAttesa = false): Salute {
  if (inAttesa) return { classe: "attesa", titolo: "Controllo in corso…" };
  const f = statoFonti[g.azienda];
  if (f) return { classe: CLASSE_SALUTE[f.stato] || "warn", titolo: f.msg || "" };
  if (g.rotto) return { classe: "bad", titolo: g.motivi.join(" · ") || "Non raggiungibile" };
  if (g.avviso) return { classe: "warn", titolo: g.motivi.join(" · ") || "Dati non aggiornati" };
  return { classe: "warn", titolo: "Non ancora letto" };
}

// ---- Gruppi di calendari -----------------------------------------------------------
export const GRUPPO_NOME_MAX = 40;

/** I gruppi del file di configurazione: solo nomi con un elenco di testi, il resto si scarta. */
export function leggiGruppi(grezzo: unknown): GruppiCalendari {
  const gruppi: GruppiCalendari = {};
  if (!grezzo || typeof grezzo !== "object" || Array.isArray(grezzo)) return gruppi;
  for (const [nome, membri] of Object.entries(grezzo)) {
    if (Array.isArray(membri)) gruppi[nome] = membri.filter((m): m is string => typeof m === "string");
  }
  return gruppi;
}

export type EsitoGruppi = { gruppi: GruppiCalendari } | { errore: string };

function nomeGruppo(gruppi: GruppiCalendari, nome: string, escludi?: string): { nome: string } | { errore: string } {
  const pulito = nome.trim().replace(/\s+/g, " ");
  if (!pulito) return { errore: "Scrivi un nome per il gruppo." };
  if (pulito.length > GRUPPO_NOME_MAX) return { errore: `Il nome può avere al massimo ${GRUPPO_NOME_MAX} caratteri.` };
  const minuscolo = pulito.toLowerCase();
  if (Object.keys(gruppi).some((n) => n !== escludi && n.toLowerCase() === minuscolo)) return { errore: "Esiste già un gruppo con questo nome." };
  return { nome: pulito };
}

export function aggiungiGruppo(gruppi: GruppiCalendari, nome: string): EsitoGruppi {
  const v = nomeGruppo(gruppi, nome);
  return "errore" in v ? v : { gruppi: { ...gruppi, [v.nome]: [] } };
}

/** Il nuovo nome prende il posto del vecchio, nello stesso ordine. */
export function rinominaGruppo(gruppi: GruppiCalendari, vecchio: string, nuovo: string): EsitoGruppi {
  const v = nomeGruppo(gruppi, nuovo, vecchio);
  if ("errore" in v) return v;
  return { gruppi: Object.fromEntries(Object.entries(gruppi).map(([n, m]) => [n === vecchio ? v.nome : n, m])) };
}

export const eliminaGruppo = (gruppi: GruppiCalendari, nome: string): GruppiCalendari =>
  Object.fromEntries(Object.entries(gruppi).filter(([n]) => n !== nome));

/** Mette un'azienda dentro o fuori da un gruppo. */
export function impostaAppartenenza(gruppi: GruppiCalendari, gruppo: string, azienda: string, dentro: boolean): GruppiCalendari {
  const membri = (gruppi[gruppo] ?? []).filter((a) => a !== azienda);
  return { ...gruppi, [gruppo]: dentro ? [...membri, azienda] : membri };
}

/** I gruppi il cui elenco, tra le aziende che esistono, coincide con quelle accese ora. */
export function gruppiAttivi(gruppi: GruppiCalendari, aziende: GruppoAzienda[]): string[] {
  const esistenti = new Set(aziende.map((g) => g.azienda));
  const accese = new Set(aziende.filter((g) => g.on > 0).map((g) => g.azienda));
  return Object.keys(gruppi).filter((n) => {
    const membri = new Set(gruppi[n].filter((a) => esistenti.has(a)));
    return membri.size > 0 && membri.size === accese.size && [...membri].every((a) => accese.has(a));
  });
}

// ---- Eventi per la griglia ---------------------------------------------------
export type OpzioniEventi = { secondari: Set<string>; ricerca: string };

export const eventoVisibile = (e: Evento) => !e.nascosto && e.account !== "Festività";

export function corrisponde(e: Evento, ricerca: string): boolean {
  const q = ricerca.trim().toLowerCase();
  if (!q) return true;
  return [e.title, e.alias, e.account, e.calendar, e.location, e.organizer].filter(Boolean).join(" ").toLowerCase().includes(q);
}

export type EventoGriglia = {
  id: string;
  title: string;
  start: string;
  end: string;
  allDay: boolean;
  backgroundColor: string;
  borderColor: string;
  classNames: string[];
  // blocco anche nel mese (di norma lì un evento con l'orario è un pallino): FullCalendar lo decide alla lettura dell'evento
  display: "block";
  extendedProps: Evento;
};

/** L'evento come lo vuole FullCalendar: riempimento tenue e barretta piena, più tenue per i calendari secondari. */
export function eventoPerGriglia(e: Evento, secondari: Set<string>): EventoGriglia {
  const sec = secondari.has(e.alias ?? "");
  return {
    id: e.id,
    title: e.title,
    start: e.start,
    end: e.end,
    allDay: e.allDay,
    backgroundColor: tintaOpaca(e.color, sec ? 7 : 14),
    borderColor: sec ? tintaOpaca(e.color, 55) : e.color,
    classNames: sec ? ["secondario"] : [],
    display: "block",
    extendedProps: e,
  };
}

export function eventiPerGriglia(eventi: Evento[], { secondari, ricerca }: OpzioniEventi): EventoGriglia[] {
  return eventi.filter((e) => eventoVisibile(e) && corrisponde(e, ricerca)).map((e) => eventoPerGriglia(e, secondari));
}

// ---- Scheda dell'evento ---------------------------------------------------------
export type AzioniEvento = { partecipa: string | null; apriInTeams: { account: string; url: string } | null; elimina: string | null };

/**
 * Cosa si può fare da una scheda nel browser. "Apri in Outlook" non c'è: serve Outlook su un PC.
 * "Apri in Teams" ci sta per le caselle Microsoft e per ogni riunione con un link Teams, non per le
 * caselle che in Teams non esistono come account né per le copie di un'azienda inoltrata, e nel
 * browser solo se la riunione ha un link da aprire.
 */
export function azioniEvento(p: Evento, senzaTeams: Set<string>): AzioniEvento {
  const casellaRemota = !!p.remoto && !p.is_google;
  const linkTeams = /teams\.(microsoft\.com|live\.com)/i.test(p.online_url || "");
  const teams =
    (casellaRemota || linkTeams) && String(p.account || "").includes("@") && !senzaTeams.has(p.alias ?? "") && !p.inoltrata_da && !!p.online_url;
  return {
    partecipa: p.online_url ? encodeURI(p.online_url) : null,
    apriInTeams: teams ? { account: p.account, url: p.online_url as string } : null,
    elimina: p.is_local && p.uid ? p.uid : null,
  };
}

// ---- Sincronizzazione --------------------------------------------------------
export function etichettaSync(ultimaSync: number | null, adesso: number, inCorso: number): string {
  if (inCorso > 0) return "Sincronizzo…";
  if (ultimaSync === null) return "—";
  const sec = Math.round((adesso - ultimaSync) / 1000);
  if (sec < 10) return "Sincronizzato ora";
  if (sec < 60) return `Sincronizzato ${sec} sec fa`;
  const min = Math.round(sec / 60);
  return min < 60 ? `Sincronizzato ${min} min fa` : `Sincronizzato ${Math.round(min / 60)} h fa`;
}

/** Avviso dei dati salvati, quando le caselle non rispondono ancora. */
export function testoDatiVecchi(quando: string | null | undefined, motivo: string | null | undefined): string {
  const ora = quando ? quando.slice(11, 16) : null;
  const cosa = motivo || "Il server dei calendari non risponde";
  return ora ? `${cosa} — questi sono i dati salvati delle ${ora}. Aggiorno da solo…` : `${cosa} — aggiorno da solo appena è pronto…`;
}

// ---- Vista a 3 giorni: lettura larga + memoria ------------------------------------
export const PRIMA_GIORNI = 7;
export const DOPO_GIORNI = 14;
export const SOGLIA_RABBOCCO = 3;

export type CacheLarga = { da: Date; a: Date };

export const eATreGiorni = (inizio: Date, fine: Date) => giorniTra(inizio, fine) === 3;
export const dentroLaCache = (c: CacheLarga | null, inizio: Date, fine: Date) => !!c && inizio >= c.da && fine <= c.a;
export const vicinoAlBordo = (c: CacheLarga, inizio: Date, fine: Date) => giorniTra(c.da, inizio) < SOGLIA_RABBOCCO || giorniTra(fine, c.a) < SOGLIA_RABBOCCO;
export const periodoLargo = (inizio: Date, fine: Date): CacheLarga => ({ da: spostaGiorni(inizio, -PRIMA_GIORNI), a: spostaGiorni(fine, DOPO_GIORNI) });

// ---- Timesheet --------------------------------------------------------------------
export function schedeTimesheet(schede: SchedaTimesheet[] | undefined): SchedaTimesheet[] {
  // una scheda con un'azione si mostra solo se un'estensione la fornisce: nel browser non ce ne sono
  return (schede ?? []).filter((a) => a && a.nome && (!a.azione || a.url));
}

export const sigleScheda = (nome: string) =>
  nome
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();

// ---- Vista adatta allo schermo -----------------------------------------------------
export const VISTA_3G = "timeGridTreGiorni";

/** La settimana non sta su un tablet in verticale (3 giorni) né su un telefono (un giorno). */
export function vistaPerSchermo(vista: string, stretto: boolean, telefono: boolean): string {
  if (telefono && (vista === "timeGridWeek" || vista === VISTA_3G)) return "timeGridDay";
  if (stretto && vista === "timeGridWeek") return VISTA_3G;
  return vista;
}
