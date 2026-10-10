// ---- Stato globale ----
let calendar = null;
let apiReady = false;
let interfacciaPronta = false;
let legendaGiaFatta = false;
let tentativiOutlook = 0;
let timerRitentativo = null;
const MAX_TENTATIVI = 12;   // ~2 minuti: quanto basta a un avvio a freddo di Outlook

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

// Schede della sezione Timesheet, dalla personalizzazione (personalizzazione.json,
// letta da Python all'avvio): {nome, colore, logo?, url?, azione?, sotto?}.
// "url" apre il gestionale in Chrome; "azione" chiama una funzione registrata
// da un'estensione personale (web/personale/estensione.js).
let TIMESHEET_AZIENDE = [];

// Gli errori JS in una finestra pywebview non si vedono da nessuna parte:
// li si rimanda a Python, che li scrive nel log.
function reportJs(msg) {
  try {
    if (window.pywebview && window.pywebview.api && window.pywebview.api.log_js) {
      window.pywebview.api.log_js(String(msg));
    }
  } catch (e) { /* niente da fare */ }
}
// Tutte le chiamate a Python passano di qui. Prima ognuna gestiva l'errore per
// conto suo, e in 25 punti su 27 non lo gestiva affatto: se la chiamata falliva,
// il pezzo di interfaccia che dipendeva da lei restava com'era (elenco vuoto,
// "Lettura da Outlook..." per sempre) senza che niente lo dicesse.
const api = new Proxy({}, {
  get: (_, nome) => (...args) => {
    const ponte = window.pywebview && window.pywebview.api;
    const funzione = ponte && ponte[nome];
    if (typeof funzione !== "function") {
      const e = new Error("collegamento con l'app non disponibile (" + String(nome) + ")");
      reportJs(e.message);
      return Promise.reject(e);
    }
    return Promise.resolve(funzione.apply(ponte, args)).catch((err) => {
      const testo = (err && err.message) ? err.message : String(err);
      reportJs(`api.${String(nome)}: ${testo}`);
      try { showBanner(`⚠ ${String(nome)}: ${testo}`, "errore"); } catch (e) { /* prima dell'avvio */ }
      throw err;
    });
  },
});

