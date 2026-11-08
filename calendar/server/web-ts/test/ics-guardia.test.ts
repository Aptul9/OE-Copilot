import fs from "node:fs";
import path from "node:path";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { azzeraMemoria } from "@/lib/core/agente";
import { apiTs, chiama } from "@/lib/core/api";
import { chiudiArchivio } from "@/lib/core/archivio";
import { parseBound } from "@/lib/core/fuso";
import { azzeraFeed, ErroreHttp, eventiFeed, feedDi, LIMITE_ICS, OPZIONI_ICS, percorsoCache, salvaCache, scarica, statoFeed, testoCalendario, TTL_S } from "@/lib/core/ics";
import { fissaOra } from "@/lib/core/orologio";
import { FESTIVITA_ID, FESTIVITA_URL } from "@/lib/core/personalizzazione";
import { controllaUrl, portaAmmessa } from "@/lib/core/rete-sicura";
import { loadConfig, modificaConfig } from "@/lib/core/stato";
import { impostaReteDiProva, posti, POSTI_MAX, TROPPE_RICHIESTE } from "@/lib/core/uscita";
import { avviaAgenteFinto, CHIAVE_AGENTE, cartellaDati, type AgenteFinto } from "./agente-condivisione";
import { avviaServerProva, type Gestore, PROVE_LOCALI, rimanda, risposta, type ServerProva } from "./server-prova";

// Il calendario da link (ics.ts) passa dalla stessa guardia dei loghi (rete-sicura.ts): le prove sono quelle degli indirizzi che il gateway non
// raggiunge (ogni classe, scritta in ogni modo), dei reindirizzamenti, dei limiti (8 MiB, 15 secondi), del contenuto (deve cominciare come un
// calendario), della copia su disco e della regola 404 e 410 (che restano quelle di prima), dei posti in comune con i loghi e delle festività.
// I server di prova stanno su 127.0.0.1: si raggiungono solo con impostaReteDiProva, e con le regole vere nessuna richiesta parte (il conto
// delle richieste di un server resta com'era).

const NOME = "Calendario di prova";
const TIPO_ICS = "text/calendar; charset=utf-8";
const ICS = (nome = NOME) => `BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Prova//IT\r\nX-WR-CALNAME:${nome}\r\nEND:VCALENDAR\r\n`;
const ILLEGGIBILE = "Il link risponde ma non contiene un calendario iCal valido. Assicurati di aver copiato l'indirizzo segreto in formato iCal.";
const NON_AMMESSO = { ok: false, error: "Il link non risponde: Indirizzo non ammesso" };
const NON_RISPONDE = { ok: false, error: "Il link non risponde: L'indirizzo non risponde" };
const COPIA_FESTE = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Feste//IT\r\nX-WR-CALNAME:Feste\r\nEND:VCALENDAR\r\n";

/** Un calendario di esattamente `byte` byte (tutti ASCII): una riga di riempimento tra l'inizio e la fine. */
function icsDa(byte: number): Buffer {
  const testa = `BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Prova//IT\r\nX-WR-CALNAME:${NOME}\r\n`;
  const coda = "END:VCALENDAR\r\n";
  const riempimento = byte - testa.length - coda.length - "X-PAD:".length - 2;
  return Buffer.from(`${testa}X-PAD:${"a".repeat(riempimento)}\r\n${coda}`, "latin1");
}

const senzaTipo =
  (corpo: Buffer | string): Gestore =>
  (_, res) =>
    void res.writeHead(200, { "Content-Length": String(Buffer.byteLength(corpo)) }).end(corpo);

const attendi = async (finche: () => boolean, ms = 5000) => {
  const t0 = Date.now();
  while (!finche()) {
    if (Date.now() - t0 > ms) throw new Error("la condizione non si è avverata");
    await new Promise((ok) => setTimeout(ok, 5));
  }
};

const scritture = (agente: AgenteFinto) => agente.richieste.filter((r) => r.startsWith("POST /api/ics"));
const copieSuDisco = (dati: string) => {
  const cartella = path.join(dati, "ics_cache");
  return fs.existsSync(cartella) ? fs.readdirSync(cartella) : [];
};

let dati: string;
let agente: AgenteFinto;

beforeEach(async () => {
  dati = cartellaDati();
  agente = await avviaAgenteFinto([]);
  process.env.OE_DATI = dati;
  process.env.AGENTE_URL = agente.url;
  process.env.AGENTE_CHIAVE = CHIAVE_AGENTE;
  delete process.env.GATEWAY_DB;
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation(() => {});
  azzeraMemoria();
  azzeraFeed();
});

