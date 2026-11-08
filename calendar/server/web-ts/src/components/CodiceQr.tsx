import { useMemo } from "react";
import { qr } from "@/lib/qr";

/** Il QR di un testo come SVG in pagina, nero su bianco qualunque sia il tema. */
export function CodiceQr({ testo, titolo }: { testo: string; titolo: string }) {
  const { size, tracciato } = useMemo(() => qr(testo), [testo]);
  return (
    <svg role="img" aria-label={titolo} viewBox={`0 0 ${size} ${size}`} shapeRendering="crispEdges" className="size-56 rounded-md border bg-white" data-testid="codice-qr">
      <path d={tracciato} fill="#000" />
    </svg>
  );
}
