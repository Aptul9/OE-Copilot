import { getMigrations } from "better-auth/db/migration";
import { beforeAll, describe, expect, it } from "vitest";
import { type Auth, APP, Browser, type Utente, utenteNuovo } from "./browser-prova";
import { base32, codiceSbagliato, hotp, segretoDaUri, totp } from "./totp";

process.env.BETTER_AUTH_SECRET = "s".repeat(48);
process.env.APP_URL = APP;
delete process.env.AUTH_2FA;

const { authOptions, createAuth } = await import("@/lib/auth");
const { openDb } = await import("@/lib/db");
const { migrateLockout, MAX_FAILED_LOGINS } = await import("@/lib/lockout");
const { syncOwner } = await import("@/lib/owner");
const { percorsoAmmesso } = await import("@/lib/secondo-fattore");

const db = openDb(":memory:");
let off: Auth;
let conTotp: Auth;
let conPasskey: Auth;

beforeAll(async () => {
  // lo schema è quello con tutti i plugin in ogni modo, come all'avvio del gateway
  await (await getMigrations(authOptions(db, "passkey"))).runMigrations();
  migrateLockout(db);
  off = createAuth(db, "off");
  conTotp = createAuth(db, "totp");
  conPasskey = createAuth(db, "passkey");
});

/** twoFactorEnabled: nelle basi nuove senza valore (NULL) finché l'utente non attiva niente, 0 in quelle migrate da quelle vecchie. */
const acceso = (email: string) => Boolean(db.prepare('SELECT twoFactorEnabled FROM "user" WHERE email = ?').pluck().get(email));

async function creaUtente(): Promise<Utente> {
  const u = utenteNuovo();
  await syncOwner(off, db, u);
  return u;
}

/** Accede, attiva il TOTP e lo conferma col primo codice: restituisce il segreto e i codici di recupero. */
async function iscrivi(a: Auth, u: Utente) {
  const b = new Browser(a);
  expect((await b.accedi(u)).status).toBe(200);
  const abilita = await b.chiama("/two-factor/enable", { password: u.password });
  expect(abilita.status).toBe(200);
  const { totpURI, backupCodes } = await abilita.json();
  const segreto = segretoDaUri(totpURI);
  const conferma = await b.chiama("/two-factor/verify-totp", { code: totp(segreto) });
  expect(conferma.status).toBe(200);
  return { segreto, codici: backupCodes as string[], b };
}

describe("il generatore di codici delle prove", () => {
  it("dà i valori dei vettori di RFC 6238 (SHA-1, otto cifre)", () => {
    const segreto = Buffer.from("12345678901234567890");
    for (const [t, atteso] of [
      [59, "94287082"],
      [1111111109, "07081804"],
      [1111111111, "14050471"],
      [1234567890, "89005924"],
      [2000000000, "69279037"],
      [20000000000, "65353130"],
    ] as const) {
      expect(totp(segreto, t, 8)).toBe(atteso);
    }
  });
  it("dà i valori dei vettori di RFC 4226 e legge il base32 di RFC 4648", () => {
    const segreto = Buffer.from("12345678901234567890");
    expect([0, 1, 2, 9].map((c) => hotp(segreto, c))).toEqual(["755224", "287082", "359152", "520489"]);
    expect(base32("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").toString()).toBe("12345678901234567890");
  });
});

