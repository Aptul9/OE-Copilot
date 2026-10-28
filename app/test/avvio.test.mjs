// Prove della pagina di avvio (src/avvio.js) senza finestra: senza rete porta comunque al sito, che si apre con la sua copia; con la rete ma
// senza risposta resta qui e lascia scegliere (riprovare, la copia, un altro server); il campo dell'indirizzo chiama imposta_indirizzo.
// node --test test/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const SCRIPT = readFileSync(new URL("../src/avvio.js", import.meta.url), "utf8");
const ORIGINE = "https://oe.prova.test";

async function avvia({ hash = "", rete = false, online = true, origine = ORIGINE, invoke = undefined } = {}) {
  const elemento = () => {
    const gestori = {};
    return {
      textContent: "",
      hidden: false,
      value: "",
      disabled: false,
      attributi: {},
      fuoco: 0,
      selezioni: 0,
      gestori,
      addEventListener: (tipo, f) => (gestori[tipo] = f),
      setAttribute(k, v) {
        this.attributi[k] = v;
      },
      removeAttribute(k) {
        delete this.attributi[k];
      },
      focus() {
        this.fuoco++;
      },
      select() {
        this.selezioni++;
      },
    };
  };
  const ids = ["rotella", "stato", "dettaglio", "azioni", "riprova", "copia", "cambia", "modulo", "indirizzo", "errore", "salva", "annulla", "esci"];
  const elementi = Object.fromEntries(ids.map((id) => [id, elemento()]));
  const vai = [];
  const timer = [];
  const annullati = [];
  const finestra = { __OE_ORIGINE__: origine, gestori: {}, addEventListener: (tipo, f) => (finestra.gestori[tipo] = f) };
  if (invoke) finestra.__TAURI_INTERNALS__ = { invoke };
  const richieste = [];
  const sandbox = {
    window: finestra,
    document: { getElementById: (id) => elementi[id] },
    location: { hash, replace: (u) => vai.push(u) },
    navigator: { onLine: online },
    fetch: async (u) => {
      richieste.push(u);
      if (!rete) throw new TypeError("Failed to fetch");
      return {};
    },
    AbortSignal: { timeout: () => undefined },
    setTimeout: (f, ms) => {
      timer.push({ f, ms });
      return timer.length;
    },
    clearTimeout: (t) => annullati.push(t),
    URL,
  };
  vm.runInNewContext(SCRIPT, sandbox);
  const aspetta = () => new Promise((fatto) => setImmediate(fatto));
  await aspetta();
  return { vai, timer, annullati, elementi, finestra, richieste, sandbox, aspetta };
}

const evento = () => ({ preventDefault() {} });

test("con la rete porta al sito", async () => {
  assert.deepEqual((await avvia({ rete: true })).vai, [`${ORIGINE}/`]);
});

test("senza rete porta lo stesso al sito, che si apre con la sua copia", async () => {
  const a = await avvia({ online: false });
  assert.deepEqual(a.vai, [`${ORIGINE}/`]);
  assert.deepEqual(a.timer, []);
});

test("con la rete ma senza risposta resta, lo dice e lascia scegliere", async () => {
  const a = await avvia();
  assert.deepEqual(a.vai, []);
  assert.equal(a.elementi.stato.textContent, "Server non raggiungibile.");
  assert.match(a.elementi.dettaglio.textContent, /oe\.prova\.test/);
  assert.equal(a.elementi.azioni.hidden, false);
  assert.equal(a.elementi.riprova.hidden, false);
  assert.equal(a.elementi.copia.hidden, false);
  assert.equal(a.elementi.modulo.hidden, false, "il modulo è nascosto dal markup, qui non è stato toccato");
  assert.deepEqual(a.timer.map((t) => t.ms), [5000]);
});

test("tornata qui perché il sito non aveva una copia: resta, niente copia da aprire, e riprova", async () => {
  const a = await avvia({ hash: "#senza-copia", online: false });
  assert.deepEqual(a.vai, []);
  assert.equal(a.elementi.stato.textContent, "Server non raggiungibile.");
  assert.equal(a.elementi.copia.hidden, true);
  assert.deepEqual(a.timer.map((t) => t.ms), [5000]);
});

