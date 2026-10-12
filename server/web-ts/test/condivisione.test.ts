import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AVANTI, INDIETRO, LINK_MAX, NOME_CALENDARIO_MAX, NOME_MAX, quandoLetto } from "@/lib/condivisione-comune";
import { chiama, METODI_TS, type NomeMetodoTs } from "@/lib/core/api";
import { archivio, chiudiArchivio } from "@/lib/core/archivio";
import {
  aggiornaCondivisione,
  azzeraAccessi,
  creaCondivisione,
  eliminaCondivisione,
  elencoCondivisioni,
  FORMA_TOKEN,
  INTERVALLO_ACCESSI_MS,
  preparaCondivisioni,
  registraAccesso,
  revocaCondivisione,
  ruotaCondivisione,
  trovaPerToken,
} from "@/lib/core/condivisione";
import { fissaOra } from "@/lib/core/orologio";
import { DEFAULT_CONFIG, modificaConfig } from "@/lib/core/stato";
import { handle, MAX_BODY, type Deps } from "@/lib/gateway";
import { cartellaDati } from "./agente-condivisione";

// I link di condivisione: la tabella (aggiunta, senza toccare il resto), la creazione, i controlli con i testi in italiano, la modifica, la revoca, la rotazione,
// il confronto del testo segreto, l'ultimo accesso (una scrittura al minuto) e i metodi dell'API attraverso il gateway con le sue regole.

let dati: string;
let registro: string[];

const ora = Date.UTC(2026, 9, 12, 10, 0, 0);
const ok = <T extends { ok: boolean }>(r: T) => {
  expect(r.ok, JSON.stringify(r)).toBe(true);
  return r as Extract<T, { ok: true }>;
};
const crea = (extra: Record<string, unknown> = {}) => ok(creaCondivisione({ nome: "Samuele", ...extra })).condivisione!;
const tokenDi = (url: string | null) => /\/condividi\/([^/]+)\.ics$/.exec(url ?? "")?.[1] ?? "";

beforeEach(() => {
  dati = cartellaDati();
  process.env.OE_DATI = dati;
  process.env.APP_URL = "https://oe.example.test";
  delete process.env.GATEWAY_DB;
  registro = [];
  for (const m of ["log", "info", "warn", "error"] as const) vi.spyOn(console, m).mockImplementation((...a: unknown[]) => void registro.push(a.map(String).join(" ")));
  fissaOra(ora);
  azzeraAccessi();
});

afterEach(() => {
  chiudiArchivio();
  fissaOra(null);
  fs.rmSync(dati, { recursive: true, force: true });
  delete process.env.OE_DATI;
  delete process.env.APP_URL;
  vi.restoreAllMocks();
});

describe("la tabella", () => {
  it("si aggiunge a quelle che ci sono senza toccarle, e rifarlo non cambia niente", () => {
    modificaConfig((c) => void (c.theme = "dark"));
    const prima = archivio().prepare("SELECT chiave, valore FROM stato_config ORDER BY chiave").all();
    const tabellePrima = archivio().prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").pluck().all() as string[];
    expect(tabellePrima).not.toContain("condivisioni");
    preparaCondivisioni();
    preparaCondivisioni();
    const tabelle = archivio().prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").pluck().all() as string[];
    expect(tabelle).toEqual([...tabellePrima, "condivisioni"].sort());
    expect(archivio().prepare("SELECT chiave, valore FROM stato_config ORDER BY chiave").all()).toEqual(prima);
  });

  it("si crea anche al primo uso, senza che l'avvio l'abbia fatto", () => {
    expect(ok(elencoCondivisioni()).condivisioni).toEqual([]);
    expect(archivio().prepare("SELECT name FROM sqlite_master WHERE name = 'condivisioni'").pluck().get()).toBe("condivisioni");
  });

  it("non aggiunge niente alla configurazione né al file che la copia: i link vivono solo nella loro tabella", () => {
    expect(Object.keys(DEFAULT_CONFIG).filter((k) => /condivi|share/i.test(k))).toEqual([]);
    crea();
    crea({ nome: "Altro", livello: "titoli" });
    revocaCondivisione(elencoCondivisioni().condivisioni[0].id);
    expect(archivio().prepare("SELECT chiave FROM stato_config").pluck().all().filter((k) => /condivi|share/i.test(String(k)))).toEqual([]);
    expect(fs.existsSync(path.join(dati, "config.json"))).toBe(false);
  });
});

