import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { handle } from "@/lib/gateway";
import { currentUser } from "@/lib/session";

// Tutto ciò che non è una pagina, /api/auth, /api/disponibilita, /api/prenota, /api/verifica-sessione o /salute, dopo il controllo della sessione: i
// metodi dell'API (POST /api/<metodo>, lib/core) e i loghi (GET /logos/<file>). Il resto è 404.
export const dynamic = "force-dynamic";

const metodi = { ha: (nome: string) => METODI_TS.has(nome), chiama: (nome: string, args: unknown[]) => chiama(nome as NomeMetodoTs, args) };

const run = (req: Request) => handle(req, { user: currentUser, metodi });

export { run as GET, run as HEAD, run as POST, run as PUT, run as PATCH, run as DELETE, run as OPTIONS };
