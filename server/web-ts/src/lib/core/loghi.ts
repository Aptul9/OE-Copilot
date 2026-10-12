import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type Database from "better-sqlite3";
import { archivio, transazione } from "./archivio";
import { nelleDati } from "./dati";
import { controllaImmagine, LATO_MAX, PIXEL_MAX, type TipoImmagine } from "./immagine";
import { personalizzazione } from "./personalizzazione";
import { type Dict, ErroreValore, isDict, messaggio } from "./py";
import { controllaUrl, ErroreRete, LIMITE_ACCETTATO, scaricaSicuro, type SoloProve, TESTI } from "./rete-sicura";

// I loghi delle aziende. Quelli di personalizzazione.json (interfaccia.loghi) restano come sono; sopra si mettono le scelte fatte dalla
// finestra Calendari, tabella `loghi` di gateway.db: nome mostrato -> indirizzo di partenza e file. Un indirizzo lo scarica il gateway
// una volta sola, al momento del salvataggio, e lo tiene in ui-dati/logos/url-<16 cifre di sha256(indirizzo)>.<estensione>: il browser
// chiede sempre e solo /logos/<file> a questo sito, quindi non contatta terzi (niente indirizzo IP svelato, niente Referer, niente logo
// che sparisce quando il sito originale lo toglie). Un indirizzo scritto a mano in personalizzazione.json si scarica allo stesso modo, in
// secondo piano e rinnovato ogni 7 giorni: finché il file non c'è la pagina mostra le iniziali.

export const LOGHI_MAX = 60;
/** Gli indirizzi di personalizzazione.json che si scaricano: gli altri si ignorano. I file gestiti qui sono al più LOGHI_MAX + STATICI_MAX. */
export const STATICI_MAX = 40;
export const TTL_STATICI_MS = 7 * 24 * 3600 * 1000;
const RIPROVA_MS = 5 * 60 * 1000;
const SCARICHI_MAX = 3;
const NOME_MAX = 200;

export const MSG = {
  nomeNonValido: "Nome non valido",
  nonImmagine: "Non è un'immagine",
  svg: "SVG non ammesso: contiene codice o riferimenti esterni",
  pixel: `Immagine troppo grande (massimo ${LATO_MAX} pixel per lato e ${PIXEL_MAX / 1_000_000} megapixel)`,
  troppi: `Troppi loghi scelti da qui (massimo ${LOGHI_MAX})`,
  salvataggio: "Non riesco a salvare il logo sul server",
  nonCe: "Per questo nome non c'è un logo scelto da qui",
  daFile: "Il logo è un file caricato: non c'è un indirizzo da rileggere",
  cambiato: "Il logo è stato cambiato nel frattempo: riprova",
} as const;

/** Un rifiuto con il testo che la pagina mostra. */
export class ErroreLogo extends ErroreValore {}

export type VoceLogo = { url: string | null; tipo: "url" | "file"; aggiornato: number };
type Riga = VoceLogo & { nome: string; file: string };

// ---- La rete, e il suo punto di prova --------------------------------------------------------------------------------------------
let reteDiProva: SoloProve | undefined;

/**
 * Solo per le prove (vitest): permette a un server di prova su 127.0.0.1 di rispondere. Nessun codice del gateway la chiama, nessuna
 * variabile d'ambiente la attiva, e in produzione (NODE_ENV, fissato dalla build) solleva: senza, scaricaSicuro usa solo le regole vere.
 */
export function impostaReteDiProva(prove: SoloProve | undefined): void {
  if (process.env.NODE_ENV === "production") throw new Error("impostaReteDiProva esiste solo per le prove");
  reteDiProva = prove;
}

/** L'indirizzo scritto da chi usa l'app, controllato e ripulito del frammento: sempre lo stesso testo per lo stesso indirizzo. */
function preparaUrl(testo: unknown): string {
  const u = controllaUrl(testo, reteDiProva?.ammettiIndirizzo, reteDiProva?.ammettiPorta);
  u.hash = "";
  return u.href;
}

