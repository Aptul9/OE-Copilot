import crypto from "node:crypto";
import http from "node:http";
import type { AddressInfo } from "node:net";
import { vi } from "vitest";

// Un servizio di notifiche finto (un server HTTP locale) e un "browser" finto che sa decifrare: le prove delle notifiche del browser non
// toccano mai un servizio vero (FCM, Mozilla, Apple).

/**
 * Fa fallire ogni fetch verso un indirizzo che non è locale: una prova che per sbaglio chiamasse un servizio vero non esce dal computer.
 * `vietate` elenca i nomi dei server che si è provato a raggiungere (le prove controllano che sia vuoto).
 */
export function soloRetiLocali() {
  const reale = globalThis.fetch;
  const vietate: string[] = [];
  const spia = vi.spyOn(globalThis, "fetch").mockImplementation((input, init) => {
    const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
    if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
      vietate.push(url.hostname);
      return Promise.reject(new Error("rete vietata nelle prove"));
    }
    return reale(input, init);
  });
  return { spia, vietate };
}

export type RichiestaPush = { percorso: string; intestazioni: http.IncomingHttpHeaders; corpo: Buffer };
export type RispostaPush = { stato: number; intestazioni?: Record<string, string>; ritardoMs?: number };

export async function avviaPushFinto() {
  const richieste: RichiestaPush[] = [];
  // le risposte dei prossimi invii, in ordine; finite queste ogni invio riesce (201)
  const risposte: RispostaPush[] = [];
  const server = http.createServer((req, res) => {
    const pezzi: Buffer[] = [];
    req.on("data", (c: Buffer) => pezzi.push(c));
    req.on("end", () => {
      richieste.push({ percorso: req.url ?? "", intestazioni: req.headers, corpo: Buffer.concat(pezzi) });
      const r = risposte.shift() ?? { stato: 201 };
      const manda = () => {
        try {
          res.writeHead(r.stato, r.intestazioni ?? {});
          res.end();
        } catch {
          // il client ha già chiuso
        }
      };
      if (r.ritardoMs) setTimeout(manda, r.ritardoMs);
      else manda();
    });
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  return {
    url,
    richieste,
    risposte,
    chiudi: () =>
      new Promise<void>((ok) => {
        server.closeAllConnections();
        server.close(() => ok());
      }),
  };
}

/** Un browser iscritto: la chiave e il segreto che manderebbe al server, e quello che serve a decifrare. */
export function browserFinto(endpoint: string) {
  const ecdh = crypto.createECDH("prime256v1");
  ecdh.generateKeys();
  const auth = crypto.randomBytes(16);
  return {
    ecdh,
    auth,
    iscrizione: { endpoint, expirationTime: null, keys: { p256dh: ecdh.getPublicKey().toString("base64url"), auth: auth.toString("base64url") } },
  };
}

/**
 * Decifra il corpo di un messaggio come farebbe il browser (RFC 8188 e 8291): sale, dimensione del blocco e chiave pubblica del server stanno
 * nell'intestazione, il resto è un solo blocco AES-128-GCM che finisce con l'etichetta di 16 byte e un delimitatore 0x02.
 */
export function decifra(corpo: Buffer, browser: { ecdh: crypto.ECDH; auth: Buffer }): string {
  const salt = corpo.subarray(0, 16);
  const dimensione = corpo.readUInt32BE(16);
  const lunghezzaChiave = corpo[20];
  const chiaveServer = corpo.subarray(21, 21 + lunghezzaChiave);
  const blocco = corpo.subarray(21 + lunghezzaChiave);
  if (dimensione !== 4096 || lunghezzaChiave !== 65 || chiaveServer[0] !== 4) throw new Error("intestazione non valida");
  const segreto = browser.ecdh.computeSecret(chiaveServer);
  const ikm = Buffer.from(crypto.hkdfSync("sha256", segreto, browser.auth, Buffer.concat([Buffer.from("WebPush: info\0"), browser.ecdh.getPublicKey(), chiaveServer]), 32));
  const cek = Buffer.from(crypto.hkdfSync("sha256", ikm, salt, Buffer.from("Content-Encoding: aes128gcm\0"), 16));
  const nonce = Buffer.from(crypto.hkdfSync("sha256", ikm, salt, Buffer.from("Content-Encoding: nonce\0"), 12));
  const d = crypto.createDecipheriv("aes-128-gcm", cek, nonce);
  d.setAuthTag(blocco.subarray(blocco.length - 16));
  const chiaro = Buffer.concat([d.update(blocco.subarray(0, blocco.length - 16)), d.final()]);
  if (chiaro[chiaro.length - 1] !== 2) throw new Error("delimitatore non valido");
  return chiaro.subarray(0, chiaro.length - 1).toString("utf-8");
}

/** Una coppia di chiavi VAPID nuova, nel formato dell'ambiente del server. */
export function chiaviVapidDiProva() {
  const { privateKey } = crypto.generateKeyPairSync("ec", { namedCurve: "P-256" });
  const jwk = privateKey.export({ format: "jwk" });
  const pubblica = Buffer.concat([Buffer.from([4]), Buffer.from(jwk.x as string, "base64url"), Buffer.from(jwk.y as string, "base64url")]).toString("base64url");
  return { VAPID_PUBLIC_KEY: pubblica, VAPID_PRIVATE_KEY: jwk.d as string, VAPID_SUBJECT: "mailto:prova@example.test" };
}
