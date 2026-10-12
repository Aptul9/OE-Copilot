import crypto from "node:crypto";
import { isIP } from "node:net";
import type Database from "better-sqlite3";
import { archivio } from "./archivio";
import { adesso } from "./orologio";
import { isDict } from "./py";
import type { Canale, Esito } from "./promemoria-canali";

// Il canale "notifiche del browser" (Web Push, RFC 8030): il server spedisce il testo al servizio di notifiche del browser (FCM per Chrome,
// Mozilla per Firefox, Apple per Safari) che lo consegna al browser anche a pagina chiusa. Il testo viaggia cifrato da un capo all'altro (RFC
// 8291, aes128gcm): il servizio di notifiche vede a chi e quando, non cosa. Chi manda si identifica con una coppia di chiavi VAPID
// (RFC 8292) che sta nell'ambiente del server. Senza dipendenze: sono una cifratura e una firma di node:crypto. I browser iscritti stanno
// nella tabella push_iscrizioni di gateway.db; l'indirizzo di un'iscrizione vale come una password (chi lo ha può scrivere a quel browser),
// quindi non si scrive in un log e non esce da un metodo dell'API.

export type Iscrizione = { endpoint: string; p256dh: string; auth: string };

export const ISCRIZIONI_MAX = 20;
const TTL_S = 300;
const ATTESA_MS = 10_000;
// un servizio di notifiche accetta almeno 4096 byte di corpo: 86 di intestazione, 16 della firma, 1 di chiusura e il margine
const TESTO_MAX_BYTE = 3800;

const b64 = (b: Buffer) => b.toString("base64url");
/** Un base64url senza altro dentro (niente spazi, niente caratteri che Buffer scarterebbe in silenzio). */
function daB64(s: unknown): Buffer | null {
  if (typeof s !== "string" || !/^[A-Za-z0-9_-]+=*$/.test(s)) return null;
  return Buffer.from(s, "base64url");
}

// ---- Le chiavi VAPID -----------------------------------------------------------------------------------------------------
type ChiaviVapid = { pubblica: string; privata: crypto.KeyObject; soggetto: string };
let memoria: { impronta: string; chiavi: ChiaviVapid | null } | null = null;

/** La coppia di chiavi e il contatto dell'ambiente (VAPID_PUBLIC_KEY, VAPID_PRIVATE_KEY, VAPID_SUBJECT), o null se mancano o non sono una coppia. */
export function chiaviVapid(): ChiaviVapid | null {
  const pubblica = (process.env.VAPID_PUBLIC_KEY ?? "").trim();
  const privata = (process.env.VAPID_PRIVATE_KEY ?? "").trim();
  const soggetto = (process.env.VAPID_SUBJECT ?? "").trim();
  const impronta = `${pubblica}\n${privata}\n${soggetto}`;
  if (memoria?.impronta === impronta) return memoria.chiavi;
  let chiavi: ChiaviVapid | null = null;
  try {
    const p = daB64(pubblica);
    const d = daB64(privata);
    if (p?.length === 65 && p[0] === 4 && d?.length === 32 && /^(mailto:[^@\s]+@[^@\s]+|https:\/\/\S+)$/.test(soggetto)) {
      const x = b64(p.subarray(1, 33));
      const y = b64(p.subarray(33));
      const chiave = crypto.createPrivateKey({ key: { kty: "EC", crv: "P-256", d: b64(d), x, y }, format: "jwk" });
      const derivata = crypto.createPublicKey(chiave).export({ format: "jwk" });
      // la pubblica che sta nell'ambiente deve essere quella della privata: se no ogni invio verrebbe rifiutato dal servizio
      if (derivata.x === x && derivata.y === y) chiavi = { pubblica: b64(p), privata: chiave, soggetto };
    }
  } catch {
    chiavi = null;
  }
  memoria = { impronta, chiavi };
  return chiavi;
}

