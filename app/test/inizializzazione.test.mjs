// Prove dello script di inizializzazione (src-tauri/src/inizializzazione.js) senza finestra: l'ambiente della pagina è finto.
// node --test test/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const SCRIPT = readFileSync(new URL("../src-tauri/src/inizializzazione.js", import.meta.url), "utf8");
const ORIGINE = "https://oe.prova.test";
const MIN = 60_000;
const UA_PC = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";
const UA_ANDROID = "Mozilla/5.0 (Linux; Android 13; sdk_gphone64_x86_64 Build/TE1A.220922.034; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/141.0.0.0 Mobile Safari/537.36";

function risposta(stato, corpo, { json = true, redirected = false } = {}) {
  return {
    status: stato,
    ok: stato >= 200 && stato < 300,
    redirected,
    headers: { get: (nome) => (nome.toLowerCase() === "content-type" ? (json ? "application/json; charset=utf-8" : "text/html") : null) },
    json: async () => corpo,
  };
}

/** La risposta di get_promemoria del server con queste impostazioni (quelle che mancano le completa il server). */
const promemoria = (p) =>
  risposta(200, { promemoria: { minuti: [10], contenuto: "completo", suono: "Default", notifica_windows: true, comparsa: true, ...p }, canali: [], ultimo_errore: null, ultimo_errore_il: null });

/**
 * Carica lo script in un ambiente finto: adesso (ms), le risposte dell'API per metodo, i file dei loghi (indirizzo -> contenuto), il
 * browser (PC o Android)
 */
