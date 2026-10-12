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

/** Solo la testata di un PNG, con la grandezza che si vuole (anche 2147483647 per 2147483647): è quello che si controlla. */
export function pngIntestazione(larghezza: number, altezza: number): Buffer {
  const intestazione = Buffer.alloc(13);
  intestazione.writeUInt32BE(larghezza, 0);
  intestazione.writeUInt32BE(altezza, 4);
  intestazione.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", intestazione), chunk("IEND", Buffer.alloc(0))]);
}

/** Le teste di JPEG, GIF, WebP e ICO: i primi byte e la grandezza, che sono quello che si controlla. */
export const jpeg = (larghezza = 8, altezza = 8) =>
  Buffer.concat([
    // JFIF (APP0) e il frame SOF0: precisione, altezza, larghezza, un componente
    Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]),
    Buffer.from("JFIF\0"),
    Buffer.alloc(9),
    Buffer.from([0xff, 0xc0, 0x00, 0x0b, 0x08, altezza >> 8, altezza & 0xff, larghezza >> 8, larghezza & 0xff, 0x01, 0x01, 0x11, 0x00]),
    Buffer.alloc(8),
  ]);

const le16 = (n: number) => {
  const b = Buffer.alloc(2);
  b.writeUInt16LE(n);
  return b;
};

export type FotogrammaGif = {
  sinistra?: number;
  sopra?: number;
  larghezza: number;
  altezza: number;
  /** Le voci della tabella dei colori locale (2, 4, 8, ..., 256), se ce l'ha. */
  tabella?: number;
  /** I sotto-blocchi dei dati, uno per elemento (non sono LZW da decodificare: il controllo guarda i blocchi). */
  dati?: Buffer[];
};

/**
 * Un GIF con la struttura vera: testata, schermo, tabella globale, i fotogrammi (ognuno con il suo blocco di controllo grafico e,
 * se la chiede, la tabella locale) e il trailer. `estensioni` mette in testa il ciclo di Netscape e un commento.
 */
export function gifConFotogrammi(schermo: [number, number], fotogrammi: FotogrammaGif[], versione: "87a" | "89a" = "89a", estensioni = false): Buffer {
  const parti: Buffer[] = [Buffer.from(`GIF${versione}`, "ascii"), le16(schermo[0]), le16(schermo[1]), Buffer.from([0x81, 0, 0]), Buffer.alloc(12)];
  if (estensioni) {
    parti.push(Buffer.from([0x21, 0xff, 0x0b]), Buffer.from("NETSCAPE2.0", "ascii"), Buffer.from([0x03, 0x01, 0x00, 0x00, 0x00]));
    parti.push(Buffer.from([0x21, 0xfe, 0x05]), Buffer.from("ciao!", "ascii"), Buffer.from([0x00]));
  }
  for (const f of fotogrammi) {
    // controllo grafico (ritardo 10 centesimi), descrittore, tabella locale, lunghezza minima dei codici, dati
    parti.push(Buffer.from([0x21, 0xf9, 0x04, 0x00, 0x0a, 0x00, 0x00, 0x00]));
    const voci = f.tabella ?? 0;
    const potenza = voci ? Math.log2(voci) - 1 : 0;
    parti.push(Buffer.from([0x2c]), le16(f.sinistra ?? 0), le16(f.sopra ?? 0), le16(f.larghezza), le16(f.altezza), Buffer.from([voci ? 0x80 | potenza : 0]));
    if (voci) parti.push(Buffer.alloc(3 * voci, 7));
    parti.push(Buffer.from([0x02]));
    for (const blocco of f.dati ?? [Buffer.from([0x44, 0x01])]) parti.push(Buffer.from([blocco.length]), blocco);
    parti.push(Buffer.from([0x00]));
  }
  parti.push(Buffer.from([0x3b]));
  return Buffer.concat(parti);
}

/** Un GIF a un fotogramma, con lo schermo e il fotogramma della grandezza data. */
export const gif = (versione: "87a" | "89a" = "89a", larghezza = 8, altezza = 8) => gifConFotogrammi([larghezza, altezza], [{ larghezza, altezza }], versione);

const riff = (blocco: Buffer) => {
  const testa = Buffer.alloc(12);
  testa.write("RIFF", 0, "ascii");
  testa.writeUInt32LE(4 + blocco.length, 4);
  testa.write("WEBP", 8, "ascii");
  return Buffer.concat([testa, blocco]);
};
/** WebP con perdita (VP8): tag del frame, firma 9d 01 2a, larghezza e altezza a 14 bit. */
export const webp = (larghezza = 8, altezza = 8) => {
  const b = Buffer.alloc(18);
  b.write("VP8 ", 0, "ascii");
  b.writeUInt32LE(10, 4);
  b.set([0x9d, 0x01, 0x2a], 11);
  b.writeUInt16LE(larghezza, 14);
  b.writeUInt16LE(altezza, 16);
  return riff(b);
};
/** WebP esteso (VP8X): la tela è larghezza e altezza meno uno, a 24 bit. */
export const webpEsteso = (larghezza = 8, altezza = 8) => {
  const b = Buffer.alloc(18);
  b.write("VP8X", 0, "ascii");
  b.writeUInt32LE(10, 4);
  b.writeUIntLE(larghezza - 1, 12, 3);
  b.writeUIntLE(altezza - 1, 15, 3);
  return riff(b);
};
/** WebP senza perdita (VP8L): firma 2f, poi larghezza e altezza meno uno a 14 bit. */
export const webpSenzaPerdita = (larghezza = 8, altezza = 8) => {
  const b = Buffer.alloc(13);
  b.write("VP8L", 0, "ascii");
  b.writeUInt32LE(5, 4);
  b[8] = 0x2f;
  b.writeUInt32LE(((larghezza - 1) | ((altezza - 1) << 14)) >>> 0, 9);
  return riff(b);
};

/** ICO: la tabella e, per ogni voce, un PNG vero dentro (come i file veri). */
export function ico(immagini = 1, lato = 8): Buffer {
  const dentro = png(lato);
  const testa = Buffer.alloc(6 + 16 * immagini);
  testa.set([0, 0, 1, 0], 0);
  testa.writeUInt16LE(immagini, 4);
  for (let k = 0; k < immagini; k++) {
    const voce = 6 + 16 * k;
    testa[voce] = lato;
    testa[voce + 1] = lato;
    testa.writeUInt16LE(1, voce + 4);
    testa.writeUInt16LE(32, voce + 6);
    testa.writeUInt32LE(dentro.length, voce + 8);
    testa.writeUInt32LE(testa.length, voce + 12);
  }
  return Buffer.concat([testa, dentro]);
}

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
