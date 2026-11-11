import { beforeEach, describe, expect, it } from "vitest";
import {
  azzera,
  blocchi,
  dataValida,
  eventiPubblici,
  fuoriFinestra,
  giorniTra,
  GIORNI_BLOCCO,
  leggiBlocco,
  leggiDisponibilita,
  leggiRichiesta,
  oggiARoma,
  rinfresca,
  spostaData,
  unisci,
} from "@/lib/disponibilita";

beforeEach(azzera);

describe("date", () => {
  it.each([
    ["2026-10-12", true],
    ["2028-02-29", true],
    ["2026-02-29", false],
    ["2026-13-01", false],
    ["2026-10-1", false],
    ["ieri", false],
    ["", false],
  ])("dataValida(%j) = %s", (s, atteso) => expect(dataValida(s)).toBe(atteso));

  it("spostaData e giorniTra passano mesi e anni", () => {
    expect(spostaData("2026-10-12", 7)).toBe("2026-10-19");
    expect(spostaData("2026-10-28", 7)).toBe("2026-11-04");
    expect(spostaData("2026-12-30", 3)).toBe("2027-01-02");
    expect(spostaData("2028-02-28", 1)).toBe("2028-02-29");
    expect(spostaData("2026-10-12", -12)).toBe("2026-09-30");
    expect(giorniTra("2026-10-12", "2026-10-19")).toBe(7);
    expect(giorniTra("2026-10-19", "2026-10-12")).toBe(-7);
    expect(giorniTra("2026-03-28", "2026-03-30")).toBe(2); // il cambio d'ora non sposta i giorni
  });

  it("oggiARoma segue l'ora legale e quella solare, qualunque fuso abbia il server", () => {
    expect(oggiARoma(new Date("2026-10-11T22:30:00Z"))).toBe("2026-10-12");
    expect(oggiARoma(new Date("2026-12-31T23:30:00Z"))).toBe("2027-01-01");
    expect(oggiARoma(new Date("2026-10-11T08:07:00Z"))).toBe("2026-10-11");
  });
});

describe("leggiRichiesta", () => {
  const q = (s: string) => new URLSearchParams(s);
  it("legge le date come le manda il calendario (con ora e scarto) e conta solo il giorno", () => {
    expect(leggiRichiesta(q("da=2026-10-12T00:00:00%2B02:00&a=2026-10-19T00:00:00%2B02:00"))).toEqual({ da: "2026-10-12", a: "2026-10-19", gruppo: "" });
  });
  it("accetta anche la sola data, e il gruppo ripulito dagli spazi", () => {
    expect(leggiRichiesta(q("da=2026-10-12&a=2026-10-13&gruppo=%20antonio%20"))).toEqual({ da: "2026-10-12", a: "2026-10-13", gruppo: "antonio" });
    expect(leggiRichiesta(q("da=2026-10-12&a=2026-10-13&gruppo=Lavoro%20di%20pi%C3%B9"))).toMatchObject({ gruppo: "Lavoro di più" });
  });
  it.each([
    ["", /da, a/],
    ["da=2026-10-12", /da, a/],
    ["da=ieri&a=oggi", /da, a/],
    ["da=2026-02-30&a=2026-03-05", /da, a/],
    ["da=2026-10-12&a=2026-10-12", /a:/], // intervallo vuoto
    ["da=2026-10-12&a=2026-10-11", /a:/], // al contrario
    ["da=2026-10-12&a=2026-12-20", /a:/], // oltre i 62 giorni
    [`da=2026-10-12&a=2026-10-13&gruppo=${"x".repeat(61)}`, /gruppo:/],
    ["da=2026-10-12&a=2026-10-13&gruppo=a%00b", /gruppo:/],
    ["da=2026-10-12&a=2026-10-13&gruppo=a%0Ab", /gruppo:/],
  ])("rifiuta %s", (s, errore) => {
    const r = leggiRichiesta(q(s));
    expect("errore" in r && r.errore).toMatch(errore);
  });
  it("62 giorni sono ammessi, 63 no", () => {
    expect("errore" in leggiRichiesta(q("da=2026-10-12&a=2026-12-13"))).toBe(false);
    expect("errore" in leggiRichiesta(q("da=2026-10-12&a=2026-12-14"))).toBe(true);
  });
  it("fuoriFinestra: oltre un anno e un mese indietro o avanti non si legge niente", () => {
    const r = (da: string, a: string) => ({ da, a, gruppo: "" });
    expect(fuoriFinestra(r("2026-10-05", "2026-10-12"), "2026-10-12")).toBe(false);
    expect(fuoriFinestra(r("2025-01-01", "2025-01-08"), "2026-10-12")).toBe(true);
    expect(fuoriFinestra(r("2028-01-01", "2028-01-08"), "2026-10-12")).toBe(true);
    expect(fuoriFinestra(r("2027-11-01", "2027-11-08"), "2026-10-12")).toBe(false);
  });
});

