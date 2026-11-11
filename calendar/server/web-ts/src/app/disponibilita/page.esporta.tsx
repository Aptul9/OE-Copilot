import type { Metadata } from "next";
import { DisponibilitaEsportata } from "@/components/PagineEsportate";

export const metadata: Metadata = { title: "Prenota un meeting", robots: { index: false, follow: false } };

export default function Disponibilita() {
  return <DisponibilitaEsportata />;
}
