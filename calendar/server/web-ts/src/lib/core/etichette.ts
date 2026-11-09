import crypto from "node:crypto";
import path from "node:path";
import { config } from "../config";
import { leggiLogo } from "../gateway";
import { invia } from "./agente";
import { interfacciaConLoghi } from "./loghi";
import { personalizzazione } from "./personalizzazione";
import { isDict } from "./py";
import { aliasPerAccount, loadConfig } from "./stato";

// Il nome e il logo di ogni account Microsoft come li mostra la barra del calendario (alias, nomi_per_dominio, loghi), e le aziende
// inoltrate con il nome della casella che ne riceve le copie: la stessa riga in tutte e due le app. Il calendario li prende con l'elenco
// degli account; a TeamsRelay arrivano dall'agente, a cui si mandano con i loghi come data URL quando cambiano.

export type Etichette = { names: Record<string, string>; logos: Record<string, string>; forwarded: Record<string, string> };

const LOGO_MAX = 200_000;

/** I loghi di personalizzazione.json e della finestra Calendari, per nome mostrato: il file in logos/ senza la versione. */
function fileLoghi(): Record<string, string> {
  const loghi = interfacciaConLoghi(personalizzazione().interfaccia).loghi;
  return Object.fromEntries(
    Object.entries(isDict(loghi) ? loghi : {})
      .filter((v): v is [string, string] => typeof v[1] === "string" && v[1].startsWith("logos/"))
      .map(([nome, v]) => [nome, v.slice("logos/".length)]),
  );
}

/** Le etichette degli indirizzi dati: i loghi come indirizzi di questo sito (/logos/...). */
export function etichetteAccount(indirizzi: string[]): Etichette {
  const cfg = loadConfig();
  const inoltrate = [...personalizzazione().aziendeInoltrate.entries()];
  const tutti = [...new Set([...indirizzi, ...inoltrate.map(([, d]) => d.via)].map((i) => i.trim().toLowerCase()).filter((i) => i.includes("@")))];
  const alias = aliasPerAccount(cfg, tutti);
  const names = Object.fromEntries(indirizzi.map((i) => i.trim().toLowerCase()).filter((i) => alias[i]).map((i) => [i, alias[i]]));
  const usati = new Set(Object.values(names));
  const file = fileLoghi();
  const logos = Object.fromEntries([...usati].filter((n) => file[n]).map((n) => [n, "/logos/" + file[n]]));
  const forwarded = Object.fromEntries(inoltrate.filter(([n]) => usati.has(n)).map(([n, d]) => [n, alias[d.via.trim().toLowerCase()] || d.via]));
  return { names, logos, forwarded };
}

type Memoria = { firma: string; letto?: number };
const globale = globalThis as typeof globalThis & { __oeEtichette?: Memoria };

/** Manda le etichette all'agente se sono cambiate dall'ultima volta, con i loghi come data URL. Un errore non ferma chi chiama. */
export async function mandaEtichette(e: Etichette): Promise<void> {
  try {
    const logos: Record<string, string> = {};
    for (const [nome, url] of Object.entries(e.logos)) {
      const logo = await leggiLogo(decodeURIComponent(url.slice("/logos/".length).split("?")[0]!), [path.join(config.dati, "logos"), config.loghiPredefiniti]);
      if (logo && logo.dati.length <= LOGO_MAX && logo.tipo.startsWith("image/")) logos[nome] = `data:${logo.tipo};base64,${Buffer.from(logo.dati).toString("base64")}`;
    }
    const corpo = { names: e.names, logos, forwarded: e.forwarded };
    const firma = crypto.createHash("sha256").update(JSON.stringify(corpo)).digest("hex");
    if (globale.__oeEtichette?.firma === firma) return;
    const r = await invia("/api/auth/shared/labels", corpo);
    if (r.ok === true) globale.__oeEtichette = { ...globale.__oeEtichette, firma };
    else console.warn(`etichette: l'agente non le ha prese (${String(r.errore ?? "")})`);
  } catch (err) {
    console.warn("etichette: non mandate", err instanceof Error ? err.message : err);
  }
}

/** Gli indirizzi dell'elenco degli account dell'agente (/api/auth/shared): le caselle e gli account Teams con il solo browser. */
export function indirizziElenco(r: Record<string, unknown>): string[] {
  const voci = [...(Array.isArray(r.accounts) ? r.accounts : []), ...(Array.isArray(r.teamsOnly) ? r.teamsOnly : [])] as { account?: unknown; email?: unknown }[];
  return voci.map((v) => (typeof v.account === "string" ? v.account : typeof v.email === "string" ? v.email : "")).filter(Boolean);
}

/** Al più ogni 10 minuti, da chi usa il calendario: rilegge l'elenco e manda le etichette se sono cambiate (TeamsRelay le vede anche senza aprire la finestra Account). */
export async function aggiornaEtichette(leggi: () => Promise<Record<string, unknown>>, adessoMs = Date.now()): Promise<void> {
  const m = (globale.__oeEtichette ??= { firma: "" });
  if (m.letto && adessoMs - m.letto < 600_000) return;
  m.letto = adessoMs;
  try {
    const r = await leggi();
    if (r.ok === true) await mandaEtichette(etichetteAccount(indirizziElenco(r)));
  } catch {
    // l'agente non risponde: si riprova tra 10 minuti
  }
}