/** I byte del file di `url`, riconosciuti dai primi byte. Solleva con il testo che la pagina mostra. */
async function scaricaLogo(url: string): Promise<{ tipo: TipoImmagine; dati: Buffer }> {
  const r = await scaricaSicuro(url, reteDiProva);
  return controllato(r.dati);
}

const TESTO_ESITO = { non_immagine: MSG.nonImmagine, svg_non_ammesso: MSG.svg, troppi_pixel: MSG.pixel } as const;

function controllato(dati: Buffer): { tipo: TipoImmagine; dati: Buffer } {
  if (dati.length > LIMITE_ACCETTATO) throw new ErroreRete("troppo_grande", TESTI.troppoGrande);
  const esito = controllaImmagine(dati);
  if ("errore" in esito) throw new ErroreLogo(TESTO_ESITO[esito.errore]);
  return { tipo: esito.tipo, dati };
}

// ---- La tabella ------------------------------------------------------------------------------------------------------------------
const SCHEMA = `CREATE TABLE IF NOT EXISTS loghi (
  nome TEXT PRIMARY KEY,
  url TEXT,
  file TEXT NOT NULL,
  tipo TEXT NOT NULL CHECK (tipo IN ('url', 'file')),
  aggiornato INTEGER NOT NULL
)`;
const pronte = new WeakSet<Database.Database>();

function tabella(): Database.Database {
  const db = archivio();
  if (!pronte.has(db)) {
    db.exec(SCHEMA);
    // dentro una transazione che poi torna indietro la tabella tornerebbe indietro con lei: ci si fida solo di quella creata fuori
    if (!db.inTransaction) pronte.add(db);
  }
  return db;
}

/** All'avvio del gateway: la tabella si aggiunge (CREATE TABLE IF NOT EXISTS), mai si cambia o si toglie niente di quello che c'è. */
export function migraLoghi(): void {
  tabella();
}

const righe = (): Riga[] => tabella().prepare("SELECT nome, url, file, tipo, aggiornato FROM loghi ORDER BY nome").all() as Riga[];
const riga = (nome: string): Riga | undefined => tabella().prepare("SELECT nome, url, file, tipo, aggiornato FROM loghi WHERE nome = ?").get(nome) as Riga | undefined;

/** I loghi scelti dalla finestra Calendari, per nome mostrato. */
export function leggiVoci(): Record<string, VoceLogo> {
  return Object.fromEntries(righe().map((r) => [r.nome, { url: r.url, tipo: r.tipo, aggiornato: r.aggiornato }]));
}

function nomeValido(nome: unknown): string {
  const n = typeof nome === "string" ? nome.trim() : "";
  if (!n || n.length > NOME_MAX || /[\u0000-\u001f\u007f]/.test(n)) throw new ErroreLogo(MSG.nomeNonValido);
  return n;
}

// ---- I file ------------------------------------------------------------------------------------------------------------------------
const cartella = () => nelleDati("logos");
const hash16 = (dati: string | Buffer) => crypto.createHash("sha256").update(dati).digest("hex").slice(0, 16);
/** Solo questi nomi sono dei loghi gestiti qui: tutto il resto della cartella (i file messi a mano) non si tocca mai. */
const GESTITO = /^(?:url|up)-[0-9a-f]{16}\.(?:png|jpg|gif|webp|ico|svg)$/;
const TEMPORANEO = /^\.logo-tmp-/;

function rimuovi(nome: string): void {
  try {
    fs.rmSync(path.join(cartella(), nome), { force: true });
  } catch (e) {
    // un file aperto da qualcun altro (Windows): ci si riprova alla prossima pulizia
    console.warn(`loghi: ${nome} non rimosso`, e);
  }
}

/** Scrive il file tutto insieme (prima uno temporaneo, poi la sostituzione): chi lo legge non vede mai un file a metà. */
function scrivi(nome: string, dati: Buffer): void {
  fs.mkdirSync(cartella(), { recursive: true });
  const tmp = path.join(cartella(), `.logo-tmp-${process.pid}-${crypto.randomBytes(6).toString("hex")}`);
  fs.writeFileSync(tmp, dati, { mode: 0o644 });
  try {
    fs.renameSync(tmp, path.join(cartella(), nome));
  } catch (e) {
    fs.rmSync(tmp, { force: true });
    throw e;
  }
}