describe("blocchi", () => {
  const lunedi = (d: string) => new Date(d + "T00:00:00Z").getUTCDay() === 1;
  it("un giorno solo è un blocco, che parte di lunedì e dura 28 giorni", () => {
    const b = blocchi("2026-10-14", "2026-10-15");
    expect(b).toHaveLength(1);
    expect(lunedi(b[0])).toBe(true);
    expect(b[0] <= "2026-10-14" && "2026-10-14" < spostaData(b[0], GIORNI_BLOCCO)).toBe(true);
  });
  it("uno stesso giorno cade sempre nello stesso blocco, qualunque sia la richiesta", () => {
    const uno = blocchi("2026-10-14", "2026-10-15");
    expect(blocchi("2026-10-12", "2026-10-19")).toContain(uno[0]);
    expect(blocchi("2026-10-14", "2026-10-17")).toContain(uno[0]);
  });
  it("un intervallo a cavallo di due blocchi ne chiede due, di fila", () => {
    const b = blocchi("2026-10-01", "2026-11-30");
    expect(b.length).toBeGreaterThanOrEqual(2);
    for (let i = 1; i < b.length; i++) expect(giorniTra(b[i - 1], b[i])).toBe(GIORNI_BLOCCO);
    expect(b[0] <= "2026-10-01").toBe(true);
    expect(spostaData(b[b.length - 1], GIORNI_BLOCCO) >= "2026-11-30").toBe(true);
  });
  it("l'ultimo giorno escluso non chiede un blocco in più", () => {
    const b = blocchi("2026-10-12", "2026-10-19");
    expect(blocchi(b[0], spostaData(b[0], GIORNI_BLOCCO))).toEqual([b[0]]);
    expect(blocchi(b[0], spostaData(b[0], GIORNI_BLOCCO + 1))).toHaveLength(2);
  });
  it("prima del 2024 funziona uguale, e 62 giorni sono al più 4 blocchi", () => {
    expect(blocchi("2023-06-10", "2023-06-11")).toHaveLength(1);
    expect(blocchi("2026-10-12", "2026-12-13").length).toBeLessThanOrEqual(4);
  });
});

describe("unisci", () => {
  it("sovrapposti e attaccati diventano uno, in ordine, il più lungo vince", () => {
    expect(
      unisci([
        ["2026-10-12T14:00:00", "2026-10-12T15:00:00"],
        ["2026-10-12T09:00:00", "2026-10-12T11:00:00"],
        ["2026-10-12T10:00:00", "2026-10-12T10:30:00"],
        ["2026-10-12T11:00:00", "2026-10-12T12:00:00"],
      ]),
    ).toEqual([
      ["2026-10-12T09:00:00", "2026-10-12T12:00:00"],
      ["2026-10-12T14:00:00", "2026-10-12T15:00:00"],
    ]);
  });
  it("un impegno lungo che arriva da due blocchi con inizi diversi è uno solo", () => {
    expect(unisci([["2026-10-29T14:15:00", "2026-11-08T10:00:00"], ["2026-10-29T15:00:00", "2026-11-05T10:00:00"]])).toEqual([["2026-10-29T14:15:00", "2026-11-08T10:00:00"]]);
  });
  it("un buco di un secondo resta un buco, e il risultato non cambia se si unisce due volte", () => {
    const v: [string, string][] = [["2026-10-12T09:00:00", "2026-10-12T10:00:00"], ["2026-10-12T10:00:01", "2026-10-12T11:00:00"]];
    expect(unisci(v)).toHaveLength(2);
    expect(unisci(unisci(v))).toEqual(unisci(v));
    expect(unisci([])).toEqual([]);
  });
  it("non cambia l'elenco di partenza", () => {
    const v: [string, string][] = [["2026-10-12T10:00:00", "2026-10-12T11:00:00"], ["2026-10-12T09:00:00", "2026-10-12T10:30:00"]];
    unisci(v);
    expect(v[0][0]).toBe("2026-10-12T10:00:00");
  });
});

