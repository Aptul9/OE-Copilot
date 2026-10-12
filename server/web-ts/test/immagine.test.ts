import { describe, expect, it } from "vitest";
import { controllaImmagine, LATO_MAX, PIXEL_MAX, svgAmmesso } from "@/lib/core/immagine";
import { type FotogrammaGif, gif, gifConFotogrammi, ico, jpeg, png, pngIntestazione, SVG_PULITO, webp, webpEsteso, webpSenzaPerdita } from "./immagini-prova";

const tipo = (dati: Uint8Array | string) => {
  const e = controllaImmagine(typeof dati === "string" ? Buffer.from(dati, "utf-8") : dati);
  return "tipo" in e ? e.tipo.ext : e.errore;
};

const SVG = (dentro: string) => `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10">${dentro}</svg>`;

describe("il tipo lo dicono i primi byte", () => {
  it("riconosce PNG, JPEG, GIF, WebP e ICO", () => {
    expect(controllaImmagine(png())).toEqual({ tipo: { ext: "png", mime: "image/png" } });
    expect(controllaImmagine(jpeg())).toEqual({ tipo: { ext: "jpg", mime: "image/jpeg" } });
    expect(controllaImmagine(gif("87a"))).toEqual({ tipo: { ext: "gif", mime: "image/gif" } });
    expect(controllaImmagine(gif("89a"))).toEqual({ tipo: { ext: "gif", mime: "image/gif" } });
    expect(controllaImmagine(webp())).toEqual({ tipo: { ext: "webp", mime: "image/webp" } });
    expect(controllaImmagine(webpEsteso())).toEqual({ tipo: { ext: "webp", mime: "image/webp" } });
    expect(controllaImmagine(webpSenzaPerdita())).toEqual({ tipo: { ext: "webp", mime: "image/webp" } });
    expect(controllaImmagine(ico(1))).toEqual({ tipo: { ext: "ico", mime: "image/vnd.microsoft.icon" } });
    expect(controllaImmagine(ico(3))).toEqual({ tipo: { ext: "ico", mime: "image/vnd.microsoft.icon" } });
    expect(controllaImmagine(Buffer.from(SVG_PULITO))).toEqual({ tipo: { ext: "svg", mime: "image/svg+xml" } });
  });

  it.each([
    ["una pagina HTML", "<!doctype html><html><body>ciao</body></html>"],
    ["una pagina HTML che contiene uno SVG", `<html><body>${SVG("<rect/>")}</body></html>`],
    ["testo", "ciao a tutti"],
    ["JSON", '{"logo": "x"}'],
    ["un XML che non è SVG", '<?xml version="1.0"?><rss version="2.0"><channel/></rss>'],
    ["un testo che nomina svg", "questo non è un <svg> vero"],
    ["uno SVG con prefisso sulla radice", '<svg:svg xmlns:svg="http://www.w3.org/2000/svg"/>'],
    ["un PDF", "%PDF-1.4\n1 0 obj"],
    ["un archivio ZIP", "PK\u0003\u0004 resto"],
    ["un eseguibile", "MZ\u0090\u0000\u0003"],
    ["un file vuoto", ""],
    ["un byte", "\u0089"],
  ])("%s non è un'immagine", (_, contenuto) => expect(tipo(contenuto)).toBe("non_immagine"));

  it("le teste mancate o troncate non passano", () => {
    expect(tipo(Buffer.alloc(0))).toBe("non_immagine");
    expect(tipo(png().subarray(0, 12))).toBe("non_immagine");
    expect(tipo(png().subarray(0, 15))).toBe("non_immagine");
    expect(tipo(Buffer.concat([png().subarray(0, 12), Buffer.from("XXXX"), Buffer.alloc(8)]))).toBe("non_immagine");
    expect(tipo(Buffer.from([0xff, 0xd8]))).toBe("non_immagine");
    expect(tipo(Buffer.from("GIF86a" + "\0".repeat(10)))).toBe("non_immagine");
    expect(tipo(Buffer.from("GIF89b" + "\0".repeat(10)))).toBe("non_immagine");
    // RIFF che non è WebP (un audio WAV)
    expect(tipo(Buffer.concat([Buffer.from("RIFF"), Buffer.from([0x20, 0, 0, 0]), Buffer.from("WAVEfmt "), Buffer.alloc(20)]))).toBe("non_immagine");
    // ICO: nessuna immagine, un cursore (tipo 2), tabella più corta del dichiarato
    expect(tipo(Buffer.concat([Buffer.from([0, 0, 1, 0, 0, 0]), Buffer.alloc(40)]))).toBe("non_immagine");
    expect(tipo(Buffer.concat([Buffer.from([0, 0, 2, 0, 1, 0]), Buffer.alloc(40)]))).toBe("non_immagine");
    expect(tipo(Buffer.concat([Buffer.from([0, 0, 1, 0, 5, 0]), Buffer.alloc(30)]))).toBe("non_immagine");
  });

  it("un file di un tipo con la testa di un altro è del tipo che dicono i byte", () => {
    // il nome o il Content-Type non entrano: qui c'è solo il contenuto
    expect(tipo(Buffer.concat([png(), Buffer.from("<script>alert(1)</script>")]))).toBe("png");
  });

  it("uno SVG con la testa di un altro tipo non è SVG; uno con byte non UTF-8 o in UTF-16 non passa", () => {
    expect(tipo(Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(SVG("<rect/>"), "utf16le")]))).toBe("non_immagine");
    expect(tipo(Buffer.concat([Buffer.from("<svg xmlns='http://www.w3.org/2000/svg'><title>"), Buffer.from([0xc3, 0x28]), Buffer.from("</title></svg>")]))).toBe("non_immagine");
  });
});

