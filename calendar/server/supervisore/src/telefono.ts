import { Adb } from "@devicefarmer/adbkit";
import type { Client } from "@devicefarmer/adbkit";
import type { Duplex } from "node:stream";

// Il telefono visto dal supervisore. Al posto di `docker exec oe-copilot-telefono <comando>` dello script e del rilevatore c'è un
// comando adb sulla rete di compose (telefono:5555). I comandi che leggono soltanto (dumpsys, getprop, date...) e quelli che
// cambiano il telefono (input, am, cmd...) hanno due metodi diversi: chi prova senza toccare il telefono usa TelefonoSolaLettura.

export type TipoErrore = "trasporto" | "tempo" | "non-permesso";

export class ErroreTelefono extends Error {
  constructor(
    messaggio: string,
    readonly tipo: TipoErrore,
  ) {
    super(messaggio);
    this.name = "ErroreTelefono";
  }
}

export interface OpzioniComando {
  timeoutMs?: number;
}

export interface Telefono {
  /** `docker exec <telefono> true`: il telefono risponde a un comando. Non solleva mai errori. */
  vivo(): Promise<boolean>;
  /** Un comando che legge soltanto: restituisce lo standard output (lo standard error si scarta come faceva 2>/dev/null). */
  leggi(comando: readonly string[], opzioni?: OpzioniComando): Promise<string>;
  /** Un comando che cambia il telefono (input, am, cmd...): l'output non serve, come in >/dev/null 2>&1. */
  agisci(comando: readonly string[], opzioni?: OpzioniComando): Promise<void>;
}

const SICURO = /^[A-Za-z0-9_@%+=:,./-]+$/;

/** Un argomento per la shell del telefono: fra apici se contiene qualcosa che la shell leggerebbe. */
export function quota(argomento: string): string {
  return SICURO.test(argomento) ? argomento : "'" + argomento.replaceAll("'", "'\\''") + "'";
}

export const riga = (comando: readonly string[]): string => comando.map(quota).join(" ");

async function entroIlTempo<T>(lavoro: PromiseLike<T>, ms: number, alScadere: () => void): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const promessa = Promise.resolve(lavoro);
  promessa.catch(() => undefined);
  const scadenza = new Promise<never>((_, rifiuta) => {
    timer = setTimeout(() => {
      alScadere();
      rifiuta(new ErroreTelefono(`nessuna risposta in ${ms / 1000} s`, "tempo"));
    }, ms);
  });
  try {
    return await Promise.race([promessa, scadenza]);
  } finally {
    clearTimeout(timer);
  }
}

function comeErrore(e: unknown): ErroreTelefono {
  if (e instanceof ErroreTelefono) return e;
  return new ErroreTelefono(e instanceof Error ? e.message : String(e), "trasporto");
}

export class TelefonoAdb implements Telefono {
  constructor(
    private readonly cliente: Client,
    private readonly seriale: string,
    private readonly timeoutMs: number,
  ) {}

  async vivo(): Promise<boolean> {
    try {
      await this.esegui(["true"], false, this.timeoutMs);
      return true;
    } catch {
      await this.ricollega();
    }
    try {
      await this.esegui(["true"], false, this.timeoutMs);
      return true;
    } catch {
      return false;
    }
  }

  leggi(comando: readonly string[], opzioni: OpzioniComando = {}): Promise<string> {
    return this.esegui(comando, true, opzioni.timeoutMs ?? this.timeoutMs);
  }

  async agisci(comando: readonly string[], opzioni: OpzioniComando = {}): Promise<void> {
    await this.esegui(comando, false, opzioni.timeoutMs ?? this.timeoutMs);
  }

  private async esegui(comando: readonly string[], senzaErrori: boolean, timeoutMs: number): Promise<string> {
    let flusso: Duplex | undefined;
    const lavoro = (async () => {
      const aperto = await this.cliente.getDevice(this.seriale).shell(riga(comando) + (senzaErrori ? " 2>/dev/null" : ""));
      flusso = aperto;
      return (await Adb.util.readAll(aperto)).toString("utf8");
    })();
    try {
      return await entroIlTempo(lavoro, timeoutMs, () => flusso?.destroy());
    } catch (e) {
      throw comeErrore(e);
    }
  }

