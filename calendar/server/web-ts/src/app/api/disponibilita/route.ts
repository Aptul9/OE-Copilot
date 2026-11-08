import { apiTs } from "@/lib/core/api";
import { fuoriFinestra, leggiDisponibilita, leggiRichiesta, oggiARoma } from "@/lib/disponibilita";

// Pubblica: niente sessione, e quindi niente dettagli. Gli impegni escono come "Occupato", con i soli orari,
// nella forma di get_events (events, errors) così la pagina li legge come sempre.
// Parametri: da e a (2026-10-12T00:00:00+02:00 o solo la data; `a` escluso, al più 62 giorni), gruppo (facoltativo).
export const dynamic = "force-dynamic";

const risposta = (stato: number, corpo: unknown, extra: Record<string, string> = {}) =>
  Response.json(corpo, { status: stato, headers: { "Cache-Control": "no-store", "X-Robots-Tag": "noindex, nofollow", ...extra } });

export async function GET(req: Request) {
  const r = leggiRichiesta(new URL(req.url).searchParams);
  if ("errore" in r) return risposta(400, { error: r.errore });
  if (fuoriFinestra(r, oggiARoma())) return risposta(200, { events: [], errors: [] });
  const e = await leggiDisponibilita(r, { core: (da, giorni, gruppo) => apiTs.get_disponibilita(da, giorni, gruppo) });
  if (e.ok) return risposta(200, { events: e.events, errors: e.errors });
  if (e.stato === 404) return risposta(404, { error: "Non c'è nessun calendario con questo nome." });
  if (e.stato === 429) return risposta(429, { error: "Troppe richieste in poco tempo: riprova fra un minuto." }, { "Retry-After": "30" });
  return risposta(502, { error: "La disponibilità non si legge in questo momento: riprova fra poco." });
}
