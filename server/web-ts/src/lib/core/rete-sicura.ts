import dns from "node:dns";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import { ErroreValore } from "./py";

// Il gateway gira su una macchina con altri servizi e con l'indirizzo dei metadati del cloud: quando scarica un indirizzo scelto da chi usa
// l'app (i loghi delle aziende) deve arrivare solo a siti pubblici. Ogni controllo sta qui, con i moduli di Node e nessuna dipendenza:
// solo http e https sulle porte 80 e 443, mai credenziali nell'indirizzo, il nome si risolve qui e ogni indirizzo trovato deve essere
// pubblico (si rifiuta tutto il nome se anche uno solo non lo è), l'indirizzo a cui ci si collega è quello controllato, e di nuovo
// controllato a collegamento fatto; ogni reindirizzamento (al massimo 3) ricomincia da capo. Tempo totale 8 secondi, al più 1 MB letto.
// Il contenuto dichiarato dal sito non conta: il tipo lo decide chi chiama, dai primi byte (immagine.ts).

export const ATTESA_MS = 8000;
export const MAX_REINDIRIZZAMENTI = 3;
/** Quello che si legge dalla rete al massimo, qualunque cosa dica il sito. */
export const LIMITE_LETTURA = 1024 * 1024;
/** Quello che si accetta di un logo. */
export const LIMITE_ACCETTATO = 512 * 1024;
const INDIRIZZO_MAX = 2048;

export type Motivo = "non_valido" | "non_ammesso" | "non_risponde" | "certificato" | "errore_http" | "reindirizzamenti" | "troppo_grande";

export const TESTI = {
  nonValido: "Indirizzo non valido",
  nonAmmesso: "Indirizzo non ammesso",
  nonRisponde: "L'indirizzo non risponde",
  certificato: "Il certificato del sito non è valido",
  reindirizzamenti: `Troppi reindirizzamenti (massimo ${MAX_REINDIRIZZAMENTI})`,
  troppoGrande: `Immagine troppo grande (massimo ${LIMITE_ACCETTATO / 1024} KB)`,
} as const;

/** Un errore con il testo che la pagina mostra (come ErroreValore) e il motivo per le prove. */
export class ErroreRete extends ErroreValore {
  constructor(
    readonly motivo: Motivo,
    messaggio: string,
  ) {
    super(messaggio);
  }
}

// ---- Quali indirizzi sono pubblici -------------------------------------------------------------------------------------------
// Tutto quello che non è un indirizzo unicast globale: loopback, privati, link-local (anche 169.254.169.254, i metadati), CGNAT,
// documentazione, multicast, riservati, non specificato; IPv6: loopback e non specificato (::/96 comprende anche gli IPv4 "compatibili"),
// IPv4 mappati, NAT64, unique-local, link-local, site-local, multicast, 6to4 e Teredo, che portano un IPv4 dentro.
const RETI_VIETATE: [string, number, "ipv4" | "ipv6"][] = [
  ["0.0.0.0", 8, "ipv4"],
  ["10.0.0.0", 8, "ipv4"],
  ["100.64.0.0", 10, "ipv4"],
  ["127.0.0.0", 8, "ipv4"],
  ["169.254.0.0", 16, "ipv4"],
  ["172.16.0.0", 12, "ipv4"],
  ["192.0.0.0", 24, "ipv4"],
  ["192.0.2.0", 24, "ipv4"],
  ["192.88.99.0", 24, "ipv4"],
  ["192.168.0.0", 16, "ipv4"],
  ["198.18.0.0", 15, "ipv4"],
  ["198.51.100.0", 24, "ipv4"],
  ["203.0.113.0", 24, "ipv4"],
  ["224.0.0.0", 4, "ipv4"],
  ["240.0.0.0", 4, "ipv4"],
  ["::", 96, "ipv6"],
  ["::ffff:0:0", 96, "ipv6"],
  ["64:ff9b::", 96, "ipv6"],
  ["64:ff9b:1::", 48, "ipv6"],
  ["100::", 64, "ipv6"],
  ["2001::", 32, "ipv6"],
  ["2001:db8::", 32, "ipv6"],
  ["2002::", 16, "ipv6"],
  ["fc00::", 7, "ipv6"],
  ["fe80::", 10, "ipv6"],
  ["fec0::", 10, "ipv6"],
  ["ff00::", 8, "ipv6"],
];
// Due elenchi separati: BlockList confronta un IPv4 anche con le reti IPv6 come se fosse "mappato" (::ffff:a.b.c.d), e con ::ffff:0:0/96
// vietata tutti gli IPv4 risulterebbero vietati.
const vietatiV4 = new net.BlockList();
const vietatiV6 = new net.BlockList();
for (const [rete, bit, famiglia] of RETI_VIETATE) (famiglia === "ipv4" ? vietatiV4 : vietatiV6).addSubnet(rete, bit, famiglia);