describe("eventiPubblici", () => {
  const imp: [string, string][] = [
    ["2026-10-12T09:00:00", "2026-10-12T10:00:00"],
    ["2026-10-12T09:00:00", "2026-10-12T10:00:00"], // doppione di un altro blocco
    ["2026-10-11T23:00:00", "2026-10-12T00:30:00"], // entra
    ["2026-10-05T09:00:00", "2026-10-05T10:00:00"], // prima
    ["2026-10-19T00:00:00", "2026-10-19T01:00:00"], // dopo (a è escluso)
  ];
  it("ogni blocco occupato diventa un 'Occupato' con i soli orari, senza doppioni né fuori intervallo", () => {
    const e = eventiPubblici(imp, [], "2026-10-12", "2026-10-19");
    expect(e.map((x) => [x.start, x.end])).toEqual([
      ["2026-10-11T23:00:00", "2026-10-12T00:30:00"],
      ["2026-10-12T09:00:00", "2026-10-12T10:00:00"],
    ]);
    for (const x of e) {
      expect(x).toMatchObject({ title: "Occupato", allDay: false, account: "" });
      expect(Object.keys(x).sort()).toEqual(["account", "allDay", "color", "end", "id", "start", "title"]);
    }
    expect(new Set(e.map((x) => x.id)).size).toBe(e.length);
  });
  it("le festività restano festività: giorno intero del calendario delle feste, col loro nome", () => {
    const e = eventiPubblici([], [["2026-11-01T00:00:00", "2026-11-02T00:00:00", "Ognissanti"], ["2026-11-01T00:00:00", "2026-11-02T00:00:00", "Ognissanti"], ["2026-12-25T00:00:00", "2026-12-26T00:00:00", "Natale"]], "2026-10-26", "2026-11-30");
    expect(e).toHaveLength(1);
    expect(e[0]).toMatchObject({ title: "Ognissanti", allDay: true, account: "Festività", start: "2026-11-01T00:00:00", end: "2026-11-02T00:00:00" });
  });
});

// ---- la lettura dei blocchi -----------------------------------------------------------------------
const BUONO = { ok: true, completo: true, impegni: [["2026-10-12T09:00:00", "2026-10-12T10:00:00"]], festivita: [["2026-11-01T00:00:00", "2026-11-02T00:00:00", "Ognissanti"]] };
const BLOCCO = blocchi("2026-10-14", "2026-10-15")[0];

function finto(risposta: unknown = BUONO) {
  const chiamate: { corpo: unknown }[] = [];
  const core = async (da: string, giorni: number, gruppo: string) => {
    chiamate.push({ corpo: [da, giorni, gruppo] });
    const r = typeof risposta === "function" ? (risposta as (n: number) => unknown)(chiamate.length) : risposta;
    if (r instanceof Error) throw r;
    return r;
  };
  return { chiamate, deps: { core } };
}

