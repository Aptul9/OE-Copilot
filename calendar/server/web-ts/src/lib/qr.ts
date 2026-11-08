import { encode } from "uqr";

/**
 * Il QR di un testo: il lato in moduli, con la zona bianca di 4 moduli attorno che i lettori vogliono, e il tracciato SVG dei
 * moduli scuri (un rettangolo per ogni riga di moduli vicini: `M<x> <y>h<n>v1h-<n>z`).
 */
export function qr(testo: string): { size: number; tracciato: string } {
  const codice = encode(testo, { ecc: "M", border: 4 });
  let tracciato = "";
  codice.data.forEach((riga, y) => {
    for (let x = 0; x < codice.size; ) {
      if (!riga[x]) {
        x++;
        continue;
      }
      let n = 1;
      while (x + n < codice.size && riga[x + n]) n++;
      tracciato += `M${x} ${y}h${n}v1h-${n}z`;
      x += n;
    }
  });
  return { size: codice.size, tracciato };
}
