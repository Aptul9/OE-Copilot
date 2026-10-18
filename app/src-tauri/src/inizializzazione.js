// Script di inizializzazione della finestra principale. Gira in ogni pagina che la finestra carica, ma lavora solo
// sull'origine del sito (window.__OE_ORIGINE__ la mette l'app). Sul PC ogni minuto legge le impostazioni dei promemoria
// (get_promemoria) e get_events con la stessa API della pagina (stesso sito, cookie di sessione; il terzo argomento vuole anche i
// calendari spenti nella barra laterale, che filtra solo la vista), chiede al nucleo
// dell'app (pianifica_promemoria, src-tauri/src/promemoria.rs) le voci di ogni riunione e anticipo, e per quelle arrivate
// manda la notifica e l'avviso a comparsa. Su Android i promemoria li programma l'app anche ad app chiusa (Promemoria.kt):
// la pagina chiede solo il permesso delle notifiche. Nei log non finisce niente che riguardi gli eventi.
(function () {
  "use strict";

  // Su Android la WebView lancia lo script due volte sulle pagine del sito (all'avvio del documento e a pagina iniziata):
  // la seconda volta non deve fare niente, altrimenti i giri sono doppi, il permesso si chiede due volte e un avviso può uscire due volte
  if (window.__oeInizializzato) return;
  window.__oeInizializzato = true;

  // Una navigazione fallita (rete caduta) lascia la pagina di errore del browser: si torna alla pagina di avvio, che
  // mostra il messaggio, riprova da sola e riporta al sito appena risponde. Il controllo è differito perché all'avvio
  // dello script l'indirizzo della pagina di errore non è ancora quello definitivo.
  setTimeout(() => {
    if (location.protocol === "chrome-error:" && window.__OE_AVVIO__) location.replace(window.__OE_AVVIO__);
  }, 1500);

  const PERIODO_MS = 60_000;
  const FINESTRA_MS = 24 * 3_600_000;
  const NON_ACCESSO_MS = 300_000;
  const ERRORE_MAX_MS = 300_000;
  const INTERFACCIA_OGNI_MS = 1_800_000;
  const CHIAVE_INVIATE = "oe-app:notifiche-inviate";
  const SU_ANDROID = typeof navigator !== "undefined" && /Android/i.test(navigator.userAgent || "");

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

  const PREDEFINITE = { grezze: null, suono: "Default", minimo: false, notifiche: true, comparsa: true };

  // Le impostazioni dei promemoria (Impostazioni, Promemoria) stanno sul server e si rileggono a ogni giro: un cambio vale dal giro dopo.
  // Un server senza il metodo (404) vale come i valori di partenza; null se la sessione non c'è più. Anticipi e contenuto li legge il
  // nucleo da `grezze`; qui restano gli interruttori e il suono dell'app per Windows. Il suono (Default, Reminder, Alarm3...) ha sempre
  // un nome: su Windows una notifica senza nome è muta
  async function leggiImpostazioni() {
    const r = await chiama("get_promemoria", []);
    if (r.status === 404) return PREDEFINITE;
    if (sessioneFinita(r)) return null;
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const dati = await r.json();
    const p = dati && typeof dati.promemoria === "object" && dati.promemoria ? dati.promemoria : {};
    return {
      grezze: p,
      suono: typeof p.suono === "string" && /^[A-Za-z0-9]{1,16}$/.test(p.suono) ? p.suono : "Default",
      minimo: p.contenuto === "minimo",
      notifiche: p.notifica_windows !== false,
      comparsa: p.comparsa !== false,
    };
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

  // Le sigle e i loghi delle aziende (get_personalizzazione), riletti ogni 30 minuti
  let interfaccia = null;
  let interfacciaAl = 0;

  async function leggiInterfaccia() {
    if (!interfaccia || Date.now() - interfacciaAl >= INTERFACCIA_OGNI_MS) {
      interfacciaAl = Date.now();
      try {
        const r = await chiama("get_personalizzazione", []);
        if (r.ok && !sessioneFinita(r)) interfaccia = ((await r.json()) || {}).interfaccia || {};
      } catch {
        // si tiene quella che c'è
      }
    }
    return interfaccia || {};
  }

  // Un file dei loghi di questo sito ("logos/roar.png?v=..."), mai un indirizzo di fuori
  function sulSito(valore) {
    try {
      const u = new URL(valore, `${origine}/`);
      return u.origin === origine ? u.pathname + u.search : "";
    } catch {
      return "";
    }
  }

  // I loghi letti, come data: (la finestra dell'avviso non ha il cookie del sito): uno per indirizzo, che cambia quando cambia il file
  const loghiLetti = new Map();

  function leggiLogo(indirizzo) {
    if (!loghiLetti.has(indirizzo)) {
      const lettura = fetch(indirizzo, { credentials: "same-origin" })
        .then((r) => (r.ok ? r.blob() : null))
        .then(
          (b) =>
            b &&
            new Promise((fatto) => {
              const f = new FileReader();
              f.onload = () => fatto(f.result);
              f.onerror = () => fatto(null);
              f.readAsDataURL(b);
            }),
        )
        .catch(() => null);
      loghiLetti.set(indirizzo, lettura);
      // una lettura fallita si ritenta all'avviso dopo
      lettura.then((logo) => logo || loghiLetti.delete(indirizzo));
    }
    return loghiLetti.get(indirizzo);
  }

  // Come la pagina (iniziali in lib/oe/logic.ts): "Alfa" -> "AL", "Beta Servizi" -> "BS"
  function iniziali(nome) {
    const s = nome.split("·")[0].trim().split("@")[0];
    const parole = s.split(/[\s._\-+]/).filter(Boolean);
    return (parole.length >= 2 ? parole[0][0] + parole[1][0] : s.slice(0, 2)).toUpperCase();
  }

  // Il logo dell'azienda di una voce come data:, o null (come mappaLoghi in lib/oe/logic.ts, gli Eventi personali hanno quello del gateway)
  async function logoDi(voce) {
    const loghi = (await leggiInterfaccia()).loghi || {};
    const logo = loghi[voce.azienda];
    const indirizzo = (typeof logo === "string" && logo && sulSito(logo)) || (voce.azienda === "Eventi personali" ? "/logos/personale.png" : "");
    return indirizzo ? await leggiLogo(indirizzo) : null;
  }

  // L'avviso a comparsa del PC: ora, titolo e logo dell'azienda, o la sua sigla nel suo colore come nel calendario. Con "Solo ora e azienda"
  // il titolo non parte
  async function avviso(voce, minimo) {
    const sigle = (await leggiInterfaccia()).sigle || {};
    return {
      chiave: voce.riunione,
      inizio: voce.inizio,
      ora: voce.ora,
      titolo: minimo ? "" : voce.chi,
      logo: await logoDi(voce),
      sigla: String(sigle[voce.azienda] || iniziali(voce.azienda)),
      colore: voce.colore,
    };
  }

  // Le riunioni di un giro, dalla prima, vanno in una notifica sola: con una riunione titolo e corpo del nucleo; con più riunioni il titolo
  // dice quante sono e quando comincia la prima, e c'è una riga per riunione ("11:00 Riunione", o l'azienda con "Solo ora e azienda"; il
  // toast ne mostra due). La fa l'app (notifica.rs): esce dal centro notifiche quando comincia la prima riunione e ha il logo dell'azienda
  // se è lo stesso per tutte; se non riesce, quella del plugin. Il suono è uno solo: con l'avviso a comparsa acceso lo fa l'avviso, e la
  // notifica senza nome di suono resta muta
  async function notifica(riunioni, imp) {
    const [prima] = riunioni;
    let t = { title: prima.titolo, body: prima.corpo };
    if (riunioni.length > 1) t = { title: `${prima.quando} · ${riunioni.length} riunioni`, body: riunioni.map((v) => `${v.ora} ${v.chi}`.trim().slice(0, 120)).join("\n") };
    if (!imp.comparsa) t.sound = imp.suono;
    const loghi = await Promise.all(riunioni.map(logoDi));
    const chiave = riunioni.map((v) => v.riunione).join("+");
    const dati = { ...t, chiave, inizio: prima.inizio, logo: loghi.every((l) => l === loghi[0]) ? loghi[0] : null };
    return invoca("mostra_notifica", { dati }).catch(() => invoca("plugin:notification|notify", { options: t }));
  }

  async function controlla() {
    const imp = await leggiImpostazioni();
    if (!imp) return "non-autenticato";
    // su Android i promemoria li programma l'app, anche ad app chiusa (Promemoria.kt): alla pagina resta il permesso, chiesto una volta
    if (SU_ANDROID) {
      await permessoNotifiche();
      return "android";
    }
    if (!imp.notifiche && !imp.comparsa) return "spente";
    let ora = Date.now();
    const r = await chiama("get_events", [isoLocale(new Date(ora)), isoLocale(new Date(ora + FINESTRA_MS)), true]);
    if (sessioneFinita(r)) return "non-autenticato";
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const dati = await r.json();
    const concesso = await permessoNotifiche();
    // un promemoria vale solo su dati freschi, non su quelli tenuti in cache quando l'agente non risponde
    if (dati && dati.stale) return "dati-non-freschi";
    if (!concesso) return "senza-permesso";
    // la richiesta del permesso può aver aspettato la risposta di chi usa il telefono
    ora = Date.now();
    const inviate = leggiInviate(ora);
    // le voci di ogni riunione e anticipo, già in ordine di inizio e con il testo; qui le riunioni con un anticipo arrivato e non ancora avvisato
    const voci = await invoca("pianifica_promemoria", { impostazioni: imp.grezze, eventi: Array.isArray(dati && dati.events) ? dati.events : [], ora });
    const arrivate = new Map();
    for (const voce of voci) {
      if (voce.at > ora) continue;
      if (!arrivate.has(voce.riunione)) arrivate.set(voce.riunione, { voce, chiavi: [] });
      arrivate.get(voce.riunione).chiavi.push(voce.chiave);
    }
    const nuove = [...arrivate.values()].filter(({ chiavi }) => !chiavi.every((c) => c in inviate));
    if (!nuove.length) return "ok";
    // una notifica sola per giro, anche con più riunioni o più anticipi già passati (app appena avviata); le riunioni si segnano dopo,
    // così una notifica che non riesce si riprova al giro dopo
    if (imp.notifiche) await notifica(nuove.map(({ voce }) => voce), imp);
    for (const { voce, chiavi } of nuove) for (const c of chiavi) inviate[c] = voce.inizio;
    salvaInviate(inviate);
    stato.inviate += nuove.length;
    // le riunioni del giro vanno insieme nello stesso avviso; dopo averle segnate, così un avviso che non riesce non fa ripetere le notifiche
    if (imp.comparsa) {
      const avvisi = [];
      for (const { voce } of nuove) avvisi.push(await avviso(voce, imp.minimo));
      await invoca("mostra_avviso", { avvisi, suono: imp.suono }).catch(() => {});
    }
    return "ok";
  }

  async function giro() {
    let prossimo = PERIODO_MS;
    stato.giri += 1;
    try {
      stato.esito = await controlla();
      // su Android la pagina non ha altro da fare
      if (stato.esito === "android") return;
      if (stato.esito === "non-autenticato") prossimo = NON_ACCESSO_MS;
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
