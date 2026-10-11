// Quello che CalendarioAndroid fa intorno all'espansione (scelta dei calendari, dettagli, doppioni): serve alle prove per
// confrontare le risposte complete dell'agente. Non fa parte del pacchetto: il pacchetto espande, qui si compone il risultato.

import { MEETING_URL, espandiRigheAndroid, type OccorrenzaAndroid, type RigaAndroid } from "../../src/index";

export type RigaDb = RigaAndroid & { calendar_id: number; organizer: string | null; deleted: number };
export interface Db {
  calendari: [number, string, string, string, number, number, number][];
  eventi: RigaDb[];
  invitati: [number, string | null, string | null, number][];
}
export interface Fonte {
  id: string;
  nome: string;
  account: string;
  calendari?: string[];
}

const TIPI_ACCOUNT = ["com.microsoft.office.outlook", "com.google"];

export function estrai(db: Db, fonte: Fonte) {
  const account = fonte.account.trim().toLowerCase();
  const scelti = (fonte.calendari ?? []).map((c) => c.trim().toLowerCase());
  const cal = new Map<number, string>();
  const sincronizzati = new Map<number, string>();
  let tipo: string | null = null;
  for (const [id, conto, tp, nome, primario, cancellato, sincr] of db.calendari) {
    if ((conto ?? "").toLowerCase() !== account || !TIPI_ACCOUNT.some((t) => (tp ?? "").startsWith(t))) continue;
    if (cancellato) continue;
    tipo = tp.startsWith("com.google") ? "caldav" : "outlook";
    if (sincr) sincronizzati.set(id, nome || "Calendario");
    const scelto = scelti.length ? scelti.includes((nome ?? "").toLowerCase()) : Boolean(primario);
    if (scelto) cal.set(id, nome || "Calendario");
  }
  const calendari = !cal.size && !scelti.length ? sincronizzati : cal;
  if (!calendari.size) return null;
  const eventi = db.eventi.filter((r) => !r.deleted && calendari.has(r.calendar_id));
  const ids = new Set(db.eventi.filter((r) => calendari.has(r.calendar_id)).map((r) => r._id));
  const persone = new Map<number, { invitati: string[]; organizzatore: string | null }>();
  for (const [evento, nome, email, relazione] of db.invitati) {
    if (!ids.has(evento)) continue;
    const p = persone.get(evento) ?? { invitati: [], organizzatore: null };
    persone.set(evento, p);
    if (email) p.invitati.push(email.toLowerCase());
    if (relazione === 2) p.organizzatore = nome || email;
  }
  return { cal: calendari, eventi, persone, tipo };
}

const sblocca = (t: string) => t.replace(/&(amp|lt|gt|quot|#39);/g, (_, e: string) => ({ amp: "&", lt: "<", gt: ">", quot: '"', "#39": "'" })[e]!);
const pulisciLink = (url: string) => sblocca(url).split(/%3[Cc](?=https?:\/\/)/)[0]!.replace(/[>).,;"']+$/, "");

/** Il confronto fra tuple di Python: la prima posizione che differisce decide. */
function pesoMaggiore(a: boolean[], b: boolean[]): boolean {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return Number(a[i]) > Number(b[i]);
  return false;
}

export function occorrenzeGrezze(db: Db, fonte: Fonte, inizio: string, fine: string): OccorrenzaAndroid[] | null {
  const lettura = estrai(db, fonte);
  return lettura ? espandiRigheAndroid(lettura.eventi, inizio, fine) : null;
}

/** Gli eventi come li restituisce CalendarioAndroid.eventi: un'occorrenza per titolo, inizio e fine (la più completa), in ordine. */
export function eventiAgente(db: Db, fonte: Fonte, inizio: string, fine: string, grezzi?: OccorrenzaAndroid[]) {
  const lettura = estrai(db, fonte);
  if (!lettura) return null;
  const { cal, eventi, persone, tipo } = lettura;
  const occorrenze = grezzi ?? espandiRigheAndroid(eventi, inizio, fine);
  const dettagli = new Map<string, { calendario: string; email: string; organizzatore: string; invitati: string[]; serie?: boolean }>();
  const perSync = new Map<string, number>();
  for (const r of eventi) if (r._sync_id) perSync.set(r._sync_id, r._id);
  for (const r of eventi) {
    const serie = r.original_id || (r.original_sync_id ? perSync.get(r.original_sync_id) : undefined);
    if (serie && r.eventStatus === 2 && r.originalInstanceTime !== null) continue;
    if (r.eventStatus === 2 || r.dtstart === null) continue;
    const uid = `a${serie || r._id}`;
    const p = persone.get(r._id);
    if (!dettagli.has(uid))
      dettagli.set(uid, {
        calendario: cal.get(r.calendar_id) ?? "",
        email: (r.organizer ?? "").toLowerCase(),
        organizzatore: p?.organizzatore || r.organizer || "",
        invitati: [...new Set(p?.invitati ?? [])].sort(),
      });
    if (serie || r.rrule || r.rdate) dettagli.get(uid)!.serie = true;
  }
  const candidati = new Map<string, { peso: boolean[]; e: Record<string, any> }>();
  for (const o of occorrenze) {
    const d = dettagli.get(o.uid);
    const link = MEETING_URL.exec(o.luogo) ?? MEETING_URL.exec(o.descrizione);
    const e = {
      id: `${fonte.id}|${o.uid}|${o.inizio}`,
      account: fonte.account.trim().toLowerCase(),
      alias: fonte.nome,
      calendar: `${fonte.account.trim().toLowerCase()} · ${d?.calendario ?? ""}`,
      title: o.titolo || "(senza titolo)",
      start: o.inizio,
      end: o.fine,
      allDay: o.tuttoGiorno,
      location: o.luogo,
      organizer: d?.organizzatore ?? "",
      organizer_email: d?.email ?? "",
      attendees: d?.invitati ?? [],
      online_url: link ? pulisciLink(link[0]) : null,
      web_link: null,
      fonte: "android",
      tipo,
    };
    const peso = [!d?.serie, Boolean(e.online_url), Boolean(o.luogo), Boolean(e.organizer_email)];
    const chiave = JSON.stringify([e.title, e.start, e.end]);
    const attuale = candidati.get(chiave);
    if (!attuale || pesoMaggiore(peso, attuale.peso)) candidati.set(chiave, { peso, e });
  }
  return [...candidati.values()].map((c) => c.e).sort((a, b) => (a.start < b.start ? -1 : a.start > b.start ? 1 : a.title < b.title ? -1 : a.title > b.title ? 1 : 0));
}
