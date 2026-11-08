import { describe, expect, it } from "vitest";
import { aggiungiGruppo, eliminaGruppo, gruppiAttivi, impostaAppartenenza, leggiGruppi, rinominaGruppo, type GruppoAzienda } from "@/lib/oe/logic";
import type { GruppiCalendari } from "@/lib/oe/types";

const az = (azienda: string, on: number, tot = 1): GruppoAzienda => ({ azienda, color: "#000", caselle: [], tot, on, rotto: false, avviso: false, motivi: [] });

describe("leggiGruppi", () => {
  it("tiene solo nomi con un elenco di testi", () => {
    expect(leggiGruppi({ antonio: ["ITV", 3, null, "BBVA"], samuele: [], rotto: "ITV", altro: { a: 1 } })).toEqual({ antonio: ["ITV", "BBVA"], samuele: [] });
  });
  it.each([[undefined], [null], ["x"], [[]], [42]])("%j non è un elenco di gruppi", (grezzo) => expect(leggiGruppi(grezzo)).toEqual({}));
});

describe("aggiungiGruppo", () => {
  it("crea un gruppo vuoto in fondo, con il nome ripulito", () => {
    const r = aggiungiGruppo({ antonio: ["ITV"] }, "  Lavoro   fisso ");
    expect(r).toEqual({ gruppi: { antonio: ["ITV"], "Lavoro fisso": [] } });
  });
  it.each([
    ["", "Scrivi un nome per il gruppo."],
    ["   ", "Scrivi un nome per il gruppo."],
    ["ANTONIO", "Esiste già un gruppo con questo nome."],
    ["x".repeat(41), "Massimo 40 caratteri."],
  ])("rifiuta %j", (nome, errore) => expect(aggiungiGruppo({ antonio: [] }, nome)).toEqual({ errore }));
});

describe("rinominaGruppo", () => {
  it("il nuovo nome prende il posto del vecchio, nello stesso ordine e con gli stessi calendari", () => {
    const r = rinominaGruppo({ a: ["ITV"], b: ["ROAR"], c: [] }, "b", "Beta");
    expect(r).toEqual({ gruppi: { a: ["ITV"], Beta: ["ROAR"], c: [] } });
    expect(Object.keys((r as { gruppi: object }).gruppi)).toEqual(["a", "Beta", "c"]);
  });
  it("lo stesso nome (anche con altre maiuscole) non è un duplicato di sé stesso", () => {
    expect(rinominaGruppo({ antonio: ["ITV"] }, "antonio", "Antonio")).toEqual({ gruppi: { Antonio: ["ITV"] } });
  });
  it("rifiuta il nome di un altro gruppo e quello vuoto", () => {
    expect(rinominaGruppo({ a: [], b: [] }, "a", "b")).toEqual({ errore: "Esiste già un gruppo con questo nome." });
    expect(rinominaGruppo({ a: [], b: [] }, "a", " ")).toEqual({ errore: "Scrivi un nome per il gruppo." });
  });
});

describe("eliminaGruppo e impostaAppartenenza", () => {
  it("elimina solo quel gruppo", () => expect(eliminaGruppo({ a: ["ITV"], b: [] }, "a")).toEqual({ b: [] }));

  it("un calendario può stare in più gruppi", () => {
    let g: GruppiCalendari = { antonio: [], samuele: [] };
    g = impostaAppartenenza(g, "antonio", "ITV", true);
    g = impostaAppartenenza(g, "samuele", "ITV", true);
    g = impostaAppartenenza(g, "samuele", "ROAR", true);
    expect(g).toEqual({ antonio: ["ITV"], samuele: ["ITV", "ROAR"] });
  });
  it("metterlo due volte non lo duplica, toglierlo lo toglie solo da lì", () => {
    let g = impostaAppartenenza({ a: ["ITV"], b: ["ITV"] }, "a", "ITV", true);
    expect(g).toEqual({ a: ["ITV"], b: ["ITV"] });
    g = impostaAppartenenza(g, "a", "ITV", false);
    expect(g).toEqual({ a: [], b: ["ITV"] });
  });
  it("non cambia l'oggetto di partenza", () => {
    const g = { a: ["ITV"] };
    impostaAppartenenza(g, "a", "ROAR", true);
    expect(g).toEqual({ a: ["ITV"] });
  });
});

describe("gruppiAttivi", () => {
  const aziende = [az("ITV", 1), az("BBVA", 1), az("ROAR", 0), az("Serco", 0)];
  it("un gruppo è attivo quando le aziende accese sono proprio le sue", () => {
    expect(gruppiAttivi({ antonio: ["ITV", "BBVA"], samuele: ["ROAR", "Serco"] }, aziende)).toEqual(["antonio"]);
  });
  it("un'azienda accesa solo in parte (un calendario su due) conta come accesa", () => {
    expect(gruppiAttivi({ solo: ["ITV"] }, [az("ITV", 1, 2), az("ROAR", 0, 1)])).toEqual(["solo"]);
  });
  it("più gruppi con le stesse aziende sono attivi insieme", () => {
    expect(gruppiAttivi({ a: ["ITV", "BBVA"], b: ["BBVA", "ITV"] }, aziende)).toEqual(["a", "b"]);
  });
  it("niente di attivo se ne manca una o ne avanza una", () => {
    expect(gruppiAttivi({ a: ["ITV"], b: ["ITV", "BBVA", "ROAR"] }, aziende)).toEqual([]);
  });
  it("i nomi che non esistono più non contano, un gruppo senza nessuna azienda esistente non è mai attivo", () => {
    expect(gruppiAttivi({ a: ["ITV", "BBVA", "Vecchia"], vuoto: [], fantasma: ["Vecchia"] }, aziende)).toEqual(["a"]);
    expect(gruppiAttivi({ vuoto: [] }, [az("ITV", 0)])).toEqual([]);
  });
});
