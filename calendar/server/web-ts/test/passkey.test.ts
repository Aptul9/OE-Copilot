import { getMigrations } from "better-auth/db/migration";
import { beforeAll, describe, expect, it } from "vitest";
import { Autenticatore } from "./autenticatore";
import { type Auth, APP, Browser, type Utente, utenteNuovo } from "./browser-prova";
import { segretoDaUri, totp } from "./totp";

process.env.BETTER_AUTH_SECRET = "s".repeat(48);
process.env.APP_URL = APP;
delete process.env.AUTH_2FA;

const { authOptions, createAuth } = await import("@/lib/auth");
const { openDb } = await import("@/lib/db");
const { migrateLockout } = await import("@/lib/lockout");
const { syncOwner } = await import("@/lib/owner");

const RP_ID = "oe.test";
const db = openDb(":memory:");
let off: Auth;
let conPasskey: Auth;

beforeAll(async () => {
  await (await getMigrations(authOptions(db, "passkey"))).runMigrations();
  migrateLockout(db);
  off = createAuth(db, "off");
  conPasskey = createAuth(db, "passkey");
});

async function creaUtente(): Promise<Utente> {
  const u = utenteNuovo();
  await syncOwner(off, db, u);
  return u;
}

/** Accede con la password e registra la passkey dell'autenticatore: restituisce il browser con la sessione. */
async function registra(u: Utente, aut: Autenticatore, nome = "Portatile") {
  const b = new Browser(conPasskey);
  expect((await b.accedi(u)).status).toBe(200);
  const opzioni = await (await b.chiama(`/passkey/generate-register-options?name=${encodeURIComponent(nome)}`)).json();
  const r = await b.chiama("/passkey/verify-registration", { response: aut.registra(opzioni), name: nome });
  return { b, r };
}

async function accediConPasskey(aut: Autenticatore, firma: (o: { challenge: string }) => unknown = (o) => aut.firma(o)) {
  const b = new Browser(conPasskey);
  const opzioni = await (await b.chiama("/passkey/generate-authenticate-options")).json();
  const r = await b.chiama("/passkey/verify-authentication", { response: firma(opzioni) });
  return { b, r };
}

