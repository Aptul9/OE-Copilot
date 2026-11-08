import { currentUser } from "@/lib/session";
import { verificaSessione } from "@/lib/verifica-sessione";

// Interrogata da Caddy (forward_auth) prima di passare `/telefono/` allo schermo del telefono: 204 con la sessione, 401 senza.
export const dynamic = "force-dynamic";

export const GET = (req: Request) => verificaSessione(req, { user: currentUser });