function ambiente({ adesso, api = {}, avvisoRotto = false, notificaRotta = false, loghi = {}, origine = ORIGINE, protocollo = "https:", memoria = {}, permesso = true, esitoRichiesta = "granted", ua = UA_PC } = {}) {
  const timer = [];
  // le notifiche mostrate, e da chi: "app" (notifica.rs, sul PC) o "plugin"
  const notifiche = [];
  const vie = [];
  // le righe dell'avviso a comparsa, e quante ne porta ogni chiamata
  const avvisi = [];
  const chiamateAvviso = [];
  // il suono che ogni chiamata dell'avviso chiede all'app
  const suoniAvviso = [];
  const chiamate = [];
  // tutto quello che lo script passa all'app, per cercarci dentro un testo
  const passati = [];
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
    navigator: { userAgent: ua },
    location: { origin: origine, protocol: protocollo, replace: (u) => chiamate.push(["replace", u]) },
    localStorage: {
      getItem: (k) => (k in memoria ? memoria[k] : null),
      setItem: (k, v) => {
        memoria[k] = String(v);
      },
    },
    setTimeout: (f, ms) => timer.push({ f, ms }),
    URL,
    // come quello del browser, per i finti contenuti dei loghi qui sotto
    FileReader: class {
      readAsDataURL(b) {
        this.result = `data:${b.type};base64,${Buffer.from(b.dati).toString("base64")}`;
        queueMicrotask(() => this.onload());
      }
    },
    fetch: async (url, opzioni) => {
      if (url.startsWith("/logos/")) {
        chiamate.push(["logo", url]);
        return url in loghi ? { ok: true, status: 200, blob: async () => ({ type: "image/png", dati: loghi[url] }) } : risposta(404, { error: "non trovato" });
      }
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
      passati.push(JSON.stringify([comando, argomenti ?? null]));
      if (comando === "mostra_avviso") {
        if (avvisoRotto) throw new Error("finestra non creata");
        avvisi.push(...JSON.parse(JSON.stringify(argomenti.avvisi)));
        chiamateAvviso.push(argomenti.avvisi.length);
        suoniAvviso.push(argomenti.suono);
        return null;
      }
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
        vie.push("plugin");
        return null;
      }
      if (comando === "mostra_notifica") {
        if (notificaRotta) throw new Error("notifica non mostrata");
        notifiche.push(JSON.parse(JSON.stringify(argomenti.dati)));
        vie.push("app");
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
    vie,
    avvisi,
    chiamateAvviso,
    suoniAvviso,
    chiamate,
    passati,
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
// la sola notifica, senza l'avviso a comparsa (acceso di norma sul PC)
const soloNotifica = promemoria({ comparsa: false });

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
  const a = ambiente({ adesso, api: { get_promemoria: soloNotifica, get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00", { location: "Sala 1" })]) } });
  assert.deepEqual(a.timer.map((t) => t.ms), [1500, 3000]);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, []);
  assert.equal(await a.giro(), 60_000);
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.notifiche[0], { title: "Tra 5 min · Riunione A", body: "Alle 10:00\nSala 1", sound: "Default", chiave: "e1|2026-10-12T10:00:00", inizio: Date.UTC(2026, 9, 12, 8, 0), logo: null });
  assert.deepEqual(a.vie, ["app"]);
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
  assert.equal(a.avvisi.length, 0);
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

test("get_promemoria: anticipi del server, valori strani scartati, una notifica per riunione e anticipo", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0); // 10:00 a Roma
  const api = { get_events: json([evento("e1", "Riunione F", "2026-10-12T10:00:00")]), get_promemoria: promemoria({ minuti: [30, 10, 10, "x", -5, 0] }) };
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

test("le impostazioni si rileggono a ogni giro: un cambio sul sito vale dal giro dopo", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  let impostazioni = { minuti: [5], notifica_windows: false, comparsa: false };
  const api = { get_events: json([evento("e1", "Riunione R", "2026-10-12T10:00:00")]), get_promemoria: () => promemoria(impostazioni) };
  const a = ambiente({ adesso: inizio - 10 * MIN, api });
  await a.giro();
  assert.equal(a.stato().esito, "spente");
  assert.equal(a.chiamate.filter(([m]) => m === "get_events").length, 0);
  impostazioni = { minuti: [10], notifica_windows: true, comparsa: false };
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.chiamate.filter(([m]) => m === "get_promemoria").length, 2);
  assert.equal(a.avvisi.length, 0);
});

test("all'inizio (0 minuti): nulla a due minuti, un avviso nell'ultimo minuto e mezzo, e dice Adesso a meno di mezzo minuto", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  const api = { get_events: json([evento("e1", "Riunione H", "2026-10-12T10:00:00")]), get_promemoria: promemoria({ minuti: [0] }) };
  const a = ambiente({ adesso: inizio - 2 * MIN, api });
  await a.giro();
  assert.equal(a.notifiche.length, 0);
  a.imposta(inizio - 20_000);
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.notifiche[0].title, "Adesso · Riunione H");
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  const presto = ambiente({ adesso: inizio - 85_000, api });
  await presto.giro();
  assert.equal(presto.notifiche[0].title, "Tra 1 min · Riunione H");
});

test("un suono solo: con l'avviso a comparsa lo fa l'avviso e la notifica è muta, senza avviso la notifica; Default se il nome è strano o manca", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  const prova = async (impostazioni) => {
    const api = { get_events: json([evento("e1", "Riunione S", "2026-10-12T10:00:00")]) };
    if (impostazioni !== undefined) api.get_promemoria = promemoria(impostazioni);
    const a = ambiente({ adesso: inizio - 5 * MIN, api });
    await a.giro();
    assert.equal(a.notifiche.length, 1);
    return { notifica: a.notifiche[0], avviso: a.suoniAvviso };
  };
  assert.equal((await prova({ comparsa: false, suono: "Reminder" })).notifica.sound, "Reminder");
  assert.equal((await prova({ comparsa: false, suono: "Alarm3" })).notifica.sound, "Alarm3");
  for (const suono of ["Default", "", "../x", "troppo-lungo-e-con-trattini", 5, null]) {
    assert.equal((await prova({ comparsa: false, suono })).notifica.sound, "Default", String(suono));
  }
  const insieme = await prova({ suono: "Reminder" });
  assert.equal("sound" in insieme.notifica, false);
  assert.deepEqual(insieme.avviso, ["Reminder"]);
  assert.deepEqual((await prova({ suono: "../x" })).avviso, ["Default"]);
  // senza get_promemoria: notifica e avviso accesi, suono predefinito all'avviso
  const vecchio = await prova(undefined);
  assert.equal("sound" in vecchio.notifica, false);
  assert.deepEqual(vecchio.avviso, ["Default"]);
});

