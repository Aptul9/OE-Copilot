import type { Metodi, NomeMetodo } from "./types";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

type Fetch = typeof fetch;

/**
 * Il client dell'API del gateway: ogni metodo (lib/core/api.ts) è un POST /api/<metodo> con gli argomenti in
 * una lista JSON. Una risposta 401 vuol dire che la sessione è finita: si torna all'accesso.
 */
export function creaApi(fetchImpl: Fetch = (...a) => fetch(...a), aSessioneFinita: () => void = vaiAllAccesso) {
  return async function api<M extends NomeMetodo>(metodo: M, ...args: Metodi[M][0]): Promise<Metodi[M][1]> {
    const r = await fetchImpl(`/api/${metodo}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(args),
    });
    const dati = await r.json().catch(() => ({}));
    if (r.status === 401) {
      aSessioneFinita();
      throw new ApiError("Sessione scaduta", 401);
    }
    if (!r.ok) throw new ApiError((dati && dati.error) || `HTTP ${r.status}`, r.status);
    return dati as Metodi[M][1];
  };
}

function vaiAllAccesso() {
  if (typeof window !== "undefined") window.location.href = `/login?next=${encodeURIComponent(window.location.pathname)}`;
}

export const api = creaApi();
