import zlib from "node:zlib";

// Immagini inventate per le prove dei loghi, fatte qui: nel repository non c'è nessun file di immagine (e il logo di nessuna azienda).

const chunk = (tipo: string, corpo: Buffer) => {
  const t = Buffer.from(tipo, "ascii");
  const lunghezza = Buffer.alloc(4);
  lunghezza.writeUInt32BE(corpo.length);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(Buffer.concat([t, corpo])));
  return Buffer.concat([lunghezza, t, corpo, crc]);
};

/** Un PNG vero, quadrato e di un colore solo. */
export function png(lato = 8, [r, g, b]: [number, number, number] = [200, 30, 30]): Buffer {
  const riga = Buffer.alloc(1 + lato * 3);
  for (let x = 0; x < lato; x++) riga.set([r, g, b], 1 + x * 3);
  const intestazione = Buffer.alloc(13);
  intestazione.writeUInt32BE(lato, 0);
  intestazione.writeUInt32BE(lato, 4);
  intestazione.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", intestazione),
    chunk("IDAT", zlib.deflateSync(Buffer.concat(Array.from({ length: lato }, () => riga)))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** Le teste di JPEG, GIF, WebP e ICO: bastano ai primi byte, che sono quello che si controlla. */
export const jpeg = () => Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]), Buffer.from("JFIF\0"), Buffer.alloc(20)]);
export const gif = (versione: "87a" | "89a" = "89a") => Buffer.concat([Buffer.from(`GIF${versione}`, "ascii"), Buffer.alloc(20)]);
export const webp = () => Buffer.concat([Buffer.from("RIFF", "ascii"), Buffer.from([0x20, 0, 0, 0]), Buffer.from("WEBPVP8 ", "ascii"), Buffer.alloc(20)]);
export const ico = (immagini = 1) => Buffer.concat([Buffer.from([0, 0, 1, 0, immagini, 0]), Buffer.alloc(16 * immagini + 8)]);

/** Uno SVG semplice, come lo esporta un programma di disegno: gradiente, riferimenti #interni, stile, titolo. */
export const SVG_PULITO = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 64 64" width="64" height="64">
  <title>Alfa</title>
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2563eb"/><stop offset="1" stop-color="#7c3aed"/></linearGradient>
    <style>.a{fill:url(#g)} .b{fill:#fff;font-family:sans-serif}</style>
    <circle id="c" cx="32" cy="32" r="30"/>
  </defs>
  <use xlink:href="#c" class="a"/>
  <use href="#c" class="a" style="fill:url(#g);opacity:.5"/>
  <text x="32" y="40" text-anchor="middle" class="b" font-size="24">A</text>
</svg>
`;
