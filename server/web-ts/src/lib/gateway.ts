import { readFile, realpath, stat } from "node:fs/promises";
import path from "node:path";
import { config } from "./config";
import { loginUrl } from "./safe-next";

// Il corpo massimo di una richiesta POST: lo stesso che aveva il servizio Python ui (server.py, MAX_CORPO)
export const MAX_BODY = 1_000_000;

export type Deps = {
  /** L'utente della sessione, o null. */
  user: (headers: Headers) => Promise<unknown>;
  /** I metodi dell'API che il gateway serve (lib/core): il nome e la risposta. */
  metodi: { ha: (nome: string) => boolean; chiama: (nome: string, args: unknown[]) => Promise<unknown> };
  /** Le cartelle dei loghi, in ordine: vince la prima che ha il file. Senza, quella dei dati (logos/) e poi quella dell'immagine. */
  loghi?: string[];
};

const json = (status: number, body: unknown) =>
  Response.json(body, { status, headers: { "Cache-Control": "no-store" } });

/** La risposta di un metodo servito qui: JSON, senza copia in cache, senza indovinare il tipo. */
const risposta = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" },
  });

/**
 * Un logo aperto da solo in una scheda è un documento di questo sito: un SVG potrebbe eseguire script con i suoi permessi. Con questa
 * politica (nessuna risorsa, nessuno script, origine isolata) non esegue e non carica niente; dentro un <img> non cambia nulla.
 */
const POLITICA_LOGHI = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/** Il nome del metodo di POST /api/<metodo>, o null per qualunque altro percorso. */
const nomeMetodo = (percorso: string) => /^\/api\/([A-Za-z_][A-Za-z0-9_]*)$/.exec(percorso)?.[1] ?? null;

/** Il nome del file di /logos/<nome>, decodificato una volta sola come faceva il servizio ui, o null per qualunque altro percorso. */
function nomeLogo(percorso: string): string | null {
  let p: string;
  try {
    p = decodeURIComponent(percorso);
  } catch {
    return null;
  }
  return p.startsWith("/logos/") ? p.slice("/logos/".length) : null;
}

const TIPI: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".svg": "image/svg+xml",
  ".ico": "image/vnd.microsoft.icon",
};

/**
 * Il logo `nome` della prima cartella che lo ha. Si risolvono i percorsi veri, collegamenti compresi, e il file deve restare
 * dentro la cartella: un nome con .., un nome assoluto o un collegamento che esce non passano. Solo file normali.
 */
async function leggiLogo(nome: string, cartelle: string[]): Promise<{ dati: Uint8Array<ArrayBuffer>; tipo: string } | null> {
  for (const cartella of cartelle) {
    try {
      const radice = await realpath(cartella);
      // un nome assoluto prende il posto della cartella, come in os.path.join. Niente path.resolve su un nome qualunque: Next,
      // nel tracciare i file da portare nell'immagine, ne copierebbe tutta la cartella del modulo (src/lib)
      const file = await realpath(path.isAbsolute(nome) ? nome : radice + path.sep + nome);
      const rel = path.relative(radice, file);
      if (!rel || rel === ".." || rel.startsWith(".." + path.sep) || path.isAbsolute(rel)) continue;
      if (!(await stat(file)).isFile()) continue;
      return { dati: new Uint8Array(await readFile(file)), tipo: TIPI[path.extname(file).toLowerCase()] ?? "application/octet-stream" };
    } catch {
      // la cartella o il file non ci sono, o il nome non è valido: si passa alla cartella dopo
    }
  }
  return null;
}

/**
 * Il cancello davanti ai metodi dell'API e ai loghi. Senza sessione una pagina rimanda a /login?next=...
 * e l'API risponde 401 JSON; con la sessione la richiesta passa, con le regole contro le richieste
 * partite da un altro sito che aveva già il servizio ui (server.py): JSON obbligatorio per i POST e
 * Sec-Fetch-Site dello stesso sito. Quello che non è un metodo servito né un logo è 404.
 */
export async function handle(req: Request, deps: Deps): Promise<Response> {
  const url = new URL(req.url);
  if (!(await deps.user(req.headers))) {
    if (url.pathname.startsWith("/api/")) return json(401, { error: "non autenticato" });
    return new Response(null, { status: 302, headers: { Location: loginUrl(url.pathname + url.search), "Cache-Control": "no-store" } });
  }
  if (!["GET", "HEAD", "POST"].includes(req.method)) return json(405, { error: "metodo non ammesso" });

  if (req.method === "POST") {
    if (Number(req.headers.get("content-length") || 0) > MAX_BODY) return json(413, { error: "richiesta troppo grande" });
    if (!/^application\/json\s*(;|$)/i.test(req.headers.get("content-type") || "")) return json(415, { error: "serve application/json" });
    if (!["same-origin", "none"].includes(req.headers.get("sec-fetch-site") || "same-origin")) {
      return json(403, { error: "richiesta partita da un altro sito" });
    }
    const body = Buffer.from(await req.arrayBuffer());
    if (body.length > MAX_BODY) return json(413, { error: "richiesta troppo grande" });
    const metodo = nomeMetodo(url.pathname);
    if (!metodo || !deps.metodi.ha(metodo)) return json(404, { error: "metodo non disponibile" });
    // gli argomenti sono una lista JSON
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

  // GET e HEAD: solo i loghi
  const nome = nomeLogo(url.pathname);
  const logo = nome === null ? null : await leggiLogo(nome, deps.loghi ?? [path.join(config.dati, "logos"), config.loghiPredefiniti]);
  if (!logo) return json(404, { error: "non trovato" });
  return new Response(req.method === "HEAD" ? null : logo.dati, {
    status: 200,
    headers: {
      "Content-Type": logo.tipo,
      "Content-Length": String(logo.dati.length),
      "Cache-Control": "no-cache",
      "X-Content-Type-Options": "nosniff",
      "Content-Security-Policy": POLITICA_LOGHI,
    },
  });
}