/** L'indirizzo di un valore di personalizzazione.json, o null se il valore è un file. */
function urlDaValore(v: unknown): string | null {
  if (typeof v !== "string" || !/^(?:https?:)?\/\//i.test(v.trim())) return null;
  try {
    const t = v.trim();
    const u = new URL(t.startsWith("//") ? `https:${t}` : t);
    u.hash = "";
    return u.href;
  } catch {
    return null;
  }
}

/** Gli indirizzi di personalizzazione.json che si scaricano, senza ripetizioni e fino a STATICI_MAX. */
function urlStatici(): string[] {
  const loghi = personalizzazione().interfaccia.loghi;
  const trovati = new Set<string>();
  for (const v of isDict(loghi) ? Object.values(loghi) : []) {
    const u = urlDaValore(v);
    if (u && trovati.size < STATICI_MAX) trovati.add(u);
  }
  return [...trovati];
}

/**
 * Toglie i file gestiti che nessuno usa più: quelli che nessuna riga della tabella indica e che non sono di un indirizzo di
 * personalizzazione.json, e gli avanzi di una scrittura interrotta. Si chiama dopo ogni modifica, a transazione chiusa. Così la cartella
 * non cresce: ci sono al più LOGHI_MAX file scelti da qui e STATICI_MAX di personalizzazione.json.
 */
export function pulisciLoghi(): void {
  let nomi: string[];
  try {
    nomi = fs.readdirSync(cartella());
  } catch {
    return;
  }
  const usati = new Set(righe().map((r) => r.file));
  const statici = new Set(urlStatici().map(hash16));
  for (const f of nomi) {
    if (TEMPORANEO.test(f)) rimuovi(f);
    else if (GESTITO.test(f) && !usati.has(f) && !(f.startsWith("url-") && statici.has(f.slice(4, 20)))) rimuovi(f);
  }
}

// ---- Le scelte fatte dalla finestra ----------------------------------------------------------------------------------------------
/** Salva la scelta: il file, la riga, e poi la pulizia di quello che non serve più. Tutto in un blocco, senza attese: due salvataggi non si intrecciano. */
function registra(nome: string, tipo: "url" | "file", url: string | null, img: TipoImmagine, dati: Buffer): VoceLogo {
  const file = `${tipo === "url" ? "url" : "up"}-${hash16(tipo === "url" ? (url as string) : dati)}.${img.ext}`;
  const aggiornato = Date.now();
  tabella();
  try {
    transazione(() => {
      const db = tabella();
      if (!riga(nome) && (db.prepare("SELECT COUNT(*) AS n FROM loghi").get() as { n: number }).n >= LOGHI_MAX) throw new ErroreLogo(MSG.troppi);
      scrivi(file, dati);
      db.prepare(
        "INSERT INTO loghi(nome, url, file, tipo, aggiornato) VALUES (?, ?, ?, ?, ?) ON CONFLICT(nome) DO UPDATE SET url = excluded.url, file = excluded.file, tipo = excluded.tipo, aggiornato = excluded.aggiornato",
      ).run(nome, url, file, tipo, aggiornato);
    });
  } finally {
    pulisciLoghi();
  }
  return { url, tipo, aggiornato };
}

/** Scarica l'indirizzo e lo mostra come anteprima, senza salvare niente. */
export async function anteprimaLogo(indirizzo: unknown): Promise<{ anteprima: string; tipo: string; byte: number }> {
  const { tipo, dati } = await scaricaLogo(preparaUrl(indirizzo));
  return { anteprima: `data:${tipo.mime};base64,${dati.toString("base64")}`, tipo: tipo.mime, byte: dati.length };
}

/** Il logo di `nome` da un indirizzo: scaricato qui, subito, una volta. Se qualcosa non va non cambia niente. */
export async function impostaLogo(nome: unknown, indirizzo: unknown): Promise<VoceLogo> {
  const n = nomeValido(nome);
  const url = preparaUrl(indirizzo);
  const { tipo, dati } = await scaricaLogo(url);
  return registra(n, "url", url, tipo, dati);
}

/** Il logo di `nome` da un file caricato (base64): stesse regole dell'indirizzo, stessi limiti. */
export function caricaLogo(nome: unknown, base64: unknown): VoceLogo {
  const n = nomeValido(nome);
  // il corpo della richiesta è già limitato a 1 MB (gateway.ts, MAX_BODY): qui basta che sia base64
  if (typeof base64 !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(base64)) throw new ErroreLogo(MSG.nonImmagine);
  const { tipo, dati } = controllato(Buffer.from(base64, "base64"));
  return registra(n, "file", null, tipo, dati);
}

/** Legge di nuovo l'indirizzo del logo di `nome`. Un logo cambiato o tolto nel frattempo da un'altra richiesta non si sovrascrive. */
export async function aggiornaLogo(nome: unknown): Promise<VoceLogo> {
  const n = nomeValido(nome);
  const prima = riga(n);
  if (!prima) throw new ErroreLogo(MSG.nonCe);
  if (prima.tipo !== "url" || !prima.url) throw new ErroreLogo(MSG.daFile);
  const { tipo, dati } = await scaricaLogo(prima.url);
  if (riga(n)?.url !== prima.url) throw new ErroreLogo(MSG.cambiato);
  return registra(n, "url", prima.url, tipo, dati);
}

/** Torna a quello di personalizzazione.json (o alle iniziali): la scelta si toglie e il suo file con lei, se nessuno lo usa. */
export function togliLogo(nome: unknown): void {
  const n = nomeValido(nome);
  transazione(() => void tabella().prepare("DELETE FROM loghi WHERE nome = ?").run(n));
  pulisciLoghi();
}

/**
 * Il nome di un'azienda cambia e il suo logo la segue. Si chiama dentro la transazione della configurazione (alias e logo cambiano insieme);
 * il file di una scelta che si perde, perché il nome nuovo ne aveva già una, lo toglie la pulizia che chi chiama fa dopo.
 */
export function rinominaLogo(prima: string, dopo: string): void {
  if (prima === dopo) return;
  const db = tabella();
  if (!riga(prima)) return;
  if (riga(dopo)) db.prepare("DELETE FROM loghi WHERE nome = ?").run(prima);
  else db.prepare("UPDATE loghi SET nome = ? WHERE nome = ?").run(dopo, prima);
}

/** Come lo mostra la pagina: un testo chiaro, e nel log il resto se non è un rifiuto previsto. */
export function esitoNegativo(nomeMetodo: string, e: unknown): { ok: false; error: string } {
  if (e instanceof ErroreValore) return { ok: false, error: e.message };
  console.error(`${nomeMetodo}: eccezione`, e);
  return { ok: false, error: MSG.salvataggio };
}

// ---- Gli indirizzi scritti a mano in personalizzazione.json -----------------------------------------------------------------------
type Stato = { inVolo: Map<string, Promise<void>>; fallite: Map<string, number> };
const globale = globalThis as typeof globalThis & { __oeLoghi?: Stato };
const stato = (): Stato => (globale.__oeLoghi ??= { inVolo: new Map(), fallite: new Map() });

/** Aspetta gli scarichi in corso (per le prove). */
export const attendiScarichi = (): Promise<unknown> => Promise.all([...stato().inVolo.values()]);
/** Dimentica gli scarichi falliti, che altrimenti si ritentano dopo 5 minuti (per le prove). */
export const azzeraScarichi = (): void => stato().fallite.clear();

function fileInCache(url: string): { file: string; mtime: number } | null {
  const prefisso = `url-${hash16(url)}.`;
  let migliore: { file: string; mtime: number } | null = null;
  try {
    for (const f of fs.readdirSync(cartella())) {
      if (!f.startsWith(prefisso) || !GESTITO.test(f)) continue;
      const mtime = fs.statSync(path.join(cartella(), f)).mtimeMs;
      if (!migliore || mtime > migliore.mtime) migliore = { file: f, mtime };
    }
  } catch {
    // la cartella non c'è ancora
  }
  return migliore;
}

function scriviStatico(url: string, img: TipoImmagine, dati: Buffer): void {
  const file = `url-${hash16(url)}.${img.ext}`;
  scrivi(file, dati);
  // se il tipo del file è cambiato resta una copia sola
  for (const f of fs.readdirSync(cartella())) if (f !== file && f.startsWith(`url-${hash16(url)}.`) && GESTITO.test(f)) rimuovi(f);
}

/** Avvia in secondo piano lo scarico di un indirizzo di personalizzazione.json: uno alla volta per indirizzo, al più SCARICHI_MAX insieme, e dopo un errore si riprova dopo 5 minuti. */
function avviaScarico(url: string): void {
  const s = stato();
  const fallita = s.fallite.get(url);
  if (s.inVolo.has(url) || s.inVolo.size >= SCARICHI_MAX || (fallita !== undefined && Date.now() - fallita < RIPROVA_MS)) return;
  const lavoro = (async () => {
    try {
      const { tipo, dati } = await scaricaLogo(url);
      scriviStatico(url, tipo, dati);
      s.fallite.delete(url);
      pulisciLoghi();
    } catch (e) {
      s.fallite.set(url, Date.now());
      if (s.fallite.size > 4 * STATICI_MAX) s.fallite.delete(s.fallite.keys().next().value as string);
      // solo il sito, mai l'indirizzo intero: potrebbe portarsi dietro un token
      console.warn(`loghi: ${new URL(url).host}: ${messaggio(e)}`);
    } finally {
      s.inVolo.delete(url);
    }
  })();
  s.inVolo.set(url, lavoro);
}

/**
 * I valori di interfaccia.loghi con gli indirizzi sostituiti dal file in cache; senza il file (ancora) la voce manca e il nome va in
 * `inCorso`. Un nome che ha una scelta fatta dalla finestra non si scarica: vince la scelta.
 */
function risolviStatici(statici: Dict, sostituiti: ReadonlySet<string>): { valori: [string, unknown][]; inCorso: string[] } {
  const valori: [string, unknown][] = [];
  const inCorso: string[] = [];
  const ammessi = new Set(urlStatici());
  for (const [nome, v] of Object.entries(statici)) {
    const url = urlDaValore(v);
    if (!url) {
      valori.push([nome, v]);
      continue;
    }
    if (!ammessi.has(url) || sostituiti.has(nome)) continue;
    const c = fileInCache(url);
    if (!c || Date.now() - c.mtime > TTL_STATICI_MS) avviaScarico(url);
    if (c) valori.push([nome, `logos/${c.file}?v=${Math.trunc(c.mtime)}`]);
    else inCorso.push(nome);
  }
  return { valori, inCorso };
}

/**
 * L'interfaccia di personalizzazione.json con i loghi uniti: prima quelli scritti nel file (un indirizzo diventa il suo file in cache, o
 * manca finché non è scaricato), sopra le scelte fatte dalla finestra, che vincono. Senza né l'una né le altre è l'oggetto di prima.
 */
export function interfacciaConLoghi(interfaccia: Dict): Dict {
  const statici = isDict(interfaccia.loghi) ? interfaccia.loghi : null;
  const scelte = righe();
  if (!statici && !scelte.length) return interfaccia;
  const { valori, inCorso } = statici ? risolviStatici(statici, new Set(scelte.map((r) => r.nome))) : { valori: [], inCorso: [] };
  const fuori: Dict = {
    ...interfaccia,
    loghi: Object.fromEntries([...valori, ...scelte.map((r) => [r.nome, `logos/${r.file}?v=${r.aggiornato}`] as [string, unknown])]),
  };
  if (inCorso.length) fuori.loghi_in_corso = inCorso;
  return fuori;
}
