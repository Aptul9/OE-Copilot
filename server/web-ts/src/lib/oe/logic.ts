import { daIstante, istante, isoSecondi, leggiIso } from "../core/fuso";
import { comeLoLegge, portaFuori } from "../valore-logo";
import type { Calendario, Config, Evento, FonteStato, GruppiCalendari, SchedaTimesheet } from "./types";

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

/**
 * Gli orari di get_events sono ore di Roma senza scarto, e il browser li leggerebbe come ore sue: si passano a istanti, così chi è in un
 * altro fuso vede la riunione all'ora di dove si trova. Un evento di tutto il giorno è una data e resta com'è, come un orario con lo scarto.
 */
export function inOraDelDispositivo(e: Evento): Evento {
  if (e.allDay) return e;
  const converti = (t: string) => {
    try {
      const { naive, scarto } = leggiIso(t);
      return scarto === null ? new Date(istante(naive)).toISOString() : t;
    } catch {
      return t;
    }
  };
  return { ...e, start: converti(e.start), end: e.end && converti(e.end) };
}

/** L'ora scritta in un campo datetime-local (ora del dispositivo) come ora di Roma senza scarto, quella che il server salva. */
export const daDatetimeLocal = (testo: string): string => {
  const ms = new Date(testo).getTime();
  return Number.isNaN(ms) ? testo : isoSecondi(daIstante(ms));
};

/** "YYYY-MM-DD" in ora locale. */
export const chiaveGiorno = (d: Date) => `${d.getFullYear()}-${due(d.getMonth() + 1)}-${due(d.getDate())}`;

/** "YYYY-MM-DDTHH:MM" in ora locale, per i campi datetime-local. */
export const perDatetimeLocal = (d: Date) => `${chiaveGiorno(d)}T${due(d.getHours())}:${due(d.getMinutes())}`;

const ora = (d: Date) => d.toLocaleTimeString("it-IT", { hour: "2-digit", minute: "2-digit" });
/** L'ultimo giorno che un evento tocca: una fine a mezzanotte (esclusiva, come nei giorni interi) è del giorno prima. */
const ultimoGiorno = (start: Date, end: Date) => (end > start && end.getHours() === 0 && end.getMinutes() === 0 ? new Date(end.getTime() - 1) : end);
const stessoGiorno = (start: Date, end: Date) => chiaveGiorno(start) === chiaveGiorno(ultimoGiorno(start, end));

/** Quando, nella scheda: il giorno e gli orari; un evento di più giorni ha anche il giorno della fine, che senza sembrerebbe finire la notte stessa. */
export function fmtRange(start: Date | null, end: Date | null, tuttoIlGiorno: boolean): string {
  if (!start) return "";
  const fine = end ?? start;
  const giorno = (d: Date) => d.toLocaleDateString("it-IT", { weekday: "long", day: "numeric", month: "long" });
  const inizio = capitalizza(giorno(start));
  if (tuttoIlGiorno) return `${inizio}${stessoGiorno(start, fine) ? "" : ` – ${giorno(ultimoGiorno(start, fine))}`} · tutto il giorno`;
  if (stessoGiorno(start, fine)) return `${inizio} · ${ora(start)}–${ora(fine)}`;
  return `${inizio}, ${ora(start)} – ${giorno(fine)}, ${ora(fine)}`;
}

/** Gli orari scritti sulla barra in cima (inAlto): le ore, e i giorni se la riunione ne tocca più d'uno. */
export function orariInCima(start: Date, end: Date): string {
  if (stessoGiorno(start, end)) return `${ora(start)}–${ora(end)}`;
  const giorno = (d: Date) => d.toLocaleDateString("it-IT", { day: "numeric", month: "short" });
  return `${giorno(start)} ${ora(start)} – ${giorno(end)} ${ora(end)}`;
}

// ---- Festività, sotto la data ------------------------------------------------
export type InfoGiorni = { festivi: Record<string, string> };