describe("uno SVG pulito si accetta", () => {
  it.each([
    ["quello dei programmi di disegno", SVG_PULITO],
    ["il più piccolo", '<svg xmlns="http://www.w3.org/2000/svg"/>'],
    ["con il BOM", "\ufeff" + SVG("<rect width='5' height='5'/>")],
    ["con spazi e un a capo prima", `\n  \t${SVG("<rect/>")}`],
    ["con la dichiarazione e un commento prima", `<?xml version="1.0" encoding="utf-8"?>\n<!-- altro -->\n${SVG("<rect/>")}`],
    ["senza dichiarazione", SVG("<circle r='3'/>")],
    ["con la dichiarazione standalone", `<?xml version="1.0" encoding="UTF-8" standalone="no"?>\n${SVG("<rect/>")}`],
    ["con il DOCTYPE di W3C", `<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">${SVG("<rect/>")}`],
    ["con i metadati di Inkscape", SVG(`<metadata><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">x</dc:title></rdf:RDF></metadata><rect inkscape:export-filename="C:\\x\\data.png" sodipodi:docname="logo.svg" data-name="Layer 1"/>`)],
    ["con riferimenti #interni", SVG(`<defs><path id="a" d="M0 0L5 5"/></defs><use xlink:href="#a"/><use href="#a"/><a href="#a"><rect/></a>`)],
    ["con gradienti e filtri in attributi", SVG(`<defs><linearGradient id="g"/><filter id="f"><feGaussianBlur stdDeviation="1"/></filter><mask id="m"/><clipPath id="c"/></defs><rect fill="url(#g)" filter="url(#f)" mask="url(#m)" clip-path="url(#c)"/>`)],
    ["con uno stile in CDATA", SVG(`<style type="text/css"><![CDATA[ .a{fill:#fff;stroke:url(#g)} .b{font:12px 'Open Sans'} ]]></style><rect class="a"/>`)],
    ["con il testo che parla di javascript e data", SVG(`<title>data: strutture</title><desc>javascript: il libro</desc><text>un onore</text>`)],
    ["con un riferimento numerico innocuo", SVG(`<text>&#169; Alfa &#x2605;</text>`)],
  ])("%s", (_, testo) => expect(tipo(testo)).toBe("svg"));
});

