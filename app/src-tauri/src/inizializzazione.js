// Script di inizializzazione della finestra principale. Gira in ogni pagina che la finestra carica, ma lavora solo
// sull'origine del sito (window.__OE_ORIGINE__ la mette l'app). Ogni minuto chiede get_events con la stessa API della
// pagina (stesso sito, cookie di sessione) e per le riunioni che iniziano entro il preavviso manda una notifica nativa
// con il plugin notification. Nei log non finisce niente che riguardi gli eventi.
(function () {
  "use strict";

  // Una navigazione fallita (rete caduta) lascia la pagina di errore del browser: si torna alla pagina di avvio, che
  // mostra il messaggio, riprova da sola e riporta al sito appena risponde. Il controllo è differito perché all'avvio
  // dello script l'indirizzo della pagina di errore non è ancora quello definitivo.
  setTimeout(() => {
    if (location.protocol === "chrome-error:" && window.__OE_AVVIO__) location.replace(window.__OE_AVVIO__);
  }, 1500);

  // Anticipi in minuti, finché il server non ne indica altri (get_promemoria, se il metodo esiste)
  const MINUTI_PREAVVISO = [10];
  const PERIODO_MS = 60_000;
  const FINESTRA_MS = 24 * 3_600_000;
  const NON_ACCESSO_MS = 300_000;
  const ERRORE_MAX_MS = 300_000;
  const PREAVVISO_AGGIORNATO_OGNI_MS = 1_800_000;
  const CHIAVE_INVIATE = "oe-app:notifiche-inviate";
  const FUSO = "Europe/Rome";

  const origine = window.__OE_ORIGINE__;
  if (!origine || location.origin !== origine) return;

  // Solo contatori, per chi guarda dall'esterno se il giro funziona
  const stato = (window.__oeNotifiche = { giri: 0, inviate: 0, errori: 0, esito: "avvio" });

  const due = (n) => String(n).padStart(2, "0");

  // Come la pagina: ISO con lo scarto orario locale (lib/oe/logic.ts)
  function isoLocale(d) {
    const scarto = -d.getTimezoneOffset();
    const assoluto = Math.abs(scarto);
    return (
      `${d.getFullYear()}-${due(d.getMonth() + 1)}-${due(d.getDate())}T${due(d.getHours())}:${due(d.getMinutes())}:${due(d.getSeconds())}` +
      `${scarto < 0 ? "-" : "+"}${due(Math.floor(assoluto / 60))}:${due(assoluto % 60)}`
    );
  }

  const formatoRoma = new Intl.DateTimeFormat("en-GB", {
    timeZone: FUSO,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  const formatoOra = new Intl.DateTimeFormat("it-IT", { timeZone: FUSO, hour: "2-digit", minute: "2-digit" });

  // Scarto in ms tra l'ora di Roma e UTC nell'istante dato
  function scartoRoma(ms) {
    const p = Object.fromEntries(formatoRoma.formatToParts(new Date(ms)).map((x) => [x.type, x.value]));
    return Date.UTC(+p.year, +p.month - 1, +p.day, +p.hour, +p.minute, +p.second) - Math.floor(ms / 1000) * 1000;
  }

  // L'istante di un orario dell'API: senza scarto è ora di Roma (come lo manda l'agente), con scarto vale quello
  function istante(testo) {
    const m = /^(\d{4})-(\d\d)-(\d\d)[T ](\d\d):(\d\d)(?::(\d\d))?(?:\.\d+)?(Z|[+-]\d\d:?\d\d)?$/.exec(String(testo || ""));
    if (!m) return NaN;
    if (m[7]) {
      const scarto = m[7] === "Z" ? "Z" : m[7].replace(/^([+-]\d\d):?(\d\d)$/, "$1:$2");
      return Date.parse(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6] || "00"}${scarto}`);
    }
    const nominale = Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +(m[6] || 0));
    let t = nominale - scartoRoma(nominale);
    t = nominale - scartoRoma(t);
    return t;
  }

  const invoca = (comando, argomenti) => window.__TAURI_INTERNALS__.invoke(comando, argomenti);

  const chiama = (metodo, argomenti) =>
    fetch(`/api/${metodo}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(argomenti),
      credentials: "same-origin",
      cache: "no-store",
    });

  // 401, un reindirizzamento o una pagina al posto del JSON: la sessione non c'è più
  const sessioneFinita = (r) =>
    r.status === 401 || r.redirected || (r.ok && !(r.headers.get("content-type") || "").includes("application/json"));

  // Su Android 13 e successivi le notifiche vogliono il permesso POST_NOTIFICATIONS: lo si chiede una volta per pagina,
  // appena c'è una sessione, e non si manda niente finché non c'è. Sul PC il plugin risponde sempre "concesso"
  let permesso = null;
  let permessoChiesto = false;

  async function permessoNotifiche() {
    if (permesso === true) return true;
    permesso = await invoca("plugin:notification|is_permission_granted");
    if (permesso === null && !permessoChiesto) {
      permessoChiesto = true;
      permesso = (await invoca("plugin:notification|request_permission")) === "granted";
    }
    return permesso === true;
  }

  let preavviso = MINUTI_PREAVVISO;
  let preavvisoAl = 0;

  // "attivo" di get_promemoria riguarda solo i promemoria del server: l'app notifica comunque
  async function aggiornaPreavviso() {
    if (Date.now() - preavvisoAl < PREAVVISO_AGGIORNATO_OGNI_MS) return;
    preavvisoAl = Date.now();
    try {
      const r = await chiama("get_promemoria", []);
      if (r.status === 404) {
        preavviso = MINUTI_PREAVVISO;
      } else if (r.ok && !sessioneFinita(r)) {
        const dati = await r.json();
        // le impostazioni stanno dentro "promemoria" nella risposta del server; una risposta piatta si legge lo stesso
        const impostazioni = dati && typeof dati.promemoria === "object" && dati.promemoria ? dati.promemoria : dati;
        const minuti = (Array.isArray(impostazioni && impostazioni.minuti) ? impostazioni.minuti : []).map(Number).filter((n) => Number.isInteger(n) && n > 0 && n <= 1440);
        preavviso = minuti.length ? [...new Set(minuti)].sort((a, b) => a - b) : MINUTI_PREAVVISO;
      }
    } catch {
      // si tiene l'elenco che c'è
    }
  }

  function leggiInviate(ora) {
    let salvate = {};
    try {
      salvate = JSON.parse(localStorage.getItem(CHIAVE_INVIATE) || "{}") || {};
    } catch {
      salvate = {};
    }
    const attive = {};
    for (const [chiave, inizio] of Object.entries(salvate)) if (Number.isFinite(inizio) && inizio > ora - 86_400_000) attive[chiave] = inizio;
    if (Object.keys(attive).length !== Object.keys(salvate).length) salvaInviate(attive);
    return attive;
  }

  function salvaInviate(inviate) {
    try {
      localStorage.setItem(CHIAVE_INVIATE, JSON.stringify(inviate));
    } catch {
      // senza memoria la prossima volta potrebbe ripetersi un avviso
    }
  }

  function testo(ev, inizio, ora) {
    const minuti = Math.max(1, Math.round((inizio - ora) / 60_000));
    const righe = [`Alle ${formatoOra.format(new Date(inizio))}`];
    const luogo = String(ev.location || "").trim();
    // il link per entrare non va nell'avviso
    if (luogo && !/https?:\/\//i.test(luogo)) righe.push(luogo.slice(0, 120));
    return { title: `Tra ${minuti} min · ${String(ev.title || "(senza titolo)").slice(0, 120)}`, body: righe.join("\n") };
  }

  async function controlla() {
    let ora = Date.now();
    const r = await chiama("get_events", [isoLocale(new Date(ora)), isoLocale(new Date(ora + FINESTRA_MS))]);
    if (sessioneFinita(r)) return "non-autenticato";
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const dati = await r.json();
    const concesso = await permessoNotifiche();
    // un promemoria vale solo su dati freschi, non su quelli tenuti in cache quando l'agente non risponde
    if (dati && dati.stale) return "dati-non-freschi";
    if (!concesso) return "senza-permesso";
    // la richiesta del permesso può aver aspettato la risposta di chi usa il telefono
    ora = Date.now();
    await aggiornaPreavviso();
    const inviate = leggiInviate(ora);
    for (const ev of Array.isArray(dati && dati.events) ? dati.events : []) {
      if (!ev || ev.allDay || ev.nascosto || !ev.start) continue;
      const inizio = istante(ev.start);
      if (!(inizio > ora)) continue;
      const raggiunti = preavviso.filter((m) => inizio - m * 60_000 <= ora);
      const chiavi = raggiunti.map((m) => `${ev.id}|${ev.start}|${m}`);
      if (!chiavi.length || chiavi.every((c) => c in inviate)) continue;
      // una notifica sola per riunione, anche se più anticipi sono già passati (app appena avviata)
      await invoca("plugin:notification|notify", { options: testo(ev, inizio, ora) });
      for (const c of chiavi) inviate[c] = inizio;
      salvaInviate(inviate);
      stato.inviate += 1;
    }
    return "ok";
  }

  async function giro() {
    let prossimo = PERIODO_MS;
    stato.giri += 1;
    try {
      if (await invoca("notifiche_attive")) {
        stato.esito = await controlla();
        if (stato.esito === "non-autenticato") prossimo = NON_ACCESSO_MS;
      } else {
        stato.esito = "spente";
      }
      stato.errori = 0;
    } catch {
      stato.errori += 1;
      stato.esito = "errore";
      prossimo = Math.min(PERIODO_MS * 2 ** stato.errori, ERRORE_MAX_MS);
    }
    setTimeout(giro, prossimo);
  }

  setTimeout(giro, 3000);
})();