export function raccogliFestivi(eventi: Evento[]): InfoGiorni {
  const festivi: Record<string, string> = {};
  for (const e of eventi) {
    if (!e.allDay || e.nascosto || e.account !== "Festività") continue;
    const da = new Date(e.start);
    let ultimo = e.end ? new Date(e.end) : new Date(da);
    // La fine di un evento "tutto il giorno" è esclusiva: "1 nov → 2 nov" è solo il 1.
    if (e.end && ultimo > da && ultimo.getHours() === 0 && ultimo.getMinutes() === 0) ultimo = new Date(ultimo.getTime() - 86400000);
    for (const d = new Date(da); chiaveGiorno(d) <= chiaveGiorno(ultimo); d.setDate(d.getDate() + 1)) festivi[chiaveGiorno(d)] = e.title;
  }
  return { festivi };
}

/** Da quante ore una riunione sta in cima alle colonne dei giorni (inAlto). */
export const ORE_IN_ALTO = 4;

/**
 * Nelle viste a colonne una riunione da ORE_IN_ALTO ore in su sta in cima, nella riga dei giorni interi, su tutti i giorni che tocca,
 * come gli eventi di più giorni in Outlook: nella griglia copriva la colonna e stringeva le riunioni accanto. Gli orari veri restano
 * nell'evento (extendedProps). La fine di un giorno intero è esclusiva: una riunione che finisce a mezzanotte non tocca il giorno dopo.
 */
export function inAlto(e: EventoGriglia): EventoGriglia {
  if (e.allDay) return e;
  const da = new Date(e.start);
  const a = new Date(e.end);
  if (!(a.getTime() - da.getTime() >= ORE_IN_ALTO * 3600000)) return e;
  return { ...e, allDay: true, start: chiaveGiorno(da), end: chiaveGiorno(spostaGiorni(a, a.getHours() || a.getMinutes() ? 1 : 0)) };
}

/**
 * Un evento in cima che comincia prima del primo giorno che si vede (`giorno`, AAAA-MM-GG) diviso lì in due: FullCalendar scrive il titolo
 * all'inizio della barra, che starebbe fra i giorni nascosti a sinistra (quelli dove si scorre), e nella parte che si vede resterebbe una barra vuota.
 */
export function spezzaAl(e: EventoGriglia, giorno: string): EventoGriglia[] {
  if (!e.allDay || !(e.start < giorno && e.end > giorno)) return [e];
  return [{ ...e, end: giorno }, { ...e, id: `${e.id}|${giorno}`, start: giorno }];
}

/** Un orario di un evento come data: un giorno intero è una data, da leggere come mezzanotte del dispositivo (new Date la prenderebbe in UTC). */
export const dataEvento = (t: string, tuttoIlGiorno: boolean) => new Date(tuttoIlGiorno ? `${t.slice(0, 10)}T00:00` : t);

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

/** La colonna di sinistra: da quanto a quanto si allarga con la maniglia, e la larghezza di partenza (px). */
export const LARGHEZZA_PANNELLO = { min: 220, max: 480, predefinita: 256 } as const;
export const limitaLarghezza = (px: number): number =>
  Number.isFinite(px) ? Math.round(Math.min(LARGHEZZA_PANNELLO.max, Math.max(LARGHEZZA_PANNELLO.min, px))) : LARGHEZZA_PANNELLO.predefinita;

/** Le voci della barra divise in (principali, secondarie): le secondarie sono quelle di `interfaccia.calendari_secondari`, ognuna nel suo ordine. */
export function separaSecondari(gruppi: GruppoAzienda[], secondari: ReadonlySet<string>): [GruppoAzienda[], GruppoAzienda[]] {
  return [gruppi.filter((g) => !secondari.has(g.azienda)), gruppi.filter((g) => secondari.has(g.azienda))];
}

