import { describe, expect, it } from "vitest";
import { dominioScritto, iconaDelSito, indirizzoDaTesto, LOGO_EVENTI_PERSONALI, mappaLoghi, urlLogo } from "@/lib/oe/logic";

// Come la pagina trasforma i loghi che riceve da get_personalizzazione in quelli che mostra: la stessa funzione per la barra, gli eventi
// e la finestra Calendari.

describe("urlLogo", () => {
  it("un file del server prende la barra davanti; un indirizzo intero o un percorso restano come sono", () => {
    expect(urlLogo("logos/alfa.png")).toBe("/logos/alfa.png");
    expect(urlLogo("logos/url-0123456789abcdef.png?v=1700000000000")).toBe("/logos/url-0123456789abcdef.png?v=1700000000000");
    expect(urlLogo("/logos/alfa.png")).toBe("/logos/alfa.png");
    expect(urlLogo("https://esempio.example/logo.png")).toBe("https://esempio.example/logo.png");
    expect(urlLogo("//esempio.example/logo.png")).toBe("//esempio.example/logo.png");
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

  it("una voce vuota o che non è un testo non rompe niente: si salta", () => {
    expect(mappaLoghi({ Alfa: "", Beta: null as unknown as string, Gamma: 5 as unknown as string, Delta: "logos/d.png" })).toEqual({
      "Eventi personali": "/logos/personale.png",
      Delta: "/logos/d.png",
    });
  });
});
