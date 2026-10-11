import { headers } from "next/headers";
import { notFound, redirect } from "next/navigation";
import { SicurezzaForm } from "@/components/SicurezzaForm";
import { auth } from "@/lib/auth";
import { config } from "@/lib/config";
import { loginUrl } from "@/lib/safe-next";

export const dynamic = "force-dynamic";

// Il secondo fattore dell'accesso: attivare e togliere il TOTP, i codici di recupero, le passkey. Con AUTH_2FA=off la pagina non c'è.
export default async function Sicurezza() {
  const modo = config.auth2fa;
  if (modo === "off") notFound();
  const sessione = await auth().api.getSession({ headers: await headers() });
  if (!sessione) redirect(loginUrl("/sicurezza"));
  return <SicurezzaForm modo={modo} email={sessione.user.email} attivo={Boolean((sessione.user as { twoFactorEnabled?: boolean | null }).twoFactorEnabled)} />;
}
