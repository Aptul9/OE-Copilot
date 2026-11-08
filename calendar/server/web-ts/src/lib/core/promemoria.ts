import type Database from "better-sqlite3";
import type { Ev } from "./caselle";
import { archivio, type Config, loadConfig, modificaConfig } from "./archivio";
import { raccogli, tuttiAccesi } from "./eventi";
import { chiave as microsecondi, daIstante, daSecondi, iso, isoSecondi, naiveLocale, piuGiorni, type Naive } from "./fuso";
import { adesso } from "./orologio";
import { type Canale, canaliDelServer, type Esito } from "./promemoria-canali";
import { isDict } from "./py";

// I promemoria delle riunioni. Le impostazioni (voce `promemoria` della configurazione) sono una sola per tutti: quando avvisare, cosa
// scrivere, il suono e i due avvisi dell'app per Windows, che le rilegge a ogni giro e fa da sé notifica e avviso a comparsa. Il server
// manda i promemoria dei canali configurati (promemoria-canali.ts, le notifiche dei browser iscritti): a ogni giro legge gli eventi che
// stanno per iniziare dalla stessa strada di get_events e manda quello che serve. Quello che è già partito sta nella tabella
// promemoria_inviati di gateway.db, così un riavvio del gateway non rimanda niente.

export const TENTATIVI_MAX = 5;
export const CONSERVA_GIORNI = 14;
export const MINUTI_MAX = 1440;
export const MINUTI_VOCI = 8;
// i suoni di sistema che l'app per Windows può chiedere alla notifica (tauri-winrt-notification): sul server si sceglie, ma li usa solo l'app
const dieci = (nome: string) => [nome, ...[2, 3, 4, 5, 6, 7, 8, 9, 10].map((n) => nome + n)];
export const SUONI = ["Default", "Reminder", "Mail", "IM", "SMS", ...dieci("Alarm"), ...dieci("Call")];
export const TESTO_PROVA = "Prova dei promemoria";
// si legge un po' oltre il minuto più lontano: un evento a cavallo del limite non sfugge fra due giri
const MARGINE_MIN = 2;
const ATTESA_MAX_S = 900;
// lo stesso problema si scrive nel log una volta ogni tanto, non a ogni giro
const LOG_OGNI_MS = 10 * 60_000;
const INTERVALLO_PREDEFINITO_S = 30;
const PRIMO_GIRO_MS = 10_000;

// ---- Le impostazioni -----------------------------------------------------------------------------------------------------
/** "minimo": solo l'ora e l'azienda, in ogni canale (notifica e avviso a comparsa dell'app, notifiche del browser). */
export type Contenuto = "completo" | "minimo";
export type Impostazioni = { minuti: number[]; contenuto: Contenuto; suono: string; notifica_windows: boolean; comparsa: boolean };

/**
 * Quello che vale finché nessuno ha salvato niente: un avviso dieci minuti prima, testo completo, notifica e avviso a comparsa accesi
 * nell'app per Windows. Non stanno in DEFAULT_CONFIG: la chiave `promemoria` si scrive nello stato alla prima modifica, come `gruppi_calendari`.
 */
export const PREDEFINITE: Impostazioni = { minuti: [10], contenuto: "completo", suono: "Default", notifica_windows: true, comparsa: true };
const INTERRUTTORI = ["notifica_windows", "comparsa"] as const;

// 0 vuol dire "all'inizio": l'avviso parte nell'ultimo minuto prima dell'inizio (un giro dura fino a 30 secondi, quindi uno ci cade sempre)
export const sogliaMs = (minuti: number) => Math.max(minuti, 1) * 60_000;

function minutiValidi(v: unknown): number[] | null {
  if (!Array.isArray(v) || !v.every((m) => Number.isInteger(m) && m >= 0 && m <= MINUTI_MAX)) return null;
  const distinti = [...new Set(v as number[])].sort((a, b) => a - b);
  return distinti.length >= 1 && distinti.length <= MINUTI_VOCI ? distinti : null;
}

/**
 * Le impostazioni dallo stato, sempre usabili: un valore che non va (la voce si può scrivere anche con save_config) cede il posto al
 * predefinito. Le voci di prima (`attivo`, `canali`) non contano più e spariscono al primo salvataggio.
 */