describe("leggiBlocco", () => {
  it("chiede alla lettura i parametri giusti: inizio del blocco, 28 giorni, gruppo", async () => {
    const { chiamate, deps } = finto();
    const e = await leggiBlocco(BLOCCO, "antonio", deps);
    expect(chiamate).toEqual([{ corpo: [BLOCCO, 28, "antonio"] }]);
    expect(e).toEqual({ ok: true, completo: true, impegni: BUONO.impegni, festivita: BUONO.festivita });
  });

  it("ricostruisce la risposta: un campo in più, un testo o un dettaglio non passano", async () => {
    const { deps } = finto({ ...BUONO, titolo: "SEGRETO", luogo: "SEGRETO", impegni: [["2026-10-12T09:00:00", "2026-10-12T10:00:00"]], festivita: [["2026-11-01T00:00:00", "2026-11-02T00:00:00", "Ognissanti\u0000\nx"]] });
    const e = await leggiBlocco(BLOCCO, "", deps);
    expect(JSON.stringify(e)).not.toContain("SEGRETO");
    expect(e.ok && e.festivita[0][2]).toBe("Ognissanti  x");
  });

  it("il nome di una festa si taglia a 80 caratteri", async () => {
    const { deps } = finto({ ...BUONO, festivita: [["2026-11-01T00:00:00", "2026-11-02T00:00:00", "x".repeat(200)]] });
    const e = await leggiBlocco(BLOCCO, "", deps);
    expect(e.ok && e.festivita[0][2]).toHaveLength(80);
  });

  it("un minuto in memoria: la seconda richiesta uguale non rilegge, una diversa sì", async () => {
    const { chiamate, deps } = finto();
    let t = 1_000_000;
    const d = { ...deps, adesso: () => t };
    await leggiBlocco(BLOCCO, "", d);
    t += 59_000;
    await leggiBlocco(BLOCCO, "", d);
    expect(chiamate).toHaveLength(1);
    await leggiBlocco(BLOCCO, "x", d);
    expect(chiamate).toHaveLength(2);
    t += 2_000;
    await leggiBlocco(BLOCCO, "", d);
    expect(chiamate).toHaveLength(3);
  });

  it("due richieste insieme fanno una lettura sola", async () => {
    const { chiamate, deps } = finto();
    const [a, b] = await Promise.all([leggiBlocco(BLOCCO, "", deps), leggiBlocco(BLOCCO, "", deps)]);
    expect(chiamate).toHaveLength(1);
    expect(a).toEqual(b);
  });

  it("dato incompleto: solo quindici secondi in memoria", async () => {
    const { chiamate, deps } = finto({ ...BUONO, completo: false });
    let t = 5_000_000;
    const d = { ...deps, adesso: () => t };
    expect(await leggiBlocco(BLOCCO, "", d)).toMatchObject({ ok: true, completo: false });
    await leggiBlocco(BLOCCO, "", d);
    expect(chiamate).toHaveLength(1);
    t += 16_000;
    await leggiBlocco(BLOCCO, "", d);
    expect(chiamate).toHaveLength(2);
  });

  it("gruppo inesistente: 404, e si ricorda un minuto", async () => {
    const { chiamate, deps } = finto({ ok: false, error: "Gruppo non trovato" });
    expect(await leggiBlocco(BLOCCO, "nessuno", deps)).toEqual({ ok: false, stato: 404 });
    await leggiBlocco(BLOCCO, "nessuno", deps);
    expect(chiamate).toHaveLength(1);
  });

  const t1 = ["2026-10-12T09:00:00", "2026-10-12T10:00:00"];
  it.each([
    ["errore della lettura", { ok: false, error: "Calendari non leggibili" }],
    ["una risposta di errore senza ok", { error: "boom" }],
    ["un testo al posto dell'oggetto", "<html>"],
    ["la lettura solleva un errore", new Error("ECONNREFUSED")],
    ["nessuna risposta", null],
    ["manca la lista degli impegni", { ok: true, completo: true, festivita: [] }],
    ["un orario di testo libero", { ...BUONO, impegni: [["9", "10"]] }],
    ["un orario con il fuso", { ...BUONO, impegni: [["2026-10-12T09:00:00+02:00", "2026-10-12T10:00:00+02:00"]] }],
    ["un impegno con un dettaglio in più", { ...BUONO, impegni: [[...t1, "titolo"]] }],
    ["un impegno al contrario", { ...BUONO, impegni: [[t1[1], t1[0]]] }],
    ["un impegno senza durata", { ...BUONO, impegni: [[t1[0], t1[0]]] }],
    ["una festa senza nome", { ...BUONO, festivita: [["2026-11-01T00:00:00", "2026-11-02T00:00:00"]] }],
    ["una festa con un nome non di testo", { ...BUONO, festivita: [["2026-11-01T00:00:00", "2026-11-02T00:00:00", 7]] }],
    ["troppi impegni", { ...BUONO, impegni: Array.from({ length: 5001 }, (_, i) => [`2026-10-12T09:00:${String(i % 60).padStart(2, "0")}`, "2026-10-12T10:00:00"]) }],
  ])("%s: 502", async (_nome, risposta) => {
    const { deps } = finto(risposta as never);
    expect(await leggiBlocco(BLOCCO, "", deps)).toEqual({ ok: false, stato: 502 });
  });

  it("`completo` conta solo se è vero davvero", async () => {
    for (const completo of ["true", 1, undefined, null]) {
      azzera();
      const { deps } = finto({ ...BUONO, completo });
      expect(await leggiBlocco(BLOCCO, "", deps)).toMatchObject({ ok: true, completo: false });
    }
  });

  it("al più venti letture vere al minuto in tutto: la ventunesima è 429, quelle in memoria no", async () => {
    const { chiamate, deps } = finto();
    let t = 9_000_000;
    const d = { ...deps, adesso: () => t };
    const esiti = [];
    for (let i = 0; i < 21; i++) esiti.push(await leggiBlocco(BLOCCO, `g${i}`, d));
    expect(chiamate).toHaveLength(20);
    expect(esiti[20]).toEqual({ ok: false, stato: 429 });
    expect(await leggiBlocco(BLOCCO, "g3", d)).toMatchObject({ ok: true });
    expect(chiamate).toHaveLength(20);
    t += 61_000;
    expect(await leggiBlocco(BLOCCO, "g20", d)).toMatchObject({ ok: true });
    expect(chiamate).toHaveLength(21);
  });

  it("la memoria non cresce oltre 64 voci", async () => {
    const { chiamate, deps } = finto();
    let t = 20_000_000;
    const d = { ...deps, adesso: () => t };
    for (let i = 0; i < 70; i++) {
      await leggiBlocco(BLOCCO, `g${i}`, d);
      t += 4_000; // una lettura ogni 4 secondi: dentro il limite, e la memoria da svuotare
    }
    expect(chiamate).toHaveLength(70);
    await leggiBlocco(BLOCCO, "g0", d); // la più vecchia è uscita: si legge ancora
    expect(chiamate).toHaveLength(71);
  });
});

