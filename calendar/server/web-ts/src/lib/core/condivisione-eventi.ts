import { AVANTI, EVENTI_MAX, INDIETRO, type Livello } from "../condivisione-comune";
import { blocchi, GIORNI_BLOCCO, spostaData, unisci } from "../disponibilita";
import { type EventoUscita, istanteDaRoma } from "../ics-uscita";
import { calendariDiInoltro } from "./agente";
import type { Ev } from "./caselle";
import type { Condivisione } from "./condivisione";
import { raccogli } from "./eventi";
import { chiave, iso, isoSecondi, leggiNaive, mezzanotte, naiveLocale, piuGiorni, type Naive } from "./fuso";
import { impegni } from "./impegni";
import { ID_CALENDARIO } from "./locali";
import { ACCOUNT_LOCALE } from "./personalizzazione";
import { type Config, loadConfig, membriGruppo, nelGruppo } from "./stato";

// Cosa c'è nel calendario di un link: gli stessi eventi che l'app mostra (raccogli, lo stesso di get_events), letti a blocchi di 28 giorni come la
// disponibilità pubblica, e ridotti a quello che il livello del link permette. Qui si decide cosa può uscire: i calendari ricevuti da altri (link ICS,
// festività, i calendari condivisi da un altro OE Copilot) non escono mai, così due istanze che si importano a vicenda non si rimandano i propri
// eventi in giro; di un evento escono gli orari e, nel livello con i titoli, il titolo e il luogo. Mai la descrizione, chi organizza e chi partecipa,
// i link per collegarsi, il nome di un'azienda o di una casella.

/** Il lavoro di un blocco al più per volta: ognuno è una richiesta all'agente. */
const BLOCCHI_INSIEME = 4;
export { EVENTI_MAX };
const TITOLO_MAX = 250;
const LUOGO_MAX = 250;

/** Un evento nel livello con i titoli, ancora in ora di Roma (come lo dà get_events): AAAA-MM-GGTHH:MM:SS, o AAAA-MM-GG con la fine esclusa per un giorno intero. */
export type EventoLetto = { inizio: string; fine: string; tuttoGiorno: boolean; titolo: string; luogo: string };
export type Lettura = { livello: Livello; da: string; a: string; impegni: [string, string][]; eventi: EventoLetto[] };

/** Il calendario non si può leggere adesso (l'agente non risponde, un'azienda pubblicata non si legge, il gruppo non c'è più): chi lo chiede riprova fra poco. */
export class LetturaNonDisponibile extends Error {}

const limita = (v: number, min: number, max: number, predefinito: number) => (Number.isFinite(v) ? Math.min(max, Math.max(min, Math.trunc(v))) : predefinito);

/** Dal giorno `da` al giorno `a` (escluso): i giorni indietro e avanti di un link, tenuti nei limiti anche se nel database ci fosse un valore fuori. */
export function finestra(c: Pick<Condivisione, "giorni_indietro" | "giorni_avanti">, oggi: string): { da: string; a: string } {
  const indietro = limita(c.giorni_indietro, INDIETRO.min, INDIETRO.max, INDIETRO.predefinito);
  const avanti = limita(c.giorni_avanti, AVANTI.min, AVANTI.max, AVANTI.predefinito);
  return { da: spostaData(oggi, -indietro), a: spostaData(oggi, avanti + 1) };
}

// ---- Cosa può uscire di un titolo e di un luogo ------------------------------------------------------------------------------
// Niente espressioni regolari che ripartono da ogni posizione (su un testo di centomila caratteri ci mettevano secondi): il testo si guarda a parole,
// una passata sola, e prima si taglia a ESAME_MAX caratteri, perché oltre non esce niente comunque.

