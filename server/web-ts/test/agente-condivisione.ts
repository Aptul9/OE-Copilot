import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

// L'agente finto delle prove della condivisione del calendario: risponde a qualunque finestra con gli eventi che si dichiarano (a differenza di
// golden-comune.ts, che ripete risposte registrate per finestre fisse), e tiene l'elenco delle richieste per vedere come si legge. Più la cartella dei dati.

export const CHIAVE_AGENTE = "chiave-di-prova";

export type EventoFinto = {
  account: string;
  title: string;
  start: string;
  end: string;
  allDay?: boolean;
  calendar?: string;
  location?: string;
  organizer?: string;
  organizer_email?: string;
  online_url?: string | null;
  attendees?: string[];
};

/** `perTesto`: l'evento si espande solo dal testo ICS che contiene questa riga (l'agente vero legge il testo; qui ci si finge di farlo). */
export type EventoIcsFinto = { uid: string; inizio: string; fine: string; titolo: string; tutto_giorno?: boolean; luogo?: string; organizzatore?: string; link?: string | null; perTesto?: string };

export type StatoFinto = { stato: "ok" | "avviso" | "errore"; msg: string };

export type AgenteFinto = {
  url: string;
  eventi: EventoFinto[];
  /** Gli eventi che l'agente espande dai calendari ICS (POST /api/ics con un intervallo). */
  ics: EventoIcsFinto[];
  /** Con true l'agente risponde 500 a tutto: com'è quando non si raggiunge. */
  giu: boolean;
  /** Le caselle che l'agente legge anche se non hanno eventi (un calendar.db che non si legge le lascia senza eventi, ma sono nell'elenco). */
  caselle: string[];
  /** Lo stato che una casella dichiara (per account): senza voce è ok. L'agente vero risponde 200 anche con una casella in errore, e lo dice qui. */
  stati: Record<string, StatoFinto>;
  /** Il segno di vita dell'aggiornamento sul server (`servizio` di /api/calendari e /api/eventi): di partenza sano. */
  servizio: Record<string, unknown> | null;
  /** Le richieste ricevute: "GET /api/eventi?inizio=...&fine=...". */
  richieste: string[];
  chiudi: () => Promise<void>;
};

export const SERVIZIO_SANO = { eta_s: 5, pausa: false, telefono: true };

export async function avviaAgenteFinto(eventi: EventoFinto[] = []): Promise<AgenteFinto> {
  const agente: AgenteFinto = { url: "", eventi, ics: [], giu: false, caselle: [], stati: {}, servizio: SERVIZIO_SANO, richieste: [], chiudi: async () => {} };
  const server = http.createServer((req, res) => {
    const risposta = (codice: number, corpo: unknown) => void res.writeHead(codice, { "Content-Type": "application/json; charset=utf-8" }).end(JSON.stringify(corpo));
    agente.richieste.push(`${req.method} ${req.url}`);
    if (agente.giu) return risposta(500, { errore: "finto: giù" });
    if (req.headers.authorization !== "Bearer " + CHIAVE_AGENTE) return risposta(401, { errore: "non autorizzato" });
    const url = new URL(req.url ?? "", "http://x");
    const caselle = [...new Set([...agente.eventi.map((e) => e.account), ...agente.caselle])].map((account) => ({
      account,
      nome: account,
      tipo: "outlook",
      eventi: 1,
      ...(agente.stati[account] ?? { stato: "ok", msg: "" }),
    }));
    const errori = caselle.filter((c) => c.stato === "errore").map((c) => `${c.nome}: ${c.msg}`);
    if (req.method === "POST" && url.pathname === "/api/ics") {
      let corpo = "";
      req.on("data", (c) => (corpo += c));
      req.on("end", () => {
        const r = JSON.parse(corpo);
        // senza intervallo il testo si valida soltanto e si dà il nome che dichiara (X-WR-CALNAME), come fa l'agente vero
        if (r.inizio === undefined) return risposta(200, { ok: true, nome: /^X-WR-CALNAME:(.*?)\r?$/m.exec(r.testo)?.[1]?.replace(/\\([,;\\])/g, "$1") ?? "" });
        const eventiIcs = agente.ics
          .filter((e) => e.fine > r.inizio.slice(0, 19) && e.inizio < r.fine.slice(0, 19) && (!e.perTesto || String(r.testo).includes(e.perTesto)))
          .map(({ perTesto, ...e }) => ({ tutto_giorno: false, luogo: "", organizzatore: "", link: null, ...e }));
        risposta(200, { ok: true, eventi: eventiIcs });
      });
      return;
    }
    if (url.pathname === "/api/calendari") return risposta(200, { caselle, servizio: agente.servizio });
    if (url.pathname === "/api/eventi") {
      const inizio = (url.searchParams.get("inizio") ?? "").slice(0, 19);
      const fine = (url.searchParams.get("fine") ?? "").slice(0, 19);
      const dentro = agente.eventi.filter((e) => e.end > inizio && e.start < fine).map((e, n) => ({ id: `finto|${e.account}|${n}|${e.start}`, ...e }));
      return risposta(200, { eventi: dentro, errori, fonti: {}, caselle, servizio: agente.servizio });
    }
    risposta(404, { errore: "sconosciuto" });
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  agente.url = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  agente.chiudi = () =>
    new Promise<void>((ok) => {
      server.closeAllConnections();
      server.close(() => ok());
    });
  return agente;
}

/** Una cartella dei dati di prova nuova, con (se si vuole) personalizzazione.json: la si cancella a fine prova. */
export function cartellaDati(personalizzazione?: Record<string, unknown>): string {
  const dati = fs.mkdtempSync(path.join(os.tmpdir(), "oe-condivisione-"));
  if (personalizzazione) fs.writeFileSync(path.join(dati, "personalizzazione.json"), JSON.stringify(personalizzazione));
  return dati;
}

/** Un testo ICS già "scaricato": la copia che il gateway tiene su disco per un calendario da link, fresca, così non si apre nessuna rete. */
export function copiaIcs(dati: string, id: string, testo: string): void {
  fs.mkdirSync(path.join(dati, "ics_cache"), { recursive: true });
  fs.writeFileSync(path.join(dati, "ics_cache", id.replace(/[^A-Za-z0-9._-]/g, "_") + ".ics"), testo);
}

/** Quello che non deve uscire mai nel livello "occupato": titoli, luoghi, chi organizza, indirizzi di posta, link per collegarsi. */
export const CORPUS = {
  titoli: ["Colloquio riservato Rossi", "Licenziamento Bianchi urgente", "Visita cardiologica Verdi", "Trattativa acquisto Gamma", "Videochiamata fornitore"],
  luoghi: ["Studio Dr. Verdi, via Roma 12", "Sala Rossa", "Ospedale Maggiore, reparto 4", "Piazza Duomo 1"],
  organizzatori: ["Anna Rossi", "Luca Bianchi"],
  posta: ["anna.rossi@alfa.example", "luca.bianchi@beta.example", "mario@alfa.example", "mario@beta.example"],
  conferenze: [
    "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?x=1&y=2",
    "https://zoom.us/j/123456789?pwd=segretissima",
    "https://meet.google.com/abc-defg-hij",
    "https://example.webex.com/meet/stanza-privata",
  ],
  aziende: ["Alfa", "Beta"],
  note: ["note private del dentista"],
};
export const TUTTO_IL_CORPUS: string[] = Object.values(CORPUS).flat();
