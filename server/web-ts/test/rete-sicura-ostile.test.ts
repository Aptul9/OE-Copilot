import net from "node:net";
import { Worker } from "node:worker_threads";
import { describe, expect, it } from "vitest";
import { controllaUrl, ErroreRete, indirizzoAmmesso, LIMITE_LETTURA, scaricaSicuro, type SoloProve, TESTI } from "@/lib/core/rete-sicura";

// La guardia contro quello che prova a farle fare un sito ostile o chi scrive un indirizzo: una raccolta di indirizzi scritti in ogni modo
// (scritta da chi ha attaccato la prima versione), le risposte del DNS che portano alla rete interna, e un sito che manda il corpo a un byte
// per volta.

const esito = (f: () => unknown): string => {
  try {
    const u = f() as URL;
    return `ammesso:${u.href}`;
  } catch (e) {
    return e instanceof ErroreRete ? `rifiutato:${e.motivo}` : `ECCEZIONE:${(e as Error).message}`;
  }
};

const DA_RIFIUTARE: string[] = [
  // IPv4 scritto in ogni modo
  "http://127.0.0.1/", "http://2130706433/", "http://0x7f000001/", "http://0x7F.0.0.1/", "http://017700000001/", "http://0177.0.0.1/", "http://0177.0.0.01/",
  "http://127.1/", "http://127.0.1/", "http://0/", "http://0.0.0.0/", "http://0x0/", "http://00/", "http://0000/", "http://127.0.0.1./", "http://127.000.000.001/",
  "http://0x7f.1/", "http://0x7f.0x0.0x0.0x1/", "http://0x7f.0.0.0x1/", "http://127.0.0.0x1/", "http://4294967295/", "http://255.255.255.255/", "http://0xffffffff/",
  "http://127.0.0.1:80/", "http://127.0.0.1:443/", "https://127.0.0.1:443/", "http://127.0.0.1:/",
  // punti e cifre unicode, codifiche
  "http://127\u30020\u30020\u30021/", "http://127\uff0e0\uff0e0\uff0e1/", "http://127\uff610\uff610\uff611/", "http://\uff11\uff12\uff17.\uff10.\uff10.\uff11/",
  "http://\u2460\u2461\u2466.0.0.1/", "http://127.0.0.1\u200b/", "http://127.0.0.1\u00ad/", "http://%31%32%37.0.0.1/", "http://127.0.0.1%2e/", "http://127%2e0%2e0%2e1/",
  "http://%6c%6f%63%61%6c%68%6f%73%74/", "http://localhost%2e/",
  // metadati del cloud e reti private
  "http://169.254.169.254/latest/meta-data/", "http://169.254.169.254/opc/v2/instance/", "http://2852039166/", "http://0xa9fea9fe/", "http://0251.0376.0251.0376/",
  "http://169.254.43518/", "http://[::ffff:169.254.169.254]/", "http://[fd00:ec2::254]/", "http://100.100.100.200/", "http://192.0.0.192/",
  "http://10.0.0.1/", "http://10.1/", "http://172.16.0.1/", "http://172.31.255.255/", "http://192.168.0.1/", "http://100.64.0.1/", "http://198.18.0.1/", "http://224.0.0.1/",
  "http://240.0.0.1/", "http://192.0.2.1/", "http://198.51.100.1/", "http://203.0.113.1/", "http://192.88.99.1/", "http://172.17.0.1/", "http://172.18.0.2/", "http://172.20.0.5/",
  // IPv6
  "http://[::1]/", "http://[0:0:0:0:0:0:0:1]/", "http://[::]/", "http://[::ffff:127.0.0.1]/", "http://[::ffff:7f00:1]/", "http://[0:0:0:0:0:ffff:7f00:1]/",
  "http://[::ffff:0:127.0.0.1]/", "http://[::ffff:0:7f00:1]/", "http://[::ffff:0:a9fe:a9fe]/", "http://[::ffff:0:8.8.8.8]/", "http://[0:0:0:0:ffff:0:7f00:1]/",
  "http://[64:ff9b::7f00:1]/", "http://[64:ff9b::127.0.0.1]/", "http://[64:ff9b:1::7f00:1]/", "http://[2002:7f00:1::]/", "http://[2002:a9fe:a9fe::]/",
  "http://[2001:0:4136:e378:8000:63bf:80ff:fffe]/", "http://[fe80::1]/", "http://[fe80::1%25eth0]/", "http://[fe80::1%eth0]/", "http://[fc00::1]/", "http://[fd12:3456::1]/",
  "http://[::127.0.0.1]/", "http://[100::1]/", "http://[fec0::1]/", "http://[ff02::1]/", "http://[2001:db8::1]/", "http://[::ffff:a9fe:a9fe]/", "http://[::1]:80/",
  "http://[::1]:443/", "https://[::1]/", "http://[::1]:8080/",
  // nomi
  "http://localhost/", "http://LOCALHOST/", "http://localhost./", "http://foo.localhost/", "http://a.b.localhost./", "http://\u24db\u24de\u24d2\u24d0\u24db\u24d7\u24de\u24e2\u24e3/",
  "http://agent/", "http://gateway/", "http://oe-copilot-gateway/", "http://schermo/", "http://caddy-ui/", "http://telefono/", "http://a/", "http://agent./",
  // porte
  "http://example.com:22/", "http://example.com:8080/", "http://example.com:3000/", "http://example.com:0/", "http://example.com:65535/", "https://example.com:8443/",
  "http://example.com:8180/", "http://example.com:5555/", "http://example.com:4445/",
  // credenziali nell'indirizzo, e i modi di nasconderle
  "http://user:pass@example.com/", "http://user@example.com/", "https://u:@example.com/", "http://:p@example.com/", "http://example.com@127.0.0.1/", "http://127.0.0.1#@example.com/",
  "http://127.0.0.1\\@example.com/", "http://127.0.0.1?@example.com/", "http://127.0.0.1/@example.com/", "http://public@127.0.0.1/",
  // schemi
  "file:///etc/passwd", "ftp://example.com/", "gopher://example.com/", "javascript:alert(1)", "data:text/plain,hi", "ws://example.com/", "wss://example.com/",
  "blob:http://example.com/x", "about:blank", "dict://127.0.0.1:11211/", "ldap://127.0.0.1/", "jar:http://example.com!/",
  // barre
  "http:127.0.0.1", "http:/127.0.0.1", "http:\\\\127.0.0.1", "http:///127.0.0.1", "HTTP://127.0.0.1/", "hTtP://127.0.0.1/",
  // spazi e caratteri di controllo
  "http://127.0.0.1 /", "http://exa mple.com/", "http://example.com/\tx", "http://exa\tmple.com/", "http://127.0.0.1\n/", "http://example.com/x\r\nHost: evil",
  "\u0000http://example.com/", "http://example.com/\u0085", "http://example.com/\u007f", "http://127.0.0.1\u00a0/", "http://127.0.0.1\u2028/", "http://exa\u0000mple.com/",
  // senza schema, vuoti, lunghi
  "127.0.0.1", "//127.0.0.1/", "example.com/logo.png", "", "   ", "http://", "http:///", "http://example.com/" + "a".repeat(2100),
];