test("app appena avviata con più anticipi già passati: un avviso solo", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  const api = { get_events: json([evento("e1", "Riunione G", "2026-10-12T10:00:00")]), get_promemoria: promemoria({ minuti: [30, 10, 5] }) };
  const a = ambiente({ adesso: inizio - 3 * MIN, api });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  await a.giro();
  assert.equal(a.notifiche.length, 1);
});

test("senza get_promemoria (404) valgono i valori di partenza: 10 minuti, notifica e avviso a comparsa", async () => {
  const inizio = Date.UTC(2026, 9, 12, 8, 0);
  const api = { get_events: json([evento("e1", "Riunione H", "2026-10-12T10:00:00")]) };
  const lontano = ambiente({ adesso: inizio - 11 * MIN, api });
  await lontano.giro();
  assert.equal(lontano.notifiche.length, 0);
  const vicino = ambiente({ adesso: inizio - 10 * MIN, api });
  await vicino.giro();
  assert.equal(vicino.notifiche.length, 1);
  assert.equal(vicino.avvisi.length, 1);
});

test("sessione finita (401, pagina di accesso, reindirizzamento): si smette e si riprova dopo 5 minuti", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  for (const r of [risposta(401, { error: "non autenticato" }), risposta(200, null, { json: false }), risposta(200, {}, { redirected: true })]) {
    // già alla lettura delle impostazioni: gli eventi non si chiedono
    const a = ambiente({ adesso, api: { get_promemoria: r, get_events: r } });
    assert.equal(await a.giro(), 300_000);
    assert.equal(a.stato().esito, "non-autenticato");
    assert.equal(a.notifiche.length, 0);
    assert.equal(a.chiamate.filter(([m]) => m === "get_events").length, 0);
    // o alla lettura degli eventi
    const b = ambiente({ adesso, api: { get_promemoria: soloNotifica, get_events: r } });
    assert.equal(await b.giro(), 300_000);
    assert.equal(b.stato().esito, "non-autenticato");
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
  const b = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_promemoria: risposta(502, { error: "gateway" }), get_events: json([evento("e1", "Riunione Z", "2026-10-12T10:00:00")]) } });
  assert.equal(await b.giro(), 120_000);
  assert.equal(b.stato().esito, "errore");
  assert.equal(b.notifiche.length, 0);
});

test("dati in cache (stale): nessun avviso", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_events: json([evento("e1", "Riunione I", "2026-10-12T10:00:00")], { stale: true }) } });
  await a.giro();
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.avvisi.length, 0);
  assert.equal(a.stato().esito, "dati-non-freschi");
});

test("lanciato due volte nella stessa pagina (WebView di Android) lavora una volta sola", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), permesso: null, ua: UA_ANDROID, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
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

test("notifica e avviso a comparsa spenti sul sito: si leggono solo le impostazioni", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_promemoria: promemoria({ notifica_windows: false, comparsa: false }), get_events: json([evento("e1", "Riunione L", "2026-10-12T10:00:00")]) } });
  assert.equal(await a.giro(), 60_000);
  assert.deepEqual(a.chiamate.map(([m]) => m), ["get_promemoria"]);
  assert.equal(a.stato().esito, "spente");
});

