import type { Modo2fa } from "./config";

// Le strade dei plugin che la pagina usa, per metodo. Tutte le altre sotto /two-factor e /passkey (l'invio di codici per
// email o SMS, la rilettura dell'URI del segreto) non servono e restano chiuse: anche quando un plugin le ha, per il
// gateway non esistono.
const TOTP = new Set([
  "POST /two-factor/enable",
  "POST /two-factor/disable",
  "POST /two-factor/generate-backup-codes",
  "POST /two-factor/verify-totp",
  "POST /two-factor/verify-backup-code",
]);
const PASSKEY = new Set([
  "GET /passkey/generate-register-options",
  "GET /passkey/generate-authenticate-options",
  "GET /passkey/list-user-passkeys",
  "POST /passkey/verify-registration",
  "POST /passkey/verify-authentication",
  "POST /passkey/delete-passkey",
  "POST /passkey/update-passkey",
]);

const normalizza = (percorso: string) => {
  let p = percorso;
  try {
    p = decodeURIComponent(p);
  } catch {
    // un percorso con una codifica rotta si guarda così com'è
  }
  return p.toLowerCase().replace(/\/{2,}/g, "/").replace(/\/+$/, "");
};

/**
 * Se una richiesta a /api/auth/<percorso> può arrivare a Better Auth. Fuori da /two-factor e /passkey passa tutto, come prima.
 * Dentro passa solo quello che la modalità del secondo fattore (AUTH_2FA) usa: con off niente, con totp i codici, con
 * passkey i codici e le passkey.
 */
export function percorsoAmmesso(modo: Modo2fa, metodo: string, percorsoAuth: string): boolean {
  const p = normalizza(percorsoAuth);
  if (!/^\/(two-factor|passkey)(\/|$)/.test(p)) return true;
  const voce = `${metodo.toUpperCase()} ${p}`;
  if (modo === "off") return false;
  return TOTP.has(voce) || (modo === "passkey" && PASSKEY.has(voce));
}