describe("uno SVG che esegue o carica qualcosa si rifiuta", () => {
  const RIFIUTI: [string, string][] = [
    ["uno script", SVG("<script>alert(1)</script>")],
    ["uno script in maiuscolo", SVG("<SCRIPT>alert(1)</SCRIPT>")],
    ["uno script con prefisso", SVG("<svg:script>alert(1)</svg:script>")],
    ["uno script con un altro prefisso", SVG('<x:script xmlns:x="http://www.w3.org/2000/svg">alert(1)</x:script>')],
    ["uno script con un prefisso fuori ASCII", SVG('<é:script xmlns:é="http://www.w3.org/2000/svg"/>')],
    ["uno script con un href", SVG('<script href="https://evil.example/x.js"/>')],
    ["uno script con lo spazio dopo il segno <", SVG("<  script>alert(1)</script>")],
    ["foreignObject", SVG('<foreignObject width="10" height="10"><div xmlns="http://www.w3.org/1999/xhtml">x</div></foreignObject>')],
    ["foreignobject in minuscolo", SVG("<foreignobject><body onload='x'/></foreignobject>")],
    ["un iframe", SVG('<iframe src="https://evil.example/"/>')],
    ["un embed", SVG('<embed src="https://evil.example/x.swf"/>')],
    ["un object", SVG('<object data="https://evil.example/x"/>')],
    ["un link", SVG('<link rel="stylesheet" href="https://evil.example/x.css"/>')],
    ["un meta", SVG('<meta http-equiv="refresh" content="0;url=https://evil.example/"/>')],
    ["un base", SVG('<base href="https://evil.example/"/>')],
    ["un audio", SVG("<audio src='x'/>")],
    ["un handler di XML Events", SVG('<handler type="application/ecmascript">alert(1)</handler>')],
    ["un listener di XML Events", SVG('<listener event="click" handler="#h"/>')],
    ["onload sulla radice", `<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>`],
    ["onload su un elemento", SVG('<rect onload="alert(1)"/>')],
    ["onclick con apici singoli", SVG("<rect onclick='alert(1)'/>")],
    ["un gestore senza virgolette", SVG("<rect onmouseover=alert(1) />")],
    ["un gestore con lo spazio prima del segno =", SVG('<rect onclick ="alert(1)"/>')],
    ["un gestore in maiuscolo", SVG('<rect ONERROR="alert(1)"/>')],
    ["un gestore con prefisso", SVG('<rect xlink:onclick="alert(1)"/>')],
    ["un gestore dopo una barra", `<svg/onload=alert(1) xmlns="http://www.w3.org/2000/svg"></svg>`],
    ["un gestore di animazione", SVG('<animate onbegin="alert(1)" attributeName="x"/>')],
    ["javascript: in un href", SVG('<a href="javascript:alert(1)"><rect/></a>')],
    ["javascript: in un xlink:href", SVG('<a xlink:href="javascript:alert(1)"><rect/></a>')],
    ["javascript: con maiuscole e spazi", SVG('<a href="  JaVaScRiPt:alert(1)"><rect/></a>')],
    ["javascript: spezzato da un a capo numerico", SVG('<a href="java&#10;script:alert(1)"><rect/></a>')],
    ["javascript: spezzato da un tab", SVG('<a href="java&#x09;script:alert(1)"><rect/></a>')],
    ["javascript: con la j in forma numerica", SVG('<a href="&#106;avascript:alert(1)"><rect/></a>')],
    ["javascript: con la j in esadecimale", SVG('<a href="&#x6A;avascript:alert(1)"><rect/></a>')],
    ["javascript: con una lettera in forma numerica senza punto e virgola", SVG('<a href="jav&#97script:alert(1)"><rect/></a>')],
    ["javascript: con uno spazio di larghezza zero", SVG('<a href="\u200bjavascript:alert(1)"><rect/></a>')],
    ["javascript: in un attributo qualunque", SVG('<rect data-x="x" title="javascript:alert(1)"/>')],
    ["vbscript:", SVG('<a href="vbscript:msgbox(1)"><rect/></a>')],
    ["data: in un image", SVG('<image href="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="/>')],
    ["data: in un use", SVG('<use xlink:href="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=#x"/>')],
    ["data: html in un xlink:href", SVG('<image xlink:href="data:text/html,&lt;script&gt;alert(1)&lt;/script&gt;"/>')],
    ["un indirizzo esterno in un image", SVG('<image href="https://evil.example/x.png"/>')],
    ["un indirizzo esterno in un xlink:href", SVG('<image xlink:href="http://evil.example/x.png"/>')],
    ["un use verso un altro documento", SVG('<use xlink:href="https://evil.example/s.svg#a"/>')],
    ["un use verso un indirizzo senza schema", SVG('<use href="//evil.example/s.svg#a"/>')],
    ["un riferimento relativo", SVG('<image href="logo.png"/>')],
    ["un riferimento assoluto sul sito", SVG('<image href="/logos/personale.png"/>')],
    ["un link verso un altro sito", SVG('<a href="https://example.org/"><rect/></a>')],
    ["un href senza virgolette", SVG("<a href=javascript:alert(1)><rect/></a>")],
    ["un feImage esterno", SVG('<filter id="f"><feImage href="http://evil.example/x"/></filter>')],
    ["un href vuoto", SVG('<use href=""/>')],
    ["xml:base", SVG('<rect xml:base="https://evil.example/"/>')],
    ["@import in uno stile", SVG("<style>@import url(https://evil.example/x.css);</style>")],
    ["@import con un commento in mezzo", SVG('<style>/**/@import/**/"x.css";</style>')],
    ["un url() esterno in uno stile", SVG("<style>.a{background:url(https://evil.example/x.png)}</style>")],
    ["un url() esterno con virgolette", SVG("<style>.a{background:url('https://evil.example/x.png')}</style>")],
    ["un url() esterno in uno stile in CDATA", SVG("<style><![CDATA[.a{background:url(//evil.example/x)}]]></style>")],
    ["un url() esterno in un attributo style", SVG('<rect style="fill:url(https://evil.example/x)"/>')],
    ["un url() con data: in uno stile", SVG('<rect style="background:url(data:image/png;base64,AAAA)"/>')],
    ["expression() in uno stile", SVG("<style>.a{fill:expression(alert(1))}</style>")],
    ["behavior in uno stile", SVG("<style>.a{behavior:url(x.htc)}</style>")],
    ["-moz-binding in uno stile", SVG("<style>.a{-moz-binding:url(x.xml#a)}</style>")],
    ["una sequenza di escape CSS", SVG("<style>.a{background:\\75 rl(https://evil.example/x)}</style>")],
    ["un url() scritto con un riferimento numerico", SVG("<style>.a{background:&#x75;rl(https://evil.example/x)}</style>")],
    ["image-set() in uno stile", SVG('<style>.a{background:image-set("https://evil.example/x.png" 1x)}</style>')],
    ["lo stile con prefisso", SVG("<svg:style>@import 'x.css';</svg:style>")],
    ["animate che porta un href", SVG('<a><animate attributeName="href" values="javascript:alert(1)"/><rect/></a>')],
    ["set che porta un xlink:href", SVG('<a><set attributeName="xlink:href" to="https://evil.example/"/><rect/></a>')],
    ["animate che cambia un gestore", SVG('<rect><animate attributeName="onclick" to="alert(1)"/></rect>')],
    ["animate con un valore javascript:", SVG('<animate attributeName="x" values="javascript:alert(1)"/>')],
    ["una entità", `<!DOCTYPE svg [<!ENTITY x "&#60;script&#62;alert(1)&#60;/script&#62;">]><svg xmlns="http://www.w3.org/2000/svg">&x;</svg>`],
    ["un DOCTYPE con sottoinsieme interno", `<!DOCTYPE svg [<!ELEMENT svg ANY>]><svg xmlns="http://www.w3.org/2000/svg"/>`],
    ["un foglio di stile xml", `<?xml-stylesheet href="https://evil.example/x.css" type="text/css"?><svg xmlns="http://www.w3.org/2000/svg"/>`],
    ["un'istruzione php", `<svg xmlns="http://www.w3.org/2000/svg"><?php echo 1 ?></svg>`],
    ["una codifica diversa da UTF-8 (UTF-7)", `<?xml version="1.0" encoding="UTF-7"?>${SVG("<rect/>")}`],
    ["una codifica diversa da UTF-8 (UTF-16)", `<?xml version="1.0" encoding="UTF-16"?>${SVG("<rect/>")}`],
    ["una codifica diversa da UTF-8 (latin1)", `<?xml version="1.0" encoding='ISO-8859-1'?>${SVG("<rect/>")}`],
    ["un byte nullo", SVG("<rect/>\u0000<script>")],
  ];
  it.each(RIFIUTI)("%s", (_, testo) => {
    expect(tipo(testo)).toBe("svg_non_ammesso");
    expect(svgAmmesso(testo)).toBe(false);
  });

  it("il testo che somiglia a un attributo, fuori da un tag, non è un attributo: si accetta; dentro un tag si rifiuta", () => {
    expect(tipo(SVG("<text>onore=1 onclick=\"alert(1)\" href='http://x.example/'</text>"))).toBe("svg");
    expect(tipo(SVG('<text onore="1">x</text>'))).toBe("svg_non_ammesso");
  });
});

