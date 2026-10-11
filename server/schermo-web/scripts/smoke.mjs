// Prova di fumo del servizio costruito (dist/index.js), senza telefono: parte, serve la pagina, e rifiuta
// quello che non deve servire (tunnel verso una porta qualunque del telefono, proxy verso un indirizzo qualunque).
// Uso: node scripts/smoke.mjs [cartella dist] [porta]   (Node 22 o più recente: WebSocket è globale)
import { spawn } from "node:child_process";
import { setTimeout as attendi } from "node:timers/promises";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const dist = process.argv[2] || "dist";
const porta = Number(process.argv[3] || 18200);
const base = `http://127.0.0.1:${porta}`;
let esiti = 0;
const verifica = (nome, ok, dettaglio = "") => {
  console.log((ok ? "ok   " : "KO   ") + nome + (ok ? "" : "  -> " + dettaglio));
  if (!ok) esiti++;
};

const server = spawn(process.execPath, ["index.js"], {
  cwd: dist,
  env: { ...process.env, PORT: String(porta), ADB_PORT: "1" },
  stdio: ["ignore", "pipe", "pipe"],
});
let log = "";
server.stdout.on("data", (d) => (log += d));
server.stderr.on("data", (d) => (log += d));

const chiusura = (url) =>
  new Promise((risolvi) => {
    const ws = new WebSocket(url);
    const t = setTimeout(() => { ws.close(); risolvi({ stato: "nessuna risposta" }); }, 5000);
    ws.onclose = (e) => { clearTimeout(t); risolvi({ codice: e.code, motivo: e.reason }); };
    ws.onerror = () => {};
  });

try {
  for (let i = 0; i < 50; i++) {
    try { if ((await fetch(base + "/salute")).ok) break; } catch { /* non ancora */ }
    await attendi(200);
  }
  const salute = await fetch(base + "/salute");
  verifica("/salute risponde ok", salute.ok && (await salute.text()) === "ok");
  const pagina = await fetch(base + "/");
  const html = await pagina.text();
  verifica("la pagina si serve e porta lo script", pagina.ok && /bundle\.js/.test(html), String(pagina.status));
  verifica("nessuna intestazione X-Powered-By", pagina.headers.get("x-powered-by") === null);
  const js = await fetch(base + "/bundle.js");
  verifica("bundle.js si serve e non è vuoto", js.ok && (await js.arrayBuffer()).byteLength > 50000);
  // il server che gira sul telefono: lo stesso file che sta nel repository (impronta fissata accanto al file)
  const jar = join(dist, "vendor/Genymobile/scrcpy/scrcpy-server.jar");
  const attesa = readFileSync(new URL("../vendor/Genymobile/scrcpy/scrcpy-server.jar.sha256", import.meta.url), "utf8").split(/\s/)[0];
  verifica("il server per il telefono è nel pacchetto, con l'impronta fissata", existsSync(jar) && createHash("sha256").update(readFileSync(jar)).digest("hex") === attesa);

  const ws = `ws://127.0.0.1:${porta}/`;
  const altra = await chiusura(ws + "?action=proxy-ws&ws=ws%3A%2F%2F127.0.0.1%3A1");
  verifica("proxy-ws (verso un indirizzo qualunque) non esiste", altra.codice === 4002, JSON.stringify(altra));
  const porta5555 = await chiusura(ws + "?action=proxy-adb&remote=tcp%3A5555&udid=telefono%3A5555");
  verifica("proxy-adb verso tcp:5555 è rifiutato", porta5555.codice === 4003, JSON.stringify(porta5555));
  const senzaUdid = await chiusura(ws + "?action=proxy-adb&remote=tcp%3A8886");
  verifica("proxy-adb senza udid è rifiutato", senzaUdid.codice === 4003, JSON.stringify(senzaUdid));
  const ignota = await chiusura(ws + "?action=shell&udid=x");
  verifica("la shell non esiste", ignota.codice === 4002, JSON.stringify(ignota));
  const lista = await new Promise((risolvi) => {
    const w = new WebSocket(ws + "?action=goog-device-list");
    const t = setTimeout(() => { w.close(); risolvi("aperto, nessun dispositivo"); }, 2500);
    w.onmessage = (m) => { clearTimeout(t); w.close(); risolvi(String(m.data).slice(0, 120)); };
    w.onclose = () => {};
    w.onerror = () => {};
  });
  verifica("la lista dei dispositivi accetta la connessione", typeof lista === "string", String(lista));
} catch (e) {
  verifica("la prova di fumo non è andata in errore", false, e instanceof Error ? e.message : String(e));
} finally {
  server.kill();
  await attendi(300);
}
if (esiti) console.log("\nlog del servizio:\n" + log.slice(-1500));
console.log(esiti ? `\nESITO: ${esiti} KO` : "\nESITO: tutto ok");
process.exit(esiti ? 1 : 0);
