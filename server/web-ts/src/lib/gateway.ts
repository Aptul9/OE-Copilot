import { loginUrl } from "./safe-next";

// Il corpo massimo che il servizio ui accetta (server.py, MAX_CORPO)
export const MAX_BODY = 1_000_000;

// Intestazioni che valgono per un solo salto: non si inoltrano. Il cookie di sessione non serve
// al servizio ui e non deve arrivarci.
const HOP = new Set([
  "connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer",
  "transfer-encoding", "upgrade", "host", "content-length", "cookie",
]);

export type Deps = {
  /** L'utente della sessione, o null. */
  user: (headers: Headers) => Promise<unknown>;
  upstream: string;
  fetch?: typeof fetch;
  /** I metodi di app.Api che il gateway serve da sé (lib/core): il nome e la risposta; gli altri passano al servizio ui. */
  metodi?: { ha: (nome: string) => boolean; chiama: (nome: string, args: unknown[]) => Promise<unknown> };
};

const json = (status: number, body: unknown) =>
  Response.json(body, { status, headers: { "Cache-Control": "no-store" } });

/** La risposta di un metodo servito qui: come quella del servizio ui (JSON, senza copia in cache, senza indovinare il tipo). */
const risposta = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" },
  });

/** Il nome del metodo di POST /api/<metodo>, o null per qualunque altro percorso. */
const nomeMetodo = (percorso: string) => /^\/api\/([A-Za-z_][A-Za-z0-9_]*)$/.exec(percorso)?.[1] ?? null;

/**
 * Il cancello davanti al servizio ui. Senza sessione una pagina rimanda a /login?next=...
 * e l'API risponde 401 JSON; con la sessione la richiesta passa, con le regole contro le
 * richieste partite da un altro sito che ha già il servizio ui (server.py): JSON obbligatorio
 * per i POST e Sec-Fetch-Site dello stesso sito.
 */
export async function handle(req: Request, deps: Deps): Promise<Response> {
  const url = new URL(req.url);
  if (!(await deps.user(req.headers))) {
    if (url.pathname.startsWith("/api/")) return json(401, { error: "non autenticato" });
    return new Response(null, { status: 302, headers: { Location: loginUrl(url.pathname + url.search), "Cache-Control": "no-store" } });
  }
  if (!["GET", "HEAD", "POST"].includes(req.method)) return json(405, { error: "metodo non ammesso" });

  let body: Buffer | undefined;
  if (req.method === "POST") {
    if (Number(req.headers.get("content-length") || 0) > MAX_BODY) return json(413, { error: "richiesta troppo grande" });
    if (!/^application\/json\s*(;|$)/i.test(req.headers.get("content-type") || "")) return json(415, { error: "serve application/json" });
    if (!["same-origin", "none"].includes(req.headers.get("sec-fetch-site") || "same-origin")) {
      return json(403, { error: "richiesta partita da un altro sito" });
    }
    body = Buffer.from(await req.arrayBuffer());
    if (body.length > MAX_BODY) return json(413, { error: "richiesta troppo grande" });
    // un metodo che il gateway serve da sé: gli argomenti sono una lista JSON, come per il servizio ui
    const metodo = nomeMetodo(url.pathname);
    if (metodo && deps.metodi?.ha(metodo)) {
      let args: unknown;
      try {
        args = JSON.parse(body.toString("utf-8") || "[]");
        if (!Array.isArray(args)) throw new Error("gli argomenti vanno in una lista");
      } catch (e) {
        return risposta(400, { error: e instanceof Error ? e.message : String(e) });
      }
      try {
        return risposta(200, await deps.metodi.chiama(metodo, args));
      } catch (e) {
        console.error(`api.${metodo}`, e);
        return risposta(500, { error: (e instanceof Error && e.message) || (e instanceof Error ? e.constructor.name : "errore") });
      }
    }
  }

  const headers = new Headers();
  req.headers.forEach((value, name) => {
    if (!HOP.has(name.toLowerCase())) headers.set(name, value);
  });
  // il servizio ui legge Content-Length: con un corpo a pezzi vedrebbe una richiesta vuota
  if (body) headers.set("content-length", String(body.length));

  let upstream: Response;
  try {
    upstream = await (deps.fetch ?? fetch)(deps.upstream + url.pathname + url.search, {
      method: req.method,
      headers,
      body: body ? new Uint8Array(body) : undefined,
      redirect: "manual",
      cache: "no-store",
    });
  } catch {
    return json(502, { error: "servizio non raggiungibile" });
  }
  const out = new Headers();
  upstream.headers.forEach((value, name) => {
    if (!HOP.has(name.toLowerCase()) && name.toLowerCase() !== "set-cookie") out.set(name, value);
  });
  out.set("Cache-Control", "no-store");
  return new Response(req.method === "HEAD" ? null : upstream.body, { status: upstream.status, headers: out });
}
