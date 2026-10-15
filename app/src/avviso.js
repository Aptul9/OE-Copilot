// La pagina dell'avviso a comparsa: una riga per riunione sotto la testata con la X. Le righe le mette l'app, già ripulite in comparsa.rs:
// all'apertura in window.__OE_AVVISI__, dopo con window.__oeMostra. Non ha comandi nativi: un clic sull'avviso chiede /apri (la finestra
// principale davanti), la X /chiudi, e l'app intercetta la navigazione. In tutti e due i casi l'avviso si chiude.
(function () {
  "use strict";

  const vai = (percorso) => location.replace(percorso);
  // le righe già mostrate: solo quelle nuove entrano con l'animazione
  const viste = new Set();

  function elemento(tag, classe, testo) {
    const e = document.createElement(tag);
    if (classe) e.className = classe;
    if (testo) e.textContent = testo;
    return e;
  }

  function riga(a) {
    const r = elemento("div", viste.has(a.id) ? "riga" : "riga nuova");
    if (a.colore) r.style.setProperty("--colore", a.colore);
    // la sigla nel colore dell'azienda, come nel calendario; il logo, se si carica, prende il suo posto
    const icona = elemento("div", "icona");
    const sigla = elemento("span", "sigla", a.sigla);
    icona.append(sigla);
    if (a.logo) {
      const logo = elemento("img", "logo");
      logo.alt = "";
      logo.hidden = true;
      logo.onload = () => {
        logo.hidden = false;
        sigla.hidden = true;
      };
      logo.src = a.logo;
      icona.append(logo);
    }
    r.append(icona, elemento("span", "ora", a.ora));
    if (a.titolo) {
      const titolo = elemento("span", "titolo", a.titolo);
      titolo.title = a.titolo;
      r.append(titolo);
    }
    return r;
  }

  window.__oeMostra = (righe) => {
    document.getElementById("righe").replaceChildren(...righe.map(riga));
    for (const a of righe) viste.add(a.id);
  };

  document.getElementById("chiudi").addEventListener("click", (e) => {
    e.stopPropagation();
    vai("/chiudi");
  });
  document.body.addEventListener("click", () => vai("/apri"));
  window.__oeMostra(window.__OE_AVVISI__ || []);
})();