/** Un indirizzo IP (non un nome) che si può raggiungere: pubblico. Qualunque altra cosa è no. */
export function indirizzoAmmesso(ip: string): boolean {
  const famiglia = net.isIP(ip);
  if (famiglia === 4) return !vietatiV4.check(ip, "ipv4");
  return famiglia === 6 && !vietatiV6.check(ip, "ipv6");
}

export const portaAmmessa = (porta: number): boolean => porta === 80 || porta === 443;

/** Lo stesso indirizzo scritto sempre allo stesso modo (IPv6 compresso e minuscolo), per confrontare quello controllato con quello collegato. */
function canonico(ip: string): string {
  if (!net.isIPv6(ip)) return ip;
  try {
    return new URL(`http://[${ip}]/`).hostname.slice(1, -1);
  } catch {
    return ip;
  }
}

/**
 * L'indirizzo scritto da chi usa l'app come URL da raggiungere, o ErroreRete. L'analisi dell'URL riporta alla forma di sempre gli indirizzi
 * scritti in altro modo (2130706433, 0x7f.1, 0177.0.0.1, [::ffff:7f00:1]) e il controllo guarda quella.
 */
export function controllaUrl(testo: unknown, ammettiIndirizzo: (ip: string) => boolean = indirizzoAmmesso, ammettiPorta: (porta: number) => boolean = portaAmmessa): URL {
  if (typeof testo !== "string") throw new ErroreRete("non_valido", TESTI.nonValido);
  const t = testo.trim();
  if (!t || t.length > INDIRIZZO_MAX || /[\u0000-\u0020\u007f-\u009f]/.test(t)) throw new ErroreRete("non_valido", TESTI.nonValido);
  let url: URL;
  try {
    url = new URL(t);
  } catch {
    throw new ErroreRete("non_valido", TESTI.nonValido);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new ErroreRete("non_ammesso", TESTI.nonAmmesso);
  if (url.username || url.password) throw new ErroreRete("non_ammesso", TESTI.nonAmmesso);
  if (!ammettiPorta(url.port ? Number(url.port) : url.protocol === "https:" ? 443 : 80)) throw new ErroreRete("non_ammesso", TESTI.nonAmmesso);
  const host = url.hostname.replace(/^\[|\]$/g, "").toLowerCase().replace(/\.$/, "");
  if (!host) throw new ErroreRete("non_valido", TESTI.nonValido);
  if (net.isIP(host)) {
    if (!ammettiIndirizzo(host)) throw new ErroreRete("non_ammesso", TESTI.nonAmmesso);
  } else if (host === "localhost" || host.endsWith(".localhost") || !host.includes(".")) {
    // un nome senza punti è un servizio della rete interna (agent, gateway...), mai un sito pubblico
    throw new ErroreRete("non_ammesso", TESTI.nonAmmesso);
  }
  return url;
}

// ---- Lo scarico ---------------------------------------------------------------------------------------------------------------
export type Risolutore = (host: string) => Promise<string[]>;

// dns.promises.lookup si legge a ogni chiamata: così le prove di sistema possono simulare la rete
const risolviDns: Risolutore = async (host) => (await dns.promises.lookup(host, { all: true, verbatim: true })).map((r) => r.address);

/**
 * Solo per le prove: nessuna variabile d'ambiente e nessuna richiesta lo imposta, e il solo punto del gateway che scarica (loghi.ts)
 * passa quello che una prova ha messo con impostaReteDiProva, che in produzione non si può chiamare. Il risolutore di nomi, le regole
 * sugli indirizzi e sulle porte, il tempo e la CA di un certificato inventato sono quello che una prova vuole cambiare per raggiungere
 * un server su 127.0.0.1.
 */
export type SoloProve = {
  risolvi?: Risolutore;
  ammettiIndirizzo?: (ip: string) => boolean;
  ammettiPorta?: (porta: number) => boolean;
  attesaMs?: number;
  ca?: string | Buffer;
};

type Rete = Required<Omit<SoloProve, "ca">> & { ca?: string | Buffer };

/**
 * La funzione che Node chiama per sapere a quale indirizzo collegarsi: risolve il nome e rifiuta se un solo indirizzo non è pubblico.
 * Gli indirizzi buoni li ricorda in `consentiti`, per controllare quello a cui il socket si collega davvero.
 */
function creaLookup(rete: Rete, consentiti: Set<string>) {
  return (host: string, opzioni: dns.LookupOptions, cb: (err: NodeJS.ErrnoException | null, indirizzo?: string | dns.LookupAddress[], famiglia?: number) => void) => {
    rete.risolvi(host).then(
      (indirizzi) => {
        if (!indirizzi.length || !indirizzi.every((a) => rete.ammettiIndirizzo(a))) {
          return cb(new ErroreRete("non_ammesso", TESTI.nonAmmesso) as unknown as NodeJS.ErrnoException);
        }
        for (const a of indirizzi) consentiti.add(canonico(a));
        if (opzioni.all) return cb(null, indirizzi.map((address) => ({ address, family: net.isIPv6(address) ? 6 : 4 })));
        cb(null, indirizzi[0], net.isIPv6(indirizzi[0]) ? 6 : 4);
      },
      (e: unknown) => cb(e as NodeJS.ErrnoException),
    );
  };
}

/** A collegamento fatto: l'indirizzo dall'altra parte deve essere uno di quelli controllati (un nome che cambia indirizzo tra il controllo e la connessione). */
export function controllaCollegato(socket: { remoteAddress?: string }, consentiti: ReadonlySet<string>, ammettiIndirizzo: (ip: string) => boolean = indirizzoAmmesso): void {
  const a = socket.remoteAddress ? canonico(socket.remoteAddress) : "";
  if (!a || !consentiti.has(a) || !ammettiIndirizzo(a)) throw new ErroreRete("non_ammesso", TESTI.nonAmmesso);
}

type Passo = { stato: number; location: string | null; tipo: string | null; dati: Buffer | null };

function errore(e: unknown): ErroreRete {
  if (e instanceof ErroreRete) return e;
  const codice = (e as NodeJS.ErrnoException | undefined)?.code ?? "";
  if (/CERT|SELF_SIGNED|UNABLE_TO_VERIFY|ERR_TLS/.test(codice)) return new ErroreRete("certificato", TESTI.certificato);
  return new ErroreRete("non_risponde", TESTI.nonRisponde);
}

/** Una sola richiesta GET, senza seguire i reindirizzamenti: lo stato, dove rimandano, e il corpo se la risposta è un successo. */
function unPasso(url: URL, rete: Rete, segnale: AbortSignal): Promise<Passo> {
  return new Promise((ok, no) => {
    const host = url.hostname.replace(/^\[|\]$/g, "");
    const consentiti = new Set<string>();
    if (net.isIP(host)) consentiti.add(canonico(host));
    const opzioni: https.RequestOptions = {
      hostname: host,
      port: url.port || undefined,
      path: url.pathname + url.search,
      method: "GET",
      // un socket per richiesta: niente connessioni tenute aperte e riusate tra un sito e l'altro
      agent: false,
      // solo i tipi che si accettano (un sito che sceglie il formato dall'intestazione non risponde con uno che non va bene), nessuna compressione
      headers: { Accept: "image/png,image/jpeg,image/gif,image/webp,image/svg+xml,image/x-icon,image/vnd.microsoft.icon,*/*;q=0.5", "Accept-Encoding": "identity", "User-Agent": "OECopilot/1.0" },
      lookup: creaLookup(rete, consentiti) as unknown as net.LookupFunction,
      ...(rete.ca ? { ca: rete.ca } : {}),
    };
    let finita = false;
    const req = (url.protocol === "https:" ? https : http).request(opzioni, (res) => {
      const stato = res.statusCode ?? 0;
      const location = typeof res.headers.location === "string" ? res.headers.location : null;
      if (stato < 200 || stato >= 300) {
        // rimando, errore: il corpo non serve
        res.destroy();
        return fine(() => ok({ stato, location, tipo: null, dati: null }));
      }
      const dichiarata = Number(res.headers["content-length"]);
      if (Number.isFinite(dichiarata) && dichiarata > LIMITE_LETTURA) return rifiuta(new ErroreRete("troppo_grande", TESTI.troppoGrande));
      const pezzi: Buffer[] = [];
      let totale = 0;
      res.on("data", (c: Buffer) => {
        totale += c.length;
        if (totale > LIMITE_LETTURA) return rifiuta(new ErroreRete("troppo_grande", TESTI.troppoGrande));
        pezzi.push(c);
      });
      res.on("end", () => fine(() => ok({ stato, location: null, tipo: res.headers["content-type"] ?? null, dati: Buffer.concat(pezzi, totale) })));
      res.on("error", (e) => rifiuta(errore(e)));
      res.on("close", () => res.complete || rifiuta(errore(null)));
    });
    // finisce una volta sola: la scadenza, un errore e la fine della risposta possono arrivare tutti, in qualunque ordine
    function fine(chiudi: () => void) {
      if (finita) return;
      finita = true;
      segnale.removeEventListener("abort", scaduto);
      chiudi();
    }
    function rifiuta(e: ErroreRete) {
      fine(() => no(e));
      req.destroy();
    }
    const scaduto = () => rifiuta(errore(null));
    if (segnale.aborted) return scaduto();
    segnale.addEventListener("abort", scaduto, { once: true });
    req.on("error", (e) => rifiuta(errore(e)));
    req.on("socket", (socket) => {
      socket.on("connect", () => {
        try {
          controllaCollegato(socket, consentiti, rete.ammettiIndirizzo);
        } catch (e) {
          rifiuta(errore(e));
        }
      });
    });
    req.end();
  });
}

export type Scaricato = { dati: Buffer; url: URL; /** Il Content-Type che dichiara il sito: non è una prova di niente. */ tipoDichiarato: string | null };

/**
 * Scarica `indirizzo` con le regole del modulo e restituisce i byte e l'indirizzo a cui si è arrivati. Solleva ErroreRete, con il testo
 * che la pagina mostra. `soloProve` non lo passa nessun codice del gateway (vedi SoloProve).
 */
export async function scaricaSicuro(indirizzo: string, soloProve?: SoloProve, limiteAccettato = LIMITE_ACCETTATO): Promise<Scaricato> {
  const rete: Rete = {
    risolvi: soloProve?.risolvi ?? risolviDns,
    ammettiIndirizzo: soloProve?.ammettiIndirizzo ?? indirizzoAmmesso,
    ammettiPorta: soloProve?.ammettiPorta ?? portaAmmessa,
    attesaMs: soloProve?.attesaMs ?? ATTESA_MS,
    ca: soloProve?.ca,
  };
  const segnale = AbortSignal.timeout(rete.attesaMs);
  let url = controllaUrl(indirizzo, rete.ammettiIndirizzo, rete.ammettiPorta);
  for (let salti = 0; ; salti++) {
    const passo = await unPasso(url, rete, segnale);
    if (passo.stato >= 300 && passo.stato < 400 && passo.location) {
      if (salti >= MAX_REINDIRIZZAMENTI) throw new ErroreRete("reindirizzamenti", TESTI.reindirizzamenti);
      // ogni tappa ricomincia dai controlli sull'indirizzo, compreso il nome da risolvere di nuovo
      let prossimo: string;
      try {
        prossimo = new URL(passo.location, url).href;
      } catch {
        throw new ErroreRete("non_valido", TESTI.nonValido);
      }
      url = controllaUrl(prossimo, rete.ammettiIndirizzo, rete.ammettiPorta);
      continue;
    }
    if (passo.stato < 200 || passo.stato >= 300 || !passo.dati) {
      throw new ErroreRete("errore_http", `L'indirizzo risponde con un errore (${passo.stato})`);
    }
    if (passo.dati.length > limiteAccettato) throw new ErroreRete("troppo_grande", TESTI.troppoGrande);
    return { dati: passo.dati, url, tipoDichiarato: passo.tipo };
  }
}
