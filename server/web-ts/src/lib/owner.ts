import type Database from "better-sqlite3";
import type { createAuth } from "./auth";
import { config, PASSWORD_MIN } from "./config";

type Auth = ReturnType<typeof createAuth>;

/**
 * Il proprietario di .env (ADMIN_EMAIL, ADMIN_PASSWORD) esiste e accede con ADMIN_PASSWORD.
 * Una password nuova in .env chiude le sessioni aperte. Restituisce cosa è stato fatto, per il log.
 */
export async function syncOwner(
  auth: Auth,
  db: Database.Database,
  { email, password }: { email: string; password: string } = { email: config.adminEmail, password: config.adminPassword },
): Promise<string> {
  const users = db.prepare('SELECT COUNT(*) FROM "user"').pluck().get() as number;
  if (!email || !password) {
    // senza utenti nessuno può entrare
    if (!users) throw new Error("Nessun utente: servono ADMIN_EMAIL e ADMIN_PASSWORD in .env per creare il proprietario");
    return "nessun proprietario in .env";
  }
  if (password.length < PASSWORD_MIN) throw new Error(`ADMIN_PASSWORD deve avere almeno ${PASSWORD_MIN} caratteri`);

  const ctx = await auth.$context;
  const found = await ctx.internalAdapter.findUserByEmail(email, { includeAccounts: true });
  if (!found) {
    const user = await ctx.internalAdapter.createUser({ email, name: "Proprietario", emailVerified: true }, { method: "admin" });
    await ctx.internalAdapter.linkAccount({
      userId: user.id,
      providerId: "credential",
      accountId: user.id,
      password: await ctx.password.hash(password),
    });
    return `proprietario ${email} creato`;
  }
  const credential = found.accounts.find((a) => a.providerId === "credential");
  if (credential?.password && (await ctx.password.verify({ hash: credential.password, password }))) {
    return `proprietario ${email}: in linea con .env`;
  }
  const hash = await ctx.password.hash(password);
  if (credential) await ctx.internalAdapter.updatePassword(found.user.id, hash);
  else await ctx.internalAdapter.linkAccount({ userId: found.user.id, providerId: "credential", accountId: found.user.id, password: hash });
  await ctx.internalAdapter.deleteUserSessions(found.user.id);
  return `proprietario ${email}: password presa da ADMIN_PASSWORD, sessioni chiuse`;
}
