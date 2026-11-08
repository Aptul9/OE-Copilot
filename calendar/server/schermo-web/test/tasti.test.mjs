// Prova della scelta testo / tasto / niente per i tasti premuti nella pagina del telefono:
//   node --test test/tasti.test.mjs (Node 22 o più recente, legge il TypeScript così com'è)
import { test } from "node:test";
import assert from "node:assert/strict";
import { classifica, spezza } from "../src/app/googDevice/KeyText.ts";

const tasto = (key, extra = {}) => classifica({ key, ctrlKey: false, altKey: false, metaKey: false, altGraph: false, isComposing: false, ...extra });

test("lettere, cifre e simboli partono come testo, con o senza Maiusc", () => {
  for (const k of ["a", "A", "7", "@", ".", ":", "_", "-", " ", "è", "€", "😀"]) assert.equal(tasto(k), "testo", k);
});

test("AltGr (Ctrl+Alt su Windows) scrive il carattere, non è una scorciatoia", () => {
  assert.equal(tasto("@", { ctrlKey: true, altKey: true }), "testo");
  assert.equal(tasto("€", { altGraph: true, ctrlKey: true, altKey: true }), "testo");
  assert.equal(tasto("@", { altGraph: true }), "testo");
});

test("Ctrl, Meta e Alt da soli lasciano il tasto al browser (incolla, ricarica)", () => {
  assert.equal(tasto("v", { ctrlKey: true }), "niente");
  assert.equal(tasto("r", { metaKey: true }), "niente");
  assert.equal(tasto("f", { altKey: true }), "niente");
});

test("i tasti che non sono caratteri partono come codici Android", () => {
  for (const k of ["Enter", "Backspace", "Delete", "Tab", "Escape", "ArrowLeft", "ArrowDown", "Home", "End", "PageUp"]) assert.equal(tasto(k), "tasto", k);
});

test("modificatori soli, tasti morti, composizione e F1..F12 non vanno al telefono", () => {
  for (const k of ["Shift", "Control", "Alt", "AltGraph", "Meta", "CapsLock", "NumLock", "Dead", "Process", "Unidentified", "F5", "F12"]) assert.equal(tasto(k), "niente", k);
  assert.equal(tasto("a", { isComposing: true }), "niente");
});

test("spezza: pezzi interi, mai a metà di un carattere a due unità", () => {
  assert.deepEqual(spezza(""), []);
  assert.deepEqual(spezza("abc", 200), ["abc"]);
  const lungo = "x".repeat(450);
  assert.deepEqual(spezza(lungo).map((p) => p.length), [200, 200, 50]);
  const emoji = "a" + "😀".repeat(5);
  const pezzi = spezza(emoji, 2);
  assert.deepEqual(pezzi, ["a😀", "😀😀", "😀😀"]);
  assert.equal(pezzi.join(""), emoji);
});
