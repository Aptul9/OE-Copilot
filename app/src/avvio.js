// Pagina di avvio inclusa nell'app: senza permessi sui comandi tranne uno (imposta_indirizzo), controlla che il sito risponda e poi ci porta la finestra.
// L'indirizzo lo mette l'app (window.__OE_ORIGINE__), non sta in questo file; si cambia dal campo di questa pagina.
(function () {
  "use strict";

  const ATTESA_MS = 6000;
  const NUOVO_TENTATIVO_MS = 5000;
  const SEGNAPOSTO = "oe.example.com";

  const origine = window.__OE_ORIGINE__;
  // #senza-copia: la finestra è tornata qui perché il sito, senza rete, non aveva una copia da mostrare (inizializzazione.js)
  const senzaCopia = location.hash === "#senza-copia";
  const nodo = (id) => document.getElementById(id);
  const rotella = nodo("rotella");
  const stato = nodo("stato");
  const dettaglio = nodo("dettaglio");
  const azioni = nodo("azioni");
  const riprova = nodo("riprova");
  const copia = nodo("copia");
  const cambia = nodo("cambia");
  const modulo = nodo("modulo");
  const campo = nodo("indirizzo");
  const errore = nodo("errore");
  const salva = nodo("salva");
  const annulla = nodo("annulla");
  const esci = nodo("esci");
  let timer = null;
  let inCorso = false;

  function mostra(titolo, testo, problema) {
    stato.textContent = titolo;
    dettaglio.textContent = testo;
    azioni.hidden = !problema;
    riprova.hidden = !problema;
    esci.hidden = !problema;
    rotella.hidden = problema;
  }

  function nonConfigurato() {
    try {
      return !origine || new URL(origine).hostname === SEGNAPOSTO;
    } catch {
      return true;
    }
  }

  function apriSito() {
    location.replace(origine + "/");
  }

  async function prova(inSilenzio) {
    if (inCorso) return;
    inCorso = true;
    clearTimeout(timer);
    const host = new URL(origine).host;
    if (!inSilenzio) mostra("Connessione…", host, false);
    try {
      // risposta opaca (no-cors): basta sapere che il server risponde, la pagina vera la carica poi la finestra
      await fetch(origine + "/login", { mode: "no-cors", cache: "no-store", signal: AbortSignal.timeout(ATTESA_MS) });
    } catch {
      // senza rete il sito si apre lo stesso se ha una copia della pagina (il suo service worker); se non l'ha si torna qui
      if (!senzaCopia && navigator.onLine === false) {
        apriSito();
        return;
      }
      // con la rete ma senza risposta (un server spostato, un indirizzo che non c'è più) si sceglie: aspettare, la copia, un altro server
      mostra("Server non raggiungibile.", `${host}. Riprovo da solo ogni ${NUOVO_TENTATIVO_MS / 1000} secondi.`, true);
      copia.hidden = senzaCopia;
      timer = setTimeout(() => prova(true), NUOVO_TENTATIVO_MS);
      inCorso = false;
      return;
    }
    apriSito();
  }

  // il campo dell'indirizzo del sito: ferma i tentativi, che non devono cambiare la pagina mentre si scrive
  function apriModulo() {
    clearTimeout(timer);
    mostra("Cambia server", nonConfigurato() ? "Questa app non ha ancora un indirizzo." : `Adesso: ${new URL(origine).host}`, false);
    rotella.hidden = true;
    errore.textContent = "";
    campo.removeAttribute("aria-invalid");
    salva.disabled = false;
    modulo.hidden = false;
    esci.hidden = false;
    campo.value = nonConfigurato() ? "" : origine;
    campo.focus();
    campo.select();
  }

  function chiudiModulo() {
    modulo.hidden = true;
    if (nonConfigurato()) return apriModulo();
    inCorso = false;
    prova(false);
  }

  async function invia(evento) {
    evento.preventDefault();
    errore.textContent = "";
    campo.removeAttribute("aria-invalid");
    const invoca = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (!invoca) {
      errore.textContent = "Questa versione dell'app non permette di cambiare server.";
      return;
    }
    salva.disabled = true;
    try {
      const riavvia = await invoca("imposta_indirizzo", { indirizzo: campo.value });
      modulo.hidden = true;
      mostra(riavvia ? "Indirizzo salvato. Riavvio…" : "Indirizzo salvato. L'app si chiude: riaprila.", "", false);
      rotella.hidden = !riavvia;
    } catch (causa) {
      errore.textContent = typeof causa === "string" ? causa : "Indirizzo non salvato.";
      campo.setAttribute("aria-invalid", "true");
      salva.disabled = false;
      campo.focus();
    }
  }

  // cancella la sessione e i dati del sito nella finestra, anche se il sito non risponde: poi la finestra torna a questa pagina
  async function esciDallAccount() {
    const invoca = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (!invoca) {
      mostra("Uscita non riuscita.", "Questa versione dell'app non permette di uscire da qui.", true);
      return;
    }
    esci.disabled = true;
    try {
      await invoca("esci_account");
    } catch (causa) {
      esci.disabled = false;
      mostra("Uscita non riuscita.", typeof causa === "string" ? causa : "", true);
    }
  }

  riprova.addEventListener("click", () => {
    inCorso = false;
    prova(false);
  });
  copia.addEventListener("click", apriSito);
  cambia.addEventListener("click", apriModulo);
  esci.addEventListener("click", esciDallAccount);
  annulla.addEventListener("click", chiudiModulo);
  modulo.addEventListener("submit", invia);
  window.addEventListener("hashchange", () => {
    if (location.hash === "#server") apriModulo();
  });
  window.addEventListener("online", () => prova(true));

  if (location.hash === "#server" || nonConfigurato()) apriModulo();
  else prova(false);
})();
