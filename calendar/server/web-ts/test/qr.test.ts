import jsQR from "jsqr";
import { describe, expect, it } from "vitest";
import { qr } from "@/lib/qr";

/** Disegna il tracciato come lo farebbe il browser (nero su bianco) e lo legge con un lettore di QR indipendente. */
function leggi(testo: string, scala = 6): string | null {
  const { size, tracciato } = qr(testo);
  const lato = size * scala;
  const pixel = new Uint8ClampedArray(lato * lato * 4).fill(255);
  for (const [, xs, ys, ns] of tracciato.matchAll(/M(\d+) (\d+)h(\d+)v1h-\d+z/g)) {
    const [x, y, n] = [Number(xs), Number(ys), Number(ns)];
    for (let py = y * scala; py < (y + 1) * scala; py++) {
      for (let px = x * scala; px < (x + n) * scala; px++) {
        const i = (py * lato + px) * 4;
        pixel[i] = pixel[i + 1] = pixel[i + 2] = 0;
      }
    }
  }
  return jsQR(pixel, lato, lato)?.data ?? null;
}

describe("il QR dell'attivazione", () => {
  it("un lettore indipendente ci legge l'URI otpauth tale e quale", () => {
    const uri =
      "otpauth://totp/OE%20Copilot:proprietario%40esempio.it?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZD&issuer=OE+Copilot&digits=6&period=30";
    expect(leggi(uri)).toBe(uri);
  });

  it("legge anche testi corti, lunghi e con accenti", () => {
    for (const testo of ["a", "https://oe.example/sicurezza", "è già così: perché", "x".repeat(300)]) expect(leggi(testo)).toBe(testo);
  });

  it("il tracciato ha il bordo bianco di quattro moduli e un lato fatto di moduli interi", () => {
    const { size, tracciato } = qr("prova");
    expect(size).toBeGreaterThanOrEqual(21 + 8);
    const rettangoli = [...tracciato.matchAll(/M(\d+) (\d+)h(\d+)v1h-\d+z/g)].map((m) => [Number(m[1]), Number(m[2]), Number(m[3])]);
    expect(rettangoli.length).toBeGreaterThan(20);
    for (const [x, y, n] of rettangoli) {
      expect(x).toBeGreaterThanOrEqual(4);
      expect(y).toBeGreaterThanOrEqual(4);
      expect(x + n).toBeLessThanOrEqual(size - 4);
      expect(y).toBeLessThan(size - 4);
    }
  });
});