/** I calendari secondari: quelli di personalizzazione.json, con sopra le scelte della finestra Calendari (true li mette, false li toglie). */
export function secondariEffettivi(delFile: readonly string[] | undefined, scelti: Record<string, boolean> | undefined): Set<string> {
  const s = new Set(delFile ?? []);
  for (const [nome, on] of Object.entries(scelti ?? {})) {
    if (on) s.add(nome);
    else s.delete(nome);
  }
  return s;
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
  if (pulito.length > GRUPPO_NOME_MAX) return { errore: `Massimo ${GRUPPO_NOME_MAX} caratteri.` };
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

// ---- Colori dinamici ------------------------------------------------------------
/** Le tinte dei colori dinamici: otto a passi uguali sulla ruota (oklch), con la stessa luminosità e la stessa saturazione, e per ultimo il grigio. */
const RUOTA = 8;
export const TINTE_DINAMICHE = [...Array.from({ length: RUOTA }, (_, i) => `oklch(0.62 0.17 ${20 + (i * 360) / RUOTA})`), "oklch(0.62 0 0)"];
const QUARTO_DORA = 15 * 60000;
/** I passi sulla ruota fra due tinte. Il grigio sta al centro: da ogni tinta dista più di un passo e meno di due. */
const distanzaTinte = (a: number, b: number) => (a === b ? 0 : Math.max(a, b) === RUOTA ? 1.5 : Math.min(Math.abs(a - b), RUOTA - Math.abs(a - b)));
const aziendaDi = (e: Evento) => e.alias || e.account;

/**
 * Un colore per azienda scelto sulla pagina che si guarda (config.colori_dinamici): due aziende con riunioni sovrapposte, o una in fila
 * all'altra (fino a un quarto d'ora di pausa), prendono tinte lontane sulla ruota. Chi aveva già una tinta (`precedenti`) la tiene finché
 * non finisce accanto a una uguale o vicina: così a ogni rilettura i colori restano quelli. Con più aziende a contatto che tinte la
 * ripetizione tocca alla coppia che si incontra meno.
 */
export function coloriDinamici(
  eventi: Evento[],
  aziende: string[],
  precedenti: ReadonlyMap<string, string> = new Map(),
  pagina?: { start: Date; end: Date },
): Map<string, string> {
  const visti = eventi.filter(
    (e) => eventoVisibile(e) && (!pagina || (Date.parse(e.start) < pagina.end.getTime() && Date.parse(e.end || e.start) >= pagina.start.getTime())),
  );
  // quante volte due aziende si toccano
  const contatti = new Map<string, Map<string, number>>();
  for (const az of [...aziende, ...visti.map(aziendaDi)]) if (!contatti.has(az)) contatti.set(az, new Map());
  const orari = visti
    .filter((e) => !e.allDay)
    .map((e) => ({ az: aziendaDi(e), da: Date.parse(e.start), a: Date.parse(e.end) }))
    .sort((x, y) => x.da - y.da);
  for (let i = 0; i < orari.length; i++) {
    for (let j = i + 1; j < orari.length && orari[j].da <= orari[i].a + QUARTO_DORA; j++) {
      const [x, y] = [orari[i].az, orari[j].az];
      if (x === y) continue;
      contatti.get(x)!.set(y, (contatti.get(x)!.get(y) ?? 0) + 1);
      contatti.get(y)!.set(x, (contatti.get(y)!.get(x) ?? 0) + 1);
    }
  }
  const peso = (az: string) => [...contatti.get(az)!.values()].reduce((s, n) => s + n, 0);
  // prima chi ha più contatti: è quella con meno scelta
  const ordine = [...contatti.keys()].sort((a, b) => peso(b) - peso(a) || (a < b ? -1 : 1));
  const tinta = new Map<string, number>();
  const usi = TINTE_DINAMICHE.map(() => 0);
  for (const az of ordine) {
    const accanto = [...contatti.get(az)!].filter(([v]) => tinta.has(v)).map(([v, n]) => ({ t: tinta.get(v)!, n }));
    // di una tinta: quanto dista dalla più vicina fra quelle accanto, e quante riunioni si toccano a quella distanza
    const voto = (t: number) => {
      const d = Math.min(...accanto.map((v) => distanzaTinte(t, v.t)));
      return { t, d, n: accanto.filter((v) => distanzaTinte(t, v.t) === d).reduce((s, v) => s + v.n, 0) };
    };
    // la più lontana; a pari distanza quella che tocca meno riunioni, poi la meno usata
    const migliore = usi.map((_, t) => voto(t)).sort((a, b) => b.d - a.d || a.n - b.n || usi[a.t] - usi[b.t] || a.t - b.t)[0];
    const prima = TINTE_DINAMICHE.indexOf(precedenti.get(az) ?? "");
    const scelta = prima >= 0 && voto(prima).d >= Math.min(2, migliore.d) ? prima : migliore.t;
    tinta.set(az, scelta);
    usi[scelta]++;
  }
  return new Map([...tinta].map(([az, t]) => [az, TINTE_DINAMICHE[t]]));
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
  return ora ? `${cosa}. Dati aggiornati alle ${ora}.` : `${cosa}.`;
}

// ---- Impostazioni del dispositivo ------------------------------------------------
/**
 * Le voci di config che restano nel dispositivo (localStorage della pagina): aspetto, vista e calendari accesi. Il resto (nomi,
 * gruppi, link ICS, promemoria) sta sul server, uguale per tutti. I calendari spenti salvati sul server restano quelli dei link condivisi.
 */
export const VOCI_DEL_DISPOSITIVO: readonly string[] = ["theme", "zoom_calendario", "colori_dinamici", "vista", "secondari_scelti", "reminder_poll_seconds", "disabled_calendars"];

const scegli = (c: Partial<Config>, delDispositivo: boolean): Partial<Config> =>
  Object.fromEntries(Object.entries(c).filter(([k]) => VOCI_DEL_DISPOSITIVO.includes(k) === delDispositivo));
export const soloDelDispositivo = (c: Partial<Config>) => scegli(c, true);
export const soloDelServer = (c: Partial<Config>) => scegli(c, false);

/** La config della pagina: quella del server con sopra quella del dispositivo. Un dispositivo che non ha niente parte da quella del server. */
export function unisciConfig(server: Config, dispositivo: Partial<Config> | null): { config: Config; dispositivo: Partial<Config> } {
  const mia = dispositivo ?? soloDelDispositivo(server);
  return { config: { ...server, ...mia }, dispositivo: mia };
}

/** I calendari spenti dopo una scelta: `acceso(c)` vero lo accende, falso lo spegne, undefined lo lascia com'è. */
export function cambiaSpenti(cals: Calendario[], spenti: readonly string[], acceso: (c: Calendario) => boolean | undefined): string[] {
  const s = new Set(spenti);
  for (const c of cals) {
    const on = acceso(c);
    if (on === true) s.delete(c.id);
    else if (on === false) s.add(c.id);
  }
  return [...s].sort();
}

// ---- Dove si apre la pagina -----------------------------------------------------
/**
 * Nell'app per Windows, nell'app per Android o in un browser. Le app mettono in window `__TAURI_INTERNALS__` (Tauri) e
 * `__OE_ORIGINE__` (il loro script); un browser nessuno dei due. Serve a non mostrare mai nelle app le notifiche del browser.
 */
export function dovePagina(w: object, userAgent: string): "windows" | "android" | "browser" {
  if (!("__TAURI_INTERNALS__" in w) && !("__OE_ORIGINE__" in w)) return "browser";
  return /Android/i.test(userAgent) ? "android" : "windows";
}

// ---- Vista a 3 giorni: lettura larga + memoria ------------------------------------
export const PRIMA_GIORNI = 7;
export const DOPO_GIORNI = 14;
export const SOGLIA_RABBOCCO = 3;

export type CacheLarga = { da: Date; a: Date };

/** Giorno, 3 giorni e settimana scorrono e mostrano anche i giorni accanto: si leggono da un intorno largo tenuto in memoria. */
export const aScorrimento = (inizio: Date, fine: Date) => [1, 3, 7].includes(giorniTra(inizio, fine));
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

/**
 * Di quanti giorni si sposta una vista che scorre al rilascio (`dx` in px, positivo = trascinata a destra = indietro): il passo più vicino
 * a dove arriverebbe frenando da sola (velocità in px/ms, frenata 0,0025 px/ms²: un trascinamento calmo resta quasi dov'è, un colpo veloce va
 * più in là), mai oltre i giorni disegnati di lato.
 */
export function giorniDiArrivo(dx: number, velocita: number, colonna: number, passo: number, bordo: number): number {
  if (!colonna) return 0;
  const max = Math.floor(bordo / passo);
  const passi = Math.round(-(dx + (velocita * Math.abs(velocita)) / 0.005) / (colonna * passo));
  return Math.max(-max, Math.min(max, passi)) * passo || 0;
}

/** La vista con cui si apre uno schermo stretto quando non ne ha scelta una sua: la settimana salvata dal computer diventa 3 giorni. */
export function vistaPerSchermo(vista: string, stretto: boolean): string {
  return stretto && vista === "timeGridWeek" ? VISTA_3G : vista;
}