/** L'intestazione Authorization di RFC 8292: un JWT ES256 per il servizio che riceve (aud), valido 12 ore, e la chiave pubblica. */
export function autorizzazioneVapid(endpoint: string, chiavi: ChiaviVapid, oraMs = adesso()): string {
  const intestazione = b64(Buffer.from(JSON.stringify({ typ: "JWT", alg: "ES256" })));
  const dati = b64(Buffer.from(JSON.stringify({ aud: new URL(endpoint).origin, exp: Math.floor(oraMs / 1000) + 12 * 3600, sub: chiavi.soggetto })));
  const firma = crypto.sign("sha256", Buffer.from(`${intestazione}.${dati}`), { key: chiavi.privata, dsaEncoding: "ieee-p1363" });
  return `vapid t=${intestazione}.${dati}.${b64(firma)}, k=${chiavi.pubblica}`;
}

// ---- La cifratura (RFC 8291) ----------------------------------------------------------------------------------------------
/**
 * Il corpo di un messaggio per un browser: intestazione (sale, dimensione del blocco, chiave pubblica effimera del server) e un solo
 * blocco cifrato con AES-128-GCM. `salt` e `effimera` si passano solo nelle prove, per rifare l'esempio dell'RFC.
 */
export function cifra(testo: string, pubblicaBrowser: Buffer, segretoAuth: Buffer, prove: { salt?: Buffer; effimera?: crypto.ECDH } = {}): Buffer {
  const salt = prove.salt ?? crypto.randomBytes(16);
  let effimera = prove.effimera;
  if (!effimera) {
    effimera = crypto.createECDH("prime256v1");
    effimera.generateKeys();
  }
  const pubblicaServer = effimera.getPublicKey();
  const segreto = effimera.computeSecret(pubblicaBrowser);
  const chiave = Buffer.from(crypto.hkdfSync("sha256", segreto, segretoAuth, Buffer.concat([Buffer.from("WebPush: info\0"), pubblicaBrowser, pubblicaServer]), 32));
  const cek = Buffer.from(crypto.hkdfSync("sha256", chiave, salt, Buffer.from("Content-Encoding: aes128gcm\0"), 16));
  const nonce = Buffer.from(crypto.hkdfSync("sha256", chiave, salt, Buffer.from("Content-Encoding: nonce\0"), 12));
  const cifratore = crypto.createCipheriv("aes-128-gcm", cek, nonce);
  const blocco = Buffer.concat([cifratore.update(Buffer.concat([Buffer.from(testo, "utf-8"), Buffer.from([2])])), cifratore.final(), cifratore.getAuthTag()]);
  const dimensione = Buffer.alloc(4);
  dimensione.writeUInt32BE(4096);
  return Buffer.concat([salt, dimensione, Buffer.from([pubblicaServer.length]), pubblicaServer, blocco]);
}

/** Il testo entro il limite in byte (UTF-8), senza spezzare un carattere. */
function troncaByte(testo: string, max = TESTO_MAX_BYTE): string {
  if (Buffer.byteLength(testo) <= max) return testo;
  let t = testo;
  while (Buffer.byteLength(t + "…") > max) t = t.slice(0, -1);
  // la fine non è mai la metà di una coppia surrogata
  if (/[\ud800-\udbff]$/.test(t)) t = t.slice(0, -1);
  return t + "…";
}

// ---- Le iscrizioni -------------------------------------------------------------------------------------------------------
const SCHEMA = `CREATE TABLE IF NOT EXISTS push_iscrizioni (
  endpoint TEXT PRIMARY KEY,
  p256dh TEXT NOT NULL,
  auth TEXT NOT NULL,
  creata_il INTEGER NOT NULL
)`;
const pronti = new WeakSet<Database.Database>();
function db(): Database.Database {
  const d = archivio();
  if (!pronti.has(d)) {
    d.exec(SCHEMA);
    pronti.add(d);
  }
  return d;
}