export function leggiImpostazioni(cfg: Config): Impostazioni {
  const g = isDict(cfg.promemoria) ? cfg.promemoria : {};
  return {
    minuti: minutiValidi(g.minuti) ?? [...PREDEFINITE.minuti],
    contenuto: g.contenuto === "minimo" ? "minimo" : "completo",
    suono: typeof g.suono === "string" && SUONI.includes(g.suono) ? g.suono : PREDEFINITE.suono,
    notifica_windows: g.notifica_windows !== false,
    comparsa: g.comparsa !== false,
  };
}

export type Rifiuto = { ok: false; error: string };

/** Le nuove impostazioni sopra le attuali: un campo che manca resta com'è. */
export function validaImpostazioni(dati: unknown, attuali: Impostazioni): { ok: true; valore: Impostazioni } | Rifiuto {
  if (!isDict(dati)) return { ok: false, error: "Le impostazioni dei promemoria non sono valide." };
  const nuove: Impostazioni = { ...attuali };
  for (const [campo, v] of Object.entries(dati)) {
    if (campo === "minuti") {
      const minuti = minutiValidi(v);
      if (!minuti) return { ok: false, error: `Minuti: da 1 a ${MINUTI_VOCI} numeri interi diversi, da 0 a ${MINUTI_MAX}.` };
      nuove.minuti = minuti;
    } else if (campo === "contenuto") {
      if (v !== "completo" && v !== "minimo") return { ok: false, error: "Il contenuto è «completo» o «minimo»." };
      nuove.contenuto = v;
    } else if (campo === "suono") {
      if (typeof v !== "string" || !SUONI.includes(v)) return { ok: false, error: "Suono sconosciuto." };
      nuove.suono = v;
    } else if ((INTERRUTTORI as readonly string[]).includes(campo)) {
      if (typeof v !== "boolean") return { ok: false, error: `«${campo}» deve essere vero o falso.` };
      nuove[campo as (typeof INTERRUTTORI)[number]] = v;
    } else return { ok: false, error: "Impostazione sconosciuta." };
  }
  return { ok: true, valore: nuove };
}

// ---- Il tempo ------------------------------------------------------------------------------------------------------------
const due = (n: number) => String(n).padStart(2, "0");

/** L'istante (millisecondi dall'epoca) di un'ora di Roma. Nell'ora che il cambio ripete vale la prima, in quella che salta vale come prima del salto. */
export function istanteDaRoma(n: Naive): number {
  const locale = Date.UTC(n.a, n.m - 1, n.g, n.h, n.mi, n.s);
  const voluta = microsecondi({ ...n, us: 0 });
  for (const scartoMin of [120, 60]) {
    const ms = locale - scartoMin * 60_000;
    if (microsecondi(daIstante(ms)) === voluta) return ms;
  }
  return locale - 60 * 60_000;
}

// ---- Il testo ------------------------------------------------------------------------------------------------------------
const durata = (min: number) => {
  if (min < 120) return `${min} min`;
  const resto = min % 60;
  return `${Math.floor(min / 60)} ore` + (resto ? ` e ${resto} min` : "");
};

function quando(inizio: Naive, adessoMs: number): string {
  const oggi = daIstante(adessoMs);
  const stesso = (a: Naive, b: Naive) => a.a === b.a && a.m === b.m && a.g === b.g;
  const ora = `alle ${due(inizio.h)}:${due(inizio.mi)}`;
  if (stesso(inizio, oggi)) return ora;
  return stesso(inizio, piuGiorni(oggi, 1)) ? `domani ${ora}` : `il ${due(inizio.g)}/${due(inizio.m)} ${ora}`;
}

const pulito = (v: unknown) => (typeof v === "string" ? v.replace(/\s+/g, " ").trim() : "");

/**
 * Il testo del promemoria; la prima riga è il titolo della notifica. I minuti sono quelli che mancano davvero, arrotondati per eccesso:
 * un avviso in ritardo (dopo un riavvio, o per un evento creato all'ultimo) dice "Tra 4 min" e non il valore configurato. Completo: titolo
 * e luogo; minimo: solo l'etichetta dell'azienda. Il link per entrare non c'è, come nella notifica dell'app.
 */