const DA_AMMETTERE: string[] = [
  "http://example.com/", "https://example.com/logo.png?x=1#f", "http://93.184.216.34/", "https://[2606:4700:4700::1111]/", "http://example.com:80/", "https://example.com:443/",
  "https://example.com:80/", "http://example.com:443/", "http://example.com:080/", "https://www.example.co.uk/a/b.png", "http://xn--bcher-kva.example/",
  "http://b\u00fccher.example/", "http://example.com./", "HTTP://EXAMPLE.COM/", " https://example.com/x.png ", "http://8.8.8.8/", "http://1.1.1.1/",
  "http://example.com\\@127.0.0.1/", "http://example.com#@127.0.0.1/",
];

describe("controllaUrl con le regole vere", () => {
  it.each(DA_RIFIUTARE)("%j si rifiuta", (indirizzo) => {
    expect(esito(() => controllaUrl(indirizzo))).toMatch(/^rifiutato:/);
  });
  it.each(DA_AMMETTERE)("%j si ammette", (indirizzo) => {
    expect(esito(() => controllaUrl(indirizzo))).toMatch(/^ammesso:/);
  });
});

describe("le risposte del DNS che portano alla rete interna, con le regole vere", () => {
  const RISPOSTE: [string, string[]][] = [
    ["loopback", ["127.0.0.1"]],
    ["loopback IPv6", ["::1"]],
    ["non specificato", ["0.0.0.0"]],
    ["non specificato IPv6", ["::"]],
    ["IPv4 mappato", ["::ffff:127.0.0.1"]],
    ["IPv4 mappato in esadecimale", ["::ffff:7f00:1"]],
    ["IPv4 tradotto", ["::ffff:0:127.0.0.1"]],
    ["IPv4 tradotto in esadecimale", ["::ffff:0:7f00:1"]],
    ["IPv4 tradotto verso i metadati", ["::ffff:0:a9fe:a9fe"]],
    ["NAT64", ["64:ff9b::7f00:1"]],
    ["6to4", ["2002:7f00:1::"]],
    ["Teredo", ["2001:0:4136:e378:8000:63bf:80ff:fffe"]],
    ["i metadati del cloud", ["169.254.169.254"]],
    ["rete privata", ["10.0.0.5"]],
    ["un pubblico e poi un loopback", ["8.8.8.8", "127.0.0.1"]],
    ["un loopback e poi un pubblico", ["127.0.0.1", "8.8.8.8"]],
    ["un pubblico e poi un loopback IPv6", ["93.184.216.34", "::1"]],
    ["un pubblico e poi i metadati", ["1.1.1.1", "169.254.169.254"]],
    ["nessuna risposta", []],
    ["un nome al posto dell'indirizzo", ["localhost"]],
    ["un indirizzo corto", ["127.1"]],
    ["un indirizzo in esadecimale", ["0x7f.0.0.1"]],
    ["un indirizzo in decimale", ["2130706433"]],
    ["un indirizzo con uno spazio", [" 127.0.0.1"]],
    ["un IPv6 con la zona", ["fe80::1%eth0"]],
    ["CGNAT", ["100.64.0.9"]],
    ["unique-local", ["fd00::1"]],
    ["una risposta che non è un testo", [undefined as unknown as string]],
    ["una risposta numerica", [2130706433 as unknown as string]],
    ["la rete dei contenitori", ["172.17.0.1"]],
  ];

  it.each(RISPOSTE)("%s: si rifiuta senza collegarsi, e con lo stesso testo di un nome che non esiste", async (_, risposta) => {
    const nonEsiste = Object.assign(new Error("getaddrinfo ENOTFOUND dns-case.example"), { code: "ENOTFOUND" });
    const dopo = (risolvi: SoloProve["risolvi"]) => scaricaSicuro("http://dns-case.example/x.png", { risolvi, attesaMs: 2000 }).then(() => null, (e: unknown) => e as ErroreRete);
    const e = await dopo(async () => risposta);
    const nessuno = await dopo(() => Promise.reject(nonEsiste));
    expect(e).toBeInstanceOf(ErroreRete);
    expect(e?.motivo).toBe("non_risponde");
    expect(e?.message).toBe(TESTI.nonRisponde);
    expect(e?.message).toBe(nessuno?.message);
    expect(e?.motivo).toBe(nessuno?.motivo);
  });

  it("e nessun collegamento parte, nemmeno verso un server che c'è", async () => {
    let connessioni = 0;
    const server = net.createServer((s) => {
      connessioni++;
      s.destroy();
    });
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
    const porta = (server.address() as net.AddressInfo).port;
    try {
      for (const [, risposta] of RISPOSTE) {
        await scaricaSicuro(`http://dns-case.example:${porta}/x.png`, { risolvi: async () => risposta, ammettiPorta: () => true, attesaMs: 2000 }).then(
          () => expect.unreachable("doveva essere rifiutato"),
          (e: unknown) => expect(e).toBeInstanceOf(ErroreRete),
        );
      }
      for (const url of ["http://127.0.0.1/", "http://localhost/", "http://[::1]/", "http://0.0.0.0/", "http://2130706433/", "http://[::ffff:0:127.0.0.1]/"]) {
        await expect(scaricaSicuro(url)).rejects.toBeInstanceOf(ErroreRete);
      }
      expect(connessioni).toBe(0);
    } finally {
      await new Promise((ok) => server.close(ok));
    }
  });

  it("un indirizzo scritto nell'indirizzo resta 'non ammesso': non c'è un nome da nascondere", () => {
    expect(() => controllaUrl("http://10.0.0.5/")).toThrow(TESTI.nonAmmesso);
    expect(() => controllaUrl("http://[::ffff:0:10.0.0.5]/")).toThrow(TESTI.nonAmmesso);
  });
});

