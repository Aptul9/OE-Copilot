// Letta al momento dell'uso: la build importa i moduli senza database né segreti.
export const config = {
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
  // Il servizio Python con la pagina e l'API, non pubblicato: ci si arriva solo da qui
  get upstream() {
    return (process.env.UI_UPSTREAM || "http://ui:8181").replace(/\/+$/, "");
  },
  // La cartella dei dati dell'app (config.json, personalizzazione.json, cache, eventi personali, loghi): ui-dati/ sul server
  get dati() {
    return process.env.OE_DATI || "/dati";
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