window.addEventListener("error", (e) =>
  reportJs(`${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) =>
  reportJs("promise non gestita: " + e.reason));

// pywebview espone l'API Python solo dopo questo evento.
window.addEventListener("pywebviewready", init);
// Fallback: se l'evento è già passato, riprova dopo il load.
window.addEventListener("load", () => setTimeout(() => { if (!apiReady) init(); }, 800));

function init() {
  if (apiReady) return;
  if (!window.pywebview || !window.pywebview.api) return;
  apiReady = true;
  // Le impostazioni personali servono prima di disegnare qualunque cosa (sigle,
  // calendari secondari...): si chiedono per prime. Se la richiesta fallisce,
  // l'app parte lo stesso con i valori neutri.
  api.get_personalizzazione()
    .then(applicaPersonalizzazione, () => {})
    .finally(avviaInterfaccia);
}

function avviaInterfaccia() {
  try {
    if (typeof FullCalendar === "undefined") {
      showBanner("⚠ FullCalendar non caricato: serve la connessione a internet al primo avvio.");
      reportJs("init: FullCalendar non caricato");
    } else {
      setupCalendar();
    }
    setupUI();
    setupAppSettings();
    // Estensioni personali già registrate (script caricati dopo app.js).
    interfacciaPronta = true;
    ESTENSIONI.forEach(_avviaEstensione);
    refreshLegend();
    // Applica tema e intervallo salvati.
    api.get_config().then((c) => {
      applicaTema(c.theme || "auto");
      applicaZoom(parseInt(c.zoom_calendario, 10) || ZOOM_DEFAULT, false);
      intervalloSync = (parseInt(c.reminder_poll_seconds, 10) || 60) * 1000;
      document.dispatchEvent(new Event("sync-interval-changed"));
    });
    avviaAutoSync();
    window.__appPronta = true;   // spegne la sentinella in index.html
    // Traccia esplicita nel log: verifica.py la usa come prova che l'interfaccia
    // è partita davvero, non solo che il processo è vivo.
    api.log_ui("interfaccia pronta");
  } catch (e) {
    reportJs("init: eccezione " + (e && e.stack ? e.stack : e));
  }
}

// ================= CALENDARIO =================
function setupCalendar() {
  calendar = new FullCalendar.Calendar($("#calendar"), {
    initialView: "timeGridWeek",
    locale: "it",
    firstDay: 1,
    nowIndicator: true,
    height: "100%",
    // Altezza naturale con scorrimento: così gli slot restano leggibili e una
    // riunione da mezz'ora ha spazio per orario e titolo (con expandRows gli
    // slot venivano schiacciati e gli eventi corti sparivano).
    expandRows: false,
    slotMinTime: "07:00:00",
    slotMaxTime: "22:00:00",
    // All'apertura la vista è posizionata sulle 9; scorrendo su si vedono
    // comunque le ore prima (07-09).
    scrollTime: "09:00:00",
    slotDuration: "00:30:00",
    slotLabelInterval: "01:00:00",
    // Riunioni in sovrapposizione affiancate invece che una sopra l'altra:
    // altrimenti quella davanti copre l'orario di quella dietro.
    slotEventOverlap: false,
    slotLabelFormat: { hour: "2-digit", minute: "2-digit", hour12: false },
    eventTimeFormat: { hour: "2-digit", minute: "2-digit", hour12: false },
    // Niente riga "Giornata": gli eventi tutto-il-giorno e le festività vengono
    // mostrati sotto l'intestazione del giorno (vedi segnaGiorniRossi).
    allDaySlot: false,
    dayHeaderFormat: { weekday: "short", day: "numeric" },
    // La barra comandi è nostra, in stile Teams: quella predefinita resta spenta.
    headerToolbar: false,
    events: loadEvents,
    eventClick: onEventClick,
    eventContent: renderEventContent,
    datesSet: onDatesSet,
    // Gli eventi appena caricati nascono visibili: se una ricerca è attiva va
    // riapplicata, altrimenti riaffiorerebbero quelli esclusi.
    eventsSet: () => {
      segnaGiorniRossi();
      if ($("#search").value.trim()) applicaRicerca();
    },
  });
  calendar.render();
}

function onDatesSet(info) {
  const label = $("#range-label");
  if (label) label.textContent = capitalizza(info.view.title);
  $$("#view-switch button").forEach((b) =>
    b.classList.toggle("is-active", b.dataset.view === info.view.type));
  // Il mini-calendario segue il periodo mostrato dal calendario grande.
  minicalSelezione = { start: info.start, end: info.end };
  if (!minicalMese) minicalMese = new Date(info.start.getFullYear(), info.start.getMonth(), 1);
  renderMinical();
  // Cambiando settimana/vista le colonne sono nuove: vanno rimarcate.
  segnaGiorniRossi();
}

// ================= MINI CALENDARIO =================
let minicalMese = null;         // primo giorno del mese mostrato
let minicalSelezione = null;    // intervallo evidenziato (dal calendario grande)
const MESI = ["Gennaio", "Febbraio", "Marzo", "Aprile", "Maggio", "Giugno",
  "Luglio", "Agosto", "Settembre", "Ottobre", "Novembre", "Dicembre"];

function stessoGiorno(a, b) {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth()
    && a.getDate() === b.getDate();
}

function renderMinical() {
  const grid = $("#minical-grid");
  const tit = $("#minical-title");
  if (!grid || !minicalMese) return;
  const anno = minicalMese.getFullYear(), mese = minicalMese.getMonth();
  tit.textContent = `${MESI[mese]} ${anno}`;

  const oggi = new Date();
  let html = ["L", "M", "M", "G", "V", "S", "D"]
    .map((g) => `<span class="mc-dow">${g}</span>`).join("");

  // La griglia parte dal lunedì della settimana che contiene il giorno 1.
  const primo = new Date(anno, mese, 1);
  const scarto = (primo.getDay() + 6) % 7;   // 0 = lunedì
  const inizio = new Date(anno, mese, 1 - scarto);

  for (let i = 0; i < 42; i++) {
    const d = new Date(inizio.getFullYear(), inizio.getMonth(), inizio.getDate() + i);
    const cls = ["mc-day"];
    if (d.getMonth() !== mese) cls.push("mc-off");
    if (stessoGiorno(d, oggi)) cls.push("mc-today");
    if (minicalSelezione && d >= minicalSelezione.start && d < minicalSelezione.end)
      cls.push("mc-sel");
    html += `<button class="${cls.join(" ")}" data-iso="${d.getFullYear()}-${d.getMonth()}-${d.getDate()}">${d.getDate()}</button>`;
  }
  grid.innerHTML = html;
  grid.querySelectorAll(".mc-day").forEach((b) =>
    b.addEventListener("click", () => {
      const [y, m, g] = b.dataset.iso.split("-").map(Number);
      if (calendar) calendar.gotoDate(new Date(y, m, g));
    }));
}

function minicalCambiaMese(delta) {
  if (!minicalMese) minicalMese = new Date();
  minicalMese = new Date(minicalMese.getFullYear(), minicalMese.getMonth() + delta, 1);
  renderMinical();
}

// ---- Giorni non lavorativi + festività/eventi tutto-il-giorno --------------
// Weekend e festività si distinguono a colpo d'occhio; le festività e gli eventi
// "tutto il giorno" NON stanno nella griglia ma sotto l'intestazione del giorno.
let festiviInfo = {};   // "YYYY-MM-DD" -> nome festività
let allDayInfo = {};    // "YYYY-MM-DD" -> [{title, color}] eventi tutto-il-giorno

function _giorno(d) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function raccogliFestivi(eventi) {
  festiviInfo = {};
  allDayInfo = {};
  eventi.forEach((e) => {
    if (!e.allDay) return;
    if (e.nascosto) return;          // calendario spento: niente riga sotto il giorno
    const da = new Date(e.start);
    let ultimo = e.end ? new Date(e.end) : new Date(da);
    // La fine di un evento "tutto il giorno" è ESCLUSIVA: "1 nov → 2 nov"
    // significa solo il 1. Senza questo, ogni festività di un giorno tingeva di
    // rosso anche il giorno dopo (il 2 novembre risultava "Tutti i Santi").
    if (e.end && ultimo > da && ultimo.getHours() === 0 && ultimo.getMinutes() === 0)
      ultimo = new Date(ultimo.getTime() - 86400000);
    for (let d = new Date(da); _giorno(d) <= _giorno(ultimo); d.setDate(d.getDate() + 1)) {
      const g = _giorno(d);
      if (e.account === "Festività") {
        // Legate al calendario Festività: spegnendolo spariscono anche queste.
        festiviInfo[g] = e.title;
      } else {
        (allDayInfo[g] = allDayInfo[g] || []).push({ title: e.title, color: e.color });
      }
    }
  });
}

function segnaGiorniRossi() {
  // Si scrive direttamente sul DOM: le festività arrivano dopo che le colonne
  // sono già disegnate, e ridisegnare la vista a ogni caricamento sfarfalla.
  // Colonne del corpo: classi weekend/festivo.
  document.querySelectorAll(
    "#calendar .fc-timegrid-col[data-date], #calendar .fc-daygrid-day[data-date]"
  ).forEach((el) => {
    const g = el.dataset.date;
    const dow = new Date(g + "T00:00:00").getDay();
    el.classList.toggle("giorno-weekend", dow === 0 || dow === 6);
    el.classList.toggle("giorno-festivo", g in festiviInfo);
  });
  // Intestazioni: classi + riga extra sotto la data (festività in rosso, chip
  // per gli eventi tutto-il-giorno).
  document.querySelectorAll("#calendar .fc-col-header-cell[data-date]").forEach((th) => {
    const g = th.dataset.date;
    const dow = new Date(g + "T00:00:00").getDay();
    th.classList.toggle("giorno-weekend", dow === 0 || dow === 6);
    th.classList.toggle("giorno-festivo", g in festiviInfo);
    const pezzi = [];
    if (festiviInfo[g])
      pezzi.push(`<div class="fc-festivo-pill" title="${escapeHtml(festiviInfo[g])}">${escapeHtml(festiviInfo[g])}</div>`);
    (allDayInfo[g] || []).forEach((ev) =>
      pezzi.push(`<div class="fc-allday-chip" style="background:${tinta(ev.color, 0.16)};border-left-color:${ev.color}" title="${escapeHtml(ev.title)}">${escapeHtml(ev.title)}</div>`));
    let extra = th.querySelector(".fc-day-extra");
    if (pezzi.length) {
      if (!extra) {
        extra = document.createElement("div");
        extra.className = "fc-day-extra";
        th.appendChild(extra);
      }
      extra.innerHTML = pezzi.join("");
    } else if (extra) {
      extra.remove();
    }
  });
  restringiWeekend();
}

// Sabato e domenica più stretti, così i giorni feriali hanno più spazio. Si
// agisce sulle <col> (uguali per intestazione e corpo, quindi restano allineate).
// FullCalendar non prevede colonne di larghezza diversa: è un ritocco, se un
// giorno le colonne si disallineano basta togliere questa chiamata.
function restringiWeekend() {
  // In FullCalendar v6 le colonne giorno NON stanno nelle <colgroup> (che
  // definiscono solo l'asse): la larghezza va messa sulle celle stesse, sia
  // dell'intestazione sia del corpo, e con lo stesso valore restano allineate.
  const LARG = "54px";
  document.querySelectorAll(
    "#calendar .fc-col-header-cell[data-date], #calendar .fc-timegrid-col[data-date]"
  ).forEach((el) => {
    const dow = new Date(el.dataset.date + "T00:00:00").getDay();
    el.style.width = (dow === 0 || dow === 6) ? LARG : "";
  });
}

function loadEvents(info, success, failure) {
  hideBanner();
  syncInizio();
  api.get_events(info.startStr, info.endStr).then((res) => {
    if (res.outlook_starting) {
      // Le caselle non rispondono ancora (Outlook si sta avviando, o il server
      // dei calendari non si raggiunge): si mostrano i dati salvati e si riprova
      // da soli, invece di lasciare all'utente il compito di premere Aggiorna.
      mostraDatiVecchi(res.stale_at, res.avviso_sorgente);
      pianificaRitentativo();
      syncFine(false);
    } else {
      // Le caselle sono tornate dopo un periodo di dati salvati: va riletto
      // anche l'elenco dei calendari. I ritentativi rapidi lo fanno, ma dopo un
      // po' smettono: se la sorgente tornava più tardi (un avvio senza rete)
      // la barra laterale restava senza le caselle fino al riavvio.
      const eraFerma = !$("#stale").classList.contains("hidden");
      tentativiOutlook = 0;
      nascondiDatiVecchi();
      if (res.errors && res.errors.length) showBanner("⚠ " + res.errors.join("  ·  "));
      syncFine(true);
      if (eraFerma) refreshLegend();
    }
    // Prima si raccolgono le festività (per la riga sotto il giorno), poi si
    // tengono per la griglia solo gli eventi visibili e NON festività (queste
    // ultime si mostrano solo sotto la data, mai come blocco nel calendario).
    // Il pallino accanto a ogni calendario racconta QUESTA lettura, non lo stato
    // di quando si è aperto l'elenco: prima poteva restare verde per ore mentre
    // una sorgente non rispondeva più.
    statoFonti = res.fonti || {};
    aggiornaPallini();
    raccogliFestivi(res.events || []);
    const evs = (res.events || [])
      .filter((e) => !e.nascosto && e.account !== "Festività")
      .map((e) => ({
        id: e.id,
        title: e.title,
        start: e.start,
        end: e.end,
        allDay: e.allDay,
        // Riempimento tenue + barretta piena: è la resa delle riunioni in Teams.
        backgroundColor: tinta(e.color, 0.14),
        borderColor: e.color,
        classNames: CALENDARI_SECONDARI.has(e.alias) ? ["secondario"] : [],
        extendedProps: e,
      }));
    success(evs);
  }).catch((err) => {
    showBanner("Errore nel caricamento: " + err);
    // Lettura fallita: i pallini non possono restare a pulsare all'infinito.
    statoFonti = {};
    $$("#legend .acct-health").forEach((p) => {
      p.className = "acct-health bad";
      p.title = "Lettura non riuscita: " + err;
    });
    syncFine(false);
    failure(err);
  });
}

// ================= SINCRONIZZAZIONE =================
let ultimaSync = null;       // Date dell'ultimo aggiornamento riuscito
let sincronizzando = 0;      // richieste in corso (per la barra)

function syncInizio() {
  sincronizzando++;
  $("#sync-bar").classList.remove("hidden");
  $("#btn-sync").classList.add("is-syncing");
  aggiornaStatoSync();
}

function syncFine(riuscito) {
  sincronizzando = Math.max(0, sincronizzando - 1);
  if (riuscito) ultimaSync = new Date();
  if (sincronizzando === 0) {
    $("#sync-bar").classList.add("hidden");
    $("#btn-sync").classList.remove("is-syncing");
  }
  aggiornaStatoSync();
}

function aggiornaStatoSync() {
  const el = $("#sync-status");
  if (!el) return;
  if (sincronizzando > 0) { el.textContent = "Sincronizzo…"; return; }
  if (!ultimaSync) { el.textContent = "—"; return; }
  const sec = Math.round((Date.now() - ultimaSync.getTime()) / 1000);
  if (sec < 10) el.textContent = "Sincronizzato ora";
  else if (sec < 60) el.textContent = `Sincronizzato ${sec} sec fa`;
  else {
    const min = Math.round(sec / 60);
    el.textContent = min < 60
      ? `Sincronizzato ${min} min fa`
      : `Sincronizzato ${Math.round(min / 60)} h fa`;
  }
}

/** Ricontrolla Outlook da solo: così un nuovo evento compare senza fare nulla. */
let intervalloSync = 60000;   // ricontrollo automatico (modificabile da impostazioni)
let _timerSync = null;

function avviaAutoSync() {
  setInterval(aggiornaStatoSync, 15000);           // aggiorna la scritta
  const pianifica = () => {
    if (_timerSync) clearInterval(_timerSync);
    _timerSync = setInterval(() => {               // ricarica gli eventi
      if (calendar && sincronizzando === 0) calendar.refetchEvents();
    }, intervalloSync);
  };
  pianifica();
  // Se l'intervallo cambia nelle impostazioni, ripianifica.
  document.addEventListener("sync-interval-changed", pianifica);
}

function forzaSync() {
  // I pallini si spengono subito: così non resta appeso il verde della lettura
  // precedente mentre questa è ancora in corso.
  palliniInAttesa();
  if (calendar) calendar.refetchEvents();
}

/** Mette tutti i pallini in "sto controllando": colore neutro e pulsante. */
function palliniInAttesa() {
  $$("#legend .acct-health").forEach((p) => {
    p.className = "acct-health attesa";
    p.title = "Controllo in corso…";
  });
}

// Pallino d'angolo: iniziali colorate per (quasi) tutti, perché i loghi delle
// aziende sono spesso troppo piccoli per leggersi in un cerchietto. Fa eccezione
// "Eventi personali", con un'icona-persona che si legge bene anche piccola.
// Gli altri loghi e le sigle arrivano dalla personalizzazione.
let LOGHI_AZIENDA = { "Eventi personali": "logos/personale.png" };

// Sigla (2 lettere) mostrata nel pallino, per avere codici puliti e non ambigui.
let SIGLE_AZIENDA = {};

// Calendari "secondari": partecipazione non obbligatoria. Gli eventi si mostrano
// sbiaditi per distinguerli da quelli dove devi esserci.
let CALENDARI_SECONDARI = new Set();

// Calendari di caselle che in Teams non esistono come account (per esempio le
// caselle condivise): per i loro eventi il pulsante "Apri in Teams" non c'è.
let CALENDARI_SENZA_TEAMS = new Set();
// Da dove arrivano le caselle: "outlook" (Outlook su questo PC) o "remota" (un
// agente su un server). Cambiano solo i testi: i dati hanno la stessa forma.
let SORGENTE_REMOTA = false;

/** Applica le impostazioni personali ricevute da Python. */
function applicaPersonalizzazione(p) {
  const ui = (p && p.interfaccia) || {};
  if (Array.isArray(ui.schede_timesheet)) TIMESHEET_AZIENDE = ui.schede_timesheet;
  LOGHI_AZIENDA = Object.assign({ "Eventi personali": "logos/personale.png" }, ui.loghi || {});
  SIGLE_AZIENDA = Object.assign({}, ui.sigle || {});
  CALENDARI_SECONDARI = new Set(ui.calendari_secondari || []);
  CALENDARI_SENZA_TEAMS = new Set(ui.calendari_senza_teams || []);
  SORGENTE_REMOTA = !!(p && p.sorgente === "remota");
  if (SORGENTE_REMOTA) {
    // I testi della pagina parlano di Outlook su questo PC: qui non vale.
    const nota = $("#nota-calendari");
    if (nota) nota.textContent = "Sono le caselle lette dal server dei calendari: "
      + "gli accessi si fanno lì, su questo PC non serve Outlook.";
    const origine = $("#nota-origine");
    if (origine) origine.innerHTML = "Gli eventi vengono letti dal <b>server dei "
      + "calendari</b>, che tiene gli accessi alle caselle. Su questo PC non serve Outlook.";
  }
}

// ---- Estensioni personali ---------------------------------------------------
// Uno script in web/personale/ (fuori dal repository) può aggiungere funzioni
// all'app chiamando registraEstensione({setup, azioni}). "azioni" sono le
// funzioni che le schede Timesheet richiamano con il loro campo "azione".
const AZIONI_TIMESHEET = {};
const ESTENSIONI = [];
function _avviaEstensione(e) {
  if (typeof e.setup !== "function") return;
  try { e.setup(); } catch (err) { reportJs("estensione: " + (err && err.stack ? err.stack : err)); }
}
function registraEstensione(e) {
  ESTENSIONI.push(e);
  Object.assign(AZIONI_TIMESHEET, e.azioni || {});
  // Se l'interfaccia è già partita, l'estensione si avvia subito.
  if (interfacciaPronta) _avviaEstensione(e);
}
window.registraEstensione = registraEstensione;

// Pallino in alto a destra dell'evento: identifica l'azienda occupando pochissimo
// spazio, così il nome non deve più prendersi una riga intera in fondo.
function _cornerAzienda(p) {
  const alias = p.alias || p.account || "";
  if (!alias) return "";
  const logo = LOGHI_AZIENDA[alias];
  if (logo)
    return `<span class="ev-corner" title="${escapeHtml(alias)}"><img src="${logo}" alt=""></span>`;
  const sigla = SIGLE_AZIENDA[alias] || iniziali(alias);
  return `<span class="ev-corner iniziali" style="background:${p.color}" title="${escapeHtml(alias)}">${escapeHtml(sigla)}</span>`;
}

// Ora d'inizio "HH:MM" (una sola resa, uguale per tutti gli eventi).
function _hhmm(d) {
  if (!d) return "";
  const p = (n) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

function renderEventContent(arg) {
  const p = arg.event.extendedProps;
  const compatta = arg.view.type === "dayGridMonth" || arg.view.type.startsWith("list");

  if (compatta) {
    // Mese e agenda: una riga sola, orario + titolo.
    const ora = arg.timeText ? `<span class="ev-time">${escapeHtml(arg.timeText)}</span> ` : "";
    return { html: `<div class="ev-line">${ora}${escapeHtml(arg.event.title)}</div>` };
  }
  // Sempre lo stesso schema: orario (piccolo, grassetto) SOPRA, titolo SOTTO —
  // anche gli eventi corti. Lo spazio per i 15 min si recupera con l'interlinea
  // stretta impostata nel CSS.
  const corner = arg.event.allDay ? "" : _cornerAzienda(p);
  // Nella vista Giorno c'è tutta la larghezza che serve: accanto all'orario si
  // scrive anche di che azienda è la riunione.
  const azienda = arg.view.type === "timeGridDay" ? (p.alias || p.account || "") : "";
  const testoOra = azienda
    ? `${escapeHtml(arg.timeText)}<span class="ev-az"> · ${escapeHtml(azienda)}</span>`
    : escapeHtml(arg.timeText || "");
  const ora = arg.timeText ? `<div class="ev-meta">${testoOra}</div>` : "";
  return {
    html: corner +
      `<div class="ev-body">${ora}<div class="ev-title">${escapeHtml(arg.event.title)}</div></div>`,
  };
}

// ================= ZOOM DEL CALENDARIO =================
// I pulsanti +/- alzano o abbassano le righe della griglia: più alte = riunioni
// più grandi e leggibili, più basse = si vede più giornata senza scorrere.
const ZOOM_MIN = 26, ZOOM_MAX = 110, ZOOM_PASSO = 8, ZOOM_DEFAULT = 58;
let altezzaSlot = ZOOM_DEFAULT;

function applicaZoom(px, salva) {
  const nuova = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(px || ZOOM_DEFAULT)));
  // Il ridisegno costa una rilettura completa di Outlook (~3s): si fa solo se
  // l'altezza cambia davvero. All'avvio, con lo zoom salvato uguale a quello di
  // partenza, prima si ridisegnava per niente e il calendario veniva letto due
  // volte a ogni apertura.
  const cambiata = nuova !== altezzaSlot;
  altezzaSlot = nuova;
  document.documentElement.style.setProperty("--slot-h", altezzaSlot + "px");
  const giu = $("#btn-zoom-out"), su = $("#btn-zoom-in"), rip = $("#btn-zoom-reset");
  if (giu) giu.disabled = altezzaSlot <= ZOOM_MIN;
  if (su) su.disabled = altezzaSlot >= ZOOM_MAX;
  if (rip) rip.disabled = altezzaSlot === ZOOM_DEFAULT;   // già alla misura normale
  // Il ridisegno va fatto dopo che il browser ha applicato la nuova altezza,
  // altrimenti FullCalendar rimisura la griglia vecchia.
  if (calendar && cambiata)
    setTimeout(() => {
      // updateSize() da solo non basta: FullCalendar rimisura la griglia ma NON
      // ricalcola le posizioni (in pixel) delle riunioni, che restano dov'erano.
      // Si ridisegna quindi la vista, rimettendosi sullo stesso ORARIO: tenere
      // lo scorrimento in pixel farebbe scivolare la vista in avanti a ogni
      // ingrandimento (le righe sono più alte) e le riunioni del mattino
      // uscirebbero sopra il bordo.
      const ora = _oraInCima();
      calendar.changeView(calendar.view.type);
      calendar.updateSize();
      if (ora) calendar.scrollToTime(ora);
    }, 0);
  if (salva) api.save_config({ zoom_calendario: altezzaSlot });
}

// Orario della riga in cima alla parte visibile della griglia: serve per
// rimettersi allo stesso punto dopo un ridisegno, indipendentemente dall'altezza
// delle righe.
function _oraInCima() {
  const sc = document.querySelector("#calendar .fc-scroller-liquid-absolute");
  if (!sc) return null;
  const y = sc.getBoundingClientRect().top;
  let migliore = null, distanza = Infinity;
  document.querySelectorAll("#calendar .fc-timegrid-slot-lane[data-time]").forEach((el) => {
    const d = Math.abs(el.getBoundingClientRect().top - y);
    if (d < distanza) { distanza = d; migliore = el.dataset.time; }
  });
  return migliore;
}

function cambiaZoom(verso) {
  applicaZoom(altezzaSlot + verso * ZOOM_PASSO, true);
}

// ================= SCHEDA EVENTO =================
function onEventClick(arg) {
  arg.jsEvent.preventDefault();
  const p = arg.event.extendedProps;
  const pop = $("#event-pop");
  const rows = [];
  rows.push(`<div class="row"><b>${escapeHtml(fmtRange(arg.event))}</b></div>`);
  rows.push(`<div class="row"><span class="pop-badge" style="background:${p.color}">${escapeHtml(p.alias || p.account || "")}</span></div>`);
  if (p.location) rows.push(`<div class="row"><span class="pop-k">Luogo</span> ${escapeHtml(p.location)}</div>`);
  if (p.inoltrata_da)
    // Copia messa da Power Automate: come organizzatore risulteresti tu (la
    // casella che la ospita), che qui non direbbe niente.
    rows.push(`<div class="row"><span class="pop-k">Origine</span> copia dal calendario ${escapeHtml(p.inoltrata_da)}</div>`);
  else if (p.organizer)
    rows.push(`<div class="row"><span class="pop-k">Organizza</span> ${escapeHtml(p.organizer)}</div>`);

  // Un evento è di Outlook se il suo id contiene un EntryID (prima del "|") e
  // non è locale né festività. Quelli letti dal server dei calendari (remoto)
  // non hanno un EntryID: se vengono da una casella di posta si possono
  // comunque aprire in Outlook cercandoli per ora e titolo.
  const idOutlook = !p.is_local && !p.remoto && p.id && p.id.indexOf("|") > 0
    && !String(p.id).startsWith("local::") && !String(p.id).startsWith("builtin::")
    && !String(p.id).startsWith("ics_");
  const casellaRemota = !!p.remoto && !p.is_google;

  const azioni = [];
  if (p.online_url)
    azioni.push(`<button class="join" data-url="${encodeURI(p.online_url)}">Partecipa</button>`);
  if (idOutlook)
    azioni.push(`<button class="ghost" data-outlook="${escapeHtml(p.id)}">Apri in Outlook</button>`);
  else if (casellaRemota)
    azioni.push(`<button class="ghost" data-outlook-remoto="1">Apri in Outlook</button>`);
  // Teams ha piu' account collegati: si passa la casella dell'evento cosi'
  // l'app si posiziona su quella prima di aprire la riunione. Il pulsante c'è
  // per le caselle Outlook e comunque per ogni riunione con un link Teams
  // (capita anche sui calendari non Outlook, se invita un cliente).
  // Le caselle che in Teams non esistono come account (per esempio le caselle
  // condivise) non hanno il pulsante: non porterebbe da nessuna parte.
  const linkTeams = /teams\.(microsoft\.com|live\.com)/i.test(p.online_url || "");
  // Nemmeno per le copie di un'azienda inoltrata: stanno nella casella di
  // un'altra azienda, e il pulsante porterebbe sul calendario sbagliato.
  if ((idOutlook || casellaRemota || linkTeams) && String(p.account || "").includes("@")
      && !CALENDARI_SENZA_TEAMS.has(p.alias) && !p.inoltrata_da)
    azioni.push(`<button class="ghost" data-teams="${escapeHtml(p.account)}"
      data-teams-url="${escapeHtml(p.online_url || "")}">Apri in Teams</button>`);
  if (p.is_local)
    azioni.push(`<button class="del" data-del="${escapeHtml(p.uid)}">Elimina</button>`);

  pop.innerHTML =
    `<h4><span class="dot" style="background:${p.color}"></span>
       <span>${escapeHtml(arg.event.title)}</span></h4>
     ${rows.join("")}
     ${azioni.length ? `<div class="pop-actions">${azioni.join("")}</div>` : ""}`;
  pop.querySelectorAll("button[data-url]").forEach((b) =>
    b.addEventListener("click", () => api.open_external(b.dataset.url)));
  pop.querySelectorAll("button[data-outlook]").forEach((b) =>
    b.addEventListener("click", () => {
      api.open_in_outlook(b.dataset.outlook);
      pop.classList.add("hidden");
    }));
  pop.querySelectorAll("button[data-outlook-remoto]").forEach((b) =>
    b.addEventListener("click", () => {
      api.apri_in_outlook_remoto(p.account, p.start, arg.event.title).then((res) => {
        if (res && !res.ok) showBanner("⚠ " + (res.error || "Evento non trovato in Outlook"));
      }, () => {});
      pop.classList.add("hidden");
    }));
  pop.querySelectorAll("button[data-teams]").forEach((b) =>
    b.addEventListener("click", () => {
      api.apri_in_teams(b.dataset.teamsUrl, b.dataset.teams);
      pop.classList.add("hidden");
    }));
  pop.querySelectorAll("button[data-del]").forEach((b) =>
    b.addEventListener("click", () => {
      api.delete_local_event(b.dataset.del).then(() => {
        pop.classList.add("hidden");
        refreshLegend();
        if (calendar) calendar.refetchEvents();
      });
    }));

  // posiziona vicino al click, restando dentro la finestra
  pop.classList.remove("hidden");
  const x = Math.min(arg.jsEvent.clientX + 8, window.innerWidth - 322);
  const y = Math.min(arg.jsEvent.clientY, window.innerHeight - pop.offsetHeight - 16);
  pop.style.left = Math.max(10, x) + "px";
  pop.style.top = Math.max(10, y) + "px";
}
document.addEventListener("click", (e) => {
  const pop = $("#event-pop");
  if (!pop.contains(e.target) && !e.target.closest(".fc-event")) pop.classList.add("hidden");
});

// ================= "I MIEI CALENDARI" (pannello) =================
// Elenco verticale per account, come in Teams: un interruttore per casella
// accende/spegne in blocco tutti i suoi calendari.
// Esito dell'ultima lettura per azienda: {"Azienda": {stato, msg}}
let statoFonti = {};

const CLASSE_SALUTE = { ok: "good", avviso: "warn", errore: "bad" };

function _salute(azienda, d) {
  // Conta prima l'ultima lettura; se non c'è ancora, lo stato dei calendari.
  const f = statoFonti[azienda];
  if (f) return { classe: CLASSE_SALUTE[f.stato] || "warn", titolo: f.msg || "" };
  if (d && d.rotto) return { classe: "bad", titolo: d.motivi.join(" · ") || "Non raggiungibile" };
  if (d && d.avviso) return { classe: "warn", titolo: d.motivi.join(" · ") || "Dati non aggiornati" };
  return { classe: "warn", titolo: "Non ancora letto" };
}

/** Ridipinge i pallini senza rileggere l'elenco dei calendari (costa una
 *  interrogazione a Outlook): si usa dopo ogni caricamento eventi. */
function aggiornaPallini() {
  $$("#legend .acct").forEach((b) => {
    const pallino = b.querySelector(".acct-health");
    if (!pallino) return;
    const s = _salute(b.dataset.azienda, null);
    pallino.className = "acct-health " + s.classe;
    pallino.title = s.titolo;
  });
}

function refreshLegend() {
  api.get_calendars().then((res) => {
    const legend = $("#legend");
    const cals = res.calendars || [];
    if (res.error) {
      // Tipicamente Outlook si sta ancora avviando: NON si svuota la lista né si
      // dà un errore secco. Si mostrano i calendari già disponibili (Google,
      // festività, locale) e si riprova da soli finché Outlook risponde.
      pianificaRitentativo();
      if (!cals.length) return;   // niente ancora da mostrare, ma il retry è in coda
    }
    // Raggruppa per account preservando l'ordine di Outlook.
    // Raggruppate per AZIENDA mostrata e non per casella: la stessa azienda può
    // avere calendari in posti diversi (per esempio un calendario Google e una
    // cartella di Outlook) e in elenco comparivano due righe con lo stesso nome,
    // di cui una sola comandava le riunioni che vedevi sparire.
    const perAccount = new Map();
    cals.forEach((c) => {
      const azienda = c.alias || c.account;
      if (!perAccount.has(azienda)) {
        perAccount.set(azienda, {
          color: c.color, alias: azienda, caselle: new Set(), tot: 0, on: 0,
          rotto: false, avviso: false, motivi: [],
        });
      }
      const g = perAccount.get(azienda);
      g.caselle.add(c.account);
      g.tot++; if (c.enabled) g.on++;
      // Basta un calendario in errore per marcare la casella: meglio un allarme
      // in più che un rosso mancato. "avviso" è il caso intermedio: i dati ci
      // sono (copia locale) ma l'ultimo aggiornamento non è riuscito.
      if (c.stato === "avviso") {
        g.avviso = true;
        if (c.stato_msg) g.motivi.push(c.stato_msg);
      } else if (c.stato && c.stato !== "ok") {
        g.rotto = true;
        if (c.stato_msg) g.motivi.push(c.stato_msg);
      }
    });

    if (!perAccount.size) {
      legend.innerHTML = `<span class="empty">Nessun calendario. Apri “Gestisci calendari”.</span>`;
    } else {
      // Voce per account, stile Teams: casella colorata (spuntata se visibile)
      // + nome. Un clic accende/spegne tutti i calendari di quella casella.
      const spunta = `<svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 6.5"
        stroke="currentColor" stroke-width="2.4" fill="none" stroke-linecap="round"
        stroke-linejoin="round"/></svg>`;
      legend.innerHTML = [...perAccount].map(([azienda, d]) => {
        const attivo = d.on > 0;
        // Pallino di salute a destra: verde = il collegamento risponde,
        // rosso = quel calendario non si sta leggendo (spiegato nel tooltip).
        const s = _salute(azienda, d);
        const salute = `<span class="acct-health ${s.classe}" title="${escapeHtml(s.titolo)}"></span>`;
        const dettaglio = [...d.caselle].join(" · ");
        return `<button class="acct ${attivo ? "on" : "off"}" data-azienda="${escapeHtml(azienda)}"
                  style="--c:${d.color}" title="${escapeHtml(dettaglio)}">
          <span class="acct-box">${spunta}</span>
          <span class="acct-name">${escapeHtml(d.alias)}</span>
          <span class="acct-only" role="button" title="Mostra solo ${escapeHtml(d.alias)}">only</span>
          ${salute}
        </button>`;
      }).join("");
      legend.querySelectorAll(".acct").forEach((b) => {
        b.addEventListener("click", () => {
          const attivo = b.classList.contains("on");
          api.set_azienda_enabled(b.dataset.azienda, !attivo)
            .then(refreshLegend);
        });
        // "only": resta accesa solo questa azienda, senza spegnere le altre a mano.
        b.querySelector(".acct-only").addEventListener("click", (ev) => {
          ev.stopPropagation();
          api.mostra_solo_azienda(b.dataset.azienda).then(refreshLegend);
        });
      });
    }
    // Al primo giro il calendario sta già caricando da sé: un refetch qui
    // significherebbe interrogare Outlook due volte per gli stessi eventi.
    if (calendar && legendaGiaFatta) calendar.refetchEvents();
    legendaGiaFatta = true;
  });
}

function openCalendarsModal(force) {
  $("#cal-list").textContent = SORGENTE_REMOTA
    ? "Lettura dal server dei calendari…"
    : (force ? "Ricerca completa in Outlook, richiede qualche secondo…"
             : "Lettura da Outlook…");
  apriModale("#modal-calendars");
  api.get_calendars(!!force).then((res) => {
    const box = $("#cal-list");
    if (res.error) {
      box.innerHTML = `<p class="err-msg">${escapeHtml(res.error)}</p>`;
      return;
    }
    const cals = res.calendars || [];
    if (!cals.length) {
      box.innerHTML = SORGENTE_REMOTA
        ? `<p class="hint">Il server dei calendari non ha caselle da mostrare.</p>`
        : `<p class="hint">Nessun calendario trovato nel profilo Outlook.</p>`;
      return;
    }
    // Raggruppati per account. L'intestazione mostra il nome (alias) con un
    // campo per rinominarlo; l'indirizzo vero è la riga piccola sotto.
    let html = "";
    let lastAccount = null;
    cals.forEach((c) => {
      if (c.account !== lastAccount) {
        lastAccount = c.account;
        const alias = c.alias || c.account;
        if (c.is_ics) {
          html += `<div class="cal-group"><span class="dot" style="background:${c.color}"></span>
            <span class="cal-group-name">${escapeHtml(alias)}</span></div>`;
        } else {
          html += `<div class="cal-group">
            <span class="dot" style="background:${c.color}"></span>
            <input class="alias-input" value="${escapeHtml(alias)}" data-account="${escapeHtml(c.account)}"
                   title="Come vuoi chiamare questo account" spellcheck="false" />
            <span class="cal-group-mail" title="${escapeHtml(c.account)}">${escapeHtml(c.account)}</span>
          </div>`;
        }
      }
      const nome = c.label.startsWith(c.account + " · ") ? c.label.slice(c.account.length + 3) : c.label;
      const coda = c.is_ics
        ? `<span class="cal-count">da link</span>
           <span class="cal-rm" data-rm="${escapeHtml(c.id)}" title="Rimuovi questo calendario">✕</span>`
        : c.count == null
          // Voci senza un conteggio (Google, aziende inoltrate): al posto di
          // "null elementi" si dice da dove arrivano.
          ? `<span class="cal-count">${escapeHtml(c.nota || "")}</span>`
          : `<span class="cal-count">${c.count} elementi</span>`;
      html += `<label class="cal-row"><input type="checkbox" data-id="${escapeHtml(c.id)}"
        ${c.enabled ? "checked" : ""} /> ${escapeHtml(nome)} ${coda}</label>`;
    });
    box.innerHTML = html;
    box.querySelectorAll("input[type=checkbox]").forEach((cb) =>
      cb.addEventListener("change", () =>
        api.set_calendar_enabled(cb.dataset.id, cb.checked).then(refreshLegend)));
    box.querySelectorAll("[data-rm]").forEach((el) =>
      el.addEventListener("click", (e) => {
        e.preventDefault();
        api.remove_ics_calendar(el.dataset.rm)
          .then(() => { openCalendarsModal(false); refreshLegend(); });
      }));
    // Il nome si salva quando si esce dal campo o si preme Invio.
    box.querySelectorAll(".alias-input").forEach((inp) => {
      const salva = () =>
        api.set_account_alias(inp.dataset.account, inp.value).then(refreshLegend);
      inp.addEventListener("change", salva);
      inp.addEventListener("keydown", (e) => { if (e.key === "Enter") inp.blur(); });
    });
  });
}

function aggiungiCalendarioDaLink() {
  const nome = $("#ics-name").value.trim();
  const url = $("#ics-url").value.trim();
  const esito = $("#ics-result");
  if (!url) { esito.textContent = "Incolla prima il link."; return; }
  esito.textContent = "Verifico il link…";
  $("#btn-add-ics").disabled = true;
  api.add_ics_calendar(nome, url).then((r) => {
    $("#btn-add-ics").disabled = false;
    if (r.ok) {
      esito.textContent = "✅ Aggiunto: " + r.feed.name;
      $("#ics-name").value = "";
      $("#ics-url").value = "";
      openCalendarsModal(false);
      refreshLegend();
      if (calendar) calendar.refetchEvents();
    } else {
      esito.textContent = "⚠ " + r.error;
    }
  }).catch((e) => {
    $("#btn-add-ics").disabled = false;
    esito.textContent = "⚠ " + e;
  });
}

// Trascinamento del divisore per ridimensionare il pannello laterale.
const SIDEPANE_MIN = 190, SIDEPANE_MAX = 400;
// Ridimensionamento della finestra dai bordi. Serve farlo qui perché WebView2
// copre tutto il client e i bordi non ricevono da soli il messaggio di resize:
// si rileva la vicinanza al bordo, si cambia il cursore e su mousedown si chiede
// a Windows di avviare il resize nativo.
function setupResizeBordi() {
  const B = 6;   // spessore della zona di presa, in px
  const cursori = {
    l: "ew-resize", r: "ew-resize", t: "ns-resize", b: "ns-resize",
    tl: "nwse-resize", br: "nwse-resize", tr: "nesw-resize", bl: "nesw-resize",
  };
  function bordoDa(e) {
    const w = window.innerWidth, h = window.innerHeight;
    const sx = e.clientX <= B, dx = e.clientX >= w - B;
    const su = e.clientY <= B, giu = e.clientY >= h - B;
    if (su && sx) return "tl"; if (su && dx) return "tr";
    if (giu && sx) return "bl"; if (giu && dx) return "br";
    if (sx) return "l"; if (dx) return "r"; if (su) return "t"; if (giu) return "b";
    return null;
  }
  let bordo = null;
  document.addEventListener("mousemove", (e) => {
    if (e.buttons) return;   // durante un trascinamento non si ricalcola
    bordo = bordoDa(e);
    document.body.style.cursor = bordo ? cursori[bordo] : "";
  });
  document.addEventListener("mousedown", (e) => {
    const b = bordoDa(e);
    if (!b) return;
    e.preventDefault();
    e.stopPropagation();   // niente trascinamento-finestra sul bordo
    if (window.pywebview && window.pywebview.api && api.win_resize)
      api.win_resize(b);
  }, true);   // in cattura: prima di easy_drag e degli altri handler
}

function setupResizer() {
  const resizer = $("#resizer");
  const app = document.querySelector(".app");
  const rail = document.querySelector(".rail");
  if (!resizer) return;
  let trascina = false;

  const muovi = (e) => {
    if (!trascina) return;
    const railW = rail ? rail.getBoundingClientRect().right : 0;
    let w = Math.round(e.clientX - railW);
    w = Math.max(SIDEPANE_MIN, Math.min(SIDEPANE_MAX, w));
    app.style.setProperty("--sidepane-w", w + "px");
  };
  const finito = () => {
    if (!trascina) return;
    trascina = false;
    resizer.classList.remove("dragging");
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    if (calendar) calendar.updateSize();
  };
  resizer.addEventListener("mousedown", (e) => {
    e.preventDefault(); e.stopPropagation();   // niente drag della finestra
    trascina = true;
    resizer.classList.add("dragging");
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
  });
  window.addEventListener("mousemove", muovi);
  window.addEventListener("mouseup", finito);
}

// ================= VISTE (Calendario / Timesheet) =================
function mostraVista(nome) {
  const app = document.querySelector(".app");
  app.classList.toggle("view-timesheet", nome === "timesheet");
  $$(".rail-btn").forEach((b) => b.classList.remove("is-active"));
  if (nome === "timesheet") {
    $("#nav-timesheet").classList.add("is-active");
    renderTimesheet();
  } else {
    $("#nav-calendar").classList.add("is-active");
    if (calendar) setTimeout(() => calendar.updateSize(), 50);
  }
}

function renderTimesheet() {
  const grid = $("#ts-grid");
  if (!grid) return;
  // Una scheda con un'"azione" si mostra solo se un'estensione la fornisce.
  const schede = TIMESHEET_AZIENDE.filter((a) => a && a.nome
    && (!a.azione || AZIONI_TIMESHEET[a.azione] || a.url));
  if (!schede.length) {
    grid.innerHTML = `<p class="hint">Nessuna scheda: si aggiungono in
      personalizzazione.json, alla voce "schede_timesheet".</p>`;
    return;
  }
  grid.innerHTML = schede.map((a, i) => {
    const iniz = a.nome.split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase();
    const logo = a.logo
      ? `<span class="ts-logo-box"><img src="${a.logo}" alt="${escapeHtml(a.nome)}"></span>`
      : `<span class="ts-logo-box"><span class="ts-iniz" style="background:${a.colore}">${escapeHtml(iniz)}</span></span>`;
    const sotto = a.sotto ? `<span class="ts-sub">${escapeHtml(a.sotto)}</span>` : "";
    return `<button class="ts-card" data-i="${i}" title="${escapeHtml(a.nome)}">
      ${logo}
      <span class="ts-name">${escapeHtml(a.nome)}${sotto}</span>
    </button>`;
  }).join("");
  grid.querySelectorAll(".ts-card").forEach((c) =>
    c.addEventListener("click", () => {
      const a = schede[+c.dataset.i];
      if (!a) return;
      if (a.azione && AZIONI_TIMESHEET[a.azione]) AZIONI_TIMESHEET[a.azione](a);
      else if (a.url) api.open_external(a.url);
      else showBanner(`Per ${a.nome} manca l'indirizzo del gestionale (vedi personalizzazione.json).`);
    }));
}