test("su Android le notifiche ci sono sempre e l'avviso a comparsa mai, qualunque cosa dica il sito", async () => {
  const api = { get_promemoria: promemoria({ notifica_windows: false, comparsa: true }), get_events: json([evento("e1", "Riunione M", "2026-10-12T10:00:00")]) };
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api, ua: UA_ANDROID });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.chiamateAvviso, []);
  assert.equal(a.chiamate.filter(([m]) => m === "get_personalizzazione").length, 0);
});

test("avviso a comparsa: ora, titolo, logo dell'azienda come data: e sigla, una volta sola, accanto alla notifica", async () => {
  const api = {
    get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00", { alias: "ROAR", account: "a@roarinc.com", color: "#2563eb" })]),
    get_personalizzazione: risposta(200, { interfaccia: { loghi: { ROAR: "logos/roar.png?v=7" } } }),
    get_promemoria: promemoria({}),
  };
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api, loghi: { "/logos/roar.png?v=7": "PNG" } });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.avvisi, [
    { chiave: "e1|2026-10-12T10:00:00", inizio: Date.UTC(2026, 9, 12, 8, 0), ora: "10:00", titolo: "Riunione A", logo: "data:image/png;base64,UE5H", sigla: "RO", colore: "#2563eb" },
  ]);
  await a.giro();
  assert.equal(a.avvisi.length, 1);
  assert.deepEqual(a.chiamate.filter(([m]) => m === "logo" || m === "get_personalizzazione").map(([m]) => m), ["get_personalizzazione", "logo"]);
});

test("avviso a comparsa senza logo: la sigla scelta o le iniziali; un logo fuori dal sito non si legge; gli Eventi personali hanno quello del gateway; le riunioni dello stesso giro vanno insieme", async () => {
  const api = {
    get_events: json([
      evento("e1", "Riunione B", "2026-10-12T10:00:00", { alias: "Beta Servizi" }),
      evento("e2", "Riunione C", "2026-10-12T10:00:00", { alias: "Gamma" }),
      evento("e3", "Dentista", "2026-10-12T10:00:00", { alias: "Eventi personali" }),
    ]),
    get_personalizzazione: risposta(200, { interfaccia: { loghi: { Gamma: "https://altro.example/x.png" }, sigle: { Gamma: "GM" } } }),
  };
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api, loghi: { "/logos/personale.png": "IO" } });
  await a.giro();
  assert.deepEqual(a.avvisi.map((v) => [v.sigla, v.logo]), [["BS", null], ["GM", null], ["EP", "data:image/png;base64,SU8="]]);
  assert.deepEqual(a.chiamate.filter(([m]) => m === "logo").map(([, u]) => u), ["/logos/personale.png"]);
  assert.deepEqual(a.chiamateAvviso, [3]);
});

test("solo l'avviso a comparsa (notifica spenta): nessuna notifica; un avviso che non riesce non fa ripetere la notifica", async () => {
  const api = { get_promemoria: promemoria({ notifica_windows: false }), get_events: json([evento("e1", "Riunione L", "2026-10-12T10:00:00")]) };
  const solo = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api });
  await solo.giro();
  assert.equal(solo.notifiche.length, 0);
  assert.equal(solo.avvisi.length, 1);
  assert.equal(solo.stato().esito, "ok");
  const rotto = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api: { get_events: api.get_events }, avvisoRotto: true });
  await rotto.giro();
  await rotto.giro();
  assert.equal(rotto.notifiche.length, 1);
  assert.equal(rotto.stato().esito, "ok");
});

test("avviso a comparsa spento: solo la notifica, con il logo dell'azienda", async () => {
  const api = {
    get_promemoria: promemoria({ comparsa: false }),
    get_events: json([evento("e1", "Riunione N", "2026-10-12T10:00:00", { alias: "ROAR" })]),
    get_personalizzazione: risposta(200, { interfaccia: { loghi: { ROAR: "logos/roar.png?v=7" } } }),
  };
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api, loghi: { "/logos/roar.png?v=7": [1, 2, 3] } });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.notifiche[0].logo, "data:image/png;base64,AQID");
  assert.deepEqual(a.chiamateAvviso, []);
});

