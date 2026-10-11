import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { AppCalendario } from "@/components/oe/AppCalendario";
import { loginUrl } from "@/lib/safe-next";
import { currentUser } from "@/lib/session";

export const dynamic = "force-dynamic";

// La nuova interfaccia (TypeScript, Next.js): sta qui finché non pareggia quella di web/, poi prenderà il posto di /.
export default async function Nuova() {
  if (!(await currentUser(await headers()))) redirect(loginUrl("/nuova"));
  return <AppCalendario />;
}