// ================= UI / MODALI =================
function setupUI() {
  // Rail
  $("#nav-calendar").addEventListener("click", () => { chiudiModali(); mostraVista("calendar"); });
  // Il sole/luna cambia tema chiaro↔scuro; l'ingranaggio apre le impostazioni.
  $("#nav-theme").addEventListener("click", toggleTema);
  $("#nav-settings").addEventListener("click", openAppSettings);

  // Azioni calendario spostate sotto "I miei calendari".
  $("#mycal-all").addEventListener("click", () => openCalendarsModal(false));
  $("#mycal-tutti").addEventListener("click", () =>
    api.mostra_solo_azienda("").then(refreshLegend));
  $("#open-cal-settings").addEventListener("click", openSettings);

  // ---- Trascinamento finestra e controlli (barra del titolo nostra) ----
  // easy_drag di pywebview trascina la finestra su qualunque mousedown che
  // arriva a window. Si blocca la propagazione sotto la barra e sugli elementi
  // interattivi della barra, così si trascina solo dalle zone vuote in alto.
  ["#app-body", "#search-wrap", "#win-controls", "#sidepane-toggle"].forEach((sel) => {
    const el = $(sel);
    if (el) el.addEventListener("mousedown", (e) => e.stopPropagation());
  });
  // Doppio clic sulla barra = ingrandisci/ripristina, come in Teams.
  $(".topbar").addEventListener("dblclick", () => api.win_maximize_toggle());
  $("#win-min").addEventListener("click", () => api.win_minimize());
  $("#win-max").addEventListener("click", () => api.win_maximize_toggle());
  $("#win-close").addEventListener("click", () => api.win_close());
  // Tornando in primo piano (es. dopo un ripristino da icona) la finestra senza
  // cornice può restare bianca: un micro-ridimensionamento la fa ridisegnare.
  window.addEventListener("focus", () => {
    if (window.pywebview && window.pywebview.api && api.win_nudge)
      api.win_nudge();
  });

  setupResizeBordi();

  // Mostra/nascondi il pannello con il mini-calendario. Il calendario deve
  // riprendersi lo spazio subito, non al prossimo aggiornamento.
  $("#sidepane-toggle").addEventListener("click", () => {
    document.querySelector(".app").classList.toggle("sidepane-hidden");
    if (calendar) { calendar.updateSize(); setTimeout(() => calendar.updateSize(), 60); }
  });

  // Espandi/collassa il rail (icone <-> icone+etichette), come in Teams.
  $("#rail-expand").addEventListener("click", () => {
    document.querySelector(".app").classList.toggle("rail-expanded");
    if (calendar) setTimeout(() => calendar.updateSize(), 180);
  });

  // Timesheet: sezione dentro l'app con le aziende.
  $("#nav-timesheet").addEventListener("click", () => { chiudiModali(); mostraVista("timesheet"); });

  // Divisore: regola la larghezza del pannello trascinando col mouse.
  setupResizer();

  // Mini-calendario e pannello account
  $("#minical-prev").addEventListener("click", () => minicalCambiaMese(-1));
  $("#minical-next").addEventListener("click", () => minicalCambiaMese(1));
  $("#btn-new-event").addEventListener("click", openEventModal);

  // Ricerca
  $("#search").addEventListener("input", pianificaRicerca);
  $("#search-clear").addEventListener("click", () => {
    $("#search").value = "";
    applicaRicerca();
    $("#search").focus();
  });

  // Barra comandi
  $("#btn-today").addEventListener("click", () => calendar && calendar.today());
  $("#btn-prev").addEventListener("click", () => calendar && calendar.prev());
  $("#btn-next").addEventListener("click", () => calendar && calendar.next());
  $("#btn-sync").addEventListener("click", forzaSync);
  $("#btn-zoom-in").addEventListener("click", () => cambiaZoom(+1));
  $("#btn-zoom-out").addEventListener("click", () => cambiaZoom(-1));
  $("#btn-zoom-reset").addEventListener("click", () => applicaZoom(ZOOM_DEFAULT, true));
  $$("#view-switch button").forEach((b) =>
    b.addEventListener("click", () => calendar && calendar.changeView(b.dataset.view)));

  // Riquadri
  $("#btn-rescan").addEventListener("click", () => openCalendarsModal(true));
  $("#btn-add-ics").addEventListener("click", aggiungiCalendarioDaLink);
  $("#btn-save-settings").addEventListener("click", saveSettings);
  $("#btn-save-event").addEventListener("click", salvaEvento);
  $("#btn-test-notify").addEventListener("click", testNotify);
  $$(".close-modal").forEach((b) => b.addEventListener("click", chiudiModali));
  $$(".modal").forEach((m) =>
    m.addEventListener("click", (e) => { if (e.target === m) chiudiModali(); }));
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") chiudiModali(); });
}

