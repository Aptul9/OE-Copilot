// Prove dello script di inizializzazione (src-tauri/src/inizializzazione.js) senza finestra: l'ambiente della pagina è finto.
// node --test test/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const SCRIPT = readFileSync(new URL("../src-tauri/src/inizializzazione.js", import.meta.url), "utf8");
const ORIGINE = "https://oe.prova.test";
const MIN = 60_000;

function risposta(stato, corpo, { json = true, redirected = false } = {}) {
  return {
    status: stato,
    ok: stato >= 200 && stato < 300,
    redirected,
    headers: { get: (nome) => (nome.toLowerCase() === "content-type" ? (json ? "application/json; charset=utf-8" : "text/html") : null) },
    json: async () => corpo,
  };
}

/** Carica lo script in un ambiente finto: adesso (ms), le risposte dell'API per metodo, l'interruttore delle notifiche */
function ambiente({ adesso, api = {}, notificheAttive = true, origine = ORIGINE, protocollo = "https:", memoria = {}, permesso = true, esitoRichiesta = "granted" } = {}) {
  const timer = [];
  const notifiche = [];
  const chiamate = [];
  const sistema = { permesso, esitoRichiesta, richieste: 0, controlli: 0 };
  let ora = adesso;
  const DataFinta = class extends Date {
    constructor(...a) {
      super(...(a.length ? a : [ora]));
    }
    static now() {
      return ora;
    }
  };
  const sandbox = {
    Date: DataFinta,
    location: { origin: origine, protocol: protocollo, replace: (u) => chiamate.push(["replace", u]) },
    localStorage: {
      getItem: (k) => (k in memoria ? memoria[k] : null),
      setItem: (k, v) => {
        memoria[k] = String(v);
      },
    },
    setTimeout: (f, ms) => timer.push({ f, ms }),
    fetch: async (url, opzioni) => {
      const metodo = url.replace("/api/", "");
      chiamate.push([metodo, JSON.parse(opzioni.body)]);
      const r = api[metodo];
      if (r === undefined) return risposta(404, { error: "metodo non disponibile" });
      if (r instanceof Error) throw r;
      return typeof r === "function" ? r() : r;
    },
  };
  sandbox.window = sandbox;
  sandbox.__OE_ORIGINE__ = ORIGINE;
  sandbox.__OE_AVVIO__ = "http://tauri.localhost/index.html";
  sandbox.__TAURI_INTERNALS__ = {
    invoke: async (comando, argomenti) => {
      if (comando === "notifiche_attive") return notificheAttive;
      if (comando === "plugin:notification|is_permission_granted") {
        sistema.controlli += 1;
        return sistema.permesso;
      }
      if (comando === "plugin:notification|request_permission") {
        sistema.richieste += 1;
        if (sistema.esitoRichiesta === "granted") sistema.permesso = true;
        return sistema.esitoRichiesta;
      }
      if (comando === "plugin:notification|notify") {
        notifiche.push(JSON.parse(JSON.stringify(argomenti.options)));
        return null;
      }
      throw new Error("comando non previsto " + comando);
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(SCRIPT, sandbox);
  return {
    sandbox,
    timer,
    notifiche,
    chiamate,
    memoria,
    sistema,
    imposta: (ms) => (ora = ms),
    /** Esegue il prossimo giro e aspetta che ne programmi un altro */
    async giro() {
      const atteso = timer.length;
      const { f } = timer[timer.length - 1];
      f();
      for (let i = 0; i < 50 && timer.length === atteso; i++) await new Promise((r) => setImmediate(r));
      assert.equal(timer.length, atteso + 1, "il giro non è finito");
      return timer[timer.length - 1].ms;
    },
    stato: () => JSON.parse(JSON.stringify(sandbox.__oeNotifiche)),
  };
}

const evento = (id, titolo, start, extra = {}) => ({ id, title: titolo, start, end: start, allDay: false, location: "", ...extra });
const json = (eventi, extra = {}) => risposta(200, { events: eventi, errors: [], ...extra });

test("fuori dall'origine del sito lo script non fa niente", () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 8, 0), origine: "http://tauri.localhost", protocollo: "http:" });
  assert.equal(a.timer.length, 1);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, []);
  assert.equal(a.sandbox.__oeNotifiche, undefined);
});

test("nella pagina di errore del browser torna alla pagina di avvio", () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 8, 0), origine: "null", protocollo: "chrome-error:" });
  assert.equal(a.timer.length, 1);
  assert.equal(a.timer[0].ms, 1500);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, [["replace", "http://tauri.localhost/index.html"]]);
});

