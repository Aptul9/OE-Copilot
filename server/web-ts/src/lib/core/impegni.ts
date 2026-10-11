import type { Ev } from "./caselle";
import { chiave, isoSecondi, mezzanotte, naiveLocale, piuGiorni, type Naive } from "./fuso";
import { pyStr } from "./py";

// Quello che la pagina pubblica (senza accesso) può sapere degli impegni: quando, mai cosa (disponibilita.py). Da qui escono solo
// orari di inizio e fine, e le festività (date pubbliche): né un titolo, né un luogo, né chi partecipa, né il nome di un calendario.

const maggiore = (a: Naive, b: Naive) => (chiave(a) >= chiave(b) ? a : b);

/**
 * [[inizio, fine], ...] (ISO, ora locale, al secondo): quando si è occupati nei `giorni` giorni da `primoGiorno`, in ordine.
 * Con `aziende` contano solo gli eventi di quelle aziende. Se si è occupati, si è occupati: gli impegni che si sovrappongono o si
 * toccano diventano un blocco solo, anche a cavallo della mezzanotte. Un giorno intero non conta, e nemmeno un impegno senza durata.
 */
export function impegni(eventi: Ev[], primoGiorno: { a: number; m: number; g: number }, giorni: number, aziende: Set<string> | null = null): string[][] {
  const da = mezzanotte(primoGiorno.a, primoGiorno.m, primoGiorno.g);
  const a = piuGiorni(da, giorni);
  const trovati: [Naive, Naive][] = [];
  for (const e of eventi) {
    if (e.allDay || !e.start) continue;
    if (aziende !== null && !aziende.has(e.alias || e.account)) continue;
    let inizio: Naive, fine: Naive;
    try {
      inizio = naiveLocale(e.start);
      fine = naiveLocale(e.end || e.start);
    } catch {
      continue;
    }
    if (chiave(fine) > chiave(inizio) && chiave(fine) > chiave(da) && chiave(inizio) < chiave(a)) trovati.push([inizio, fine]);
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