export const iscrizioni = (): Iscrizione[] => db().prepare("SELECT endpoint, p256dh, auth FROM push_iscrizioni ORDER BY creata_il, endpoint").all() as Iscrizione[];

/**
 * L'indirizzo di un servizio di notifiche: https, un nome (non un indirizzo IP, non un nome della rete interna), porta 443, senza
 * credenziali. Il server ci manda dei dati: non deve poterlo fare verso un servizio qualunque della sua rete. Solo nelle prove (`locali`) vale anche un indirizzo di questo computer.
 */
function indirizzoValido(v: unknown, locali: boolean): string | null {
  if (typeof v !== "string" || v.length > 2048) return null;
  let u: URL;
  try {
    u = new URL(v);
  } catch {
    return null;
  }
  if (u.username || u.password || u.hash) return null;
  if (locali && (u.hostname === "127.0.0.1" || u.hostname === "localhost") && (u.protocol === "http:" || u.protocol === "https:")) return u.href;
  const nome = u.hostname.replace(/^\[|\]$/g, "");
  if (u.protocol !== "https:" || (u.port && u.port !== "443")) return null;
  if (isIP(nome) || !nome.includes(".") || /(^|\.)(localhost|local|internal|lan|home|corp)$/i.test(nome)) return null;
  return u.href;
}

/** La chiave pubblica del browser (65 byte, un punto vero di P-256: RFC 8291 lo chiede) e il segreto di autenticazione (16 byte). */
function chiaviBrowser(p256dh: unknown, auth: unknown): { p256dh: string; auth: string } | null {
  const p = daB64(p256dh);
  const a = daB64(auth);
  if (!p || !a || p.length !== 65 || p[0] !== 4 || a.length !== 16) return null;
  try {
    crypto.createPublicKey({ key: { kty: "EC", crv: "P-256", x: b64(p.subarray(1, 33)), y: b64(p.subarray(33)) }, format: "jwk" });
  } catch {
    return null;
  }
  return { p256dh: b64(p), auth: b64(a) };
}

/** Salva l'iscrizione che il browser ha fatto (PushSubscription.toJSON()): la stessa di prima si aggiorna. */
export function iscrivi(dati: unknown, locali = false): { ok: true } | { ok: false; error: string } {
  const chiavi = isDict(dati) && isDict(dati.keys) ? chiaviBrowser(dati.keys.p256dh, dati.keys.auth) : null;
  const endpoint = isDict(dati) ? indirizzoValido(dati.endpoint, locali) : null;
  if (!endpoint || !chiavi) return { ok: false, error: "L'iscrizione del browser non è valida." };
  const d = db();
  const esiste = d.prepare("SELECT 1 FROM push_iscrizioni WHERE endpoint = ?").get(endpoint);
  if (!esiste && iscrizioni().length >= ISCRIZIONI_MAX) return { ok: false, error: `Ci sono già ${ISCRIZIONI_MAX} browser iscritti.` };
  d.prepare(
    `INSERT INTO push_iscrizioni(endpoint, p256dh, auth, creata_il) VALUES (?, ?, ?, ?)
     ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth`,
  ).run(endpoint, chiavi.p256dh, chiavi.auth, adesso());
  return { ok: true };
}

/** Toglie l'iscrizione di un browser; se non c'è non cambia niente. */
export function disiscrivi(endpoint: unknown, locali = false): { ok: true } {
  const e = indirizzoValido(endpoint, locali);
  if (e) db().prepare("DELETE FROM push_iscrizioni WHERE endpoint = ?").run(e);
  return { ok: true };
}

// ---- Il canale -----------------------------------------------------------------------------------------------------------
type Consegna = "fatta" | "scaduta" | { errore: string; attesaS?: number };