test("un evento fra 5 minuti avvisa una volta sola, con ora e luogo (ora di Roma, estate)", async () => {
  // 09:55 a Roma (UTC+2) = 07:55 UTC, evento alle 10:00 senza scarto
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00", { location: "Sala 1" })]) } });
  assert.deepEqual(a.timer.map((t) => t.ms), [1500, 3000]);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, []);
  assert.equal(await a.giro(), 60_000);
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.notifiche[0], { title: "Tra 5 min · Riunione A", body: "Alle 10:00\nSala 1" });
  assert.equal(await a.giro(), 60_000);
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.stato(), { giri: 2, inviate: 1, errori: 0, esito: "ok" });
});

test("stesso evento in inverno (UTC+1) e con scarto esplicito", async () => {
  const inverno = ambiente({ adesso: Date.UTC(2026, 11, 1, 8, 55), api: { get_events: json([evento("e1", "Riunione B", "2026-12-01T10:00:00")]) } });
  await inverno.giro();
  assert.equal(inverno.notifiche[0].title, "Tra 5 min · Riunione B");
  const scarto = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_events: json([evento("e2", "Riunione C", "2026-10-12T10:00:00+02:00"), evento("e3", "Riunione D", "2026-10-12T08:00:00Z")]) } });
  await scarto.giro();
  assert.deepEqual(scarto.notifiche.map((n) => n.title), ["Tra 5 min · Riunione C", "Tra 5 min · Riunione D"]);
});

test("il link per entrare non finisce nell'avviso", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_events: json([evento("e1", "Riunione E", "2026-10-12T10:00:00", { location: "https://teams.microsoft.com/l/meetup-join/x" })]) } });
  await a.giro();
  assert.equal(a.notifiche[0].body, "Alle 10:00");
});

test("non avvisano: giornata intera, nascosti, già iniziati, troppo lontani, senza orario", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({
    adesso,
    api: {
      get_events: json([
        evento("a", "Tutto il giorno", "2026-10-12T00:00:00", { allDay: true }),
        evento("b", "Nascosto", "2026-10-12T10:00:00", { nascosto: true }),
        evento("c", "Già iniziato", "2026-10-12T09:50:00"),
        evento("d", "Lontano", "2026-10-12T11:00:00"),
        evento("e", "Senza orario", ""),
        evento("f", "Orario illeggibile", "domani"),
      ]),
    },
  });
  await a.giro();
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.stato().esito, "ok");
});

test("la memoria degli avvisi già dati sopravvive al ricaricamento e si ripulisce", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const memoria = {};
  const api = { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) };
  const a = ambiente({ adesso, api, memoria });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  const salvate = JSON.parse(memoria["oe-app:notifiche-inviate"]);
  assert.deepEqual(Object.keys(salvate), ["e1|2026-10-12T10:00:00|10"]);
  const b = ambiente({ adesso, api, memoria });
  await b.giro();
  assert.equal(b.notifiche.length, 0);
  memoria["oe-app:notifiche-inviate"] = JSON.stringify({ vecchia: adesso - 2 * 86_400_000, "e1|2026-10-12T10:00:00|10": adesso + 5 * MIN });
  const c = ambiente({ adesso, api: { get_events: json([]) }, memoria });
  await c.giro();
  assert.deepEqual(Object.keys(JSON.parse(memoria["oe-app:notifiche-inviate"])), ["e1|2026-10-12T10:00:00|10"]);
});

test("get_promemoria: anticipi del server, una notifica per riunione e anticipo, attivo=false non spegne l'app", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0); // 10:00 a Roma
  const api = {
    get_events: json([evento("e1", "Riunione F", "2026-10-12T10:00:00")]),
    get_promemoria: risposta(200, { promemoria: { attivo: false, minuti: [30, 10, 10, "x", -5, 0], canali: ["telegram"], contenuto: "minimo" }, canali: [], ultimo_giro: null }),
  };
  const a = ambiente({ adesso: inizio - 20 * MIN, api });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.notifiche[0].title, "Tra 20 min · Riunione F");
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  a.imposta(inizio - 9 * MIN);
  await a.giro();
  assert.equal(a.notifiche.length, 2);
  assert.equal(a.notifiche[1].title, "Tra 9 min · Riunione F");
});

test("app appena avviata con più anticipi già passati: un avviso solo", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  const api = { get_events: json([evento("e1", "Riunione G", "2026-10-12T10:00:00")]), get_promemoria: risposta(200, { minuti: [30, 10, 5] }) };
  const a = ambiente({ adesso: inizio - 3 * MIN, api });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  await a.giro();
  assert.equal(a.notifiche.length, 1);
});