/** Quanti caratteri di un titolo o di un luogo si guardano: un ingresso di centomila caratteri costa quanto uno di mille. Si taglia PRIMA di ogni espressione regolare. */
const ESAME_MAX = 1000;
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g;
/** Invisibili e di direzione: spazi a larghezza zero, LRM e RLM, incorporamenti, forzature e isolati di verso, BOM. Spariscono: un testo non deve cambiare verso né nascondere pezzi. */
const FORMATO = /[\u061c\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]/g;
const SCHEMI_SENZA_BARRE = ["mailto", "tel", "sip", "sips", "callto", "skype", "msteams", "zoommtg", "webex", "facetime", "sms"];
const SITI_CONFERENZA = [
  "teams.microsoft.com",
  "teams.live.com",
  "zoom.us",
  "zoomgov.com",
  "meet.google.com",
  "webex.com",
  "gotomeeting.com",
  "gotomeet.me",
  "join.skype.com",
  "whereby.com",
  "bluejeans.com",
  "chime.aws",
  "meet.jit.si",
  "8x8.vc",
  "discord.gg",
];
/** Quello che serve per entrare in una riunione a telefono o online: ID e codici, numeri da chiamare. Un luogo che ne parla non esce, nemmeno a pezzi. */
const DATI_DI_ACCESSO =
  /\b(?:(?:conference|meeting|conferenza|riunione|webinar)\s*id|id\s*(?:conference|meeting|conferenza|riunione)|pass\s?code|pin|dial[\s-]*in|(?:codice|code)\s*(?:di\s*)?(?:accesso|riunione|conferenza|access|meeting)|numero\s*(?:di\s*)?(?:accesso|conferenza)|chiamami|join\s*by\s*phone)\b/i;
// il testo che i programmi di posta mettono nel luogo di una riunione online: dice solo che è online
const SOLO_CONFERENZA = /^(?:(?:riunione|incontro|meeting)\s+(?:di\s+)?)?(?:microsoft\s+teams|ms\s+teams|teams|zoom|google\s+meet|webex|skype(?:\s+for\s+business)?|gotomeeting|online|virtuale)(?:\s+(?:meeting|riunione|call|chiamata|incontro))?$/i;

const èCifra = (c: number) => c >= 48 && c <= 57;
const èLettera = (c: number) => c >= 97 && c <= 122;
/** La punteggiatura che può stare intorno a una parola: ogni segno ASCII, e le virgolette e i puntini che i programmi di scrittura mettono al loro posto. */
const èBordo = (c: number) =>
  c <= 47 || (c >= 58 && c <= 64) || (c >= 91 && c <= 96) || (c >= 123 && c <= 127) || c === 0xab || c === 0xbb || (c >= 0x2018 && c <= 0x201f) || c === 0x2026 || c === 0x2039 || c === 0x203a;