describe("modalità totp", () => {
  it("chi non ha attivato il TOTP entra come sempre, con la sola password", async () => {
    const u = await creaUtente();
    const b = new Browser(conTotp);
    const r = await b.accedi(u);
    expect(r.status).toBe(200);
    expect((await r.json()).twoFactorRedirect).toBeUndefined();
    expect(b.conSessione).toBe(true);
    expect((await b.utente()).email).toBe(u.email);
    expect(await b.cancello()).toBe(204);
  });

  it("l'attivazione chiede la password e dà l'URI del segreto e dieci codici di recupero", async () => {
    const u = await creaUtente();
    const b = new Browser(conTotp);
    await b.accedi(u);
    expect((await b.chiama("/two-factor/enable", { password: "una password che non è la sua" })).status).toBe(400);
    const r = await b.chiama("/two-factor/enable", { password: u.password });
    expect(r.status).toBe(200);
    const { totpURI, backupCodes } = await r.json();
    const uri = new URL(totpURI);
    expect(uri.protocol).toBe("otpauth:");
    expect(uri.host).toBe("totp");
    expect(uri.searchParams.get("issuer")).toBe("OE Copilot");
    expect(uri.searchParams.get("digits")).toBe("6");
    expect(uri.searchParams.get("period")).toBe("30");
    expect(decodeURIComponent(uri.pathname)).toContain(u.email);
    expect(backupCodes).toHaveLength(10);
    for (const c of backupCodes) expect(c).toMatch(/^[a-zA-Z0-9]{5}-[a-zA-Z0-9]{5}$/);
  });

  it("finché il primo codice non è confermato il secondo fattore non vale: l'accesso resta quello di prima", async () => {
    const u = await creaUtente();
    const b = new Browser(conTotp);
    await b.accedi(u);
    await b.chiama("/two-factor/enable", { password: u.password });
    const verificato = db.prepare('SELECT t.verified FROM twoFactor t JOIN "user" u ON u.id = t.userId WHERE u.email = ?').pluck().get(u.email);
    expect(verificato).toBe(0);
    expect(acceso(u.email)).toBe(false);
    const altro = new Browser(conTotp);
    expect((await altro.accedi(u)).status).toBe(200);
    expect(altro.conSessione).toBe(true);
  });

  it("un primo codice sbagliato non attiva niente, quello giusto sì", async () => {
    const u = await creaUtente();
    const b = new Browser(conTotp);
    await b.accedi(u);
    const { totpURI } = await (await b.chiama("/two-factor/enable", { password: u.password })).json();
    const segreto = segretoDaUri(totpURI);
    expect((await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) })).status).toBe(401);
    expect(acceso(u.email)).toBe(false);
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
    expect(acceso(u.email)).toBe(true);
  });

  it("il segreto sta cifrato nel database, mai in chiaro", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const riga = db.prepare('SELECT t.secret, t.backupCodes FROM twoFactor t JOIN "user" u ON u.id = t.userId WHERE u.email = ?').get(u.email) as { secret: string; backupCodes: string };
    expect(riga.secret).not.toContain(base32Inverso(segreto));
    expect(riga.secret).toMatch(/^[0-9a-f]+$/);
    expect(riga.backupCodes).toMatch(/^[0-9a-f]+$/);
  });

  it("dopo l'attivazione la password da sola non apre: niente cookie di sessione prima del codice, né per il cancello di Caddy", async () => {
    const u = await creaUtente();
    await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    const r = await b.accedi(u);
    expect(r.status).toBe(200);
    expect(await r.json()).toMatchObject({ twoFactorRedirect: true, twoFactorMethods: ["totp"] });
    expect(b.conSessione).toBe(false);
    expect(await b.utente()).toBeNull();
    expect(await b.cancello()).toBe(302);
    expect([...b.jar.keys()].some((k) => k.endsWith("two_factor"))).toBe(true);
  });

  it("con il codice giusto la sessione nasce, e vale anche per il cancello di Caddy", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    await b.accedi(u);
    const r = await b.chiama("/two-factor/verify-totp", { code: totp(segreto) });
    expect(r.status).toBe(200);
    expect(b.conSessione).toBe(true);
    expect((await b.utente()).email).toBe(u.email);
    expect(await b.cancello()).toBe(204);
    expect([...b.jar.keys()].some((k) => k.endsWith("two_factor"))).toBe(false);
  });

  it("con un codice sbagliato non c'è sessione, e la sfida resta valida per un altro tentativo", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    await b.accedi(u);
    const male = await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) });
    expect(male.status).toBe(401);
    expect(b.conSessione).toBe(false);
    expect(await b.cancello()).toBe(302);
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
    expect(b.conSessione).toBe(true);
  });

  it("'fidati di questo dispositivo' non esiste: anche chiedendolo, la password da sola non basta mai", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    await b.accedi(u);
    const r = await b.chiama("/two-factor/verify-totp", { code: totp(segreto), trustDevice: true });
    expect(r.status).toBe(200);
    expect([...b.jar.keys()].some((k) => k.endsWith("trust_device"))).toBe(false);
    const dopo = new Browser(conTotp);
    for (const [k, v] of b.jar) if (k.endsWith("trust_device")) dopo.jar.set(k, v);
    expect(await (await dopo.accedi(u)).json()).toMatchObject({ twoFactorRedirect: true });
    expect(dopo.conSessione).toBe(false);
  });

  it("senza aver passato la password il codice non apre niente", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    const r = await b.chiama("/two-factor/verify-totp", { code: totp(segreto) });
    expect(r.status).toBe(401);
    expect(b.conSessione).toBe(false);
  });

  it("cinque codici sbagliati di fila bloccano l'account per un quarto d'ora, anche per il codice giusto", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    for (let i = 0; i < MAX_FAILED_LOGINS; i++) {
      const b = new Browser(conTotp);
      await b.accedi(u);
      expect((await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) })).status).toBe(401);
    }
    const b = new Browser(conTotp);
    await b.accedi(u);
    const r = await b.chiama("/two-factor/verify-totp", { code: totp(segreto) });
    expect(r.status).toBe(429);
    expect(b.conSessione).toBe(false);
    const blocco = Date.parse(String(db.prepare('SELECT t.lockedUntil FROM twoFactor t JOIN "user" u ON u.id = t.userId WHERE u.email = ?').pluck().get(u.email)));
    expect(blocco - Date.now()).toBeGreaterThan(14 * 60_000);
    expect(blocco - Date.now()).toBeLessThanOrEqual(15 * 60_000);
    // finito il quarto d'ora il codice giusto torna a valere
    db.prepare('UPDATE twoFactor SET lockedUntil = ? WHERE userId = (SELECT id FROM "user" WHERE email = ?)').run(new Date(Date.now() - 1000).toISOString(), u.email);
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
  });

  it("cinque errori su una stessa sfida bloccano l'account: il sesto, anche giusto, è rifiutato", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    await b.accedi(u);
    for (let i = 0; i < 4; i++) expect((await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) })).status).toBe(401);
    expect((await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) })).status).toBe(401);
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(429);
  });

  it("il limite per indirizzo non scatta prima del blocco per account: tutti i client hanno lo stesso indirizzo", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    const b = new Browser(conTotp);
    const stesso = { "x-forwarded-for": "172.18.0.1" };
    await b.chiama("/sign-in/email", { email: u.email, password: u.password }, stesso);
    for (let i = 0; i < MAX_FAILED_LOGINS; i++) expect((await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) }, stesso)).status).toBe(401);
    const bloccato = await b.chiama("/two-factor/verify-totp", { code: totp(segreto) }, stesso);
    expect(bloccato.status).toBe(429);
    expect((await bloccato.json()).code).toBe("ACCOUNT_TEMPORARILY_LOCKED");
  });

  it("un accesso riuscito azzera il conto degli errori di codice", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    for (let giro = 0; giro < 3; giro++) {
      const b = new Browser(conTotp);
      await b.accedi(u);
      for (let i = 0; i < MAX_FAILED_LOGINS - 1; i++) await b.chiama("/two-factor/verify-totp", { code: codiceSbagliato(segreto) });
      expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
    }
  });

  it("il blocco della password non si mescola con quello dei codici: password giuste di fila non bloccano il proprietario", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    for (let i = 0; i < MAX_FAILED_LOGINS + 2; i++) {
      const b = new Browser(conTotp);
      const r = await b.accedi(u);
      expect(r.status).toBe(200);
      expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
    }
    expect(db.prepare("SELECT COUNT(*) FROM login_failures WHERE email = ?").pluck().get(u.email)).toBe(0);
  });

  it("la password sbagliata conta come prima, anche per chi ha il TOTP", async () => {
    const u = await creaUtente();
    await iscrivi(conTotp, u);
    for (let i = 0; i < MAX_FAILED_LOGINS; i++) expect((await new Browser(conTotp).chiama("/sign-in/email", { email: u.email, password: "sbagliata " + i + "x".repeat(12) })).status).toBe(401);
    expect((await new Browser(conTotp).accedi(u)).status).toBe(429);
  });

  describe("codici di recupero", () => {
    it("un codice apre una volta sola", async () => {
      const u = await creaUtente();
      const { codici } = await iscrivi(conTotp, u);
      const b = new Browser(conTotp);
      await b.accedi(u);
      expect((await b.chiama("/two-factor/verify-backup-code", { code: codici[0] })).status).toBe(200);
      expect(b.conSessione).toBe(true);
      expect(await b.cancello()).toBe(204);

      const secondo = new Browser(conTotp);
      await secondo.accedi(u);
      expect((await secondo.chiama("/two-factor/verify-backup-code", { code: codici[0] })).status).toBe(401);
      expect(secondo.conSessione).toBe(false);
      // gli altri nove valgono ancora
      expect((await secondo.chiama("/two-factor/verify-backup-code", { code: codici[1] })).status).toBe(200);
      expect(secondo.conSessione).toBe(true);
    });

    it("un codice inventato non apre e conta come un errore di codice", async () => {
      const u = await creaUtente();
      await iscrivi(conTotp, u);
      const b = new Browser(conTotp);
      await b.accedi(u);
      expect((await b.chiama("/two-factor/verify-backup-code", { code: "aaaaa-bbbbb" })).status).toBe(401);
      expect(b.conSessione).toBe(false);
      const conto = db.prepare('SELECT t.failedVerificationCount FROM twoFactor t JOIN "user" x ON x.id = t.userId WHERE x.email = ?').pluck().get(u.email);
      expect(conto).toBe(1);
    });

    it("dieci codici nuovi sostituiscono i vecchi, e serve la password", async () => {
      const u = await creaUtente();
      const { codici, b } = await iscrivi(conTotp, u);
      expect((await b.chiama("/two-factor/generate-backup-codes", { password: "una password che non è la sua" })).status).toBe(400);
      const r = await b.chiama("/two-factor/generate-backup-codes", { password: u.password });
      expect(r.status).toBe(200);
      const nuovi = (await r.json()).backupCodes as string[];
      expect(nuovi).toHaveLength(10);
      const c = new Browser(conTotp);
      await c.accedi(u);
      expect((await c.chiama("/two-factor/verify-backup-code", { code: codici[0] })).status).toBe(401);
      expect((await c.chiama("/two-factor/verify-backup-code", { code: nuovi[0] })).status).toBe(200);
    });
  });

  describe("disattivazione", () => {
    it("chiede la password, toglie il segreto e l'accesso torna alla sola password", async () => {
      const u = await creaUtente();
      const { b } = await iscrivi(conTotp, u);
      expect((await b.chiama("/two-factor/disable", { password: "una password che non è la sua" })).status).toBe(400);
      expect((await b.chiama("/two-factor/disable", { password: u.password })).status).toBe(200);
      expect(db.prepare('SELECT COUNT(*) FROM twoFactor t JOIN "user" x ON x.id = t.userId WHERE x.email = ?').pluck().get(u.email)).toBe(0);
      expect(acceso(u.email)).toBe(false);
      const dopo = new Browser(conTotp);
      const r = await dopo.accedi(u);
      expect(r.status).toBe(200);
      expect((await r.json()).twoFactorRedirect).toBeUndefined();
      expect(dopo.conSessione).toBe(true);
    });

    it("senza sessione non si disattiva niente", async () => {
      const u = await creaUtente();
      await iscrivi(conTotp, u);
      const r = await new Browser(conTotp).chiama("/two-factor/disable", { password: u.password });
      expect(r.status).toBe(401);
      expect(acceso(u.email)).toBe(true);
    });
  });

  it("si può attivare di nuovo dopo aver disattivato, con un segreto nuovo", async () => {
    const u = await creaUtente();
    const prima = await iscrivi(conTotp, u);
    await prima.b.chiama("/two-factor/disable", { password: u.password });
    const dopo = await iscrivi(conTotp, u);
    expect(dopo.segreto.equals(prima.segreto)).toBe(false);
    const b = new Browser(conTotp);
    expect(await (await b.accedi(u)).json()).toMatchObject({ twoFactorRedirect: true });
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(prima.segreto) })).status).toBe(401);
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(dopo.segreto) })).status).toBe(200);
  });

  it("le strade che la pagina non usa non si aprono nemmeno al gateway", () => {
    for (const p of ["/two-factor/send-otp", "/two-factor/verify-otp", "/two-factor/get-totp-uri", "/passkey/list-user-passkeys"]) {
      expect(percorsoAmmesso("totp", "POST", p)).toBe(false);
    }
  });
});

