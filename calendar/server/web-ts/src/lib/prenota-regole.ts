// Le regole degli orari che si possono prenotare dalla pagina pubblica, per la pagina (che grigia e non lascia scegliere il resto) e per
// il server (lib/prenota.ts, che le controlla di nuovo): dal lunedì al venerdì, dalle 9 alle 18 di Roma, a quarti d'ora, almeno un quarto
// d'ora, con due ore di preavviso e fino a 60 giorni avanti.

export const PASSO_MIN = 15;
export const APRE_MIN = 9 * 60;
export const CHIUDE_MIN = 18 * 60;
export const PREAVVISO_MIN = 120;
export const GIORNI_AVANTI = 60;

/** Un orario come lo si legge a Roma: giorno, minuti dalla mezzanotte, giorno della settimana (0 domenica) e secondi. */
export type OraRoma = { giorno: string; minuti: number; dow: number; secondi: number; ms: number };

const ROMA = new Intl.DateTimeFormat("en-US", { timeZone: "Europe/Rome", hourCycle: "h23", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", weekday: "short" });
const DOW: Record<string, number> = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };

/** Un istante del dispositivo come ora di Roma. */
export function aRoma(d: Date): OraRoma {
  const p: Record<string, string> = {};
  for (const x of ROMA.formatToParts(d)) p[x.type] = x.value;
  return { giorno: `${p.year}-${p.month}-${p.day}`, minuti: Number(p.hour) * 60 + Number(p.minute), dow: DOW[p.weekday], secondi: Number(p.second), ms: d.getTime() };
}

/** Perché quell'orario non si prenota (frase per chi prenota), o null se va bene. Non sa dei calendari: quello lo dice il server. */
export function fuoriRegola(inizio: OraRoma, fine: OraRoma, oraMs: number): string | null {
  if (inizio.giorno !== fine.giorno) return "Il meeting deve iniziare e finire nello stesso giorno.";
  if (inizio.secondi || fine.secondi || inizio.minuti % PASSO_MIN || fine.minuti % PASSO_MIN) return "Gli orari vanno a quarti d'ora (:00, :15, :30, :45).";
  if (fine.minuti - inizio.minuti < PASSO_MIN) return "Il meeting dura almeno 15 minuti.";
  if (inizio.dow === 0 || inizio.dow === 6) return "Si prenota dal lunedì al venerdì.";
  if (inizio.minuti < APRE_MIN || fine.minuti > CHIUDE_MIN) return "Si prenota dalle 9:00 alle 18:00.";
  if (inizio.ms < oraMs + PREAVVISO_MIN * 60_000) return "Serve almeno due ore di preavviso.";
  if (inizio.ms > oraMs + GIORNI_AVANTI * 86_400_000) return `Si prenota al più ${GIORNI_AVANTI} giorni avanti.`;
  return null;
}
