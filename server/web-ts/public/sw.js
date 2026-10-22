// Il service worker della pagina (README.md, 4.9 e 4.11): le notifiche del browser e la copia per quando manca la rete.
// Notifiche: il server manda un testo, la prima riga è il titolo e il resto il corpo. badge è l'icona della barra di stato di Android
// (bianca su trasparente, da app/scripts/genera_icona.py): senza, Chrome ci mette la sua campanella.
// Copia: la pagina (/) e le letture dell'API vengono dalla rete e se ne tiene l'ultima risposta; senza rete risponde la copia. I file di
// /_next/static/ cambiano nome quando cambia il contenuto: si prendono dalla copia. Le scritture passano sempre dalla rete. All'uscita
// (sign-out) pagina e dati salvati si cancellano. Non chiama nessun indirizzo fuori dal sito. Una Cache Storage che non si apre (navigazione
// privata, spazio finito) non ferma niente: si va in rete come senza service worker.
const VERSIONE = "oe-1";
const PAGINE = VERSIONE + "-pagine";
// ponytail: FILE cresce di qualche MB a ogni aggiornamento del sito; si svuota cambiando VERSIONE
const FILE = VERSIONE + "-file";
const DATI = VERSIONE + "-dati";
const LETTURE = ["get_config", "get_personalizzazione", "get_calendars", "get_events", "get_loghi"];
const EVENTI_MAX = 60;
const ATTESA_PAGINA_MS = 4000;
const GIORNO_MS = 86_400_000;

/** La cache, o null se il browser non la apre. */
const negozio = (nome) => caches.open(nome).catch(() => null);

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) =>
  e.waitUntil(
    Promise.all([
      self.clients.claim(),
      caches.keys().then((nomi) => Promise.all(nomi.filter((n) => !n.startsWith(VERSIONE + "-")).map((n) => caches.delete(n)))),
      // la pagina e i suoi file subito, non alla prossima apertura: la prima volta il service worker arriva a pagina già caricata
      fetch("/").then(tieniPagina).catch(() => {}),
    ]),
  ),
);

self.addEventListener("push", (e) => {
  const testo = e.data ? e.data.text() : "";
  const [titolo, ...resto] = testo.split("\n");
  e.waitUntil(self.registration.showNotification(titolo || "Promemoria", { body: resto.join("\n"), icon: "/icon.png", badge: "/badge.png" }));
});

self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  e.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((finestre) => (finestre.length ? finestre[0].focus() : self.clients.openWindow("/"))),
  );
});

self.addEventListener("fetch", (e) => {
  const r = e.request;
  const u = new URL(r.url);
  if (u.origin !== self.location.origin || u.pathname.startsWith("/telefono/")) return;
  if (r.method === "POST") {
    if (u.pathname === "/api/auth/sign-out") e.waitUntil(Promise.all([caches.delete(PAGINE), caches.delete(DATI)]));
    const metodo = u.pathname.slice("/api/".length);
    if (u.pathname.startsWith("/api/") && LETTURE.includes(metodo)) e.respondWith(lettura(e, metodo));
    return;
  }
  if (r.method !== "GET") return;
  if (r.mode === "navigate") {
    if (u.pathname === "/") e.respondWith(pagina(e));
    return;
  }
  if (u.pathname.startsWith("/_next/static/")) e.respondWith(dallaCopia(e));
  else if (u.pathname.startsWith("/logos/") || /\.(png|ico|svg|webp|woff2?)$/.test(u.pathname)) e.respondWith(copiaPoiRete(e));
});

