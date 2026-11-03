import crypto from "node:crypto";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import type { Soglie } from "../configurazione.js";
import { scriviAtomico } from "../file.js";
import {
  ErroreTipo,
  ErroreValore,
  comeInteroPython,
  comeLatin1,
  decodificaBase64,
  dividiPerDifetto,
  epocaDaLog,
  intero,
  leggiJsonPython,
  righe,
  veroPython,
} from "../python.js";
import { apriCopia } from "./db.js";
import { coda } from "./log.js";
import {
  battitoJson,
  chiaveOutlookBin,
  dbOutlook,
  logOutlook,
  secondiAdesso,
  tokenOutlookJson,
  type ContestoRilevatore,
  type Verdetto,
} from "./tipi.js";

const RIGA_CASELLA = /^\w\t(\S+)\t.*isSyncingForAccount: accountId=(\d+), hxAccountIsSyncing=\w+.*=> isSyncing=(true|false)/;
const RIGA_PUSH = /^\w\t(\S+)\t.*FCM push sync is (available|unavailable)/;

/** Mappa indirizzo -> verdetto: l'ordine è quello in cui il dict di Python vedeva le caselle. */
export type Verdetti = Map<string, Verdetto>;

export interface TokenPosta {
  upn: string;
  token: string;
  exp: number;
  endpoint_type: "outlook_rest" | "graph";
  base_url: string;
  scopes: string;
}

function decifraTokenHx(cifrato: string, chiave: Buffer): string | null {
  try {
    const b64 = cifrato.replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - (cifrato.length % 4)) % 4);
    const raw = Buffer.from(b64, "base64");
    if (raw.length < 33) return null;
    const iv = raw.subarray(5, 17);
    const tag = raw.subarray(raw.length - 16);
    const ct = raw.subarray(17, raw.length - 16);
    const dec = crypto.createDecipheriv("aes-256-gcm", chiave, iv);
    dec.setAuthTag(tag);
    return Buffer.concat([dec.update(ct), dec.final()]).toString("utf8");
  } catch {
    return null;
  }
}

function classificaJwt(jwt: string, adessoS: number): TokenPosta | null {
  const parti = jwt.split(".");
  if (parti.length < 3 || !parti[1]) return null;
  try {
    const b64 = parti[1].replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - (parti[1].length % 4)) % 4);
    const claims = JSON.parse(Buffer.from(b64, "base64").toString("utf8")) as Record<string, unknown>;
    const upn = String(claims.upn ?? claims.email ?? claims.unique_name ?? "").trim();
    const exp = Number(claims.exp ?? 0);
    if (!upn || !Number.isFinite(exp) || exp <= adessoS) return null;
    const aud = String(claims.aud ?? "");
    const scp = String(claims.scp ?? "");
    if (aud.includes("outlook.office.com") || aud.includes("outlook.office365.com")) {
      if (["Mail.Read", "Mail.ReadWrite", "EAS.AccessAsUser", "OWA.AccessAsUser"].some((s) => scp.includes(s))) {
        return { upn, token: jwt, exp: Math.trunc(exp), endpoint_type: "outlook_rest", base_url: "https://outlook.office.com/api/v2.0", scopes: scp };
      }
    } else if (aud.includes("graph.microsoft.com") || aud === "00000003-0000-0000-c000-000000000000") {
      if (scp.includes("Mail.Read") || scp.includes("Mail.ReadWrite")) {
        return { upn, token: jwt, exp: Math.trunc(exp), endpoint_type: "graph", base_url: "https://graph.microsoft.com/v1.0", scopes: scp };
      }
    }
    return null;
  } catch {
    return null;
  }
}