describe("creare un link", () => {
  it("con il solo nome: livello occupato, tutti i calendari, 7 giorni indietro e 90 avanti, nome del calendario vuoto", () => {
    const c = crea();
    expect(c).toMatchObject({ nome: "Samuele", livello: "occupato", ambito: "tutti", gruppo: "", giorni_indietro: 7, giorni_avanti: 90, nome_calendario: "", ultimo_accesso: null, accessi: 0, revocato: null, creato: ora, aggiornato: ora, gruppo_mancante: false });
    expect(c.id).toMatch(/^[0-9a-f]{12}$/);
  });

  it("l'indirizzo è APP_URL, /condividi/ e 32 byte casuali in base64url (43 caratteri), e la pagina non riceve altro del testo segreto", () => {
    const c = crea();
    expect(c.url).toMatch(/^https:\/\/oe\.example\.test\/condividi\/[A-Za-z0-9_-]{43}\.ics$/);
    expect(tokenDi(c.url)).toMatch(FORMA_TOKEN);
    expect(Object.keys(c)).not.toContain("token");
    expect(Object.keys(c)).not.toContain("sale");
    expect(JSON.stringify(c).split(tokenDi(c.url)).length - 1).toBe(1);
  });

  it("l'indirizzo segue APP_URL, anche senza barra o con più barre in fondo", () => {
    process.env.APP_URL = "http://localhost:3190///";
    expect(crea().url).toMatch(/^http:\/\/localhost:3190\/condividi\/[A-Za-z0-9_-]{43}\.ics$/);
  });

  it("ogni link ha il suo testo segreto, e un testo segreto è casuale (mai lo stesso in 50 link)", () => {
    const visti = new Set<string>();
    for (let i = 0; i < LINK_MAX; i++) visti.add(tokenDi(crea({ nome: `Link ${i}` }).url));
    expect(visti.size).toBe(LINK_MAX);
    // non c'è un prefisso comune né una parte che somigli a un contatore o a un'ora
    const prefissi = new Set([...visti].map((t) => t.slice(0, 4)));
    expect(prefissi.size).toBeGreaterThan(LINK_MAX / 2);
  });

  it("con tutti i campi", () => {
    modificaConfig((c) => void (c.gruppi_calendari = { famiglia: ["Alfa"] }));
    const c = crea({ nome: "  Anna  ", livello: "titoli", ambito: "gruppo", gruppo: "famiglia", giorni_indietro: 30, giorni_avanti: 365, nome_calendario: "  Antonio  " });
    expect(c).toMatchObject({ nome: "Anna", livello: "titoli", ambito: "gruppo", gruppo: "famiglia", giorni_indietro: 30, giorni_avanti: 365, nome_calendario: "Antonio" });
  });

  it("i giorni arrivano anche come testo di cifre (da un campo del modulo)", () => {
    expect(crea({ giorni_indietro: "3", giorni_avanti: " 14 " })).toMatchObject({ giorni_indietro: 3, giorni_avanti: 14 });
  });

  it("l'elenco li dà dal più vecchio, con l'indirizzo di ognuno", () => {
    fissaOra(ora);
    const a = crea({ nome: "A" });
    fissaOra(ora + 1000);
    const b = crea({ nome: "B" });
    expect(ok(elencoCondivisioni()).condivisioni.map((c) => [c.nome, c.url])).toEqual([
      ["A", a.url],
      ["B", b.url],
    ]);
    expect(elencoCondivisioni().max).toBe(LINK_MAX);
  });

  it("i testi segreti stanno nel database come ci si aspetta: uno per link, e solo nella tabella", () => {
    const c = crea();
    const riga = archivio().prepare("SELECT token, sale FROM condivisioni WHERE id = ?").get(c.id) as { token: string; sale: string };
    expect(riga.token).toBe(tokenDi(c.url));
    expect(riga.sale).toMatch(/^[0-9a-f]{32}$/);
    expect(riga.sale).not.toBe(riga.token);
  });
});

