/** Chiude la sessione sul server e porta alla pagina di accesso; se il server non risponde si va lo stesso all'accesso. */
export function esci(): void {
  fetch("/api/auth/sign-out", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })
    .catch(() => {})
    .finally(() => {
      window.location.href = "/login";
    });
}