describe("passkey: registrazione e accesso", () => {
  it("registrata con la sessione, la passkey apre una sessione senza password, e vale per il cancello di Caddy", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    const { r } = await registra(u, aut, "Portatile");
    expect(r.status).toBe(200);
    expect(db.prepare('SELECT COUNT(*) FROM passkey p JOIN "user" x ON x.id = p.userId WHERE x.email = ?').pluck().get(u.email)).toBe(1);

    const { b, r: accesso } = await accediConPasskey(aut);
    expect(accesso.status).toBe(200);
    expect(b.conSessione).toBe(true);
    expect((await b.utente()).email).toBe(u.email);
    expect(await b.cancello()).toBe(204);
  });

  it("la lista dà il nome e non dà niente di segreto; si può rinominare e togliere", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    const { b } = await registra(u, aut, "Telefono");
    const lista = await (await b.chiama("/passkey/list-user-passkeys")).json();
    expect(lista).toHaveLength(1);
    expect(lista[0].name).toBe("Telefono");
    expect(lista[0]).not.toHaveProperty("privateKey");
    expect((await b.chiama("/passkey/update-passkey", { id: lista[0].id, name: "Telefono di casa" })).status).toBe(200);
    expect((await (await b.chiama("/passkey/list-user-passkeys")).json())[0].name).toBe("Telefono di casa");
    expect((await b.chiama("/passkey/delete-passkey", { id: lista[0].id })).status).toBe(200);
    expect(await (await b.chiama("/passkey/list-user-passkeys")).json()).toEqual([]);
    const { b: dopo, r } = await accediConPasskey(aut);
    expect(r.status).toBe(401);
    expect(dopo.conSessione).toBe(false);
  });

  it("la passkey di un altro non si toglie e non si rinomina", async () => {
    const mia = await creaUtente();
    const sua = await creaUtente();
    const { b: browserMio } = await registra(mia, new Autenticatore(APP, RP_ID), "Mia");
    const { b: browserSuo } = await registra(sua, new Autenticatore(APP, RP_ID), "Sua");
    const idSua = (await (await browserSuo.chiama("/passkey/list-user-passkeys")).json())[0].id;
    expect((await browserMio.chiama("/passkey/delete-passkey", { id: idSua })).status).toBeGreaterThanOrEqual(400);
    expect((await browserMio.chiama("/passkey/update-passkey", { id: idSua, name: "rubata" })).status).toBeGreaterThanOrEqual(400);
    expect((await (await browserSuo.chiama("/passkey/list-user-passkeys")).json())[0].name).toBe("Sua");
  });

  it("senza sessione non si registra niente e non si legge la lista", async () => {
    const b = new Browser(conPasskey);
    expect((await b.chiama("/passkey/generate-register-options")).status).toBe(401);
    expect((await b.chiama("/passkey/list-user-passkeys")).status).toBe(401);
  });

  it("con una sessione di più di un giorno non si registra: si rientra con la password", async () => {
    const u = await creaUtente();
    const b = new Browser(conPasskey);
    await b.accedi(u);
    db.prepare('UPDATE session SET createdAt = ? WHERE userId = (SELECT id FROM "user" WHERE email = ?)').run(new Date(Date.now() - 2 * 86_400_000).toISOString(), u.email);
    const r = await b.chiama("/passkey/generate-register-options");
    expect(r.status).toBe(403);
    // il codice che la pagina riconosce per dire di rientrare con la password
    expect((await r.json()).code).toBe("SESSION_NOT_FRESH");
  });

  it("una firma di un'altra chiave non apre", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    await registra(u, aut);
    const { b, r } = await accediConPasskey(aut, (o) => aut.firma(o, Autenticatore.chiaveEstranea()));
    expect(r.status).toBeGreaterThanOrEqual(400);
    expect(b.conSessione).toBe(false);
  });

  it("una passkey fatta per un altro sito (origine o nome del sito diversi) non apre", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    await registra(u, aut);
    for (const [origine, rpId] of [["https://evil.test", RP_ID], [APP, "evil.test"]] as const) {
      const { b, r } = await accediConPasskey(aut, (o) => aut.per(origine, rpId).firma(o));
      expect(r.status).toBeGreaterThanOrEqual(400);
      expect(b.conSessione).toBe(false);
    }
  });

  it("una sfida si usa una volta sola: la stessa risposta ripetuta non apre", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    await registra(u, aut);
    const b = new Browser(conPasskey);
    const opzioni = await (await b.chiama("/passkey/generate-authenticate-options")).json();
    const risposta = aut.firma(opzioni);
    expect((await b.chiama("/passkey/verify-authentication", { response: risposta })).status).toBe(200);
    const altro = new Browser(conPasskey);
    altro.jar = new Map(b.jar);
    altro.jar.delete([...altro.jar.keys()].find((k) => k.endsWith("session_token"))!);
    expect((await altro.chiama("/passkey/verify-authentication", { response: risposta })).status).toBeGreaterThanOrEqual(400);
    expect(altro.conSessione).toBe(false);
  });

  it("un contatore che non cresce (credenziale copiata) è rifiutato", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    await registra(u, aut);
    expect((await accediConPasskey(aut)).r.status).toBe(200);
    expect((await accediConPasskey(aut)).r.status).toBe(200);
    aut.contatore = 0;
    const { b, r } = await accediConPasskey(aut);
    expect(r.status).toBeGreaterThanOrEqual(400);
    expect(b.conSessione).toBe(false);
  });

  it("la passkey è un accesso completo: chi ha anche il TOTP non deve dare il codice con la passkey, ma lo dà con la password", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    const { b: sessione } = await registra(u, aut);
    const abilita = await (await sessione.chiama("/two-factor/enable", { password: u.password })).json();
    const segreto = segretoDaUri(abilita.totpURI);
    expect((await sessione.chiama("/two-factor/verify-totp", { code: totp(segreto) })).status).toBe(200);

    const { b, r } = await accediConPasskey(aut);
    expect(r.status).toBe(200);
    expect(b.conSessione).toBe(true);

    const conPassword = new Browser(conPasskey);
    expect(await (await conPassword.accedi(u)).json()).toMatchObject({ twoFactorRedirect: true });
    expect(conPassword.conSessione).toBe(false);
  });

  it("con off le passkey registrate non servono a niente: le strade non ci sono, la password basta", async () => {
    const u = await creaUtente();
    const aut = new Autenticatore(APP, RP_ID);
    await registra(u, aut);
    const b = new Browser(off);
    expect((await b.chiama("/passkey/generate-authenticate-options")).status).toBe(404);
    expect((await b.chiama("/passkey/verify-authentication", { response: aut.firma({ challenge: "x" }) })).status).toBe(404);
    expect((await b.accedi(u)).status).toBe(200);
    expect(b.conSessione).toBe(true);
  });
});
