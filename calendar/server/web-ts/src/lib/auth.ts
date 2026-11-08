import { passkey } from "@better-auth/passkey";
import { betterAuth, type BetterAuthOptions } from "better-auth";
import { APIError, createAuthMiddleware } from "better-auth/api";
import { twoFactor } from "better-auth/plugins/two-factor";
import type Database from "better-sqlite3";
import { config, type Modo2fa, PASSWORD_MAX, PASSWORD_MIN } from "./config";
import { appDb } from "./db";
import { clearFailures, LOCKOUT_MINUTES, lockedFor, MAX_FAILED_LOGINS, normalizeEmail, registerFailure } from "./lockout";

export const SESSION_DAYS = 30;

/**
 * I plugin del secondo fattore. Con off non ce n'è nessuno: le strade /two-factor e /passkey non esistono e l'accesso è quello di sempre.
 * La passkey include il TOTP: chi entra con la password, anche con le passkey accese, passa dal codice se lo ha attivato.
 */
function pluginPerModo(modo: Modo2fa) {
  if (modo === "off") return [];
  const url = new URL(config.appUrl);
  const codici = twoFactor({
    issuer: "OE Copilot",
    // Gli errori di codice si contano per account come quelli della password: stessi numeri di lockout.ts, e il conto
    // sta nella riga twoFactor (failedVerificationCount, lockedUntil), che il plugin azzera a ogni codice giusto.
    accountLockout: { enabled: true, maxFailedAttempts: MAX_FAILED_LOGINS, durationSeconds: LOCKOUT_MINUTES * 60 },
    // niente "fidati di questo dispositivo": il cookie nasce già scaduto, la password passa sempre dal codice
    trustDeviceMaxAge: 0,
  });
  if (modo === "totp") return [codici];
  return [
    codici,
    passkey({
      // l'identità del sito per il browser: nome e origine dell'indirizzo pubblico (APP_URL), non quelli della richiesta
      rpID: url.hostname,
      rpName: "OE Copilot",
      origin: url.origin,
      // "Accedi con passkey" parte senza email: servono credenziali rilevabili dal browser, e il PIN o il dito sul dispositivo
      authenticatorSelection: { residentKey: "required", userVerification: "required" },
    }),
  ];
}

/** La password è giusta ma l'accesso non è ancora finito: il plugin ha tolto la sessione e chiede il codice. */
const chiedeIlCodice = (risposta: unknown) =>
  typeof risposta === "object" && risposta !== null && (risposta as { twoFactorRedirect?: unknown }).twoFactorRedirect === true;

export function authOptions(db: Database.Database = appDb(), modo: Modo2fa = config.auth2fa) {
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
      // Con il secondo fattore lo stesso vale per i codici: il plugin li limita a 3 ogni 10 secondi per indirizzo, e il freno
      // che conta è il blocco per account (5 errori, 15 minuti).
      customRules: {
        "/sign-in/email": { window: 60, max: 30 },
        ...(modo === "off" ? {} : { "/two-factor/verify-totp": { window: 60, max: 30 }, "/two-factor/verify-backup-code": { window: 60, max: 30 } }),
      },
    },
    plugins: pluginPerModo(modo),
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
      // Conta l'errore, o azzera il conto se la password era giusta. Con il secondo fattore la sessione nasce e il
      // plugin la toglie subito dopo (e poi newSession è null): senza guardare la risposta ogni password giusta
      // varrebbe come un errore, e cinque accessi del proprietario lo bloccherebbero.
      after: createAuthMiddleware(async (ctx) => {
        if (ctx.path !== "/sign-in/email") return;
        const email = normalizeEmail((ctx.body as { email?: unknown } | undefined)?.email);
        if (!email) return;
        if (ctx.context.newSession || chiedeIlCodice(ctx.context.returned)) return clearFailures(db, email);
        if (lockedFor(db, email) === 0) registerFailure(db, email);
      }),
    },
  } satisfies BetterAuthOptions;
}

export const createAuth = (db: Database.Database = appDb(), modo: Modo2fa = config.auth2fa) => betterAuth(authOptions(db, modo));
type Auth = ReturnType<typeof createAuth>;
let instance: Auth | null = null;

/** Creata al primo uso: la build importa i moduli senza database né segreti. */
export function auth(): Auth {
  instance ??= createAuth();
  return instance;
}
