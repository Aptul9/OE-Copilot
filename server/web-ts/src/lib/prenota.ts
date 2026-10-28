import { occupato } from "./core/eventi";
import { istante, leggiNaive, type Naive } from "./core/fuso";
import { aggiungiEvento } from "./core/locali";
import { adesso } from "./core/orologio";
import { type Canale, canaliDelServer } from "./core/promemoria-canali";
import { loadConfig } from "./core/stato";
import { dimenticaDisponibilita, GRUPPO_MAX } from "./disponibilita";
import { fuoriRegola as regolaRoma, type OraRoma } from "./prenota-regole";

// Prenotazione di un meeting dalla pagina pubblica (meet, /disponibilita): chi la apre sceglie uno spazio libero sulla griglia e
// lascia nome, email, azienda e argomento. L'evento entra subito nel calendario come evento personale che blocca il tempo,
// per il gruppo della pagina (?gruppo=) o per tutti i gruppi; a noi arriva una notifica push. Le regole stanno qui e non solo
// nella pagina: dal lunedì al venerdì, dalle 9 alle 18 (ora di Roma), a passi di un quarto d'ora, con due ore di preavviso e
// fino a 60 giorni avanti (lib/prenota-regole.ts, le stesse della pagina), solo in uno spazio che i calendari danno libero.

/** Prenotazioni accettate in un'ora, in tutto: dietro sslh chi prenota non si distingue, il limite è uno solo. */
export const PRENOTAZIONI_ORA = 6;

