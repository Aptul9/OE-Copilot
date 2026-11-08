import { createHash, generateKeyPairSync, randomBytes, sign, type KeyObject } from "node:crypto";

// Un autenticatore WebAuthn in software (ES256, attestazione "none"): fa le veci del browser nelle prove, così la cerimonia
// di registrazione e di accesso con passkey si prova per intero contro il server, senza dispositivo.

const sha256 = (b: Buffer | string) => createHash("sha256").update(b).digest();
const b64u = (b: Buffer) => b.toString("base64url");

function intestazioneCbor(maggiore: number, n: number): Buffer {
  if (n < 24) return Buffer.from([(maggiore << 5) | n]);
  if (n < 256) return Buffer.from([(maggiore << 5) | 24, n]);
  return Buffer.from([(maggiore << 5) | 25, n >> 8, n & 255]);
}

/** CBOR minimo: interi, byte, testi e mappe (nell'ordine in cui sono scritte). */
function cbor(v: number | string | Buffer | Map<number | string, unknown>): Buffer {
  if (typeof v === "number") return v >= 0 ? intestazioneCbor(0, v) : intestazioneCbor(1, -1 - v);
  if (typeof v === "string") {
    const b = Buffer.from(v);
    return Buffer.concat([intestazioneCbor(3, b.length), b]);
  }
  if (Buffer.isBuffer(v)) return Buffer.concat([intestazioneCbor(2, v.length), v]);
  return Buffer.concat([intestazioneCbor(5, v.size), ...[...v].flatMap(([k, x]) => [cbor(k), cbor(x as Buffer)])]);
}

type Opzioni = { challenge: string; rp?: { id: string }; rpId?: string };

export class Autenticatore {
  readonly idCredenziale = randomBytes(32);
  private chiavi = generateKeyPairSync("ec", { namedCurve: "P-256" });
  contatore = 0;

  constructor(private origine: string, private rpId: string) {}

  /** Cambia il sito per cui si dichiara (per provare che un'origine diversa è rifiutata). */
  per(origine: string, rpId = this.rpId) {
    this.origine = origine;
    this.rpId = rpId;
    return this;
  }

  private datiAutenticatore(flag: number, credenziale?: Buffer) {
    const c = Buffer.alloc(4);
    c.writeUInt32BE(this.contatore);
    const base = Buffer.concat([sha256(this.rpId), Buffer.from([flag]), c]);
    return credenziale ? Buffer.concat([base, credenziale]) : base;
  }

  private datiClient(tipo: string, challenge: string) {
    return Buffer.from(JSON.stringify({ type: tipo, challenge, origin: this.origine, crossOrigin: false }));
  }

  /** La risposta di navigator.credentials.create() alle opzioni di registrazione, nella forma JSON di @simplewebauthn/browser. */
  registra(opzioni: Opzioni) {
    const jwk = this.chiavi.publicKey.export({ format: "jwk" });
    const cose = cbor(
      new Map<number, unknown>([
        [1, 2],
        [3, -7],
        [-1, 1],
        [-2, Buffer.from(jwk.x!, "base64url")],
        [-3, Buffer.from(jwk.y!, "base64url")],
      ]) as Map<number | string, unknown>,
    );
    const lunghezza = Buffer.alloc(2);
    lunghezza.writeUInt16BE(this.idCredenziale.length);
    // UP, UV, dati della credenziale presenti
    const dati = this.datiAutenticatore(0x01 | 0x04 | 0x40, Buffer.concat([Buffer.alloc(16), lunghezza, this.idCredenziale, cose]));
    const attestazione = cbor(new Map<string, unknown>([["fmt", "none"], ["attStmt", new Map()], ["authData", dati]]) as Map<number | string, unknown>);
    return {
      id: b64u(this.idCredenziale),
      rawId: b64u(this.idCredenziale),
      type: "public-key",
      authenticatorAttachment: "platform",
      response: { clientDataJSON: b64u(this.datiClient("webauthn.create", opzioni.challenge)), attestationObject: b64u(attestazione), transports: ["internal"] },
    };
  }

  /** La risposta di navigator.credentials.get() alle opzioni di accesso. */
  firma(opzioni: Opzioni, chiave: KeyObject = this.chiavi.privateKey) {
    this.contatore++;
    const dati = this.datiAutenticatore(0x01 | 0x04);
    const client = this.datiClient("webauthn.get", opzioni.challenge);
    const firma = sign("sha256", Buffer.concat([dati, sha256(client)]), chiave);
    return {
      id: b64u(this.idCredenziale),
      rawId: b64u(this.idCredenziale),
      type: "public-key",
      authenticatorAttachment: "platform",
      response: { clientDataJSON: b64u(client), authenticatorData: b64u(dati), signature: b64u(firma) },
    };
  }

  /** Una chiave che non è quella registrata, per provare una firma falsa. */
  static chiaveEstranea() {
    return generateKeyPairSync("ec", { namedCurve: "P-256" }).privateKey;
  }
}
