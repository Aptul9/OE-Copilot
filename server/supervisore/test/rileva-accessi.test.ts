import { writeFileSync, rmSync } from "node:fs";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { OrologioFinto } from "../src/orologio.js";
import { accessiGoogle } from "../src/rilevatore/google.js";
import { freschezzaOutlook, type Verdetti } from "../src/rilevatore/outlook.js";
import { peggiore } from "../src/rilevatore/accessi.js";
import { secondiDaDataOra } from "../src/python.js";
import { MAIL_ACCOUNTS, SOGLIE, TelefonoFinto, creaCartelle, contesto, creaDb, rigaCasella, rigaPush, scrivi, type Cartelle } from "./aiuti.js";

// Ogni caso della vecchia prova Python del rilevatore (tolta dal repository; dati finti, nessun telefono) con le stesse attese, nello stesso ordine: i
// file di una sezione restano per la successiva, come nella prova in Python.

const ADESSO = 1_760_000_000;
let c: Cartelle;
let ctx: ReturnType<typeof contesto>;

const fa = (secondi: number): number => ADESSO - secondi;
const casella = (secondiFa: number, idh: number, sincronizza = "true"): string => rigaCasella(fa(secondiFa), idh, sincronizza);
const push = (secondiFa: number, disponibile = true): string => rigaPush(fa(secondiFa), disponibile);

function battito(davanti: boolean, bloccato = false): void {
  writeFileSync(
    c.battito,
    `{"quando": ${ADESSO}, "pausa": false, "telefono": true, "outlook_davanti": ${davanti}, "bloccato": ${bloccato}}`,
  );
}

const verdetti = (): Promise<Verdetti> => freschezzaOutlook(ctx, ADESSO);

async function controlla(v: Verdetti | Promise<Verdetti>, email: string, statoAtteso: string, testo = ""): Promise<void> {
  const trovato = (await v).get(email);
  expect(trovato, email).toBeDefined();
  expect(trovato?.stato).toBe(statoAtteso);
  expect(trovato?.msg).toContain(testo);
}

beforeAll(() => {
  c = creaCartelle();
  creaDb(c.db, [MAIL_ACCOUNTS]);
  ctx = contesto(c, new TelefonoFinto(), new OrologioFinto(ADESSO * 1000));
});

afterAll(() => c.pulisci());

describe("freschezza di Outlook", () => {
  test("1. Outlook vivo, push attive, telefono bloccato (non davanti): nessun avviso", async () => {
    battito(false);
    scrivi(c.log, "calendarSync.log", [casella(60, 32768), casella(55, 32772), casella(30, 32768), casella(25, 32772)]);
    scrivi(c.log, "notifications.log", [push(20)]);
    const v = await verdetti();
    await controlla(v, "roar@example.com", "ok");
    await controlla(v, "band@example.com", "ok");
  });

  test("2. una casella muta da 15 minuti: giallo, con i minuti", async () => {
    scrivi(c.log, "calendarSync.log", [casella(900, 32772), casella(30, 32768)]);
    await controlla(verdetti(), "band@example.com", "avviso", "da 15 min");
    await controlla(verdetti(), "roar@example.com", "ok");
  });

  test("3. muta da un'ora: rosso", async () => {
    scrivi(c.log, "calendarSync.log", [casella(3600, 32772), casella(30, 32768)]);
    await controlla(verdetti(), "band@example.com", "errore", "fermo o sospeso");
  });

  test("4. Outlook dice di aver sospeso la casella: giallo", async () => {
    scrivi(c.log, "calendarSync.log", [casella(30, 32772, "false"), casella(30, 32768)]);
    await controlla(verdetti(), "band@example.com", "avviso", "sospeso");
  });

  test("5. nessuna riga per la casella (log ruotato da poco, la coda non la contiene): rosso", async () => {
    scrivi(c.log, "calendarSync.log", [casella(30, 32768)]);
    await controlla(verdetti(), "band@example.com", "errore");
  });

  test("6. push assenti e Outlook non davanti: giallo; con Outlook davanti: ok", async () => {
    scrivi(c.log, "calendarSync.log", [casella(30, 32768), casella(30, 32772)]);
    scrivi(c.log, "notifications.log", [push(20, false)]);
    await controlla(verdetti(), "roar@example.com", "avviso", "notifiche push");
    battito(true);
    await controlla(verdetti(), "roar@example.com", "ok");
    battito(false);
  });

  test("7. una riga sulle push vecchia di un'ora non dice niente", async () => {
    scrivi(c.log, "notifications.log", [push(3600, false)]);
    await controlla(verdetti(), "roar@example.com", "ok");
  });

  test("8. log ruotato: il corrente è vuoto, le righe stanno in calendarSync.log.1", async () => {
    scrivi(c.log, "calendarSync.log", []);
    scrivi(c.log, "calendarSync.log.1", [casella(40, 32768), casella(40, 32772)]);
    await controlla(verdetti(), "band@example.com", "ok");
  });

  test("9. log di Outlook assenti: nessun verdetto invece di un falso rosso o verde", async () => {
    rmSync(path.join(c.log, "calendarSync.log"));
    expect((await verdetti()).size).toBe(0);
  });
});

