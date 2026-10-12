import { Readable } from "node:stream";
import type { Client } from "@devicefarmer/adbkit";
import { describe, expect, test } from "vitest";
import { ErroreTelefono, TelefonoAdb, TelefonoSolaLettura, controllaSolaLettura, quota, riga, type Telefono } from "../src/telefono.js";
import { TelefonoFinto } from "./aiuti.js";

const permesso = (...comando: string[]): void => controllaSolaLettura(comando);
const rifiutato = (...comando: string[]): void => {
  expect(() => controllaSolaLettura(comando), comando.join(" ")).toThrowError(ErroreTelefono);
};

describe("la riga di comando per la shell del telefono", () => {
  test("gli argomenti semplici restano come sono, gli altri vanno fra apici", () => {
    expect(riga(["dumpsys", "power"])).toBe("dumpsys power");
    expect(riga(["am", "start", "-f", "0x10008000", "-n", "com.microsoft.office.outlook/.MainActivity"])).toBe("am start -f 0x10008000 -n com.microsoft.office.outlook/.MainActivity");
    expect(riga(["date", "+%Y-%m-%d %H:%M:%S"])).toBe("date '+%Y-%m-%d %H:%M:%S'");
    expect(quota("a;b")).toBe("'a;b'");
    expect(quota("l'uno")).toBe("'l'\\''uno'");
    expect(quota("")).toBe("''");
    expect(riga(["cat", "/data/x; rm -rf /"])).toBe("cat '/data/x; rm -rf /'");
  });
});

describe("la guardia delle sole letture (le prove contro il telefono in funzione)", () => {
  test("passano le letture dell'elenco", () => {
    permesso("getprop", "sys.boot_completed");
    permesso("dumpsys", "jobscheduler");
    permesso("dumpsys", "power");
    permesso("dumpsys", "window");
    permesso("dumpsys", "content");
    permesso("cat", "/data/local/tmp/x");
    permesso("ls", "-l", "/data");
    permesso("logcat", "-d", "-t", "50");
    permesso("tail", "-n", "20", "/data/x.log");
    permesso("stat", "-c", "%A", "/data");
    permesso("date");
    permesso("date", "+%Y-%m-%d %H:%M:%S");
    permesso("pidof", "adbd");
  });

  test("ogni comando che cambia il telefono è rifiutato prima di arrivare ad adb", () => {
    rifiutato("input", "tap", "360", "5");
    rifiutato("input", "keyevent", "224");
    rifiutato("am", "start", "-n", "com.microsoft.office.outlook/.MainActivity");
    rifiutato("am", "force-stop", "com.microsoft.office.outlook");
    rifiutato("cmd", "jobscheduler", "run", "-f", "android", "9");
    rifiutato("cmd", "statusbar", "collapse");
    rifiutato("settings", "put", "global", "stay_on_while_plugged_in", "7");
    rifiutato("svc", "power", "stayon", "true");
    rifiutato("wm", "size", "reset");
    rifiutato("pm", "grant", "x", "y");
    rifiutato("kill", "1");
    rifiutato("reboot");
    rifiutato("sh", "-c", "input tap 1 1");
    rifiutato("rm", "/data/x");
    rifiutato("true");
    rifiutato();
  });

  test("anche le varianti delle letture che scrivono o restano in attesa", () => {
    rifiutato("dumpsys", "battery", "set", "ac", "1");
    rifiutato("dumpsys", "battery", "reset");
    rifiutato("dumpsys", "deviceidle", "whitelist", "+x");
    rifiutato("dumpsys");
    rifiutato("dumpsys", "power", "x");
    rifiutato("logcat");
    rifiutato("logcat", "-c");
    rifiutato("logcat", "-d", "-c");
    rifiutato("logcat", "-d", "-f", "/data/x");
    rifiutato("logcat", "-d", "-G", "1M");
    rifiutato("date", "-s", "2026-01-01");
    rifiutato("date", "011200002026");
    rifiutato("tail", "-f", "/data/x.log");
    rifiutato("tail", "-n", "5", "-F", "/data/x.log");
    rifiutato("tail", "-fn", "5", "/data/x.log");
    rifiutato("tail", "--follow", "/data/x.log");
  });

  test("TelefonoSolaLettura: le letture arrivano al telefono, il resto no, e `vivo` legge una proprietà", async () => {
    const vero = new TelefonoFinto();
    vero.risposta = () => "1\n";
    const t: Telefono = new TelefonoSolaLettura(vero);
    expect(await t.vivo()).toBe(true);
    expect(await t.leggi(["dumpsys", "power"])).toBe("1\n");
    await expect(t.agisci(["input", "tap", "360", "5"])).rejects.toThrowError(ErroreTelefono);
    await expect(t.leggi(["input", "tap", "360", "5"])).rejects.toThrowError(ErroreTelefono);
    expect(vero.letture).toEqual(["getprop sys.boot_completed", "dumpsys power"]);
    expect(vero.azioni).toEqual([]);
  });
});

