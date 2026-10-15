// Pagina di avvio inclusa nell'app: senza permessi sui comandi, controlla che il sito risponda e poi ci porta la finestra.
// L'indirizzo lo mette l'app (window.__OE_ORIGINE__), non sta in questo file.
(function () {
  "use strict";

  const ATTESA_MS = 6000;
  const NUOVO_TENTATIVO_MS = 5000;
  const SEGNAPOSTO = "oe.example.com";

  const origine = window.__OE_ORIGINE__;
  const rotella = document.getElementById("rotella");
  const stato = document.getElementById("stato");
  const dettaglio = document.getElementById("dettaglio");
  const riprova = document.getElementById("riprova");
  let timer = null;
  let inCorso = false;

  function mostra(titolo, testo, errore) {
    stato.textContent = titolo;
    dettaglio.textContent = testo;
    riprova.hidden = !errore;
    rotella.hidden = errore;
  }

  function nonConfigurato() {
    try {
      return !origine || new URL(origine).hostname === SEGNAPOSTO;
    } catch {
      return true;
    }
  }

  async function prova() {
    if (inCorso) return;
    inCorso = true;
    clearTimeout(timer);
    const host = new URL(origine).host;
    mostra("Connessione…", host, false);
    try {
      // risposta opaca (no-cors): basta sapere che il server risponde, la pagina vera la carica poi la finestra
      await fetch(origine + "/login", { mode: "no-cors", cache: "no-store", signal: AbortSignal.timeout(ATTESA_MS) });
    } catch {
      mostra("Server non raggiungibile.", `${host}. Riprovo tra ${NUOVO_TENTATIVO_MS / 1000} secondi.`, true);
      timer = setTimeout(prova, NUOVO_TENTATIVO_MS);
      inCorso = false;
      return;
    }
    location.replace(origine + "/");
  }

  if (nonConfigurato()) {
    mostra("Indirizzo del server mancante.", "Ricostruisci l'app con OE_URL.", true);
    riprova.hidden = true;
    rotella.hidden = true;
    return;
  }
  riprova.addEventListener("click", prova);
  window.addEventListener("online", prova);
  prova();
})();