/** Dove una parola si spezza per trovare un indirizzo incollato a un'etichetta: "Meet:meet.google.com/abc", "sala(bit.ly/x)". */
const INCOLLATE = /[:;,=()[\]<>"'|]+/;

/** La parola senza la punteggiatura intorno: (www.esempio.it), "zoom.us". Due indici, nessuna espressione regolare. */
function senzaBordi(s: string): string {
  let i = 0;
  let j = s.length;
  while (i < j && èBordo(s.charCodeAt(i))) i++;
  while (j > i && èBordo(s.charCodeAt(j - 1))) j--;
  return s.slice(i, j);
}

/** Un nome di sito: etichette di lettere, cifre, trattini e lettere non ASCII separate da punti, l'ultima che comincia con una lettera ("bit.ly", "8x8.vc", "app.slack.com"); o un indirizzo IPv4. */
function nomeDiSito(host: string): boolean {
  const etichette = host.split(".");
  if (etichette.length < 2) return false;
  let tutteCifre = true;
  for (const e of etichette) {
    if (!e) return false;
    for (let k = 0; k < e.length; k++) {
      const c = e.charCodeAt(k);
      if (!(èCifra(c) || èLettera(c) || c === 45 || c === 95 || c > 127)) return false;
      if (!èCifra(c)) tutteCifre = false;
    }
  }
  const ultima = etichette[etichette.length - 1].charCodeAt(0);
  return èLettera(ultima) || ultima > 127 || (etichette.length === 4 && tutteCifre);
}

/** Un sito: www.; `sito/percorso` senza schema, anche con :porta (bit.ly/X, 8x8.vc/X/sala, app.slack.com/huddle/X, 192.168.1.10/sala); il nome di un servizio di riunioni anche da solo. */
function èUnSito(s: string): boolean {
  if (s.startsWith("www.")) return true;
  const barra = s.indexOf("/");
  if (barra > 0 && nomeDiSito(s.slice(0, barra).split(":")[0])) return true;
  const fineHost = s.search(/[/?#:]/);
  const host = fineHost < 0 ? s : s.slice(0, fineHost);
  return SITI_CONFERENZA.some((h) => host === h || host.endsWith("." + h));
}

/**
 * Una parola che è, o porta a, un indirizzo. Se lo è esce tutta la parola, fino allo spazio: un indirizzo può avere dentro parentesi, virgole e punti, e
 * fermarsi al primo lascerebbe in giro il resto (https://esempio.it/a(b)SEGRETO?x=1). Sono indirizzi: tutto quello che ha `://`; mailto:, tel:, sip: e simili;
 * un sito (èUnSito), anche se è incollato a un'etichetta con uno dei segni di INCOLLATE; con `ancheLaPosta` un indirizzo di posta.
 */
function èUnIndirizzo(parola: string, ancheLaPosta: boolean): boolean {
  const p = parola.toLowerCase();
  if (p.includes("://")) return true;
  const s = senzaBordi(p);
  for (const schema of SCHEMI_SENZA_BARRE) if (s.startsWith(schema + ":")) return true;
  if (ancheLaPosta) {
    const chiocciola = p.indexOf("@");
    if (chiocciola > 0 && p.indexOf(".", chiocciola) > chiocciola + 1) return true;
  }
  if (èUnSito(s)) return true;
  const pezzi = p.split(INCOLLATE);
  return pezzi.length > 1 && pezzi.some((x) => èUnSito(senzaBordi(x)));
}

/**
 * Il testo a una riga sola, senza caratteri di controllo né invisibili e senza le parole che sono indirizzi. Una passata, tempo lineare: senza il
 * taglio a ESAME_MAX anche con centomila caratteri (le prove lo misurano qui, la lunghezza la limitano i due chiamanti).
 */
export function pulisciTesto(testo: string, ancheLaPosta: boolean): string {
  return testo
    .replace(CONTROLLO, " ")
    .replace(FORMATO, "")
    .split(/\s+/)
    .filter((parola) => parola !== "" && !èUnIndirizzo(parola, ancheLaPosta))
    .join(" ");
}

/** Toglie da capo e da coda spazi e separatori (, ; : | / -) con due indici: una regex con $ ripartirebbe da ogni posizione di una fila lunga. */
function rifilato(s: string): string {
  const separatore = (c: number) => c === 32 || c === 44 || c === 59 || c === 58 || c === 124 || c === 47 || c === 45;
  let i = 0;
  let j = s.length;
  while (i < j && separatore(s.charCodeAt(i))) i++;
  while (j > i && separatore(s.charCodeAt(j - 1))) j--;
  return s.slice(i, j);
}

const tagliato = (testo: string, max: number) => (testo.length > max ? [...testo].slice(0, max).join("").trimEnd() : testo);

/** Il titolo come esce nel livello con i titoli: senza link (un link incollato nel titolo darebbe accesso a una riunione), a una riga sola e non troppo lungo. */
export function titoloEsportabile(titolo: unknown): string {
  const t = pulisciTesto(String(titolo ?? "").slice(0, ESAME_MAX), false);
  return tagliato(t, TITOLO_MAX) || "(senza titolo)";
}

/**
 * Il luogo come esce nel livello con i titoli: senza link né indirizzi di posta, e vuoto se resta solo l'etichetta di una riunione online
 * ("Microsoft Teams Meeting": non dice dove) o se parla di ID, codici, PIN o numeri per collegarsi: chi ha il link a una riunione ha anche quelli.
 */
export function luogoEsportabile(luogo: unknown): string {
  const t = rifilato(pulisciTesto(String(luogo ?? "").slice(0, ESAME_MAX), true).replaceAll(" ,", ",").replaceAll(" ;", ";").replaceAll(" :", ":"));
  return DATI_DI_ACCESSO.test(t) || SOLO_CONFERENZA.test(t) ? "" : tagliato(t, LUOGO_MAX);
}

// ---- La lettura ---------------------------------------------------------------------------------------------------------------
const due = (n: number) => String(n).padStart(2, "0");
const data = (n: Naive) => `${String(n.a).padStart(4, "0")}-${due(n.m)}-${due(n.g)}`;

/** Un giorno intero come date AAAA-MM-GG con la fine esclusa: l'agente dà la mezzanotte del giorno dopo, un evento locale la fine nello stesso giorno. */
function giornoIntero(e: Ev): { inizio: string; fine: string } | null {
  let ini: Naive, fin: Naive;
  try {
    ini = naiveLocale(String(e.start));
    fin = naiveLocale(String(e.end || e.start));
  } catch {
    return null;
  }
  const inizio = data(ini);
  const finisceAMezzanotte = fin.h === 0 && fin.mi === 0 && fin.s === 0 && fin.us === 0;
  let fine = finisceAMezzanotte && data(fin) > inizio ? data(fin) : spostaData(data(fin), 1);
  if (fine <= inizio) fine = spostaData(inizio, 1);
  return { inizio, fine };
}

/** Solo quello che nasce qui: una casella letta dall'agente o un evento personale. Tutto il resto (link ICS, festività) non esce. */
const nostro = (e: Ev) => e.remoto === true || e.is_local === true;

/** Gli eventi di un blocco nel livello con i titoli, ognuno ridotto a orari, titolo e luogo. */
function eventiDelBlocco(eventi: Ev[], membri: Set<string> | null): EventoLetto[] {
  const out: EventoLetto[] = [];
  for (const e of eventi) {
    if (membri !== null && !nelGruppo(e, membri)) continue;
    if (!e.start) continue;
    const titolo = titoloEsportabile(e.title);
    const luogo = luogoEsportabile(e.location);
    if (e.allDay) {
      const g = giornoIntero(e);
      if (g) out.push({ ...g, tuttoGiorno: true, titolo, luogo });
      continue;
    }
    let ini: Naive, fin: Naive;
    try {
      ini = naiveLocale(String(e.start));
      fin = naiveLocale(String(e.end || e.start));
    } catch {
      continue;
    }
    out.push({ inizio: isoSecondi(ini), fine: isoSecondi(chiave(fin) > chiave(ini) ? fin : ini), tuttoGiorno: false, titolo, luogo });
  }
  return out;
}

/**
 * Le aziende che il link pubblica e che questa lettura dà in errore: un calendar.db che non si legge, un accesso scaduto, il servizio che aggiorna il
 * telefono fermo. Senza i loro eventi il calendario sarebbe incompleto, e per chi lo importa incompleto vuol dire "libero" per mezz'ora. Si decide dagli
 * stessi stati che la pagina mostra con il pallino rosso (meta.stato_aziende, per azienda il peggiore dei suoi stati): quelli di avviso (telefono
 * bloccato, aggiornamento in ritardo, in pausa) no, i dati ci sono. Con "tutti" contano le aziende accese, con un gruppo quelle del gruppo: un'azienda
 * fuori da quello che il link pubblica può essere in errore senza che il link se ne accorga.
 */
function aziendeInErrore(cfg: Config, meta: Ev, errori: string[], membri: Set<string> | null): string[] {
  const dentro = (nome: string, accesa: boolean) => (membri === null ? accesa : membri.has(nome));
  const guaste = new Set<string>();
  for (const [azienda, s] of Object.entries((meta.stato_aziende ?? {}) as Record<string, { stato?: string }>)) {
    if (s?.stato === "errore" && dentro(azienda, Boolean(meta.aziende?.[azienda]))) guaste.add(azienda);
  }
  // le aziende inoltrate hanno le copie delle riunioni nel calendario di un'altra casella: se quella non si legge, mancano
  for (const c of calendariDiInoltro(cfg)) {
    const nome = String(c.alias || c.account);
    if (c.stato === "errore" && dentro(nome, Boolean(c.enabled))) guaste.add(nome);
  }
  // gli eventi personali stanno su questo disco: si rompono solo se uno di loro non si legge
  const spenti = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  if (errori.some((e) => e.startsWith(`${ACCOUNT_LOCALE}:`)) && dentro(ACCOUNT_LOCALE, !spenti.has(ID_CALENDARIO))) guaste.add(ACCOUNT_LOCALE);
  return [...guaste];
}

type Blocco = { impegni: string[][]; eventi: EventoLetto[] };

async function leggiBlocco(cfg: Config, inizioBlocco: string, livello: Livello, membri: Set<string> | null): Promise<Blocco> {
  const [a, m, g] = inizioBlocco.split("-").map(Number);
  const inizio = mezzanotte(a, m, g);
  // allowCache false: niente copia vecchia su disco, e niente scritture; conFeed false: i calendari ricevuti da altri non si leggono nemmeno
  const [letti, errori, meta] = await raccogli(cfg, iso(inizio), iso(piuGiorni(inizio, GIORNI_BLOCCO)), false, false);
  // l'agente non ha risposto: senza dati non si pubblica un calendario vuoto, che per chi lo importa vorrebbe dire "libero"
  if (meta.outlook_starting) throw new LetturaNonDisponibile("l'agente non risponde");
  // l'agente ha risposto ma una delle aziende pubblicate no (200 con l'errore dentro): stesso discorso, i suoi eventi mancherebbero in silenzio
  const guaste = aziendeInErrore(cfg, meta, errori, membri);
  if (guaste.length) throw new LetturaNonDisponibile(`non si leggono: ${guaste.join(", ")}`);
  const eventi = letti.filter(nostro);
  return livello === "occupato" ? { impegni: impegni(eventi, { a, m, g }, GIORNI_BLOCCO, membri), eventi: [] } : { impegni: [], eventi: eventiDelBlocco(eventi, membri) };
}

/**
 * Quello che il link pubblica oggi. Con "tutti" valgono gli interruttori della barra laterale (le aziende spente non escono), con un gruppo
 * contano le aziende del gruppo e basta, come in /disponibilita?gruppo=. Solleva LetturaNonDisponibile se l'agente non risponde, se un'azienda pubblicata
 * non si legge, se il gruppo non c'è o se gli eventi sono più di EVENTI_MAX.
 */
export async function leggiPerCondivisione(link: Condivisione, oggi: string): Promise<Lettura> {
  const { da, a } = finestra(link, oggi);
  const config = loadConfig();
  let membri: Set<string> | null = null;
  if (link.ambito === "gruppo") {
    membri = membriGruppo(config, link.gruppo);
    if (membri === null) throw new LetturaNonDisponibile("gruppo non trovato");
  }
  // una copia di lavoro che non va su disco (le sorgenti salvano la configurazione quando assegnano un colore)
  const cfg: Config = { ...config, _sola_lettura: true, ...(membri ? { disabled_calendars: [] } : {}) };
  const inizi = blocchi(da, a);
  const letti: Blocco[] = [];
  for (let i = 0; i < inizi.length; i += BLOCCHI_INSIEME) {
    letti.push(...(await Promise.all(inizi.slice(i, i + BLOCCHI_INSIEME).map((b) => leggiBlocco(cfg, b, link.livello, membri)))));
  }
  const inizioFinestra = `${da}T00:00:00`;
  const fineFinestra = `${a}T00:00:00`;
  // un impegno lungo a cavallo di due blocchi arriva due volte: si uniscono, e si tengono quelli che toccano la finestra
  const occupati = unisci(letti.flatMap((b) => b.impegni) as [string, string][]).filter(([i, f]) => f > inizioFinestra && i < fineFinestra);
  const visti = new Set<string>();
  const eventi = letti
    .flatMap((b) => b.eventi)
    .filter((e) => {
      const dentro = e.tuttoGiorno ? e.fine > da && e.inizio < a : (e.fine > inizioFinestra || (e.fine === e.inizio && e.inizio >= inizioFinestra)) && e.inizio < fineFinestra;
      // lo stesso evento nei due blocchi che attraversa, con lo stesso titolo e lo stesso orario, è uno solo (come in senzaDoppioni)
      const k = JSON.stringify([e.titolo.toLowerCase(), e.inizio, e.fine, e.tuttoGiorno]);
      if (!dentro || visti.has(k)) return false;
      visti.add(k);
      return true;
    })
    .sort((x, y) => (x.inizio < y.inizio ? -1 : x.inizio > y.inizio ? 1 : x.fine < y.fine ? -1 : x.fine > y.fine ? 1 : x.titolo < y.titolo ? -1 : x.titolo > y.titolo ? 1 : 0));
  // tagliare in silenzio vorrebbe dire pubblicare i primi eventi e dare per liberi i mesi dopo: meglio non rispondere, e dire di accorciare i giorni
  if (occupati.length + eventi.length > EVENTI_MAX) throw new LetturaNonDisponibile(`più di ${EVENTI_MAX} eventi nella finestra: servono meno giorni`);
  return { livello: link.livello, da, a, impegni: occupati, eventi };
}

/** Gli eventi di una lettura come li scrive il calendario: gli orari diventano istanti UTC, i giorni interi restano date. */
export function inEventiUscita(l: Lettura): EventoUscita[] {
  const ora = (testo: string) => istanteDaRoma(leggiNaive(testo));
  if (l.livello === "occupato") return l.impegni.map(([i, f]) => ({ tipo: "ora", inizio: ora(i), fine: ora(f), titolo: "", luogo: "" }));
  return l.eventi.map((e): EventoUscita => (e.tuttoGiorno ? { tipo: "giorno", inizio: e.inizio, fine: e.fine, titolo: e.titolo, luogo: e.luogo } : { tipo: "ora", inizio: ora(e.inizio), fine: ora(e.fine), titolo: e.titolo, luogo: e.luogo }));
}