// ---- La grandezza di un raster ------------------------------------------------------------------------------------------------------
describe("la grandezza di un raster", () => {
  const FORMATI: [string, (l: number, a: number) => Buffer][] = [
    ["PNG", pngIntestazione],
    ["JPEG", jpeg],
    ["GIF", (l, a) => gif("89a", l, a)],
    ["WebP con perdita (VP8)", webp],
    ["WebP esteso (VP8X)", webpEsteso],
    ["WebP senza perdita (VP8L)", webpSenzaPerdita],
  ];

  it("i limiti sono 4096 pixel per lato e 16 megapixel", () => {
    expect(LATO_MAX).toBe(4096);
    expect(PIXEL_MAX).toBe(16_000_000);
  });

  describe.each(FORMATI)("%s", (_, fai) => {
    it.each([
      [1, 1],
      [8, 8],
      [4096, 1],
      [1, 4096],
      [4096, 3906],
      [3906, 4096],
      [4000, 4000],
    ])("%i x %i si accetta", (l, a) => expect(controllaImmagine(fai(l, a))).toMatchObject({ tipo: expect.anything() }));

    it.each([
      ["un lato sopra 4096 (larghezza)", 4097, 1],
      ["un lato sopra 4096 (altezza)", 1, 4097],
      ["16 megapixel sfondati con i due lati nel limite", 4096, 4096],
      ["16 megapixel sfondati di poco", 4000, 4001],
      ["due lati grandi", 8000, 8000],
    ])("%s si rifiuta con il testo della grandezza", (_n, l, a) => {
      expect(controllaImmagine(fai(l, a))).toEqual({ errore: "troppi_pixel" });
    });

    it("una testata che dice 0 pixel non è un'immagine", () => {
      // VP8X e VP8L scrivono "meno uno": lì lo 0 non si può dire
      if (/VP8[XL]/.test(_)) return;
      expect(controllaImmagine(fai(0, 8))).toEqual({ errore: "non_immagine" });
      expect(controllaImmagine(fai(8, 0))).toEqual({ errore: "non_immagine" });
    });
  });

  it.each([
    ["PNG 2147483647 x 2147483647", pngIntestazione(0x7fffffff, 0x7fffffff)],
    ["PNG 65535 x 65535", pngIntestazione(65535, 65535)],
    ["PNG 30000 x 30000", pngIntestazione(30000, 30000)],
    ["PNG 4294967295 x 1", pngIntestazione(0xffffffff, 1)],
    ["JPEG 65535 x 65535", jpeg(65535, 65535)],
    ["GIF 65535 x 65535", gif("89a", 65535, 65535)],
    ["WebP VP8 16383 x 16383", webp(16383, 16383)],
    ["WebP VP8X 16777216 x 16777216", webpEsteso(1 << 24, 1 << 24)],
    ["WebP VP8L 16384 x 16384", webpSenzaPerdita(16384, 16384)],
  ])("%s si rifiuta", (_, dati) => expect(controllaImmagine(dati)).toEqual({ errore: "troppi_pixel" }));

  it("una testata troncata non è un'immagine, in ogni formato", () => {
    const intero = (b: Buffer, n: number) => b.subarray(0, n);
    // PNG: la testata arriva a 24 byte
    for (let n = 8; n < 24; n++) expect(tipo(intero(pngIntestazione(8, 8), n)), `PNG ${n}`).toBe("non_immagine");
    expect(tipo(intero(pngIntestazione(8, 8), 24))).toBe("png");
    // GIF: si cammina sui blocchi e serve il trailer (la prova di ogni lunghezza di un GIF animato sta più sotto)
    for (let n = 6; n < gif().length; n++) expect(tipo(intero(gif(), n)), `GIF ${n}`).toBe("non_immagine");
    expect(tipo(gif())).toBe("gif");
    // WebP: 30 byte (VP8 e VP8X), 25 (VP8L)
    for (let n = 12; n < 30; n++) {
      expect(tipo(intero(webp(), n)), `VP8 ${n}`).toBe("non_immagine");
      expect(tipo(intero(webpEsteso(), n)), `VP8X ${n}`).toBe("non_immagine");
    }
    for (let n = 12; n < 25; n++) expect(tipo(intero(webpSenzaPerdita(), n)), `VP8L ${n}`).toBe("non_immagine");
    expect(tipo(intero(webp(), 30))).toBe("webp");
    expect(tipo(intero(webpSenzaPerdita(), 25))).toBe("webp");
    // JPEG: senza il segmento SOF intero non c'è una grandezza
    const j = jpeg();
    const sof = j.indexOf(Buffer.from([0xff, 0xc0]));
    for (let n = 3; n < sof + 9; n++) expect(tipo(intero(j, n)), `JPEG ${n}`).toBe("non_immagine");
    expect(tipo(intero(j, sof + 9))).toBe("jpg");
  });

  it("un WebP con un primo blocco che non è VP8, VP8L o VP8X, o con la firma del frame sbagliata, non è un'immagine", () => {
    const strano = webp();
    strano.write("VP8Z", 12, "ascii");
    expect(tipo(strano)).toBe("non_immagine");
    const firma = webp();
    firma[23] = 0x00;
    expect(tipo(firma)).toBe("non_immagine");
    const senzaFirma = webpSenzaPerdita();
    senzaFirma[20] = 0x00;
    expect(tipo(senzaFirma)).toBe("non_immagine");
  });

  describe("JPEG: il segmento SOF si cerca come lo cerca il browser", () => {
    const soi = Buffer.from([0xff, 0xd8]);
    const segmento = (marker: number, corpo: Buffer) => {
      const testa = Buffer.from([0xff, marker, 0, 0]);
      testa.writeUInt16BE(corpo.length + 2, 2);
      return Buffer.concat([testa, corpo]);
    };
    const sofCorpo = (l: number, a: number) => Buffer.from([8, a >> 8, a & 0xff, l >> 8, l & 0xff, 1, 1, 0x11, 0]);

    it("ogni SOF vale, tranne DHT (c4), JPG (c8) e DAC (cc), che hanno quei numeri ma sono altro", () => {
      for (const m of [0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf]) {
        expect(controllaImmagine(Buffer.concat([soi, segmento(m, sofCorpo(8000, 8))])), m.toString(16)).toEqual({ errore: "troppi_pixel" });
        expect(tipo(Buffer.concat([soi, segmento(m, sofCorpo(8, 8))])), m.toString(16)).toBe("jpg");
      }
      for (const m of [0xc4, 0xc8, 0xcc]) expect(tipo(Buffer.concat([soi, segmento(m, sofCorpo(8, 8))])), m.toString(16)).toBe("non_immagine");
    });

    it("i segmenti prima del SOF si saltano per la loro lunghezza (anche un APP1 con dentro un SOF finto), i byte di riempimento pure", () => {
      const finto = segmento(0xe1, Buffer.concat([Buffer.from("Exif\0\0"), soi, segmento(0xc0, sofCorpo(8000, 8000))]));
      expect(tipo(Buffer.concat([soi, finto, segmento(0xc0, sofCorpo(8, 8))]))).toBe("jpg");
      expect(controllaImmagine(Buffer.concat([soi, finto, segmento(0xc0, sofCorpo(8000, 8000))]))).toEqual({ errore: "troppi_pixel" });
      const riempimento = Buffer.from([0xff, 0xff, 0xff]);
      expect(tipo(Buffer.concat([soi, riempimento, finto, riempimento, segmento(0xc2, sofCorpo(8, 8))]))).toBe("jpg");
    });

    it("senza un SOF (EOI o inizio dei dati prima, lunghezza assurda, fuori sincronia) non è un'immagine", () => {
      expect(tipo(Buffer.concat([soi, Buffer.from([0xff, 0xd9]), segmento(0xc0, sofCorpo(8, 8))]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([soi, segmento(0xda, Buffer.alloc(4)), segmento(0xc0, sofCorpo(8, 8))]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([soi, Buffer.from([0xff, 0xe0, 0x00, 0x01]), segmento(0xc0, sofCorpo(8, 8))]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([soi, segmento(0xe0, Buffer.alloc(2)), Buffer.from([0x12, 0x34]), segmento(0xc0, sofCorpo(8, 8))]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([soi, Buffer.from([0xff, 0x00, 0x00, 0x05]), segmento(0xc0, sofCorpo(8, 8))]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([soi, segmento(0xe0, Buffer.alloc(60000)), Buffer.from([0xff, 0xc0])]))).toBe("non_immagine");
    });

    it("l'altezza 0 (si dice dopo, con DNL) non è un'immagine", () => {
      expect(tipo(Buffer.concat([soi, segmento(0xc0, sofCorpo(8, 0))]))).toBe("non_immagine");
    });
  });

  describe("ICO: la grandezza dell'immagine dentro, non quella che dice la tabella", () => {
    /** Un ICO con una voce che dice 256 x 256 e il contenuto dato in fondo. */
    const conVoce = (dentro: Buffer, offset?: number) => {
      const testa = Buffer.alloc(22);
      testa.set([0, 0, 1, 0, 1, 0], 0);
      testa.writeUInt32LE(dentro.length, 14);
      testa.writeUInt32LE(offset ?? testa.length, 18);
      return Buffer.concat([testa, dentro]);
    };
    const dib = (dimensioneTestata: number, l: number, a: number) => {
      const b = Buffer.alloc(Math.max(dimensioneTestata, 16) + 8);
      b.writeUInt32LE(dimensioneTestata, 0);
      if (dimensioneTestata === 12) {
        b.writeUInt16LE(l, 4);
        b.writeUInt16LE(a, 6);
      } else {
        b.writeInt32LE(l, 4);
        b.writeInt32LE(a, 8);
      }
      return b;
    };

    it("un PNG dentro si guarda: 65535 x 65535 dietro una tabella che dice 256 si rifiuta", () => {
      expect(tipo(conVoce(png(8)))).toBe("ico");
      expect(tipo(conVoce(pngIntestazione(256, 256)))).toBe("ico");
      expect(controllaImmagine(conVoce(pngIntestazione(65535, 65535)))).toEqual({ errore: "troppi_pixel" });
      expect(controllaImmagine(conVoce(pngIntestazione(5000, 1)))).toEqual({ errore: "troppi_pixel" });
    });

    it("un DIB dentro si guarda: l'altezza conta anche la maschera, il doppio", () => {
      expect(tipo(conVoce(dib(40, 32, 64)))).toBe("ico");
      expect(tipo(conVoce(dib(40, 4000, 6000)))).toBe("ico");
      expect(controllaImmagine(conVoce(dib(40, 5000, 8)))).toEqual({ errore: "troppi_pixel" });
      expect(controllaImmagine(conVoce(dib(40, 8, 2 * 5000)))).toEqual({ errore: "troppi_pixel" });
      expect(controllaImmagine(conVoce(dib(40, 4000, 2 * 4001)))).toEqual({ errore: "troppi_pixel" });
      expect(controllaImmagine(conVoce(dib(108, 65535, 65535)))).toEqual({ errore: "troppi_pixel" });
      expect(controllaImmagine(conVoce(dib(40, -65535, 8)))).toEqual({ errore: "troppi_pixel" });
      expect(controllaImmagine(conVoce(dib(12, 65535, 65535)))).toEqual({ errore: "troppi_pixel" });
      expect(tipo(conVoce(dib(12, 32, 32)))).toBe("ico");
    });

    it("una voce che non si legge (fuori dal file, formato che non è PNG né DIB) non costa niente: il browser non la disegna", () => {
      expect(tipo(conVoce(Buffer.alloc(40, 7), 100000))).toBe("ico");
      expect(tipo(conVoce(Buffer.from("GIF89a" + "\0".repeat(30)), 22))).toBe("ico");
      expect(tipo(conVoce(Buffer.alloc(3), 21))).toBe("ico");
    });

    it("basta una voce grande tra molte buone", () => {
      const buoni = ico(3, 16);
      const voce = 6 + 16 * 2 + 12;
      const grande = Buffer.concat([buoni, pngIntestazione(9000, 9000)]);
      grande.writeUInt32LE(buoni.length, voce);
      expect(controllaImmagine(grande)).toEqual({ errore: "troppi_pixel" });
      expect(tipo(buoni)).toBe("ico");
    });

    it("256 voci si guardano tutte, e una tabella che non sta nel file non è un ICO", () => {
      expect(tipo(ico(256, 8))).toBe("ico");
      expect(tipo(Buffer.concat([Buffer.from([0, 0, 1, 0, 0, 1]), Buffer.alloc(100)]))).toBe("non_immagine");
    });
  });

  describe("GIF: ogni fotogramma e la tela, non solo lo schermo logico", () => {
    const troppi = { errore: "troppi_pixel" };
    const uno = (schermo: [number, number], f: FotogrammaGif) => gifConFotogrammi(schermo, [f]);

    it("il caso trovato: lo schermo 16 x 16 e un fotogramma 4200 x 4200 (35 byte di struttura) non passa", () => {
      const grande = uno([16, 16], { larghezza: 4200, altezza: 4200 });
      expect(controllaImmagine(grande)).toEqual(troppi);
      // lo stesso con i byte contati a mano: testata, schermo, tabella da 2 voci, un fotogramma di 2 byte di dati, trailer
      const a_mano = Buffer.concat([
        Buffer.from("GIF89a"),
        Buffer.from([16, 0, 16, 0, 0x80, 0, 0, 0, 0, 0, 255, 255, 255]),
        Buffer.from([0x2c, 0, 0, 0, 0, 0x68, 0x10, 0x68, 0x10, 0, 2, 2, 0x44, 0x01, 0, 0x3b]),
      ]);
      expect(a_mano.length).toBe(35);
      expect(controllaImmagine(a_mano)).toEqual(troppi);
      // e con un fotogramma della grandezza dello schermo si apre
      expect(tipo(uno([16, 16], { larghezza: 16, altezza: 16 }))).toBe("gif");
    });

    it.each([
      ["un fotogramma sopra 4096 di larghezza", [8, 8], { larghezza: 4097, altezza: 1 }],
      ["un fotogramma sopra 4096 di altezza", [8, 8], { larghezza: 1, altezza: 4097 }],
      ["un fotogramma sopra i 16 megapixel con i lati nel limite", [8, 8], { larghezza: 4096, altezza: 4096 }],
      ["un fotogramma da 65535 x 65535", [8, 8], { larghezza: 65535, altezza: 65535 }],
      ["lo schermo e il fotogramma da 4200 x 4200", [4200, 4200], { larghezza: 4200, altezza: 4200 }],
      ["lo schermo sopra il limite con un fotogramma piccolo", [5000, 8], { larghezza: 8, altezza: 8 }],
      ["la tela cresce a destra: un fotogramma di 200 spostato di 4000", [16, 16], { sinistra: 4000, larghezza: 200, altezza: 16 }],
      ["la tela cresce in basso: un fotogramma di 200 spostato di 4000", [16, 16], { sopra: 4000, larghezza: 16, altezza: 200 }],
      ["la tela cresce a 65536 con un fotogramma di un pixel", [16, 16], { sinistra: 65535, larghezza: 1, altezza: 1 }],
      ["la tela oltre i 16 megapixel con i lati nel limite (4000 x 4001)", [4000, 4001], { larghezza: 4000, altezza: 4001 }],
    ] as [string, [number, number], FotogrammaGif][])("%s si rifiuta con il testo della grandezza", (_n, schermo, f) => {
      expect(controllaImmagine(uno(schermo, f))).toEqual(troppi);
    });

    it("basta un fotogramma grande tra molti buoni, anche l'ultimo", () => {
      const buoni = Array.from({ length: 20 }, () => ({ larghezza: 16, altezza: 16 }));
      expect(tipo(gifConFotogrammi([16, 16], buoni))).toBe("gif");
      expect(controllaImmagine(gifConFotogrammi([16, 16], [...buoni, { larghezza: 4200, altezza: 4200 }]))).toEqual(troppi);
    });

    it("i GIF veri passano: animato con ciclo e commento, tabelle locali, 87a e 89a, nei limiti (4096 x 3906, 4000 x 4000)", () => {
      const animato = gifConFotogrammi(
        [32, 32],
        [
          { larghezza: 32, altezza: 32 },
          { sinistra: 4, sopra: 4, larghezza: 24, altezza: 24, tabella: 16, dati: [Buffer.alloc(255, 1), Buffer.alloc(100, 2)] },
          { sinistra: 8, sopra: 8, larghezza: 8, altezza: 8, tabella: 256 },
          { larghezza: 1, altezza: 1, tabella: 2, dati: [] },
        ],
        "89a",
        true,
      );
      expect(tipo(animato)).toBe("gif");
      expect(tipo(gif("87a"))).toBe("gif");
      expect(tipo(gif("89a", 4096, 3906))).toBe("gif");
      expect(tipo(gif("89a", 4000, 4000))).toBe("gif");
      // un fotogramma in un angolo della tela che entra nello schermo
      expect(tipo(uno([100, 100], { sinistra: 99, sopra: 99, larghezza: 1, altezza: 1 }))).toBe("gif");
    });

    it("ogni lunghezza più corta di un GIF animato, con tabelle ed estensioni, non è un'immagine; intero sì", () => {
      const animato = gifConFotogrammi([32, 32], [{ larghezza: 32, altezza: 32 }, { sinistra: 2, larghezza: 8, altezza: 8, tabella: 8, dati: [Buffer.alloc(50, 3)] }], "89a", true);
      for (let n = 0; n < animato.length; n++) expect(tipo(animato.subarray(0, n)), `${n} byte`).toBe("non_immagine");
      expect(tipo(animato)).toBe("gif");
      // quello che c'è dopo il trailer il browser non lo guarda: un file che lo ha si accetta com'è
      expect(tipo(Buffer.concat([animato, Buffer.from("<script>alert(1)</script>")]))).toBe("gif");
    });

    it("gli altri formati non cambiano: PNG e ICO nei limiti si accettano, sopra no, e dei byte di un GIF dentro non contano", () => {
      expect(tipo(png(16))).toBe("png");
      expect(tipo(ico(2, 16))).toBe("ico");
      expect(controllaImmagine(pngIntestazione(LATO_MAX + 1, 8))).toEqual(troppi);
      expect(controllaImmagine(pngIntestazione(4200, 4200))).toEqual(troppi);
      // un ICO che porta dentro un PNG enorme resta rifiutato come prima
      const enorme = pngIntestazione(5000, 5000);
      const voce = Buffer.from([0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 32, 0, enorme.length & 0xff, 0, 0, 0, 22, 0, 0, 0]);
      expect(controllaImmagine(Buffer.concat([voce, enorme]))).toEqual(troppi);
      // un PNG e un JPEG che hanno nei dati un GIF con un fotogramma enorme restano quello che dicono le loro testate
      const nascosto = gifConFotogrammi([16, 16], [{ larghezza: 4200, altezza: 4200 }]);
      expect(tipo(Buffer.concat([png(16), nascosto]))).toBe("png");
      expect(tipo(Buffer.concat([jpeg(16, 16), nascosto]))).toBe("jpg");
      // e un file che comincia con la firma di un GIF 4200 x 4200 senza fotogrammi non è un GIF
      expect(tipo(gif("89a", 4200, 16).subarray(0, 13))).toBe("non_immagine");
    });

    it("una struttura che non torna non è un GIF", () => {
      const intero = gif();
      const taglia = (da: number, byte: number[]) => Buffer.concat([intero.subarray(0, da), Buffer.from(byte), intero.subarray(da + byte.length)]);
      // lo schermo a 0, senza fotogrammi (solo il trailer), un blocco che non esiste (0x05), un sotto-blocco più lungo del file
      expect(tipo(taglia(6, [0, 0, 8, 0]))).toBe("non_immagine");
      expect(tipo(taglia(6, [8, 0, 0, 0]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([intero.subarray(0, 13 + 12), Buffer.from([0x3b])]))).toBe("non_immagine");
      expect(tipo(Buffer.concat([intero.subarray(0, 13 + 12), Buffer.from([0x05, 0x3b])]))).toBe("non_immagine");
      // un blocco che non esiste davanti a un fotogramma buono: non si salta, il file non torna
      expect(tipo(Buffer.concat([intero.subarray(0, 13 + 12), Buffer.from([0x05]), intero.subarray(13 + 12)]))).toBe("non_immagine");
      // un'estensione con un'etichetta che non si conosce è un'estensione: si salta per i suoi sotto-blocchi
      expect(tipo(Buffer.concat([intero.subarray(0, 13 + 12), Buffer.from([0x21, 0x77, 0x02, 0x01, 0x02, 0x00]), intero.subarray(13 + 12)]))).toBe("gif");
      expect(tipo(Buffer.concat([intero.subarray(0, 13 + 12), Buffer.from([0x21, 0xfe, 0xff, 0x61])]))).toBe("non_immagine");
      // un fotogramma senza il trailer, e un fotogramma il cui descrittore esce dal file
      expect(tipo(intero.subarray(0, intero.length - 1))).toBe("non_immagine");
      expect(tipo(Buffer.concat([intero.subarray(0, 13 + 12), Buffer.from([0x2c, 0, 0, 0, 0, 8, 0])]))).toBe("non_immagine");
      // la tabella globale che dichiara più di quanto il file ha
      expect(tipo(Buffer.concat([Buffer.from("GIF89a"), Buffer.from([8, 0, 8, 0, 0xf7, 0, 0]), Buffer.alloc(30)]))).toBe("non_immagine");
    });
  });
});
