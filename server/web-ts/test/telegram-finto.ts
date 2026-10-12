import http from "node:http";
import type { AddressInfo } from "node:net";
import { vi } from "vitest";

// Un server HTTP locale che fa le veci dell'API di Telegram: le prove dei promemoria non toccano mai api.telegram.org.

export type RichiestaTelegram = { metodo: string; percorso: string; tipo: string | undefined; corpo: Record<string, unknown> };
export type RispostaFinta = { stato: number; corpo?: unknown; testo?: string; intestazioni?: Record<string, string>; ritardoMs?: number };

export const TOKEN_DI_PROVA = "123456:TOKEN-SEGRETO-DI-PROVA";
export const CHAT_DI_PROVA = "987654321";

export async function avviaTelegramFinto() {
  const richieste: RichiestaTelegram[] = [];
  // le risposte dei prossimi invii, in ordine; finite queste ogni invio riesce
  const risposte: RispostaFinta[] = [];
  const server = http.createServer((req, res) => {
    let testo = "";
    req.on("data", (c) => (testo += c));
    req.on("end", () => {
      let corpo: Record<string, unknown> = {};
      try {
        corpo = JSON.parse(testo);
      } catch {
        // un corpo che non è JSON resta vuoto
      }
      richieste.push({ metodo: req.method ?? "", percorso: req.url ?? "", tipo: req.headers["content-type"], corpo });
      const r = risposte.shift() ?? { stato: 200, corpo: { ok: true, result: { message_id: 1 } } };
      const manda = () => {
        try {
          res.writeHead(r.stato, { "Content-Type": "application/json", ...(r.intestazioni ?? {}) });
          res.end(r.testo ?? JSON.stringify(r.corpo ?? {}));
        } catch {
          // il client ha già chiuso (prova del tempo scaduto)
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

/**
 * Fa fallire ogni fetch verso un indirizzo che non è locale: una prova che per sbaglio chiamasse il vero Telegram non esce dal computer.
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