function apriModale(sel, railSel) {
  $(sel).classList.remove("hidden");
  if (railSel && $(railSel)) {
    $$(".rail-btn").forEach((b) => b.classList.remove("is-active"));
    $(railSel).classList.add("is-active");
  }
}

function chiudiModali() {
  // Tutte le finestre modali, comprese quelle aggiunte dalle estensioni.
  $$(".modal").forEach((m) => m.classList.add("hidden"));
  // L'elemento attivo del rail segue la vista corrente, non sempre Calendario.
  const timesheet = document.querySelector(".app").classList.contains("view-timesheet");
  $$(".rail-btn").forEach((b) => b.classList.remove("is-active"));
  $(timesheet ? "#nav-timesheet" : "#nav-calendar").classList.add("is-active");
}

// ---- Nuovo evento (calendario "Altro") --------------------------------
function perDatetimeLocal(d) {
  // datetime-local vuole "YYYY-MM-DDTHH:MM" in ora locale.
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

function openEventModal() {
  // Preimposta sull'ora tonda successiva, durata un'ora.
  const ora = new Date();
  ora.setMinutes(0, 0, 0); ora.setHours(ora.getHours() + 1);
  const fine = new Date(ora.getTime() + 60 * 60 * 1000);
  $("#ev-title").value = "";
  $("#ev-location").value = "";
  $("#ev-notes").value = "";
  $("#ev-allday").checked = false;
  $("#ev-start").value = perDatetimeLocal(ora);
  $("#ev-end").value = perDatetimeLocal(fine);
  $("#ev-result").textContent = "";
  $("#modal-event").classList.remove("hidden");
  setTimeout(() => $("#ev-title").focus(), 50);
}

function salvaEvento() {
  const dati = {
    title: $("#ev-title").value,
    start: $("#ev-start").value,
    end: $("#ev-end").value,
    allDay: $("#ev-allday").checked,
    location: $("#ev-location").value,
    notes: $("#ev-notes").value,
  };
  $("#ev-result").textContent = "Salvo…";
  api.add_local_event(dati).then((r) => {
    if (r.ok) {
      chiudiModali();
      refreshLegend();
      if (calendar) calendar.refetchEvents();
    } else {
      $("#ev-result").textContent = "⚠ " + r.error;
    }
  });
}

function openSettings() {
  api.get_config().then((c) => {
    $("#cfg-minutes").value = (c.reminder_minutes_before || []).join(", ");
    $("#cfg-toast").checked = !!c.windows_toast;
    const tg = c.telegram || {};
    $("#cfg-tg-enabled").checked = !!tg.enabled;
    $("#cfg-tg-token").value = tg.bot_token || "";
    $("#cfg-tg-chat").value = tg.chat_id || "";
    $("#test-result").textContent = "";
    apriModale("#modal-settings");
  });
}

// ---- Impostazioni dell'app (tema, avvio, sincronizzazione) -------------
let temaCorrente = "auto";

function applicaTema(tema) {
  temaCorrente = tema || "auto";
  const root = document.documentElement;
  if (temaCorrente === "auto") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", temaCorrente);
  $$("#cfg-theme button").forEach((b) =>
    b.classList.toggle("is-active", b.dataset.theme === temaCorrente));
  aggiornaIconaTema();
}

// Il pulsante sole/luna: mostra lo stato attuale e al clic passa chiaro↔scuro.
const _SVG_SOLE = `<svg viewBox="0 0 24 24" width="21" height="21" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
  <circle cx="12" cy="12" r="4"/>
  <path d="M12 2.8v1.9M12 19.3v1.9M21.2 12h-1.9M4.7 12H2.8M18.5 5.5l-1.3 1.3M6.8 17.2l-1.3 1.3M18.5 18.5l-1.3-1.3M6.8 6.8L5.5 5.5"/></svg>`;
const _SVG_LUNA = `<svg viewBox="0 0 24 24" width="21" height="21" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round">
  <path d="M20.5 14.2a8.3 8.3 0 01-10.7-10 8.3 8.3 0 1010.7 10z"/></svg>`;

function _temaScuroAttivo() {
  if (temaCorrente === "dark") return true;
  if (temaCorrente === "light") return false;
  return window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
}
function aggiornaIconaTema() {
  const ico = $("#theme-ico");
  if (ico) ico.innerHTML = _temaScuroAttivo() ? _SVG_SOLE : _SVG_LUNA;
}
function toggleTema() {
  const nuovo = _temaScuroAttivo() ? "light" : "dark";
  applicaTema(nuovo);
  api.save_config({ theme: nuovo });
}

// Stato dei calendari collegati direttamente (i calendari Google). Serve a
// capire a colpo d'occhio se l'integrazione regge o se il token va rifatto:
// senza, un'autorizzazione scaduta si manifesterebbe solo con eventi mancanti.
const ETICHETTE_STATO = {
  ok: { testo: "Collegato", classe: "st-ok" },
  problema: { testo: "Problema temporaneo", classe: "st-warn" },
  riautorizzare: { testo: "Da riautorizzare", classe: "st-bad" },
  assente: { testo: "Non configurato", classe: "st-bad" },
};

function mostraIntegrazioni(verifica) {
  const box = $("#integrazioni-list");
  const aiuto = $("#integr-aiuto");
  box.innerHTML = `<div class="hint">${verifica ? "Verifico…" : "Controllo…"}</div>`;
  aiuto.textContent = "";
  return api.get_integrazioni(!!verifica).then((res) => {
    const voci = (res && res.integrazioni) || [];
    if (!voci.length) { box.innerHTML = `<div class="hint">Nessuna integrazione.</div>`; return; }
    box.innerHTML = voci.map((v) => {
      const e = ETICHETTE_STATO[v.stato] || ETICHETTE_STATO.problema;
      const dettagli = [];
      if (v.ultimo_ok) dettagli.push("ultimo aggiornamento " + v.ultimo_ok.replace("T", " ").slice(11, 16));
      if (v.stato === "ok" && v.scade) dettagli.push("token valido fino alle " + v.scade);
      return `<div class="integr-row">
        <span class="integr-dot ${e.classe}"></span>
        <span class="integr-name">${escapeHtml(v.nome)}<small>${escapeHtml(v.tipo)} · ${escapeHtml(v.indirizzo)}</small></span>
        <span class="integr-state ${e.classe}">${e.testo}<small>${escapeHtml(dettagli.join(" · ") || v.messaggio || "")}</small></span>
      </div>`;
    }).join("");
    // Se serve l'intervento, si dice esattamente cosa fare: il token si rinnova
    // dall'add-in che l'aveva ottenuto, non da qui.
    if (voci.some((v) => v.stato === "riautorizzare" || v.stato === "assente")) {
      aiuto.innerHTML = "Per rimettere a posto: apri <b>Outlook → scheda CalDav Synchronizer → "
        + "Synchronization Profiles</b>, togli la spunta <i>Inactive</i> al profilo "
        + "di quel calendario Google, premi <b>Test settings</b> e completa l'accesso. "
        + "Poi torna qui e premi «Verifica adesso».";
    }
  }).catch((e) => { box.innerHTML = `<div class="hint">Stato non leggibile: ${escapeHtml(String(e))}</div>`; });
}

function openAppSettings() {
  chiudiModali();
  api.get_config().then((c) => {
    applicaTema(c.theme || "auto");
    $("#cfg-sync-interval").value = String(c.reminder_poll_seconds || 60);
    $("#cfg-start-minimized").checked = !!c.start_minimized;
    $("#appsettings-result").textContent = "";
    // Lo stato dell'avvio automatico lo dà il sistema (chiave di registro).
    api.get_autostart().then((r) => {
      $("#cfg-autostart").checked = !!(r && r.enabled);
    });
    mostraIntegrazioni(false);
    apriModale("#modal-appsettings", "#nav-settings");
  });
}

function setupAppSettings() {
  $$("#cfg-theme button").forEach((b) =>
    b.addEventListener("click", () => {
      applicaTema(b.dataset.theme);
      api.save_config({ theme: temaCorrente });
    }));
  $("#cfg-sync-interval").addEventListener("change", (e) => {
    const v = parseInt(e.target.value, 10) || 60;
    intervalloSync = v * 1000;
    document.dispatchEvent(new Event("sync-interval-changed"));
    api.save_config({ reminder_poll_seconds: v });
  });
  $("#cfg-start-minimized").addEventListener("change", (e) =>
    api.save_config({ start_minimized: e.target.checked }));
  $("#btn-verifica-integr").addEventListener("click", () => mostraIntegrazioni(true));
  $("#cfg-autostart").addEventListener("change", (e) => {
    $("#appsettings-result").textContent = "Applico…";
    api.set_autostart(e.target.checked).then((r) => {
      $("#appsettings-result").textContent = (r && r.ok) ? "Fatto" : ("⚠ " + (r && r.error || ""));
    });
  });
}

function saveSettings() {
  const minutes = $("#cfg-minutes").value.split(",")
    .map((s) => parseInt(s.trim(), 10)).filter((n) => !isNaN(n) && n >= 0);
  const newCfg = {
    reminder_minutes_before: minutes.length ? minutes : [10],
    windows_toast: $("#cfg-toast").checked,
    telegram: {
      enabled: $("#cfg-tg-enabled").checked,
      bot_token: $("#cfg-tg-token").value.trim(),
      chat_id: $("#cfg-tg-chat").value.trim(),
    },
  };
  api.save_config(newCfg).then(() => {
    chiudiModali();
    refreshLegend();
  });
}

function testNotify() {
  $("#test-result").textContent = "Invio…";
  // salva prima, così usa i valori correnti
  saveSettingsSilent().then(() =>
    api.test_notification().then((r) => {
      $("#test-result").textContent = r.message;
    }));
}
function saveSettingsSilent() {
  const newCfg = {
    windows_toast: $("#cfg-toast").checked,
    telegram: {
      enabled: $("#cfg-tg-enabled").checked,
      bot_token: $("#cfg-tg-token").value.trim(),
      chat_id: $("#cfg-tg-chat").value.trim(),
    },
  };
  return api.save_config(newCfg);
}

// ================= RICERCA =================
// Filtra gli eventi già in pagina: nessuna nuova interrogazione a Outlook,
// così la lista si restringe mentre si scrive invece che dopo due secondi.
let timerRicerca = null;
let filtroInCorso = false;   // setProp() riscatena eventsSet: senza guardia si cicla

function applicaRicerca() {
  if (!calendar || filtroInCorso) return;
  filtroInCorso = true;
  try {
    _applicaRicerca();
  } finally {
    filtroInCorso = false;
  }
}

function _applicaRicerca() {
  const q = $("#search").value.trim().toLowerCase();
  $("#search-clear").classList.toggle("hidden", !q);
  let trovati = 0;
  calendar.getEvents().forEach((ev) => {
    if (!q) {
      ev.setProp("display", "auto");
      return;
    }
    const p = ev.extendedProps || {};
    const testo = [ev.title, p.alias, p.account, p.calendar, p.location, p.organizer]
      .filter(Boolean).join(" ").toLowerCase();
    const ok = testo.includes(q);
    if (ok) trovati++;
    ev.setProp("display", ok ? "auto" : "none");
  });
  if (q && trovati === 0) {
    showBanner(`Nessuna riunione trovata per “${$("#search").value.trim()}”.`, "ricerca");
  } else {
    hideBanner("ricerca");
  }
}

function pianificaRicerca() {
  clearTimeout(timerRicerca);
  timerRicerca = setTimeout(applicaRicerca, 160);
}

// ================= AVVIO DI OUTLOOK IN SOTTOFONDO =================
function pianificaRitentativo() {
  if (timerRitentativo || tentativiOutlook >= MAX_TENTATIVI) return;
  tentativiOutlook++;
  timerRitentativo = setTimeout(() => {
    timerRitentativo = null;
    // Quando Outlook diventa pronto vanno recuperati ENTRAMBI: la lista
    // calendari (sidebar) e gli eventi (griglia). Prima si ricaricava solo la
    // griglia, e la sidebar restava vuota per sempre.
    refreshLegend();
    if (calendar) calendar.refetchEvents();
  }, 10000);
}

function mostraDatiVecchi(quando, motivo) {
  const el = $("#stale");
  const ora = quando ? quando.slice(11, 16) : null;
  // "motivo" lo dà chi legge le caselle quando non è Outlook a mancare.
  const cosa = motivo || "Outlook si sta avviando";
  el.textContent = ora
    ? `${cosa} — questi sono i dati salvati delle ${ora}. Aggiorno da solo…`
    : `${cosa} — aggiorno da solo appena è pronto…`;
  el.classList.remove("hidden");
}

function nascondiDatiVecchi() { $("#stale").classList.add("hidden"); }

// ================= BANNER + UTILITÀ =================
// Il banner è condiviso fra avvisi diversi: senza distinguerli, il messaggio
// "nessun risultato" della ricerca (che si ripulisce a ogni render) cancellava
// gli errori di caricamento un istante dopo averli mostrati.
let bannerTipo = null;

function showBanner(msg, tipo = "errore") {
  const b = $("#banner");
  b.textContent = msg;
  b.classList.toggle("banner-errore", tipo === "errore");
  b.classList.remove("hidden");
  bannerTipo = tipo;
}

function hideBanner(tipo = null) {
  // Con un tipo si nasconde solo il proprio avviso; senza, si pulisce tutto
  // (lo fa il caricamento, che poi rimette gli errori eventuali).
  if (tipo && bannerTipo && bannerTipo !== tipo) return;
  $("#banner").classList.add("hidden");
  bannerTipo = null;
}

function capitalizza(s) {
  return s ? s.charAt(0).toUpperCase() + s.slice(1) : s;
}

/** Iniziali per il cerchietto: "Alfa" -> "AL", "Beta Servizi" -> "BS",
 *  "Alfa · m.rossi" -> "AL" (conta solo la parte prima del punto medio). */
function iniziali(nome) {
  let s = String(nome || "").split("·")[0].trim();   // scarta il distinguo dopo il ·
  s = s.split("@")[0];                                // se è un indirizzo, la parte locale
  const parole = s.split(/[\s._\-+]/).filter(Boolean);
  if (parole.length >= 2) return (parole[0][0] + parole[1][0]).toUpperCase();
  return s.slice(0, 2).toUpperCase();
}

/** Colore di riempimento tenue a partire dal colore pieno dell'account. */
function tinta(hex, alpha) {
  const m = /^#?([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(hex || "");
  if (!m) return hex;
  const [r, g, b] = [1, 2, 3].map((i) => parseInt(m[i], 16));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function fmtRange(ev) {
  const opt = { hour: "2-digit", minute: "2-digit" };
  const day = ev.start
    ? ev.start.toLocaleDateString("it-IT", { weekday: "long", day: "numeric", month: "long" })
    : "";
  if (ev.allDay) return `${capitalizza(day)} · tutto il giorno`;
  const s = ev.start ? ev.start.toLocaleTimeString("it-IT", opt) : "";
  const e = ev.end ? ev.end.toLocaleTimeString("it-IT", opt) : "";
  return `${capitalizza(day)} · ${s}–${e}`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

