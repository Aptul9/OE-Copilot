import type { Metadata } from "next";
import { RemoteExported } from "@/components/ExportedPages";

export const metadata: Metadata = { title: "Remote Teams" };

export default function Remote() {
  return <RemoteExported />;
}