describe("un sito che manda il corpo a un byte per volta", () => {
  const PROVA: SoloProve = {
    risolvi: async (host) => (host.endsWith(".test") ? ["127.0.0.1"] : [host]),
    ammettiIndirizzo: (ip) => ip === "127.0.0.1" || indirizzoAmmesso(ip),
    ammettiPorta: () => true,
    attesaMs: 20_000,
  };

  /** Un server che a ogni richiesta risponde con i byte dati e poi, se `chiudi`, chiude. */
  async function server(risposta: Buffer, chiudi = false) {
    const aperti = new Set<net.Socket>();
    const srv = net.createServer((s) => {
      aperti.add(s);
      s.on("error", () => {});
      s.on("close", () => aperti.delete(s));
      s.once("data", () => (chiudi ? s.end(risposta) : s.write(risposta)));
    });
    await new Promise<void>((ok) => srv.listen(0, "127.0.0.1", ok));
    return {
      porta: (srv.address() as net.AddressInfo).port,
      chiudi: () =>
        new Promise<void>((ok) => {
          for (const s of aperti) s.destroy();
          srv.close(() => ok());
        }),
    };
  }

  /** La risposta con il corpo a pezzi (Transfer-Encoding: chunked) da `pezzo` byte l'uno, scritta tutta in un buffer solo. */
  const aPezzi = (corpo: Buffer, pezzo: number) => {
    const testa = Buffer.from("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
    const coda = Buffer.from("0\r\n\r\n");
    const intestazione = (n: number) => Buffer.from(`${n.toString(16)}\r\n`);
    const pezzi = Math.ceil(corpo.length / pezzo);
    const fuori = Buffer.alloc(testa.length + corpo.length + pezzi * (intestazione(pezzo).length + 2) + coda.length);
    let o = testa.copy(fuori);
    for (let i = 0; i < corpo.length; i += pezzo) {
      const n = Math.min(pezzo, corpo.length - i);
      if (pezzo === 1) {
        // un milione di pezzi: niente oggetti intermedi
        fuori[o++] = 0x31;
        fuori[o++] = 0x0d;
        fuori[o++] = 0x0a;
        fuori[o++] = corpo[i];
        fuori[o++] = 0x0d;
        fuori[o++] = 0x0a;
        continue;
      }
      o += intestazione(n).copy(fuori, o);
      o += corpo.copy(fuori, o, i, i + n);
      fuori[o++] = 0x0d;
      fuori[o++] = 0x0a;
    }
    coda.copy(fuori, o);
    return fuori.subarray(0, o + coda.length);
  };

  // un thread a parte guarda la memoria di tutto il processo mentre questo è occupato a ricevere i pezzi
  const campionatore = () => {
    const w = new Worker(
      `const { parentPort } = require("node:worker_threads");
       let massimo = 0;
       const t = setInterval(() => { massimo = Math.max(massimo, process.memoryUsage.rss()); }, 2);
       parentPort.on("message", () => { clearInterval(t); parentPort.postMessage(massimo); });`,
      { eval: true },
    );
    return {
      ferma: () =>
        new Promise<number>((ok) => {
          w.once("message", (m: number) => (void w.terminate(), ok(m)));
          w.postMessage("stop");
        }),
    };
  };

  it("un milione di pezzi da un byte oltre il limite si interrompe, e la memoria cresce di poco (prima di questa prova: oltre 400 MB)", { timeout: 60_000 }, async () => {
    const corpo = Buffer.alloc(LIMITE_LETTURA + 1, 65);
    const s = await server(aPezzi(corpo, 1));
    try {
      await new Promise((ok) => setTimeout(ok, 300));
      const base = process.memoryUsage.rss();
      const c = campionatore();
      const t0 = performance.now();
      const e = await scaricaSicuro(`http://pezzi.test:${s.porta}/x.png`, PROVA).then(() => null, (x: unknown) => x as ErroreRete);
      const ms = performance.now() - t0;
      const picco = await c.ferma();
      expect(e).toBeInstanceOf(ErroreRete);
      expect(e?.motivo).toBe("troppo_grande");
      const crescitaMB = (picco - base) / 1048576;
      expect(crescitaMB, `la memoria è cresciuta di ${crescitaMB.toFixed(0)} MB in ${ms.toFixed(0)} ms`).toBeLessThan(150);
      expect(ms).toBeLessThan(15_000);
    } finally {
      await s.chiudi();
    }
  });

  it("un milione di pezzi da un byte esatti (1 MB) arrivano interi e nello stesso ordine; uno in più è troppo", { timeout: 60_000 }, async () => {
    const corpo = Buffer.alloc(LIMITE_LETTURA);
    for (let i = 0; i < corpo.length; i++) corpo[i] = (i * 7 + (i >> 8)) & 0xff;
    const intero = await server(aPezzi(corpo, 1), true);
    try {
      const r = await scaricaSicuro(`http://pezzi.test:${intero.porta}/x.png`, PROVA, LIMITE_LETTURA);
      expect(r.dati.length).toBe(LIMITE_LETTURA);
      expect(r.dati.equals(corpo)).toBe(true);
    } finally {
      await intero.chiudi();
    }
    const unoInPiu = await server(aPezzi(Buffer.concat([corpo, Buffer.from([1])]), 1), true);
    try {
      const e = await scaricaSicuro(`http://pezzi.test:${unoInPiu.porta}/x.png`, PROVA, LIMITE_LETTURA).then(() => null, (x: unknown) => x as ErroreRete);
      expect(e?.motivo).toBe("troppo_grande");
    } finally {
      await unoInPiu.chiudi();
    }
  });

  it("pezzi di ogni grandezza ricostruiscono lo stesso corpo: 1, 7, 1000, 65536 byte, e un corpo vuoto", async () => {
    const corpo = Buffer.from(Array.from({ length: 200_000 }, (_, i) => (i * 31 + 5) & 0xff));
    for (const pezzo of [1000, 7, 65536, 199_999, 200_000]) {
      const s = await server(aPezzi(pezzo === 7 ? corpo.subarray(0, 30_000) : corpo, pezzo), true);
      try {
        const atteso = pezzo === 7 ? corpo.subarray(0, 30_000) : corpo;
        const r = await scaricaSicuro(`http://pezzi.test:${s.porta}/x.png`, PROVA, LIMITE_LETTURA);
        expect(r.dati.equals(atteso), `pezzi da ${pezzo}`).toBe(true);
      } finally {
        await s.chiudi();
      }
    }
    const vuoto = await server(Buffer.from("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"), true);
    try {
      expect((await scaricaSicuro(`http://pezzi.test:${vuoto.porta}/x.png`, PROVA)).dati.length).toBe(0);
    } finally {
      await vuoto.chiudi();
    }
  });
});