describe("modalità off", () => {
  it("le strade del secondo fattore non esistono: né nel plugin né per il gateway", async () => {
    const b = new Browser(off);
    for (const p of ["/two-factor/enable", "/two-factor/verify-totp", "/two-factor/verify-backup-code", "/two-factor/disable"]) {
      expect((await b.chiama(p, { password: "x", code: "123456" })).status).toBe(404);
    }
    for (const p of ["/passkey/generate-authenticate-options", "/passkey/list-user-passkeys"]) expect((await b.chiama(p)).status).toBe(404);
    for (const metodo of ["GET", "POST"]) {
      for (const p of ["/two-factor/enable", "/two-factor/verify-totp", "/passkey/generate-authenticate-options", "/TWO-FACTOR/enable", "/two-factor//enable/", "/passkey"]) {
        expect(percorsoAmmesso("off", metodo, p)).toBe(false);
      }
    }
  });

  it("con il TOTP già attivato (si è passati da totp a off) basta la password: è la strada di recupero", async () => {
    const u = await creaUtente();
    await iscrivi(conTotp, u);
    const b = new Browser(off);
    const r = await b.accedi(u);
    expect(r.status).toBe(200);
    expect((await r.json()).twoFactorRedirect).toBeUndefined();
    expect(b.conSessione).toBe(true);
    expect((await b.utente()).email).toBe(u.email);
    expect(await b.cancello()).toBe(204);
  });

  it("passare da off a totp e ritorno non cancella niente: il codice vale di nuovo", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conTotp, u);
    await new Browser(off).accedi(u);
    const b = new Browser(conTotp);
    expect(await (await b.accedi(u)).json()).toMatchObject({ twoFactorRedirect: true });
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
  });

  it("l'accesso con la password è quello di sempre: stessa risposta, blocco dopo cinque errori", async () => {
    const u = await creaUtente();
    const b = new Browser(off);
    const r = await b.accedi(u);
    expect(r.status).toBe(200);
    expect(Object.keys(await r.json()).sort()).toEqual(["redirect", "token", "user"]);
    for (let i = 0; i < MAX_FAILED_LOGINS; i++) expect((await new Browser(off).chiama("/sign-in/email", { email: u.email, password: "sbagliata " + i + "x".repeat(12) })).status).toBe(401);
    expect((await new Browser(off).accedi(u)).status).toBe(429);
  });
});

