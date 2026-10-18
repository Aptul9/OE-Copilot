// Prove dello script di inizializzazione (src-tauri/src/inizializzazione.js) senza finestra: l'ambiente della pagina è finto, e anche il
// nucleo dei promemoria dell'app (pianifica_promemoria): qui le voci sono date dalla prova, l'ora di Roma e i testi li prova cargo test
// (src-tauri/src/promemoria.rs).
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
// 10:00 a Roma (UTC+2) e 09:55, cinque minuti prima
const DIECI = Date.UTC(2026, 9, 12, 8, 0);
const ADESSO = DIECI - 5 * MIN;

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

/** Una voce come la dà il nucleo dell'app: la riunione `id` alle 10:00, l'anticipo, e quello che serve alla prova */
function voce(id, minuti = 10, extra = {}) {
  const inizio = extra.inizio ?? DIECI;
  const riunione = `${id}|2026-10-12T10:00:00`;
  return {
    chiave: `${riunione}|${minuti}`,
    riunione,
    minuti,
    at: inizio - minuti * MIN,
    inizio,
    ora: "10:00",
    quando: "Tra 5 min",
    chi: `Riunione ${id}`,
    titolo: `Tra 5 min · Riunione ${id}`,
    corpo: "Alle 10:00",
    azienda: "",
    colore: "",
    allarme: 0,
    notifica: 0,
    ...extra,
  };
}

/**
 * Carica lo script in un ambiente finto: adesso (ms), le risposte dell'API per metodo, le voci del nucleo (una lista, o una funzione
 * degli argomenti), i file dei loghi (indirizzo -> contenuto), il browser (PC o Android)
 */
function ambiente({ adesso = ADESSO, api = {}, voci = [], avvisoRotto = false, notificaRotta = false, loghi = {}, origine = ORIGINE, protocollo = "https:", memoria = {}, permesso = true, esitoRichiesta = "granted", ua = UA_PC } = {}) {
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
  // gli argomenti di ogni chiamata al nucleo
  const pianifica = [];
  // tutto quello che lo script passa all'app per mostrarlo, per cercarci dentro un testo
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
  const copia = (x) => JSON.parse(JSON.stringify(x));
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
      if (comando === "pianifica_promemoria") {
        pianifica.push(copia(argomenti));
        return copia(typeof voci === "function" ? voci(argomenti) : voci);
      }
      passati.push(JSON.stringify([comando, argomenti ?? null]));
      if (comando === "mostra_avviso") {
        if (avvisoRotto) throw new Error("finestra non creata");
        avvisi.push(...copia(argomenti.avvisi));
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
        notifiche.push(copia(argomenti.options));
        vie.push("plugin");
        return null;
      }
      if (comando === "mostra_notifica") {
        if (notificaRotta) throw new Error("notifica non mostrata");
        notifiche.push(copia(argomenti.dati));
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
    pianifica,
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
    /** Esegue l'ultimo giro (su Android non ne programma un altro) */
    async ultimoGiro() {
      const atteso = timer.length;
      timer[timer.length - 1].f();
      for (let i = 0; i < 50; i++) await new Promise((r) => setImmediate(r));
      assert.equal(timer.length, atteso, "è partito un altro giro");
    },
    stato: () => copia(sandbox.__oeNotifiche),
  };
}

const evento = (id, titolo, start, extra = {}) => ({ id, title: titolo, start, end: start, allDay: false, location: "", ...extra });
const json = (eventi, extra = {}) => risposta(200, { events: eventi, errors: [], ...extra });
const EVENTI = json([evento("e1", "Riunione A", "2026-10-12T10:00:00")]);
// la sola notifica, senza l'avviso a comparsa (acceso di norma sul PC)
const soloNotifica = promemoria({ comparsa: false });

test("fuori dall'origine del sito lo script non fa niente", () => {
  const a = ambiente({ origine: "http://tauri.localhost", protocollo: "http:" });
  assert.equal(a.timer.length, 1);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, []);
  assert.equal(a.sandbox.__oeNotifiche, undefined);
});

test("nella pagina di errore del browser torna alla pagina di avvio", () => {
  const a = ambiente({ origine: "null", protocollo: "chrome-error:" });
  assert.equal(a.timer.length, 1);
  assert.equal(a.timer[0].ms, 1500);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, [["replace", "http://tauri.localhost/index.html"]]);
});

