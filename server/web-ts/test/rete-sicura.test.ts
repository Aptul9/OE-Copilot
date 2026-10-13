import { execFileSync } from "node:child_process";
import fs from "node:fs";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import {
  ATTESA_MS,
  controllaCollegato,
  controllaUrl,
  ErroreRete,
  indirizzoAmmesso,
  LIMITE_ACCETTATO,
  LIMITE_LETTURA,
  MAX_REINDIRIZZAMENTI,
  type Opzioni,
  portaAmmessa,
  scaricaSicuro,
  TESTI,
  type SoloProve,
} from "@/lib/core/rete-sicura";
import { png } from "./immagini-prova";
import { avviaServerProva, rimanda, risposta, type ServerProva } from "./server-prova";

const SORGENTI = fileURLToPath(new URL("../src", import.meta.url));

/** Quello che la prova toglie alle regole: 127.0.0.1 è raggiungibile (è il server di prova), le porte sono libere, i nomi .test vanno a 127.0.0.1. */
const locale = (extra: SoloProve = {}): SoloProve => ({
  risolvi: async (host) => (host.endsWith(".test") ? ["127.0.0.1"] : [host]),
  ammettiIndirizzo: (ip) => ip === "127.0.0.1" || indirizzoAmmesso(ip),
  ammettiPorta: () => true,
  attesaMs: 3000,
  ...extra,
});

const rifiutato = async (promessa: Promise<unknown>) => {
  const e = await promessa.then(
    () => null,
    (x: unknown) => x,
  );
  expect(e, "doveva essere rifiutato").toBeInstanceOf(ErroreRete);
  return e as ErroreRete;
};

describe("quali indirizzi IP sono pubblici", () => {
  const VIETATI = [
    // IPv4: i confini di ogni rete vietata, dentro
    "0.0.0.0", "0.255.255.255", "10.0.0.0", "10.255.255.255", "100.64.0.0", "100.100.100.200", "100.127.255.255", "127.0.0.1", "127.255.255.254",
    "169.254.0.0", "169.254.169.254", "169.254.170.2", "169.254.255.255", "172.16.0.0", "172.31.255.255", "192.0.0.1", "192.0.2.1", "192.88.99.1",
    "192.168.0.1", "192.168.255.255", "198.18.0.0", "198.19.255.255", "198.51.100.1", "203.0.113.1", "224.0.0.1", "239.255.255.255", "240.0.0.1",
    "255.255.255.255",
    // IPv6
    "::", "::1", "::2", "0:0:0:0:0:0:0:1", "::127.0.0.1", "::ffff:127.0.0.1", "::ffff:7f00:1", "::FFFF:7F00:1", "::ffff:10.0.0.1",
    "::ffff:169.254.169.254", "::ffff:8.8.8.8", "::ffff:0:7f00:1", "::ffff:0:127.0.0.1", "::ffff:0:a9fe:a9fe", "::ffff:0:8.8.8.8", "::ffff:0:0:0", "::ffff:0:ffff:ffff",
    "0:0:0:0:ffff:0:7f00:1", "64:ff9b::7f00:1", "64:ff9b::808:808", "64:ff9b:1::1", "100::1", "2001::1", "2001:0:4136:e378:8000:63bf:3fff:fdd2",
    "2001:db8::1", "2002::1", "2002:7f00:1::1", "2002:a9fe:a9fe::1", "fc00::1", "fd00::1", "fdff:ffff:ffff::1", "fe80::1", "FE80::1", "febf::1", "fec0::1",
    "ff00::", "ff02::1", "ff05::2",
  ];
  const AMMESSI = [
    "1.1.1.1", "8.8.8.8", "9.255.255.255", "11.0.0.1", "93.184.216.34", "100.63.255.255", "100.128.0.1", "126.255.255.255", "128.0.0.1", "169.253.255.255",
    "169.255.0.1", "172.15.255.255", "172.32.0.1", "192.0.1.1", "192.167.255.255", "192.169.0.1", "198.17.255.255", "198.20.0.1", "198.51.101.1",
    "203.0.112.255", "203.0.114.1", "223.255.255.255", "2606:4700:4700::1111", "2001:4860:4860::8888", "2a00:1450:4001:81b::200e", "2001:1::1",
    "2001:db7::1", "2003::1", "2600::1",
  ];

  it.each(VIETATI)("%s non è ammesso", (ip) => expect(indirizzoAmmesso(ip)).toBe(false));
  it.each(AMMESSI)("%s è ammesso", (ip) => expect(indirizzoAmmesso(ip)).toBe(true));
  it.each(["", "localhost", "example.org", "1.2.3", "1.2.3.4.5", "256.1.1.1", "::g", "0x7f.0.0.1", "127.1", "2130706433", " 8.8.8.8", "8.8.8.8 "])(
    "%j non è un indirizzo IP, quindi non è ammesso",
    (x) => expect(indirizzoAmmesso(x)).toBe(false),
  );
  it("le sole porte ammesse sono 80 e 443", () => {
    for (const p of [80, 443]) expect(portaAmmessa(p)).toBe(true);
    for (const p of [0, 21, 22, 25, 53, 3000, 3190, 5432, 6379, 8080, 8180, 8443, 9200, 65535]) expect(portaAmmessa(p)).toBe(false);
  });
});

