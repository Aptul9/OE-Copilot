import { prenota } from "@/lib/prenota";

// Pubblica, come /api/disponibilita: la prenotazione di un meeting dalla pagina della disponibilità (lib/prenota.ts).
// Corpo JSON: inizio e fine (ora di Roma, 2026-10-30T10:00:00), nome, email, azienda, argomento, gruppo.
export const dynamic = "force-dynamic";

const CORPO_MAX = 8 * 1024;

const risposta = (stato: number, corpo: unknown, extra: Record<string, string> = {}) =>
  Response.json(corpo, { status: stato, headers: { "Cache-Control": "no-store", "X-Robots-Tag": "noindex, nofollow", ...extra } });

export async function POST(req: Request) {
  if (!(req.headers.get("content-type") ?? "").toLowerCase().startsWith("application/json")) return risposta(415, { error: "Serve un corpo JSON." });
  const grezzo = await req.text().catch(() => "");
  if (grezzo.length > CORPO_MAX) return risposta(413, { error: "Richiesta troppo grande." });
  let corpo: unknown;
  try {
    corpo = JSON.parse(grezzo);
  } catch {
    return risposta(400, { error: "Richiesta non valida." });
  }
  const e = await prenota(corpo);
  if (e.ok) return risposta(200, { ok: true, inizio: e.inizio, fine: e.fine });
  return risposta(e.stato, { error: e.errore }, e.stato === 429 ? { "Retry-After": "600" } : {});
}