describe("i controlli, con i testi in italiano", () => {
  const rifiuta = (dati: unknown, atteso: RegExp) => {
    const r = creaCondivisione(dati);
    expect(r.ok).toBe(false);
    expect(r.ok === false && r.error).toMatch(atteso);
    return r;
  };

  it("il nome serve, non troppo lungo, senza a capo", () => {
    rifiuta({}, /Scrivi per chi è il link/);
    rifiuta({ nome: "   " }, /Scrivi per chi è il link/);
    rifiuta({ nome: 5 }, /Il nome non è valido/);
    rifiuta({ nome: "x".repeat(NOME_MAX + 1) }, new RegExp(`al più ${NOME_MAX} caratteri`));
    rifiuta({ nome: "a\nb" }, /a capo/);
    rifiuta({ nome: "a\u0000b" }, /caratteri di controllo/);
    expect(creaCondivisione({ nome: "x".repeat(NOME_MAX) }).ok).toBe(true);
  });

  it("i dati devono essere un oggetto", () => {
    for (const d of [null, undefined, "testo", 7, [], [{ nome: "x" }]]) rifiuta(d, /dati del link non sono validi/);
  });

  it("il livello e cosa condividere sono fra quelli che ci sono", () => {
    rifiuta({ nome: "A", livello: "tutto" }, /Il livello non è valido/);
    rifiuta({ nome: "A", livello: 1 }, /Il livello non è valido/);
    rifiuta({ nome: "A", ambito: "qualcosa" }, /Cosa condividere non è valido/);
  });

  it("con un gruppo, il gruppo serve e deve esserci", () => {
    rifiuta({ nome: "A", ambito: "gruppo" }, /Scegli il gruppo/);
    rifiuta({ nome: "A", ambito: "gruppo", gruppo: "inesistente" }, /Il gruppo «inesistente» non esiste/);
    rifiuta({ nome: "A", ambito: "gruppo", gruppo: 3 }, /Il nome del gruppo non è valido/);
    modificaConfig((c) => void (c.gruppi_calendari = { uno: ["Alfa"] }));
    expect(creaCondivisione({ nome: "A", ambito: "gruppo", gruppo: "uno" }).ok).toBe(true);
    // con tutti i calendari un gruppo scritto per sbaglio si ignora
    expect(crea({ ambito: "tutti", gruppo: "inesistente" }).gruppo).toBe("");
  });

  it.each([
    [{ giorni_indietro: -1 }, /Giorni indietro: .* da 0 a 30/],
    [{ giorni_indietro: INDIETRO.max + 1 }, /Giorni indietro/],
    [{ giorni_indietro: 1.5 }, /Giorni indietro/],
    [{ giorni_indietro: "abc" }, /Giorni indietro/],
    [{ giorni_indietro: null }, /Giorni indietro/],
    [{ giorni_avanti: 0 }, /Giorni avanti: .* da 1 a 365/],
    [{ giorni_avanti: AVANTI.max + 1 }, /Giorni avanti/],
    [{ giorni_avanti: "-5" }, /Giorni avanti/],
  ])("i giorni fuori dai limiti (%j) si rifiutano", (campi, errore) => {
    rifiuta({ nome: "A", ...campi }, errore);
  });

  it("i limiti stessi si accettano: 0 e 30 indietro, 1 e 365 avanti", () => {
    expect(crea({ giorni_indietro: 0, giorni_avanti: 1 })).toMatchObject({ giorni_indietro: 0, giorni_avanti: 1 });
    expect(crea({ giorni_indietro: INDIETRO.max, giorni_avanti: AVANTI.max })).toMatchObject({ giorni_indietro: 30, giorni_avanti: 365 });
  });

  it("il nome del calendario: non troppo lungo, senza a capo, di testo", () => {
    rifiuta({ nome: "A", nome_calendario: "x".repeat(NOME_CALENDARIO_MAX + 1) }, /nome del calendario è troppo lungo/);
    rifiuta({ nome: "A", nome_calendario: "a\nb" }, /nome del calendario non può avere a capo/);
    rifiuta({ nome: "A", nome_calendario: 4 }, /nome del calendario non è valido/);
  });

  it("al più 50 link, revocati compresi: poi dice di toglierne", () => {
    for (let i = 0; i < LINK_MAX; i++) crea({ nome: `L${i}` });
    rifiuta({ nome: "uno di troppo" }, new RegExp(`già ${LINK_MAX} link`));
    eliminaCondivisione(elencoCondivisioni().condivisioni[0].id);
    expect(creaCondivisione({ nome: "ora sì" }).ok).toBe(true);
  });

  it("un rifiuto non crea niente", () => {
    creaCondivisione({ nome: "A", livello: "no" });
    expect(elencoCondivisioni().condivisioni).toEqual([]);
  });
});

