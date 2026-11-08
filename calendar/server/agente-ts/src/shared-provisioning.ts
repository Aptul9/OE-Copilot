// Attivazione esplicita di un account Microsoft condiviso (shared-account.ts): la casella, già collegata con il suo accesso, diventa la
// fonte unica di posta, calendario e chat Teams per le capacità scelte. Tenant e object id vengono dai claims di un token appena rinnovato,
// mai da quello che manda la pagina; l'origine dal client con cui la casella ha fatto l'accesso. Dopo il salvataggio ogni capacità scelta
// si prova con un token vero: se una non passa, la casella torna com'era.
//   GET  /api/auth/shared   {"accounts": [{"account", "shared": {"capabilities", "origin"} | null, "status", "message", "teams"}],
//                            "teamsOnly": [{"email", "slot", "state", "detail"}], "teamsAt"}
//   POST /api/auth/shared   {"account", "capabilities": ["mail", "calendar", "chat"]} attiva o cambia (anche nessuna: la casella resta
//                            collegata e nessuno la usa); {"account", "enabled": false} spegne
// Una casella non condivisa vale posta e calendario, senza chat: è l'accesso di sempre.
import { claimsJwt, upnDi } from "./outlook-rest.ts";
import { OUTLOOK_WEB_CLIENT_ID } from "./sessione-web.ts";
import type { SharedAccount } from "./shared-account.ts";
import type { RapportoTeams } from "./shared-teams.ts";

export type Capability = SharedAccount["capabilities"][number];
export const CAPABILITIES: readonly Capability[] = ["mail", "calendar", "chat"];

/** Le risorse che una capacità deve poter ottenere: la chat sono i quattro servizi che l'agente Teams senza browser usa. */
const PROVE: Record<Capability, string[]> = { mail: ["outlook"], calendar: ["outlook"], chat: ["csa", "ic3", "spaces", "presence"] };

const TEAMS_WEB_CLIENT_ID = "5e3ce6c0-2b1f-4285-8d4b-75ee78787346";
const GUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Quello che serve del GestoreToken. `impostaShared` scrive (o toglie, con undefined) i metadati condivisi e svuota i token per risorsa. */
export interface ArchivioCondiviso {
  voceAccount(email: string): { provider: "microsoft" | "google"; client_id?: string; refresh_token?: string; errore?: string; shared?: SharedAccount } | undefined;
  elencoAccount(): Record<string, { provider: "microsoft" | "google"; errore?: string; shared?: SharedAccount }>;
  ottieniAccessToken(email: string, forza?: boolean): Promise<string>;
  ottieniRisorsa(email: string, resource: string, force?: boolean): Promise<string>;
  impostaShared(email: string, shared: SharedAccount | undefined): void;
}

export class ErroreCondivisione extends Error {}

/** Le capacità chieste: un elenco di quelle note (anche vuoto), senza doppioni, nell'ordine fisso. */
export function capacitaValide(valore: unknown): Capability[] {
  if (!Array.isArray(valore) || !valore.every((c) => typeof c === "string" && (CAPABILITIES as readonly string[]).includes(c))) {
    throw new ErroreCondivisione("capabilities: un elenco tra mail, calendar, chat");
  }
  return CAPABILITIES.filter((c) => valore.includes(c));
}

/** L'origine del refresh_token della casella, dal client con cui ha fatto l'accesso: decide come Microsoft lo rinnova. */
export function origineDi(clientId: string | undefined): SharedAccount["origin"] {
  if (clientId === OUTLOOK_WEB_CLIENT_ID) return "outlook-spa";
  if (clientId === TEAMS_WEB_CLIENT_ID) return "teams-spa";
  return "mobile";
}

/** Tenant e object id dai claims del token della casella: devono essere GUID e il token deve essere proprio di quella casella. */
export function identitaDaToken(token: string, casella: string): { tenantId: string; objectId: string } {
  const claims = claimsJwt(token);
  const tid = typeof claims.tid === "string" ? claims.tid : "";
  const oid = typeof claims.oid === "string" ? claims.oid : "";
  if (!GUID.test(tid) || !GUID.test(oid)) throw new ErroreCondivisione("il token della casella non dice tenant e object id");
  if (upnDi(claims) !== casella) throw new ErroreCondivisione("il token appartiene a un'altra casella");
  return { tenantId: tid, objectId: oid };
}

const pubblico = (s: SharedAccount | undefined) => (s ? { capabilities: [...s.capabilities], origin: s.origin } : null);

/**
 * L'elenco unico degli account: ogni casella Microsoft con le sue capacità, lo stato dell'accesso e, se TeamsRelay ha mandato un rapporto
 * recente, l'account Teams che la usa; poi gli account Teams con il solo accesso del browser, che il calendario non ha.
 */
