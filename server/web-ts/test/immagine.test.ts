import { describe, expect, it } from "vitest";
import { controllaImmagine, svgAmmesso } from "@/lib/core/immagine";
import { gif, ico, jpeg, png, SVG_PULITO, webp } from "./immagini-prova";

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

  it("uno SVG con del testo che somiglia a un gestore si rifiuta lo stesso: un dubbio vale un no", () => {
    expect(tipo(SVG("<text>onore=1</text>"))).toBe("svg_non_ammesso");
  });
});
