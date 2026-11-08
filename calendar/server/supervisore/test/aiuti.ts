import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { Soglie } from "../src/configurazione.js";
import { OrologioFinto } from "../src/orologio.js";
import type { ContestoRilevatore } from "../src/rilevatore/tipi.js";
import type { OpzioniComando, Telefono } from "../src/telefono.js";

export const SOGLIE: Soglie = { outlookGialloS: 600, outlookRossoS: 1800, pushRecenteS: 600, googleGialloS: 900, googleRossoS: 2700 };

/** Un telefono che risponde con quello che gli dice la prova e ricorda i comandi, uno per riga come li scriveva lo script. */
export class TelefonoFinto implements Telefono {
  letture: string[] = [];
  azioni: string[] = [];
  sveglio = true;
  risposta: (comando: readonly string[], opzioni?: OpzioniComando) => string | Promise<string> = () => "";

  async vivo(): Promise<boolean> {
    return this.sveglio;
  }

  async leggi(comando: readonly string[], opzioni?: OpzioniComando): Promise<string> {
    this.letture.push(comando.join(" "));
    return this.risposta(comando, opzioni);
  }

  async agisci(comando: readonly string[]): Promise<void> {
    this.azioni.push(comando.join(" "));
  }
}

export interface Cartelle {
  radice: string;
  dati: string;
  stato: string;
  log: string;
  databases: string;
  db: string;
  uscita: string;
  battito: string;
  googleStato: string;
  pulisci: () => void;
}

/** La stessa struttura del server: dati del telefono con le cartelle di Outlook, e stato/ accanto. */
export function creaCartelle(): Cartelle {
  const radice = mkdtempSync(path.join(os.tmpdir(), "supervisore-"));
  const dati = path.join(radice, "telefono", "data");
  const stato = path.join(radice, "stato");
  const log = path.join(dati, "data/com.microsoft.office.outlook/app_logs");
  const databases = path.join(dati, "data/com.microsoft.office.outlook/databases");
  mkdirSync(log, { recursive: true });
  mkdirSync(databases, { recursive: true });
  mkdirSync(stato, { recursive: true });
  return {
    radice,
    dati,
    stato,
    log,
    databases,
    db: path.join(databases, "acompliAcct.db"),
    uscita: path.join(stato, "accessi.json"),
    battito: path.join(stato, "battito.json"),
    googleStato: path.join(stato, "google-sync.json"),
    pulisci: () => rmSync(radice, { recursive: true, force: true }),
  };
}

export function contesto(c: Cartelle, telefono: Telefono, orologio: OrologioFinto, soglie: Soglie = SOGLIE): ContestoRilevatore {
  return { dati: c.dati, uscita: c.uscita, telefono, orologio, soglie, log: () => undefined };
}

export function creaDb(percorso: string, istruzioni: string[]): void {
  const db = new DatabaseSync(percorso);
  try {
    for (const i of istruzioni) db.exec(i);
  } finally {
    db.close();
  }
}

export const MAIL_ACCOUNTS =
  "create table mailAccounts (accountID integer, hxAccountId text, email text); " +
  "insert into mailAccounts values (32768, 'roar_Hx', 'Roar@example.com'), (32769, null, 'roar@example.com'), " +
  "(32772, 'band_Hx', 'band@example.com'), (32773, null, 'band@example.com')";

const due = (n: number, cifre = 2): string => String(n).padStart(cifre, "0");

/** 2026-10-11T05:03:21.996+0000, l'ora (UTC) dei log di Outlook. */
export function stamp(epocaSecondi: number): string {
  const t = new Date(Math.round(epocaSecondi * 1000));
  return (
    `${t.getUTCFullYear()}-${due(t.getUTCMonth() + 1)}-${due(t.getUTCDate())}T${due(t.getUTCHours())}:${due(t.getUTCMinutes())}:` +
    `${due(t.getUTCSeconds())}.${due(t.getUTCMilliseconds(), 3)}+0000`
  );
}

export const rigaCasella = (epocaSecondi: number, idh: number, sincronizza = "true"): string =>
  `D\t${stamp(epocaSecondi)}\t[ci=x]\tSyncAdapterThread\toutlook.calendarSync > CalendarSyncAccou\t` +
  `isSyncingForAccount: accountId=${idh}, hxAccountIsSyncing=${sincronizza}, calendarSyncAccount!=null=true, ` +
  `isSyncable>0=true => isSyncing=${sincronizza}\n`;

export const rigaPush = (epocaSecondi: number, disponibile = true): string =>
  `I\t${stamp(epocaSecondi)}\t[ci=x]\tOM_APP_SESSION_MA\toutlook.Notifications > FcmTokenReaderWr\t` +
  `FCM push sync is ${disponibile ? "available" : "unavailable due to missing token"}\n`;

export const scrivi = (cartella: string, nome: string, righe: string[]): void => writeFileSync(path.join(cartella, nome), righe.join(""), "utf8");
