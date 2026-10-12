import { existsSync, statSync } from "node:fs";
import { chmod, chown, mkdir, stat } from "node:fs/promises";
import path from "node:path";
import type { Configurazione } from "./configurazione.js";
import { scriviAtomico } from "./file.js";
import type { Orologio } from "./orologio.js";
import { ErroreTelefono, type Telefono } from "./telefono.js";

// Il ciclo di sincronizza_telefono.sh: stesse decisioni nello stesso ordine, stessi comandi al telefono, stesso battito. Un giro dura
// circa 35 secondi: 30 di pausa (OGNI) più il lavoro e le attese del giro (1 se si riaccende lo schermo, 2 e 2 per la schermata
// iniziale e per Outlook, 3 se Outlook non torna sulla vista principale).

export interface Battito {
  pausa: boolean;
  telefono: boolean;
  davanti: boolean;
  bloccato: boolean;
}

export interface DipendenzeCiclo {
  config: Configurazione;
  telefono: Telefono;
  orologio: Orologio;
  /** rileva_accessi.py: gira ogni ACCESSI_OGNI giri se il telefono risponde. */
  rilevatore: () => Promise<unknown>;
  log: (messaggio: string) => void;
  /** I guasti già scritti nel log: lo mette `esegui`, chi lancia un giro solo (le prove) può non darlo. */
  guasti?: Guasti;
}

export interface RisultatoGiro {
  battito: Battito;
  /** Perché il telefono non è stato raggiunto (adb caduto, telefono spento, nessuna risposta), se non lo è stato. */
  nonRaggiunto?: string;
}

/** Un guasto che dura si scrive una volta sola e poi quando passa: un rigo ogni 35 secondi riempirebbe il log per giorni. */
export class Guasti {
  private readonly attuali = new Map<string, string>();

  constructor(private readonly log: (messaggio: string) => void) {}

  segnala(chiave: string, motivo: string): void {
    if (this.attuali.get(chiave) === motivo) return;
    this.attuali.set(chiave, motivo);
    this.log(`${chiave}: ${motivo}`);
  }

  risolto(chiave: string): void {
    if (this.attuali.delete(chiave)) this.log(`${chiave}: di nuovo a posto`);
  }
}

const MODELLI_OUTLOOK = "com\\.microsoft\\.office\\.outlook";
const FOCUS_OUTLOOK = "com.acompli.acompli.CentralActivity";
const UTENTE_SCHERMO = 10001;

const messaggio = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** `[ -e PAUSA ]`, oppure `[ -f stato/schermo/PAUSA ]` più giovane di PAUSA_SCHERMO_SCADE secondi (stat -c %Y, 0 se manca). */
export function pausaAttiva(c: Configurazione, orologio: Orologio): boolean {
  let pausa = existsSync(c.pausaManuale);
  try {
    const s = statSync(c.pausaSchermo);
    if (s.isFile() && Math.floor(orologio.adesso() / 1000) - Math.floor(s.mtimeMs / 1000) < c.pausaSchermoScade) pausa = true;
  } catch {
    // senza file o illeggibile: nessuna pausa
  }
  return pausa;
}

/** Gli identificativi dei lavori di Android da forzare: `grep -oE "JOB #1000/N: hex com.android.calendar/(Outlook|Google)" | sed | sort -u`. */
export function idLavori(lavori: string, conGoogle: boolean): string[] {
  const modelli = conGoogle ? `${MODELLI_OUTLOOK}|com\\.google` : MODELLI_OUTLOOK;
  const ricerca = new RegExp(`JOB #1000/([0-9]+): [0-9a-f]+ com\\.android\\.calendar/(?:${modelli})`, "g");
  const id = new Set<string>();
  for (const m of lavori.matchAll(ricerca)) id.add(m[1] ?? "");
  return [...id].sort();
}

const primaRigaCon = (testo: string, ago: string): string => testo.split("\n").find((r) => r.includes(ago)) ?? "";

/** Il testo di battito.json come lo scriveva printf nello script (stessi spazi, a capo finale). */
export function testoBattito(quando: number, b: Battito): string {
  return `{"quando": ${quando}, "pausa": ${b.pausa}, "telefono": ${b.telefono}, "outlook_davanti": ${b.davanti}, "bloccato": ${b.bloccato}}\n`;
}

/** `mkdir -p stato` e `install -d -o 10001 -g 10001 -m 755 stato/schermo`: la cartella del file PAUSA del servizio schermo. */
export async function preparaCartelle(c: Configurazione, log: (m: string) => void): Promise<void> {
  const schermo = path.dirname(c.pausaSchermo);
  try {
    await mkdir(c.statoDir, { recursive: true });
    await mkdir(schermo, { recursive: true, mode: 0o755 });
    const s = await stat(schermo);
    if (s.uid !== UTENTE_SCHERMO || s.gid !== UTENTE_SCHERMO) await chown(schermo, UTENTE_SCHERMO, UTENTE_SCHERMO);
    if ((s.mode & 0o777) !== 0o755) await chmod(schermo, 0o755);
  } catch (e) {
    log(`cartella ${schermo} non preparata: ${messaggio(e)}`);
  }
}

