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
  get adminEmail() {
    return (process.env.ADMIN_EMAIL || "").trim().toLowerCase();
  },
  get adminPassword() {
    return process.env.ADMIN_PASSWORD || "";
  },
};

export const PASSWORD_MIN = 12;
export const PASSWORD_MAX = 128;