test("il tentativo automatico non cambia la pagina finché il server tace, e porta al sito quando risponde", async () => {
  const a = await avvia();
  const prima = a.elementi.stato.textContent;
  a.timer[0].f();
  await a.aspetta();
  assert.equal(a.elementi.stato.textContent, prima, "niente 'Connessione…' a ogni giro");
  assert.equal(a.elementi.azioni.hidden, false);
  assert.equal(a.timer.length, 2);
  // il server torna: il giro dopo porta al sito
  a.sandbox.fetch = async () => ({});
  a.timer[1].f();
  await a.aspetta();
  assert.deepEqual(a.vai, [`${ORIGINE}/`]);
});

test("Apri la copia salvata porta al sito", async () => {
  const a = await avvia();
  a.elementi.copia.gestori.click();
  assert.deepEqual(a.vai, [`${ORIGINE}/`]);
});

test("Riprova rifà il controllo con 'Connessione…'", async () => {
  const a = await avvia();
  a.elementi.riprova.gestori.click();
  assert.equal(a.elementi.stato.textContent, "Connessione…");
  assert.equal(a.elementi.azioni.hidden, true);
  await a.aspetta();
  assert.equal(a.richieste.length, 2);
});

test("Cambia server ferma i tentativi, apre il campo con l'indirizzo di adesso e ci mette il cursore", async () => {
  const a = await avvia();
  a.elementi.cambia.gestori.click();
  assert.deepEqual(a.annullati.at(-1), 1, "il timer dei tentativi è fermato");
  const { modulo, indirizzo, azioni, stato, dettaglio } = a.elementi;
  assert.equal(modulo.hidden, false);
  assert.equal(azioni.hidden, true);
  assert.equal(indirizzo.value, ORIGINE);
  assert.equal(indirizzo.fuoco, 1);
  assert.equal(indirizzo.selezioni, 1);
  assert.equal(stato.textContent, "Cambia server");
  assert.equal(dettaglio.textContent, "Adesso: oe.prova.test");
});

test("dalla voce del menu (#server) il campo si apre subito, senza controllare il sito", async () => {
  const a = await avvia({ hash: "#server", rete: true });
  assert.deepEqual(a.richieste, []);
  assert.deepEqual(a.vai, []);
  assert.equal(a.elementi.modulo.hidden, false);
  assert.equal(a.elementi.indirizzo.value, ORIGINE);
});

test("la pagina già aperta passa a #server: si apre il campo", async () => {
  const a = await avvia({ rete: true });
  a.sandbox.location.hash = "#server";
  a.finestra.gestori.hashchange();
  assert.equal(a.elementi.modulo.hidden, false);
});

test("Annulla chiude il campo e rifà il controllo", async () => {
  const a = await avvia({ hash: "#server", rete: true });
  a.elementi.annulla.gestori.click();
  assert.equal(a.elementi.modulo.hidden, true);
  await a.aspetta();
  assert.deepEqual(a.vai, [`${ORIGINE}/`]);
});

test("senza indirizzo (segnaposto) il campo si apre vuoto e Annulla lo riapre", async () => {
  const a = await avvia({ origine: "https://oe.example.com", rete: true });
  assert.deepEqual(a.richieste, []);
  assert.equal(a.elementi.modulo.hidden, false);
  assert.equal(a.elementi.indirizzo.value, "");
  assert.equal(a.elementi.dettaglio.textContent, "Questa app non ha ancora un indirizzo.");
  a.elementi.annulla.gestori.click();
  assert.equal(a.elementi.modulo.hidden, false);
  assert.deepEqual(a.richieste, []);
});