const LIMITI = { nome: 100, email: 254, azienda: 100, argomento: 1000 } as const;
const NAIVE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:00)?$/;
const EMAIL = /^[^\s@<>()",;:]+@[^\s@<>()",;:]+\.[^\s@<>()",;:]{2,}$/;
// eslint-disable-next-line no-control-regex
const CONTROLLO = /[\u0000-\u0008\u000b-\u001f\u007f]/g;

export type Richiesta = { inizio: Naive; fine: Naive; nome: string; email: string; azienda: string; argomento: string; gruppo: string };
export type Esito = { ok: true; inizio: string; fine: string } | { ok: false; stato: 400 | 404 | 409 | 429 | 502; errore: string };

const minuti = (n: Naive) => n.h * 60 + n.mi;
const due = (n: number) => String(n).padStart(2, "0");
const giorno = (n: Naive) => `${n.a}-${due(n.m)}-${due(n.g)}`;
const ora = (n: Naive) => `${due(n.h)}:${due(n.mi)}`;
const testo = (n: Naive) => `${giorno(n)}T${ora(n)}:00`;
const pulito = (v: unknown, max: number, righe = false) =>
  typeof v === "string" ? (righe ? v.replace(CONTROLLO, " ") : v.replace(/[\r\n\t]+/g, " ").replace(CONTROLLO, " ")).trim().slice(0, max + 1) : "";

/** Il corpo della richiesta, controllato campo per campo; la data e gli orari sono ore di Roma senza scarto (come Nuovo evento). */
export function leggiRichiesta(corpo: unknown): Richiesta | { errore: string } {
  if (!corpo || typeof corpo !== "object" || Array.isArray(corpo)) return { errore: "Richiesta non valida." };
  const c = corpo as Record<string, unknown>;
  // il campo nascosto: lo riempie solo un programma che compila tutto quello che trova
  if (typeof c.sito === "string" && c.sito.trim()) return { errore: "Richiesta non valida." };
  if (typeof c.inizio !== "string" || typeof c.fine !== "string" || !NAIVE.test(c.inizio) || !NAIVE.test(c.fine)) return { errore: "Data o orario non validi." };
  let inizio: Naive, fine: Naive;
  try {
    inizio = leggiNaive(c.inizio);
    fine = leggiNaive(c.fine);
  } catch {
    return { errore: "Data o orario non validi." };
  }
  const nome = pulito(c.nome, LIMITI.nome);
  const email = pulito(c.email, LIMITI.email);
  const azienda = pulito(c.azienda, LIMITI.azienda);
  const argomento = pulito(c.argomento, LIMITI.argomento, true);
  const gruppo = pulito(c.gruppo, GRUPPO_MAX);
  if (nome.length < 2) return { errore: "Scrivi il tuo nome." };
  if (nome.length > LIMITI.nome) return { errore: `Il nome è troppo lungo (al più ${LIMITI.nome} caratteri).` };
  if (email.length > LIMITI.email || !EMAIL.test(email)) return { errore: "L'email non sembra giusta." };
  if (azienda.length > LIMITI.azienda) return { errore: `Il nome dell'azienda è troppo lungo (al più ${LIMITI.azienda} caratteri).` };
  if (argomento.length > LIMITI.argomento) return { errore: `L'argomento è troppo lungo (al più ${LIMITI.argomento} caratteri).` };
  if (gruppo.length > GRUPPO_MAX) return { errore: "Gruppo non valido." };
  return { inizio, fine, nome, email, azienda, argomento, gruppo };
}

/** Le regole sull'orario (lib/prenota-regole.ts), a prescindere dai calendari; null se va bene. */
export function fuoriRegola(r: Pick<Richiesta, "inizio" | "fine">, oraMs: number): string | null {
  const roma = (n: Naive): OraRoma => ({ giorno: giorno(n), minuti: minuti(n), dow: new Date(Date.UTC(n.a, n.m - 1, n.g)).getUTCDay(), secondi: n.s, ms: istante(n) });
  return regolaRoma(roma(r.inizio), roma(r.fine), oraMs);
}

type Deps = { canali?: Canale[]; oraMs?: () => number };

const accettate: number[] = [];
// una prenotazione alla volta: due richieste per lo stesso spazio non passano entrambe il controllo
let coda: Promise<unknown> = Promise.resolve();

/** Svuota il conteggio delle prenotazioni (per le prove). */
export function azzera() {
  accettate.length = 0;
  coda = Promise.resolve();
}

export function prenota(corpo: unknown, deps: Deps = {}): Promise<Esito> {
  const giro = coda.then(() => prenotaOra(corpo, deps));
  coda = giro.catch(() => undefined);
  return giro;
}

async function prenotaOra(corpo: unknown, deps: Deps): Promise<Esito> {
  const r = leggiRichiesta(corpo);
  if ("errore" in r) return { ok: false, stato: 400, errore: r.errore };
  const oraMs = deps.oraMs?.() ?? adesso();
  const regola = fuoriRegola(r, oraMs);
  if (regola) return { ok: false, stato: 400, errore: regola };
  while (accettate.length && accettate[0] <= oraMs - 3_600_000) accettate.shift();
  if (accettate.length >= PRENOTAZIONI_ORA) return { ok: false, stato: 429, errore: "Troppe prenotazioni in poco tempo: riprova più tardi." };

  const cfg = loadConfig();
  let impegni: string[][], feste: string[][];
  try {
    [impegni, feste] = await occupato(cfg, { a: r.inizio.a, m: r.inizio.m, g: r.inizio.g }, 1, r.gruppo);
  } catch (e) {
    if (e instanceof Error && e.message === "Gruppo non trovato") return { ok: false, stato: 404, errore: "Non c'è nessun calendario con questo nome." };
    console.error("prenota: calendari non leggibili", e);
    return { ok: false, stato: 502, errore: "Non riesco a leggere i calendari in questo momento: riprova fra poco." };
  }
  if (feste.length) return { ok: false, stato: 409, errore: "Quel giorno è festivo." };
  const [da, a] = [testo(r.inizio), testo(r.fine)];
  if (impegni.some(([i, f]) => i < a && f > da)) return { ok: false, stato: 409, errore: "Questo orario non è più libero: scegline un altro." };

  // per il gruppo della pagina; dalla pagina di tutti, per ogni gruppo (si vede e blocca il tempo in ciascuno)
  const g = cfg.gruppi_calendari;
  const gruppi = r.gruppo ? [r.gruppo] : g && typeof g === "object" && !Array.isArray(g) ? Object.keys(g) : [];
  aggiungiEvento({
    title: `Meeting con ${r.nome}${r.azienda ? ` (${r.azienda})` : ""}`,
    start: da,
    end: a,
    allDay: false,
    location: "",
    notes: [`Prenotato da meet il ${new Date(oraMs).toISOString().slice(0, 16).replace("T", " ")} UTC`, `Email: ${r.email}`, r.azienda && `Azienda: ${r.azienda}`, r.argomento && `Argomento: ${r.argomento}`].filter(Boolean).join("\n"),
    blocca: true,
    gruppi,
  });
  accettate.push(oraMs);
  dimenticaDisponibilita();

  const quando = `${new Intl.DateTimeFormat("it-IT", { weekday: "short", day: "numeric", month: "short", timeZone: "UTC" }).format(Date.UTC(r.inizio.a, r.inizio.m - 1, r.inizio.g))} ${ora(r.inizio)}-${ora(r.fine)}`;
  const messaggio = [`Meeting prenotato: ${quando}`, `${r.nome}${r.azienda ? ` (${r.azienda})` : ""} · ${r.email}`, r.argomento].filter(Boolean).join("\n");
  for (const c of deps.canali ?? canaliDelServer()) {
    if (!c.configurato()) continue;
    // la prenotazione c'è comunque: una notifica che non parte si scrive nel log
    void c.invia(messaggio).then((e) => e.ok || console.error(`prenota: notifica ${c.nome} non inviata: ${e.errore}`), (e) => console.error(`prenota: notifica ${c.nome}`, e));
  }
  return { ok: true, inizio: da, fine: a };
}