// Il client adb finto: ricorda cosa gli viene chiesto e risponde con quello che dice la prova.
function clienteFinto(shell: (comando: string) => Promise<Readable>) {
  const registro = { shell: [] as string[], connect: [] as string[], disconnect: [] as string[] };
  const client = {
    getDevice: (seriale: string) => ({
      shell: async (comando: string) => {
        registro.shell.push(`${seriale} ${comando}`);
        return shell(comando);
      },
    }),
    connect: async (host: string, porta: number) => void registro.connect.push(`${host}:${porta}`),
    disconnect: async (host: string, porta: number) => void registro.disconnect.push(`${host}:${porta}`),
  } as unknown as Client;
  return { client, registro };
}

const flusso = (testo: string): Readable =>
  new Readable({
    read() {
      this.push(Buffer.from(testo));
      this.push(null);
    },
  });

const senzaFine = (): Readable => new Readable({ read() {} });

describe("TelefonoAdb: il client adb al posto di docker exec", () => {
  test("una lettura chiede l'output e scarta lo standard error, un'azione non ne usa", async () => {
    const { client, registro } = clienteFinto(async () => flusso("uno\ncaffè € 日本\n"));
    const t = new TelefonoAdb(client, "telefono:5555", 1000);
    expect(await t.leggi(["dumpsys", "power"])).toBe("uno\ncaffè € 日本\n");
    await t.agisci(["input", "tap", "360", "5"]);
    expect(registro.shell).toEqual(["telefono:5555 dumpsys power 2>/dev/null", "telefono:5555 input tap 360 5"]);
  });

  test("un comando che non finisce in tempo: errore di attesa e flusso chiuso", async () => {
    const aperto = senzaFine();
    const { client } = clienteFinto(async () => aperto);
    const t = new TelefonoAdb(client, "telefono:5555", 40);
    const errore = await t.leggi(["dumpsys", "content"]).catch((e: unknown) => e);
    expect(errore).toBeInstanceOf(ErroreTelefono);
    expect((errore as ErroreTelefono).tipo).toBe("tempo");
    expect(aperto.destroyed).toBe(true);
    const per = await t.leggi(["dumpsys", "content"], { timeoutMs: 20 }).catch((e: unknown) => e);
    expect((per as ErroreTelefono).message).toContain("0.02 s");
  });

  test("adb che rifiuta il comando (dispositivo non trovato, connessione caduta): errore di trasporto", async () => {
    const { client } = clienteFinto(async () => {
      throw new Error("Failure: 'device offline'");
    });
    const errore = await new TelefonoAdb(client, "telefono:5555", 1000).agisci(["input", "tap", "1", "1"]).catch((e: unknown) => e);
    expect(errore).toBeInstanceOf(ErroreTelefono);
    expect((errore as ErroreTelefono).tipo).toBe("trasporto");
    expect((errore as ErroreTelefono).message).toContain("device offline");
  });

  test("vivo: al primo comando che passa non si ricollega, altrimenti disconnect e connect e un secondo tentativo", async () => {
    const buono = clienteFinto(async () => flusso(""));
    expect(await new TelefonoAdb(buono.client, "telefono:5555", 1000).vivo()).toBe(true);
    expect(buono.registro.connect).toEqual([]);

    let chiamate = 0;
    const cade = clienteFinto(async () => {
      if (++chiamate === 1) throw new Error("device 'telefono:5555' not found");
      return flusso("");
    });
    expect(await new TelefonoAdb(cade.client, "telefono:5555", 1000).vivo()).toBe(true);
    expect(cade.registro.disconnect).toEqual(["telefono:5555"]);
    expect(cade.registro.connect).toEqual(["telefono:5555"]);
    expect(cade.registro.shell).toEqual(["telefono:5555 true", "telefono:5555 true"]);
  });

  test("vivo: se il telefono non risponde nemmeno dopo la riconnessione è falso, senza sollevare errori", async () => {
    const { client, registro } = clienteFinto(async () => {
      throw new Error("connect ECONNREFUSED");
    });
    expect(await new TelefonoAdb(client, "telefono:5555", 1000).vivo()).toBe(false);
    expect(registro.shell.length).toBe(2);
    expect(registro.connect).toEqual(["telefono:5555"]);
  });

  test("vivo: un comando che non risponde in tempo conta come telefono che non risponde", async () => {
    const { client } = clienteFinto(async () => senzaFine());
    expect(await new TelefonoAdb(client, "telefono:5555", 30).vivo()).toBe(false);
  });
});