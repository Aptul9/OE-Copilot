import { calendariDiInoltro, elencoCalendari, tuttiGliEventi, type Meta } from "./agente";
import { ANNULLATO_RE, cedeIlPosto, titoloBase, type Ev } from "./caselle";
import { ErroreData, iso, mezzanotte, oraHHMM, parseBound, piuGiorni } from "./fuso";
import { elencoFeed, eventiFeed } from "./ics";
import { elencoLocali, eventiLocali } from "./locali";
import { impegni, festivita } from "./impegni";
import { adesso } from "./orologio";
import { FESTIVITA_NOME, personalizzazione } from "./personalizzazione";
import { messaggio } from "./py";
import { membriGruppo, type Config } from "./stato";

// La raccolta degli eventi da tutte le sorgenti, in un posto solo (era eventi.py, fino al commit 142e789): le caselle lette dall'agente, i link ICS e il
// calendario locale. Vanno lette insieme, non a due a due, così le due strade (la pagina e la disponibilità) non possono divergere.

const GRAVITA: Record<string, number> = { ok: 0, avviso: 1, errore: 2 };

/** I calendari delle caselle; solleva AgenteNonDisponibile. */
export const calendariCaselle = (cfg: Config, force = false) => elencoCalendari(cfg, force);

/**
 * Le voci della barra laterale che non sono calendari delle caselle: un elenco solo, usato da tutti, perché una sorgente
 * dimenticata in uno solo dei punti si accenderebbe ma non si spegnerebbe.
 */
export const calendariExtra = (cfg: Config): Ev[] => [...elencoFeed(cfg), ...elencoLocali(cfg), ...calendariDiInoltro(cfg)];

/**
 * Toglie lo stesso appuntamento arrivato da più fonti, anche con "FW: " o "RE: " davanti a uno dei titoli. Si tiene la copia della casella,
 * ma se la stessa riunione arriva ANCHE dal calendario Google di un'azienda (l'agente marca is_google), si adotta la sua etichetta e il suo colore.
 */
export function senzaDoppioni(eventi: Ev[]): Ev[] {
  const indice = new Map<string, number>();
  const out: Ev[] = [];
  for (const e of eventi) {
    const chiave = JSON.stringify([titoloBase(String(e.title || "").trim()).toLowerCase(), e.start ?? null, e.end ?? null]);
    const posto = indice.get(chiave);
    if (posto === undefined) {
      indice.set(chiave, out.length);
      out.push(e);
      continue;
    }
    const tenuto = out[posto];
    if ((tenuto.inoltrata_da && !e.inoltrata_da) || cedeIlPosto(tenuto, e)) {
      // La copia di un flusso cede il posto all'invito vero, che ha il link per partecipare e l'organizzatore reale; un invito inoltrato cede a qualunque altra copia
      out[posto] = e;
      continue;
    }
    if (e.is_google && !tenuto.is_google) {
      tenuto.alias = "alias" in e ? e.alias : tenuto.alias;
      tenuto.color = "color" in e ? e.color : tenuto.color;
    }
  }
  return out;
}

/** Come è andata QUESTA lettura, azienda per azienda: il pallino accanto a ogni calendario. Se un'azienda ha più sorgenti vince la peggiore. */
export function statoFonti(cfg: Config, meta: Meta, errori: string[] = []): Record<string, { stato: string; msg: string }> {
  const ora = oraHHMM(adesso());
  // L'agente dice da sé com'è andata, azienda per azienda: sa se una casella manca o se l'aggiornamento sul server è fermo
  const propri = (meta.stato_aziende || {}) as Record<string, { stato: string; msg: string }>;
  const fonti: Record<string, { stato: string; msg: string }> = {};
  for (const azienda of Object.keys(meta.aziende || {})) fonti[azienda] = { ...(propri[azienda] || { stato: "ok", msg: `Letto alle ${ora}` }) };
  let altri: Ev[];
  try {
    altri = calendariExtra(cfg);
  } catch (e) {
    console.error("stato fonti: elenco calendari non leggibile", e);
    return fonti;
  }
  for (const c of altri) {
    const nome = c.alias || c.account;
    const stato = c.stato || "ok";
    const msg = c.stato_msg || "";
    const prec = fonti[nome];
    if (!prec) {
      fonti[nome] = { stato, msg };
      continue;
    }
    // Più sorgenti per la stessa azienda: si tengono tutti i messaggi e vince lo stato peggiore
    fonti[nome] = {
      stato: (GRAVITA[stato] ?? 0) > (GRAVITA[prec.stato] ?? 0) ? stato : prec.stato,
      msg: [prec.msg, msg].filter(Boolean).join(" · "),
    };
  }
  // Se un errore nomina un'azienda, il suo pallino DEVE essere rosso
  for (const testo of errori) {
    const i = String(testo).indexOf(":");
    const nome = (i === -1 ? String(testo) : String(testo).slice(0, i)).trim();
    const dettaglio = i === -1 ? "" : String(testo).slice(i + 1);
    if (nome in fonti) fonti[nome] = { stato: "errore", msg: dettaglio.trim() || nome };
  }
  return fonti;
}

