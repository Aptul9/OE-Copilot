import { betterAuth, type BetterAuthOptions } from "better-auth";
import { APIError, createAuthMiddleware } from "better-auth/api";
import type Database from "better-sqlite3";
import { config, PASSWORD_MAX, PASSWORD_MIN } from "./config";
import { appDb } from "./db";
import { clearFailures, lockedFor, normalizeEmail, registerFailure } from "./lockout";

export const SESSION_DAYS = 30;

export function authOptions(db: Database.Database = appDb()) {
  const secure = config.appUrl.startsWith("https://");
  return {
    appName: "OE Copilot",
    database: db,
    secret: config.authSecret,
    baseURL: config.appUrl,
    trustedOrigins: [config.appUrl],
    emailAndPassword: {
      enabled: true,
      // l'unico utente è il proprietario, creato all'avvio da ADMIN_EMAIL e ADMIN_PASSWORD
      disableSignUp: true,
      minPasswordLength: PASSWORD_MIN,
      maxPasswordLength: PASSWORD_MAX,
    },
    session: { expiresIn: 60 * 60 * 24 * SESSION_DAYS, updateAge: 60 * 60 * 24 },
    advanced: {
      // Il cookie di sessione è la credenziale: HttpOnly (non lo legge nessuno script), Secure
      // (solo su HTTPS) e SameSite (non parte con una richiesta nata su un altro sito).
      defaultCookieAttributes: { httpOnly: true, sameSite: "lax", secure },
      useSecureCookies: secure,
    },
    rateLimit: {
      enabled: true,
      window: 60,
      max: 100,
      // Dietro sslh e docker-proxy ogni client ha lo stesso indirizzo: i limiti per indirizzo
      // sono un solo contatore per tutti. Quello vero per la password è il blocco per account.
      customRules: { "/sign-in/email": { window: 60, max: 30 } },
    },
    hooks: {
      // Rifiuta l'accesso mentre l'account è bloccato, prima di guardare la password.
      before: createAuthMiddleware(async (ctx) => {
        if (ctx.path !== "/sign-in/email") return;
        const email = normalizeEmail((ctx.body as { email?: unknown } | undefined)?.email);
        const left = email ? lockedFor(db, email) : 0;
        if (left > 0) {
          throw new APIError("TOO_MANY_REQUESTS", {
            message: `Troppi tentativi falliti. Riprova tra ${Math.max(1, Math.ceil(left / 60))} minuti.`,
          });
        }
      }),
      // Conta l'errore, o azzera il conto se l'accesso è riuscito.
      after: createAuthMiddleware(async (ctx) => {
        if (ctx.path !== "/sign-in/email") return;
        const email = normalizeEmail((ctx.body as { email?: unknown } | undefined)?.email);
        if (!email) return;
        if (ctx.context.newSession) return clearFailures(db, email);
        if (lockedFor(db, email) === 0) registerFailure(db, email);
      }),
    },
  } satisfies BetterAuthOptions;
}

export const createAuth = (db: Database.Database = appDb()) => betterAuth(authOptions(db));
type Auth = ReturnType<typeof createAuth>;
let instance: Auth | null = null;

/** Creata al primo uso: la build importa i moduli senza database né segreti. */
export function auth(): Auth {
  instance ??= createAuth();
  return instance;
}