async function aggiornaTokenPosta(
  c: ContestoRilevatore,
  db: { prepare: (sql: string) => { all: () => Record<string, unknown>[] } },
): Promise<void> {
  const percorsoChiave = chiaveOutlookBin(c);
  if (!existsSync(percorsoChiave)) return;
  let chiave: Buffer;
  try {
    chiave = await readFile(percorsoChiave);
    if (chiave.length !== 32) return;
  } catch {
    return;
  }
  const adessoS = secondiAdesso(c);
  const perAccount = new Map<string, TokenPosta>();
  const aggiungi = (cifrato: unknown) => {
    if (typeof cifrato !== "string" || !cifrato) return;
    const jwt = decifraTokenHx(cifrato, chiave);
    if (!jwt) return;
    const info = classificaJwt(jwt, adessoS);
    if (!info) return;
    const k = info.upn.toLowerCase();
    const prec = perAccount.get(k);
    if (
      !prec ||
      (prec.endpoint_type === "graph" && info.endpoint_type === "outlook_rest") ||
      (prec.endpoint_type === info.endpoint_type && info.exp > prec.exp)
    ) {
      perAccount.set(k, info);
    }
  };
  try {
    for (const r of db
      .prepare("select token as t from tokenStore where token is not null and (resource like '%outlook%' or resource like '%graph%')")
      .all()) {
      aggiungi(r.t);
    }
    for (const r of db.prepare("select directToken as t from mailAccounts where directToken is not null").all()) {
      aggiungi(r.t);
    }
    const payload = JSON.stringify({ generato: Math.trunc(adessoS), account: [...perAccount.values()] });
    await scriviAtomico(tokenOutlookJson(c), payload);
  } catch {
    // La raccolta dei token per la posta non deve mai interrompere il rilevatore degli accessi.
  }
}

/**
 * Caselle Microsoft, accesso: la bandierina needsReauth nel registro di Outlook. Si leggono solo l'indirizzo, l'identificativo
 * dell'account e la colonna needsReauth, mai il valore del token.
 */
export async function accessiOutlook(c: ContestoRilevatore): Promise<Verdetti> {
  const percorso = dbOutlook(c);
  if (!existsSync(percorso)) return new Map();
  const { db, chiudi } = await apriCopia(percorso);
  try {
    const reauth = new Map<string, boolean>();
    for (const r of db.prepare("select distinct lower(email) as e from mailAccounts where email is not null").all()) reauth.set(String(r.e), false);
    // Il servizio MFA di Microsoft (adnotifications.windowsazure.com) chiede spesso un nuovo accesso per registrare le informazioni di
    // sicurezza: non riguarda posta e calendario, che continuano a funzionare.
    const righeToken = db
      .prepare(
        "select accountId as a, needsReauth as n from tokenStore where needsReauth is not null and needsReauth != 0 " +
          "and (resource is null or resource not like '%adnotifications.windowsazure.com%')",
      )
      .all();
    for (const r of righeToken) {
      const acc = r.a;
      if (typeof acc !== "string") throw new ErroreTipo("accountId non è un testo");
      let chiaro: string;
      try {
        chiaro = comeLatin1(decodificaBase64(acc + "=".repeat((4 - (acc.length % 4)) % 4))).toLowerCase();
      } catch (e) {
        if (e instanceof ErroreValore) continue;
        throw e;
      }
      for (const email of reauth.keys()) if (chiaro.includes(email)) reauth.set(email, true);
    }
    await aggiornaTokenPosta(c, db);
    return new Map(
      [...reauth].map(([e, daRifare]) => [
        e,
        daRifare ? { stato: "errore", msg: "Accesso scaduto sul telefono: login da rifare in Outlook" } : { stato: "ok", msg: "" },
      ]),
    );
  } finally {
    chiudi();
  }
}