export function canalePush({ locali = false, attesaMs = ATTESA_MS }: { locali?: boolean; attesaMs?: number } = {}): Canale {
  async function consegna(testo: string, s: Iscrizione, chiavi: ChiaviVapid): Promise<Consegna> {
    let r: Response;
    try {
      r = await fetch(s.endpoint, {
        method: "POST",
        headers: {
          Authorization: autorizzazioneVapid(s.endpoint, chiavi),
          "Content-Encoding": "aes128gcm",
          "Content-Type": "application/octet-stream",
          TTL: String(TTL_S),
          Urgency: "high",
        },
        body: new Uint8Array(cifra(testo, Buffer.from(s.p256dh, "base64url"), Buffer.from(s.auth, "base64url"))),
        signal: AbortSignal.timeout(attesaMs),
        redirect: "manual",
        cache: "no-store",
      });
    } catch (e) {
      const scaduta = e instanceof Error && (e.name === "TimeoutError" || e.name === "AbortError");
      return { errore: scaduta ? "Notifiche del browser: il servizio non risponde (tempo scaduto)" : "Notifiche del browser: servizio non raggiungibile" };
    }
    await r.arrayBuffer().catch(() => null);
    if (r.status >= 200 && r.status < 300) return "fatta";
    if (r.status === 404 || r.status === 410) return "scaduta";
    if (r.status === 429) {
      const attesa = Number(r.headers.get("retry-after"));
      return { errore: "Notifiche del browser: il servizio limita gli invii (429)", attesaS: Number.isFinite(attesa) && attesa > 0 ? attesa : 30 };
    }
    if (r.status === 401 || r.status === 403) return { errore: `Notifiche del browser: il servizio rifiuta le chiavi VAPID (${r.status}): controlla VAPID_PUBLIC_KEY, VAPID_PRIVATE_KEY e VAPID_SUBJECT` };
    if (r.status === 413) return { errore: "Notifiche del browser: messaggio troppo grande (413)" };
    return { errore: r.status >= 500 ? `Notifiche del browser: servizio non disponibile (${r.status})` : `Notifiche del browser: il servizio ha risposto ${r.status}` };
  }

  return {
    nome: "push",
    // pronto a mandare: le chiavi ci sono e almeno un browser è iscritto
    configurato: () => chiaviVapid() !== null && iscrizioni().length > 0,
    dettagli: () => {
      const chiavi = chiaviVapid();
      return { chiavi: chiavi !== null, chiave_pubblica: chiavi?.pubblica ?? null, iscrizioni: chiavi ? iscrizioni().length : 0 };
    },
    async invia(testo): Promise<Esito> {
      const chiavi = chiaviVapid();
      if (!chiavi) return { ok: false, errore: "Notifiche del browser: chiavi VAPID non configurate sul server" };
      const lista = iscrizioni().filter((s) => indirizzoValido(s.endpoint, locali));
      if (!lista.length) return { ok: false, errore: "Notifiche del browser: nessun browser iscritto" };
      const corto = troncaByte(testo);
      const esiti = await Promise.all(lista.map((s) => consegna(corto, s, chiavi)));
      // un'iscrizione che il servizio non conosce più (404, 410) è scaduta: si toglie
      lista.forEach((s, i) => {
        if (esiti[i] === "scaduta") db().prepare("DELETE FROM push_iscrizioni WHERE endpoint = ?").run(s.endpoint);
      });
      // basta un browser che la riceve
      if (esiti.includes("fatta")) return { ok: true };
      const guasti = esiti.filter((e): e is { errore: string; attesaS?: number } => typeof e === "object");
      if (!guasti.length) return { ok: false, errore: "Notifiche del browser: le iscrizioni erano scadute e si sono tolte, riattivale dalle impostazioni" };
      const attesa = Math.max(0, ...guasti.map((g) => g.attesaS ?? 0));
      return { ok: false, errore: guasti[0].errore, ...(attesa ? { attesaS: attesa } : {}) };
    },
  };
}
