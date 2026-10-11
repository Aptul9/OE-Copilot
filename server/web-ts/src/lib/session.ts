import { auth } from "./auth";

export type SessionUser = { id: string; email: string; name: string };

/** L'utente della sessione di una richiesta, verificata sul database (non basta la presenza del cookie). */
export async function currentUser(headers: Headers): Promise<SessionUser | null> {
  const s = await auth().api.getSession({ headers });
  return s?.user ?? null;
}
