import { ErroreNonSupportato, ErroreTipo, ErroreValore, espandiIcs, espandiRigheAndroid, validaIcs } from "../../src/index.ts";
import { type Db, type Fonte, type RigaDb, eventiAgente } from "./android.ts";

export const FONTE_PROVA: Fonte = { id: "android::mario@alfa.example", nome: "Alfa", account: "mario@alfa.example" };
export const dbDiUnCaso = (righe: RigaDb[]): Db => ({ calendari: [[1, "mario@alfa.example", "com.microsoft.office.outlook", "Calendar", 1, 0, 1]], eventi: righe, invitati: [] });

export type Esito = { stato: "uguale" | "diverso" | "non-supportato"; dettaglio: string };

const ordina = <T extends Record<string, any>>(v: T[], chiavi: string[]) =>
  [...v].sort((a, b) => {
    for (const k of chiavi) if (a[k] !== b[k]) return String(a[k]) < String(b[k]) ? -1 : 1;
    return 0;
  });
/** JSON con le chiavi in ordine, per confrontare due oggetti che le hanno scritte in ordine diverso. */
export const canonico = (v: unknown): string => JSON.stringify(v, (_k, x) => (x && typeof x === "object" && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : x));
const sintesi = (v: unknown) => (canonico(v) ?? "niente").slice(0, 260);

function tentativo<T>(f: () => T): { valore: T } | { errore: unknown } {
  try {
    return { valore: f() };
  } catch (errore) {
    return { errore };
  }
}

/** Confronto di una risposta attesa (o un errore) con quello che si ottiene: `uguale` se coincidono. */
function confrontaLista(atteso: unknown, ottenuto: { valore: unknown[] } | { errore: unknown }, chiavi: string[], soloErrore = false): Esito {
  const errAtteso = !Array.isArray(atteso);
  if ("errore" in ottenuto) {
    if (ottenuto.errore instanceof ErroreNonSupportato && !errAtteso) return { stato: "non-supportato", dettaglio: ottenuto.errore.message };
    if (!errAtteso) return { stato: "diverso", dettaglio: `errore ${String(ottenuto.errore)} dove Python risponde ${sintesi(atteso)}` };
    // il testo dell'errore è quello che il gateway mostra: deve essere lo stesso di Python (registrato fino a 200 caratteri)
    const messaggio = (atteso as { messaggio?: string }).messaggio;
    const nostro = ottenuto.errore instanceof Error ? ottenuto.errore.message.slice(0, 200) : String(ottenuto.errore);
    return messaggio === undefined || messaggio === nostro ? { stato: "uguale", dettaglio: "" } : { stato: "diverso", dettaglio: `errore con un altro testo: Python ${JSON.stringify(messaggio)}, qui ${JSON.stringify(nostro)}` };
  }
  if (errAtteso) return { stato: "diverso", dettaglio: `Python dà errore ${sintesi(atteso)}, qui ${ottenuto.valore.length} occorrenze` };
  if (soloErrore) return { stato: "uguale", dettaglio: "" };
  const a = canonico(ordina(atteso as any[], chiavi));
  const b = canonico(ordina(ottenuto.valore as any[], chiavi));
  if (a === b) return { stato: "uguale", dettaglio: "" };
  const x = ordina(atteso as any[], chiavi);
  const y = ordina(ottenuto.valore as any[], chiavi);
  const trovato = x.findIndex((e, k) => canonico(e) !== canonico(y[k]));
  const i = trovato < 0 ? Math.min(x.length, y.length) : trovato;
  return { stato: "diverso", dettaglio: `Python ${x.length}, qui ${y.length}; primo diverso: atteso ${sintesi(x[i])} ottenuto ${sintesi(y[i])}` };
}

const CHIAVI_GREZZI = ["uid", "inizio", "fine", "titolo", "luogo"];
const CHIAVI_ICS = ["inizio", "uid", "fine", "titolo", "luogo"];

