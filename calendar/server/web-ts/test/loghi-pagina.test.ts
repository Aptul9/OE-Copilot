import { describe, expect, it } from "vitest";
import { dominioScritto, iconaDelSito, indirizzoDaTesto, LOGO_EVENTI_PERSONALI, mappaLoghi, urlLogo } from "@/lib/oe/logic";

// Come la pagina trasforma i loghi che riceve da get_personalizzazione in quelli che mostra: la stessa funzione per la barra, gli eventi
// e la finestra Calendari.

const BARRA = "\\";
const PAGINA = "https://app.example.test/";

describe("urlLogo", () => {
  it("un file del server prende la barra davanti, un percorso resta com'è", () => {
    expect(urlLogo("logos/alfa.png")).toBe("/logos/alfa.png");
    expect(urlLogo("logos/url-0123456789abcdef.png?v=1700000000000")).toBe("/logos/url-0123456789abcdef.png?v=1700000000000");
    expect(urlLogo("/logos/alfa.png")).toBe("/logos/alfa.png");
    expect(urlLogo("  logos/alfa.png  ")).toBe("/logos/alfa.png");
  });

  // quello che il browser carica da un altro sito la pagina non lo mette mai in un <img>: ci pensa il gateway, che lo scarica e lo serve da qui
  const FUORI = [
    "https://esempio.example/logo.png",
    "HTTP://esempio.example/logo.png",
    "//esempio.example/logo.png",
    "///esempio.example/logo.png",
    `/${BARRA}esempio.example/logo.png`,
    `${BARRA}${BARRA}esempio.example${BARRA}logo.png`,
    `${BARRA}/esempio.example/logo.png`,
    `/${BARRA}/esempio.example/logo.png`,
    `https:${BARRA}${BARRA}esempio.example${BARRA}logo.png`,
    "/\t/esempio.example/logo.png",
    "/\n/esempio.example/logo.png",
    "\t//esempio.example/logo.png",
    "\u0000 //esempio.example/logo.png",
    " \r\n//esempio.example/logo.png",
    "ht\ttps://esempio.example/logo.png",
    "data:image/png;base64,AAAA",
    "javascript:alert(1)",
    "ftp://esempio.example/logo.png",
    "blob:https://esempio.example/0f5a",
    "file:///etc/passwd",
    "C:" + BARRA + "loghi" + BARRA + "a.png",
    "https:esempio.example/logo.png",
  ];
  it.each(FUORI)("%j non si carica da un altro sito: nessun indirizzo", (valore) => {
    expect(urlLogo(valore)).toBe("");
  });

  it.each([...FUORI, "logos/alfa.png", "/logos/alfa.png", "alfa.png", "../alfa.png", "./alfa.png", "logos/a b.png", "logos/a" + BARRA + "b.png", "/ /esempio.example/x.png", ".//esempio.example/x.png", "%2f%2fesempio.example/x.png", "/%5cesempio.example/x.png"])(
    "il browser, davanti a %j, non va mai su un altro sito",
    (valore) => {
      const url = urlLogo(valore);
      if (!url) return;
      // l'indirizzo che la pagina darebbe a <img src>, risolto come lo risolve il browser
      expect(new URL(url, PAGINA).origin).toBe(new URL(PAGINA).origin);
    },
  );

  it("una barra rovesciata dopo ? o # non è un percorso: resta", () => {
    expect(urlLogo(`logos/url-0123456789abcdef.png?v=1${BARRA}2`)).toBe(`/logos/url-0123456789abcdef.png?v=1${BARRA}2`);
  });
});

describe("un dominio scritto da solo", () => {
  it.each([["esempio.org", "esempio.org"], ["  WWW.Esempio.IT ", "www.esempio.it"], ["sub.esempio.co.uk", "sub.esempio.co.uk"], ["esempio.org:443", "esempio.org:443"]])(
    "%j offre l'icona del sito",
    (testo, dominio) => {
      expect(dominioScritto(testo)).toBe(dominio);
      expect(iconaDelSito(dominio)).toBe(`https://${dominio}/favicon.ico`);
    },
  );

  it.each(["", "esempio", "https://esempio.org", "esempio.org/logo.png", "esempio.org?x=1", "a b.org", "http://127.0.0.1", "esempio.org.", ".org"])("%j non è un dominio da solo", (testo) => {
    expect(dominioScritto(testo)).toBeNull();
  });

  it("un dominio con un percorso prende https:// davanti; il resto va com'è scritto (se lo rifiuta, è il server)", () => {
    expect(indirizzoDaTesto("esempio.org/logo.png")).toBe("https://esempio.org/logo.png");
    expect(indirizzoDaTesto(" www.esempio.org/a?b=1 ")).toBe("https://www.esempio.org/a?b=1");
    expect(indirizzoDaTesto("https://esempio.org/logo.png")).toBe("https://esempio.org/logo.png");
    expect(indirizzoDaTesto("http://esempio.org/logo.png")).toBe("http://esempio.org/logo.png");
    expect(indirizzoDaTesto("esempio.org")).toBe("esempio.org");
    expect(indirizzoDaTesto("  ")).toBe("");
    expect(indirizzoDaTesto("ftp://esempio.org/x")).toBe("ftp://esempio.org/x");
  });
});

describe("mappaLoghi", () => {
  it("senza loghi dichiarati c'è solo quello degli Eventi personali, che sta nell'immagine del gateway", () => {
    expect(mappaLoghi(undefined)).toEqual({ "Eventi personali": LOGO_EVENTI_PERSONALI });
    expect(mappaLoghi({})).toEqual({ "Eventi personali": "/logos/personale.png" });
  });

  it("i loghi dichiarati (personalizzazione.json e scelte della finestra) si aggiungono, con l'indirizzo per il browser", () => {
    expect(mappaLoghi({ Alfa: "logos/alfa.png", Beta: "logos/url-0123456789abcdef.svg?v=5" })).toEqual({
      "Eventi personali": "/logos/personale.png",
      Alfa: "/logos/alfa.png",
      Beta: "/logos/url-0123456789abcdef.svg?v=5",
    });
  });

  it("un logo dichiarato per gli Eventi personali vince su quello predefinito", () => {
    expect(mappaLoghi({ "Eventi personali": "logos/mio.png" })["Eventi personali"]).toBe("/logos/mio.png");
  });

  it("un valore che porta a un altro sito, scritto in qualunque modo, non entra nella mappa: la pagina non carica niente da lì", () => {
    const fuori = {
      Alfa: "https://esempio.example/a.png",
      Beta: `/${BARRA}esempio.example/b.png`,
      Gamma: `${BARRA}${BARRA}esempio.example${BARRA}c.png`,
      Delta: "//esempio.example/d.png",
      Epsilon: "/\t/esempio.example/e.png",
      Zeta: "data:image/png;base64,AAAA",
      Eta: "logos/h.png",
    };
    const mappa = mappaLoghi(fuori);
    expect(mappa).toEqual({ "Eventi personali": "/logos/personale.png", Eta: "/logos/h.png" });
    for (const url of Object.values(mappa)) expect(new URL(url, PAGINA).origin).toBe(new URL(PAGINA).origin);
  });

  it("una voce vuota o che non è un testo non rompe niente: si salta", () => {
    expect(mappaLoghi({ Alfa: "", Beta: null as unknown as string, Gamma: 5 as unknown as string, Delta: "logos/d.png" })).toEqual({
      "Eventi personali": "/logos/personale.png",
      Delta: "/logos/d.png",
    });
  });
});