describe("cambiare un link", () => {
  it("cambia i campi dati e lascia gli altri com'erano; l'indirizzo non cambia", () => {
    const c = crea({ livello: "occupato", giorni_indietro: 3, nome_calendario: "Mio" });
    fissaOra(ora + 5000);
    const r = ok(aggiornaCondivisione(c.id, { livello: "titoli" })).condivisione!;
    expect(r).toMatchObject({ nome: "Samuele", livello: "titoli", giorni_indietro: 3, giorni_avanti: 90, nome_calendario: "Mio", url: c.url });
    expect(r.aggiornato).toBe(ora + 5000);
    expect(ok(aggiornaCondivisione(c.id, { nome: "Nuovo", giorni_avanti: 10 })).condivisione).toMatchObject({ nome: "Nuovo", livello: "titoli", giorni_avanti: 10 });
    expect(ok(aggiornaCondivisione(c.id, { nome_calendario: "" })).condivisione!.nome_calendario).toBe("");
  });

  it("l'ora di modifica cresce sempre, anche con due modifiche nello stesso istante (serve a chi tiene il calendario in memoria)", () => {
    const c = crea();
    const a = ok(aggiornaCondivisione(c.id, { nome: "Uno" })).condivisione!.aggiornato;
    const b = ok(aggiornaCondivisione(c.id, { nome: "Due" })).condivisione!.aggiornato;
    expect(b).toBeGreaterThan(a);
    expect(a).toBeGreaterThan(c.aggiornato);
  });

  it("gli stessi controlli della creazione, e un rifiuto non cambia niente", () => {
    const c = crea();
    for (const dati of [{ nome: "" }, { livello: "x" }, { giorni_avanti: 400 }, { ambito: "gruppo", gruppo: "no" }, { nome_calendario: "x".repeat(81) }, null]) {
      expect(aggiornaCondivisione(c.id, dati).ok, JSON.stringify(dati)).toBe(false);
    }
    expect(ok(elencoCondivisioni()).condivisioni[0]).toEqual(c);
  });

  it("un link che non c'è o con un id che non ha la forma giusta: non trovato", () => {
    for (const id of ["0123456789ab", "x", "", null, 5, "../etc"]) {
      const r = aggiornaCondivisione(id, { nome: "x" });
      expect(r).toEqual({ ok: false, error: "Link non trovato." });
    }
  });

  it("un link revocato non si cambia più", () => {
    const c = crea();
    revocaCondivisione(c.id);
    expect(aggiornaCondivisione(c.id, { nome: "x" })).toEqual({ ok: false, error: "Il link è revocato: non si può più cambiare." });
  });

  it("passare da un gruppo a tutti i calendari toglie il gruppo, e un gruppo che sparisce dopo non blocca le altre modifiche", () => {
    modificaConfig((c) => void (c.gruppi_calendari = { famiglia: ["Alfa"] }));
    const c = crea({ ambito: "gruppo", gruppo: "famiglia" });
    expect(ok(elencoCondivisioni()).condivisioni[0].gruppo_mancante).toBe(false);
    modificaConfig((cfg) => void (cfg.gruppi_calendari = {}));
    expect(ok(elencoCondivisioni()).condivisioni[0].gruppo_mancante).toBe(true);
    expect(ok(aggiornaCondivisione(c.id, { nome: "Rinominato" })).condivisione).toMatchObject({ nome: "Rinominato", gruppo_mancante: true });
    expect(aggiornaCondivisione(c.id, { gruppo: "famiglia" }).ok).toBe(false);
    expect(ok(aggiornaCondivisione(c.id, { ambito: "tutti" })).condivisione).toMatchObject({ ambito: "tutti", gruppo: "", gruppo_mancante: false });
  });
});