const DUMP = `Sync Status
Account utente@example.com u0 com.microsoft.office.outlook.USER_ACCOUNT
=======================================================================
com.android.calendar  1         true     Total  0    0     4466  4     0     8     4478  0     0    1m14s  PERIODIC SUCCESS
                                         Today             685                     685              10s    2026-10-11 07:09:39
Account g@example.com u0 com.google
=======================================================================
com.android.calendar                     1         true     Total  1    0     242  3     0     478   724  1     1    20m39s  %(esito)s
                                                            Today             173              346   519             14m51s  %(ora)s
                                                            Yestr  1          69   3           132   205  1     1    5m47s
com.android.contacts                     -1        false    Total  0    0     0    0     0     1     1    1     0    0s      OTHER FAILURE
                                                            Today                                                    0s      2026-10-10 21:22:56
                                                            Yestr                              1     1    1          0s      internal-error
`;

const TELEFONO_ORA = "2026-10-11 09:00:00";

function google(esito: string, minutiFa: number, ricordo: Record<string, number>) {
  const oraTelefono = secondiDaDataOra(TELEFONO_ORA);
  const ora = new Date((oraTelefono - minutiFa * 60) * 1000).toISOString().slice(0, 19).replace("T", " ");
  const dump = DUMP.replace("%(esito)s", esito).replace("%(ora)s", ora);
  return accessiGoogle(dump, oraTelefono, ADESSO, new Map(Object.entries(ricordo)), SOGLIE);
}

describe("calendari Google", () => {
  test("10a. riuscito 1 minuto fa: ok (il fallimento dei contatti non conta)", () => {
    const [uscita] = google("OTHER SUCCESS", 1, {});
    expect(uscita.get("g@example.com")?.stato).toBe("ok");
  });

  test("10b. fallimento isolato: ok", () => {
    const [uscita] = google("OTHER FAILURE", 1, { "g@example.com": ADESSO - 120 });
    expect(uscita.get("g@example.com")?.stato).toBe("ok");
  });

  test("10c. in errore da 30 minuti: giallo", () => {
    const [uscita] = google("OTHER FAILURE", 1, { "g@example.com": ADESSO - 1800 });
    expect(uscita.get("g@example.com")?.stato).toBe("avviso");
    expect(uscita.get("g@example.com")?.msg).toContain("30 min");
  });

  test("10d. in errore da 2 ore: rosso", () => {
    const [uscita] = google("OTHER FAILURE", 1, { "g@example.com": ADESSO - 7200 });
    expect(uscita.get("g@example.com")?.stato).toBe("errore");
    expect(uscita.get("g@example.com")?.msg).toContain("accesso scaduto");
  });

  test("10e. sincronizzazione ferma da 40 minuti: giallo", () => {
    const [uscita] = google("OTHER SUCCESS", 40, {});
    expect(uscita.get("g@example.com")?.stato).toBe("avviso");
    expect(uscita.get("g@example.com")?.msg).toContain("non parte");
  });

  test("10f. fallimento senza un successo mai visto: si parte da ora", () => {
    const [uscita, ricordo] = google("OTHER FAILURE", 1, {});
    expect(uscita.get("g@example.com")?.stato).toBe("ok");
    expect(ricordo.get("g@example.com")).toBe(ADESSO);
  });
});

test("11. la somma dei verdetti tiene il più grave e unisce i messaggi", () => {
  expect(peggiore({ stato: "avviso", msg: "uno" }, { stato: "errore", msg: "due" })).toEqual({ stato: "errore", msg: "uno · due" });
});

describe("telefono bloccato (seconda sezione 8 della prova in Python)", () => {
  test("schermata di blocco su: giallo per le caselle Outlook, con il messaggio che dice cosa fare", async () => {
    scrivi(c.log, "calendarSync.log", [casella(30, 32768), casella(30, 32772)]);
    scrivi(c.log, "notifications.log", [push(20)]);
    battito(false, true);
    await controlla(verdetti(), "roar@example.com", "avviso", "Telefono bloccato");
    await controlla(verdetti(), "band@example.com", "avviso", "sbloccalo dalla pagina Telefono");
  });

  test("sbloccato: ok", async () => {
    battito(true, false);
    await controlla(verdetti(), "roar@example.com", "ok");
  });

  test("un guasto vero (casella muta) pesa più del blocco", async () => {
    scrivi(c.log, "calendarSync.log", [casella(3600, 32772), casella(30, 32768)]);
    battito(false, true);
    await controlla(verdetti(), "band@example.com", "errore", "fermo o sospeso");
    battito(false);
  });
});
