import { createHmac } from "node:crypto";

// TOTP di RFC 6238 (HMAC-SHA1, passo di 30 secondi) su node:crypto: le prove generano i codici da sé, senza usare la libreria che li controlla.

const ALFABETO = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/** Base32 di RFC 4648 (senza padding) in byte: così si legge il segreto dell'URI otpauth. */
export function base32(testo: string): Buffer {
  let bit = 0;
  let valore = 0;
  const byte: number[] = [];
  for (const c of testo.replace(/=+$/, "").toUpperCase()) {
    const i = ALFABETO.indexOf(c);
    if (i < 0) throw new Error(`carattere base32 non valido: ${c}`);
    valore = (valore << 5) | i;
    bit += 5;
    if (bit >= 8) {
      byte.push((valore >>> (bit - 8)) & 0xff);
      bit -= 8;
    }
  }
  return Buffer.from(byte);
}

/** HOTP (RFC 4226): le cifre del contatore dato. */
export function hotp(segreto: Buffer, contatore: number, cifre = 6): string {
  const c = Buffer.alloc(8);
  c.writeBigUInt64BE(BigInt(contatore));
  const h = createHmac("sha1", segreto).update(c).digest();
  const o = h[h.length - 1] & 0x0f;
  const n = ((h[o] & 0x7f) << 24) | (h[o + 1] << 16) | (h[o + 2] << 8) | h[o + 3];
  return String(n % 10 ** cifre).padStart(cifre, "0");
}

/** TOTP (RFC 6238) all'istante dato, in secondi. */
export const totp = (segreto: Buffer, secondi = Date.now() / 1000, cifre = 6, passo = 30) => hotp(segreto, Math.floor(secondi / passo), cifre);

/** Il segreto di un URI otpauth://totp/...?secret=... */
export function segretoDaUri(uri: string): Buffer {
  const s = new URL(uri).searchParams.get("secret");
  if (!s) throw new Error("l'URI non ha il segreto");
  return base32(s);
}

/** Un codice che non vale nel passo di adesso né in quelli accanto (il server ne accetta tre). */
export function codiceSbagliato(segreto: Buffer, secondi = Date.now() / 1000): string {
  const buoni = new Set([-2, -1, 0, 1, 2].map((d) => totp(segreto, secondi + d * 30)));
  for (let n = 0; ; n++) {
    const candidato = String(100000 + n * 7919).slice(0, 6);
    if (!buoni.has(candidato)) return candidato;
  }
}
