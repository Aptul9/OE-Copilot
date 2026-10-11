import { afterAll, describe, expect, it } from "vitest";
import { type Esito, esitoCaso } from "./ausiliari/confronto";
import { leggiGolden } from "./ausiliari/dati";

// Casi inventati per le categorie che il golden dell'agente non copre (cambio dell'ora, esclusioni in tutte le forme, modifiche e
// annullamenti, giorni interi, fusi, regole, bordi della finestra, durate, calendari ICS...), con la risposta che le librerie vere
// (icalendar 7.3.0, recurring-ical-events 3.8.2, x-wr-timezone 2.0.1, dateutil 2.9.0) danno nell'immagine dell'agente, in Linux.
// Ogni caso è uguale a Python salvo quelli elencati qui, con il perché: se uno cambia esito la prova lo dice.
const extra = leggiGolden<{ librerie: Record<string, string>; casi: any[] }>("ricorrenze-extra.json");

const NOTI: Record<string, { stato: Esito["stato"]; motivo: string }> = {
  "regola-yearly-byweekno": { stato: "non-supportato", motivo: "BYWEEKNO: non lo scrivono né Outlook né Google" },
  "regola-hourly": { stato: "non-supportato", motivo: "frequenza oraria: non la scrivono né Outlook né Google" },
  "regola-hourly-attraversa-cambio": { stato: "non-supportato", motivo: "frequenza oraria" },
  "regola-minutely": { stato: "non-supportato", motivo: "frequenza al minuto" },
  "ics-regola-hourly": { stato: "non-supportato", motivo: "frequenza oraria" },
  "ics-vtimezone-personalizzata": { stato: "diverso", motivo: "un VTIMEZONE definito nel file non si interpreta: valgono i nomi IANA e una tabella dei nomi di Windows" },
};

const esiti = new Map<string, Esito>();
const categorie = [...new Set(extra.casi.map((c) => `${c.tipo}/${c.categoria}`))];

describe.each(categorie)("ricorrenze, categoria %s", (categoria) => {
  it.each(extra.casi.filter((c) => `${c.tipo}/${c.categoria}` === categoria).map((c) => [c.nome, c] as const))("%s", (nome, caso) => {
    const esito = esitoCaso(caso);
    esiti.set(nome, esito);
    const noto = NOTI[nome];
    expect(esito.stato === "diverso" && !noto ? esito.dettaglio : esito.stato).toBe(noto ? noto.stato : "uguale");
  });
});

afterAll(() => {
  const righe: string[] = [];
  let uguali = 0;
  for (const cat of categorie) {
    const casi = extra.casi.filter((c) => `${c.tipo}/${c.categoria}` === cat);
    const conta = { uguale: 0, diverso: 0, "non-supportato": 0 };
    const fuori: string[] = [];
    for (const c of casi) {
      const e = esiti.get(c.nome);
      if (!e) continue;
      conta[e.stato]++;
      if (e.stato !== "uguale") fuori.push(`${c.nome} (${e.stato === "non-supportato" ? "non supportato" : "diverso"}: ${NOTI[c.nome]?.motivo ?? "?"})`);
    }
    uguali += conta.uguale;
    righe.push(`${(conta.diverso + conta["non-supportato"] ? "NO  " : "OK  ") + cat.padEnd(22)} ${String(conta.uguale).padStart(3)}/${String(casi.length).padEnd(3)} uguali a Python${fuori.length ? ": " + fuori.join("; ") : ""}`);
  }
  console.log(`\nEsito per categoria (casi inventati, ${uguali} di ${extra.casi.length} uguali alle librerie Python ${JSON.stringify(extra.librerie)})\n${righe.join("\n")}`);
});
