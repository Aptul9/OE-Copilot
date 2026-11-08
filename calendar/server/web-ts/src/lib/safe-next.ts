/**
 * Dove andare dopo l'accesso, restando su questo sito.
 *
 * `?next=` è quello che c'era nella barra degli indirizzi. Non basta controllare che inizi con
 * `/`: i browser leggono `/\evil.com` come `//evil.com` e scartano tab, CR e LF, quindi un link
 * a `/login?next=/%5Cevil.com` raccoglierebbe le credenziali e poi porterebbe su un altro sito.
 * La destinazione si risolve contro un'origine finta e deve restare lì.
 */
export function safeNext(next: string | null | undefined): string {
  if (!next || !next.startsWith("/") || next.startsWith("//") || /[\u0000-\u001f\\]/.test(next)) return "/";
  try {
    const url = new URL(next, "http://gateway.invalid");
    if (url.origin !== "http://gateway.invalid") return "/";
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return "/";
  }
}

/** La pagina di accesso, che poi riporta su `next`. */
export const loginUrl = (next: string) => `/login?next=${encodeURIComponent(next)}`;
