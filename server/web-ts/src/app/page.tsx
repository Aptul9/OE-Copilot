import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { AppCalendario } from "@/components/oe/AppCalendario";
import { loginUrl } from "@/lib/safe-next";
import { currentUser } from "@/lib/session";

export const dynamic = "force-dynamic";

// L'interfaccia in TypeScript.
export default async function Home() {
  if (!(await currentUser(await headers()))) redirect(loginUrl("/"));
  return <AppCalendario />;
}
