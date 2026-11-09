// I nomi e i loghi degli account Microsoft come li mostra il calendario, mandati dal gateway (POST /api/auth/shared/labels, chiave piena):
// l'agente li tiene su disco e li ridà con l'elenco degli account (GET /api/auth/shared), così TeamsRelay mostra gli stessi del calendario.
//   {"names": {"<indirizzo>": "NPO"}, "logos": {"NPO": "data:image/png;base64,..."}, "forwarded": {"NPO": "Bandpey"}}
// forwarded: le aziende le cui riunioni arrivano copiate in un'altra casella, con il nome di quella casella.
import fs from "node:fs";

export type Etichette = { names: Record<string, string>; logos: Record<string, string>; forwarded: Record<string, string> };

export class ErroreEtichette extends Error {}

const MAX_VOCI = 128;
const MAX_LOGO = 300_000;
const EMAIL = /^[^\s@]{1,128}@[^\s@]{1,253}$/;
const DATA_URL = /^data:image\/(?:png|jpeg|gif|webp|svg\+xml|vnd\.microsoft\.icon|x-icon);base64,[A-Za-z0-9+/]+=*$/;

const nome = (v: unknown): string => {
  if (typeof v !== "string" || !v.trim() || v.length > 64 || /[\x00-\x1f\x7f]/.test(v)) throw new ErroreEtichette("nome non valido");
  return v.trim();
};

function mappa(v: unknown, campo: string, chiave: (k: string) => string, valore: (x: unknown) => string): Record<string, string> {
  if (v === undefined) return {};
  if (v === null || typeof v !== "object" || Array.isArray(v)) throw new ErroreEtichette(`${campo}: serve un oggetto`);
  const voci = Object.entries(v as Record<string, unknown>);
  if (voci.length > MAX_VOCI) throw new ErroreEtichette(`${campo}: al massimo ${MAX_VOCI} voci`);
  return Object.fromEntries(voci.map(([k, x]) => [chiave(k), valore(x)]));
}

/** Il corpo del gateway, controllato: indirizzi, nomi brevi, loghi solo come immagini in data URL. Nessun altro campo. */
export function leggiEtichette(corpo: unknown): Etichette {
  if (corpo === null || typeof corpo !== "object" || Array.isArray(corpo)) throw new ErroreEtichette('serve {"names", "logos", "forwarded"}');
  const r = corpo as Record<string, unknown>;
  const estranei = Object.keys(r).filter((k) => !["names", "logos", "forwarded"].includes(k));
  if (estranei.length) throw new ErroreEtichette(`campi non ammessi: ${estranei.join(", ").slice(0, 80)}`);
  return {
    names: mappa(r.names, "names", (k) => {
      const e = k.trim().toLowerCase();
      if (!EMAIL.test(e)) throw new ErroreEtichette("names: indirizzo non valido");
      return e;
    }, nome),
    logos: mappa(r.logos, "logos", nome, (x) => {
      if (typeof x !== "string" || x.length > MAX_LOGO || !DATA_URL.test(x)) throw new ErroreEtichette("logos: serve un'immagine in data URL");
      return x;
    }),
    forwarded: mappa(r.forwarded, "forwarded", nome, nome),
  };
}

/** Le ultime etichette salvate, o nessuna se il file manca o non è valido. */
export function caricaEtichette(file: string): Etichette | null {
  try {
    return leggiEtichette(JSON.parse(fs.readFileSync(file, "utf8")));
  } catch {
    return null;
  }
}

export function salvaEtichette(file: string, e: Etichette): void {
  fs.writeFileSync(file + ".tmp", JSON.stringify(e), { mode: 0o600 });
  fs.renameSync(file + ".tmp", file);
}