export function testoPromemoria(ev: Ev, inizio: Naive, restantiMs: number, contenuto: Contenuto, adessoMs: number): string {
  const righe = [`Tra ${durata(Math.max(1, Math.ceil(restantiMs / 60_000)))}, ${quando(inizio, adessoMs)}`];
  if (contenuto === "minimo") {
    const azienda = pulito(ev.alias);
    if (azienda) righe.push(azienda);
  } else {
    righe.push(pulito(ev.title) || "(senza titolo)");
    const luogo = pulito(ev.location);
    if (luogo && !/https?:\/\//i.test(luogo)) righe.push(luogo);
  }
  return righe.join("\n");
}

// ---- Quello che è già partito --------------------------------------------------------------------------------------------
// Un esito è "inviato" (finito bene), "riprovo" (almeno un tentativo fallito, se ne farà un altro) o "fallito" (TENTATIVI_MAX falliti: non
// si riprova più). La chiave è (occorrenza, inizio, minuti, canale): un evento spostato ha un inizio nuovo e si avvisa di nuovo.
export type Riga = {
  chiave: string;
  evento: string;
  inizio: string;
  minuti: number;
  canale: string;
  inviatoIl: number;
  esito: "inviato" | "riprovo" | "fallito";
  tentativi: number;
  errore: string | null;
};

export type Store = {
  leggi(chiave: string): Riga | undefined;
  scrivi(righe: Riga[]): void;
  /** Toglie le righe più vecchie di `prima` (millisecondi dall'epoca); restituisce quante. */
  pulisci(prima: number): number;
};

const SCHEMA = `CREATE TABLE IF NOT EXISTS promemoria_inviati (
  chiave TEXT PRIMARY KEY,
  evento TEXT NOT NULL,
  inizio TEXT NOT NULL,
  minuti INTEGER NOT NULL,
  canale TEXT NOT NULL,
  inviato_il INTEGER NOT NULL,
  esito TEXT NOT NULL,
  tentativi INTEGER NOT NULL,
  errore TEXT
)`;

export const chiaveDi = (evento: string, inizio: string, minuti: number, canale: string) => JSON.stringify([evento, inizio, minuti, canale]);

/** La tabella nel database dello stato (gateway.db): si crea alla prima volta che serve, anche se la connessione viene riaperta altrove. */
export function creaStore(dbDi: () => Database.Database = archivio): Store {
  const pronti = new WeakSet<Database.Database>();
  const db = () => {
    const d = dbDi();
    if (!pronti.has(d)) {
      d.exec(SCHEMA);
      pronti.add(d);
    }
    return d;
  };
  return {
    leggi: (chiave) =>
      db()
        .prepare("SELECT chiave, evento, inizio, minuti, canale, inviato_il AS inviatoIl, esito, tentativi, errore FROM promemoria_inviati WHERE chiave = ?")
        .get(chiave) as Riga | undefined,
    scrivi: (righe) => {
      const d = db();
      const scrivi = d.prepare(
        `INSERT INTO promemoria_inviati(chiave, evento, inizio, minuti, canale, inviato_il, esito, tentativi, errore)
         VALUES (@chiave, @evento, @inizio, @minuti, @canale, @inviatoIl, @esito, @tentativi, @errore)
         ON CONFLICT(chiave) DO UPDATE SET inviato_il = excluded.inviato_il, esito = excluded.esito, tentativi = excluded.tentativi, errore = excluded.errore`,
      );
      d.transaction(() => righe.forEach((r) => scrivi.run(r)))();
    },
    pulisci: (prima) => db().prepare("DELETE FROM promemoria_inviati WHERE inviato_il < ?").run(prima).changes,
  };
}

// ---- Il motore -----------------------------------------------------------------------------------------------------------
export type StatoMotore = { ultimoGiro: number | null; ultimoErrore: { testo: string; alle: number } | null };

export type Dipendenze = {
  /** L'ora: orologio.ts, così le prove la fissano. */
  adesso: () => number;
  /** Gli eventi fra due ore locali (ISO senza scarto), dalla lettura del gateway, e quante sorgenti hanno avuto problemi. Solleva se la lettura fallisce. */
  leggiEventi: (inizio: string, fine: string) => Promise<{ eventi: Ev[]; problemi: number }>;
  /** Le impostazioni, lette a ogni giro. */
  impostazioni: () => Impostazioni;
  canali: () => Canale[];
  store: Store;
  stato?: StatoMotore;
};

export type Riepilogo = { letti: number; inviati: number; falliti: number; abbandonati: number };
export type Motore = { giro(): Promise<Riepilogo>; stato: StatoMotore };

type Candidato = { ev: Ev; id: string; inizio: string; naive: Naive; istante: number };

/**
 * Gli eventi che possono avere un promemoria: a orario (un giorno intero no, e nemmeno una voce nascosta, come le festività di un
 * calendario spento), non ancora iniziati, con un inizio che si legge. Un evento annullato o cancellato non arriva qui: l'agente non lo
 * dà (stato annullato e righe cancellate sono scartati alla lettura del telefono, le occorrenze annullate di una serie diventano
 * esclusioni); non si distingue invece un evento libero, provvisorio o rifiutato: l'agente non legge quei campi. In ordine di inizio.
 */
function candidati(eventi: Ev[], ora: number): Candidato[] {
  const visti = new Set<string>();
  const out: Candidato[] = [];
  for (const ev of eventi) {
    if (ev.allDay || ev.nascosto || !ev.start) continue;
    let naive: Naive;
    try {
      naive = naiveLocale(String(ev.start));
    } catch {
      continue;
    }
    const istante = istanteDaRoma(naive);
    if (istante <= ora) continue;
    const id = String(ev.id ?? `${ev.account ?? ""}|${ev.start}`);
    const inizio = isoSecondi(naive);
    const chiave = JSON.stringify([id, inizio]);
    if (visti.has(chiave)) continue;
    visti.add(chiave);
    out.push({ ev, id, inizio, naive, istante });
  }
  return out.sort((a, b) => a.istante - b.istante || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/**
 * Un giro: se c'è un canale pronto legge le impostazioni e gli eventi dei prossimi minuti e manda quello che serve.
 * Per ogni evento e canale parte al più un messaggio a giro: se più valori dei minuti sono scaduti insieme (primo giro dopo un riavvio, evento
 * creato all'ultimo) il testo è lo stesso, e li chiude tutti. Un invio che fallisce non segna niente come inviato e ferma quel canale fino
 * al giro dopo (con il limite del servizio, per il tempo che dice): ai tentativi si arriva un giro alla volta, dall'evento più vicino.
 */
export function creaMotore(dip: Dipendenze): Motore {
  const stato: StatoMotore = dip.stato ?? { ultimoGiro: null, ultimoErrore: null };
  const bloccoFinoA = new Map<string, number>();
  const ultimiLog = new Map<string, number>();
  let inCorso = false;

  /** Un problema: si ricorda come ultimo errore e, una volta ogni tanto per `codice`, si scrive nel log (`riga`, se diversa dal testo). Solo frasi fisse e numeri, mai dati di un evento. */
  const segnala = (codice: string, testo: string, riga = testo) => {
    const t = dip.adesso();
    stato.ultimoErrore = { testo, alle: t };
    if (t - (ultimiLog.get(codice) ?? -Infinity) < LOG_OGNI_MS) return;
    ultimiLog.set(codice, t);
    console.warn(`promemoria: ${riga}`);
  };

  async function giroInterno(r: Riepilogo): Promise<void> {
    const inizioGiro = dip.adesso();
    stato.ultimoGiro = inizioGiro;
    // nessun canale pronto (di norma: nessun browser iscritto) non è un problema: il giro non fa altro
    const canali = dip.canali().filter((c) => c.configurato());
    if (!canali.length) return;
    const imp = dip.impostazioni();
    dip.store.pulisci(inizioGiro - CONSERVA_GIORNI * 86_400_000);

    const fine = inizioGiro + (Math.max(...imp.minuti) + MARGINE_MIN) * 60_000;
    let eventi: Ev[];
    try {
      const lettura = await dip.leggiEventi(iso(daIstante(inizioGiro)), iso(daIstante(fine)));
      eventi = lettura.eventi;
      if (lettura.problemi) segnala("sorgenti", "la lettura degli eventi ha avuto problemi", `la lettura degli eventi ha avuto problemi (${lettura.problemi} sorgenti)`);
    } catch (e) {
      return void segnala("lettura", "lettura degli eventi non riuscita", `lettura degli eventi non riuscita (${e instanceof Error ? e.name : "errore"})`);
    }
    const vicini = candidati(eventi, inizioGiro);
    r.letti = vicini.length;

    for (const canale of canali) {
      for (const c of vicini) {
        if ((bloccoFinoA.get(canale.nome) ?? 0) > dip.adesso()) break;
        const t = dip.adesso();
        const restanti = c.istante - t;
        if (restanti <= 0) continue;
        const aperti = imp.minuti
          .filter((n) => restanti <= sogliaMs(n))
          .map((n) => {
            const chiave = chiaveDi(c.id, c.inizio, n, canale.nome);
            return { n, chiave, riga: dip.store.leggi(chiave) };
          })
          .filter((x) => !x.riga || x.riga.esito === "riprovo");
        if (!aperti.length) continue;

        let esito: Esito;
        try {
          esito = await canale.invia(testoPromemoria(c.ev, c.naive, restanti, imp.contenuto, t));
        } catch {
          // un canale che solleva è un invio fallito come un altro: si conta il tentativo, e il giro non si ferma a ogni giro per sempre
          esito = { ok: false, errore: `invio sul canale ${canale.nome} non riuscito` };
        }
        const dopo = dip.adesso();
        const riga = (x: (typeof aperti)[number], come: Riga["esito"], errore: string | null): Riga => ({
          chiave: x.chiave,
          evento: c.id,
          inizio: c.inizio,
          minuti: x.n,
          canale: canale.nome,
          inviatoIl: dopo,
          esito: come,
          tentativi: (x.riga?.tentativi ?? 0) + 1,
          errore,
        });
        if (esito.ok) {
          dip.store.scrivi(aperti.map((x) => riga(x, "inviato", null)));
          r.inviati++;
          continue;
        }
        const righe = aperti.map((x) => riga(x, (x.riga?.tentativi ?? 0) + 1 >= TENTATIVI_MAX ? "fallito" : "riprovo", esito.errore));
        const chiusi = righe.filter((x) => x.esito === "fallito").length;
        dip.store.scrivi(righe);
        r.falliti++;
        r.abbandonati += chiusi;
        if (esito.attesaS) bloccoFinoA.set(canale.nome, dopo + Math.min(Math.max(1, Math.ceil(esito.attesaS)), ATTESA_MAX_S) * 1000);
        // l'abbandono ha un codice suo: non deve restare nascosto dal limite sulla riga dei tentativi falliti
        if (chiusi) segnala(`abbandono:${canale.nome}`, esito.errore, `${chiusi} promemoria abbandonati dopo ${TENTATIVI_MAX} tentativi sul canale ${canale.nome}: ${esito.errore}`);
        else segnala(`invio:${canale.nome}`, esito.errore);
        break;
      }
    }
    if (r.inviati) console.info(`promemoria: inviati ${r.inviati}`);
  }

  return {
    stato,
    async giro() {
      const r: Riepilogo = { letti: 0, inviati: 0, falliti: 0, abbandonati: 0 };
      if (inCorso) return r;
      inCorso = true;
      try {
        await giroInterno(r);
      } catch (e) {
        // il testo dell'errore può nominare un percorso, un indirizzo o un titolo: nel log va una frase fissa e il tipo
        segnala("giro", "giro non riuscito", `giro non riuscito (${e instanceof Error ? e.name : "errore"})`);
      } finally {
        inCorso = false;
      }
      return r;
    },
  };
}

// ---- Il giro ogni tanto, dentro il gateway -------------------------------------------------------------------------------
type Programma = { motore: Motore; timer: ReturnType<typeof setTimeout> | null; fermo: boolean; alSigterm: () => void };
const globale = globalThis as typeof globalThis & { __oePromemoria?: Programma; __oePromemoriaStato?: StatoMotore };

/** Quando e come è andato l'ultimo giro: in globalThis, perché le rotte di Next hanno ciascuna la sua copia dei moduli e lo stato è uno solo. */
export const statoGlobale = (): StatoMotore => (globale.__oePromemoriaStato ??= { ultimoGiro: null, ultimoErrore: null });

const intervalloMs = () => {
  const s = Number(process.env.PROMEMORIA_INTERVALLO_S);
  return (Number.isInteger(s) && s >= 5 && s <= 3600 ? s : INTERVALLO_PREDEFINITO_S) * 1000;
};

/** Il motore sui dati veri: la stessa lettura di get_events per le app (tutti i calendari, ma sempre fresca: un promemoria non parte da dati vecchi), lo stato di gateway.db, i canali del server. */
export function dipendenzeReali(stato: StatoMotore = statoGlobale()): Dipendenze {
  return {
    adesso,
    leggiEventi: async (inizio, fine) => {
      const [eventi, errori] = await raccogli(tuttiAccesi(loadConfig()), inizio, fine, false);
      return { eventi, problemi: errori.length };
    },
    impostazioni: () => leggiImpostazioni(loadConfig()),
    canali: canaliDelServer,
    store: creaStore(),
    stato,
  };
}

/** Fa partire il giro ogni PROMEMORIA_INTERVALLO_S secondi (30). Una volta sola per processo: richiamarla non aggiunge un secondo timer. */
export function avviaPromemoria(): void {
  if (globale.__oePromemoria) return;
  const attesa = intervalloMs();
  const motore = creaMotore(dipendenzeReali());
  const alSigterm = () => {
    fermaPromemoria();
    // un gestore del segnale toglie l'uscita di norma: se nessun altro ne ha uno, il processo si ferma da sé
    if (process.listenerCount("SIGTERM") === 0) process.kill(process.pid, "SIGTERM");
  };
  const programma: Programma = { motore, timer: null, fermo: false, alSigterm };
  globale.__oePromemoria = programma;
  // il giro dopo parte a giro finito: due giri non si sovrappongono mai, nemmeno con un agente lento
  const pianifica = (ms: number) => {
    if (programma.fermo) return;
    programma.timer = setTimeout(() => {
      programma.timer = null;
      void motore.giro().finally(() => pianifica(attesa));
    }, ms);
    programma.timer.unref();
  };
  pianifica(PRIMO_GIRO_MS);
  process.once("SIGTERM", alSigterm);
  console.info(`promemoria: pronto, un giro ogni ${attesa / 1000} s`);
}

/** Ferma il giro (SIGTERM, prove). */
export function fermaPromemoria(): void {
  const p = globale.__oePromemoria;
  if (!p) return;
  p.fermo = true;
  if (p.timer) clearTimeout(p.timer);
  process.off("SIGTERM", p.alSigterm);
  globale.__oePromemoria = undefined;
}

// ---- I metodi dell'API ---------------------------------------------------------------------------------------------------
/** Le impostazioni, quali canali il server ha configurato e l'ultimo problema: mai dati di un evento. */
export function leggiPromemoria() {
  const s = statoGlobale();
  return {
    promemoria: leggiImpostazioni(loadConfig()),
    canali: canaliDelServer().map((c) => ({ nome: c.nome, configurato: c.configurato(), ...c.dettagli?.() })),
    ultimo_errore: s.ultimoErrore?.testo ?? null,
    ultimo_errore_il: s.ultimoErrore ? daSecondi(s.ultimoErrore.alle / 1000) : null,
  };
}

export function salvaPromemoria(dati: unknown): { ok: true; promemoria: Impostazioni } | Rifiuto {
  const r = validaImpostazioni(dati, leggiImpostazioni(loadConfig()));
  if (!r.ok) return r;
  modificaConfig((cfg) => void (cfg.promemoria = r.valore));
  return { ok: true, promemoria: r.valore };
}

/** Manda il testo fisso di prova su un canale: mai dati di un evento, e solo su richiesta. */
export async function provaPromemoria(nome: unknown, canali: Canale[] = canaliDelServer()): Promise<{ ok: true } | Rifiuto> {
  const canale = canali.find((c) => c.nome === nome);
  if (!canale) return { ok: false, error: "Canale sconosciuto." };
  if (!canale.configurato()) return { ok: false, error: "Canale non configurato sul server." };
  const esito = await canale.invia(TESTO_PROVA);
  return esito.ok ? { ok: true } : { ok: false, error: esito.errore };
}