export function esitoAndroid(caso: any, finestra: string): Esito[] {
  const [inizio, fine] = finestra.split("|") as [string, string];
  const atteso = caso.atteso[finestra];
  const db = dbDiUnCaso(caso.righe);
  const grezzi = tentativo(() => espandiRigheAndroid(db.eventi, inizio, fine));
  const grezziRegistrati = "valore" in grezzi ? { valore: grezzi.valore.map(({ descrizione: _d, ...resto }) => resto) } : grezzi;
  const e1 = confrontaLista(atteso.grezzi, grezziRegistrati, CHIAVI_GREZZI);
  const e1Detto = { ...e1, dettaglio: e1.dettaglio && `grezzi: ${e1.dettaglio}` };
  if (atteso.finali === undefined) return [e1Detto];
  // dei doppioni tolti si confrontano i campi che contano (nel golden dell'agente, l'evento intero)
  const finali = tentativo(() => (eventiAgente(db, FONTE_PROVA, inizio, fine, "valore" in grezzi ? grezzi.valore : undefined) as any[]).map(({ id, start, end, allDay, title, location }) => ({ id, start, end, allDay, title, location })));
  const e2 = confrontaLista(atteso.finali, finali, ["start", "title", "id", "end"]);
  return [e1Detto, { ...e2, dettaglio: e2.dettaglio && `finali: ${e2.dettaglio}` }];
}

export function esitoIcs(caso: any, finestra: string): Esito[] {
  const [inizio, fine] = finestra.split("|") as [string, string];
  const atteso = caso.atteso.eventi[finestra];
  const ottenuto = tentativo(() => espandiIcs(caso.testo, inizio, fine));
  const e1 = confrontaLista(atteso.ok ? atteso.eventi : { errore: atteso.errore, messaggio: atteso.messaggio }, ottenuto, CHIAVI_ICS);
  const out = [e1];
  if (!atteso.ok && "errore" in ottenuto && e1.stato === "uguale") {
    const valore = ottenuto.errore instanceof ErroreValore || ottenuto.errore instanceof ErroreTipo;
    if (valore !== atteso.valore) out.push({ stato: "diverso", dettaglio: `tipo di errore: Python valore=${atteso.valore}, qui ${valore}` });
  }
  return out;
}

export function esitoValida(caso: any): Esito {
  const atteso = caso.atteso.valida;
  const ottenuto = tentativo(() => validaIcs(caso.testo));
  if ("errore" in ottenuto) {
    const valore = ottenuto.errore instanceof ErroreValore || ottenuto.errore instanceof ErroreTipo;
    const messaggio = ottenuto.errore instanceof Error ? ottenuto.errore.message.slice(0, 200) : "";
    return !atteso.ok && valore === atteso.valore && messaggio === atteso.messaggio ? { stato: "uguale", dettaglio: "" } : { stato: "diverso", dettaglio: `valida: errore ${String(ottenuto.errore)} contro ${sintesi(atteso)}` };
  }
  return atteso.ok && atteso.nome === ottenuto.valore ? { stato: "uguale", dettaglio: "" } : { stato: "diverso", dettaglio: `valida: ${sintesi(ottenuto.valore)} contro ${sintesi(atteso)}` };
}

export function esitoCaso(caso: any): Esito {
  const esiti: Esito[] = [];
  if (caso.tipo === "android") for (const f of caso.finestre) esiti.push(...esitoAndroid(caso, f.join("|")));
  else {
    esiti.push(esitoValida(caso));
    for (const f of caso.finestre) esiti.push(...esitoIcs(caso, f.join("|")));
  }
  const diversi = esiti.filter((e) => e.stato === "diverso");
  if (diversi.length) return { stato: "diverso", dettaglio: diversi.map((e) => e.dettaglio).slice(0, 2).join(" || ") };
  const nonSupp = esiti.find((e) => e.stato === "non-supportato");
  return nonSupp ? { stato: "non-supportato", dettaglio: nonSupp.dettaglio } : { stato: "uguale", dettaglio: "" };
}