test("una riunione con l'anticipo raggiunto avvisa una volta sola, con il testo del nucleo", async () => {
  const a = ambiente({ api: { get_promemoria: soloNotifica, get_events: EVENTI }, voci: [voce("e1", 10, { corpo: "Alle 10:00\nSala 1" })] });
  assert.deepEqual(a.timer.map((t) => t.ms), [1500, 3000]);
  a.timer[0].f();
  assert.deepEqual(a.chiamate, []);
  assert.equal(await a.giro(), 60_000);
  assert.deepEqual(a.notifiche, [{ title: "Tra 5 min · Riunione e1", body: "Alle 10:00\nSala 1", sound: "Default", chiave: "e1|2026-10-12T10:00:00", inizio: DIECI, logo: null }]);
  assert.deepEqual(a.vie, ["app"]);
  assert.equal(await a.giro(), 60_000);
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.stato(), { giri: 2, inviate: 1, errori: 0, esito: "ok" });
});

test("il nucleo riceve le impostazioni del server così come sono, gli eventi letti e l'ora del giro; senza get_promemoria nessuna impostazione", async () => {
  const p = { minuti: [10, 30], contenuto: "minimo", suono: "Reminder", notifica_windows: true, comparsa: false };
  const eventi = [evento("e1", "Riunione A", "2026-10-12T10:00:00"), evento("e2", "Riunione B", "2026-10-12T11:00:00")];
  const a = ambiente({ api: { get_promemoria: promemoria(p), get_events: json(eventi) } });
  await a.giro();
  assert.deepEqual(a.pianifica, [{ impostazioni: p, eventi, ora: ADESSO }]);
  const vecchio = ambiente({ api: { get_events: json(eventi) } });
  await vecchio.giro();
  assert.deepEqual(vecchio.pianifica, [{ impostazioni: null, eventi, ora: ADESSO }]);
  const senzaEventi = ambiente({ api: { get_events: risposta(200, { errors: [] }) } });
  await senzaEventi.giro();
  assert.deepEqual(senzaEventi.pianifica[0].eventi, []);
});

test("una voce non ancora raggiunta non avvisa, e avvisa al giro in cui arriva", async () => {
  const a = ambiente({ adesso: DIECI - 11 * MIN, api: { get_promemoria: soloNotifica, get_events: EVENTI }, voci: [voce("e1", 10)] });
  await a.giro();
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.stato().esito, "ok");
  a.imposta(DIECI - 10 * MIN);
  await a.giro();
  assert.equal(a.notifiche.length, 1);
});

test("più riunioni nello stesso giro: una notifica sola, dalla prima, con il logo solo se è lo stesso per tutte", async () => {
  const roar = { get_personalizzazione: risposta(200, { interfaccia: { loghi: { ROAR: "logos/roar.png?v=7", Beta: "logos/beta.png" } } }) };
  const loghi = { "/logos/roar.png?v=7": [1, 2, 3], "/logos/beta.png": [4, 5, 6] };
  const prima = voce("e1", 10, { quando: "Tra 10 min", chi: "Prima", titolo: "Tra 10 min · Prima", azienda: "ROAR" });
  const seconda = voce("e2", 30, { inizio: DIECI + 15 * MIN, ora: "10:15", quando: "Tra 25 min", chi: "Seconda", azienda: "ROAR" });
  const prova = async (voci) => {
    const a = ambiente({ adesso: DIECI - 10 * MIN, api: { get_promemoria: promemoria({ minuti: [10, 30] }), get_events: EVENTI, ...roar }, voci, loghi });
    await a.giro();
    return a;
  };
  const due = await prova([prima, seconda]);
  assert.equal(due.notifiche.length, 1);
  assert.deepEqual(due.notifiche[0], {
    title: "Tra 10 min · 2 riunioni",
    body: "10:00 Prima\n10:15 Seconda",
    chiave: "e1|2026-10-12T10:00:00+e2|2026-10-12T10:00:00",
    inizio: DIECI,
    logo: "data:image/png;base64,AQID",
  });
  assert.equal(due.stato().inviate, 2);
  assert.deepEqual(due.chiamateAvviso, [2]);
  const diverse = await prova([prima, { ...seconda, azienda: "Beta" }]);
  assert.equal(diverse.notifiche[0].logo, null);
  // una riga per riunione, al più 120 caratteri
  const lunga = await prova([prima, { ...seconda, chi: "x".repeat(200) }]);
  assert.equal(lunga.notifiche[0].body.split("\n")[1], `10:15 ${"x".repeat(114)}`);
});

