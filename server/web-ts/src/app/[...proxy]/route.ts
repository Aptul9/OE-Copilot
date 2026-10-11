import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { config } from "@/lib/config";
import { handle } from "@/lib/gateway";
import { currentUser } from "@/lib/session";

// Tutto ciò che non è /, /login, /api/auth o /salute: la pagina e l'API, dopo il controllo della sessione. I metodi di app.Api già
// scritti in TypeScript (lib/core) si servono da qui; gli altri passano ancora al servizio ui (Python).
export const dynamic = "force-dynamic";

const metodi = { ha: (nome: string) => METODI_TS.has(nome), chiama: (nome: string, args: unknown[]) => chiama(nome as NomeMetodoTs, args) };

const run = (req: Request) => handle(req, { user: currentUser, upstream: config.upstream, metodi });

export { run as GET, run as HEAD, run as POST, run as PUT, run as PATCH, run as DELETE, run as OPTIONS };
