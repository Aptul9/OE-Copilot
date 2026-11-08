import crypto from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { afterAll, describe, expect, it } from "vitest";
import { ErroreTipo, ErroreValore, espandiIcs, espandiRigheAndroid, confrontaPy, validaIcs } from "../src/index.ts";
import { canonico } from "./ausiliari/confronto.ts";
import { leggiGolden } from "./ausiliari/dati.ts";

// Le regole orarie (HOURLY, MINUTELY, SECONDLY), BYWEEKNO e BYEASTER (che c'è solo in dateutil): casi nuovi registrati con le librerie vere (icalendar 7.3.0, recurring-ical-events 3.8.2,
// dateutil 2.9.0, nell'immagine dell'agente, TZ=Europe/Rome). Per ogni caso e finestra il numero di occorrenze e l'impronta sha256 della lista
// nell'ordine in cui Python la dà (h) e ordinata (hs), o l'errore con il suo testo. Le liste intere non stanno nel repository.
const registrato = { casi: [...leggiGolden<{ casi: any[] }>("ricorrenze-orarie.json").casi, ...leggiGolden<{ casi: any[] }>("ricorrenze-rare.json").casi] };

const impronta = (v: unknown) => crypto.createHash("sha256").update(canonico(v), "utf-8").digest("hex").slice(0, 20);
const riassunto = (lista: unknown[]) => ({ n: lista.length, h: impronta(lista), hs: impronta([...lista].sort((a, b) => confrontaPy(canonico(a), canonico(b)))) });
const errore = (e: unknown) => ({ errore: e instanceof ErroreValore ? "ValueError" : e instanceof ErroreTipo ? "TypeError" : "?", messaggio: (e as Error).message.slice(0, 200) });

function android(caso: any, finestra: string) {
  const [inizio, fine] = finestra.split("|") as [string, string];
  try {
    return riassunto(espandiRigheAndroid(caso.righe, inizio, fine).map(({ descrizione: _d, ...resto }) => resto));
  } catch (e) {
    return errore(e);
  }
}

function ics(caso: any, finestra: string) {
  const [inizio, fine] = finestra.split("|") as [string, string];
  try {
    return riassunto(espandiIcs(caso.testo, inizio, fine));
  } catch (e) {
    return errore(e);
  }
}

const esiti = new Map<string, boolean>();
const categorie = [...new Set(registrato.casi.map((c) => c.categoria as string))];

describe.each(categorie)("regole orarie e settimane dell'anno, categoria %s", (categoria) => {
  it.each(registrato.casi.filter((c) => c.categoria === categoria).map((c) => [c.nome, c] as const))("%s", (nome, caso) => {
    const attesi = caso.atteso;
    const ottenuti: Record<string, unknown> = {};
    for (const f of caso.finestre as string[][]) {
      const k = f.join("|");
      const o = caso.tipo === "android" ? android(caso, k) : ics(caso, k);
      // l'errore di Python ha anche il tipo "valore" (ValueError o TypeError): qui basta che il testo sia lo stesso
      const a = attesi[k];
      ottenuti[k] = "errore" in a ? { errore: a.errore, messaggio: (o as any).messaggio, ...("valore" in a ? { valore: a.valore } : {}) } : o;
    }
    if (caso.tipo === "ics" && "errore" in attesi.valida) expect(() => validaIcs(caso.testo)).toThrow();
    const uguali = Object.keys(attesi).every((k) => k === "valida" || isDeepStrictEqual(ottenuti[k], attesi[k]));
    esiti.set(nome, uguali);
    expect(Object.fromEntries(Object.entries(ottenuti).filter(([k]) => k !== "valida"))).toEqual(Object.fromEntries(Object.entries(attesi).filter(([k]) => k !== "valida")));
  });
});

afterAll(() => {
  const righe = categorie.map((cat) => {
    const casi = registrato.casi.filter((c) => c.categoria === cat);
    const ok = casi.filter((c) => esiti.get(c.nome)).length;
    return `${ok === casi.length ? "OK  " : "NO  "}${cat.padEnd(22)} ${String(ok).padStart(3)}/${String(casi.length).padEnd(3)} uguali a Python`;
  });
  console.log(`\nRegole orarie, BYWEEKNO e BYEASTER (${[...esiti.values()].filter(Boolean).length} di ${registrato.casi.length} casi uguali alle librerie Python)\n${righe.join("\n")}`);
});