test("la memoria degli avvisi già dati sopravvive al ricaricamento e si ripulisce", async () => {
  const memoria = {};
  const voci = [voce("e1", 10)];
  const a = ambiente({ api: { get_events: EVENTI }, voci, memoria });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  const salvate = JSON.parse(memoria["oe-app:notifiche-inviate"]);
  assert.deepEqual(salvate, { "e1|2026-10-12T10:00:00|10": DIECI });
  const b = ambiente({ api: { get_events: EVENTI }, voci, memoria });
  await b.giro();
  assert.equal(b.notifiche.length, 0);
  memoria["oe-app:notifiche-inviate"] = JSON.stringify({ vecchia: ADESSO - 2 * 86_400_000, "e1|2026-10-12T10:00:00|10": DIECI });
  const c = ambiente({ api: { get_events: json([]) }, memoria });
  await c.giro();
  assert.deepEqual(Object.keys(JSON.parse(memoria["oe-app:notifiche-inviate"])), ["e1|2026-10-12T10:00:00|10"]);
});

test("più anticipi della stessa riunione raggiunti insieme (app appena avviata): un avviso solo, e un anticipo nuovo avvisa di nuovo", async () => {
  const voci = [voce("e1", 5), voce("e1", 10), voce("e1", 30)];
  const a = ambiente({ adesso: DIECI - 12 * MIN, api: { get_events: EVENTI }, voci });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(Object.keys(JSON.parse(a.memoria["oe-app:notifiche-inviate"])).sort(), ["e1|2026-10-12T10:00:00|30"]);
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  a.imposta(DIECI - 3 * MIN);
  await a.giro();
  assert.equal(a.notifiche.length, 2);
  assert.deepEqual(Object.keys(JSON.parse(a.memoria["oe-app:notifiche-inviate"])).sort(), ["e1|2026-10-12T10:00:00|10", "e1|2026-10-12T10:00:00|30", "e1|2026-10-12T10:00:00|5"]);
  await a.giro();
  assert.equal(a.notifiche.length, 2);
});

test("le impostazioni si rileggono a ogni giro: un cambio sul sito vale dal giro dopo", async () => {
  let impostazioni = { minuti: [5], notifica_windows: false, comparsa: false };
  const api = { get_events: EVENTI, get_promemoria: () => promemoria(impostazioni) };
  const a = ambiente({ api, voci: [voce("e1", 10)] });
  await a.giro();
  assert.equal(a.stato().esito, "spente");
  assert.equal(a.chiamate.filter(([m]) => m === "get_events").length, 0);
  assert.equal(a.pianifica.length, 0);
  impostazioni = { minuti: [10], notifica_windows: true, comparsa: false };
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.chiamate.filter(([m]) => m === "get_promemoria").length, 2);
  assert.equal(a.pianifica[0].impostazioni.minuti[0], 10);
  assert.equal(a.avvisi.length, 0);
});

