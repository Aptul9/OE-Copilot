import { randomBytes } from "node:crypto";
import type { createAuth } from "@/lib/auth";
import { verificaSessione } from "@/lib/verifica-sessione";

export const APP = "https://oe.test";

export type Auth = ReturnType<typeof createAuth>;
export type Utente = { email: string; password: string };

// Utenti di prova, a caso e solo in memoria: la password di nessuno vero compare qui.
export const utenteNuovo = (): Utente => ({ email: `prova-${randomBytes(4).toString("hex")}@oe.test`, password: randomBytes(12).toString("hex") });

let indirizzi = 0;

/** Un browser: il suo vasetto dei cookie e un indirizzo diverso a ogni richiesta (il limite per indirizzo non è quello che si prova qui). */
export class Browser {
  jar = new Map<string, string>();
  constructor(private a: Auth, private origine = APP) {}

  async chiama(percorso: string, corpo?: unknown, intestazioniExtra: Record<string, string> = {}) {
    const intestazioni: Record<string, string> = { origin: this.origine, "x-forwarded-for": `10.1.${(++indirizzi >> 8) & 255}.${indirizzi & 255}`, ...intestazioniExtra };
    if (corpo !== undefined) intestazioni["content-type"] = "application/json";
    const cookie = this.cookie;
    if (cookie) intestazioni.cookie = cookie;
    const r = await this.a.handler(
      new Request(this.origine + "/api/auth" + percorso, { method: corpo === undefined ? "GET" : "POST", headers: intestazioni, body: corpo === undefined ? undefined : JSON.stringify(corpo) }),
    );
    for (const riga of r.headers.getSetCookie()) {
      const [coppia, ...attributi] = riga.split(";").map((s) => s.trim());
      const i = coppia.indexOf("=");
      const valore = coppia.slice(i + 1);
      if (!valore || attributi.some((x) => /^max-age=0$/i.test(x))) this.jar.delete(coppia.slice(0, i));
      else this.jar.set(coppia.slice(0, i), valore);
    }
    return r;
  }

  get cookie() {
    return [...this.jar].map(([k, v]) => `${k}=${v}`).join("; ");
  }
  accedi(u: Utente) {
    return this.chiama("/sign-in/email", { email: u.email, password: u.password });
  }
  get conSessione() {
    return [...this.jar.keys()].some((k) => k.endsWith("session_token"));
  }
  async utente() {
    return (await (await this.chiama("/get-session")).json())?.user ?? null;
  }
  /** La risposta del cancello di Caddy (forward_auth) a una richiesta con i cookie di questo browser. */
  async cancello() {
    const req = new Request("http://gateway/api/verifica-sessione", { headers: { cookie: this.cookie, "x-forwarded-uri": "/telefono/", "sec-fetch-dest": "document" } });
    return (await verificaSessione(req, { user: async (h) => (await this.a.api.getSession({ headers: h }))?.user ?? null })).status;
  }
}
