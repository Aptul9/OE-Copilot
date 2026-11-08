import { auth } from "@/lib/auth";
import { config } from "@/lib/config";
import { percorsoAmmesso } from "@/lib/secondo-fattore";

export const dynamic = "force-dynamic";

// Il secondo fattore (AUTH_2FA) apre le strade /two-factor e /passkey solo quando è acceso; con off rispondono 404
function gestisci(req: Request) {
  const percorso = new URL(req.url).pathname.replace(/^\/api\/auth/, "");
  if (!percorsoAmmesso(config.auth2fa, req.method, percorso)) {
    return Response.json({ error: "secondo fattore non attivo" }, { status: 404, headers: { "Cache-Control": "no-store" } });
  }
  return auth().handler(req);
}

export function GET(req: Request) {
  return gestisci(req);
}

export function POST(req: Request) {
  return gestisci(req);
}
