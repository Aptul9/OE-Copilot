import { loginUrl, safeNext } from "./safe-next";

export type Deps = {
  /** L'utente della sessione, o null (la stessa verifica sul database del cancello davanti a ui). */
  user: (headers: Headers) => Promise<unknown>;
};

/**
 * Risposta per il `forward_auth` di Caddy davanti allo schermo del telefono (`/telefono/`): 204 se la richiesta porta una
 * sessione valida, altrimenti la risposta che Caddy rimanda al browser com'è. Una pagina (navigazione, o Accept html)
 * va alla pagina di accesso e poi torna dove stava; il resto, WebSocket compreso, riceve 401 in JSON.
 * Caddy inoltra i cookie e l'indirizzo originale in `X-Forwarded-Uri`, che passa da `safeNext`.
 */
export async function verificaSessione(req: Request, deps: Deps): Promise<Response> {
  const intestazioni = { "Cache-Control": "no-store" };
  if (await deps.user(req.headers)) return new Response(null, { status: 204, headers: intestazioni });
  const aggiornamento = !!req.headers.get("upgrade");
  const pagina = !aggiornamento && (req.headers.get("sec-fetch-dest") === "document" || /text\/html/i.test(req.headers.get("accept") || ""));
  if (pagina) return new Response(null, { status: 302, headers: { ...intestazioni, Location: loginUrl(safeNext(req.headers.get("x-forwarded-uri"))) } });
  return Response.json({ error: "non autenticato" }, { status: 401, headers: intestazioni });
}