afterEach(async () => {
  vi.unstubAllEnvs();
  impostaReteDiProva(undefined);
  // nessun posto resta occupato da una prova
  expect(posti().inUso).toBe(0);
  expect(posti().inCoda).toBe(0);
  await agente.chiudi();
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  for (const n of ["OE_DATI", "AGENTE_URL", "AGENTE_CHIAVE", "OE_LOGHI_PROVA", "OE_PROVA", "ALLOW_LOOPBACK", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy"]) delete process.env[n];
  vi.restoreAllMocks();
});

// ---- Gli indirizzi che il gateway non raggiunge ---------------------------------------------------------------------------------------
const VIETATI: [string, string[]][] = [
  [
    "loopback",
    [
      "http://127.0.0.1/c.ics", "https://127.0.0.1/c.ics", "http://127.1/c.ics", "http://2130706433/c.ics", "http://0x7f.0.0.1/c.ics", "http://0177.0.0.1/c.ics",
      "http://127.0.0.1./c.ics", "http://0.0.0.0/c.ics", "http://localhost/c.ics", "http://LOCALHOST./c.ics", "http://calendari.localhost/c.ics", "HTTP://127.0.0.1/c.ics",
      "http://\uff11\uff12\uff17.\uff10.\uff10.\uff11/c.ics", "http://%31%32%37.0.0.1/c.ics", "webcal://127.0.0.1/c.ics", "WebCal://localhost/c.ics",
    ],
  ],
  ["reti private", ["http://10.0.0.5/c.ics", "http://10.255.255.254/c.ics", "http://172.16.0.9/c.ics", "http://172.31.255.255/c.ics", "http://192.168.1.10/c.ics", "http://100.64.0.1/c.ics", "http://198.18.0.1/c.ics"]],
  [
    "link-local e metadati del cloud",
    [
      "http://169.254.169.254/latest/meta-data/", "https://169.254.169.254/c.ics", "http://169.254.169.254:80/computeMetadata/v1/", "http://2852039166/latest/meta-data/",
      "http://0xa9fea9fe/latest/meta-data/", "http://100.100.100.200/latest/meta-data/", "http://[fd00:ec2::254]/latest/meta-data/", "webcal://169.254.169.254/latest/meta-data/",
    ],
  ],
  [
    "le reti di Docker e i nomi dei servizi",
    ["http://172.17.0.1/c.ics", "http://172.18.0.2/c.ics", "http://agent/api/ics", "http://agent:8080/c.ics", "http://gateway/c.ics", "http://oe-copilot-gateway/c.ics", "http://caddy-ui/c.ics", "http://schermo/c.ics", "http://agent./c.ics"],
  ],
  [
    "IPv6 e IPv4 dentro IPv6",
    [
      "http://[::1]/c.ics", "http://[::]/c.ics", "http://[::ffff:127.0.0.1]/c.ics", "http://[::ffff:7f00:1]/c.ics", "http://[::ffff:0:127.0.0.1]/c.ics", "http://[::ffff:169.254.169.254]/latest/meta-data/",
      "http://[64:ff9b::a00:1]/c.ics", "http://[2002:a00:1::]/c.ics", "http://[2001:0:4136:e378:8000:63bf:80ff:fffe]/c.ics", "http://[fe80::1]/c.ics", "http://[fc00::1]/c.ics",
      "http://[fd12:3456::1]/c.ics", "http://[ff02::1]/c.ics", "https://[::1]/c.ics",
    ],
  ],
  [
    "credenziali nell'indirizzo",
    ["http://utente:segreto@calendari.example.org/c.ics", "https://utente@calendari.example.org/c.ics", "http://:segreto@calendari.example.org/c.ics", "http://calendari.example.org@127.0.0.1/c.ics", "http://127.0.0.1#@calendari.example.org/c.ics"],
  ],
  [
    "porte che non sono 80 e 443",
    ["http://calendari.example.org:22/c.ics", "http://calendari.example.org:8080/c.ics", "https://calendari.example.org:8443/c.ics", "http://calendari.example.org:6379/c.ics", "https://calendari.example.org:444/c.ics", "http://calendari.example.org:3000/c.ics"],
  ],
];

const SCHEMI = [
  "ftp://calendari.example.org/c.ics", "file:///etc/passwd", "gopher://calendari.example.org/", "data:text/calendar,BEGIN:VCALENDAR%0D%0AEND:VCALENDAR", "javascript:alert(1)",
  "ws://calendari.example.org/c.ics", "wss://calendari.example.org/c.ics", "ldap://127.0.0.1/", "dict://127.0.0.1:11211/", "blob:http://calendari.example.org/x", "",
  "   ", "calendari.example.org/c.ics", "//127.0.0.1/c.ics", "127.0.0.1", "http:127.0.0.1", "mailto:utente@calendari.example.org",
];

describe("add_ics_calendar con le regole vere: gli indirizzi che il gateway non raggiunge", () => {
  let canarino: ServerProva;
  const risolti: string[] = [];

  beforeAll(async () => {
    // un sito su 127.0.0.1 che risponderebbe con un calendario: se la guardia sbagliasse, il conto delle sue richieste lo direbbe
    canarino = await avviaServerProva({ "/c.ics": risposta(ICS(), TIPO_ICS) });
  });
  afterAll(() => canarino.chiudi());
  beforeEach(() => {
    risolti.length = 0;
    // solo il risolutore è della prova (e conta i nomi): le regole sugli indirizzi, sulle porte e sugli schemi sono le vere
    impostaReteDiProva({ risolvi: async (nome) => (risolti.push(nome), []) });
  });

  const nienteDiFatto = () => {
    expect(scritture(agente), "l'agente non doveva essere chiamato").toEqual([]);
    expect(feedDi(loadConfig()), "nessun calendario aggiunto").toEqual([]);
    expect(copieSuDisco(dati), "nessuna copia su disco").toEqual([]);
  };

  it.each(VIETATI)("%s: Indirizzo non ammesso, prima di ogni DNS e di ogni collegamento", async (_classe, indirizzi) => {
    for (const url of indirizzi) {
      expect(await apiTs.add_ics_calendar("Prova", url), url).toEqual(NON_AMMESSO);
    }
    expect(risolti, "nessun nome si risolve").toEqual([]);
    nienteDiFatto();
  });

  it("gli schemi che non sono http e https, e gli indirizzi vuoti o senza schema, hanno il testo di sempre e non fanno niente", async () => {
    for (const url of SCHEMI) {
      expect(await apiTs.add_ics_calendar("Prova", url), JSON.stringify(url)).toEqual({ ok: false, error: "Il link deve iniziare con https:// (o webcal://)." });
    }
    expect(risolti).toEqual([]);
    nienteDiFatto();
  });

  it("un indirizzo scritto male è 'non valido': spazi, caratteri di controllo, più di 2048 caratteri, nessun nome", async () => {
    for (const url of ["http://calendari example.org/c.ics", "http://calendari.example.org/c\tics", "http://calendari.example.org/x\r\nHost: evil", "http://", "http:///", "http://calendari.example.org/" + "a".repeat(2100)]) {
      expect(await apiTs.add_ics_calendar("Prova", url), url.slice(0, 60)).toEqual({ ok: false, error: "Il link non risponde: Indirizzo non valido" });
    }
    expect(risolti).toEqual([]);
    nienteDiFatto();
  });

  it("senza il punto di prova un server su 127.0.0.1 non si raggiunge, nemmeno sulla sua porta e nemmeno con le porte libere: nessuna richiesta parte", async () => {
    const prima = canarino.richieste.length;
    impostaReteDiProva(undefined);
    expect(await apiTs.add_ics_calendar("x", canarino.url("/c.ics"))).toEqual(NON_AMMESSO);
    expect(await apiTs.add_ics_calendar("x", `http://localhost:${canarino.porta}/c.ics`)).toEqual(NON_AMMESSO);
    impostaReteDiProva({ ammettiPorta: () => true });
    expect(await apiTs.add_ics_calendar("x", canarino.url("/c.ics"))).toEqual(NON_AMMESSO);
    await expect(scarica(canarino.url("/c.ics"))).rejects.toMatchObject({ message: "Indirizzo non ammesso" });
    expect(canarino.richieste.length).toBe(prima);
    nienteDiFatto();
  });

  it("nessuna variabile d'ambiente e nessun argomento della richiesta apre la guardia", async () => {
    impostaReteDiProva(undefined);
    const prima = canarino.richieste.length;
    for (const k of ["OE_LOGHI_PROVA", "OE_PROVA", "ALLOW_LOOPBACK", "HTTP_PROXY", "HTTPS_PROXY"]) process.env[k] = "1";
    vi.stubEnv("NODE_ENV", "development");
    expect(await apiTs.add_ics_calendar("x", canarino.url("/c.ics"))).toEqual(NON_AMMESSO);
    // gli argomenti che il gateway non conosce non contano: il metodo prende il nome e l'indirizzo e basta
    const chiamata = await chiama("add_ics_calendar", ["x", canarino.url("/c.ics"), { ammettiIndirizzo: true, soloProve: { ammettiPorta: true }, risolvi: "127.0.0.1" }, "soloProve"]);
    expect(chiamata).toEqual(NON_AMMESSO);
    expect(canarino.richieste.length).toBe(prima);
    nienteDiFatto();
  });

  it("il punto di prova in produzione non si imposta", () => {
    vi.stubEnv("NODE_ENV", "production");
    expect(() => impostaReteDiProva(PROVE_LOCALI)).toThrow("solo per le prove");
  });

  it("un nome che porta alla rete interna risponde come un nome che non esiste: dal testo non si ricava quali nomi interni ci sono", async () => {
    const interni: Record<string, string[]> = {
      "interno.example.org": ["10.0.0.9"],
      "docker.example.org": ["172.18.0.2"],
      "metadati.example.org": ["169.254.169.254"],
      "loopback.example.org": ["127.0.0.1"],
      "v6.example.org": ["fd00::5"],
      // basta un indirizzo non pubblico tra quelli pubblici per rifiutare tutto il nome
      "misto.example.org": ["93.184.216.34", "10.0.0.9"],
    };
    impostaReteDiProva({
      risolvi: async (nome) => {
        risolti.push(nome);
        if (interni[nome]) return interni[nome];
        throw Object.assign(new Error("getaddrinfo ENOTFOUND"), { code: "ENOTFOUND" });
      },
    });
    const inesistente = await apiTs.add_ics_calendar("x", "https://non-esiste.example.org/c.ics");
    expect(inesistente).toEqual(NON_RISPONDE);
    for (const nome of Object.keys(interni)) expect(await apiTs.add_ics_calendar("x", `https://${nome}/c.ics`), nome).toEqual(inesistente);
    expect(risolti).toHaveLength(1 + Object.keys(interni).length);
    nienteDiFatto();
  });
});

// ---- Reindirizzamenti ---------------------------------------------------------------------------------------------------------------------
describe("add_ics_calendar: ogni tappa di un reindirizzamento ricomincia dai controlli", () => {
  let s: ServerProva;

  beforeAll(async () => {
    s = await avviaServerProva({
      "/c.ics": risposta(ICS(), TIPO_ICS),
      "/r1": rimanda("/r2"),
      "/r2": rimanda("/r3"),
      "/r3": rimanda("/c.ics"),
      "/q1": rimanda("/q2"),
      "/q2": rimanda("/q3"),
      "/q3": rimanda("/q4"),
      "/q4": rimanda("/c.ics"),
      "/relativo": rimanda("c.ics"),
      "/a-404": rimanda("/non-c-e.ics"),
      "/a-metadati": rimanda("http://169.254.169.254/latest/meta-data/"),
      "/a-privato": rimanda("http://10.1.2.3/c.ics"),
      "/a-loopback": rimanda("http://127.0.0.2/c.ics"),
      "/a-loopback6": rimanda("http://[::1]/c.ics"),
      "/a-mappato": rimanda("http://[::ffff:7f00:2]/c.ics"),
      "/a-decimale": rimanda("http://2852039166/latest/meta-data/"),
      "/a-localhost": rimanda("http://localhost/c.ics"),
      "/a-docker": rimanda("http://agent/api/ics"),
      "/a-ftp": rimanda("ftp://calendari.example.org/c.ics"),
      "/a-file": rimanda("file:///etc/passwd"),
      "/a-credenziali": rimanda("http://utente:segreto@calendari.example.org/c.ics"),
      "/a-porta": rimanda("http://calendari.example.org:6379/c.ics"),
      "/a-nome-interno": rimanda("http://interno.test/c.ics"),
      "/a-rotto": rimanda("http://[::1"),
    });
  });
  afterAll(() => s.chiudi());
  beforeEach(() => {
    impostaReteDiProva({
      ...PROVE_LOCALI,
      // le porte sono le vere, più quella del server di prova; i nomi .test portano a 127.0.0.1, tranne `interno.test` che porta alla rete interna
      ammettiPorta: (porta) => porta === s.porta || portaAmmessa(porta),
      risolvi: async (nome) => (nome === "interno.test" ? ["10.0.0.9"] : nome.endsWith(".test") ? ["127.0.0.1"] : [nome]),
    });
  });

  it("fino a 3 reindirizzamenti il calendario si legge (anche con un indirizzo relativo), al quarto no", async () => {
    expect(await apiTs.add_ics_calendar("Prova", s.url("/r1"))).toMatchObject({ ok: true, feed: { name: "Prova", url: s.url("/r1") } });
    modificaConfig((c) => void (c.ics_calendars = []));
    expect(await apiTs.add_ics_calendar("Prova", s.url("/relativo"))).toMatchObject({ ok: true });
    modificaConfig((c) => void (c.ics_calendars = []));
    const prima = s.richieste.length;
    expect(await apiTs.add_ics_calendar("Prova", s.url("/q1"))).toEqual({ ok: false, error: "Il link non risponde: Troppi reindirizzamenti (massimo 3)" });
    expect(s.richieste.slice(prima)).toEqual(["/q1", "/q2", "/q3", "/q4"]);
  });

  it.each([
    ["i metadati del cloud", "/a-metadati"],
    ["una rete privata", "/a-privato"],
    ["un altro indirizzo di loopback", "/a-loopback"],
    ["::1", "/a-loopback6"],
    ["un IPv4 dentro IPv6", "/a-mappato"],
    ["i metadati scritti in decimale", "/a-decimale"],
    ["localhost", "/a-localhost"],
    ["un nome di servizio di Docker", "/a-docker"],
    ["uno schema che non è http", "/a-ftp"],
    ["file:", "/a-file"],
    ["delle credenziali nell'indirizzo", "/a-credenziali"],
    ["una porta che non è 80 né 443", "/a-porta"],
  ])("un reindirizzamento verso %s si rifiuta prima di collegarsi: una sola richiesta al sito, niente nella configurazione", async (_nome, percorso) => {
    const prima = s.richieste.length;
    expect(await apiTs.add_ics_calendar("Prova", s.url(percorso))).toEqual(NON_AMMESSO);
    expect(s.richieste.slice(prima)).toEqual([percorso]);
    expect(feedDi(loadConfig())).toEqual([]);
    expect(copieSuDisco(dati)).toEqual([]);
    expect(scritture(agente)).toEqual([]);
  });

  it("un reindirizzamento verso un nome che porta alla rete interna risponde come un nome che non esiste", async () => {
    expect(await apiTs.add_ics_calendar("Prova", s.url("/a-nome-interno"))).toEqual(NON_RISPONDE);
  });

  it("un indirizzo di reindirizzamento rotto è 'non valido'", async () => {
    expect(await apiTs.add_ics_calendar("Prova", s.url("/a-rotto"))).toEqual({ ok: false, error: "Il link non risponde: Indirizzo non valido" });
  });

  it("un errore dopo un reindirizzamento porta l'indirizzo dell'ultima tappa, che è pubblico per forza", async () => {
    expect(await apiTs.add_ics_calendar("Prova", s.url("/a-404"))).toEqual({
      ok: false,
      error: `Il link non risponde: 404 Client Error: Not Found for url: ${s.url("/non-c-e.ics")}`,
    });
  });
});

// ---- Il limite: 8 MiB ---------------------------------------------------------------------------------------------------------------------
describe("il limite di 8 MiB, letti e accettati", () => {
  const TROPPO = `Il calendario è troppo grande (massimo ${LIMITE_ICS / 1024 / 1024} MB)`;
  const esatto = icsDa(LIMITE_ICS);
  const unOltre = icsDa(LIMITE_ICS + 1);
  let s: ServerProva;
  const aPezzi = (corpo: Buffer): Gestore => (_, res) => {
    res.writeHead(200, { "Content-Type": TIPO_ICS });
    for (let i = 0; i < corpo.length; i += 65_536) res.write(corpo.subarray(i, i + 65_536));
    res.end();
  };

  beforeAll(async () => {
    s = await avviaServerProva({
      "/esatto.ics": risposta(esatto, TIPO_ICS),
      "/uno-oltre.ics": risposta(unOltre, TIPO_ICS),
      "/esatto-a-pezzi.ics": aPezzi(esatto),
      "/uno-oltre-a-pezzi.ics": aPezzi(unOltre),
      "/enorme.ics": (_, res) => {
        res.writeHead(200, { "Content-Type": TIPO_ICS });
        const blocco = Buffer.alloc(65_536, 97);
        const scrivi = () => {
          while (!res.destroyed) if (!res.write(blocco)) return void res.once("drain", scrivi);
        };
        scrivi();
      },
    });
  });
  afterAll(() => s.chiudi());
  beforeEach(() => impostaReteDiProva(PROVE_LOCALI));

  it("i numeri: 8 MiB, 15 secondi, quello che si legge e quello che si accetta sono lo stesso", () => {
    expect(LIMITE_ICS).toBe(8 * 1024 * 1024);
    expect(esatto.length).toBe(LIMITE_ICS);
    expect(unOltre.length).toBe(LIMITE_ICS + 1);
    expect(OPZIONI_ICS).toMatchObject({ limiteLettura: 8 * 1024 * 1024, limiteAccettato: 8 * 1024 * 1024, attesaMs: 15_000, userAgent: "CalendarHub/1.0" });
  });

  it("8 MiB esatti si accettano (con la lunghezza dichiarata e a pezzi), un byte in più no, in tutti e due i modi", async () => {
    expect((await scarica(s.url("/esatto.ics"))).length).toBe(LIMITE_ICS);
    expect((await scarica(s.url("/esatto-a-pezzi.ics"))).length).toBe(LIMITE_ICS);
    await expect(scarica(s.url("/uno-oltre.ics"))).rejects.toMatchObject({ message: TROPPO });
    await expect(scarica(s.url("/uno-oltre-a-pezzi.ics"))).rejects.toMatchObject({ message: TROPPO });
  });

  it("add_ics_calendar con 8 MiB esatti aggiunge il calendario; con un byte in più dice il testo della grandezza e non scrive niente", async () => {
    expect(await apiTs.add_ics_calendar("", s.url("/esatto.ics"))).toMatchObject({ ok: true, feed: { name: NOME } });
    expect(fs.statSync(percorsoCache(feedDi(loadConfig())[0].id)).size).toBe(LIMITE_ICS);
    modificaConfig((c) => void (c.ics_calendars = []));
    fs.rmSync(path.join(dati, "ics_cache"), { recursive: true, force: true });
    for (const percorso of ["/uno-oltre.ics", "/uno-oltre-a-pezzi.ics"]) {
      expect(await apiTs.add_ics_calendar("", s.url(percorso)), percorso).toEqual({ ok: false, error: `Il link non risponde: ${TROPPO}` });
    }
    expect(feedDi(loadConfig())).toEqual([]);
    expect(copieSuDisco(dati)).toEqual([]);
  });

  it("un flusso che non finisce mai si interrompe: il server non riesce a mandarne più di una manciata di MiB oltre il limite", async () => {
    const t0 = Date.now();
    await expect(scarica(s.url("/enorme.ics"))).rejects.toMatchObject({ message: TROPPO });
    expect(Date.now() - t0).toBeLessThan(10_000);
  });
});

// ---- Il contenuto ---------------------------------------------------------------------------------------------------------------------
describe("il contenuto deve essere un calendario: i primi byte, non il Content-Type", () => {
  const PNG = Buffer.from("89504e470d0a1a0a0000000d49484452", "hex");
  const GZIP = Buffer.from("1f8b0800000000000003", "hex");
  const BOM = Buffer.from([0xef, 0xbb, 0xbf]);
  const BUONI: Record<string, [Gestore, string?]> = {
    "text/calendar con charset": [risposta(ICS(), TIPO_ICS)],
    "text/calendar senza charset": [risposta(ICS(), "text/calendar")],
    "text/plain": [risposta(ICS(), "text/plain; charset=utf-8")],
    "application/octet-stream": [risposta(ICS(), "application/octet-stream")],
    "nessun Content-Type": [senzaTipo(ICS())],
    "il tipo scritto con le maiuscole": [risposta(ICS(), "Text/Calendar; Charset=UTF-8")],
    "un tipo con altri parametri": [risposta(ICS(), "text/calendar; method=PUBLISH; charset=utf-8")],
    "con il BOM UTF-8": [risposta(Buffer.concat([BOM, Buffer.from(ICS())]), TIPO_ICS)],
    "con il BOM e senza charset": [risposta(Buffer.concat([BOM, Buffer.from(ICS())]), "text/calendar")],
    "dopo righe vuote e spazi": [risposta("\r\n \t\r\n  " + ICS(), TIPO_ICS)],
    "dopo il BOM e delle righe vuote": [risposta(Buffer.concat([BOM, Buffer.from("\r\n\r\n" + ICS())]), TIPO_ICS)],
    "con BEGIN:VCALENDAR in minuscolo": [risposta(ICS().replace("BEGIN:VCALENDAR", "begin:vcalendar"), TIPO_ICS)],
  };
  const CATTIVI: Record<string, Gestore> = {
    "una pagina HTML": risposta("<html><body>Accedi per vedere il calendario</body></html>", "text/html"),
    "una pagina HTML dichiarata text/calendar": risposta("<html><body>Accedi per vedere il calendario</body></html>", TIPO_ICS),
    "un JSON": risposta('{"errore":"non autorizzato"}', "application/json"),
    "un JSON dichiarato text/plain": risposta('{"errore":"non autorizzato"}', "text/plain"),
    "una risposta vuota": risposta("", TIPO_ICS),
    "solo spazi": risposta("  \r\n\r\n \t", TIPO_ICS),
    "il solo BOM": risposta(BOM, TIPO_ICS),
    "un evento senza il calendario": risposta("BEGIN:VEVENT\r\nSUMMARY:x\r\nEND:VEVENT\r\n", TIPO_ICS),
    "del testo prima del calendario": risposta("Ecco il tuo calendario:\r\n" + ICS(), TIPO_ICS),
    "un carattere prima di BEGIN": risposta("x" + ICS(), TIPO_ICS),
    "BEGIN:VCALENDAR scritto a metà": risposta("BEGIN:VCALENDA\r\n", TIPO_ICS),
    "BEGIN:VCALENDARX": risposta("BEGIN:VCALENDAR".slice(0, -1) + "X\r\n", TIPO_ICS),
    "un'immagine PNG": risposta(PNG, "image/png"),
    "un PNG dichiarato text/calendar": risposta(PNG, TIPO_ICS),
    "un corpo compresso con gzip dichiarato text/calendar": risposta(GZIP, TIPO_ICS),
    "un calendario vero dichiarato text/html": risposta(ICS(), "text/html; charset=utf-8"),
    "un calendario vero dichiarato application/json": risposta(ICS(), "application/json"),
    "un calendario vero dichiarato image/png": risposta(ICS(), "image/png"),
    "un calendario vero dichiarato application/xml": risposta(ICS(), "application/xml"),
  };
  let s: ServerProva;
  const percorso = (gruppo: string, nome: string) => `/${gruppo}/${encodeURIComponent(nome)}.ics`;

  beforeAll(async () => {
    s = await avviaServerProva({
      ...Object.fromEntries(Object.entries(BUONI).map(([n, [g]]) => [percorso("buono", n), g])),
      ...Object.fromEntries(Object.entries(CATTIVI).map(([n, g]) => [percorso("cattivo", n), g])),
      "/accenti.ics": risposta(ICS("Festività di prova, è già qui"), TIPO_ICS),
    });
  });
  afterAll(() => s.chiudi());
  beforeEach(() => impostaReteDiProva(PROVE_LOCALI));

  it.each(Object.keys(BUONI))("%s si accetta, e il testo è quello del calendario", async (nome) => {
    const testo = await scarica(s.url(percorso("buono", nome)));
    expect(testo.replace(/^\s+/, "").toUpperCase().startsWith("BEGIN:VCALENDAR")).toBe(true);
    expect(testo.includes("\uFEFF") || testo.includes("\u00ef\u00bb\u00bf"), "il BOM non resta nel testo").toBe(false);
    expect(await apiTs.add_ics_calendar("", s.url(percorso("buono", nome)))).toMatchObject({ ok: true, feed: { name: NOME } });
  });

  it.each(Object.keys(CATTIVI))("%s: 'non contiene un calendario', senza copia, senza voce e senza chiamare l'agente", async (nome) => {
    await expect(scarica(s.url(percorso("cattivo", nome)))).rejects.toMatchObject({ message: "il link risponde ma non contiene un calendario iCal" });
    expect(await apiTs.add_ics_calendar("X", s.url(percorso("cattivo", nome)))).toEqual({ ok: false, error: ILLEGGIBILE });
    expect(scritture(agente)).toEqual([]);
    expect(feedDi(loadConfig())).toEqual([]);
    expect(copieSuDisco(dati)).toEqual([]);
  });

  it("il testo si decodifica come prima: il charset dell'intestazione, e le lettere accentate arrivano intere", async () => {
    expect(await scarica(s.url("/accenti.ics"))).toBe(ICS("Festività di prova, è già qui"));
  });
});

// ---- La copia su disco e la regola 404 e 410 -------------------------------------------------------------------------------------------
describe("la copia su disco e la regola 404 e 410 restano quelle di prima", () => {
  const ID = "ics_prova";
  const VECCHIA = ICS("Calendario vecchio");
  const NUOVA = ICS("Calendario nuovo");
  const risposta_ = { codice: 200, corpo: NUOVA as Buffer | string, tipo: TIPO_ICS as string | null, cade: false };
  let s: ServerProva;
  let url: string;
  const invecchia = () => fissaOra(Date.now() + (TTL_S + 60) * 1000);
  const copia = () => fs.readFileSync(percorsoCache(ID), "utf-8");

  beforeAll(async () => {
    s = await avviaServerProva({
      "/vivo.ics": (req, res) => {
        if (risposta_.cade) return void req.socket.destroy();
        const testa: Record<string, string> = risposta_.tipo === null ? {} : { "Content-Type": risposta_.tipo };
        res.writeHead(risposta_.codice, testa).end(risposta_.codice === 200 ? risposta_.corpo : "no");
      },
    });
    url = s.url("/vivo.ics");
  });
  afterAll(() => s.chiudi());
  beforeEach(() => {
    impostaReteDiProva({ ...PROVE_LOCALI, ammettiPorta: (porta) => porta === s.porta || portaAmmessa(porta) });
    Object.assign(risposta_, { codice: 200, corpo: NUOVA, tipo: TIPO_ICS, cade: false });
    salvaCache(ID, VECCHIA);
  });

  it("una copia fresca (meno di 30 minuti) si legge senza rete", async () => {
    const prima = s.richieste.length;
    expect(await testoCalendario(ID, url)).toEqual([VECCHIA, null]);
    expect(s.richieste.length).toBe(prima);
  });

  it("scaduta la copia, un solo scarico, e la copia nuova la sostituisce", async () => {
    invecchia();
    const prima = s.richieste.length;
    expect(await testoCalendario(ID, url)).toEqual([NUOVA, null]);
    expect(s.richieste.length).toBe(prima + 1);
    expect(copia()).toBe(NUOVA);
  });

  it.each([404, 410])("%i: la copia si toglie e il testo dice che il link è stato revocato o cambiato", async (codice) => {
    risposta_.codice = codice;
    invecchia();
    expect(await testoCalendario(ID, url)).toEqual([null, `il link non risponde più (${codice}): è stato revocato o cambiato da chi lo aveva dato`]);
    expect(fs.existsSync(percorsoCache(ID))).toBe(false);
  });

  it("la regola guarda lo stato e non il testo: ErroreHttp porta lo stato e il testo di sempre (404 Client Error: ... for url: ...)", async () => {
    risposta_.codice = 404;
    const e = await scarica(url).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(ErroreHttp);
    expect((e as ErroreHttp).stato).toBe(404);
    expect((e as Error).message).toBe(`404 Client Error: Not Found for url: ${url}`);
    risposta_.codice = 503;
    expect(((await scarica(url).catch((x: unknown) => x)) as Error).message).toBe(`503 Server Error: Service Unavailable for url: ${url}`);
  });

  it("le festività non seguono la regola: un 404 è un guasto del servizio e la copia resta", async () => {
    salvaCache(FESTIVITA_ID, COPIA_FESTE);
    fs.utimesSync(percorsoCache(FESTIVITA_ID), new Date(Date.now() - 3_600_000), new Date(Date.now() - 3_600_000));
    risposta_.codice = 404;
    const [testo, errore] = await testoCalendario(FESTIVITA_ID, url);
    expect(testo).toBe(COPIA_FESTE);
    expect(errore).toContain("aggiornamento non riuscito");
    expect(fs.existsSync(percorsoCache(FESTIVITA_ID))).toBe(true);
  });

  it.each([500, 502, 503])("%i: meglio dati vecchi che nessun dato, la copia resta", async (codice) => {
    risposta_.codice = codice;
    invecchia();
    const [testo, errore] = await testoCalendario(ID, url);
    expect(testo).toBe(VECCHIA);
    expect(errore).toContain("aggiornamento non riuscito (");
    expect(errore).toContain(String(codice));
    expect(copia()).toBe(VECCHIA);
  });

  it("la rete che cade a metà tiene la copia", async () => {
    risposta_.cade = true;
    invecchia();
    const [testo, errore] = await testoCalendario(ID, url);
    expect(testo).toBe(VECCHIA);
    expect(errore).toBe("aggiornamento non riuscito (L'indirizzo non risponde)");
  });

  it("una risposta che non è un calendario (200 con una pagina di accesso) non sostituisce la copia: resta quella di prima, con un avviso", async () => {
    risposta_.corpo = "<html><body>Accedi</body></html>";
    risposta_.tipo = "text/html";
    invecchia();
    expect(await testoCalendario(ID, url)).toEqual([VECCHIA, "aggiornamento non riuscito (il link risponde ma non contiene un calendario iCal)"]);
    expect(copia()).toBe(VECCHIA);
    // un calendario vero dichiarato con un tipo che non va: lo stesso
    risposta_.corpo = NUOVA;
    risposta_.tipo = "application/json";
    expect((await testoCalendario(ID, url))[1]).toBe("aggiornamento non riuscito (il link risponde ma non contiene un calendario iCal)");
    expect(copia()).toBe(VECCHIA);
  });

  it("senza copia e con una risposta che non va: nessun testo e il motivo; con il link rimesso a posto torna da solo", async () => {
    fs.rmSync(percorsoCache(ID), { force: true });
    risposta_.codice = 500;
    expect((await testoCalendario(ID, url))[0]).toBeNull();
    risposta_.codice = 200;
    expect(await testoCalendario(ID, url)).toEqual([NUOVA, null]);
  });

  it("senza il Content-Type una risposta che comincia come un calendario si accetta anche al rinnovo", async () => {
    risposta_.tipo = null;
    invecchia();
    expect(await testoCalendario(ID, url)).toEqual([NUOVA, null]);
  });

  it("un calendario sottoscritto prima della guardia con un indirizzo che ora non si raggiunge: la copia resta e lo dice, senza rete", async () => {
    const prima = s.richieste.length;
    invecchia();
    for (const vecchio of ["http://10.1.2.3/c.ics", "http://169.254.169.254/latest/meta-data/", "http://[::1]/c.ics", "http://localhost/c.ics", "http://agent/api/ics", "http://calendari.example.org:8080/c.ics"]) {
      expect(await testoCalendario(ID, vecchio), vecchio).toEqual([VECCHIA, "aggiornamento non riuscito (Indirizzo non ammesso)"]);
    }
    fs.rmSync(percorsoCache(ID), { force: true });
    expect(await testoCalendario(ID, "http://10.1.2.3/c.ics")).toEqual([null, "Indirizzo non ammesso"]);
    expect(s.richieste.length).toBe(prima);
  });

  it("nella pagina il pallino è giallo con la copia e rosso senza, e il testo non ripete l'indirizzo", async () => {
    // le festività hanno la copia fresca (nessuna rete); quella del calendario è di un'ora fa e il suo indirizzo ora non si raggiunge
    salvaCache(FESTIVITA_ID, COPIA_FESTE);
    modificaConfig((c) => void (c.ics_calendars = [{ id: ID, name: "Amico", url: "http://10.1.2.3/c.ics" }]));
    fs.utimesSync(percorsoCache(ID), new Date(Date.now() - 3_600_000), new Date(Date.now() - 3_600_000));
    const da = parseBound("2026-10-12T00:00:00");
    const a = parseBound("2026-10-19T00:00:00");
    const [, errori] = await eventiFeed(loadConfig(), da, a);
    expect(statoFeed()[ID]).toEqual({ stato: "avviso", msg: "aggiornamento non riuscito (Indirizzo non ammesso)" });
    expect(errori).toContain("Amico: aggiornamento non riuscito (Indirizzo non ammesso)");
    fs.rmSync(percorsoCache(ID), { force: true });
    await eventiFeed(loadConfig(), da, a);
    expect(statoFeed()[ID]).toEqual({ stato: "errore", msg: "Indirizzo non ammesso" });
  });
});

// ---- I posti sono quelli dei loghi -----------------------------------------------------------------------------------------------------
describe("i collegamenti verso l'esterno: gli stessi posti dei loghi", () => {
  beforeEach(() => impostaReteDiProva(PROVE_LOCALI));

  it("al più 4 calendari insieme; gli altri aspettano il posto; con la coda piena (16) il successivo riceve 'Troppe richieste' e il posto non si perde", async () => {
    let aperte = 0;
    let massimo = 0;
    const s = await avviaServerProva({
      "/lento.ics": (_, res) => {
        aperte++;
        massimo = Math.max(massimo, aperte);
        // tanto da far arrivare tutti e 4 i primi collegamenti prima che il primo finisca, anche su una macchina lenta
        setTimeout(() => {
          aperte--;
          res.writeHead(200, { "Content-Type": TIPO_ICS }).end(ICS());
        }, 150);
      },
    });
    try {
      expect(POSTI_MAX).toBe(4);
      const lavori = Array.from({ length: POSTI_MAX + 16 }, () => scarica(s.url("/lento.ics")));
      expect(posti().inUso).toBe(POSTI_MAX);
      expect(posti().inCoda).toBe(16);
      // lo scarico dei loghi in secondo piano prende un posto solo se è libero adesso: non lo è
      expect(posti().provaPrendere()).toBeNull();
      const troppe = await scarica(s.url("/lento.ics")).catch((x: unknown) => x);
      expect((troppe as Error).message).toBe(TROPPE_RICHIESTE);
      // dalla finestra: il testo da solo, senza "Il link non risponde"
      expect(await apiTs.add_ics_calendar("x", s.url("/lento.ics"))).toEqual({ ok: false, error: "Troppe richieste, riprova tra poco" });
      const testi = await Promise.all(lavori);
      expect(testi.every((t) => t === ICS())).toBe(true);
      expect(massimo).toBeLessThanOrEqual(POSTI_MAX);
      expect(massimo).toBe(POSTI_MAX);
    } finally {
      await s.chiudi();
    }
  });

  it("un posto si rende anche quando lo scarico fallisce (404, non calendario, rete che cade, indirizzo vietato)", async () => {
    const s = await avviaServerProva({
      "/c.ics": risposta(ICS(), TIPO_ICS),
      "/html.ics": risposta("<html></html>", "text/html"),
      "/cade.ics": (req) => void req.socket.destroy(),
    });
    try {
      for (let giro = 0; giro < 3; giro++) {
        for (const p of ["/non-c-e.ics", "/html.ics", "/cade.ics"]) await scarica(s.url(p)).catch(() => null);
        await scarica("http://10.0.0.5/c.ics").catch(() => null);
      }
      expect(posti().inUso).toBe(0);
      // e tutti e 4 i posti tornano disponibili
      const presi = Array.from({ length: POSTI_MAX }, () => posti().provaPrendere());
      expect(presi.every((r) => r !== null)).toBe(true);
      for (const r of presi) r?.();
    } finally {
      await s.chiudi();
    }
  });
});

// ---- Intestazioni e proxy ---------------------------------------------------------------------------------------------------------------
describe("che cosa parte verso il sito del calendario", () => {
  it("Accept per i calendari, lo User-Agent di sempre, nessuna compressione e nessuna credenziale; nessun proxy dell'ambiente", async () => {
    const proxy = await avviaServerProva({ "/": risposta("proxy", "text/plain") });
    const s = await avviaServerProva({ "/c.ics": risposta(ICS(), TIPO_ICS) });
    try {
      impostaReteDiProva(PROVE_LOCALI);
      for (const k of ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy"]) process.env[k] = `http://127.0.0.1:${proxy.porta}`;
      expect(await scarica(s.url("/c.ics"))).toBe(ICS());
      expect(proxy.richieste, "il proxy dell'ambiente non si usa").toEqual([]);
      const h = s.ultime;
      expect(h.accept).toBe("text/calendar, text/plain;q=0.9, */*;q=0.1");
      expect(h["user-agent"]).toBe("CalendarHub/1.0");
      expect(h["accept-encoding"]).toBe("identity");
      expect(h.connection).toBe("close");
      for (const nome of ["cookie", "authorization", "referer", "origin", "x-forwarded-for"]) expect(h[nome], nome).toBeUndefined();
    } finally {
      await proxy.chiudi();
      await s.chiudi();
    }
  });

  it("un sito che ignora Accept-Encoding e manda gzip non fa passare niente: non è un calendario", async () => {
    const s = await avviaServerProva({ "/g.ics": (_, res) => void res.writeHead(200, { "Content-Type": TIPO_ICS, "Content-Encoding": "gzip" }).end(Buffer.from("1f8b08000000000000ff4b2ac9c82c4e05002a4e5b2705000000", "hex")) });
    try {
      impostaReteDiProva(PROVE_LOCALI);
      await expect(scarica(s.url("/g.ics"))).rejects.toMatchObject({ message: "il link risponde ma non contiene un calendario iCal" });
    } finally {
      await s.chiudi();
    }
  });

  it("il tempo totale è quello della prova se c'è (qui 300 ms), e un sito che non risponde non tiene il posto", async () => {
    const s = await avviaServerProva({ "/appeso.ics": () => {} });
    try {
      impostaReteDiProva({ ...PROVE_LOCALI, attesaMs: 300 });
      const t0 = Date.now();
      expect(await apiTs.add_ics_calendar("x", s.url("/appeso.ics"))).toEqual(NON_RISPONDE);
      expect(Date.now() - t0).toBeLessThan(5000);
      expect(posti().inUso).toBe(0);
    } finally {
      await s.chiudi();
    }
  });
});

// ---- Le festività ---------------------------------------------------------------------------------------------------------------------
describe("il calendario delle festività (Google) passa dalla guardia", () => {
  it("l'indirizzo vero è pubblico, https sulla 443: lo ammettono le regole vere e il percorso resta com'è (%23 e %40)", () => {
    const u = controllaUrl(FESTIVITA_URL);
    expect(u.protocol).toBe("https:");
    expect(u.hostname).toBe("calendar.google.com");
    expect(u.port).toBe("");
    expect(u.pathname + u.search).toBe("/calendar/ical/it.italian%23holiday%40group.v.calendar.google.com/public/basic.ics");
  });

  it("il percorso con %23 e %40 arriva al sito tale e quale, e il calendario si legge senza Content-Type particolari", async () => {
    const percorso = "/calendar/ical/it.italian%23holiday%40group.v.calendar.google.com/public/basic.ics";
    expect(FESTIVITA_URL.endsWith(percorso)).toBe(true);
    const s = await avviaServerProva({ [percorso]: risposta(COPIA_FESTE, "text/calendar; charset=UTF-8") });
    try {
      // un nome pubblico finto che porta al server di prova (la 443 vera non c'è): le regole sugli indirizzi sono quelle vere, tranne 127.0.0.1
      impostaReteDiProva({ ...PROVE_LOCALI, risolvi: async (nome) => (nome === "calendar.test" ? ["127.0.0.1"] : [nome]) });
      expect(await scarica(`http://calendar.test:${s.porta}${percorso}`)).toBe(COPIA_FESTE);
      expect(s.richieste).toEqual([percorso]);
    } finally {
      await s.chiudi();
    }
  });
});