test("senza get_promemoria (404) l'anticipo è la costante di 10 minuti", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  const api = { get_events: json([evento("e1", "Riunione H", "2026-10-12T10:00:00")]) };
  const lontano = ambiente({ adesso: inizio - 11 * MIN, api });
  await lontano.giro();
  assert.equal(lontano.notifiche.length, 0);
  const vicino = ambiente({ adesso: inizio - 10 * MIN, api });
  await vicino.giro();
  assert.equal(vicino.notifiche.length, 1);
});

test("sessione finita (401, pagina di accesso, reindirizzamento): si smette e si riprova dopo 5 minuti", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  for (const r of [risposta(401, { error: "non autenticato" }), risposta(200, null, { json: false }), risposta(200, {}, { redirected: true })]) {
    const a = ambiente({ adesso, api: { get_events: r } });
    assert.equal(await a.giro(), 300_000);
    assert.equal(a.stato().esito, "non-autenticato");
    assert.equal(a.notifiche.length, 0);
    assert.equal(a.chiamate.filter(([m]) => m === "get_promemoria").length, 0);
  }
});

test("errori di rete: l'attesa raddoppia fino a 5 minuti e si azzera al primo successo", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  let ok = false;
  const a = ambiente({ adesso, api: { get_events: () => (ok ? json([]) : Promise.reject(new Error("rete"))) } });
  assert.deepEqual([await a.giro(), await a.giro(), await a.giro(), await a.giro()], [120_000, 240_000, 300_000, 300_000]);
  ok = true;
  assert.equal(await a.giro(), 60_000);
  assert.deepEqual(a.stato(), { giri: 5, inviate: 0, errori: 0, esito: "ok" });
});

test("5xx: errore con attesa crescente, niente avvisi", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_events: risposta(502, { error: "agente" }) } });
  assert.equal(await a.giro(), 120_000);
  assert.equal(a.stato().esito, "errore");
});

test("dati in cache (stale): nessun avviso", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_events: json([evento("e1", "Riunione I", "2026-10-12T10:00:00")], { stale: true }) } });
  await a.giro();
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.stato().esito, "dati-non-freschi");
});

test("lanciato due volte nella stessa pagina (WebView di Android) lavora una volta sola", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), permesso: null, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
  const timerPrima = a.timer.length;
  const contatori = a.sandbox.__oeNotifiche;
  vm.runInContext(SCRIPT, a.sandbox);
  assert.equal(a.timer.length, timerPrima);
  assert.equal(a.sandbox.__oeNotifiche, contatori);
  await a.giro();
  assert.equal(a.chiamate.filter(([metodo]) => metodo === "get_events").length, 1);
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 1);
});

test("interruttore Notifiche spento: nessuna richiesta al sito", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), notificheAttive: false, api: { get_events: json([evento("e1", "Riunione L", "2026-10-12T10:00:00")]) } });
  assert.equal(await a.giro(), 60_000);
  assert.equal(a.chiamate.length, 0);
  assert.equal(a.stato().esito, "spente");
});

test("la richiesta è la stessa della pagina: POST get_events con inizio e fine ISO a 24 ore", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55, 30);
  const a = ambiente({ adesso, api: { get_events: json([]) } });
  await a.giro();
  const [metodo, argomenti] = a.chiamate[0];
  assert.equal(metodo, "get_events");
  assert.equal(argomenti.length, 2);
  assert.match(argomenti[0], /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$/);
  assert.equal(new Date(argomenti[1]).getTime() - new Date(argomenti[0]).getTime(), 24 * 3_600_000);
});

test("permesso di notifica da chiedere (Android 13+): si chiede una volta, poi l'avviso parte", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, permesso: null, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
  await a.giro();
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 1);
  await a.giro();
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 1);
});

test("permesso rifiutato: niente avviso, niente nuova richiesta, l'avviso parte quando il permesso arriva", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, permesso: null, esitoRichiesta: "denied", api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
  await a.giro();
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.stato().esito, "senza-permesso");
  await a.giro();
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 0);
  a.sistema.permesso = true;
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.stato().esito, "ok");
});

test("permesso già negato per sempre (false): nessuna richiesta", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, permesso: false, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
  await a.giro();
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.stato().esito, "senza-permesso");
});

test("permesso già concesso (PC): nessuna richiesta, il controllo non si ripete dopo il primo sì", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
  await a.giro();
  await a.giro();
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.sistema.controlli, 1);
  assert.equal(a.notifiche.length, 1);
});

test("senza sessione il permesso non si chiede", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, permesso: null, api: { get_events: risposta(401, { error: "non autenticato" }) } });
  await a.giro();
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.sistema.controlli, 0);
});