describe("il testo segreto: trovare, revocare, ruotare", () => {
  it("un testo che vale si trova, e dà il link con il suo sale", () => {
    const c = crea();
    const t = trovaPerToken(tokenDi(c.url));
    expect(t?.id).toBe(c.id);
    expect(t?.sale).toMatch(/^[0-9a-f]{32}$/);
  });

  it("non si trova un testo sconosciuto, uno che ha un solo carattere diverso, uno che non ha la forma giusta, uno vuoto", () => {
    const c = crea();
    const vero = tokenDi(c.url);
    const sbagliato = (vero[0] === "A" ? "B" : "A") + vero.slice(1);
    const casuale = crypto.randomBytes(32).toString("base64url");
    for (const t of [sbagliato, casuale, "", vero.slice(0, 42), vero + "A", vero.replace(/.$/, "!"), vero.toLowerCase() === vero ? vero.toUpperCase() : vero.toLowerCase(), "x".repeat(200), "../" + vero, vero + "\n", " " + vero]) {
      expect(trovaPerToken(t), t).toBeNull();
    }
  });

  it("il confronto scorre tutti i link con timingSafeEqual, trovi o no: non si ferma al primo", () => {
    const link = Array.from({ length: 6 }, (_, i) => crea({ nome: `L${i}` }));
    const spia = vi.spyOn(crypto, "timingSafeEqual");
    trovaPerToken(tokenDi(link[0].url));
    expect(spia).toHaveBeenCalledTimes(6);
    spia.mockClear();
    trovaPerToken(crypto.randomBytes(32).toString("base64url"));
    expect(spia).toHaveBeenCalledTimes(6);
    spia.mockClear();
    trovaPerToken("non valido");
    expect(spia).toHaveBeenCalledTimes(6);
    // sempre su impronte della stessa lunghezza (timingSafeEqual con lunghezze diverse solleverebbe)
    for (const c of spia.mock.calls) expect(c[0].byteLength).toBe(c[1].byteLength);
  });

  it("revocare: l'indirizzo non vale più subito, il testo segreto si cancella dal database, la riga resta con il suo storico", () => {
    const c = crea({ nome: "Da revocare" });
    registraAccesso(c.id, ora);
    fissaOra(ora + 60_000);
    const r = ok(revocaCondivisione(c.id)).condivisione!;
    expect(r).toMatchObject({ id: c.id, nome: "Da revocare", url: null, revocato: ora + 60_000, accessi: 1, ultimo_accesso: ora });
    expect(trovaPerToken(tokenDi(c.url))).toBeNull();
    expect(archivio().prepare("SELECT token FROM condivisioni WHERE id = ?").pluck().get(c.id)).toBeNull();
    // rifarlo non cambia niente (e non sposta l'ora della revoca)
    fissaOra(ora + 120_000);
    expect(ok(revocaCondivisione(c.id)).condivisione!.revocato).toBe(ora + 60_000);
    expect(ok(elencoCondivisioni()).condivisioni[0].url).toBeNull();
  });

  it("un link revocato non si ruota", () => {
    const c = crea();
    revocaCondivisione(c.id);
    expect(ruotaCondivisione(c.id)).toEqual({ ok: false, error: "Il link è revocato: se ne crea uno nuovo." });
  });

  it("ruotare: il vecchio testo smette di valere subito, il nuovo vale, il resto non cambia (sale compreso, quindi gli identificativi degli eventi)", () => {
    const c = crea({ nome: "Da ruotare", livello: "titoli" });
    const vecchio = tokenDi(c.url);
    const saleVecchio = trovaPerToken(vecchio)?.sale;
    fissaOra(ora + 1000);
    const r = ok(ruotaCondivisione(c.id)).condivisione!;
    const nuovo = tokenDi(r.url);
    expect(nuovo).toMatch(FORMA_TOKEN);
    expect(nuovo).not.toBe(vecchio);
    expect(trovaPerToken(vecchio)).toBeNull();
    expect(trovaPerToken(nuovo)).toMatchObject({ id: c.id, nome: "Da ruotare", livello: "titoli", sale: saleVecchio });
    expect(r.aggiornato).toBeGreaterThan(c.aggiornato);
    expect(ruotaCondivisione("0123456789ab")).toEqual({ ok: false, error: "Link non trovato." });
  });

  it("eliminare toglie la riga qualunque sia lo stato, e un id che non c'è non cambia niente", () => {
    const a = crea({ nome: "A" });
    const b = crea({ nome: "B" });
    revocaCondivisione(b.id);
    expect(eliminaCondivisione(a.id)).toEqual({ ok: true });
    expect(eliminaCondivisione(b.id)).toEqual({ ok: true });
    expect(eliminaCondivisione("0123456789ab")).toEqual({ ok: true });
    expect(eliminaCondivisione("non un id")).toEqual({ ok: true });
    expect(ok(elencoCondivisioni()).condivisioni).toEqual([]);
    expect(trovaPerToken(tokenDi(a.url))).toBeNull();
  });
});

