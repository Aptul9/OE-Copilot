import path from "node:path";

/** Il secondo fattore dell'accesso: spento, codici TOTP con codici di recupero, oppure anche le passkey (che includono il TOTP). */
export const MODI_2FA = ["off", "totp", "passkey"] as const;
export type Modo2fa = (typeof MODI_2FA)[number];

/** AUTH_2FA: senza valore è off; un valore che non è uno dei tre è un errore, così un refuso non spegne il secondo fattore in silenzio. */
export function modo2fa(valore: string | undefined = process.env.AUTH_2FA): Modo2fa {
  const v = (valore ?? "").trim().toLowerCase();
  if (!v) return "off";
  if ((MODI_2FA as readonly string[]).includes(v)) return v as Modo2fa;
  throw new Error(`AUTH_2FA non valido: "${valore}". Valori ammessi: ${MODI_2FA.join(", ")}`);
}

// Letta al momento dell'uso: la build importa i moduli senza database né segreti.
export const config = {
  // Il secondo fattore dell'accesso (AUTH_2FA, default off)
  get auth2fa(): Modo2fa {
    return modo2fa();
  },
  // Indirizzo pubblico: APP_URL, altrimenti https://UI_HOST
  get appUrl() {
    const esplicito = process.env.APP_URL;
    if (esplicito) return esplicito.replace(/\/+$/, "");
    const host = process.env.UI_HOST || "localhost:3000";
    return (/^https?:\/\//.test(host) ? host : `https://${host}`).replace(/\/+$/, "");
  },
  get authSecret() {
    return process.env.BETTER_AUTH_SECRET || "";
  },
  // Database delle sessioni: sta in ui-dati/, la cartella che sopravvive agli aggiornamenti
  get db() {
    return process.env.GATEWAY_DB || "/dati/gateway.db";
  },
  // La cartella dei dati dell'app (config.json, personalizzazione.json, cache, eventi personali, loghi): ui-dati/ sul server
  get dati() {
    return process.env.OE_DATI || "/dati";
  },
  // I loghi che porta l'immagine (personale.png, l'icona degli eventi personali): LOGHI_PREDEFINITI, altrimenti logos-predefiniti accanto a server.js
  get loghiPredefiniti() {
    return process.env.LOGHI_PREDEFINITI || path.join(process.cwd(), "logos-predefiniti");
  },
  // Il database dello stato dell'app (configurazione ed eventi personali): lo stesso delle sessioni; senza GATEWAY_DB, gateway.db nella cartella dei dati
  get dbStato() {
    return process.env.GATEWAY_DB || path.join(this.dati, "gateway.db");
  },
  // L'agente sulla rete di Docker e la sua chiave
  get agenteUrl() {
    return (process.env.AGENTE_URL || "http://agent:8180").replace(/\/+$/, "");
  },
  get agenteChiave() {
    return (process.env.AGENTE_CHIAVE || "").trim();
  },
  get adminEmail() {
    return (process.env.ADMIN_EMAIL || "").trim().toLowerCase();
  },
  get adminPassword() {
    return process.env.ADMIN_PASSWORD || "";
  },
};

export const PASSWORD_MIN = 12;
export const PASSWORD_MAX = 128;