test("sul PC la notifica la fa l'app, con la chiave della riunione e l'inizio; se non riesce la fa il plugin; su Android sempre il plugin", async () => {
  const api = { get_promemoria: soloNotifica, get_events: json([evento("e1", "Riunione P", "2026-10-12T10:00:00")]) };
  const rotta = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api, notificaRotta: true });
  await rotta.giro();
  assert.deepEqual(rotta.vie, ["plugin"]);
  assert.deepEqual(rotta.notifiche, [{ title: "Tra 5 min · Riunione P", body: "Alle 10:00", sound: "Default" }]);
  const android = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api, ua: UA_ANDROID });
  await android.giro();
  assert.deepEqual(android.vie, ["plugin"]);
  assert.equal(android.chiamate.filter(([m]) => m === "get_personalizzazione").length, 0);
});

test("Solo ora e azienda: la notifica ha l'azienda al posto del titolo e niente luogo, l'avviso a comparsa niente titolo; il titolo non arriva mai all'app", async () => {
  const api = {
    get_promemoria: promemoria({ contenuto: "minimo" }),
    get_events: json([
      evento("e1", "Colloquio riservato", "2026-10-12T10:00:00", { alias: "ROAR", location: "Sala del consiglio" }),
      evento("e2", "Visita privata", "2026-10-12T10:00:00", { alias: "" }),
    ]),
  };
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55), api });
  await a.giro();
  assert.deepEqual(
    a.notifiche.map(({ title, body }) => ({ title, body })),
    [
      { title: "Tra 5 min · ROAR", body: "Alle 10:00" },
      { title: "Tra 5 min", body: "Alle 10:00" },
    ],
  );
  assert.deepEqual(a.avvisi.map((v) => [v.titolo, v.ora, v.sigla]), [["", "10:00", "RO"], ["", "10:00", ""]]);
  const tutto = a.passati.join("\n");
  for (const segreto of ["Colloquio", "riservato", "Visita privata", "Sala del consiglio"]) assert.equal(tutto.includes(segreto), false, segreto);
});

test("la richiesta è la stessa della pagina: POST get_events con inizio e fine ISO a 24 ore", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55, 30);
  const a = ambiente({ adesso, api: { get_events: json([]) } });
  await a.giro();
  const [metodo, argomenti] = a.chiamate.find(([m]) => m === "get_events");
  assert.equal(metodo, "get_events");
  assert.equal(argomenti.length, 2);
  assert.match(argomenti[0], /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$/);
  assert.equal(new Date(argomenti[1]).getTime() - new Date(argomenti[0]).getTime(), 24 * 3_600_000);
  assert.deepEqual(a.chiamate.find(([m]) => m === "get_promemoria"), ["get_promemoria", []]);
});

test("permesso di notifica da chiedere (Android 13+): si chiede una volta, poi l'avviso parte", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, permesso: null, ua: UA_ANDROID, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
  await a.giro();
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 1);
  await a.giro();
  assert.equal(a.sistema.richieste, 1);
  assert.equal(a.notifiche.length, 1);
});

test("permesso rifiutato: niente avviso, niente nuova richiesta, l'avviso parte quando il permesso arriva", async () => {
  const adesso = Date.UTC(2026, 9, 12, 7, 55);
  const a = ambiente({ adesso, permesso: null, esitoRichiesta: "denied", ua: UA_ANDROID, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
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
  const a = ambiente({ adesso, permesso: false, ua: UA_ANDROID, api: { get_events: json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]) } });
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
  const a = ambiente({ adesso, permesso: null, ua: UA_ANDROID, api: { get_events: risposta(401, { error: "non autenticato" }) } });
  await a.giro();
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.sistema.controlli, 0);
});
