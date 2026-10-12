import { AVANTI, INDIETRO, type Livello } from "../condivisione-comune";
import { blocchi, GIORNI_BLOCCO, spostaData, unisci } from "../disponibilita";
import { type EventoUscita, istanteDaRoma } from "../ics-uscita";
import type { Ev } from "./caselle";
import type { Condivisione } from "./condivisione";
import { raccogli } from "./eventi";
import { chiave, iso, isoSecondi, leggiNaive, mezzanotte, naiveLocale, piuGiorni, type Naive } from "./fuso";
import { impegni } from "./impegni";
import { type Config, loadConfig, membriGruppo } from "./stato";

// Cosa c'è nel calendario di un link: gli stessi eventi che l'app mostra (raccogli, lo stesso di get_events), letti a blocchi di 28 giorni come la
// disponibilità pubblica, e ridotti a quello che il livello del link permette. Qui si decide cosa può uscire: i calendari ricevuti da altri (link ICS,
// festività, i calendari condivisi da un altro OE Copilot) non escono mai, così due istanze che si importano a vicenda non si rimandano i propri
// eventi in giro; di un evento escono gli orari e, nel livello con i titoli, il titolo e il luogo. Mai la descrizione, chi organizza e chi partecipa,
// i link per collegarsi, il nome di un'azienda o di una casella.

/** Il lavoro di un blocco al più per volta: ognuno è una richiesta all'agente. */
const BLOCCHI_INSIEME = 4;
/** Quanti eventi al più in un calendario condiviso: oltre, si scrive nel log e si pubblicano i primi. */
export const EVENTI_MAX = 5000;
const TITOLO_MAX = 250;
const LUOGO_MAX = 250;

/** Un evento nel livello con i titoli, ancora in ora di Roma (come lo dà get_events): AAAA-MM-GGTHH:MM:SS, o AAAA-MM-GG con la fine esclusa per un giorno intero. */
export type EventoLetto = { inizio: string; fine: string; tuttoGiorno: boolean; titolo: string; luogo: string };
export type Lettura = { livello: Livello; da: string; a: string; impegni: [string, string][]; eventi: EventoLetto[] };

/** Il calendario non si può leggere adesso (l'agente non risponde, il gruppo non c'è più): chi lo chiede riprova fra poco. */
export class LetturaNonDisponibile extends Error {}

const limita = (v: number, min: number, max: number, predefinito: number) => (Number.isFinite(v) ? Math.min(max, Math.max(min, Math.trunc(v))) : predefinito);

/** Dal giorno `da` al giorno `a` (escluso): i giorni indietro e avanti di un link, tenuti nei limiti anche se nel database ci fosse un valore fuori. */
export function finestra(c: Pick<Condivisione, "giorni_indietro" | "giorni_avanti">, oggi: string): { da: string; a: string } {
  const indietro = limita(c.giorni_indietro, INDIETRO.min, INDIETRO.max, INDIETRO.predefinito);
  const avanti = limita(c.giorni_avanti, AVANTI.min, AVANTI.max, AVANTI.predefinito);
  return { da: spostaData(oggi, -indietro), a: spostaData(oggi, avanti + 1) };
}

