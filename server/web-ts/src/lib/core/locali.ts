import { aggiungiLocale, leggiRigheLocali, togliLocali } from "./archivio";
import type { Ev } from "./caselle";
import { chiave, ErroreData, leggiIso, type Naive } from "./fuso";
import { ACCOUNT_LOCALE } from "./personalizzazione";
import { ErroreValore, isDict, pyBool, testoPulito, tipoPy } from "./py";
import { colorForAccount, type Config } from "./stato";

// Gli eventi aggiunti a mano dentro l'app, calendario "Altro" (era local_events.py, fino al commit 142e789): l'unico calendario su cui l'app scrive. Stanno nel
// database dello stato (archivio.ts), così non toccano nessuna casella di posta e non richiedono permessi.

export const ID_CALENDARIO = "local::altro";

/** Gli eventi personali, nell'ordine in cui sono stati aggiunti. */
export const leggiLocali = (): Ev[] => leggiRigheLocali();

/** Il prossimo identificativo: il più alto fra quelli fatti di sole cifre, più uno (si parte da 1). */
function nuovoId(esistenti: Ev[]): string {
  const numeri = esistenti.map((e) => String(e?.uid ?? "")).filter((u) => /^[0-9]+$/.test(u)).map(Number);
  return String(numeri.length ? Math.max(...numeri) + 1 : 1);
}

/**
 * Aggiunge un evento. dati: title, start, end, allDay, location, notes, blocca. Solleva ErroreValore con il testo che la pagina mostra.
 * `blocca` ("Blocca il tempo", impegni.ts) si salva solo se la pagina lo manda: un evento che non ce l'ha resta come prima.
 */
export function aggiungiEvento(dati: unknown): void {
  const d = pyBool(dati) ? dati : {};
  if (!isDict(d)) throw new TypeError(`'${tipoPy(d)}' object has no attribute 'get'`);
  const titolo = testoPulito(d.title);
  if (!titolo) throw new ErroreValore("Il titolo è obbligatorio.");
  const inizio = testoPulito(d.start);
  if (!inizio) throw new ErroreValore("Manca la data/ora di inizio.");
  try {
    leggiIso(inizio);
    if (pyBool(d.end)) {
      if (typeof d.end !== "string") throw new TypeError("fromisoformat: argument must be str");
      leggiIso(d.end);
    }
  } catch (e) {
    if (e instanceof ErroreData) throw new ErroreValore(`Data non valida: ${e.message}`);
    throw e;
  }
  aggiungiLocale((esistenti) => ({
    uid: nuovoId(esistenti),
    title: titolo,
    start: inizio,
    end: testoPulito(d.end) || inizio,
    allDay: pyBool(d.allDay),
    location: testoPulito(d.location),
    notes: testoPulito(d.notes),
    ...(typeof d.blocca === "boolean" ? { blocca: d.blocca } : {}),
  }));
}

/** Toglie l'evento con quell'identificativo; se non c'è non succede niente. */
export const eliminaEvento = (uid: unknown): void => togliLocali(uid);

/** La voce singola dell'elenco calendari: il calendario locale è sempre presente. */
export function elencoLocali(cfg: Config): Ev[] {
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  return [
    {
      id: ID_CALENDARIO,
      account: ACCOUNT_LOCALE,
      alias: ACCOUNT_LOCALE,
      label: ACCOUNT_LOCALE,
      url: null,
      count: leggiLocali().length,
      is_ics: false,
      is_local: true,
      color: colorForAccount(cfg, ACCOUNT_LOCALE),
      enabled: !disabled.has(ID_CALENDARIO),
      // Sta su questo disco: non c'è un collegamento che possa cadere
      stato: "ok",
      stato_msg: "Locale",
    },
  ];
}

/** Gli eventi locali che si sovrappongono all'intervallo (ora locale senza fuso). */
export function eventiLocali(cfg: Config, inizio: Naive, fine: Naive): Ev[] {
  const disabled = new Set(Array.isArray(cfg.disabled_calendars) ? cfg.disabled_calendars : []);
  if (disabled.has(ID_CALENDARIO)) return [];
  const colore = colorForAccount(cfg, ACCOUNT_LOCALE);
  const out: Ev[] = [];
  for (const e of leggiLocali()) {
    let s, f;
    try {
      s = leggiIso(String(e.start));
      f = leggiIso(String(e.end || e.start));
    } catch (x) {
      if (x instanceof ErroreData) continue;
      throw x;
    }
    // Python non confronta un'ora locale con una che ha lo scarto
    if (s.scarto !== null || f.scarto !== null) throw new Error("can't compare offset-naive and offset-aware datetimes");
    if (!(chiave(f.naive) > chiave(inizio) && chiave(s.naive) < chiave(fine))) continue;
    out.push({
      id: `local::${e.uid}`,
      uid: e.uid,
      account: ACCOUNT_LOCALE,
      alias: ACCOUNT_LOCALE,
      calendar: ACCOUNT_LOCALE,
      color: colore,
      title: e.title || "(senza titolo)",
      start: e.start,
      end: e.end || e.start,
      allDay: pyBool(e.allDay),
      location: e.location || "",
      organizer: "",
      online_url: null,
      web_link: null,
      is_local: true,
      ...(typeof e.blocca === "boolean" ? { blocca: e.blocca } : {}),
    });
  }
  return out;
}
