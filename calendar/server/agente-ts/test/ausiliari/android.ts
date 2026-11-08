// Le prove del motore passano dalla vera lettura dell'agente (src/fonte.ts): le righe del caso diventano un calendar.db in memoria e si
// leggono con le stesse query SQL, la stessa scelta dei calendari e gli stessi doppioni tolti che in produzione.

import { estrai, eventiDaLettura, espandiRigheAndroid, type OccorrenzaAndroid } from "../../src/index.ts";
import { type Db, type Fonte, creaDbInMemoria } from "./memdb.ts";

export type { Db, Fonte } from "./memdb.ts";
export type { RigaDb } from "../../src/index.ts";

function leggi(db: Db, fonte: Fonte) {
  const sql = creaDbInMemoria(db);
  let tipo: string | null = null;
  try {
    const lettura = estrai(sql, fonte.account.trim().toLowerCase(), (fonte.calendari ?? []).map((c) => c.trim().toLowerCase()), (t) => (tipo = t));
    return { lettura, tipo };
  } finally {
    sql.close();
  }
}

export function occorrenzeGrezze(db: Db, fonte: Fonte, inizio: string, fine: string): OccorrenzaAndroid[] | null {
  const { lettura } = leggi(db, fonte);
  return lettura ? espandiRigheAndroid(lettura.eventi, inizio, fine) : null;
}

/** Gli eventi come li restituisce CalendarioAndroid.eventi: un'occorrenza per titolo, inizio e fine (la più completa), in ordine. */
export function eventiAgente(db: Db, fonte: Fonte, inizio: string, fine: string, grezzi?: OccorrenzaAndroid[]) {
  const { lettura, tipo } = leggi(db, fonte);
  if (!lettura) return null;
  return eventiDaLettura(lettura, { id: fonte.id, nome: fonte.nome, account: fonte.account.trim().toLowerCase(), tipo }, inizio, fine, "Europe/Rome", grezzi);
}