describe("percorsoAmmesso", () => {
  it("fuori da /two-factor e /passkey passa tutto, in ogni modo", () => {
    for (const modo of ["off", "totp", "passkey"] as const) {
      for (const p of ["/sign-in/email", "/sign-out", "/get-session", "/sign-up/email", "/two-factorx", "/passkeys-no", ""]) {
        expect(percorsoAmmesso(modo, "POST", p)).toBe(true);
      }
    }
  });

  it("con totp passano i codici e non le passkey, con passkey tutti e due", () => {
    expect(percorsoAmmesso("totp", "POST", "/two-factor/verify-totp")).toBe(true);
    expect(percorsoAmmesso("totp", "GET", "/two-factor/verify-totp")).toBe(false);
    expect(percorsoAmmesso("totp", "GET", "/passkey/generate-authenticate-options")).toBe(false);
    expect(percorsoAmmesso("passkey", "GET", "/passkey/generate-authenticate-options")).toBe(true);
    expect(percorsoAmmesso("passkey", "POST", "/passkey/verify-authentication")).toBe(true);
    expect(percorsoAmmesso("passkey", "POST", "/two-factor/verify-totp")).toBe(true);
    expect(percorsoAmmesso("passkey", "POST", "/two-factor/send-otp")).toBe(false);
  });

  it("maiuscole, barre doppie, barra finale e codifiche non aggirano il controllo", () => {
    expect(percorsoAmmesso("off", "POST", "/Two-Factor/Verify-TOTP")).toBe(false);
    expect(percorsoAmmesso("off", "POST", "/two%2Dfactor/verify-totp")).toBe(false);
    expect(percorsoAmmesso("off", "POST", "//two-factor//verify-totp/")).toBe(false);
    expect(percorsoAmmesso("totp", "POST", "/two-factor/send-otp/")).toBe(false);
    expect(percorsoAmmesso("totp", "POST", "/two-factor/%zz")).toBe(false);
  });
});

