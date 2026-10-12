// Un cliente HTTP minimo e i passi di server/strumenti/prova_agente_golden.py (leggi_tutto, leggi_ics, normalizza), per parlare con l'agente
// vero come fa il gateway: una richiesta, la risposta intera.

import http from "node:http";
import net from "node:net";

export interface Risposta {
  stato: number;
  intestazioni: http.IncomingHttpHeaders;
  corpo: string;
  byte: Buffer;
}

export function chiedi(
  base: string,
  metodo: string,
  percorso: string,
  opz: { chiave?: string | null; corpo?: string | Buffer; intestazioni?: Record<string, string> } = {},
): Promise<Risposta> {
  return new Promise((ok, ko) => {
    const u = new URL(base);
    const intestazioni: Record<string, string> = { ...opz.intestazioni };
    if (opz.chiave) intestazioni.Authorization = "Bearer " + opz.chiave;
    if (opz.corpo !== undefined) intestazioni["Content-Length"] = String(Buffer.byteLength(opz.corpo));
    const r = http.request({ host: u.hostname, port: u.port, method: metodo, path: percorso, headers: intestazioni, agent: false }, (res) => {
      const pezzi: Buffer[] = [];
      res.on("data", (c: Buffer) => pezzi.push(c));
      res.on("end", () => {
        const byte = Buffer.concat(pezzi);
        ok({ stato: res.statusCode ?? 0, intestazioni: res.headers, corpo: byte.toString("utf-8"), byte });
      });
    });
    r.on("error", ko);
    if (opz.corpo !== undefined) r.write(opz.corpo);
    r.end();
  });
}

/** Una richiesta scritta byte per byte su un socket: per quello che un cliente HTTP non manderebbe mai. Dà tutto quello che il server risponde fino alla chiusura. */
export function grezza(porta: number, richiesta: string | Buffer, attesaMs = 4000): Promise<Buffer> {
  return new Promise((ok) => {
    const s = net.connect(porta, "127.0.0.1", () => s.write(typeof richiesta === "string" ? Buffer.from(richiesta, "latin1") : richiesta));
    const pezzi: Buffer[] = [];
    s.on("data", (c: Buffer) => pezzi.push(c));
    s.on("error", () => ok(Buffer.concat(pezzi)));
    s.on("close", () => ok(Buffer.concat(pezzi)));
    setTimeout(() => s.destroy(), attesaMs);
  });
}

/** L'ora del giorno cambia a ogni lettura: si toglie, il resto deve restare identico. */
export function normalizza<T>(dati: T): T {
  const nuovo = JSON.parse(JSON.stringify(dati).replace(/alle \d\d:\d\d/g, "alle HH:MM"));
  if (nuovo && typeof nuovo === "object" && !Array.isArray(nuovo) && "generato" in nuovo) nuovo.generato = "ORA";
  return nuovo;
}

export const CHIAVE_PROVA = "chiave-di-prova";
export const codifica = (testo: string) => encodeURIComponent(testo).replace(/[!'()*]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());

export async function leggiTutto(base: string, finestre: string[], chiave = CHIAVE_PROVA) {
  const out: { calendari: unknown; eventi: Record<string, unknown> } = { calendari: normalizza(JSON.parse((await chiedi(base, "GET", "/api/calendari", { chiave })).corpo)), eventi: {} };
  for (const k of finestre) {
    const [inizio, fine] = k.split("|") as [string, string];
    out.eventi[k] = normalizza(JSON.parse((await chiedi(base, "GET", `/api/eventi?inizio=${codifica(inizio)}&fine=${codifica(fine)}`, { chiave })).corpo));
  }
  return out;
}

export async function chiediIcs(base: string, corpo: unknown, chiave: string | null = CHIAVE_PROVA, percorso = "/api/ics") {
  const r = await chiedi(base, "POST", percorso, { chiave, corpo: JSON.stringify(corpo), intestazioni: { "Content-Type": "application/json" } });
  return { stato: r.stato, dati: r.corpo ? (JSON.parse(r.corpo) as Record<string, unknown>) : null };
}

// ---- le richieste dei casi di bordo (golden/agente-bordi.json), costruite come le costruisce il registratore di Python -------------------------------------

export interface CasoRichiesta {
  nome: string;
  scenario: string;
  metodo: string;
  percorso: string;
  chiave?: string;
  senzaChiave?: boolean;
  intestazioni?: string[];
  corpo?: { testo?: string; hex?: string; codifica?: string };
  riempiA?: number;
  lunghezzaDichiarata?: number;
  senzaLunghezza?: boolean;
  senzaCorpo?: boolean;
}

export interface RispostaGrezza {
  stato?: number;
  motivo?: string;
  tipo?: string | null;
  lunghezza?: string | null;
  corpo?: string;
  chiusa?: boolean;
  timeout?: boolean;
  azzerata?: boolean;
}

function corpoDi(r: CasoRichiesta, ics: Record<string, string>): Buffer | null {
  const c = r.corpo;
  if (!c) return null;
  let b: Buffer;
  if (c.hex !== undefined) b = Buffer.from(c.hex, "hex");
  else {
    let t = c.testo ?? "";
    for (const [nome, testo] of Object.entries(ics)) t = t.replaceAll(`@@${nome}@@`, JSON.stringify(testo).slice(1, -1));
    b = Buffer.from(t, "utf-8");
    if (c.codifica === "utf8-bom") b = Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), b]);
  }
  return r.riempiA ? Buffer.concat([b, Buffer.alloc(r.riempiA - b.length, 0x20)]) : b;
}