test("un suono solo: con l'avviso a comparsa lo fa l'avviso e la notifica è muta, senza avviso la notifica; Default se il nome è strano o manca", async () => {
  const prova = async (impostazioni) => {
    const api = { get_events: EVENTI };
    if (impostazioni !== undefined) api.get_promemoria = promemoria(impostazioni);
    const a = ambiente({ api, voci: [voce("e1", 10)] });
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

test("sessione finita (401, pagina di accesso, reindirizzamento): si smette e si riprova dopo 5 minuti", async () => {
  for (const r of [risposta(401, { error: "non autenticato" }), risposta(200, null, { json: false }), risposta(200, {}, { redirected: true })]) {
    // già alla lettura delle impostazioni: gli eventi non si chiedono
    const a = ambiente({ api: { get_promemoria: r, get_events: r }, voci: [voce("e1")] });
    assert.equal(await a.giro(), 300_000);
    assert.equal(a.stato().esito, "non-autenticato");
    assert.equal(a.notifiche.length, 0);
    assert.equal(a.chiamate.filter(([m]) => m === "get_events").length, 0);
    // o alla lettura degli eventi
    const b = ambiente({ api: { get_promemoria: soloNotifica, get_events: r }, voci: [voce("e1")] });
    assert.equal(await b.giro(), 300_000);
    assert.equal(b.stato().esito, "non-autenticato");
    assert.equal(b.pianifica.length, 0);
  }
});

test("errori di rete: l'attesa raddoppia fino a 5 minuti e si azzera al primo successo", async () => {
  let ok = false;
  const a = ambiente({ api: { get_events: () => (ok ? json([]) : Promise.reject(new Error("rete"))) } });
  assert.deepEqual([await a.giro(), await a.giro(), await a.giro(), await a.giro()], [120_000, 240_000, 300_000, 300_000]);
  ok = true;
  assert.equal(await a.giro(), 60_000);
  assert.deepEqual(a.stato(), { giri: 5, inviate: 0, errori: 0, esito: "ok" });
});

test("5xx: errore con attesa crescente, niente avvisi", async () => {
  const a = ambiente({ api: { get_events: risposta(502, { error: "agente" }) }, voci: [voce("e1")] });
  assert.equal(await a.giro(), 120_000);
  assert.equal(a.stato().esito, "errore");
  const b = ambiente({ api: { get_promemoria: risposta(502, { error: "gateway" }), get_events: EVENTI }, voci: [voce("e1")] });
  assert.equal(await b.giro(), 120_000);
  assert.equal(b.stato().esito, "errore");
  assert.equal(b.notifiche.length, 0);
});

test("dati in cache (stale): nessun avviso", async () => {
  const a = ambiente({ api: { get_events: json([evento("e1", "Riunione I", "2026-10-12T10:00:00")], { stale: true }) }, voci: [voce("e1")] });
  await a.giro();
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.avvisi.length, 0);
  assert.equal(a.pianifica.length, 0);
  assert.equal(a.stato().esito, "dati-non-freschi");
});

test("notifica e avviso a comparsa spenti sul sito: si leggono solo le impostazioni", async () => {
  const a = ambiente({ api: { get_promemoria: promemoria({ notifica_windows: false, comparsa: false }), get_events: EVENTI }, voci: [voce("e1")] });
  assert.equal(await a.giro(), 60_000);
  assert.deepEqual(a.chiamate.map(([m]) => m), ["get_promemoria"]);
  assert.equal(a.stato().esito, "spente");
});

test("su Android la pagina chiede il permesso delle notifiche una volta e lascia i promemoria all'app", async () => {
  const api = { get_promemoria: promemoria({ notifica_windows: false, comparsa: true }), get_events: EVENTI };
  const a = ambiente({ api, ua: UA_ANDROID, permesso: null, voci: [voce("e1")] });
  await a.ultimoGiro();
  assert.equal(a.sistema.richieste, 1);
  assert.deepEqual(a.chiamate.map(([m]) => m), ["get_promemoria"]);
  assert.equal(a.pianifica.length, 0);
  assert.equal(a.notifiche.length, 0);
  assert.deepEqual(a.chiamateAvviso, []);
  assert.equal(a.stato().esito, "android");
  // permesso rifiutato o già negato: nessun avviso dalla pagina, e la richiesta non si ripete
  for (const [permesso, esitoRichiesta, richieste] of [[null, "denied", 1], [false, "granted", 0], [true, "granted", 0]]) {
    const b = ambiente({ api, ua: UA_ANDROID, permesso, esitoRichiesta, voci: [voce("e1")] });
    await b.ultimoGiro();
    assert.equal(b.sistema.richieste, richieste, `${permesso} ${esitoRichiesta}`);
    assert.equal(b.notifiche.length, 0);
    assert.equal(b.stato().esito, "android");
  }
});

test("su Android senza sessione il permesso non si chiede, e si riprova dopo 5 minuti", async () => {
  const a = ambiente({ ua: UA_ANDROID, permesso: null, api: { get_promemoria: risposta(401, { error: "non autenticato" }) } });
  assert.equal(await a.giro(), 300_000);
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.sistema.controlli, 0);
});

test("lanciato due volte nella stessa pagina (WebView di Android) lavora una volta sola", async () => {
  const a = ambiente({ permesso: null, ua: UA_ANDROID, api: { get_events: EVENTI } });
  const timerPrima = a.timer.length;
  const contatori = a.sandbox.__oeNotifiche;
  vm.runInContext(SCRIPT, a.sandbox);
  assert.equal(a.timer.length, timerPrima);
  assert.equal(a.sandbox.__oeNotifiche, contatori);
  await a.ultimoGiro();
  assert.equal(a.chiamate.filter(([metodo]) => metodo === "get_promemoria").length, 1);
  assert.equal(a.sistema.richieste, 1);
});

test("avviso a comparsa: ora, titolo, logo dell'azienda come data: e sigla, una volta sola, accanto alla notifica", async () => {
  const api = { get_events: EVENTI, get_personalizzazione: risposta(200, { interfaccia: { loghi: { ROAR: "logos/roar.png?v=7" } } }), get_promemoria: promemoria({}) };
  const a = ambiente({ api, voci: [voce("e1", 10, { chi: "Riunione A", azienda: "ROAR", colore: "#2563eb" })], loghi: { "/logos/roar.png?v=7": "PNG" } });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.deepEqual(a.avvisi, [{ chiave: "e1|2026-10-12T10:00:00", inizio: DIECI, ora: "10:00", titolo: "Riunione A", logo: "data:image/png;base64,UE5H", sigla: "RO", colore: "#2563eb" }]);
  await a.giro();
  assert.equal(a.avvisi.length, 1);
  assert.deepEqual(a.chiamate.filter(([m]) => m === "logo" || m === "get_personalizzazione").map(([m]) => m), ["get_personalizzazione", "logo"]);
});

test("avviso a comparsa senza logo: la sigla scelta o le iniziali; un logo fuori dal sito non si legge; gli Eventi personali hanno quello del gateway; le riunioni dello stesso giro vanno insieme", async () => {
  const api = { get_events: EVENTI, get_personalizzazione: risposta(200, { interfaccia: { loghi: { Gamma: "https://altro.example/x.png" }, sigle: { Gamma: "GM" } } }) };
  const voci = [voce("e1", 10, { azienda: "Beta Servizi" }), voce("e2", 10, { azienda: "Gamma" }), voce("e3", 10, { azienda: "Eventi personali" })];
  const a = ambiente({ api, voci, loghi: { "/logos/personale.png": "IO" } });
  await a.giro();
  assert.deepEqual(a.avvisi.map((v) => [v.sigla, v.logo]), [["BS", null], ["GM", null], ["EP", "data:image/png;base64,SU8="]]);
  assert.deepEqual(a.chiamate.filter(([m]) => m === "logo").map(([, u]) => u), ["/logos/personale.png"]);
  assert.deepEqual(a.chiamateAvviso, [3]);
});

test("solo l'avviso a comparsa (notifica spenta): nessuna notifica; un avviso che non riesce non fa ripetere la notifica", async () => {
  const solo = ambiente({ api: { get_promemoria: promemoria({ notifica_windows: false }), get_events: EVENTI }, voci: [voce("e1")] });
  await solo.giro();
  assert.equal(solo.notifiche.length, 0);
  assert.equal(solo.avvisi.length, 1);
  assert.equal(solo.stato().esito, "ok");
  const rotto = ambiente({ api: { get_events: EVENTI }, voci: [voce("e1")], avvisoRotto: true });
  await rotto.giro();
  await rotto.giro();
  assert.equal(rotto.notifiche.length, 1);
  assert.equal(rotto.stato().esito, "ok");
});

test("avviso a comparsa spento: solo la notifica, con il logo dell'azienda", async () => {
  const api = { get_promemoria: promemoria({ comparsa: false }), get_events: EVENTI, get_personalizzazione: risposta(200, { interfaccia: { loghi: { ROAR: "logos/roar.png?v=7" } } }) };
  const a = ambiente({ api, voci: [voce("e1", 10, { azienda: "ROAR" })], loghi: { "/logos/roar.png?v=7": [1, 2, 3] } });
  await a.giro();
  assert.equal(a.notifiche.length, 1);
  assert.equal(a.notifiche[0].logo, "data:image/png;base64,AQID");
  assert.deepEqual(a.chiamateAvviso, []);
});

test("sul PC la notifica la fa l'app, con la chiave della riunione e l'inizio; se non riesce la fa il plugin", async () => {
  const rotta = ambiente({ api: { get_promemoria: soloNotifica, get_events: EVENTI }, voci: [voce("e1")], notificaRotta: true });
  await rotta.giro();
  assert.deepEqual(rotta.vie, ["plugin"]);
  assert.deepEqual(rotta.notifiche, [{ title: "Tra 5 min · Riunione e1", body: "Alle 10:00", sound: "Default" }]);
});

test("Solo ora e azienda: il testo è quello del nucleo, l'avviso a comparsa senza titolo, e il titolo non arriva mai a notifica e avviso", async () => {
  const api = {
    get_promemoria: promemoria({ contenuto: "minimo" }),
    get_events: json([evento("e1", "Colloquio riservato", "2026-10-12T10:00:00", { alias: "ROAR", location: "Sala del consiglio" }), evento("e2", "Visita privata", "2026-10-12T10:00:00", { alias: "" })]),
  };
  // il nucleo con "minimo" mette l'azienda al posto del titolo (src-tauri/src/promemoria.rs)
  const roar = voce("e1", 10, { chi: "ROAR", titolo: "Tra 5 min · ROAR", azienda: "ROAR" });
  const anonima = voce("e2", 10, { chi: "", titolo: "Tra 5 min", azienda: "" });
  const a = ambiente({ api, voci: [roar, anonima] });
  await a.giro();
  assert.deepEqual(a.notifiche.map(({ title, body }) => ({ title, body })), [{ title: "Tra 5 min · 2 riunioni", body: "10:00 ROAR\n10:00" }]);
  assert.deepEqual(a.avvisi.map((v) => [v.titolo, v.ora, v.sigla]), [["", "10:00", "RO"], ["", "10:00", ""]]);
  const una = ambiente({ api, voci: [roar] });
  await una.giro();
  assert.deepEqual([una.notifiche[0].title, una.notifiche[0].body], ["Tra 5 min · ROAR", "Alle 10:00"]);
  const tutto = a.passati.join("\n") + una.passati.join("\n");
  for (const segreto of ["Colloquio", "riservato", "Visita privata", "Sala del consiglio"]) assert.equal(tutto.includes(segreto), false, segreto);
});

test("la richiesta è quella della pagina più un terzo argomento: POST get_events con inizio e fine ISO a 24 ore e tutti i calendari", async () => {
  const a = ambiente({ adesso: Date.UTC(2026, 9, 12, 7, 55, 30), api: { get_events: json([]) } });
  await a.giro();
  const [metodo, argomenti] = a.chiamate.find(([m]) => m === "get_events");
  assert.equal(metodo, "get_events");
  assert.equal(argomenti.length, 3);
  assert.match(argomenti[0], /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$/);
  assert.equal(new Date(argomenti[1]).getTime() - new Date(argomenti[0]).getTime(), 24 * 3_600_000);
  // i promemoria contano anche le aziende spente nella barra laterale, che filtra solo la vista
  assert.equal(argomenti[2], true);
  assert.deepEqual(a.chiamate.find(([m]) => m === "get_promemoria"), ["get_promemoria", []]);
});

test("permesso negato sul PC (false): nessuna richiesta, nessun avviso", async () => {
  const a = ambiente({ permesso: false, api: { get_events: EVENTI }, voci: [voce("e1")] });
  await a.giro();
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.notifiche.length, 0);
  assert.equal(a.stato().esito, "senza-permesso");
});

test("permesso già concesso (PC): nessuna richiesta, il controllo non si ripete dopo il primo sì", async () => {
  const a = ambiente({ api: { get_events: EVENTI }, voci: [voce("e1")] });
  await a.giro();
  await a.giro();
  assert.equal(a.sistema.richieste, 0);
  assert.equal(a.sistema.controlli, 1);
  assert.equal(a.notifiche.length, 1);
});