describe("blocchi vecchi e giro in secondo piano", () => {
  /** Una lettura che risponde quando la prova lo dice. */
  function lenta() {
    const attese: ((v: unknown) => void)[] = [];
    let n = 0;
    const core = (_da: string, _giorni: number, gruppo: string) => {
      n++;
      return new Promise((risolvi) => attese.push(risolvi)).then((v) => v ?? { ...BUONO, impegni: [[`2026-10-12T0${n}:00:00`, "2026-10-12T10:00:00"]], gruppo });
    };
    return { attese, chiamate: () => n, core };
  }

  it("scaduto il minuto, il blocco vecchio si dà subito e il nuovo si legge in secondo piano", async () => {
    const l = lenta();
    let t = 30_000_000;
    const d = { core: l.core, adesso: () => t };
    const prima = leggiBlocco(BLOCCO, "", d);
    l.attese[0](undefined);
    const a = await prima;
    t += 120_000;
    const b = await leggiBlocco(BLOCCO, "", d); // non aspetta la lettura nuova
    expect(b).toEqual(a);
    expect(l.chiamate()).toBe(2);
    l.attese[1](undefined);
    await new Promise((r) => setTimeout(r, 0));
    const c = await leggiBlocco(BLOCCO, "", d);
    expect(c).not.toEqual(a);
    expect(l.chiamate()).toBe(2);
  });

  it("una lettura nuova che non riesce lascia il blocco vecchio", async () => {
    const { deps } = finto((n: number) => (n === 1 ? BUONO : { error: "boom" }));
    let t = 40_000_000;
    const d = { ...deps, adesso: () => t };
    const a = await leggiBlocco(BLOCCO, "", d);
    t += 120_000;
    await leggiBlocco(BLOCCO, "", d);
    await new Promise((r) => setTimeout(r, 0));
    t += 1_000;
    expect(await leggiBlocco(BLOCCO, "", d)).toEqual(a);
  });

  it("oltre sei ore il blocco vecchio non vale più e si aspetta la lettura", async () => {
    const l = lenta();
    let t = 50_000_000;
    const d = { core: l.core, adesso: () => t };
    const p = leggiBlocco(BLOCCO, "", d);
    l.attese[0](undefined);
    await p;
    t += 6 * 3600_000 + 1;
    let fatto = false;
    const q = leggiBlocco(BLOCCO, "", d).then(() => (fatto = true));
    await new Promise((r) => setTimeout(r, 0));
    expect(fatto).toBe(false);
    l.attese[1](undefined);
    await q;
  });

  it("il giro rilegge i blocchi delle prossime settimane, anche per i gruppi chiesti, e lascia quelli che non ci sono più", async () => {
    const chiesti: string[] = [];
    const core = async (da: string, _g: number, gruppo: string) => {
      chiesti.push(`${da}|${gruppo}`);
      return gruppo === "sparito" ? { ok: false, error: "Gruppo non trovato" } : BUONO;
    };
    await leggiDisponibilita({ da: "2026-10-12", a: "2026-10-19", gruppo: "lavoro" }, { core });
    await leggiDisponibilita({ da: "2026-10-12", a: "2026-10-19", gruppo: "sparito" }, { core });
    chiesti.length = 0;
    const n = await rinfresca({ core }, "2026-10-14");
    const attesi = blocchi("2026-10-07", "2026-12-14");
    expect(chiesti.filter((c) => c.endsWith("|"))).toEqual(attesi.map((b) => `${b}|`));
    expect(chiesti.filter((c) => c.endsWith("|lavoro"))).toEqual(attesi.map((b) => `${b}|lavoro`));
    expect(chiesti.filter((c) => c.endsWith("|sparito"))).toHaveLength(1);
    expect(n).toBe(attesi.length * 2 + 1);
    chiesti.length = 0;
    await rinfresca({ core }, "2026-10-14");
    expect(chiesti.some((c) => c.endsWith("|sparito"))).toBe(false);
    // dopo il giro la pagina non rilegge
    chiesti.length = 0;
    await leggiDisponibilita({ da: "2026-10-12", a: "2026-10-19", gruppo: "" }, { core });
    expect(chiesti).toEqual([]);
  });
});