describe("l'ultimo accesso: una scrittura al minuto per link", () => {
  const riga = (id: string) => archivio().prepare("SELECT ultimo_accesso, accessi FROM condivisioni WHERE id = ?").get(id) as { ultimo_accesso: number | null; accessi: number };

  it("il primo accesso si scrive subito; quelli dopo, nel minuto, restano in memoria; poi tutti in una volta", () => {
    const c = crea();
    expect(riga(c.id)).toEqual({ ultimo_accesso: null, accessi: 0 });
    registraAccesso(c.id, ora);
    expect(riga(c.id)).toEqual({ ultimo_accesso: ora, accessi: 1 });
    registraAccesso(c.id, ora + 10_000);
    registraAccesso(c.id, ora + 30_000);
    registraAccesso(c.id, ora + INTERVALLO_ACCESSI_MS - 1);
    expect(riga(c.id)).toEqual({ ultimo_accesso: ora, accessi: 1 });
    registraAccesso(c.id, ora + INTERVALLO_ACCESSI_MS);
    expect(riga(c.id)).toEqual({ ultimo_accesso: ora + INTERVALLO_ACCESSI_MS, accessi: 5 });
  });

  it("l'elenco dice comunque il vero: aggiunge a quello che c'è nel database quello che aspetta in memoria", () => {
    const c = crea();
    registraAccesso(c.id, ora);
    registraAccesso(c.id, ora + 20_000);
    registraAccesso(c.id, ora + 40_000);
    expect(riga(c.id)).toEqual({ ultimo_accesso: ora, accessi: 1 });
    expect(ok(elencoCondivisioni()).condivisioni[0]).toMatchObject({ ultimo_accesso: ora + 40_000, accessi: 3 });
  });

  it("ogni link ha il suo minuto", () => {
    const a = crea({ nome: "A" });
    const b = crea({ nome: "B" });
    registraAccesso(a.id, ora);
    registraAccesso(b.id, ora + 1000);
    expect(riga(a.id).accessi).toBe(1);
    expect(riga(b.id).accessi).toBe(1);
  });

  it("un link eliminato non lascia accessi in memoria, e uno che non c'è più non fa un errore", () => {
    const c = crea();
    registraAccesso(c.id, ora);
    registraAccesso(c.id, ora + 5000);
    eliminaCondivisione(c.id);
    expect(() => registraAccesso(c.id, ora + 120_000)).not.toThrow();
    expect(ok(elencoCondivisioni()).condivisioni).toEqual([]);
  });

  it("se il database non risponde non solleva (chi legge il calendario non deve pagarne il conto) e lo scrive nel log senza il testo segreto", () => {
    const c = crea();
    const spia = vi.spyOn(archivio(), "prepare").mockImplementation(() => {
      throw new Error("database bloccato");
    });
    expect(() => registraAccesso(c.id, ora)).not.toThrow();
    spia.mockRestore();
    expect(registro.some((r) => r.includes("accesso non registrato"))).toBe(true);
    expect(registro.join("\n")).not.toContain(tokenDi(c.url));
  });
});

