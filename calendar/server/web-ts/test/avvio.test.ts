import { randomBytes } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Autenticatore } from "./autenticatore";
import { APP, Browser, type Utente, utenteNuovo } from "./browser-prova";

// L'avvio del gateway (src/server/boot.ts) con ogni valore di AUTH_2FA, su un database temporaneo e un proprietario a caso.

let cartella: string;
let proprietario: Utente;
const AMBIENTE = ["AUTH_2FA", "GATEWAY_DB", "OE_DATI", "APP_URL", "BETTER_AUTH_SECRET", "ADMIN_EMAIL", "ADMIN_PASSWORD"];

beforeEach(() => {
  vi.resetModules();
  cartella = fs.mkdtempSync(path.join(os.tmpdir(), "oe-avvio-"));
  proprietario = utenteNuovo();
  process.env.OE_DATI = cartella;
  process.env.GATEWAY_DB = path.join(cartella, "gateway.db");
  process.env.APP_URL = APP;
  process.env.BETTER_AUTH_SECRET = randomBytes(24).toString("hex");
  process.env.ADMIN_EMAIL = proprietario.email;
  process.env.ADMIN_PASSWORD = proprietario.password;
  delete process.env.AUTH_2FA;
});

/** Chiude le connessioni della copia dei moduli in uso (due sullo stesso file) prima di ripartire o di togliere la cartella. */
async function chiudi() {
  const { appDb } = await import("@/lib/db");
  const { chiudiArchivio } = await import("@/lib/core/archivio");
  appDb().close();
  chiudiArchivio();
}

afterEach(async () => {
  await chiudi();
  for (const v of AMBIENTE) delete process.env[v];
  vi.restoreAllMocks();
  fs.rmSync(cartella, { recursive: true, force: true });
});

async function avvia() {
  const registro = vi.spyOn(console, "log").mockImplementation(() => {});
  const { boot } = await import("@/server/boot");
  await boot();
  const righe = registro.mock.calls.map((c) => c.join(" "));
  registro.mockRestore();
  return righe;
}

async function browser(origine = APP) {
  const { auth } = await import("@/lib/auth");
  return new Browser(auth(), origine);
}

async function tabelleEColonne() {
  const { appDb } = await import("@/lib/db");
  const db = appDb();
  const tabelle = db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").pluck().all() as string[];
  const colonne = (db.prepare('PRAGMA table_info("user")').all() as { name: string }[]).map((c) => c.name);
  return { tabelle, colonne };
}

describe("avvio senza AUTH_2FA (off)", () => {
  it("parte, lo dice nel log, crea le tabelle del secondo fattore e il proprietario entra con la password", async () => {
    const righe = await avvia();
    expect(righe.some((r) => r.includes("(AUTH_2FA): off"))).toBe(true);
    const { tabelle, colonne } = await tabelleEColonne();
    expect(tabelle).toEqual(expect.arrayContaining(["user", "session", "account", "verification", "login_failures", "twoFactor", "passkey"]));
    expect(colonne).toContain("twoFactorEnabled");

    const r = await (await browser()).accedi(proprietario);
    expect(r.status).toBe(200);
    expect(Object.keys(await r.json()).sort()).toEqual(["redirect", "token", "user"]);
  });

  it("le strade del secondo fattore rispondono 404 (qui dal plugin che non c'è)", async () => {
    await avvia();
    const b = await browser();
    for (const p of ["/two-factor/verify-totp", "/passkey/verify-authentication"]) expect((await b.chiama(p, {})).status).toBe(404);
    expect((await b.chiama("/passkey/generate-authenticate-options")).status).toBe(404);
  });

  it("un valore vuoto vale off", async () => {
    process.env.AUTH_2FA = "";
    const righe = await avvia();
    expect(righe.some((r) => r.includes("(AUTH_2FA): off"))).toBe(true);
  });

  it("riavviato non cambia niente: stesse tabelle, stesso utente", async () => {
    await avvia();
    const prima = await tabelleEColonne();
    const { appDb } = await import("@/lib/db");
    const utenti = appDb().prepare('SELECT COUNT(*) FROM "user"').pluck().get();
    await chiudi();
    vi.resetModules();
    const righe = await avvia();
    expect(await tabelleEColonne()).toEqual(prima);
    const { appDb: dopo } = await import("@/lib/db");
    expect(dopo().prepare('SELECT COUNT(*) FROM "user"').pluck().get()).toBe(utenti);
    expect(righe.some((r) => r.includes("in linea con .env"))).toBe(true);
  });
});