async function lavoroSulTelefono(conta: number, b: Battito, d: DipendenzeCiclo): Promise<void> {
  const { config: c, telefono: t, orologio: o } = d;
  const lavori = await t.leggi(["dumpsys", "jobscheduler"]);
  for (const id of idLavori(lavori, conta % c.googleOgni === 0)) {
    await t.agisci(["cmd", "jobscheduler", "run", "-f", "android", id]);
  }
  const sveglio = async () => (await t.leggi(["dumpsys", "power"])).includes("mWakefulness=Awake");
  if (!b.pausa) {
    if (!(await sveglio())) {
      await t.agisci(["input", "keyevent", "224"]);
      await o.dormi(1000);
    }
    await t.agisci(["am", "start", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.HOME"]);
    await o.dormi(2000);
    await t.agisci(["am", "start", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-n", "com.microsoft.office.outlook/.MainActivity"]);
    await o.dormi(2000);
    // Outlook torna sull'ultima schermata aperta: se è rimasto nelle impostazioni non scarica niente, e lo si riapre da capo (NEW_TASK|CLEAR_TASK)
    if (!primaRigaCon(await t.leggi(["dumpsys", "window"]), "mCurrentFocus").includes(FOCUS_OUTLOOK)) {
      await t.agisci(["am", "start", "-f", "0x10008000", "-n", "com.microsoft.office.outlook/.MainActivity"]);
      await o.dormi(3000);
    }
  }
  const finestre = await t.leggi(["dumpsys", "window"]);
  b.davanti = primaRigaCon(finestre, "mCurrentFocus").includes(FOCUS_OUTLOOK);
  b.bloccato = finestre.includes("isKeyguardShowing=true");
  // Un tocco sulla barra di stato (nessuna app lo riceve) conta come attività dell'utente e rimanda il blocco a 15 minuti dei criteri di Outlook
  if (c.toccoOgni > 0 && conta % c.toccoOgni === 0 && (await sveglio())) {
    await t.agisci(["input", "tap", "360", "5"]);
  }
}

/** Un giro: stato della pausa, comandi al telefono, battito, e ogni ACCESSI_OGNI giri il rilevatore. */
export async function giro(conta: number, d: DipendenzeCiclo): Promise<RisultatoGiro> {
  const { config: c, telefono: t, orologio: o } = d;
  const guasti = d.guasti ?? new Guasti(d.log);
  const b: Battito = { pausa: pausaAttiva(c, o), telefono: false, davanti: false, bloccato: false };
  let nonRaggiunto: string | undefined;
  try {
    if (await t.vivo()) {
      b.telefono = true;
      await lavoroSulTelefono(conta, b, d);
    } else {
      nonRaggiunto = "nessuna risposta ai comandi adb";
    }
  } catch (e) {
    // Un comando che non arriva al telefono (adb caduto, telefono spento, nessuna risposta): il giro si ferma qui e il battito lo dice.
    // Lo script andava avanti comando per comando, e con il telefono già giù li provava tutti.
    if (!(e instanceof ErroreTelefono)) throw e;
    b.telefono = false;
    b.davanti = false;
    b.bloccato = false;
    nonRaggiunto = e.message;
  }
  try {
    await scriviAtomico(path.join(c.statoDir, "battito.json"), testoBattito(Math.floor(o.adesso() / 1000), b));
    guasti.risolto("battito.json non scritto");
  } catch (e) {
    guasti.segnala("battito.json non scritto", messaggio(e));
  }
  if (b.telefono && conta % c.accessiOgni === 0) {
    try {
      await d.rilevatore();
      guasti.risolto("rilevatore degli accessi non riuscito");
    } catch (e) {
      guasti.segnala("rilevatore degli accessi non riuscito", messaggio(e));
    }
  }
  return nonRaggiunto === undefined ? { battito: b } : { battito: b, nonRaggiunto };
}

/**
 * Il ciclo senza fine (finché il segnale non scatta): un giro, poi OGNI secondi di pausa. Un giro che si rompe non ferma il servizio. Nel
 * log finiscono i guasti, una volta quando cominciano e una quando passano, non un rigo a ogni giro.
 */
export async function esegui(d: DipendenzeCiclo, segnale: AbortSignal): Promise<void> {
  const guasti = new Guasti(d.log);
  const dipendenze: DipendenzeCiclo = { ...d, guasti };
  let conta = 0;
  while (!segnale.aborted) {
    try {
      const { battito, nonRaggiunto } = await giro(conta, dipendenze);
      if (battito.telefono) guasti.risolto("telefono non raggiunto");
      else guasti.segnala("telefono non raggiunto", nonRaggiunto ?? "errore");
      guasti.risolto("giro non riuscito");
    } catch (e) {
      guasti.segnala("giro non riuscito", messaggio(e));
    }
    conta += 1;
    await d.orologio.dormi(d.config.ogni * 1000, segnale);
  }
}
