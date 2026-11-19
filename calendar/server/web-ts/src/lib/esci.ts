/** Chiude la sessione sul server e porta alla pagina di accesso; se il server non risponde si va lo stesso all'accesso. Entrati da EverID,
 * il gateway mette in window.__OE__.esci l'uscita di EverID, che chiude anche quella. */
export function esci(): void {
  const dopo = (window as { __OE__?: { esci?: string } }).__OE__?.esci || "/login";
  fetch("/api/auth/sign-out", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })
    .catch(() => {})
    .finally(() => {
      window.location.href = dopo;
    });
}