describe("modalità passkey", () => {
  it("parte e tiene anche i codici: chi ha il TOTP lo usa con la password", async () => {
    const u = await creaUtente();
    const { segreto } = await iscrivi(conPasskey, u);
    const b = new Browser(conPasskey);
    expect(await (await b.accedi(u)).json()).toMatchObject({ twoFactorRedirect: true });
    expect(b.conSessione).toBe(false);
    expect((await b.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);
  });

  it("la richiesta di accesso con passkey si apre senza sessione e porta l'identità del sito, da APP_URL", async () => {
    const r = await new Browser(conPasskey).chiama("/passkey/generate-authenticate-options");
    expect(r.status).toBe(200);
    const o = await r.json();
    expect(o.rpId).toBe("oe.test");
    expect(typeof o.challenge).toBe("string");
    expect(o.allowCredentials ?? []).toEqual([]);
  });

  it("registrare una passkey chiede la sessione, e le opzioni chiedono una credenziale rilevabile con verifica dell'utente", async () => {
    expect((await new Browser(conPasskey).chiama("/passkey/generate-register-options")).status).toBe(401);
    const u = await creaUtente();
    const b = new Browser(conPasskey);
    await b.accedi(u);
    const r = await b.chiama("/passkey/generate-register-options?name=prova");
    expect(r.status).toBe(200);
    const o = await r.json();
    expect(o.rp).toEqual({ name: "OE Copilot", id: "oe.test" });
    expect(o.authenticatorSelection).toMatchObject({ residentKey: "required", userVerification: "required" });
    expect(o.attestation).toBe("none");
  });

  it("in totp le strade delle passkey non esistono", async () => {
    expect((await new Browser(conTotp).chiama("/passkey/generate-authenticate-options")).status).toBe(404);
  });
});

function base32Inverso(segreto: Buffer): string {
  // il segreto che sta nell'URI è il testo del segreto cifrato nel database: qui basta che il testo in chiaro non vi compaia
  return segreto.toString("utf8");
}