/** Verdetto per una casella: ultima = (secondi, sincronizza) o undefined, push = (secondi, disponibile) o null. */
export function verdettoOutlook(
  ultima: readonly [number, boolean] | undefined,
  push: readonly [number, boolean] | null,
  davanti: boolean,
  adesso: number,
  bloccato: boolean,
  soglie: Soglie,
): Verdetto {
  if (ultima === undefined) {
    return {
      stato: "errore",
      msg: `Outlook non conferma la sincronizzazione della casella da oltre ${dividiPerDifetto(soglie.outlookRossoS, 60)} min: Outlook fermo o sospeso sul telefono`,
    };
  }
  const [quando, sincronizza] = ultima;
  const eta = intero(adesso - quando);
  if (!sincronizza) return { stato: "avviso", msg: "Outlook ha sospeso la sincronizzazione della casella" };
  if (eta > soglie.outlookRossoS) {
    return { stato: "errore", msg: `Outlook non conferma la sincronizzazione della casella da ${dividiPerDifetto(eta, 60)} min: Outlook fermo o sospeso sul telefono` };
  }
  if (eta > soglie.outlookGialloS) {
    return { stato: "avviso", msg: `Outlook non conferma la sincronizzazione della casella da ${dividiPerDifetto(eta, 60)} min` };
  }
  if (bloccato) {
    return {
      stato: "avviso",
      msg: "Telefono bloccato: le cancellazioni e le modifiche fatte in Outlook arrivano in ritardo, sbloccalo dalla pagina Telefono",
    };
  }
  if (push !== null && !push[1] && adesso - push[0] < soglie.pushRecenteS && !davanti) {
    return {
      stato: "avviso",
      msg: "Outlook non riceve le notifiche push dal server e non è in primo piano: a telefono bloccato le novità non arrivano",
    };
  }
  return { stato: "ok", msg: "" };
}

/**
 * Caselle Microsoft, freschezza: Outlook non scrive quando ha scaricato dal server, quindi si misura che sia vivo con la casella attiva
 * (l'ultima riga isSyncingForAccount di calendarSync.log, a cui risponde dopo ogni sincronizzazione forzata) e che le notifiche push
 * funzionino (notifications.log). Senza i log non si dice niente: un controllo che non può guardare non deve dare un verdetto.
 */
export async function freschezzaOutlook(c: ContestoRilevatore, adessoDato?: number): Promise<Verdetti> {
  const adesso = adessoDato ?? secondiAdesso(c);
  const sync = await coda(path.join(logOutlook(c), "calendarSync.log"));
  if (sync === null || !existsSync(dbOutlook(c))) return new Map();
  const ultima = new Map<string, [number, boolean]>();
  for (const riga of righe(sync)) {
    const m = RIGA_CASELLA.exec(riga);
    const epoca = m ? epocaDaLog(m[1] ?? "") : null;
    if (m && epoca) ultima.set(m[2] ?? "", [epoca, m[3] === "true"]);
  }
  let push: [number, boolean] | null = null;
  for (const riga of righe((await coda(path.join(logOutlook(c), "notifications.log"))) ?? "")) {
    const m = RIGA_PUSH.exec(riga);
    const epoca = m ? epocaDaLog(m[1] ?? "") : null;
    if (m && epoca) push = [epoca, m[2] === "available"];
  }
  let davanti = true;
  let bloccato = false;
  try {
    const battito: unknown = leggiJsonPython(await readFile(battitoJson(c), "utf8"));
    if (typeof battito !== "object" || battito === null || Array.isArray(battito)) throw new ErroreValore("battito non è un oggetto");
    const b = battito as Record<string, unknown>;
    davanti = veroPython("outlook_davanti" in b ? b.outlook_davanti : true);
    bloccato = veroPython("bloccato" in b ? b.bloccato : false);
  } catch {
    davanti = true;
    bloccato = false;
  }
  const { db, chiudi } = await apriCopia(dbOutlook(c));
  const caselle = new Map<number, string>();
  try {
    for (const r of db.prepare("select accountID as i, lower(email) as e from mailAccounts where hxAccountId is not null and email is not null").all()) {
      caselle.set(comeInteroPython(r.i), String(r.e));
    }
  } finally {
    chiudi();
  }
  const verdetti: Verdetti = new Map();
  for (const [i, e] of caselle) verdetti.set(e, verdettoOutlook(ultima.get(String(i)), push, davanti, adesso, bloccato, c.soglie));
  return verdetti;
}
