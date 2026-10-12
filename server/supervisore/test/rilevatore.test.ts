import { chmodSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { afterAll, beforeEach, describe, expect, test } from "vitest";
import { OrologioFinto } from "../src/orologio.js";
import {
  ErroreValore,
  comeNumero,
  decodificaBase64,
  epocaDaLog,
  intero,
  jsonPython,
  leggiJsonPython,
  numeroDecimale,
  righe,
  secondiDaDataOra,
  veroPython,
  type ValoreJson,
} from "../src/python.js";
import { rilevaAccessi, senzaIndirizzi } from "../src/rilevatore/accessi.js";
import { verificaGoogle } from "../src/rilevatore/google.js";
import { problemaIngressi } from "../src/rilevatore/ingressi.js";
import { coda } from "../src/rilevatore/log.js";
import { accessiOutlook } from "../src/rilevatore/outlook.js";
import { ErroreTelefono } from "../src/telefono.js";
import { MAIL_ACCOUNTS, TelefonoFinto, contesto, creaCartelle, creaDb, rigaCasella, rigaPush, scrivi, type Cartelle } from "./aiuti.js";

const ADESSO = 1_760_000_000;
const fa = (s: number): number => ADESSO - s;

describe("le regole di Python usate dal rilevatore", () => {
  test("str.splitlines: separatori, riga finale vuota", () => {
    expect(righe("a\nb\r\nc\rd\u2028e\x85f\x0bg\x0ch\x1ci")).toEqual(["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
    expect(righe("a\n")).toEqual(["a"]);
    expect(righe("a\n\n")).toEqual(["a", ""]);
    expect(righe("")).toEqual([]);
    expect(righe("\n")).toEqual([""]);
  });

  test("bool() dei valori di un JSON", () => {
    for (const v of [null, false, 0, "", [], {}]) expect(veroPython(v)).toBe(false);
    for (const v of [true, 1, "no", [0], { a: 1 }, Number.NaN, Number.POSITIVE_INFINITY]) expect(veroPython(v)).toBe(true);
  });

  test("json.loads accetta NaN e Infinity, ma solo dove va un valore", () => {
    expect(leggiJsonPython('{"a": 1}')).toEqual({ a: 1 });
    expect(leggiJsonPython('{"a": NaN, "b": [Infinity, -Infinity]}')).toEqual({ a: Number.NaN, b: [Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY] });
    expect(leggiJsonPython('{"a": "NaN, Infinity"}')).toEqual({ a: "NaN, Infinity" });
    expect(() => leggiJsonPython('{NaN: 1}')).toThrow();
    expect(() => leggiJsonPython('{"a": NaN')).toThrow();
    expect(() => leggiJsonPython("")).toThrow();
  });

  test("int() di un numero non esiste per NaN e per l'infinito, e tronca verso lo zero", () => {
    expect(intero(2.9)).toBe(2);
    expect(intero(-2.9)).toBe(-2);
    expect(() => intero(Number.NaN)).toThrowError(ErroreValore);
    expect(() => intero(Number.POSITIVE_INFINITY)).toThrowError(ErroreValore);
  });

  test("strptime: anni sotto il 100, date che non esistono, ore fuori misura", () => {
    expect(secondiDaDataOra("1970-01-01 00:00:00")).toBe(0);
    expect(secondiDaDataOra("0001-01-01 00:00:00")).toBe(-62135596800);
    expect(secondiDaDataOra("2024-02-29 12:00:00")).toBe(Date.UTC(2024, 1, 29, 12) / 1000);
    expect(() => secondiDaDataOra("2025-02-29 12:00:00")).toThrowError(ErroreValore);
    expect(() => secondiDaDataOra("2026-10-11 24:00:00")).toThrowError(ErroreValore);
    expect(() => secondiDaDataOra("")).toThrowError(ErroreValore);
    expect(() => secondiDaDataOra("2026-10-11")).toThrowError(ErroreValore);
  });

  test("le date dei log di Outlook: fuso, frazione, formati che Python rifiuta", () => {
    expect(epocaDaLog("1970-01-01T00:00:00.000+0000")).toBe(0);
    expect(epocaDaLog("1970-01-01T01:00:00.500+0100")).toBe(0.5);
    expect(epocaDaLog("1970-01-01T00:00:00.5Z")).toBe(0.5);
    expect(epocaDaLog("0001-01-01T00:00:00.000+0000")).toBe(-62135596800);
    expect(epocaDaLog("2026-10-11T05:03:21.996+0000")).toBe(Date.UTC(2026, 9, 11, 5, 3, 21) / 1000 + 0.996);
    expect(epocaDaLog("2026-10-11T05:03:21.9960000+0000")).toBeNull();
    expect(epocaDaLog("2026-10-11T05:03:21+0000")).toBeNull();
    expect(epocaDaLog("2026-10-11T05:03:21.996")).toBeNull();
    expect(epocaDaLog("2026-13-11T05:03:21.996+0000")).toBeNull();
    expect(epocaDaLog("2026-10-11T24:03:21.996+0000")).toBeNull();
    expect(epocaDaLog("2026-10-11T05:03:21.996+2400")).toBeNull();
  });

  test("base64.b64decode(s, altchars=b'-_'): caratteri fuori alfabeto saltati, riempimento, lunghezze impossibili", () => {
    const testo = (s: string): string => Buffer.from(decodificaBase64(s)).toString("latin1");
    expect(testo("QUJD")).toBe("ABC");
    expect(testo("QUI=")).toBe("AB");
    expect(() => testo("QUI")).toThrowError(ErroreValore);
    expect(testo("Q U\nJD")).toBe("ABC");
    expect(testo("Pz8-")).toBe("??>");
    expect(testo("QQ==QUJD")).toBe("A");
    expect(() => testo("A")).toThrowError(ErroreValore);
    expect(() => testo("é")).toThrowError(ErroreValore);
    expect(testo("")).toBe("");
  });

  test("float(): numeri, booleani e testi, con gli errori di Python", () => {
    expect(comeNumero(" 1.5 ")).toBe(1.5);
    expect(comeNumero("1e3")).toBe(1000);
    expect(comeNumero(true)).toBe(1);
    expect(() => comeNumero("abc")).toThrowError(ErroreValore);
    expect(() => comeNumero(null)).toThrow();
    expect(() => comeNumero([1])).toThrow();
  });

  test("json.dumps: separatori, escape, non ASCII e numeri con la frazione", () => {
    const v = new Map<string, ValoreJson>([
      ["a", "è \"x\"\n"],
      ["b", new Map<string, ValoreJson>([["n", 3]])],
      ["c", [true, null]],
    ]);
    expect(jsonPython(v)).toBe('{"a": "\\u00e8 \\"x\\"\\n", "b": {"n": 3}, "c": [true, null]}');
    expect(numeroDecimale(1760000000)).toBe("1760000000.0");
    expect(numeroDecimale(1760000000.25)).toBe("1760000000.25");
  });
});

let c: Cartelle;
beforeEach(() => {
  c?.pulisci();
  c = creaCartelle();
});
afterAll(() => c?.pulisci());

const nuovoContesto = (telefono = new TelefonoFinto()) => contesto(c, telefono, new OrologioFinto(ADESSO * 1000));

describe("la coda dei log", () => {
  test("le ultime 600000 byte senza la prima riga se il file è più grande, tutto il file se no, e il precedente prima", async () => {
    const file = path.join(c.log, "x.log");
    writeFileSync(file, "uno\ndue\ntre\n");
    expect(await coda(file)).toBe("uno\ndue\ntre\n");
    expect(await coda(file, 8)).toBe("tre\n");
    expect(await coda(file, 11)).toBe("due\ntre\n");
    expect(await coda(file, 12)).toBe("uno\ndue\ntre\n");
    writeFileSync(file + ".1", "vecchio\n");
    expect(await coda(file)).toBe("vecchio\n\nuno\ndue\ntre\n");
    writeFileSync(file, "");
    expect(await coda(file)).toBe("vecchio\n\n");
    expect(await coda(path.join(c.log, "assente.log"))).toBeNull();
  });
});

describe("accessi di Outlook: la bandierina needsReauth", () => {
  test("una casella con il login da rifare è rossa, le altre ok, e il resto del token non si legge", async () => {
    const accountId = Buffer.from("xx" + "roar@example.com" + "yy").toString("base64url");
    creaDb(c.db, [
      MAIL_ACCOUNTS,
      "create table tokenStore (accountId text, needsReauth integer, resource text)",
      `insert into tokenStore values ('${accountId}', 1, 'https://outlook.office365.com'), ('${Buffer.from("band@example.com").toString("base64")}', 1, 'https://adnotifications.windowsazure.com/x')`,
    ]);
    const v = await accessiOutlook(nuovoContesto());
    expect(v.get("roar@example.com")).toEqual({ stato: "errore", msg: "Accesso scaduto sul telefono: login da rifare in Outlook" });
    expect(v.get("band@example.com")).toEqual({ stato: "ok", msg: "" });
  });

  test("senza database nessun verdetto", async () => {
    expect((await accessiOutlook(nuovoContesto())).size).toBe(0);
  });
});

describe("rilevaAccessi, il main di rileva_accessi.py", () => {
  test("scrive accessi.json come json.dump e dice quante caselle e quali sistemare", async () => {
    creaDb(c.db, [MAIL_ACCOUNTS]);
    scrivi(c.log, "calendarSync.log", [rigaCasella(fa(30), 32768), rigaCasella(fa(3600), 32772)]);
    scrivi(c.log, "notifications.log", [rigaPush(fa(20))]);
    writeFileSync(c.battito, '{"outlook_davanti": true, "bloccato": false}');
    const telefono = new TelefonoFinto();
    const riepilogo = await rilevaAccessi(nuovoContesto(telefono));
    expect(riepilogo).toBe("accessi: 2 caselle esaminate, da sistemare: band@example.com (errore)");
    expect(readFileSync(c.uscita, "utf8")).toBe(
      '{"generato": 1760000000, "caselle": {"roar@example.com": {"stato": "ok", "msg": ""}, "band@example.com": {"stato": "errore", ' +
        '"msg": "Outlook non conferma la sincronizzazione della casella da 60 min: Outlook fermo o sospeso sul telefono"}}}',
    );
    expect(telefono.letture).toEqual(["dumpsys content", "date +%Y-%m-%d %H:%M:%S"]);
  });

  test("il riepilogo per il log non ha gli indirizzi delle caselle", () => {
    expect(senzaIndirizzi("accessi: 5 caselle esaminate, da sistemare: a@example.com (errore), b@example.com (avviso)")).toBe("accessi: 5 caselle esaminate, da sistemare: 2");
    expect(senzaIndirizzi("accessi: 5 caselle esaminate, da sistemare: nessuna")).toBe("accessi: 5 caselle esaminate, da sistemare: nessuna");
    expect(senzaIndirizzi("accessi: 1 caselle esaminate, da sistemare: a@example.com (avviso)")).toBe("accessi: 1 caselle esaminate, da sistemare: 1");
  });

  test("un controllo che si rompe non ferma gli altri e finisce nel log", async () => {
    writeFileSync(c.db, "non è un database");
    scrivi(c.log, "calendarSync.log", [rigaCasella(fa(30), 32768)]);
    const dettagli: string[] = [];
    const ctx = { ...nuovoContesto(), log: (m: string) => dettagli.push(m) };
    ctx.telefono = new TelefonoFinto();
    const riepilogo = await rilevaAccessi(ctx);
    expect(riepilogo).toBe("accessi: 0 caselle esaminate, da sistemare: nessuna");
    expect(dettagli.map((d) => d.replace(/\(.*\)$/, "()"))).toEqual(["rilevatore: accessi Outlook non riuscito ()", "rilevatore: freschezza Outlook non riuscito ()"]);
    expect(JSON.parse(readFileSync(c.uscita, "utf8"))).toEqual({ generato: ADESSO, caselle: {} });
  });
});

describe("calendari Google dal telefono", () => {
  const DUMP = (esito: string, ora: string): string =>
    `Account g@example.com u0 com.google\ncom.android.calendar 1 true Total 1 ${esito}\n   Today 173 ${ora}\n`;

  test("legge dumpsys content e l'ora dal telefono (25 e 10 secondi di attesa), scrive il ricordo e lo rilegge", async () => {
    const telefono = new TelefonoFinto();
    const opzioni: (number | undefined)[] = [];
    telefono.risposta = (comando, o) => {
      opzioni.push(o?.timeoutMs);
      return comando[0] === "date" ? "2026-10-11 09:00:00\n" : DUMP("OTHER SUCCESS", "2026-10-11 08:59:00");
    };
    const v = await verificaGoogle(nuovoContesto(telefono), ADESSO);
    expect(v.get("g@example.com")).toEqual({ stato: "ok", msg: "" });
    expect(opzioni).toEqual([25_000, 10_000]);
    expect(readFileSync(c.googleStato, "utf8")).toBe(`{"g@example.com": ${ADESSO - 60}.0}`);
    telefono.risposta = (comando) => (comando[0] === "date" ? "2026-10-11 09:00:00\n" : DUMP("OTHER FAILURE", "2026-10-11 08:59:00"));
    expect((await verificaGoogle(nuovoContesto(telefono), ADESSO + 100)).get("g@example.com")?.stato).toBe("ok");
    expect(readFileSync(c.googleStato, "utf8")).toBe(`{"g@example.com": ${ADESSO - 60}.0}`);
  });

  test("telefono che non risponde, ora illeggibile o niente dal telefono: nessun verdetto e nessun file", async () => {
    const telefono = new TelefonoFinto();
    telefono.risposta = () => {
      throw new ErroreTelefono("nessuna risposta in 25 s", "tempo");
    };
    expect((await verificaGoogle(nuovoContesto(telefono), ADESSO)).size).toBe(0);
    telefono.risposta = (comando) => (comando[0] === "date" ? "non una data" : DUMP("OTHER SUCCESS", "2026-10-11 08:59:00"));
    expect((await verificaGoogle(nuovoContesto(telefono), ADESSO)).size).toBe(0);
    telefono.risposta = () => "";
    expect((await verificaGoogle(nuovoContesto(telefono), ADESSO)).size).toBe(0);
    expect(() => readFileSync(c.googleStato)).toThrow();
  });
});

describe("perché il rilevatore non legge i dati di Outlook", () => {
  test("niente da dire se si leggono, il motivo se mancano o non si leggono", async () => {
    expect(await problemaIngressi(c.dati)).toMatch(/^dati di Outlook: database non c'è, calendarSync.log non c'è$/);
    creaDb(c.db, [MAIL_ACCOUNTS]);
    scrivi(c.log, "calendarSync.log", [rigaCasella(fa(30), 32768)]);
    expect(await problemaIngressi(c.dati)).toBeNull();
    mkdirSync(path.join(c.dati, "altro"), { recursive: true });
    if (process.platform !== "win32" && process.getuid?.() !== 0) {
      chmodSync(path.join(c.dati, "data/com.microsoft.office.outlook"), 0o000);
      expect(await problemaIngressi(c.dati)).toMatch(/non si legge \(EACCES, utente \d+\)/);
      chmodSync(path.join(c.dati, "data/com.microsoft.office.outlook"), 0o700);
    }
  });
});