describe("avvio su un database fatto dall'immagine di prima", () => {
  it("aggiunge tabelle e colonna senza toccare i dati: l'utente c'è ancora, con la sua password, e il secondo fattore è spento", async () => {
    const { openDb } = await import("@/lib/db");
    const { getMigrations } = await import("better-auth/db/migration");
    const { authOptions, createAuth } = await import("@/lib/auth");
    const { migrateLockout } = await import("@/lib/lockout");
    const { syncOwner } = await import("@/lib/owner");
    // come l'avvio di prima: solo le tabelle di Better Auth e quella del blocco, niente plugin
    const vecchio = openDb(process.env.GATEWAY_DB!);
    await (await getMigrations(authOptions(vecchio, "off"))).runMigrations();
    migrateLockout(vecchio);
    await syncOwner(createAuth(vecchio, "off"), vecchio, proprietario);
    expect((vecchio.prepare('PRAGMA table_info("user")').all() as { name: string }[]).map((c) => c.name)).not.toContain("twoFactorEnabled");
    const idPrima = vecchio.prepare('SELECT id FROM "user"').pluck().get();
    vecchio.close();

    await avvia();
    const { appDb } = await import("@/lib/db");
    expect(appDb().prepare('SELECT id FROM "user"').pluck().all()).toEqual([idPrima]);
    expect(appDb().prepare('SELECT twoFactorEnabled FROM "user"').pluck().get()).toBe(0);
    expect((await (await browser()).accedi(proprietario)).status).toBe(200);
  });
});

describe("avvio con AUTH_2FA=totp", () => {
  it("parte e apre le strade dei codici, non quelle delle passkey", async () => {
    process.env.AUTH_2FA = "totp";
    const righe = await avvia();
    expect(righe.some((r) => r.includes("(AUTH_2FA): totp"))).toBe(true);
    // senza sessione le strade dei codici rispondono 401 (esistono), quelle delle passkey 404
    const b = await browser();
    expect((await b.chiama("/two-factor/enable", { password: "x" })).status).toBe(401);
    expect((await b.chiama("/passkey/generate-authenticate-options")).status).toBe(404);
  });
});

describe("avvio con AUTH_2FA=passkey", () => {
  it("parte, deriva nome del sito e origine da APP_URL, e la cerimonia con una passkey arriva in fondo", async () => {
    const sito = "oe.example.com";
    process.env.AUTH_2FA = "passkey";
    process.env.APP_URL = `https://${sito}`;
    const righe = await avvia();
    expect(righe.some((r) => r.includes("(AUTH_2FA): passkey"))).toBe(true);

    const b = await browser(`https://${sito}`);
    expect((await b.accedi(proprietario)).status).toBe(200);
    const aut = new Autenticatore(`https://${sito}`, sito);
    const opzioniRegistrazione = await (await b.chiama("/passkey/generate-register-options?name=prova")).json();
    expect(opzioniRegistrazione.rp.id).toBe(sito);
    expect((await b.chiama("/passkey/verify-registration", { response: aut.registra(opzioniRegistrazione), name: "prova" })).status).toBe(200);

    const nuovo = await browser(`https://${sito}`);
    const opzioniAccesso = await (await nuovo.chiama("/passkey/generate-authenticate-options")).json();
    expect(opzioniAccesso.rpId).toBe(sito);
    expect((await nuovo.chiama("/passkey/verify-authentication", { response: aut.firma(opzioniAccesso) })).status).toBe(200);
    expect(nuovo.conSessione).toBe(true);
    expect((await nuovo.utente()).email).toBe(proprietario.email);
  });
});

describe("AUTH_2FA con un valore sbagliato", () => {
  it.each(["si", "true", "2fa", "totp,passkey", "passkeys"])("%s: l'avvio si ferma con un errore che dice i valori ammessi, prima di toccare il database", async (valore) => {
    process.env.AUTH_2FA = valore;
    const uscita = vi.spyOn(process, "exit").mockImplementation(((codice?: number) => {
      throw new Error(`exit ${codice}`);
    }) as never);
    const errori = vi.spyOn(console, "error").mockImplementation(() => {});
    const { boot } = await import("@/server/boot");
    await expect(boot()).rejects.toThrow("exit 1");
    expect(uscita).toHaveBeenCalledWith(1);
    expect(errori.mock.calls.flat().join(" ")).toMatch(/AUTH_2FA non valido.*off, totp, passkey/);
    expect(fs.existsSync(process.env.GATEWAY_DB!)).toBe(false);
  });

  it("maiuscole e spazi non contano: ' TOTP ' vale totp", async () => {
    process.env.AUTH_2FA = " TOTP ";
    const righe = await avvia();
    expect(righe.some((r) => r.includes("(AUTH_2FA): totp"))).toBe(true);
  });
});
