/** Chiude la sessione locale e porta alla pagina di accesso. */
export function esci(): void {
  fetch("/api/auth/sign-out", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })
    .catch(() => {})
    .finally(() => {
      window.location.href = "/login";
    });
}
