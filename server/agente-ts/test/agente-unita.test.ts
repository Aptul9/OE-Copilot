import { describe, expect, it } from "vitest";
import { ErroreSenzaRisposta, ErroreValore, analizzaTarget, confrontaPy, htmlUnescape, isoNaive, jsonPy, oraLocaleDa, parseQs, pulisciLink, pyReprStr, pyStrip } from "../src/index.ts";
import { HTML5 } from "../src/entita.ts";
import { pyFloat } from "../src/server.ts";
import { leggiGolden } from "./ausiliari/dati.ts";

// I pezzi che l'agente TypeScript riscrive, uno per uno contro Python 3.12.15 (immagine dell'agente, TZ=Europe/Rome): datetime.fromisoformat e
// _locale, html.unescape e _pulisci_link, urlparse e parse_qs, json.dumps, repr, str.strip, sorted, float. Le risposte sono registrate sulla VM
// (un campione: il registro completo, 40000 casi, è fuori dal repository e si è confrontato per intero).
const u = leggiGolden("agente-unita.json");

/** Raccoglie le differenze e le mostra in fondo: se ce ne sono, il test dice le prime. */
function controlla(nome: string, casi: any[], f: (c: any) => unknown) {
  it(`${nome}: ${casi.length} casi uguali a Python`, () => {
    const diverse: unknown[] = [];
    for (const c of casi) {
      let ottenuto: unknown;
      try {
        ottenuto = f(c);
      } catch (e) {
        ottenuto = e instanceof ErroreSenzaRisposta ? { err: e.message.split(": ")[0], msg: e.message.split(": ").slice(1).join(": ") } : { err: e instanceof ErroreValore ? "ValueError" : "?", msg: (e as Error).message };
      }
      const atteso = "ok" in c ? c.ok : "out" in c ? c.out : { err: c.err, msg: c.msg };
      if (JSON.stringify(ottenuto) !== JSON.stringify(atteso)) diverse.push({ in: c.in, atteso, ottenuto });
    }
    expect(diverse.slice(0, 5)).toEqual([]);
  });
}

describe("agente TypeScript contro Python, pezzo per pezzo", () => {
  it("la tabella delle entità HTML5 ha i nomi di Python", () => expect(HTML5.size).toBe(u.html5));
  controlla("datetime.fromisoformat e _locale", u.iso, (c) => isoNaive(oraLocaleDa(c.in, "Europe/Rome")));
  controlla("html.unescape", u.html, (c) => htmlUnescape(c.in));
  controlla("_pulisci_link", u.link, (c) => pulisciLink(c.in));
  controlla("urlparse e parse_qs", u.url, (c) => {
    const { percorso, query } = analizzaTarget(c.in);
    return { path: percorso, query, qs: [...parseQs(query)].map(([k, v]) => [k, v]) };
  });
  controlla("json.dumps(ensure_ascii=False)", u.json, (c) => jsonPy(JSON.parse(c.in)));
  controlla("repr di una stringa", u.repr, (c) => pyReprStr(c.in));
  controlla("str.strip", u.strip, (c) => pyStrip(c.in));
  controlla("sorted di stringhe (per punto di codice)", u.sort, (c) => [...c.in].sort(confrontaPy));
  controlla("float", u.float, (c) => {
    const x = pyFloat(c.in);
    return Number.isNaN(x) ? "NaN" : x === Infinity ? "Infinity" : x === -Infinity ? "-Infinity" : x;
  });
});