test("Salva e riavvia chiama imposta_indirizzo con quello che c'è nel campo", async () => {
  const chiamate = [];
  const a = await avvia({ hash: "#server", invoke: async (comando, argomenti) => (chiamate.push([comando, argomenti]), true) });
  a.elementi.indirizzo.value = "oe.nuovo.test";
  await a.elementi.modulo.gestori.submit(evento());
  // gli oggetti nati nel contesto della pagina hanno un altro prototipo: si confronta il JSON
  assert.equal(JSON.stringify(chiamate), JSON.stringify([["imposta_indirizzo", { indirizzo: "oe.nuovo.test" }]]));
  assert.equal(a.elementi.stato.textContent, "Indirizzo salvato. Riavvio…");
  assert.equal(a.elementi.rotella.hidden, false);
  assert.equal(a.elementi.modulo.hidden, true);
});

test("dove l'app non riparte da sola lo dice: si riapre a mano", async () => {
  const a = await avvia({ hash: "#server", invoke: async () => false });
  await a.elementi.modulo.gestori.submit(evento());
  assert.equal(a.elementi.stato.textContent, "Indirizzo salvato. L'app si chiude: riaprila.");
  assert.equal(a.elementi.rotella.hidden, true);
});

test("un indirizzo rifiutato resta nel campo con il motivo accanto", async () => {
  const a = await avvia({ hash: "#server", invoke: async () => Promise.reject("Serve https:// (http:// solo per localhost).") });
  a.elementi.indirizzo.value = "http://oe.nuovo.test";
  await a.elementi.modulo.gestori.submit(evento());
  assert.equal(a.elementi.errore.textContent, "Serve https:// (http:// solo per localhost).");
  assert.equal(a.elementi.indirizzo.attributi["aria-invalid"], "true");
  assert.equal(a.elementi.salva.disabled, false);
  assert.equal(a.elementi.modulo.hidden, false);
  assert.ok(a.elementi.indirizzo.fuoco >= 2, "il cursore torna nel campo");
  // una nuova prova toglie l'errore vecchio
  a.sandbox.window.__TAURI_INTERNALS__.invoke = async () => true;
  await a.elementi.modulo.gestori.submit(evento());
  assert.equal(a.elementi.errore.textContent, "");
  assert.equal(a.elementi.indirizzo.attributi["aria-invalid"], undefined);
});

test("senza il comando dell'app (versione vecchia) lo dice e non fa altro", async () => {
  const a = await avvia({ hash: "#server" });
  await a.elementi.modulo.gestori.submit(evento());
  assert.equal(a.elementi.errore.textContent, "Questa versione dell'app non permette di cambiare server.");
  assert.equal(a.elementi.salva.disabled, false);
});

test("Esci dall'account si vede quando il sito non risponde e nel campo del server, non durante il controllo", async () => {
  const muto = await avvia();
  assert.equal(muto.elementi.esci.hidden, false);
  const campo = await avvia({ hash: "#server" });
  assert.equal(campo.elementi.esci.hidden, false);
  const riuscito = await avvia({ rete: true });
  assert.equal(riuscito.elementi.esci.hidden, true);
});

test("Esci dall'account chiama esci_account, senza argomenti", async () => {
  const chiamate = [];
  const a = await avvia({ invoke: async (comando, argomenti) => (chiamate.push([comando, argomenti]), undefined) });
  a.elementi.esci.gestori.click();
  await a.aspetta();
  assert.equal(JSON.stringify(chiamate), JSON.stringify([["esci_account", undefined]]));
  assert.equal(a.elementi.esci.disabled, true, "durante l'uscita non si preme due volte");
});

test("se l'uscita non riesce lo dice con il motivo e si può riprovare", async () => {
  const a = await avvia({ invoke: async () => Promise.reject("finestra non trovata") });
  a.elementi.esci.gestori.click();
  await a.aspetta();
  assert.equal(a.elementi.stato.textContent, "Uscita non riuscita.");
  assert.equal(a.elementi.dettaglio.textContent, "finestra non trovata");
  assert.equal(a.elementi.esci.disabled, false);
});

test("senza il comando dell'app (versione vecchia) l'uscita lo dice", async () => {
  const a = await avvia();
  a.elementi.esci.gestori.click();
  await a.aspetta();
  assert.equal(a.elementi.stato.textContent, "Uscita non riuscita.");
});
