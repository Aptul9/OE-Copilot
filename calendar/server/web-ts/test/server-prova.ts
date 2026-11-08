import http from "node:http";
import type { AddressInfo } from "node:net";
import { indirizzoAmmesso, type SoloProve } from "@/lib/core/rete-sicura";

// Un server HTTP di prova su 127.0.0.1, con le risposte che una prova chiede: un percorso, una risposta. Conta le richieste arrivate:
// una richiesta che la guardia doveva fermare prima di collegarsi si riconosce dal conto che resta zero.

/** Le regole di prova per raggiungere un server di prova: 127.0.0.1 e una porta qualunque; il resto (reti private, metadati) resta vietato. Per impostaReteDiProva. */
export const PROVE_LOCALI: SoloProve = {
  ammettiIndirizzo: (ip) => ip === "127.0.0.1" || indirizzoAmmesso(ip),
  ammettiPorta: () => true,
};

export type Gestore = (req: http.IncomingMessage, res: http.ServerResponse) => void;

export type ServerProva = {
  porta: number;
  url: (percorso: string) => string;
  /** I percorsi richiesti, nell'ordine. */
  richieste: string[];
  /** Le intestazioni dell'ultima richiesta. */
  ultime: http.IncomingHttpHeaders;
  chiudi: () => Promise<void>;
};

export async function avviaServerProva(rotte: Record<string, Gestore>): Promise<ServerProva> {
  const richieste: string[] = [];
  const stato = { ultime: {} as http.IncomingHttpHeaders };
  const sockets = new Set<import("node:net").Socket>();
  const server = http.createServer((req, res) => {
    richieste.push(req.url ?? "");
    stato.ultime = req.headers;
    const percorso = new URL(req.url ?? "/", "http://x").pathname;
    const gestore = rotte[percorso];
    if (!gestore) return void res.writeHead(404, { "Content-Type": "text/plain" }).end("no");
    gestore(req, res);
  });
  server.on("connection", (s) => {
    sockets.add(s);
    s.on("close", () => sockets.delete(s));
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  const porta = (server.address() as AddressInfo).port;
  return {
    porta,
    url: (p) => `http://127.0.0.1:${porta}${p}`,
    richieste,
    get ultime() {
      return stato.ultime;
    },
    chiudi: () =>
      new Promise<void>((ok) => {
        for (const s of sockets) s.destroy();
        server.close(() => ok());
      }),
  };
}

/** Una risposta fissa: stato, tipo e corpo. */
export const risposta =
  (corpo: Buffer | string, tipo = "image/png", stato = 200, extra: Record<string, string> = {}): Gestore =>
  (_, res) =>
    void res.writeHead(stato, { "Content-Type": tipo, "Content-Length": String(Buffer.byteLength(corpo)), ...extra }).end(corpo);

/** Un reindirizzamento. */
export const rimanda =
  (dove: string, stato = 302): Gestore =>
  (_, res) =>
    void res.writeHead(stato, { Location: dove, "Content-Length": "0" }).end();