// ---- Cosa può uscire di un titolo e di un luogo ------------------------------------------------------------------------------
const SCHEMI = "(?:[a-z][a-z0-9+.-]*:\\/\\/|(?:mailto|tel|sip|sips|callto|skype|msteams|zoommtg|webex|facetime|sms):)";
const LINK = new RegExp(`\\b${SCHEMI}[^\\s<>"')\\]]+|\\bwww\\.[^\\s<>"')\\]]+`, "gi");
const SITI_CONFERENZA = /\b(?:[a-z0-9-]+\.)*(?:teams\.microsoft\.com|teams\.live\.com|zoom\.us|zoomgov\.com|meet\.google\.com|webex\.com|gotomeet(?:ing)?\.(?:com|me)|join\.skype\.com|whereby\.com|bluejeans\.com|chime\.aws|meet\.jit\.si)\b[^\s<>"')\]]*/gi;
const INDIRIZZO_POSTA = /[\w.%+-]+@[\w-]+(?:\.[\w-]+)+/g;
// il testo che i programmi di posta mettono nel luogo di una riunione online: dice solo che è online
const SOLO_CONFERENZA = /^(?:(?:riunione|incontro|meeting)\s+(?:di\s+)?)?(?:microsoft\s+teams|ms\s+teams|teams|zoom|google\s+meet|webex|skype(?:\s+for\s+business)?|gotomeeting|online|virtuale)(?:\s+(?:meeting|riunione|call|chiamata|incontro))?$/i;
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g;

const tagliato = (testo: string, max: number) => (testo.length > max ? [...testo].slice(0, max).join("").trimEnd() : testo);

/** Il titolo come esce nel livello con i titoli: senza link (un link incollato nel titolo darebbe accesso a una riunione), a una riga sola e non troppo lungo. */
export function titoloEsportabile(titolo: unknown): string {
  const t = String(titolo ?? "")
    .replace(CONTROLLO, " ")
    .replace(LINK, "")
    .replace(SITI_CONFERENZA, "")
    .replace(/\s{2,}/g, " ")
    .trim();
  return tagliato(t, TITOLO_MAX) || "(senza titolo)";
}

/**
 * Il luogo come esce nel livello con i titoli: senza link né indirizzi di posta, e vuoto se resta solo l'etichetta di una riunione online
 * ("Microsoft Teams Meeting"): quella non dice dove, e il link per collegarsi non deve uscire.
 */
export function luogoEsportabile(luogo: unknown): string {
  const t = String(luogo ?? "")
    .replace(CONTROLLO, " ")
    .replace(LINK, " ")
    .replace(SITI_CONFERENZA, " ")
    .replace(INDIRIZZO_POSTA, " ")
    .replace(/\s{2,}/g, " ")
    .replace(/\s+([,;:])/g, "$1")
    .replace(/^[\s,;:|/-]+|[\s,;:|/-]+$/g, "");
  return SOLO_CONFERENZA.test(t) ? "" : tagliato(t, LUOGO_MAX);
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
    if (membri !== null && !membri.has(e.alias || e.account)) continue;
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

type Blocco = { impegni: string[][]; eventi: EventoLetto[] };

async function leggiBlocco(cfg: Config, inizioBlocco: string, livello: Livello, membri: Set<string> | null): Promise<Blocco> {
  const [a, m, g] = inizioBlocco.split("-").map(Number);
  const inizio = mezzanotte(a, m, g);
  // allowCache false: niente copia vecchia su disco, e niente scritture; conFeed false: i calendari ricevuti da altri non si leggono nemmeno
  const [letti, , meta] = await raccogli(cfg, iso(inizio), iso(piuGiorni(inizio, GIORNI_BLOCCO)), false, false);
  // l'agente non ha risposto: senza dati non si pubblica un calendario vuoto, che per chi lo importa vorrebbe dire "libero"
  if (meta.outlook_starting) throw new LetturaNonDisponibile("l'agente non risponde");
  const eventi = letti.filter(nostro);
  return livello === "occupato" ? { impegni: impegni(eventi, { a, m, g }, GIORNI_BLOCCO, membri), eventi: [] } : { impegni: [], eventi: eventiDelBlocco(eventi, membri) };
}

/**
 * Quello che il link pubblica oggi. Con "tutti" valgono gli interruttori della barra laterale (le aziende spente non escono), con un gruppo
 * contano le aziende del gruppo e basta, come in /disponibilita?gruppo=. Solleva LetturaNonDisponibile se l'agente non risponde o se il gruppo non c'è.
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
  if (occupati.length + eventi.length > EVENTI_MAX) console.warn(`condivisione ${link.id}: ${occupati.length + eventi.length} eventi, se ne pubblicano ${EVENTI_MAX}`);
  return { livello: link.livello, da, a, impegni: occupati.slice(0, EVENTI_MAX), eventi: eventi.slice(0, EVENTI_MAX) };
}

/** Gli eventi di una lettura come li scrive il calendario: gli orari diventano istanti UTC, i giorni interi restano date. */
export function inEventiUscita(l: Lettura): EventoUscita[] {
  const ora = (testo: string) => istanteDaRoma(leggiNaive(testo));
  if (l.livello === "occupato") return l.impegni.map(([i, f]) => ({ tipo: "ora", inizio: ora(i), fine: ora(f), titolo: "", luogo: "" }));
  return l.eventi.map((e): EventoUscita => (e.tuttoGiorno ? { tipo: "giorno", inizio: e.inizio, fine: e.fine, titolo: e.titolo, luogo: e.luogo } : { tipo: "ora", inizio: ora(e.inizio), fine: ora(e.fine), titolo: e.titolo, luogo: e.luogo }));
}