describe("leggiDisponibilita", () => {
  const R = { da: "2026-10-12", a: "2026-10-19", gruppo: "" };

  it("una settimana: un blocco, gli eventi del calendario con titolo 'Occupato'", async () => {
    const { chiamate, deps } = finto();
    const e = await leggiDisponibilita(R, deps);
    expect(chiamate).toHaveLength(1);
    expect(e.ok && e.errors).toEqual([]);
    expect(e.ok && e.events.map((x) => [x.title, x.allDay])).toEqual([["Occupato", false]]);
  });

  it("un intervallo su due blocchi li legge tutti e due e unisce, senza doppioni", async () => {
    const due = blocchi("2026-10-01", "2026-11-30");
    expect(due.length).toBeGreaterThanOrEqual(2);
    const { chiamate, deps } = finto(); // ogni blocco risponde lo stesso impegno: va contato una volta
    const e = await leggiDisponibilita({ da: "2026-10-01", a: "2026-11-30", gruppo: "" }, deps);
    expect(chiamate).toHaveLength(due.length);
    expect(e.ok && e.events.filter((x) => !x.allDay)).toHaveLength(1);
    expect(e.ok && e.events.filter((x) => x.allDay)).toHaveLength(1);
  });

  it("un blocco incompleto (un accesso scaduto) non mette avvisi nella pagina pubblica", async () => {
    const { deps } = finto((n: number) => ({ ...BUONO, completo: n !== 1 }));
    const e = await leggiDisponibilita({ da: "2026-10-01", a: "2026-11-30", gruppo: "" }, deps);
    expect(e.ok && e.errors).toEqual([]);
    expect(e.ok && e.events.length).toBeGreaterThan(0);
  });

  it("se un blocco manca, il più specifico degli errori: 404 prima di 429 prima di 502", async () => {
    const risposte = [{ ok: false, error: "Gruppo non trovato" }, { error: "boom" }];
    const { deps } = finto((n: number) => risposte[n - 1] ?? BUONO);
    expect(await leggiDisponibilita({ da: "2026-10-01", a: "2026-11-30", gruppo: "x" }, deps)).toEqual({ ok: false, stato: 404 });
    azzera();
    const { deps: d2 } = finto((n: number) => (n === 1 ? { error: "boom" } : BUONO));
    expect(await leggiDisponibilita({ da: "2026-10-01", a: "2026-11-30", gruppo: "" }, d2)).toEqual({ ok: false, stato: 502 });
  });

  it("niente di quello che la lettura aggiunge arriva alla pagina", async () => {
    const { deps } = finto({ ...BUONO, titolo: "SEGRETO", impegni: BUONO.impegni });
    const e = await leggiDisponibilita(R, deps);
    expect(JSON.stringify(e)).not.toMatch(/SEGRETO|titolo|luogo|organizer|location/i);
  });
});