/** Salva la pagina e i file di /_next/static/ che nomina. Una risposta che non è la pagina (l'accesso, un errore) non si salva. */
async function tieniPagina(res) {
  if (!res.ok || res.redirected) return;
  const [pagine, file] = await Promise.all([negozio(PAGINE), negozio(FILE)]);
  if (!pagine || !file) return;
  const html = await res.clone().text();
  const nomi = [...new Set(html.match(/\/_next\/static\/[^"'\s)\\]+/g) || [])];
  await Promise.all(nomi.map((n) => file.match(n).then((c) => c || file.add(n).catch(() => {}))));
  await pagine.put("/", res);
}

/** La pagina dalla rete; se la rete non c'è, o non risponde in ATTESA_PAGINA_MS, la copia. */
async function pagina(e) {
  const copia = await negozio(PAGINE).then((c) => c?.match("/"));
  const rete = fetch(e.request);
  e.waitUntil(rete.then((res) => tieniPagina(res.clone())).catch(() => {}));
  if (!copia) return rete;
  return Promise.race([rete.catch(() => copia), new Promise((fatto) => setTimeout(() => fatto(copia), ATTESA_PAGINA_MS))]);
}

async function dallaCopia(e) {
  const c = await negozio(FILE);
  const copia = await c?.match(e.request);
  if (copia) return copia;
  const res = await fetch(e.request);
  if (c && res.ok) e.waitUntil(c.put(e.request, res.clone()).catch(() => {}));
  return res;
}

/** Icone e loghi: la copia subito, se c'è, e intanto la rete la aggiorna. */
async function copiaPoiRete(e) {
  const c = await negozio(FILE);
  const copia = await c?.match(e.request);
  const rete = fetch(e.request).then(async (res) => {
    if (c && res.ok) await c.put(e.request, res.clone()).catch(() => {});
    return res;
  });
  e.waitUntil(rete.catch(() => {}));
  return copia || rete;
}

/** Una lettura dell'API: la rete, e se ne tiene la risposta per metodo e argomenti; senza rete la copia. */
async function lettura(e, metodo) {
  const r = e.request;
  const corpo = await r.clone().text();
  const chiave = `/__copia/${metodo}?${encodeURIComponent(corpo)}`;
  const c = await negozio(DATI);
  let res;
  try {
    res = await fetch(r);
  } catch (errore) {
    const copia = c && (await (metodo === "get_events" ? eventiSalvati(c, corpo) : c.match(chiave).then((x) => x || ultimaDi(c, metodo))).catch(() => null));
    if (copia) return copia;
    throw errore;
  }
  if (c && res.ok) e.waitUntil(salva(c, chiave, res.clone(), metodo).catch(() => {}));
  return res;
}

async function salva(c, chiave, res, metodo) {
  await c.put(chiave, res);
  if (metodo !== "get_events") return;
  const tutte = (await c.keys()).filter((k) => new URL(k.url).pathname === "/__copia/get_events");
  await Promise.all(tutte.slice(0, Math.max(0, tutte.length - EVENTI_MAX)).map((k) => c.delete(k)));
}

async function ultimaDi(c, metodo) {
  const sue = (await c.keys()).filter((k) => new URL(k.url).pathname === `/__copia/${metodo}`);
  return sue.length ? c.match(sue[sue.length - 1]) : null;
}

const due = (n) => String(n).padStart(2, "0");
const oraLocale = (d) => `${d.getFullYear()}-${due(d.getMonth() + 1)}-${due(d.getDate())}T${due(d.getHours())}:${due(d.getMinutes())}:${due(d.getSeconds())}`;

/**
 * Senza rete get_events risponde con gli eventi di tutte le letture salvate che toccano l'intervallo (con un giorno di margine: gli orari
 * sono di Roma senza scarto), da quelle con gli stessi calendari spenti se ce ne sono. Dice di essere una copia (outlook_starting e
 * stale_at, come quando il server dei calendari non risponde): la pagina lo scrive e riprova da sola.
 */
async function eventiSalvati(c, corpo) {
  let da, a, coda;
  try {
    [da, a, ...coda] = JSON.parse(corpo);
  } catch {
    return null;
  }
  const inizio = Date.parse(da) - GIORNO_MS;
  const fine = Date.parse(a) + GIORNO_MS;
  const voci = [];
  for (const k of await c.keys()) {
    const u = new URL(k.url);
    if (u.pathname !== "/__copia/get_events") continue;
    const argomenti = JSON.parse(decodeURIComponent(u.search.slice(1)));
    voci.push({ uguale: JSON.stringify(argomenti.slice(2)) === JSON.stringify(coda), chiave: k });
  }
  const scelte = voci.some((v) => v.uguale) ? voci.filter((v) => v.uguale) : voci;
  if (!scelte.length) return null;
  const visti = new Set();
  const events = [];
  let quando = 0;
  let fonti = {};
  for (const v of scelte) {
    const res = await c.match(v.chiave);
    const salvata = Date.parse(res.headers.get("date") || "") || 0;
    const dati = await res.json();
    if (salvata >= quando) [quando, fonti] = [salvata, dati.fonti || {}];
    for (const ev of dati.events || []) {
      const id = `${ev.id}|${ev.start}`;
      if (visti.has(id) || Date.parse(ev.end || ev.start) < inizio || Date.parse(ev.start) > fine) continue;
      visti.add(id);
      events.push(ev);
    }
  }
  const risposta = { events, errors: [], fonti, stale: true, stale_at: quando ? oraLocale(new Date(quando)) : null, outlook_starting: true, avviso_sorgente: "Senza rete" };
  return new Response(JSON.stringify(risposta), { headers: { "Content-Type": "application/json" } });
}
