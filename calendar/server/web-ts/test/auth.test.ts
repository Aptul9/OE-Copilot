import { getMigrations } from "better-auth/db/migration";
import { beforeAll, describe, expect, it } from "vitest";

const APP = "https://oe.test";
const EMAIL = "proprietario@oe.test";
const PASSWORD = "una password lunga e casuale";

process.env.BETTER_AUTH_SECRET = "s".repeat(48);
process.env.APP_URL = APP;

const { authOptions, createAuth } = await import("@/lib/auth");
const { openDb } = await import("@/lib/db");
const { migrateLockout } = await import("@/lib/lockout");
const { syncOwner } = await import("@/lib/owner");

const db = openDb(":memory:");
const auth = createAuth(db);

function call(path: string, body?: unknown, cookie?: string) {
  const headers: Record<string, string> = { origin: APP, "x-forwarded-for": "172.18.0.1" };
  if (body !== undefined) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  return auth.handler(
    new Request(APP + "/api/auth" + path, { method: body === undefined ? "GET" : "POST", headers, body: body === undefined ? undefined : JSON.stringify(body) }),
  );
}
const signIn = (password: string) => call("/sign-in/email", { email: EMAIL, password });
const cookieOf = (r: Response) => (r.headers.get("set-cookie") || "").split(";")[0];
const azzera = () => db.prepare("DELETE FROM login_failures").run();

beforeAll(async () => {
  await (await getMigrations(authOptions(db))).runMigrations();
  migrateLockout(db);
  await syncOwner(auth, db, { email: EMAIL, password: PASSWORD });
});

describe("accesso", () => {
  it("la password giusta apre la sessione, col cookie HttpOnly, Secure e SameSite", async () => {
    const r = await signIn(PASSWORD);
    expect(r.status).toBe(200);
    const setCookie = r.headers.get("set-cookie") || "";
    expect(setCookie).toMatch(/__Secure-better-auth\.session_token=/);
    expect(setCookie).toMatch(/HttpOnly/i);
    expect(setCookie).toMatch(/Secure/i);
    expect(setCookie).toMatch(/SameSite=Lax/i);
    const sessione = await call("/get-session", undefined, cookieOf(r));
    expect((await sessione.json()).user.email).toBe(EMAIL);
  });

  it("senza cookie la sessione non c'è", async () => {
    expect(await (await call("/get-session")).json()).toBeNull();
  });

  it("un cookie inventato non apre niente", async () => {
    const r = await call("/get-session", undefined, "__Secure-better-auth.session_token=inventato.firmato");
    expect(await r.json()).toBeNull();
  });

  it("la registrazione è spenta", async () => {
    const r = await call("/sign-up/email", { email: "altro@oe.test", name: "x", password: "un altra password lunga" });
    expect(r.status).toBeGreaterThanOrEqual(400);
    azzera();
    expect((await call("/sign-in/email", { email: "altro@oe.test", password: "un altra password lunga" })).status).toBe(401);
  });

  it("la password sbagliata è 401, e al quinto errore l'account si blocca anche per quella giusta", async () => {
    azzera();
    for (let i = 0; i < 5; i++) expect((await signIn("sbagliata numero " + i + " ".repeat(10))).status).toBe(401);
    const bloccato = await signIn(PASSWORD);
    expect(bloccato.status).toBe(429);
    expect((await bloccato.json()).message).toMatch(/Troppi tentativi/);
    // finito il quarto d'ora l'accesso torna e il conto si azzera
    db.prepare("UPDATE login_failures SET locked_until = 1").run();
    expect((await signIn(PASSWORD)).status).toBe(200);
    expect(db.prepare("SELECT COUNT(*) FROM login_failures").pluck().get()).toBe(0);
  });
});

describe("proprietario", () => {
  it("allo stesso avvio non cambia niente", async () => {
    expect(await syncOwner(auth, db, { email: EMAIL, password: PASSWORD })).toMatch(/in linea con \.env/);
  });

  it("una password nuova in .env sostituisce la vecchia e chiude le sessioni", async () => {
    azzera();
    const prima = cookieOf(await signIn(PASSWORD));
    expect(await syncOwner(auth, db, { email: EMAIL, password: "password nuova molto lunga" })).toMatch(/sessioni chiuse/);
    expect(await (await call("/get-session", undefined, prima)).json()).toBeNull();
    azzera();
    expect((await signIn(PASSWORD)).status).toBe(401);
    azzera();
    expect((await signIn("password nuova molto lunga")).status).toBe(200);
  });

  it("senza ADMIN_* e senza utenti non si parte", async () => {
    const vuoto = openDb(":memory:");
    const a = createAuth(vuoto);
    await (await getMigrations(authOptions(vuoto))).runMigrations();
    await expect(syncOwner(a, vuoto, { email: "", password: "" })).rejects.toThrow(/ADMIN_EMAIL/);
  });

  it("una password troppo corta è rifiutata", async () => {
    await expect(syncOwner(auth, db, { email: EMAIL, password: "corta" })).rejects.toThrow(/almeno 12/);
  });
});
