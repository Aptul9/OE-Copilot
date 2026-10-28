import type { Metadata } from "next";
import { AppCalendario } from "@/components/oe/AppCalendario";
import { GRUPPO_MAX } from "@/lib/disponibilita";

// Senza accesso: lo stesso calendario della pagina principale, con gli impegni ridotti a "Occupato"; un clic su uno spazio libero prenota un meeting.
// ?gruppo=nome mostra solo gli impegni dei calendari di quel gruppo.
export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Prenota un meeting", robots: { index: false, follow: false } };

export default async function Disponibilita({ searchParams }: { searchParams: Promise<{ gruppo?: string | string[] }> }) {
  const g = (await searchParams).gruppo;
  // eslint-disable-next-line no-control-regex
  const gruppo = typeof g === "string" && g.length <= GRUPPO_MAX && !/[\u0000-\u001f\u007f]/.test(g) ? g.trim() : "";
  return <AppCalendario pubblico={{ gruppo }} />;
}
