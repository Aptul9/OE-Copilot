// Prova del file PAUSA: node --test test/pausa.test.mjs (Node 22 o più recente, legge il TypeScript così com'è)
import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const cartella = mkdtempSync(join(tmpdir(), "pausa-"));
const file = join(cartella, "PAUSA");
process.env.PAUSA_FILE = file;
process.env.PAUSA_SCRIVI_MS = "1000";
process.env.PAUSA_PING_MS = "1000";
const { sorveglia, conteggio } = await import("../src/server/services/Pausa.ts");

class FintoWS extends EventEmitter {
  pings = 0;
  terminato = false;
  ping() { this.pings++; }
  terminate() { this.terminato = true; this.emit("close"); }
}

test("il file c'è finché c'è un flusso e sparisce con l'ultimo", () => {
  const a = new FintoWS(), b = new FintoWS();
  assert.equal(existsSync(file), false);
  sorveglia(a);
  assert.equal(existsSync(file), true);
  assert.match(readFileSync(file, "utf8"), /^\d+\n$/);
  sorveglia(b);
  assert.equal(conteggio(), 2);
  a.emit("close");
  assert.equal(existsSync(file), true, "ne resta uno aperto");
  b.emit("close");
  assert.equal(conteggio(), 0);
  assert.equal(existsSync(file), false);
});

test("finché c'è un flusso il file si riscrive, con la data di adesso", () => {
  mock.timers.enable({ apis: ["setInterval", "Date"], now: 1_000_000 });
  const a = new FintoWS();
  sorveglia(a);
  assert.equal(readFileSync(file, "utf8"), "1000000\n");
  mock.timers.tick(1000);
  assert.equal(readFileSync(file, "utf8"), "1001000\n");
  a.emit("pong"); mock.timers.tick(1000);
  a.emit("close");
  mock.timers.reset();
  assert.equal(existsSync(file), false);
});

test("un browser che non risponde ai ping viene chiuso e la pausa finisce", () => {
  mock.timers.enable({ apis: ["setInterval"] });
  const muto = new FintoWS();
  sorveglia(muto);
  mock.timers.tick(1000);   // primo giro: ping
  assert.equal(muto.pings, 1);
  assert.equal(muto.terminato, false);
  mock.timers.tick(1000);   // secondo giro: nessun pong, si chiude
  assert.equal(muto.terminato, true);
  assert.equal(conteggio(), 0);
  assert.equal(existsSync(file), false);
  mock.timers.reset();
});

test("un browser che risponde resta", () => {
  mock.timers.enable({ apis: ["setInterval"] });
  const vivo = new FintoWS();
  sorveglia(vivo);
  for (let i = 0; i < 5; i++) { mock.timers.tick(1000); vivo.emit("pong"); }
  assert.equal(vivo.terminato, false);
  assert.equal(existsSync(file), true);
  vivo.emit("close");
  mock.timers.reset();
  assert.equal(existsSync(file), false);
  rmSync(cartella, { recursive: true, force: true });
});