export function statoCondivisione(archivio: ArchivioCondiviso, teams?: RapportoTeams) {
  const accounts = Object.entries(archivio.elencoAccount())
    .filter(([, v]) => v.provider === "microsoft")
    .map(([account, v]) => ({
      account,
      shared: pubblico(v.shared),
      status: v.errore ? "errore" : "ok",
      message: v.errore ?? "",
      teams: teams?.accounts.get(account) ?? null,
    }));
  return { accounts, teamsOnly: teams?.others ?? [], teamsAt: teams?.at ?? 0 };
}

/**
 * Attiva o cambia le capacità di una casella Microsoft già collegata. Una casella già condivisa tiene la sua identità; una nuova la prende
 * da un token rinnovato ora. Ogni risorsa delle capacità scelte si chiede a Microsoft con il nuovo stato: un rifiuto rimette lo stato di
 * prima e l'errore dice quale capacità non passa.
 */
export async function attivaCondivisione(archivio: ArchivioCondiviso, account: string, capabilities: unknown): Promise<{ account: string; shared: ReturnType<typeof pubblico> }> {
  const casella = account.trim().toLowerCase();
  const caps = capacitaValide(capabilities);
  const voce = archivio.voceAccount(casella);
  if (!voce) throw new ErroreCondivisione("Casella non trovata");
  if (voce.provider !== "microsoft") throw new ErroreCondivisione("Solo una casella Microsoft può essere condivisa");
  const prima = voce.shared;
  // togliere soltanto non chiede niente a Microsoft: si spegne Teams (o posta, o calendario) anche con l'accesso scaduto
  if (prima && caps.every((c) => prima.capabilities.includes(c))) {
    const meno: SharedAccount = { ...prima, capabilities: caps };
    archivio.impostaShared(casella, meno);
    return { account: casella, shared: pubblico(meno) };
  }
  if (!voce.refresh_token) throw new ErroreCondivisione("La casella non ha un accesso rinnovabile: accedi di nuovo prima di condividerla");
  if (voce.errore) throw new ErroreCondivisione(`Accesso da rifare prima di condividere: ${voce.errore}`);
  const identita = prima ? { tenantId: prima.tenantId, objectId: prima.objectId } : identitaDaToken(await archivio.ottieniAccessToken(casella, true), casella);
  const nuovo: SharedAccount = { ...identita, capabilities: caps, origin: prima?.origin ?? origineDi(voce.client_id) };
  archivio.impostaShared(casella, nuovo);
  // una risorsa sola anche quando due capacità la usano (posta e calendario: outlook)
  for (const risorsa of new Set(caps.flatMap((c) => PROVE[c]))) {
    try {
      await archivio.ottieniRisorsa(casella, risorsa, true);
    } catch (e) {
      archivio.impostaShared(casella, prima);
      const per = caps.filter((c) => PROVE[c].includes(risorsa)).join(", ");
      throw new ErroreCondivisione(`Capacità ${per} non disponibile per questa casella (${risorsa}): ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  return { account: casella, shared: pubblico(nuovo) };
}

/** Spegne la condivisione: la casella torna all'accesso suo, posta e calendario come prima, nessun token per Teams. */
export function disattivaCondivisione(archivio: ArchivioCondiviso, account: string): { account: string; shared: null } {
  const casella = account.trim().toLowerCase();
  const voce = archivio.voceAccount(casella);
  if (!voce) throw new ErroreCondivisione("Casella non trovata");
  if (voce.shared) archivio.impostaShared(casella, undefined);
  return { account: casella, shared: null };
}

/** La richiesta HTTP di /api/auth/shared, già autorizzata: il corpo della risposta, sempre 200 con ok come le altre di /api/auth. */
export async function richiestaCondivisione(archivio: ArchivioCondiviso | undefined, metodo: "GET" | "POST", corpo: unknown, teams?: RapportoTeams): Promise<Record<string, unknown>> {
  if (!archivio) return { ok: false, errore: "gestore token non configurato" };
  if (metodo === "GET") return { ok: true, ...statoCondivisione(archivio, teams) };
  try {
    const r = corpo !== null && typeof corpo === "object" && !Array.isArray(corpo) ? (corpo as Record<string, unknown>) : {};
    if (typeof r.account !== "string" || !r.account.trim()) throw new ErroreCondivisione("account mancante");
    if (r.enabled === false) return { ok: true, ...disattivaCondivisione(archivio, r.account) };
    return { ok: true, ...(await attivaCondivisione(archivio, r.account, r.capabilities)) };
  } catch (e) {
    return { ok: false, errore: e instanceof Error ? e.message : String(e) };
  }
}