  /** Dopo un riavvio del telefono la connessione cade: si rifà, come faceva il ciclo `adb connect` dell'avvio di schermo. */
  private async ricollega(): Promise<void> {
    const i = this.seriale.lastIndexOf(":");
    const host = i < 0 ? this.seriale : this.seriale.slice(0, i);
    const porta = i < 0 ? 5555 : Number(this.seriale.slice(i + 1));
    for (const passo of [() => this.cliente.disconnect(host, porta), () => this.cliente.connect(host, porta)]) {
      try {
        await entroIlTempo(passo(), Math.min(this.timeoutMs, 10_000), () => undefined);
      } catch {
        // il passo dopo, o il giro dopo, riprova
      }
    }
  }
}

export function creaTelefonoAdb(opzioni: { telefono: string; adbHost: string; adbPorta: number; timeoutComandoMs: number }): TelefonoAdb {
  const cliente = Adb.createClient({ host: opzioni.adbHost, port: opzioni.adbPorta });
  const timeout = opzioni.timeoutComandoMs > 0 ? opzioni.timeoutComandoMs : 30_000;
  return new TelefonoAdb(cliente, opzioni.telefono, timeout);
}

const LETTURE = new Set(["dumpsys", "getprop", "cat", "ls", "logcat", "tail", "stat", "date", "pidof"]);
const SERVIZI_DUMPSYS = new Set(["jobscheduler", "power", "window", "content"]);
const OPZIONI_LOGCAT_CHE_SCRIVONO = new Set(["-c", "--clear", "-G", "--buffer-size", "-f", "-r", "-n", "-P", "--prune"]);

/** Solleva ErroreTelefono("non-permesso") se il comando non è una lettura dell'elenco: dumpsys (solo jobscheduler, power, window, content), getprop, cat, ls, logcat -d, tail, stat, date, pidof. */
export function controllaSolaLettura(comando: readonly string[]): void {
  const rifiuta = (motivo: string): never => {
    throw new ErroreTelefono(`comando non permesso (${motivo}): ${riga(comando)}`, "non-permesso");
  };
  const [nome = "", ...argomenti] = comando;
  if (!LETTURE.has(nome)) rifiuta("non è una lettura");
  if (nome === "dumpsys" && !(argomenti.length === 1 && SERVIZI_DUMPSYS.has(argomenti[0] ?? ""))) rifiuta("dumpsys solo di jobscheduler, power, window, content");
  if (nome === "logcat" && (!argomenti.includes("-d") || argomenti.some((a) => OPZIONI_LOGCAT_CHE_SCRIVONO.has(a)))) rifiuta("logcat solo con -d");
  if (nome === "date" && !(argomenti.length === 0 || (argomenti.length === 1 && (argomenti[0] ?? "").startsWith("+")))) rifiuta("date non imposta l'ora");
  if (nome === "tail" && argomenti.some((a) => /^(-[a-zA-Z]*[fF]|--follow)/.test(a))) rifiuta("tail non resta in attesa");
}

/**
 * Il telefono vero, ma solo per le letture dell'elenco: ogni altro comando solleva un errore prima di arrivare ad adb. Serve alle
 * prove contro il telefono in funzione, dove un tocco o un `am start` non devono partire per nessuna strada.
 */
export class TelefonoSolaLettura implements Telefono {
  invii: string[] = [];

  constructor(private readonly interno: Telefono) {}

  async vivo(): Promise<boolean> {
    await this.leggi(["getprop", "sys.boot_completed"]);
    return true;
  }

  async leggi(comando: readonly string[], opzioni?: OpzioniComando): Promise<string> {
    controllaSolaLettura(comando);
    this.invii.push(riga(comando));
    return this.interno.leggi(comando, opzioni);
  }

  async agisci(comando: readonly string[]): Promise<void> {
    throw new ErroreTelefono(`azione non permessa: ${riga(comando)}`, "non-permesso");
  }
}