/** Una richiesta scritta riga per riga su un socket, come fa il registratore di Python, e la risposta letta come la legge lui (fino al Content-Length). */
export function richiestaCompleta(porta: number, r: CasoRichiesta, chiaveDefault: string, ics: Record<string, string>): Promise<RispostaGrezza> {
  const corpo = corpoDi(r, ics);
  const righe = [`${r.metodo} ${r.percorso} HTTP/1.1`, "Host: 127.0.0.1", "Connection: close"];
  const extra = r.intestazioni ?? [];
  if (!r.senzaChiave && !extra.some((x) => x.toLowerCase().startsWith("authorization"))) righe.push("Authorization: Bearer " + (r.chiave ?? chiaveDefault));
  righe.push(...extra);
  if (corpo && !r.senzaLunghezza) righe.push(`Content-Length: ${r.lunghezzaDichiarata ?? corpo.length}`);
  else if (r.lunghezzaDichiarata !== undefined) righe.push(`Content-Length: ${r.lunghezzaDichiarata}`);
  const testa = Buffer.from(righe.join("\r\n") + "\r\n\r\n", "latin1");
  return new Promise((ok) => {
    const s = net.connect(porta, "127.0.0.1");
    let dati = Buffer.alloc(0);
    let finito = false;
    const fine = (x: RispostaGrezza) => {
      if (finito) return;
      finito = true;
      s.destroy();
      ok(x);
    };
    const prova = (chiusa = false) => {
      const i = dati.indexOf("\r\n\r\n");
      if (i < 0) return;
      const linee = dati.subarray(0, i).toString("latin1").split("\r\n");
      const [, codice, ...motivo] = linee[0]!.split(" ");
      const intest = new Map<string, string>();
      for (const l of linee.slice(1)) {
        const k = l.indexOf(":");
        if (!intest.has(l.slice(0, k).trim().toLowerCase())) intest.set(l.slice(0, k).trim().toLowerCase(), l.slice(k + 1).trim());
      }
      const resto = dati.subarray(i + 4);
      const lung = intest.has("content-length") ? Number(intest.get("content-length")) : null;
      // senza Content-Length (la risposta di Node a una richiesta che non legge) il corpo è quello che arriva fino alla chiusura
      if (r.metodo === "HEAD" || (lung !== null && resto.length >= lung) || (lung === null && chiusa))
        fine({ stato: Number(codice), motivo: motivo.join(" "), tipo: intest.get("content-type") ?? null, lunghezza: intest.get("content-length") ?? null, corpo: resto.toString("utf-8") });
    };
    s.setTimeout(r.senzaCorpo ? 4000 : 40000, () => fine({ timeout: true }));
    s.on("connect", () => {
      s.write(testa);
      if (corpo && !r.senzaCorpo) s.write(corpo);
    });
    s.on("data", (c: Buffer) => {
      dati = Buffer.concat([dati, c]);
      prova();
    });
    s.on("error", () => {
      prova(true);
      fine(dati.indexOf("\r\n\r\n") < 0 ? { azzerata: true } : {});
    });
    s.on("close", () => {
      prova(true);
      fine(dati.indexOf("\r\n\r\n") < 0 ? { chiusa: true } : {});
    });
  });
}