describe("controllaUrl", () => {
  it.each([
    "https://example.org/logo.png",
    "http://example.org/a/b.png?x=1&y=2",
    "HTTPS://EXAMPLE.ORG/Logo.PNG",
    "https://example.org:443/x",
    "http://example.org:80/x",
    "  https://example.org/x.png  ",
    "https://sub.dominio.example.co.uk/a%20b.png",
    "https://xn--e1afmkfd.xn--p1ai/",
    "https://[2606:4700:4700::1111]/x.png",
    "http://8.8.8.8/x.png",
    "https://example.org./x.png",
  ])("%j è un indirizzo valido", (testo) => {
    const u = controllaUrl(testo);
    expect(["http:", "https:"]).toContain(u.protocol);
  });

  it("toglie lo spazio ai lati e tiene il resto come l'analisi dell'URL lo scrive", () => {
    expect(controllaUrl("  HTTPS://EXAMPLE.ORG/Logo.PNG?a=1#f ").href).toBe("https://example.org/Logo.PNG?a=1#f");
  });

  const rifiuti: [string, string[]][] = [
    [
      "uno schema che non è http o https",
      [
        "ftp://example.org/x.png", "file:///etc/passwd", "file://localhost/etc/passwd", "gopher://example.org/", "javascript:alert(1)",
        "data:image/png;base64,AAAA", "ws://example.org/", "wss://example.org/", "blob:https://example.org/0f5a", "jar:http://example.org/a.jar!/",
        "dict://example.org:11211/", "ldap://example.org/", "sftp://example.org/x", "mailto:a@example.org",
      ],
    ],
    [
      "credenziali nell'indirizzo",
      [
        "https://utente:segreta@example.org/x.png", "https://utente@example.org/x.png", "https://:segreta@example.org/x.png",
        "http://example.org@127.0.0.1/", "http://example.org:80@127.0.0.1/", "https://a%40b:c@example.org/",
      ],
    ],
    [
      "una porta che non è 80 né 443",
      [
        "http://example.org:8080/", "https://example.org:8443/x.png", "http://example.org:22/", "http://example.org:25/", "http://example.org:3306/",
        "http://example.org:6379/", "http://example.org:65535/", "http://example.org:0/", "http://example.org:81/", "https://example.org:444/",
      ],
    ],
    [
      "un nome interno",
      [
        "http://localhost/", "http://LOCALHOST/", "http://localhost./", "http://foo.localhost/", "http://a.b.localhost./", "http://intranet/", "http://gateway/x.png",
        "http://agent/", "http://schermo/", "http://telefono/", "https://metadata/",
      ],
    ],
    [
      "un indirizzo IPv4 che non è pubblico, in ogni modo di scriverlo",
      [
        "http://127.0.0.1/", "http://127.0.0.1:80/", "http://127.1/", "http://127.0.1/", "http://0x7f.0.0.1/", "http://0x7f000001/", "http://0177.0.0.1/",
        "http://017700000001/", "http://2130706433/", "http://%31%32%37.0.0.1/", "http://0.0.0.0/", "http://0/", "http://10.0.0.1/", "http://10.1.2.3:80/x.png",
        "http://172.16.0.1/", "http://172.31.255.255/", "http://192.168.1.1/", "http://169.254.169.254/latest/meta-data/", "http://169.254.169.254./",
        "http://2852039166/", "http://0xa9fea9fe/", "http://100.64.0.1/", "http://100.100.100.200/", "http://198.18.0.1/", "http://224.0.0.1/", "http://255.255.255.255/",
        "http://192.0.0.192/",
      ],
    ],
    [
      "un indirizzo IPv6 che non è pubblico, in ogni modo di scriverlo",
      [
        "http://[::1]/", "http://[::]/", "http://[0:0:0:0:0:0:0:1]/", "http://[::ffff:127.0.0.1]/", "http://[::ffff:7f00:1]/", "http://[::ffff:a9fe:a9fe]/",
        "http://[::ffff:8.8.8.8]/", "http://[::ffff:0:127.0.0.1]/", "http://[::ffff:0:7f00:1]/", "http://[::ffff:0:a9fe:a9fe]/", "http://[::ffff:0:8.8.8.8]/",
        "http://[fe80::1]/", "http://[FE80::1]/", "http://[fd00::1]/", "http://[fc00::1]:80/", "http://[ff02::1]/",
        "http://[64:ff9b::7f00:1]/", "http://[2002:7f00:1::]/", "http://[::127.0.0.1]/", "https://[2001:db8::1]/", "http://[fec0::1]/",
      ],
    ],
  ];
  for (const [nome, elenco] of rifiuti) {
    it.each(elenco)(`${nome}: %j è rifiutato`, (testo) => {
      expect(() => controllaUrl(testo)).toThrow(ErroreRete);
      try {
        controllaUrl(testo);
      } catch (e) {
        expect((e as ErroreRete).motivo).toBe("non_ammesso");
        expect((e as Error).message).toBe("Indirizzo non ammesso");
      }
    });
  }

  it.each([
    "", "   ", "non è un indirizzo", "example.org/x.png", "//example.org/x.png", "http://", "https://exa mple.org/", "http://example.org/\nx", "http://example.org/\tx",
    "http://example.org/\u0000", "http://exa\u0085mple.org/", "http://example.org/" + "a".repeat(2100), "http://[::1", "http://[zz]/", "http://1.2.3.256/", "http://999999999999/",
  ])("%j non è un indirizzo valido", (testo) => {
    const e = (() => {
      try {
        controllaUrl(testo);
      } catch (x) {
        return x;
      }
      return null;
    })();
    expect(e).toBeInstanceOf(ErroreRete);
    expect(["non_valido", "non_ammesso"]).toContain((e as ErroreRete).motivo);
  });

  it.each([null, undefined, 5, {}, [], true])("%j non è un testo", (x) => {
    expect(() => controllaUrl(x)).toThrow("Indirizzo non valido");
  });

  it("le regole sugli indirizzi e sulle porte si possono cambiare solo passandole: senza, valgono quelle vere", () => {
    expect(() => controllaUrl("http://127.0.0.1:8080/")).toThrow(ErroreRete);
    expect(controllaUrl("http://127.0.0.1:8080/", (ip) => ip === "127.0.0.1", () => true).port).toBe("8080");
  });
});

