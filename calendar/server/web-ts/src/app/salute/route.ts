// Controllo di Docker: il gateway risponde. Non passa dal controllo della sessione.
export const dynamic = "force-dynamic";

export function GET() {
  return new Response("ok", { headers: { "Content-Type": "text/plain", "Cache-Control": "no-store" } });
}
