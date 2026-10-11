import { config } from "@/lib/config";
import { handle } from "@/lib/gateway";
import { currentUser } from "@/lib/session";

// Tutto ciò che non è /login, /api/auth o /salute: la pagina e l'API del servizio ui, dopo il controllo della sessione.
export const dynamic = "force-dynamic";

const run = (req: Request) => handle(req, { user: currentUser, upstream: config.upstream });

export { run as GET, run as HEAD, run as POST, run as PUT, run as PATCH, run as DELETE, run as OPTIONS };
