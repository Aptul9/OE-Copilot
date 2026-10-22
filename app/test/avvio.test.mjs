// Prove della pagina di avvio (src/avvio.js) senza finestra: senza rete porta comunque al sito, che si apre con la sua copia; se il sito
// non l'ha (#senza-copia) resta qui e riprova.
// node --test test/
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const SCRIPT = readFileSync(new URL("../src/avvio.js", import.meta.url), "utf8");
const ORIGINE = "https://oe.prova.test";

async function avvia({ hash = "", rete = false } = {}) {
  const elemento = () => ({ textContent: "", hidden: false, addEventListener() {} });
  const elementi = { rotella: elemento(), stato: elemento(), dettaglio: elemento(), riprova: elemento() };
  const vai = [];
  const timer = [];
  const sandbox = {
    window: { __OE_ORIGINE__: ORIGINE, addEventListener() {} },
    document: { getElementById: (id) => elementi[id] },
    location: { hash, replace: (u) => vai.push(u) },
    fetch: async () => {
      if (!rete) throw new TypeError("Failed to fetch");
      return {};
    },
    AbortSignal: { timeout: () => undefined },
    setTimeout: (f, ms) => timer.push(ms),
    clearTimeout() {},
    URL,
  };
  vm.runInNewContext(SCRIPT, sandbox);
  await new Promise((fatto) => setImmediate(fatto));
  return { vai, timer, elementi };
}

test("con la rete porta al sito", async () => {
  assert.deepEqual((await avvia({ rete: true })).vai, [`${ORIGINE}/`]);
});

test("senza rete porta lo stesso al sito, che si apre con la sua copia", async () => {
  const a = await avvia();
  assert.deepEqual(a.vai, [`${ORIGINE}/`]);
  assert.deepEqual(a.timer, []);
});

test("tornata qui perché il sito non aveva una copia: resta, lo dice e riprova", async () => {
  const a = await avvia({ hash: "#senza-copia" });
  assert.deepEqual(a.vai, []);
  assert.equal(a.elementi.stato.textContent, "Server non raggiungibile.");
  assert.deepEqual(a.timer, [5000]);
});
