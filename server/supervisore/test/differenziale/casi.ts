// Dati inventati per il confronto fra rileva_accessi.py e il rilevatore in TypeScript: nessun dato vero. Un caso è una cartella con quello
// che il rilevatore legge (dati/data/com.microsoft.office.outlook/{app_logs,databases}, stato/{battito.json,google-sync.json}), le risposte
// del telefono (dumpsys-content.txt, date.txt), l'ora (adesso) e le variabili d'ambiente (env.json).
import { copyFileSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { DatabaseSync, type SQLInputValue } from "node:sqlite";

export const ADESSO = 1760200000.5;

type Contenuto = string | Buffer;
type Riga = SQLInputValue[];

interface Opzioni {
  logs?: Record<string, Contenuto | null>;
  /** Come si riempie il database; null = nessun database, senza questa chiave il database di partenza. */
  db?: ((file: string) => void) | null;
  /** Il testo di battito.json; null = il file non c'è. */
  battito?: string | null;
  ricordo?: string | null;
  dumpsys?: Contenuto | null;
  data?: string | null;
  env?: Record<string, string>;
}

interface OpzioniDb {
  mail?: Riga[];
  token?: Riga[];
  tipi?: boolean;
}

interface OpzioniData {
  frazione?: number;
  fuso?: string;
}

const due = (n: number, c = 2): string => String(n).padStart(c, "0");

function stamp(secondi: number, { frazione = 3, fuso = "+0000" }: OpzioniData = {}): string {
  const t = new Date(Math.round(secondi * 1000));
  const f = due(t.getUTCMilliseconds(), 3).padEnd(frazione, "0").slice(0, frazione);
  return `${t.getUTCFullYear()}-${due(t.getUTCMonth() + 1)}-${due(t.getUTCDate())}T${due(t.getUTCHours())}:${due(t.getUTCMinutes())}:${due(t.getUTCSeconds())}.${f}${fuso}`;
}

const fa = (s: number): number => ADESSO - s;

const casella = (s: number, id: number, sincronizza = "true", opz?: OpzioniData): string =>
  `D\t${stamp(fa(s), opz)}\t[ci=x]\tSyncAdapterThread\toutlook.calendarSync > CalendarSyncAccou\tisSyncingForAccount: accountId=${id}, hxAccountIsSyncing=${sincronizza}, calendarSyncAccount!=null=true, isSyncable>0=true => isSyncing=${sincronizza}\n`;

const casellaConTesto = (testo: string, id: number): string =>
  `D\t${stamp(fa(30))}\t[ci=x]\tSyncAdapterThread\toutlook.calendarSync > CalendarSyncAccou\t${testo}isSyncingForAccount: accountId=${id}, hxAccountIsSyncing=true, calendarSyncAccount!=null=true, isSyncable>0=true => isSyncing=true\n`;

const push = (s: number, disponibile = true, opz?: OpzioniData): string =>
  `I\t${stamp(fa(s), opz)}\t[ci=x]\tOM_APP_SESSION_MA\toutlook.Notifications > FcmTokenReaderWr\tFCM push sync is ${disponibile ? "available" : "unavailable due to missing token"}\n`;

const rumore = (n: number, s0 = 4000): string =>
  Array.from({ length: n }, (_, i) => `D\t${stamp(fa(s0 + i))}\t[ci=x]\tMain\toutlook.other > Thing\triga di altro tipo numero ${i} ${"x".repeat(60)}\n`).join("");

const base64 = (t: string, { url = false, pad = true } = {}): string => {
  let s = Buffer.from(t, "latin1").toString("base64");
  if (url) s = s.replaceAll("+", "-").replaceAll("/", "_");
  return pad ? s : s.replace(/=+$/, "");
};

const MAIL: Riga[] = [
  [32768, "roar_Hx", "Roar@example.com"],
  [32769, null, "roar@example.com"],
  [32772, "band_Hx", "band@example.com"],
  [32773, null, "band@example.com"],
];

const DUMP = (esito: string, ora: string, conto = "g@example.com"): string => `Sync Status
Account utente@example.com u0 com.microsoft.office.outlook.USER_ACCOUNT
=======================================================================
com.android.calendar  1         true     Total  0    0     4466  4     0     8     4478  0     0    1m14s  PERIODIC SUCCESS
                                         Today             685                     685              10s    2026-10-11 07:09:39
Account ${conto} u0 com.google
=======================================================================
com.android.calendar                     1         true     Total  1    0     242  3     0     478   724  1     1    20m39s  ${esito}
                                                            Today             173              346   519             14m51s  ${ora}
                                                            Yestr  1          69   3           132   205  1     1    5m47s
com.android.contacts                     -1        false    Total  0    0     0    0     0     1     1    1     0    0s      OTHER FAILURE
                                                            Today                                                    0s      2026-10-10 21:22:56
                                                            Yestr                              1     1    1          0s      internal-error
`;

const TELEFONO_ORA = "2026-10-11 09:00:00";

const ORA_FA = (min: number): string => {
  const d = new Date(Date.UTC(2026, 9, 11, 9, 0, 0) - min * 60000);
  return `${d.getUTCFullYear()}-${due(d.getUTCMonth() + 1)}-${due(d.getUTCDate())} ${due(d.getUTCHours())}:${due(d.getUTCMinutes())}:${due(d.getUTCSeconds())}`;
};

function creaDb(file: string, { mail = MAIL, token = [], tipi = true }: OpzioniDb = {}): void {
  const db = new DatabaseSync(file);
  db.exec(`create table mailAccounts (accountID ${tipi ? "integer" : ""}, hxAccountId ${tipi ? "text" : ""}, email ${tipi ? "text" : ""})`);
  db.exec(`create table tokenStore (accountId ${tipi ? "text" : ""}, needsReauth ${tipi ? "integer" : ""}, resource ${tipi ? "text" : ""})`);
  const m = db.prepare("insert into mailAccounts values (?, ?, ?)");
  for (const r of mail) m.run(...r);
  const t = db.prepare("insert into tokenStore values (?, ?, ?)");
  for (const r of token) t.run(...r);
  db.close();
}

const conDb =
  (opz: OpzioniDb) =>
  (file: string): void =>
    creaDb(file, opz);

const tok = (acc: SQLInputValue, nr: SQLInputValue = 1, res: SQLInputValue = "https://outlook.office365.com"): Riga => [acc, nr, res];

/** Scrive tutti i casi in `uscita` (svuotata prima) e restituisce quanti sono. */
export function generaCasi(uscita: string): number {
  rmSync(uscita, { recursive: true, force: true });
  let n = 0;
  /** Crea la cartella di un caso. */
  function caso(nome: string, o: Opzioni = {}): void {
    n += 1;
    const dir = path.join(uscita, nome);
    const log = path.join(dir, "dati/data/com.microsoft.office.outlook/app_logs");
    const dbs = path.join(dir, "dati/data/com.microsoft.office.outlook/databases");
    mkdirSync(log, { recursive: true });
    mkdirSync(dbs, { recursive: true });
    mkdirSync(path.join(dir, "stato"), { recursive: true });
    const logs: Record<string, Contenuto | null> = { "calendarSync.log": "", "notifications.log": "", ...(o.logs ?? {}) };
    for (const [k, v] of Object.entries(logs)) if (v !== null) writeFileSync(path.join(log, k), v);
    const riempi = o.db === undefined ? creaDb : o.db;
    if (riempi) riempi(path.join(dbs, "acompliAcct.db"));
    const battito = o.battito === undefined ? '{"quando": 1760199990, "pausa": false, "telefono": true, "outlook_davanti": true, "bloccato": false}' : o.battito;
    if (battito !== null) writeFileSync(path.join(dir, "stato/battito.json"), battito);
    if (o.ricordo !== undefined && o.ricordo !== null) writeFileSync(path.join(dir, "stato/google-sync.json"), o.ricordo);
    if (o.dumpsys !== undefined && o.dumpsys !== null) writeFileSync(path.join(dir, "dumpsys-content.txt"), o.dumpsys);
    if (o.data !== null) writeFileSync(path.join(dir, "date.txt"), o.data ?? TELEFONO_ORA + "\n");
    writeFileSync(path.join(dir, "adesso"), String(ADESSO));
    writeFileSync(path.join(dir, "env.json"), JSON.stringify(o.env ?? {}));
  }
  const VIVI = [casella(60, 32768), casella(55, 32772), casella(30, 32768), casella(25, 32772)].join("");
  const OK = { "calendarSync.log": VIVI, "notifications.log": push(20) };
  
  // rotazione e finestra del log
  caso("log-rotazione-nuovo-vuoto", { logs: { "calendarSync.log": "", "calendarSync.log.1": casella(40, 32768) + casella(40, 32772), "notifications.log": "", "notifications.log.1": push(100, false) } });
  caso("log-rotazione-nuovo-una-riga", { logs: { "calendarSync.log": casella(5, 32768), "calendarSync.log.1": casella(40, 32768) + casella(40, 32772) } });
  caso("log-solo-corrente", { logs: OK });
  caso("log-solo-precedente", { logs: { "calendarSync.log": null, "calendarSync.log.1": VIVI } });
  caso("log-niente", { logs: { "calendarSync.log": null, "notifications.log": null } });
  caso("log-notifiche-assenti", { logs: { "calendarSync.log": VIVI, "notifications.log": null } });
  function grande(byte: number, primaRiga: string, vive: string): string {
    const riga = (len: number): string => "k".repeat(len - 1) + "\n";
    let resto = byte - Buffer.byteLength(primaRiga) - Buffer.byteLength(vive);
    let mezzo = "";
    while (resto > 400) {
      mezzo += riga(200);
      resto -= 200;
    }
    mezzo += riga(resto);
    const t = primaRiga + mezzo + vive;
    if (Buffer.byteLength(t) !== byte) throw new Error("dimensione sbagliata " + Buffer.byteLength(t) + " invece di " + byte);
    return t;
  }
  const VIVE = casella(30, 32768) + casella(29, 32768);
  for (const byte of [599999, 600000, 600001, 700000]) {
    caso(`log-grande-${byte}`, { logs: { "calendarSync.log": grande(byte, casella(50, 32772) + casella(100, 32768), VIVE), "notifications.log": push(20) } });
  }
  caso("log-grande-con-precedente", {
    logs: { "calendarSync.log": grande(650000, casella(50, 32772), VIVE), "calendarSync.log.1": grande(900000, casella(40, 32772), casella(45, 32772)), "notifications.log": grande(650000, push(30, false), push(25)), "notifications.log.1": push(400, false) },
  });
  caso("log-grande-solo-precedente-grande", { logs: { "calendarSync.log": VIVE, "calendarSync.log.1": grande(900000, casella(40, 32772), casella(45, 32772)), "notifications.log": push(20) } });
  caso("log-grande-senza-capo-nella-finestra", { logs: { "calendarSync.log": "y".repeat(700000) + "\n" + VIVE + casella(20, 32772), "notifications.log": push(20) } });
  caso("log-grande-una-sola-riga", { logs: { "calendarSync.log": "y".repeat(700000), "calendarSync.log.1": VIVE, "notifications.log": push(20) } });caso("log-crlf", { logs: { "calendarSync.log": VIVI.replaceAll("\n", "\r\n"), "notifications.log": push(20).replaceAll("\n", "\r\n") } });
  caso("log-senza-capo-finale", { logs: { "calendarSync.log": VIVI.replace(/\n$/, ""), "notifications.log": push(20).replace(/\n$/, "") } });
  caso("log-solo-cr", { logs: { "calendarSync.log": VIVI.replaceAll("\n", "\r"), "notifications.log": push(20) } });
  for (const [nome, sep] of [["u2028", "\u2028"], ["vt", "\v"], ["ff", "\f"], ["fs", "\x1c"], ["nel", "\x85"], ["u2029", "\u2029"]]) {
    caso(`log-separatore-${nome}`, { logs: { "calendarSync.log": casellaConTesto(`messaggio${sep}con separatore `, 32768) + casella(30, 32772), "notifications.log": push(20) } });
  }
  caso("log-utf8-invalido", {
    logs: { "calendarSync.log": Buffer.concat([Buffer.from("D\t" + stamp(fa(30)) + "\t[ci=x]\tSyncAdapterThread\tx\t"), Buffer.from([0xff, 0xfe, 0xc3]), Buffer.from(` isSyncingForAccount: accountId=32768, hxAccountIsSyncing=true, y=>z => isSyncing=true\n${casella(30, 32772)}`)]), "notifications.log": push(20) },
  });
  caso("log-utf8-troncato-nella-finestra", {
    logs: { "calendarSync.log": Buffer.concat([Buffer.from("y".repeat(599990), "latin1"), Buffer.from("\n"), Buffer.from([0xe2, 0x82]), Buffer.from("\n" + VIVI)]), "notifications.log": push(20) },
  });
  caso("log-utf8-multibyte", { logs: { "calendarSync.log": casellaConTesto("caffè € 日本 ", 32768) + casella(30, 32772), "notifications.log": push(20) } });
  
  // date dei log
  const varianti: [string, string][] = [
    ["fuso-z", casella(30, 32768, "true", { fuso: "Z" })],
    ["fuso-piu-0100", casella(30, 32768, "true", { fuso: "+0100" })],
    ["fuso-meno-0530", casella(30, 32768, "true", { fuso: "-0530" })],
    ["fuso-due-punti", casella(30, 32768, "true", { fuso: "+01:00" })],
    ["fuso-secondi", casella(30, 32768, "true", { fuso: "+010203" })],
    ["fuso-secondi-frazione", casella(30, 32768, "true", { fuso: "+01:02:03.5" })],
    ["fuso-2400", casella(30, 32768, "true", { fuso: "+2400" })],
    ["fuso-2359", casella(30, 32768, "true", { fuso: "+2359" })],
    ["fuso-minuti-60", casella(30, 32768, "true", { fuso: "+0160" })],
    ["fuso-manca", casella(30, 32768).replace("+0000", "")],
    ["frazione-1", casella(30, 32768, "true", { frazione: 1 })],
    ["frazione-6", casella(30, 32768, "true", { frazione: 6 })],
    ["frazione-7", casella(30, 32768).replace(/\.(\d{3})\+0000/, ".$10000+0000")],
    ["frazione-0", casella(30, 32768).replace(/\.\d{3}\+0000/, ".+0000")],
    ["senza-frazione", casella(30, 32768).replace(/\.\d{3}\+0000/, "+0000")],
    ["mese-13", casella(30, 32768).replace(/-10-/, "-13-")],
    ["mese-00", casella(30, 32768).replace(/-10-/, "-00-")],
    ["giorno-32", casella(30, 32768).replace(/-1(\d)T/, "-32T")],
    ["ore-24", casella(30, 32768).replace(/T\d\d:/, "T24:")],
    ["secondi-60", casella(30, 32768).replace(/:\d\d\./, ":60.")],
    ["secondi-61", casella(30, 32768).replace(/:\d\d\./, ":61.")],
    ["bisestile-ok", casella(30, 32768).replace(/\d{4}-\d\d-\d\dT/, "2024-02-29T")],
    ["bisestile-no", casella(30, 32768).replace(/\d{4}-\d\d-\d\dT/, "2025-02-29T")],
    ["anno-0999", casella(30, 32768).replace(/^D\t\d{4}/, "D\t0999")],
    ["anno-0001", casella(30, 32768).replace(/^D\t\d{4}-\d\d-\d\d/, "D\t0001-01-01")],
    ["anno-9999", casella(30, 32768).replace(/^D\t\d{4}-\d\d-\d\d/, "D\t9999-12-31")],
    ["mese-una-cifra", casella(30, 32768).replace(/-10-/, "-1-")],
    ["data-futura", casella(-3600, 32768)],
    ["data-lontana", casella(86400 * 40, 32768)],
    ["spazio-iniziale", " " + casella(30, 32768)],
    ["livello-minuscolo", casella(30, 32768).replace(/^D/, "d")],
    ["livello-due-lettere", casella(30, 32768).replace(/^D/, "DD")],
    ["tab-doppio", casella(30, 32768).replace("\t[ci=x]", "\t\t[ci=x]")],
    ["id-zero-iniziale", casella(30, 32768).replace("accountId=32768", "accountId=032768")],
    ["id-grande", casella(30, 32768).replace("accountId=32768", "accountId=99999999999999999999")],
    ["sync-maiuscolo", casella(30, 32768).replaceAll("=true", "=TRUE")],
    ["sync-nome-diverso", casella(30, 32768).replace("hxAccountIsSyncing", "hxAccountSyncing")],
    ["sync-prima-di-hx", casella(30, 32768).replace("hxAccountIsSyncing=true", "hxAccountIsSyncing=")],
  ];
  for (const [nome, riga] of varianti) caso(`data-${nome}`, { logs: { "calendarSync.log": riga + casella(30, 32772), "notifications.log": push(20) } });
  caso("data-fuso-in-notifiche", { logs: { "calendarSync.log": VIVI, "notifications.log": push(20, false, { fuso: "+0100" }) } });
  caso("data-notifiche-fuso-z", { logs: { "calendarSync.log": VIVI, "notifications.log": push(20, false, { fuso: "Z" }) } });
  caso("data-notifiche-rotte", { logs: { "calendarSync.log": VIVI, "notifications.log": push(20, false).replace(/-10-/, "-13-") } });
  
  // soglie e giudizi
  const mute: [string, string[]][] = [
    ["muta-599", [casella(599, 32772), casella(30, 32768)]],
    ["muta-600", [casella(600, 32772), casella(30, 32768)]],
    ["muta-601", [casella(601, 32772), casella(30, 32768)]],
    ["muta-1799", [casella(1799, 32772), casella(30, 32768)]],
    ["muta-1800", [casella(1800, 32772), casella(30, 32768)]],
    ["muta-1801", [casella(1801, 32772), casella(30, 32768)]],
    ["muta-3600", [casella(3600, 32772), casella(30, 32768)]],
    ["muta-un-giorno", [casella(86400, 32772), casella(30, 32768)]],
    ["sospesa", [casella(30, 32772, "false"), casella(30, 32768)]],
    ["sospesa-e-vecchia", [casella(5000, 32772, "false"), casella(30, 32768)]],
    ["riga-ultima-vince", [casella(5000, 32772, "false"), casella(30, 32772), casella(30, 32768)]],
    ["riga-ultima-vince-peggio", [casella(30, 32772), casella(5000, 32772), casella(30, 32768)]],
    ["entrambe-mute", [casella(5000, 32772), casella(2000, 32768)]],
    ["nessuna-riga", [rumore(5)]],
  ];
  const battiti: [string, string][] = [
    ["davanti", '{"quando": 1760199990, "pausa": false, "telefono": true, "outlook_davanti": true, "bloccato": false}'],
    ["dietro", '{"quando": 1760199990, "pausa": false, "telefono": true, "outlook_davanti": false, "bloccato": false}'],
    ["bloccato", '{"quando": 1760199990, "pausa": false, "telefono": true, "outlook_davanti": true, "bloccato": true}'],
  ];
  for (const [nome, righe] of mute) {
    for (const [bn, bat] of battiti) {
      caso(`soglia-${nome}-${bn}`, { logs: { "calendarSync.log": righe.join(""), "notifications.log": push(20, false) }, battito: bat });
    }
  }  const notifiche: [string, string][] = [
    ["disponibili-recenti", push(20)],
    ["non-disponibili-599", push(599, false)],
    ["non-disponibili-600", push(600, false)],
    ["non-disponibili-601", push(601, false)],
    ["ultima-vince", push(100, false) + push(20)],
    ["ultima-vince-negativa", push(20) + push(100, false)],
    ["nel-futuro", push(-500, false)],
  ];
  for (const [nome, righe] of notifiche) {
    caso(`push-${nome}-dietro`, { logs: { "calendarSync.log": VIVI, "notifications.log": righe }, battito: '{"outlook_davanti": false}' });
  }
  caso("env-soglie-strette", { logs: { "calendarSync.log": [casella(70, 32772), casella(30, 32768)].join(""), "notifications.log": push(20) }, env: { OUTLOOK_GIALLO_S: "60", OUTLOOK_ROSSO_S: "120" } });
  caso("env-soglie-larghe", { logs: { "calendarSync.log": [casella(5000, 32772), casella(30, 32768)].join(""), "notifications.log": push(20) }, env: { OUTLOOK_GIALLO_S: "6000", OUTLOOK_ROSSO_S: "9000" } });
  caso("env-soglie-rosso-sotto-giallo", { logs: { "calendarSync.log": [casella(700, 32772), casella(30, 32768)].join(""), "notifications.log": push(20) }, env: { OUTLOOK_GIALLO_S: "900", OUTLOOK_ROSSO_S: "300" } });
  caso("env-soglie-zero", { logs: { "calendarSync.log": [casella(1, 32772), casella(30, 32768)].join(""), "notifications.log": push(20) }, env: { OUTLOOK_GIALLO_S: "0", OUTLOOK_ROSSO_S: "0" } });
  
  // battito.json
  for (const [nome, bat] of [
    ["assente", null],
    ["vuoto", ""],
    ["json-rotto", '{"outlook_davanti": tru'],
    ["lista", "[]"],
    ["lista-piena", "[1]"],
    ["stringa", '"x"'],
    ["numero", "5"],
    ["null", "null"],
    ["oggetto-vuoto", "{}"],
    ["davanti-null", '{"outlook_davanti": null}'],
    ["davanti-zero", '{"outlook_davanti": 0}'],
    ["davanti-uno", '{"outlook_davanti": 1}'],
    ["davanti-stringa-vuota", '{"outlook_davanti": ""}'],
    ["davanti-stringa-no", '{"outlook_davanti": "no"}'],
    ["davanti-lista-vuota", '{"outlook_davanti": []}'],
    ["davanti-oggetto-vuoto", '{"outlook_davanti": {}}'],
    ["davanti-falso-bloccato-uno", '{"outlook_davanti": false, "bloccato": 1}'],
    ["bloccato-stringa", '{"bloccato": "no"}'],
    ["bloccato-zero-virgola", '{"bloccato": 0.0}'],
    ["bloccato-lista", '{"bloccato": [0]}'],
    ["bloccato-null", '{"bloccato": null}'],
    ["bom", '\ufeff{"outlook_davanti": false, "bloccato": true}'],
    ["nan", '{"bloccato": NaN}'],
    ["infinito", '{"bloccato": Infinity, "outlook_davanti": false}'],
    ["chiave-doppia", '{"bloccato": true, "bloccato": false}'],
    ["annidato", '{"outlook_davanti": {"a": 1}}'],
    ["maiuscole", '{"Bloccato": true, "OUTLOOK_DAVANTI": false}'],
    ["righe-in-piu", '{"outlook_davanti": false}\n\n'],
    ["testo-dopo", '{"outlook_davanti": false} x'],
  ]) {
    caso(`battito-${nome}`, { logs: { "calendarSync.log": VIVI, "notifications.log": push(20, false) }, battito: bat });
  }
  
  // database
  caso("db-assente", { logs: OK, db: null });
  caso("db-vuoto-senza-tabelle", { logs: OK, db: (f) => new DatabaseSync(f).close() });
  caso("db-senza-token", { logs: OK, db: (f) => { const d = new DatabaseSync(f); d.exec("create table mailAccounts (accountID integer, hxAccountId text, email text); insert into mailAccounts values (32768,'h','a@example.com')"); d.close(); } });
  caso("db-senza-mailaccounts", { logs: OK, db: (f) => { const d = new DatabaseSync(f); d.exec("create table tokenStore (accountId text, needsReauth integer, resource text)"); d.close(); } });
  caso("db-non-un-db", { logs: OK, db: (f) => writeFileSync(f, "questo non è un database sqlite ".repeat(40)) });
  caso("db-file-vuoto", { logs: OK, db: (f) => writeFileSync(f, "") });
  caso("db-solo-hx-nullo", { logs: OK, db: conDb({ mail: [[1, null, "a@example.com"], [2, null, "b@example.com"]] }) });
  caso("db-email-nulle", { logs: OK, db: conDb({ mail: [[32768, "h", null], [32772, "h2", "band@example.com"], [5, null, null]] }) });
  caso("db-id-testo", { logs: { "calendarSync.log": casella(30, 123) + casella(30, 32772), "notifications.log": push(20) }, db: conDb({ mail: [["00123", "h", "t@example.com"], ["32772", "h2", "band@example.com"]], tipi: false }) });
  caso("db-id-reale", { logs: { "calendarSync.log": casella(30, 3) + casella(30, 32772), "notifications.log": push(20) }, db: conDb({ mail: [[3.5, "h", "t@example.com"], [32772, "h2", "band@example.com"]], tipi: false }) });
  caso("db-id-non-numerico", { logs: OK, db: conDb({ mail: [["abc", "h", "t@example.com"], [32772, "h2", "band@example.com"]], tipi: false }) });
  caso("db-id-nullo", { logs: OK, db: conDb({ mail: [[null, "h", "t@example.com"], [32772, "h2", "band@example.com"]], tipi: false }) });
  caso("db-id-duplicato", { logs: OK, db: conDb({ mail: [[32768, "h", "roar@example.com"], [32768, "h", "altro@example.com"], [32772, "h2", "band@example.com"]] }) });
  caso("db-stessa-email-due-id", { logs: { "calendarSync.log": casella(30, 1) + casella(5000, 2), "notifications.log": push(20) }, db: conDb({ mail: [[1, "h", "t@example.com"], [2, "h2", "T@example.com"]] }) });
  caso("db-email-maiuscole-non-ascii", { logs: OK, db: conDb({ mail: [[32768, "h", "ÀRoar@Example.COM"], [32772, "h2", "ÉBand@example.com"]] }) });
  caso("db-email-con-spazi", { logs: OK, db: conDb({ mail: [[32768, "h", " roar@example.com "], [32772, "h2", "band@example.com"]] }) });
  caso("db-molte-caselle", { logs: { "calendarSync.log": Array.from({ length: 40 }, (_, i) => casella(30 + i * 40, 1000 + i)).join(""), "notifications.log": push(20) }, db: conDb({ mail: Array.from({ length: 40 }, (_, i) => [1000 + i, `h${i}`, `u${i}@example.com`]) }) });
  
  // needsReauth
  const E1 = "roar@example.com";
  const tokenCases: [string, Riga[]][] = [
    ["base64-standard", [tok(base64(`xx${E1}yy`))]],
    ["base64-url", [tok(base64(`???>>>${E1}~~~~`, { url: true }))]],
    ["base64-senza-padding", [tok(base64(`a${E1}`, { pad: false }))]],
    ["base64-con-padding-doppio", [tok(base64(`ab${E1}`) + "=")]],
    ["base64-maiuscolo", [tok(base64(`xx${E1.toUpperCase()}yy`))]],
    ["base64-con-spazi", [tok(base64(`xx${E1}yy`).replace(/^(.{8})/, "$1 \n\t"))]],
    ["base64-con-punti", [tok(base64(`xx${E1}yy`).replace(/^(.{8})/, "$1.~"))]],
    ["base64-lunghezza-impossibile", [tok("A".repeat(9))]],
    ["base64-uno-solo", [tok("A")]],
    ["base64-solo-uguale", [tok("====")]],
    ["base64-uguale-in-mezzo", [tok("QUJD=REVG")]],
    ["base64-uguale-dopo-due", [tok("QQ==QUJD")]],
    ["base64-uguale-singolo", [tok("QQ=")]],
    ["base64-vuoto", [tok("")]],
    ["base64-non-ascii", [tok(`é${base64(E1)}`)]],
    ["base64-lungo", [tok(base64(`${"x".repeat(3000)}${E1}`))]],
    ["base64-mezzo-guasto", [tok("%%%%"), tok(base64(`xx${E1}yy`))]],
    ["reauth-zero", [tok(base64(`xx${E1}yy`), 0)]],
    ["reauth-nullo", [tok(base64(`xx${E1}yy`), null)]],
    ["reauth-testo", [tok(base64(`xx${E1}yy`), "1")]],
    ["reauth-testo-zero", [tok(base64(`xx${E1}yy`), "0")]],
    ["reauth-reale", [tok(base64(`xx${E1}yy`), 0.5)]],
    ["risorsa-mfa", [tok(base64(`xx${E1}yy`), 1, "https://adnotifications.windowsazure.com/x")]],
    ["risorsa-mfa-maiuscola", [tok(base64(`xx${E1}yy`), 1, "https://ADNOTIFICATIONS.WINDOWSAZURE.COM/x")]],
    ["risorsa-nulla", [tok(base64(`xx${E1}yy`), 1, null)]],
    ["risorsa-mfa-e-altra", [tok(base64(`xx${E1}yy`), 1, "https://adnotifications.windowsazure.com/x"), tok(base64("zz" + "band@example.com"), 1, "mail")]],
    ["due-caselle", [tok(base64(`xx${E1}yy`)), tok(base64("zzband@example.com"))]],
    ["account-nullo", [tok(null)]],
    ["account-numero", [tok(12345)]],
    ["account-blob", [[Buffer.from("QUJD"), 1, "x"]]],
    ["email-in-testo-corto", [tok(base64("band@example.comzzz"))]],
    ["email-sottostringa", [tok(base64("subroar@example.comtail"))]],
  ];
  for (const [nome, token] of tokenCases) caso(`token-${nome}`, { logs: OK, db: conDb({ token }), battito: '{"outlook_davanti": false, "bloccato": true}' });
  caso("token-senza-tipi", { logs: OK, db: conDb({ token: [tok(base64(`xx${E1}yy`), "0"), tok(base64("zzband@example.com"), "2")], tipi: false }) });
  caso("db-wal", {
    logs: OK,
    db: (f) => {
      const d = new DatabaseSync(f);
      d.exec("pragma journal_mode=wal; create table mailAccounts (accountID integer, hxAccountId text, email text); create table tokenStore (accountId text, needsReauth integer, resource text)");
      d.exec("checkpoint_placeholder".length ? "insert into mailAccounts values (32768,'h','roar@example.com'),(32772,'h2','band@example.com')" : "");
      d.exec(`insert into tokenStore values ('${base64(`xx${E1}yy`)}', 1, 'mail')`);
      copyFileSync(f, f + ".copia");
      copyFileSync(f + "-wal", f + ".walcopia");
      try { copyFileSync(f + "-shm", f + ".shmcopia"); } catch { /* nessun shm */ }
      d.close();
      rmSync(f, { force: true });
      rmSync(f + "-wal", { force: true });
      rmSync(f + "-shm", { force: true });
      copyFileSync(f + ".copia", f);
      copyFileSync(f + ".walcopia", f + "-wal");
      try { copyFileSync(f + ".shmcopia", f + "-shm"); } catch { /* nessun shm */ }
      rmSync(f + ".copia", { force: true });
      rmSync(f + ".walcopia", { force: true });
      rmSync(f + ".shmcopia", { force: true });
    },
  });
  
  // Google
  const googleCasi: [string, Contenuto, string | null][] = [
    ["successo-1-min", DUMP("OTHER SUCCESS", ORA_FA(1)), null],
    ["successo-14-min", DUMP("OTHER SUCCESS", ORA_FA(14)), null],
    ["successo-15-min", DUMP("OTHER SUCCESS", ORA_FA(15)), null],
    ["successo-16-min", DUMP("OTHER SUCCESS", ORA_FA(16)), null],
    ["successo-40-min", DUMP("OTHER SUCCESS", ORA_FA(40)), null],
    ["successo-10-ore", DUMP("OTHER SUCCESS", ORA_FA(600)), null],
    ["fallimento-nuovo", DUMP("OTHER FAILURE", ORA_FA(1)), null],
    ["fallimento-ricordo-2-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 120}}`],
    ["fallimento-ricordo-14-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 899}}`],
    ["fallimento-ricordo-15-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 900}}`],
    ["fallimento-ricordo-16-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 901}}`],
    ["fallimento-ricordo-44-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 2699}}`],
    ["fallimento-ricordo-45-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 2700}}`],
    ["fallimento-ricordo-46-min", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 2701}}`],
    ["fallimento-ricordo-2-ore", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${ADESSO - 7200}}`],
    ["successo-ricordo-piu-recente", DUMP("OTHER SUCCESS", ORA_FA(40)), `{"g@example.com": ${ADESSO - 10}}`],
    ["successo-ricordo-piu-vecchio", DUMP("OTHER SUCCESS", ORA_FA(2)), `{"g@example.com": ${ADESSO - 99999}}`],
    ["ricordo-stringa", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": "${ADESSO - 1000}"}`],
    ["ricordo-stringa-rotta", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": "abc"}`],
    ["ricordo-null", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": null}`],
    ["ricordo-lista", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": [1]}`],
    ["ricordo-booleano", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": true}`],
    ["ricordo-intero", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": ${Math.floor(ADESSO) - 1000}}`],
    ["ricordo-esponente", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": 1.7601e9}`],
    ["ricordo-altre-chiavi", DUMP("OTHER FAILURE", ORA_FA(1)), `{"altro@example.com": ${ADESSO - 5}, "g@example.com": ${ADESSO - 1000.25}, "zz": 0}`],
    ["ricordo-rotto", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": 1`],
    ["ricordo-lista-esterna", DUMP("OTHER FAILURE", ORA_FA(1)), `[1, 2]`],
    ["ricordo-vuoto", DUMP("OTHER FAILURE", ORA_FA(1)), ``],
    ["ricordo-negativo", DUMP("OTHER SUCCESS", ORA_FA(1)), `{"g@example.com": -5}`],
    ["ricordo-nan", DUMP("OTHER FAILURE", ORA_FA(1)), `{"g@example.com": NaN}`],
    ["email-maiuscola", DUMP("OTHER SUCCESS", ORA_FA(1), "G@Example.COM"), null],
    ["email-maiuscola-con-ricordo", DUMP("OTHER FAILURE", ORA_FA(1), "G@Example.COM"), `{"g@example.com": ${ADESSO - 1000}}`],
    ["dump-vuoto", "", null],
    ["dump-solo-righe-di-altro", "Sync Status\ncom.android.calendar 1 true Total OTHER SUCCESS\n   Today 2026-10-11 08:00:00\n", null],
    ["dump-conto-senza-calendario", "Account g@example.com u0 com.google\n====\ncom.android.contacts -1 false Total OTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-ora-di-altro-adattatore", "Account g@example.com u0 com.google\n====\ncom.android.calendar 1 true Total 1 OTHER SUCCESS\ncom.android.contacts -1 false Total x\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-due-conti", DUMP("OTHER SUCCESS", ORA_FA(2)) + DUMP("OTHER FAILURE", ORA_FA(3), "h@example.com").replace(/^Sync Status\n/, ""), `{"h@example.com": ${ADESSO - 3000}}`],
    ["dump-stesso-conto-due-volte", DUMP("OTHER SUCCESS", ORA_FA(2)) + DUMP("OTHER FAILURE", ORA_FA(3)).replace(/^Sync Status\n/, ""), null],
    ["dump-account-altro-tipo", "Account g@example.com u0 com.google.work\n====\ncom.android.calendar 1 true Total 1 OTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-account-u10", "Account g@example.com u10 com.google\n====\ncom.android.calendar 1 true Total 1 OTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-account-senza-u", "Account g@example.com com.google\n====\ncom.android.calendar 1 true Total 1 OTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-spazio-finale", "Account g@example.com u0 com.google   \n====\ncom.android.calendar 1 true Total 1 OTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-account-esaurito-da-altro-account", "Account g@example.com u0 com.google\nAccount x@example.com u0 com.other\ncom.android.calendar 1 true Total 1 OTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-esito-senza-prefisso", "Account g@example.com u0 com.google\ncom.android.calendar 1 true Total SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-esito-minuscolo", "Account g@example.com u0 com.google\ncom.android.calendar 1 true Total other success\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-calendar-senza-spazio", "Account g@example.com u0 com.google\ncom.android.calendar\tOTHER SUCCESS\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-piu-ore-nella-riga", "Account g@example.com u0 com.google\ncom.android.calendar 1 true Total 1 OTHER SUCCESS 2026-10-11 08:00:00\n   Today 2026-10-11 08:59:00\n", null],
    ["dump-ora-invalida", DUMP("OTHER SUCCESS", "2026-13-45 99:99:99"), null],
    ["dump-ora-24", DUMP("OTHER SUCCESS", "2026-10-11 24:00:00"), null],
    ["dump-ora-bisestile-no", DUMP("OTHER SUCCESS", "2025-02-29 08:00:00"), null],
    ["dump-ora-con-spazi-finali", DUMP("OTHER SUCCESS", ORA_FA(2) + "   "), null],
    ["dump-crlf", DUMP("OTHER SUCCESS", ORA_FA(2)).replaceAll("\n", "\r\n"), null],
    ["dump-nel", DUMP("OTHER SUCCESS", ORA_FA(2)).replace("Yestr  1", "Yestr\x85  1"), null],
    ["dump-lungo", DUMP("OTHER SUCCESS", ORA_FA(2)) + "riga di altro\n".repeat(20000), null],
  ];
  for (const [nome, dumpsys, ricordo] of googleCasi) caso(`google-${nome}`, { logs: OK, dumpsys, ricordo });
  caso("google-data-vuota", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "" });
  caso("google-data-assente", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: null });
  caso("google-data-rotta", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "non una data\n" });
  caso("google-data-con-spazi", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "  2026-10-11 09:00:00 \n\n" });
  caso("google-data-formato-diverso", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "Sun Oct 11 09:00:00 UTC 2026\n" });
  caso("google-data-24", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "2026-10-11 24:00:00\n" });
  caso("google-data-mese-una-cifra", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "2026-1-1 09:00:00\n" });
  caso("google-data-telefono-indietro", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1)), data: "2026-10-11 08:00:00\n" });
  caso("google-senza-dumpsys", { logs: OK, dumpsys: null });
  caso("google-dump-utf8-invalido", { logs: OK, dumpsys: Buffer.concat([Buffer.from(DUMP("OTHER SUCCESS", ORA_FA(1))), Buffer.from([0xff, 0xfe]), Buffer.from(" invalido\n")]) });
  caso("google-env-soglie", { logs: OK, dumpsys: DUMP("OTHER FAILURE", ORA_FA(1)), ricordo: `{"g@example.com": ${ADESSO - 130}}`, env: { GOOGLE_GIALLO_S: "60", GOOGLE_ROSSO_S: "120" } });
  
  // somma dei verdetti: stessa casella da più fonti
  caso("somma-reauth-e-bloccato", {
    logs: OK,
    db: conDb({ token: [tok(base64(`xx${E1}yy`))] }),
    battito: '{"outlook_davanti": true, "bloccato": true}',
    dumpsys: DUMP("OTHER FAILURE", ORA_FA(1), "roar@example.com"),
    ricordo: `{"roar@example.com": ${ADESSO - 5000}}`,
  });
  caso("somma-google-e-outlook-ok", { logs: OK, dumpsys: DUMP("OTHER SUCCESS", ORA_FA(1), "roar@example.com") });
  caso("somma-tre-messaggi", {
    logs: { "calendarSync.log": casella(900, 32768), "notifications.log": push(20) },
    db: conDb({ token: [tok(base64(`xx${E1}yy`))] }),
    battito: '{"outlook_davanti": false, "bloccato": true}',
    dumpsys: DUMP("OTHER FAILURE", ORA_FA(1), "roar@example.com"),
    ricordo: `{"roar@example.com": ${ADESSO - 1000}}`,
  });
  
    return n;
}