/**
 * Toglie gli eventi delle aziende spente in "I miei calendari". Il filtro guarda l'AZIENDA mostrata, non la cartella che ospita
 * l'evento: un invito archiviato nel calendario predefinito, che è di un'altra azienda, segue l'interruttore della sua azienda.
 */
function soloAziendeAccese(cfg: Config, eventi: Ev[], aziendeCaselle: Record<string, boolean>): Ev[] {
  const acceso: Record<string, boolean> = { ...aziendeCaselle };
  for (const c of calendariExtra(cfg)) {
    const nome = c.alias || c.account;
    acceso[nome] = Boolean(acceso[nome]) || Boolean(c.enabled);
  }
  const spente = new Set(Object.entries(acceso).filter(([, on]) => !on).map(([nome]) => nome));
  if (!spente.size) return eventi;
  return eventi.filter((e) => !spente.has(e.alias || e.account));
}

/**
 * Tutti gli eventi dell'intervallo, da ogni sorgente. -> (eventi, errori, meta). `allowCache` false vieta di rispondere con l'ultima
 * lettura salvata: serve alla disponibilità, che non deve dire "libero" per dati vecchi. `conFeed` false lascia fuori i calendari da link ICS
 * (anche le festività): la condivisione del calendario non pubblica mai quello che l'app ha ricevuto da altri.
 */
export async function raccogli(cfg: Config, startIso: string, endIso: string, allowCache = true, conFeed = true): Promise<[Ev[], string[], Meta]> {
  let [eventi, errori, meta] = await tuttiGliEventi(cfg, startIso, endIso, allowCache);
  if (!personalizzazione().mostraTuttoIlGiorno) eventi = eventi.filter((e) => !e.allDay);
  let inizio, fine;
  try {
    inizio = parseBound(startIso);
    fine = parseBound(endIso);
  } catch (e) {
    if (!(e instanceof ErroreData)) throw e;
    return [eventi, [...errori, `Intervallo non valido: ${e.message}`], meta];
  }
  try {
    const [trovati, erroriFonte] = conFeed ? await eventiFeed(cfg, inizio, fine) : [[], []];
    eventi = [...eventi, ...trovati];
    errori = [...errori, ...erroriFonte];
  } catch (e) {
    // Una sorgente rotta non deve portarsi via le altre
    console.error("raccogli: calendari da link falliti", e);
    errori.push(`Calendari da link: ${messaggio(e)}`);
  }
  try {
    eventi = [...eventi, ...eventiLocali(cfg, inizio, fine)];
  } catch (e) {
    console.error("raccogli: eventi locali falliti", e);
    errori.push(`Eventi personali: ${messaggio(e)}`);
  }
  // Una riunione annullata resta nel calendario con "Canceled:" nel titolo: qui, che è la strada di pagina, disponibilità, condivisione e promemoria, non esce
  eventi = senzaDoppioni(eventi.filter((e) => !ANNULLATO_RE.test(String(e.title || ""))));
  eventi = soloAziendeAccese(cfg, eventi, (meta.aziende || {}) as Record<string, boolean>);
  eventi.sort((a, b) => ((a.start || "") < (b.start || "") ? -1 : (a.start || "") > (b.start || "") ? 1 : 0));
  meta.fonti = statoFonti(cfg, meta, errori);
  return [eventi, errori, meta];
}

/**
 * Cosa può sapere la pagina pubblica dei `giorni` giorni da `primoGiorno`: (impegni, festività, completo), solo orari, mai cosa c'è.
 * Contano tutti i calendari, accesi o spenti nella barra laterale: gli interruttori sono un filtro della vista, e chi guarda la
 * disponibilità non deve vedere "libero" perché ieri sera si è lasciato acceso un solo calendario. Con `gruppo` contano solo le
 * aziende di quel gruppo (Error se non esiste). `completo` è falso se una sorgente non ha risposto bene.
 */
export async function occupato(cfg: Config, primoGiorno: { a: number; m: number; g: number }, giorni: number, gruppo = ""): Promise<[string[][], string[][], boolean]> {
  let membri: Set<string> | null = null;
  if (gruppo) {
    membri = membriGruppo(cfg, gruppo);
    if (membri === null) throw new Error("Gruppo non trovato");
  }
  // Copia di lavoro: senza interruttori e che non va su disco (le sorgenti salvano la configurazione quando assegnano un colore,
  // e vi scriverebbero gli interruttori vuoti)
  const copia: Config = { ...cfg, disabled_calendars: [], _sola_lettura: true };
  const inizio = mezzanotte(primoGiorno.a, primoGiorno.m, primoGiorno.g);
  const [eventi, errori, meta] = await raccogli(copia, iso(inizio), iso(piuGiorni(inizio, giorni)), false);
  const completo = !errori.length && !meta.outlook_starting && !meta.stale;
  // il calendario che un altro OE Copilot ha condiviso con noi non è un impegno nostro: non rende occupata la pagina pubblica
  return [impegni(eventi.filter((e) => !e.da_condivisione), primoGiorno, giorni, membri), festivita(eventi, FESTIVITA_NOME), completo];
}
