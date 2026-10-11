import fs from "node:fs";
import type { Ev } from "./caselle";
import { nelleDati } from "./dati";
import { chiave, ErroreData, leggiIso, type Naive } from "./fuso";
import { ACCOUNT_LOCALE } from "./personalizzazione";
import { pyBool } from "./py";
import { colorForAccount, type Config } from "./stato";

// Gli eventi aggiunti a mano dentro l'app, calendario "Altro" (local_events.py): l'unico calendario su cui l'app scrive. Sta in un
// file JSON nella cartella dei dati, così non tocca nessuna casella di posta e non richiede permessi.

export const ID_CALENDARIO = "local::altro";

export const percorsoLocali = () => nelleDati("local_events.json");

/** Gli eventi del file. Qui ci sono dati che esistono solo qui: un file illeggibile non deve passare inosservato. */
export function leggiLocali(): Ev[] {
  try {
    const dati = JSON.parse(fs.readFileSync(percorsoLocali(), "utf-8"));
    return Array.isArray(dati) ? dati : [];
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") console.error(`eventi personali: file non leggibile (${percorsoLocali()})`, e);
    return []; // la prima volta: nessun evento personale, è normale
  }
}

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
    });
  }
  return out;
}