describe("quandoLetto", () => {
  it.each([
    [null, "mai"],
    [ora, "adesso"],
    [ora - 59_000, "adesso"],
    [ora - 60_000, "1 min fa"],
    [ora - 12 * 60_000, "12 min fa"],
    [ora - 59 * 60_000, "59 min fa"],
    [ora - 60 * 60_000, "1 ora fa"],
    [ora - 3 * 3_600_000, "3 ore fa"],
    [ora - 24 * 3_600_000, "1 giorno fa"],
    [ora - 50 * 3_600_000, "2 giorni fa"],
    [ora + 5000, "adesso"],
  ])("%j -> %s", (ms, atteso) => expect(quandoLetto(ms, ora)).toBe(atteso));
});

// ---- I metodi dell'API, attraverso il gateway con le sue regole ------------------------------------------------------
const METODI = ["get_condivisioni", "add_condivisione", "update_condivisione", "revoke_condivisione", "delete_condivisione", "rotate_condivisione"];
let chiamate: string[];
const deps = (utente: unknown): Deps => ({
  user: async () => utente,
  loghi: [],
  metodi: {
    ha: (nome) => METODI_TS.has(nome),
    chiama: (nome, args) => {
      chiamate.push(nome);
      return chiama(nome as NomeMetodoTs, args);
    },
  },
});
const req = (percorso: string, init: RequestInit = {}) => new Request("http://gateway:3000" + percorso, init);
const post = (percorso: string, corpo: unknown, headers: Record<string, string> = {}) =>
  req(percorso, { method: "POST", body: typeof corpo === "string" ? corpo : JSON.stringify(corpo), headers: { "content-type": "application/json", ...headers } });
const SESSIONE = { id: "u" };
const api = async (nome: string, args: unknown[] = []) => {
  const r = await handle(post(`/api/${nome}`, args), deps(SESSIONE));
  return { stato: r.status, corpo: await r.json() };
};