describe("a collegamento fatto si controlla di nuovo", () => {
  it("l'indirizzo dall'altra parte deve essere uno di quelli controllati, e ancora pubblico", () => {
    const buoni = new Set(["8.8.8.8", "2606:4700:4700::1111"]);
    expect(() => controllaCollegato({ remoteAddress: "8.8.8.8" }, buoni)).not.toThrow();
    // lo stesso IPv6 scritto per esteso
    expect(() => controllaCollegato({ remoteAddress: "2606:4700:4700:0:0:0:0:1111" }, buoni)).not.toThrow();
    for (const dopo of ["10.0.0.5", "127.0.0.1", "169.254.169.254", "::1", "::ffff:8.8.8.8", "1.1.1.1", "", undefined]) {
      expect(() => controllaCollegato({ remoteAddress: dopo }, buoni), String(dopo)).toThrow("Indirizzo non ammesso");
    }
    // nemmeno un indirizzo controllato prima, se nel frattempo le regole lo vietano
    expect(() => controllaCollegato({ remoteAddress: "8.8.8.8" }, buoni, () => false)).toThrow("Indirizzo non ammesso");
  });
});

describe("lo scarico", () => {
  let s: ServerProva;
  let intestazioni: Record<string, string | string[] | undefined> = {};
  let mezzoGiro = 0;
  let inviatiEnorme = 0;
  const PNG = png();
  const esatto = Buffer.concat([PNG, Buffer.alloc(LIMITE_ACCETTATO - PNG.length)]);

  beforeAll(async () => {
    s = await avviaServerProva({
      "/png": risposta(PNG, "text/html"),
      "/vuoto": risposta("", "image/png", 204),
      "/e404": risposta("non c'è", "text/plain", 404),
      "/e500": risposta("guasto", "text/plain", 500),
      "/a-404": rimanda("/e404"),
      "/r1": rimanda("/r2"),
      "/r2": rimanda("/r3"),
      "/r3": rimanda("/png"),
      "/q1": rimanda("/q2"),
      "/q2": rimanda("/q3"),
      "/q3": rimanda("/q4"),
      "/q4": rimanda("/png"),
      "/giro": rimanda("/giro"),
      "/senza-dove": (_, res) => void res.writeHead(302, { "Content-Length": "0" }).end(),
      "/assoluto": (req, res) => void res.writeHead(302, { Location: `http://${req.headers.host}/png`, "Content-Length": "0" }).end(),
      "/a-privato": rimanda("http://10.0.0.5/logo.png"),
      "/a-metadati": rimanda("http://169.254.169.254/latest/meta-data/"),
      "/a-loopback6": rimanda("http://[::1]/x.png"),
      "/a-mappato": rimanda("http://[::ffff:7f00:1]/x.png"),
      "/a-decimale": rimanda("http://2852039166/x.png"),
      "/a-localhost": rimanda("http://localhost/x.png"),
      "/a-nome-interno": rimanda("http://interno.test/x.png"),
      "/a-ftp": rimanda("ftp://example.org/x.png"),
      "/a-file": rimanda("file:///etc/passwd"),
      "/a-javascript": rimanda("javascript:alert(1)"),
      "/a-dati": rimanda("data:text/html,<script>alert(1)</script>"),
      "/a-credenziali": rimanda("http://utente:segreta@example.org/x.png"),
      "/a-porta": rimanda("http://example.org:6379/"),
      "/a-rotto": rimanda("http://[::1"),
      "/rebind": (req, res) => void res.writeHead(302, { Location: `http://rebind.test:${new URL("http://" + req.headers.host).port}/png`, "Content-Length": "0" }).end(),
      "/grande": risposta(Buffer.alloc(600 * 1024, 1)),
      "/esatto": risposta(esatto),
      "/uno-oltre": risposta(Buffer.concat([esatto, Buffer.from([1])])),
      "/due-mega": risposta(Buffer.alloc(2 * 1024 * 1024, 1)),
      "/a-pezzi": (_, res) => {
        res.writeHead(200, { "Content-Type": "image/png" });
        for (let i = 0; i < 11; i++) res.write(Buffer.alloc(64 * 1024, 2));
        res.end();
      },
      "/enorme": (_, res) => {
        res.writeHead(200, { "Content-Type": "image/png" });
        const blocco = Buffer.alloc(64 * 1024, 3);
        // scrive quanto il collegamento accetta (si ferma quando i buffer sono pieni e riparte a `drain`): quello che manda è poco più di quello che il cliente legge
        const scrivi = () => {
          while (!res.destroyed) {
            inviatiEnorme += blocco.length;
            if (!res.write(blocco)) return void res.once("drain", scrivi);
          }
        };
        scrivi();
      },
      "/appeso": () => {},
      "/goccia": (_, res) => {
        res.writeHead(200, { "Content-Type": "image/png" });
        const t = setInterval(() => res.write("x"), 30);
        res.on("close", () => clearInterval(t));
      },
      "/echo": (req, res) => {
        intestazioni = req.headers;
        res.writeHead(200, { "Content-Type": "image/png", "Content-Length": String(PNG.length) }).end(PNG);
      },
      "/gzip": (_, res) => void res.writeHead(200, { "Content-Type": "image/png", "Content-Encoding": "gzip" }).end(PNG),
      "/mezzo-giro": (_, res) => {
        mezzoGiro++;
        res.writeHead(200, { "Content-Length": "1000", "Content-Type": "image/png" });
        res.write(PNG);
        setTimeout(() => res.destroy(), 30);
      },
    });
  });
  afterAll(() => s.chiudi());

  describe("con le regole vere", () => {
    it("un server su 127.0.0.1 non si raggiunge: nessuna richiesta parte", async () => {
      const prima = s.richieste.length;
      // la porta del server di prova non è 80 né 443
      expect((await rifiutato(scaricaSicuro(s.url("/png")))).message).toBe("Indirizzo non ammesso");
      // aperta la porta, 127.0.0.1 resta vietato
      const e = await rifiutato(scaricaSicuro(s.url("/png"), { ammettiPorta: () => true }));
      expect(e.motivo).toBe("non_ammesso");
      expect(s.richieste.length).toBe(prima);
    });

    it("nessuna variabile d'ambiente cambia le regole", async () => {
      const chiavi = ["OE_LOGHI_PROVA", "OE_PROVA", "LOGHI_PROVA", "OE_RETE_PROVA", "OE_LOGHI_AMMETTI", "ALLOW_LOOPBACK", "NODE_ENV", "OE_DATI"];
      const salvate = Object.fromEntries(chiavi.map((k) => [k, process.env[k]]));
      try {
        for (const k of chiavi) if (k !== "OE_DATI") process.env[k] = k === "NODE_ENV" ? "development" : "1";
        const prima = s.richieste.length;
        await rifiutato(scaricaSicuro(s.url("/png"), { ammettiPorta: () => true }));
        expect(s.richieste.length).toBe(prima);
      } finally {
        for (const k of chiavi) {
          if (salvate[k] === undefined) delete process.env[k];
          else process.env[k] = salvate[k];
        }
      }
    });

    it("il modulo non legge l'ambiente, e i soli due punti che scaricano (loghi, calendari da link) non passano altro che il punto di prova", () => {
      expect(fs.readFileSync(path.join(SORGENTI, "lib/core/rete-sicura.ts"), "utf-8")).not.toMatch(/process\.env/);
      const chiamate: string[] = [];
      const visita = (cartella: string) => {
        for (const voce of fs.readdirSync(cartella, { withFileTypes: true })) {
          const p = path.join(cartella, voce.name);
          if (voce.isDirectory()) visita(p);
          else if (/\.tsx?$/.test(voce.name)) {
            for (const riga of fs.readFileSync(p, "utf-8").split("\n")) if (/\bscaricaSicuro\(/.test(riga) && !/export async function scaricaSicuro/.test(riga)) chiamate.push(`${path.relative(SORGENTI, p).replace(/\\/g, "/")}: ${riga.trim()}`);
          }
        }
      };
      visita(SORGENTI);
      expect(chiamate.sort()).toEqual([
        "lib/core/ics.ts: r = await posti().esegui(() => scaricaSicuro(url, reteDiProva(), OPZIONI_ICS));",
        "lib/core/loghi.ts: const r = await scaricaSicuro(url, reteDiProva());",
      ]);
    });

    it("un nome che porta a un indirizzo non pubblico (localhost, o un nome che risolve a 127.0.0.1) si rifiuta prima di collegarsi", async () => {
      const prima = s.richieste.length;
      await rifiutato(scaricaSicuro(`http://localhost:${s.porta}/png`, { ammettiPorta: () => true }));
      await rifiutato(scaricaSicuro(`http://127.0.0.1.nip.io:${s.porta}/png`, { ammettiPorta: () => true, risolvi: async () => ["127.0.0.1"] }));
      expect(s.richieste.length).toBe(prima);
    });
  });

  describe("con il server di prova raggiungibile", () => {
    it("scarica i byte e dice dove è arrivato; il tipo dichiarato dal sito è solo quello che dice", async () => {
      const r = await scaricaSicuro(s.url("/png"), locale());
      expect(r.dati.equals(PNG)).toBe(true);
      expect(r.tipoDichiarato).toBe("text/html");
      expect(r.url.href).toBe(s.url("/png"));
    });

    it("un nome risolto dal risolutore si raggiunge, ma solo all'indirizzo controllato", async () => {
      const visti: string[] = [];
      const r = await scaricaSicuro(`http://logo.prova.test:${s.porta}/png`, locale({ risolvi: async (h) => (visti.push(h), ["127.0.0.1"]) }));
      expect(r.dati.length).toBe(PNG.length);
      expect(visti).toEqual(["logo.prova.test"]);
    });

    it("manda solo quello che serve: niente cookie, niente credenziali, niente compressione", async () => {
      await scaricaSicuro(s.url("/echo"), locale());
      expect(intestazioni["accept-encoding"]).toBe("identity");
      // chiede solo i formati che accetta (niente image/avif né image/*)
      expect(String(intestazioni.accept)).toContain("image/webp");
      expect(String(intestazioni.accept)).not.toMatch(/avif|image\/\*/);
      expect(intestazioni["user-agent"]).toBe("OECopilot/1.0");
      for (const h of ["cookie", "authorization", "referer", "origin", "x-forwarded-for"]) expect(intestazioni[h], h).toBeUndefined();
      expect(intestazioni.connection).toBe("close");
    });

    it("quello che arriva compresso non si decomprime: sono i byte come sono", async () => {
      const r = await scaricaSicuro(s.url("/gzip"), locale());
      expect(r.dati.equals(PNG)).toBe(true);
    });

    it("segue al più 3 reindirizzamenti", async () => {
      expect(MAX_REINDIRIZZAMENTI).toBe(3);
      const r = await scaricaSicuro(s.url("/r1"), locale());
      expect(r.url.pathname).toBe("/png");
      expect(r.dati.length).toBe(PNG.length);
      const prima = s.richieste.length;
      expect((await rifiutato(scaricaSicuro(s.url("/q1"), locale()))).motivo).toBe("reindirizzamenti");
      expect(s.richieste.slice(prima)).toEqual(["/q1", "/q2", "/q3", "/q4"]);
      expect((await rifiutato(scaricaSicuro(s.url("/giro"), locale()))).message).toBe("Troppi reindirizzamenti (massimo 3)");
    });

    it("un reindirizzamento con un indirizzo assoluto o relativo si segue", async () => {
      expect((await scaricaSicuro(s.url("/assoluto"), locale())).dati.length).toBe(PNG.length);
    });

    it("un reindirizzamento senza dove, o verso un indirizzo rotto, è un errore", async () => {
      expect((await rifiutato(scaricaSicuro(s.url("/senza-dove"), locale()))).motivo).toBe("errore_http");
      expect((await rifiutato(scaricaSicuro(s.url("/a-rotto"), locale()))).motivo).toBe("non_valido");
    });

    it.each([
      ["/a-privato", "10.0.0.5"],
      ["/a-metadati", "169.254.169.254"],
      ["/a-loopback6", "[::1]"],
      ["/a-mappato", "[::ffff:7f00:1]"],
      ["/a-decimale", "2852039166, i metadati scritti come numero"],
      ["/a-localhost", "localhost"],
      ["/a-ftp", "ftp"],
      ["/a-file", "file"],
      ["/a-javascript", "javascript"],
      ["/a-dati", "data"],
      ["/a-credenziali", "credenziali"],
      ["/a-porta", "porta 6379"],
    ])("un reindirizzamento %s (%s) si ferma alla tappa, senza collegarsi", async (percorso) => {
      const prima = s.richieste.length;
      // le regole vere sugli indirizzi (con 127.0.0.1 per il server di prova) e sulle porte (80, 443 e quella del server di prova)
      const e = await rifiutato(scaricaSicuro(s.url(percorso), locale({ ammettiPorta: (p) => p === s.porta || portaAmmessa(p) })));
      expect(e.message).toBe("Indirizzo non ammesso");
      expect(s.richieste.slice(prima)).toEqual([percorso]);
    });

    it("un nome che cambia indirizzo tra una tappa e l'altra (DNS rebinding) si ferma alla seconda", async () => {
      let chiamate = 0;
      const risolvi = async (host: string) => {
        if (host !== "rebind.test") return [host];
        chiamate++;
        // la prima volta il server di prova, la seconda un indirizzo della rete interna
        return chiamate === 1 ? ["127.0.0.1"] : ["10.0.0.5"];
      };
      const prima = s.richieste.length;
      const e = await rifiutato(scaricaSicuro(`http://rebind.test:${s.porta}/rebind`, locale({ risolvi })));
      expect(e.message).toBe("L'indirizzo non risponde");
      expect(chiamate).toBe(2);
      expect(s.richieste.slice(prima)).toEqual(["/rebind"]);
    });

    it("se anche uno solo degli indirizzi di un nome non è pubblico, il nome si rifiuta", async () => {
      for (const indirizzi of [["8.8.8.8", "10.0.0.5"], ["10.0.0.5", "8.8.8.8"], ["8.8.8.8", "169.254.169.254"], ["127.0.0.1", "::1"], ["2606:4700:4700::1111", "fe80::1"], []]) {
        const prima = s.richieste.length;
        const e = await rifiutato(scaricaSicuro(`http://misto.test:${s.porta}/png`, locale({ risolvi: async () => indirizzi, ammettiIndirizzo: indirizzoAmmesso })));
        expect(e.motivo, indirizzi.join(",")).toBe("non_risponde");
        expect(s.richieste.length).toBe(prima);
      }
    });

    it("un nome che porta alla rete interna risponde come un nome che non esiste: dal testo non si ricava quali nomi interni esistono", async () => {
      const nonEsiste = Object.assign(new Error("getaddrinfo ENOTFOUND x.test"), { code: "ENOTFOUND" });
      const nessuno = await rifiutato(scaricaSicuro("http://nessuno.test/x.png", locale({ risolvi: () => Promise.reject(nonEsiste) })));
      for (const risposta of [["10.0.0.5"], ["169.254.169.254"], ["127.0.0.1"], ["::1"], ["fe80::1"], ["::ffff:0:7f00:1"], ["8.8.8.8", "10.0.0.5"]]) {
        const e = await rifiutato(scaricaSicuro("http://interno.test/x.png", locale({ risolvi: async () => risposta, ammettiIndirizzo: indirizzoAmmesso })));
        expect(e.message, risposta.join(",")).toBe(nessuno.message);
        expect(e.motivo, risposta.join(",")).toBe(nessuno.motivo);
      }
      // anche a una tappa di reindirizzamento: un indirizzo scritto è 'non ammesso' (lo ha scelto chi lo manda), un nome no
      const prima = s.richieste.length;
      const tappa = await rifiutato(scaricaSicuro(s.url("/a-nome-interno"), locale({ risolvi: async (h) => (h === "interno.test" ? ["10.0.0.5"] : [h]), ammettiIndirizzo: (ip) => ip === "127.0.0.1" || indirizzoAmmesso(ip) })));
      expect(tappa.message).toBe(nessuno.message);
      expect(tappa.motivo).toBe("non_risponde");
      expect(s.richieste.slice(prima)).toEqual(["/a-nome-interno"]);
    });

    it("un nome che non si risolve, o un server che rifiuta il collegamento, è 'non risponde'", async () => {
      const nonEsiste = Object.assign(new Error("getaddrinfo ENOTFOUND nessuno.test"), { code: "ENOTFOUND" });
      expect((await rifiutato(scaricaSicuro("http://nessuno.test/x.png", locale({ risolvi: async () => Promise.reject(nonEsiste) })))).message).toBe("L'indirizzo non risponde");
      // una porta su cui nessuno ascolta
      const libera = await avviaServerProva({});
      const porta = libera.porta;
      await libera.chiudi();
      const e = await rifiutato(scaricaSicuro(`http://127.0.0.1:${porta}/x.png`, locale()));
      expect(e.message).toBe("L'indirizzo non risponde");
    });

    it("la stessa regola sull'indirizzo a collegamento fatto: se il controllo cambia idea, si ferma", async () => {
      let volte = 0;
      // la prima volta (al momento di risolvere) 127.0.0.1 va bene, la seconda (a collegamento fatto) no
      const ammetti = (ip: string) => (ip === "127.0.0.1" ? ++volte === 1 : indirizzoAmmesso(ip));
      const e = await rifiutato(scaricaSicuro(`http://tardi.test:${s.porta}/png`, locale({ ammettiIndirizzo: ammetti })));
      expect(e.message).toBe("Indirizzo non ammesso");
      expect(volte).toBeGreaterThanOrEqual(2);
    });

    it("uno stato di errore è un errore, con il numero", async () => {
      expect((await rifiutato(scaricaSicuro(s.url("/e404"), locale()))).message).toBe("L'indirizzo risponde con un errore (404)");
      expect((await rifiutato(scaricaSicuro(s.url("/e500"), locale()))).message).toBe("L'indirizzo risponde con un errore (500)");
      expect((await rifiutato(scaricaSicuro(s.url("/nessuna-rotta"), locale()))).motivo).toBe("errore_http");
      // 204: va bene lo stato, il contenuto è vuoto e lo rifiuta chi guarda i byte
      expect((await scaricaSicuro(s.url("/vuoto"), locale())).dati.length).toBe(0);
    });

    it("una risposta che si interrompe a metà non è un'immagine intera", async () => {
      const e = await rifiutato(scaricaSicuro(s.url("/mezzo-giro"), locale()));
      expect(e.motivo).toBe("non_risponde");
      expect(mezzoGiro).toBe(1);
    });

    describe("i limiti di grandezza", () => {
      it("512 KB si accettano, un byte in più no", async () => {
        expect(LIMITE_ACCETTATO).toBe(512 * 1024);
        expect((await scaricaSicuro(s.url("/esatto"), locale())).dati.length).toBe(512 * 1024);
        expect((await rifiutato(scaricaSicuro(s.url("/uno-oltre"), locale()))).message).toBe("Immagine troppo grande (massimo 512 KB)");
        expect((await rifiutato(scaricaSicuro(s.url("/grande"), locale()))).motivo).toBe("troppo_grande");
      });

      it("con più di 1 MB dichiarato non si legge niente; senza lunghezza si legge al più 1 MB", async () => {
        expect(LIMITE_LETTURA).toBe(1024 * 1024);
        expect((await rifiutato(scaricaSicuro(s.url("/due-mega"), locale()))).motivo).toBe("troppo_grande");
        // 704 KB a pezzi, senza Content-Length: si legge tutto e poi si rifiuta perché oltre 512 KB
        expect((await rifiutato(scaricaSicuro(s.url("/a-pezzi"), locale()))).message).toBe("Immagine troppo grande (massimo 512 KB)");
      });

      it("un flusso che non finisce mai si interrompe: il server non riesce a mandarne più di una manciata di MB", async () => {
        inviatiEnorme = 0;
        const t0 = Date.now();
        const e = await rifiutato(scaricaSicuro(s.url("/enorme"), locale({ attesaMs: 8000 })));
        expect(e.message).toBe("Immagine troppo grande (massimo 512 KB)");
        expect(Date.now() - t0).toBeLessThan(5000);
        // il cliente ha letto al più 1 MB più un pezzo; quello che il server ha affidato ai buffer del sistema è poco di più
        await new Promise((ok) => setTimeout(ok, 200));
        const dopoUnPoco = inviatiEnorme;
        expect(dopoUnPoco).toBeGreaterThanOrEqual(LIMITE_LETTURA);
        expect(dopoUnPoco).toBeLessThan(10 * 1024 * 1024);
        // e dopo l'interruzione non ne manda più
        await new Promise((ok) => setTimeout(ok, 200));
        expect(inviatiEnorme).toBe(dopoUnPoco);
      });

      it("il limite per un chiamante che ne accetta di più (la pagina da cui prendere un'icona) arriva a 1 MB", async () => {
        const r = await scaricaSicuro(s.url("/a-pezzi"), locale(), LIMITE_LETTURA);
        expect(r.dati.length).toBe(11 * 64 * 1024);
      });
    });

    describe("Opzioni: quello che cambia per chi non scarica loghi (il calendario da link)", () => {
      const DUE_MB = 2 * 1024 * 1024;

      it("senza opzioni valgono quelli dei loghi (8 secondi, 1 MB letto, 512 KB accettati, il testo delle immagini), anche con un oggetto vuoto", async () => {
        expect(ATTESA_MS).toBe(8000);
        expect(LIMITE_LETTURA).toBe(1024 * 1024);
        expect(LIMITE_ACCETTATO).toBe(512 * 1024);
        expect((await rifiutato(scaricaSicuro(s.url("/grande"), locale()))).message).toBe(TESTI.troppoGrande);
        expect((await rifiutato(scaricaSicuro(s.url("/grande"), locale(), {}))).message).toBe(TESTI.troppoGrande);
        expect((await rifiutato(scaricaSicuro(s.url("/due-mega"), locale(), {}))).motivo).toBe("troppo_grande");
      });

      it("il limite letto e quello accettato si cambiano, e con loro il testo dell'errore", async () => {
        const r = await scaricaSicuro(s.url("/due-mega"), locale(), { limiteLettura: DUE_MB, limiteAccettato: DUE_MB });
        expect(r.dati.length).toBe(DUE_MB);
        // letto fin dove si dice, accettato un byte meno
        const e = await rifiutato(scaricaSicuro(s.url("/due-mega"), locale(), { limiteLettura: DUE_MB, limiteAccettato: DUE_MB - 1, testoTroppoGrande: "Il calendario è troppo grande" }));
        expect(e.motivo).toBe("troppo_grande");
        expect(e.message).toBe("Il calendario è troppo grande");
        // dichiarato oltre quello che si legge: non si legge niente
        expect((await rifiutato(scaricaSicuro(s.url("/due-mega"), locale(), { limiteLettura: DUE_MB - 1, limiteAccettato: DUE_MB }))).message).toBe(TESTI.troppoGrande);
      });

      it("Accept e User-Agent si cambiano; Accept-Encoding resta identity e nessuna credenziale parte", async () => {
        await scaricaSicuro(s.url("/echo"), locale(), { accept: "text/calendar", userAgent: "Prova/2.0" });
        expect(intestazioni.accept).toBe("text/calendar");
        expect(intestazioni["user-agent"]).toBe("Prova/2.0");
        expect(intestazioni["accept-encoding"]).toBe("identity");
        for (const h of ["cookie", "authorization", "referer", "origin"]) expect(intestazioni[h], h).toBeUndefined();
      });

      it("il tempo totale si cambia; quello di una prova, se c'è, vince", async () => {
        const t0 = Date.now();
        const e = await rifiutato(scaricaSicuro(s.url("/appeso"), locale({ attesaMs: undefined }), { attesaMs: 300 }));
        expect(e.motivo).toBe("non_risponde");
        expect(Date.now() - t0).toBeLessThan(2500);
        const u0 = Date.now();
        await rifiutato(scaricaSicuro(s.url("/appeso"), locale({ attesaMs: 300 }), { attesaMs: 60_000 }));
        expect(Date.now() - u0).toBeLessThan(2500);
      });

      it("uno stato HTTP porta anche lo stato, la frase e l'indirizzo dell'ultima tappa; il testo per la pagina non cambia e gli altri errori non hanno il dettaglio", async () => {
        const e = await rifiutato(scaricaSicuro(s.url("/a-404"), locale()));
        expect(e.motivo).toBe("errore_http");
        expect(e.message).toBe("L'indirizzo risponde con un errore (404)");
        expect(e.http).toEqual({ stato: 404, testo: "Not Found", url: s.url("/e404") });
        expect((await rifiutato(scaricaSicuro(s.url("/grande"), locale()))).http).toBeUndefined();
        expect((await rifiutato(scaricaSicuro("http://10.0.0.5/x.png", locale()))).http).toBeUndefined();
      });

      it("le regole sugli indirizzi, sulle porte e sui reindirizzamenti non si cambiano con le opzioni", async () => {
        const larghe: Opzioni = { limiteLettura: DUE_MB, limiteAccettato: DUE_MB, attesaMs: 60_000, accept: "*/*" };
        expect((await rifiutato(scaricaSicuro("http://169.254.169.254/latest/meta-data/", undefined, larghe))).motivo).toBe("non_ammesso");
        expect((await rifiutato(scaricaSicuro("http://calendari.example.org:8080/c.ics", undefined, larghe))).motivo).toBe("non_ammesso");
        expect((await rifiutato(scaricaSicuro(s.url("/a-metadati"), locale(), larghe))).motivo).toBe("non_ammesso");
        expect((await rifiutato(scaricaSicuro(s.url("/q1"), locale(), larghe))).motivo).toBe("reindirizzamenti");
      });
    });

    describe("il tempo", () => {
      it("la risposta che non arriva si interrompe alla scadenza", async () => {
        const t0 = Date.now();
        const e = await rifiutato(scaricaSicuro(s.url("/appeso"), locale({ attesaMs: 300 })));
        expect(e.message).toBe("L'indirizzo non risponde");
        expect(Date.now() - t0).toBeGreaterThanOrEqual(250);
        expect(Date.now() - t0).toBeLessThan(2500);
      });

      it("la scadenza è una sola per tutto, anche per un corpo a gocce che non resta mai fermo", async () => {
        const t0 = Date.now();
        const e = await rifiutato(scaricaSicuro(s.url("/goccia"), locale({ attesaMs: 400 })));
        expect(e.motivo).toBe("non_risponde");
        expect(Date.now() - t0).toBeLessThan(2500);
      });

      it("la scadenza copre anche il nome che non si risolve mai", async () => {
        const t0 = Date.now();
        const e = await rifiutato(scaricaSicuro("http://lento.test/x.png", locale({ attesaMs: 300, risolvi: () => new Promise(() => {}) })));
        expect(e.motivo).toBe("non_risponde");
        expect(Date.now() - t0).toBeLessThan(2500);
      });

      it("senza indicarla vale 8 secondi, per tutto lo scarico", { timeout: 20000 }, async () => {
        expect(ATTESA_MS).toBe(8000);
        const t0 = Date.now();
        const { attesaMs: _, ...senzaTempo } = locale();
        const e = await rifiutato(scaricaSicuro(s.url("/appeso"), senzaTempo));
        const trascorso = Date.now() - t0;
        expect(e.message).toBe("L'indirizzo non risponde");
        expect(trascorso).toBeGreaterThanOrEqual(7800);
        expect(trascorso).toBeLessThan(10500);
      });
    });
  });
});

// ---- HTTPS: il certificato si controlla, e con la sua CA si passa -------------------------------------------------------------
function certificatoDiProva(): { key: string; cert: string } | null {
  const cartella = fs.mkdtempSync(path.join(os.tmpdir(), "oe-tls-"));
  try {
    const k = path.join(cartella, "k.pem");
    const c = path.join(cartella, "c.pem");
    execFileSync(
      "openssl",
      ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", k, "-out", c, "-days", "2", "-subj", "/CN=prova.test", "-addext", "subjectAltName=DNS:prova.test"],
      { stdio: "ignore", timeout: 30000 },
    );
    return { key: fs.readFileSync(k, "utf-8"), cert: fs.readFileSync(c, "utf-8") };
  } catch {
    return null;
  } finally {
    fs.rmSync(cartella, { recursive: true, force: true });
  }
}

describe("https", () => {
  const credenziali = certificatoDiProva();
  const PNG = png();
  let server: https.Server;
  let porta = 0;

  beforeAll(async () => {
    if (!credenziali) return;
    server = https.createServer(credenziali, (req, res) => {
      if (req.url === "/rimanda-http") return void res.writeHead(302, { Location: "http://10.0.0.5/x.png", "Content-Length": "0" }).end();
      res.writeHead(200, { "Content-Type": "image/png", "Content-Length": String(PNG.length) }).end(PNG);
    });
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
    porta = (server.address() as import("node:net").AddressInfo).port;
  });
  afterAll(() => {
    server?.closeAllConnections();
    server?.close();
  });

  const prove = (extra: SoloProve = {}) => locale({ risolvi: async () => ["127.0.0.1"], ...extra });

  it("con la CA del certificato il nome e il certificato corrispondono e i byte arrivano", async (ctx) => {
    if (!credenziali) return ctx.skip("openssl non c'è: il certificato di prova non si può fare");
    const r = await scaricaSicuro(`https://prova.test:${porta}/x.png`, prove({ ca: credenziali.cert }));
    expect(r.dati.equals(PNG)).toBe(true);
    expect(r.url.protocol).toBe("https:");
  });

  it("senza la CA un certificato che non è di una autorità fidata si rifiuta: la verifica non si spegne mai", async (ctx) => {
    if (!credenziali) return ctx.skip("openssl non c'è");
    const e = await rifiutato(scaricaSicuro(`https://prova.test:${porta}/x.png`, prove()));
    expect(e.motivo).toBe("certificato");
    expect(e.message).toBe("Il certificato del sito non è valido");
  });

  it("un nome diverso da quello del certificato si rifiuta anche con la sua CA", async (ctx) => {
    if (!credenziali) return ctx.skip("openssl non c'è");
    const e = await rifiutato(scaricaSicuro(`https://altro.test:${porta}/x.png`, prove({ ca: credenziali.cert })));
    expect(e.motivo).toBe("certificato");
  });

  it("anche da https un reindirizzamento verso http su un indirizzo interno si ferma", async (ctx) => {
    if (!credenziali) return ctx.skip("openssl non c'è");
    const e = await rifiutato(scaricaSicuro(`https://prova.test:${porta}/rimanda-http`, prove({ ca: credenziali.cert })));
    expect(e.message).toBe("Indirizzo non ammesso");
  });

  it("https su una porta che parla http semplice non risponde", async () => {
    const semplice = await avviaServerProva({ "/png": risposta(PNG) });
    try {
      const e = await rifiutato(scaricaSicuro(`https://127.0.0.1:${semplice.porta}/png`, locale()));
      expect(["non_risponde", "certificato"]).toContain(e.motivo);
    } finally {
      await semplice.chiudi();
    }
  });
});

describe("i testi che la pagina mostra", () => {
  it("sono quelli scelti, in italiano", () => {
    expect(TESTI.nonAmmesso).toBe("Indirizzo non ammesso");
    expect(TESTI.nonRisponde).toBe("L'indirizzo non risponde");
    expect(TESTI.troppoGrande).toBe("Immagine troppo grande (massimo 512 KB)");
    expect(TESTI.nonValido).toBe("Indirizzo non valido");
  });
});
