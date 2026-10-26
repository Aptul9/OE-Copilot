import type { Ev } from "./caselle";
import { chiave, isoSecondi, mezzanotte, naiveLocale, piuGiorni, type Naive } from "./fuso";
import { pyStr } from "./py";

// Quello che la pagina pubblica (senza accesso) può sapere degli impegni: quando, mai cosa (era disponibilita.py, fino al commit 142e789). Da qui escono solo
// orari di inizio e fine, e le festività (date pubbliche): né un titolo, né un luogo, né chi partecipa, né il nome di un calendario.

/** Un giorno in unità di chiave (microsecondi). */
const GIORNO = 86_400_000_000;

/**
 * Un blocco voluto: un evento personale con "Blocca il tempo", o uno delle caselle segnato "Occupato" in Outlook o Google (occupato,
 * dall'agente). Un evento personale con "Blocca il tempo" spento non rende mai occupati; uno senza la scelta (fatto prima che ci fosse) conta
 * come uno delle caselle libero.
 */
const blocca = (e: Ev): boolean => (e.is_local ? e.blocca === true : e.occupato === true);

/**
 * Un giorno intero, con la fine esclusiva come in iCalendar: da mezzanotte del primo giorno a mezzanotte dopo l'ultimo. Una fine con l'ora (un
 * evento personale salvato dai campi data e ora) o non dopo l'inizio chiude il suo giorno.
 */
function giorniInteri(start: string, end: string): [Naive, Naive] {
  const s = naiveLocale(start);
  const f = naiveLocale(end);
  const inizio = mezzanotte(s.a, s.m, s.g);
  const fine = mezzanotte(f.a, f.m, f.g);
  return [inizio, chiave(fine) !== chiave(f) || chiave(fine) <= chiave(inizio) ? piuGiorni(fine, 1) : fine];
}

const maggiore = (a: Naive, b: Naive) => (chiave(a) >= chiave(b) ? a : b);

/**
 * [[inizio, fine], ...] (ISO, ora locale, al secondo): quando si è occupati nei `giorni` giorni da `primoGiorno`, in ordine.
 * Con `aziende` contano solo gli eventi di quelle aziende. Se si è occupati, si è occupati: gli impegni che si sovrappongono o si
 * toccano diventano un blocco solo, anche a cavallo della mezzanotte. Non conta un impegno senza durata. Un giorno intero o un impegno da
 * GIORNO in su conta solo se è un blocco (blocca): altrimenti è una finestra (un change di ServiceNow di sei settimane) o un promemoria, non
 * un'ora in cui non si risponde, e renderebbe occupate settimane intere. Un giorno intero che blocca occupa da mezzanotte a mezzanotte.
 */
export function impegni(eventi: Ev[], primoGiorno: { a: number; m: number; g: number }, giorni: number, aziende: Set<string> | null = null): string[][] {
  const da = mezzanotte(primoGiorno.a, primoGiorno.m, primoGiorno.g);
  const a = piuGiorni(da, giorni);
  const trovati: [Naive, Naive][] = [];
  for (const e of eventi) {
    if (!e.start) continue;
    if (aziende !== null && !aziende.has(e.alias || e.account)) continue;
    const blocco = blocca(e);
    if (e.is_local && e.blocca === false) continue;
    let inizio: Naive, fine: Naive;
    try {
      [inizio, fine] = e.allDay ? giorniInteri(e.start, e.end || e.start) : [naiveLocale(e.start), naiveLocale(e.end || e.start)];
    } catch {
      continue;
    }
    const durata = chiave(fine) - chiave(inizio);
    if ((e.allDay || durata >= GIORNO) && !blocco) continue;
    if (durata > 0 && chiave(fine) > chiave(da) && chiave(inizio) < chiave(a)) trovati.push([inizio, fine]);
  }
  trovati.sort((x, y) => chiave(x[0]) - chiave(y[0]) || chiave(x[1]) - chiave(y[1]));
  const uniti: [Naive, Naive][] = [];
  for (const [inizio, fine] of trovati) {
    const ultimo = uniti[uniti.length - 1];
    if (ultimo && chiave(inizio) <= chiave(ultimo[1])) ultimo[1] = maggiore(ultimo[1], fine);
    else uniti.push([inizio, fine]);
  }
  return uniti.map(([i, f]) => [isoSecondi(i), isoSecondi(f)]);
}

/** [[inizio, fine, nome], ...]: le festività, cioè gli eventi di un giorno intero del calendario `calendario`. */
export function festivita(eventi: Ev[], calendario: string): string[][] {
  const trovate = new Map<string, string[]>();
  for (const e of eventi) {
    if (e.allDay && e.account === calendario && e.start) {
      const voce = [pyStr(e.start), pyStr(e.end || e.start), pyStr(e.title || "")];
      trovate.set(JSON.stringify(voce), voce);
    }
  }
  return [...trovate.values()].sort((x, y) => {
    for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
    return 0;
  });
}