describe("i metodi dell'API", () => {
  beforeEach(() => {
    chiamate = [];
  });

  it("sono fra quelli che il gateway serve da sé", () => {
    for (const m of METODI) expect(METODI_TS.has(m), m).toBe(true);
  });

  it("senza sessione: 401 in JSON, e niente gira", async () => {
    for (const m of METODI) {
      const r = await handle(post(`/api/${m}`, m === "add_condivisione" ? [{ nome: "x" }] : []), deps(null));
      expect(r.status, m).toBe(401);
      expect(await r.json()).toEqual({ error: "non autenticato" });
    }
    expect(chiamate).toEqual([]);
    expect(archivio().prepare("SELECT COUNT(*) FROM sqlite_master WHERE name = 'condivisioni'").pluck().get()).toBe(0);
  });

  it("un POST che parte da un altro sito: 403; non JSON: 415; troppo grande: 413; gli altri metodi HTTP: 405; un corpo che non è una lista: 400", async () => {
    for (const m of METODI) {
      for (const sito of ["cross-site", "same-site"]) expect((await handle(post(`/api/${m}`, [], { "sec-fetch-site": sito }), deps(SESSIONE))).status, `${m} ${sito}`).toBe(403);
      expect((await handle(req(`/api/${m}`, { method: "POST", body: "[]", headers: { "content-type": "text/plain" } }), deps(SESSIONE))).status).toBe(415);
      expect((await handle(post(`/api/${m}`, "x".repeat(MAX_BODY + 1)), deps(SESSIONE))).status).toBe(413);
      for (const metodo of ["DELETE", "PUT", "PATCH", "OPTIONS"]) expect((await handle(req(`/api/${m}`, { method: metodo }), deps(SESSIONE))).status, `${m} ${metodo}`).toBe(405);
      for (const corpo of ['{"nome": "x"}', "5", "non json"]) expect((await handle(post(`/api/${m}`, corpo), deps(SESSIONE))).status, `${m} ${corpo}`).toBe(400);
      expect((await handle(req(`/api/${m}`), deps(SESSIONE))).status).toBe(404);
    }
    expect(chiamate).toEqual([]);
  });

  it("la risposta è JSON senza copia in cache", async () => {
    const r = await handle(post("/api/get_condivisioni", []), deps(SESSIONE));
    expect(r.headers.get("content-type")).toMatch(/^application\/json/);
    expect(r.headers.get("cache-control")).toBe("no-store");
  });

  it("dal giro completo: crea, elenca, cambia, ruota, revoca, elimina", async () => {
    expect(await api("get_condivisioni")).toEqual({ stato: 200, corpo: { ok: true, condivisioni: [], max: LINK_MAX } });
    const creato = await api("add_condivisione", [{ nome: "Samuele", livello: "titoli", nome_calendario: "Antonio" }]);
    expect(creato.stato).toBe(200);
    expect(creato.corpo.ok).toBe(true);
    const id = creato.corpo.condivisione.id as string;
    const vecchio = tokenDi(creato.corpo.condivisione.url);
    expect(vecchio).toMatch(FORMA_TOKEN);
    expect((await api("get_condivisioni")).corpo.condivisioni).toEqual([creato.corpo.condivisione]);

    const cambiato = await api("update_condivisione", [id, { giorni_avanti: 30 }]);
    expect(cambiato.corpo.condivisione).toMatchObject({ id, livello: "titoli", giorni_avanti: 30 });
    expect(await api("update_condivisione", [id, { giorni_avanti: 999 }])).toEqual({ stato: 200, corpo: { ok: false, error: "Giorni avanti: un numero intero da 1 a 365." } });

    const ruotato = await api("rotate_condivisione", [id]);
    const nuovo = tokenDi(ruotato.corpo.condivisione.url);
    expect(nuovo).not.toBe(vecchio);
    expect(trovaPerToken(vecchio)).toBeNull();
    expect(trovaPerToken(nuovo)?.id).toBe(id);

    expect((await api("revoke_condivisione", [id])).corpo.condivisione).toMatchObject({ id, url: null });
    expect(trovaPerToken(nuovo)).toBeNull();
    expect((await api("get_condivisioni")).corpo.condivisioni[0]).toMatchObject({ id, url: null });

    expect(await api("delete_condivisione", [id])).toEqual({ stato: 200, corpo: { ok: true } });
    expect((await api("get_condivisioni")).corpo.condivisioni).toEqual([]);
  });

  it("gli argomenti sbagliati non fanno un errore del server: un rifiuto in italiano, mai il testo segreto", async () => {
    const c = crea();
    const segreto = tokenDi(c.url);
    for (const [metodo, args] of [
      ["add_condivisione", []],
      ["add_condivisione", [null]],
      ["add_condivisione", [{ nome: "x", livello: {} }]],
      ["update_condivisione", []],
      ["update_condivisione", [c.id]],
      ["update_condivisione", [c.id, "x"]],
      ["update_condivisione", [{ id: c.id }, {}]],
      ["revoke_condivisione", [[c.id]]],
      ["rotate_condivisione", [segreto]],
      ["delete_condivisione", [segreto]],
    ] as [string, unknown[]][]) {
      const r = await api(metodo, args);
      expect(r.stato, `${metodo} ${JSON.stringify(args)}`).toBe(200);
      expect(JSON.stringify(r.corpo)).not.toContain(segreto);
    }
    // quelli che dovevano cambiare qualcosa non l'hanno cambiato
    expect(trovaPerToken(segreto)?.id).toBe(c.id);
    expect(ok(elencoCondivisioni()).condivisioni).toHaveLength(1);
  });

  it("il testo segreto non finisce in nessun log, né quando va bene né quando va male", async () => {
    const c = crea({ nome: "Log" });
    const segreto = tokenDi(c.url);
    await api("update_condivisione", [c.id, { livello: "titoli" }]);
    await api("update_condivisione", [c.id, { livello: "no" }]);
    const nuovo = tokenDi((await api("rotate_condivisione", [c.id])).corpo.condivisione.url);
    await api("revoke_condivisione", [c.id]);
    await api("delete_condivisione", [c.id]);
    expect(registro.length).toBeGreaterThan(0);
    for (const t of [segreto, nuovo, c.url as string]) expect(registro.join("\n")).not.toContain(t);
  });
});
